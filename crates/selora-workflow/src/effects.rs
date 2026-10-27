//! Per-run tool-effect ledger: durable evidence for a fail-closed resume (P2).
//!
//! A workflow ask may run side-effecting tools (`write_file`, `edit_file`,
//! `delete_file`, `bash`). The engine's journal only proves that an ask
//! *started*; it cannot prove whether the tool's effect landed before a crash.
//! This ledger closes that gap for the resume preflight:
//!
//! * before a side-effecting tool executes, the host appends a `started`
//!   record and makes it durable;
//! * after the tool returns (success, error or cancellation), it appends a
//!   matching `finished` record with the same `effect_id` and `attempt`.
//!
//! A crash therefore leaves `started` without `finished`. That is durable
//! evidence of a partial effect, so [`crate::run_workflow_persisted`] refuses
//! to resume without an explicit acknowledgement. A corrupt ledger is a typed
//! error, never treated as empty.
//!
//! # Stable identity and replay
//!
//! `effect_id` is derived from the durable run id, the owning step/ask, the
//! model's `call_id` and the tool name. Re-dispatching the same tool call
//! therefore produces the same id. When the ledger proves that call already
//! finished successfully, the host does **not** run it again: it appends a
//! `replayed` record and returns a synthetic success to the agent. When the
//! earlier attempt is unfinished or failed, the host refuses to run it again
//! until an `acknowledged` record for that attempt exists. Re-using one
//! `call_id` with different arguments is refused, never reinterpreted.
//!
//! # Acknowledgements
//!
//! [`crate::persisted::acknowledge_partial_effects`] appends one
//! `acknowledged` record per risky attempt, naming the durable step/ask, the
//! actor and the reason. That record is the audit evidence for the owner's
//! decision and is what permits one retry of that attempt.
//!
//! # Durability and remaining windows
//!
//! Every record is one `write_all` followed by `sync_all`, so it is durable
//! when the call returns. Appends are serialized by a per-run lock file
//! (`tool-effects.lock`) held across the read, the append and the sync, so two
//! hosts cannot interleave sequence numbers. On Unix the run directory is
//! `fsync`ed after the ledger file is created.
//!
//! This is **not** exactly-once across an OS or power loss:
//!
//! * A process crash after the tool's effect lands but before the `finished`
//!   append leaves an unmatched `started`; resume fails closed.
//! * A process crash after `finished` is durable but before the engine records
//!   the ask's result leaves a completed effect whose ask resumes; the same
//!   `call_id` replays without re-running, but a new model turn can issue a
//!   new `call_id` and a new effect.
//! * A power loss can lose a completed append despite `sync_all` (drive caches
//!   without flush-on-commit, and Windows directory metadata which this code
//!   cannot `fsync`), and can leave a torn final line; a torn line fails
//!   closed as corruption.
//! * A side effect that lands *outside* the ledger's knowledge (a process
//!   spawned by `bash` and still running, a command that ignored cancellation)
//!   cannot be fenced by a userspace append. Stopping such work is the
//!   process-tree/Job Object concern, not this ledger's.
//!
//! # Format
//!
//! `.selora/workflow-runs/{run_id}/tool-effects.jsonl`, one JSON object per
//! line, contiguous `seq` starting at 1. Records of one attempt share
//! `effect_id` and `attempt`; `call_id` is the model's tool-call id. `step`
//! names the durable step/ask when the host knows it. `args` is a redacted
//! summary: paths and byte counts only, never file bodies, command text, or
//! secrets. `attempt` defaults to 1 so ledgers written before retries existed
//! stay readable.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::journal::{check_file, checked_run_dir};
use crate::{RunId, WorkflowError};

/// The ledger file inside one run directory.
pub const TOOL_EFFECTS_FILE: &str = "tool-effects.jsonl";
/// The per-run append lock. One holder appends at a time.
pub const TOOL_EFFECTS_LOCK_FILE: &str = "tool-effects.lock";

/// Longest path kept in a redacted argument summary.
const MAX_SUMMARY_PATH_CHARS: usize = 256;
/// Longest ledger file accepted by the preflight (16 MiB).
const MAX_LEDGER_BYTES: u64 = 16 * 1024 * 1024;
/// How long an append waits for the per-run lock before failing closed.
const LOCK_WAIT: Duration = Duration::from_secs(5);
/// Poll interval while waiting for the per-run lock.
const LOCK_POLL: Duration = Duration::from_millis(5);

/// The durable step/ask identity that owned a tool call, when known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEffectStep {
    /// `WorkflowStep::index`, as text (matches `IncompleteAsk::step_id`).
    pub step_id: String,
    /// The named workflow agent, when the step is an ask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// One durable fact about a tool effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolEffectPhase {
    /// Appended before the tool executes.
    Started,
    /// Appended after the tool returned (or was cancelled).
    Finished,
    /// The owner accepted this attempt's risk; permits one retry.
    Acknowledged,
    /// A completed attempt was not run again; the host returned evidence.
    Replayed,
}

/// One line of the per-run tool-effect ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEffectRecord {
    /// Contiguous 1-based position in this run's ledger.
    pub seq: u64,
    /// Run identity; a record from another run is corruption, not evidence.
    pub run_id: String,
    /// Stable identity shared by every record of one tool call.
    pub effect_id: String,
    /// Durable step/ask identity, when the host knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<ToolEffectStep>,
    /// The model's tool-call id.
    pub call_id: String,
    /// Tool name (`write_file`, `edit_file`, `delete_file`, `bash`).
    pub tool: String,
    /// Redacted argument summary: paths and byte counts only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Which durable fact this line records.
    pub phase: ToolEffectPhase,
    /// 1-based attempt of this effect; a retry needs an acknowledgement.
    #[serde(default = "first_attempt")]
    pub attempt: u32,
    /// Present only on `finished`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// Present only on a failed `finished`; a fixed category, no tool output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Present only on `acknowledged`: who accepted the risk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_by: Option<String>,
    /// Present only on `acknowledged`: why the risk was accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_reason: Option<String>,
}

const fn first_attempt() -> u32 {
    1
}

/// How [`ToolEffectLedger::begin_effect`] resolved one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectBegin {
    /// No durable completion: the caller may run the tool. `started` is durable.
    Execute {
        /// Stable identity of this effect.
        effect_id: String,
        /// The attempt that was just recorded.
        attempt: u32,
    },
    /// A durable completed attempt exists: never run the tool again.
    AlreadyCompleted {
        /// Stable identity of this effect.
        effect_id: String,
        /// The completed attempt.
        attempt: u32,
    },
}

impl EffectBegin {
    /// The stable effect id either way.
    #[must_use]
    pub fn effect_id(&self) -> &str {
        match self {
            Self::Execute { effect_id, .. } | Self::AlreadyCompleted { effect_id, .. } => effect_id,
        }
    }

    /// The attempt the ledger recorded or proved completed.
    #[must_use]
    pub fn attempt(&self) -> u32 {
        match self {
            Self::Execute { attempt, .. } | Self::AlreadyCompleted { attempt, .. } => *attempt,
        }
    }

    /// Whether the tool must be executed now.
    #[must_use]
    pub fn must_execute(&self) -> bool {
        matches!(self, Self::Execute { .. })
    }
}

/// Latest durable outcome of one effect id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectOutcome {
    /// A `started` attempt has no `finished` record.
    Open,
    /// The latest attempt recorded an outcome.
    Finished {
        /// Whether the tool reported success.
        ok: bool,
    },
}

/// The latest attempt of one effect, folded from its ledger records.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectState {
    /// Stable identity.
    pub effect_id: String,
    /// Latest attempt number.
    pub attempt: u32,
    /// Durable step/ask identity, when the ledger knows it.
    pub step: Option<ToolEffectStep>,
    /// The model's tool-call id.
    pub call_id: String,
    /// Tool name.
    pub tool: String,
    /// Redacted argument summary.
    pub args: Option<Value>,
    /// Latest attempt state.
    pub outcome: EffectOutcome,
    /// An acknowledgement exists for the latest attempt.
    pub acknowledged: bool,
    /// A `replayed` record exists for the latest attempt.
    pub replayed: bool,
}

/// Fold validated records into the latest attempt of every effect id.
///
/// The result is ordered by `effect_id`, so two callers compute the same
/// classification from the same ledger.
#[must_use]
pub fn effect_states(records: &[ToolEffectRecord]) -> Vec<EffectState> {
    let mut states: BTreeMap<&str, EffectState> = BTreeMap::new();
    for record in records {
        match record.phase {
            ToolEffectPhase::Started => {
                states.insert(
                    record.effect_id.as_str(),
                    EffectState {
                        effect_id: record.effect_id.clone(),
                        attempt: record.attempt,
                        step: record.step.clone(),
                        call_id: record.call_id.clone(),
                        tool: record.tool.clone(),
                        args: record.args.clone(),
                        outcome: EffectOutcome::Open,
                        acknowledged: false,
                        replayed: false,
                    },
                );
            }
            ToolEffectPhase::Finished => {
                if let Some(state) = states.get_mut(record.effect_id.as_str()) {
                    state.outcome = EffectOutcome::Finished {
                        ok: record.ok == Some(true),
                    };
                }
            }
            ToolEffectPhase::Acknowledged => {
                if let Some(state) = states.get_mut(record.effect_id.as_str()) {
                    state.acknowledged = true;
                }
            }
            ToolEffectPhase::Replayed => {
                if let Some(state) = states.get_mut(record.effect_id.as_str()) {
                    state.replayed = true;
                }
            }
        }
    }
    states.into_values().collect()
}

/// The path of one run's tool-effect ledger.
#[must_use]
pub fn tool_effects_path(workspace: &Path, run_id: &RunId) -> PathBuf {
    crate::journal::run_dir(workspace, run_id).join(TOOL_EFFECTS_FILE)
}

/// The stable effect id for one tool call.
///
/// Derived only from durable identities (run, owning step/ask, the model's
/// call id, tool name). Argument contents never enter the id, so no content
/// fingerprint is persisted; a re-used id with different arguments is refused
/// by [`ToolEffectLedger::begin_effect`] instead.
#[must_use]
pub fn stable_effect_id(
    run_id: &str,
    step: Option<&ToolEffectStep>,
    call_id: &str,
    tool: &str,
) -> String {
    let empty = "";
    let parts = [
        run_id,
        step.map_or(empty, |step| step.step_id.as_str()),
        step.and_then(|step| step.agent.as_deref()).unwrap_or(empty),
        call_id,
        tool,
    ];
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(2 + 32);
    hex.push_str("fx-");
    for byte in &digest[..16] {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn ledger_error(message: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::ToolEffectLedger {
        message: message.to_string(),
    }
}

/// The lock file name next to a ledger (`tool-effects.jsonl` →
/// `tool-effects.lock`).
fn lock_file_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map_or_else(
            || TOOL_EFFECTS_LOCK_FILE.to_owned(),
            |stem| format!("{stem}.lock"),
        )
}

/// Read and validate one run's ledger. A missing file is an empty ledger; a
/// malformed, non-contiguous, foreign or unpaired-at-finish file is a typed
/// error.
///
/// # Errors
/// Invalid run id, untrusted path, oversized file, unreadable bytes, or any
/// record that cannot be proven well formed.
pub fn load_tool_effects(
    workspace: &Path,
    run_id: &RunId,
) -> Result<Vec<ToolEffectRecord>, WorkflowError> {
    let dir = checked_run_dir(workspace, run_id, false).map_err(ledger_error)?;
    load_records_at(&dir.join(TOOL_EFFECTS_FILE), run_id.as_str())
}

/// One attempt as proven by the records read so far.
struct TrackedAttempt {
    attempt: u32,
    call_id: String,
    tool: String,
    step: Option<ToolEffectStep>,
    args: Option<Value>,
    open: bool,
    ok: Option<bool>,
    acknowledged: bool,
    replayed: bool,
}

fn load_records_at(path: &Path, run_id: &str) -> Result<Vec<ToolEffectRecord>, WorkflowError> {
    check_file(path).map_err(ledger_error)?;
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(ledger_error(format!(
                "cannot inspect {}: {source}",
                path.display()
            )))
        }
    };
    if size > MAX_LEDGER_BYTES {
        return Err(ledger_error(format!(
            "{} is {size} bytes, over the {MAX_LEDGER_BYTES}-byte cap",
            path.display()
        )));
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(ledger_error(format!(
                "cannot read {}: {source}",
                path.display()
            )))
        }
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        ledger_error(format!(
            "{} is not UTF-8; the ledger is corrupt",
            path.display()
        ))
    })?;
    let mut records = Vec::new();
    let mut effects: BTreeMap<String, TrackedAttempt> = BTreeMap::new();
    for (offset, line) in text.lines().enumerate() {
        let line_number = offset + 1;
        if line.trim().is_empty() {
            return Err(ledger_error(format!(
                "{} has an empty record at line {line_number}",
                path.display()
            )));
        }
        let record: ToolEffectRecord = serde_json::from_str(line).map_err(|error| {
            ledger_error(format!(
                "{} has an unreadable record at line {line_number}: {error}",
                path.display()
            ))
        })?;
        if record.seq != offset as u64 + 1 {
            return Err(ledger_error(format!(
                "{} has a non-contiguous sequence at line {line_number}",
                path.display()
            )));
        }
        if record.run_id != run_id {
            return Err(ledger_error(format!(
                "{} has a record for another run at line {line_number}",
                path.display()
            )));
        }
        if record.effect_id.trim().is_empty()
            || record.call_id.trim().is_empty()
            || record.tool.trim().is_empty()
        {
            return Err(ledger_error(format!(
                "{} has an incomplete record at line {line_number}",
                path.display()
            )));
        }
        if record.attempt == 0 {
            return Err(ledger_error(format!(
                "{} has attempt 0 at line {line_number}",
                path.display()
            )));
        }
        match record.phase {
            ToolEffectPhase::Started => {
                if record.ok.is_some()
                    || record.error.is_some()
                    || record.acknowledged_by.is_some()
                    || record.acknowledged_reason.is_some()
                {
                    return Err(ledger_error(format!(
                        "{} has an outcome field on a started record at line {line_number}",
                        path.display()
                    )));
                }
                if record.attempt != 1
                    && !effects
                        .get(record.effect_id.as_str())
                        .is_some_and(|previous| {
                            previous.attempt.checked_add(1) == Some(record.attempt)
                        })
                {
                    return Err(ledger_error(format!(
                        "{} starts attempt {} out of order at line {line_number}",
                        path.display(),
                        record.attempt
                    )));
                }
                match effects.get_mut(record.effect_id.as_str()) {
                    Some(previous) => {
                        if previous.call_id != record.call_id
                            || previous.tool != record.tool
                            || previous.step != record.step
                            || previous.args != record.args
                        {
                            return Err(ledger_error(format!(
                                "{} changed the call identity at line {line_number}",
                                path.display()
                            )));
                        }
                        if !previous.acknowledged {
                            return Err(ledger_error(format!(
                                "{} starts a retry without an acknowledgement at line {line_number}",
                                path.display()
                            )));
                        }
                        if previous.ok == Some(true) {
                            return Err(ledger_error(format!(
                                "{} retries an attempt that completed successfully at line {line_number}",
                                path.display()
                            )));
                        }
                        previous.attempt = record.attempt;
                        previous.open = true;
                        previous.ok = None;
                        previous.acknowledged = false;
                        previous.replayed = false;
                    }
                    None => {
                        effects.insert(
                            record.effect_id.clone(),
                            TrackedAttempt {
                                attempt: record.attempt,
                                call_id: record.call_id.clone(),
                                tool: record.tool.clone(),
                                step: record.step.clone(),
                                args: record.args.clone(),
                                open: true,
                                ok: None,
                                acknowledged: false,
                                replayed: false,
                            },
                        );
                    }
                }
            }
            ToolEffectPhase::Finished => {
                if record.ok.is_none() {
                    return Err(ledger_error(format!(
                        "{} has a finished record without an outcome at line {line_number}",
                        path.display()
                    )));
                }
                if record.ok == Some(true) && record.error.is_some() {
                    return Err(ledger_error(format!(
                        "{} has an error on a successful finish at line {line_number}",
                        path.display()
                    )));
                }
                if record.acknowledged_by.is_some() || record.acknowledged_reason.is_some() {
                    return Err(ledger_error(format!(
                        "{} has acknowledgement evidence on a finished record at line {line_number}",
                        path.display()
                    )));
                }
                let Some(state) = effects.get_mut(record.effect_id.as_str()) else {
                    return Err(ledger_error(format!(
                        "{} has a finished record without a start at line {line_number}",
                        path.display()
                    )));
                };
                if state.attempt != record.attempt {
                    return Err(ledger_error(format!(
                        "{} finishes the wrong attempt at line {line_number}",
                        path.display()
                    )));
                }
                if !state.open {
                    return Err(ledger_error(format!(
                        "{} finishes one attempt twice at line {line_number}",
                        path.display()
                    )));
                }
                if state.call_id != record.call_id || state.tool != record.tool {
                    return Err(ledger_error(format!(
                        "{} changed the call identity at line {line_number}",
                        path.display()
                    )));
                }
                state.open = false;
                state.ok = record.ok;
            }
            ToolEffectPhase::Acknowledged => {
                if record.ok.is_some() || record.error.is_some() {
                    return Err(ledger_error(format!(
                        "{} has an outcome field on an acknowledged record at line {line_number}",
                        path.display()
                    )));
                }
                if record
                    .acknowledged_by
                    .as_deref()
                    .is_none_or(|by| by.trim().is_empty())
                {
                    return Err(ledger_error(format!(
                        "{} has no acknowledgement actor at line {line_number}",
                        path.display()
                    )));
                }
                let Some(state) = effects.get_mut(record.effect_id.as_str()) else {
                    return Err(ledger_error(format!(
                        "{} acknowledges an effect without a start at line {line_number}",
                        path.display()
                    )));
                };
                if state.attempt != record.attempt {
                    return Err(ledger_error(format!(
                        "{} acknowledges the wrong attempt at line {line_number}",
                        path.display()
                    )));
                }
                if state.acknowledged {
                    return Err(ledger_error(format!(
                        "{} acknowledges one attempt twice at line {line_number}",
                        path.display()
                    )));
                }
                state.acknowledged = true;
            }
            ToolEffectPhase::Replayed => {
                if record.ok.is_some()
                    || record.error.is_some()
                    || record.acknowledged_by.is_some()
                    || record.acknowledged_reason.is_some()
                {
                    return Err(ledger_error(format!(
                        "{} has an outcome field on a replayed record at line {line_number}",
                        path.display()
                    )));
                }
                let Some(state) = effects.get_mut(record.effect_id.as_str()) else {
                    return Err(ledger_error(format!(
                        "{} replays an effect without a start at line {line_number}",
                        path.display()
                    )));
                };
                if state.attempt != record.attempt || state.ok != Some(true) || state.open {
                    return Err(ledger_error(format!(
                        "{} replays an attempt that did not complete at line {line_number}",
                        path.display()
                    )));
                }
                if state.replayed {
                    return Err(ledger_error(format!(
                        "{} replays one attempt twice at line {line_number}",
                        path.display()
                    )));
                }
                state.replayed = true;
            }
        }
        records.push(record);
    }
    Ok(records)
}

/// Append-only writer for one run's ledger. One per host; appends are
/// serialized under a mutex **and** a per-run lock file held across the read,
/// the write and the sync.
pub struct ToolEffectLedger {
    path: PathBuf,
    lock_path: PathBuf,
    run_id: String,
    file: Mutex<fs::File>,
}

impl ToolEffectLedger {
    /// Open (or create) the ledger for a run under its checked run directory.
    ///
    /// # Errors
    /// Invalid run id, untrusted path, or a corrupt existing ledger.
    pub fn open_for_run(workspace: &Path, run_id: &RunId) -> Result<Self, WorkflowError> {
        let dir = checked_run_dir(workspace, run_id, true).map_err(ledger_error)?;
        Self::open_path(dir.join(TOOL_EFFECTS_FILE), run_id.as_str().to_owned())
    }

    /// Open (or create) a ledger at an explicit path already confined to the
    /// run directory by the caller.
    ///
    /// # Errors
    /// Untrusted path, unreadable existing records, or an I/O failure.
    pub fn open_path(path: PathBuf, run_id: String) -> Result<Self, WorkflowError> {
        load_records_at(&path, &run_id)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                ledger_error(format!("cannot create {}: {source}", parent.display()))
            })?;
        }
        check_file(&path).map_err(ledger_error)?;
        let existed = path.is_file();
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| ledger_error(format!("cannot open {}: {source}", path.display())))?;
        let lock_path = path.with_file_name(lock_file_name(&path));
        check_file(&lock_path).map_err(ledger_error)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| {
                ledger_error(format!("cannot open {}: {source}", lock_path.display()))
            })?;
        drop(lock);
        if !existed {
            sync_directory(&path);
        }
        Ok(Self {
            path,
            lock_path,
            run_id,
            file: Mutex::new(file),
        })
    }

    /// The ledger path (for diagnostics; never printed with arguments).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Resolve and durably record the start of one tool call.
    ///
    /// On [`EffectBegin::Execute`] a `started` record is durable when this
    /// returns, so the caller may run the tool. On
    /// [`EffectBegin::AlreadyCompleted`] the same call already finished
    /// successfully; the caller must not run it again.
    ///
    /// # Errors
    /// I/O failure, a corrupt ledger, a changed argument summary for a known
    /// call id, or an earlier attempt that is still unfinished or failed
    /// without an acknowledgement. The caller must not execute the tool.
    pub fn begin_effect(
        &self,
        step: Option<&ToolEffectStep>,
        call_id: &str,
        tool: &str,
        args: Value,
    ) -> Result<EffectBegin, WorkflowError> {
        let effect_id = stable_effect_id(&self.run_id, step, call_id, tool);
        let expected = effect_id.clone();
        self.transact(move |records, outgoing| {
            let existing = effect_states(records)
                .into_iter()
                .find(|state| state.effect_id == expected);
            let Some(state) = existing else {
                outgoing.push(ToolEffectRecord {
                    seq: 0,
                    run_id: String::new(),
                    effect_id: effect_id.clone(),
                    step: step.cloned(),
                    call_id: call_id.to_owned(),
                    tool: tool.to_owned(),
                    args: Some(args.clone()),
                    phase: ToolEffectPhase::Started,
                    attempt: 1,
                    ok: None,
                    error: None,
                    acknowledged_by: None,
                    acknowledged_reason: None,
                });
                return Ok(EffectBegin::Execute {
                    effect_id,
                    attempt: 1,
                });
            };
            if state.call_id != call_id
                || state.tool != tool
                || state.step.as_ref() != step
                || state.args.as_ref() != Some(&args)
            {
                return Err(ledger_error(format!(
                    "effect {effect_id} was recorded with different call arguments; refusing to reinterpret it"
                )));
            }
            match state.outcome {
                EffectOutcome::Finished { ok: true } => {
                    if !state.replayed {
                        outgoing.push(ToolEffectRecord {
                            seq: 0,
                            run_id: String::new(),
                            effect_id: effect_id.clone(),
                            step: state.step.clone(),
                            call_id: call_id.to_owned(),
                            tool: tool.to_owned(),
                            args: None,
                            phase: ToolEffectPhase::Replayed,
                            attempt: state.attempt,
                            ok: None,
                            error: None,
                            acknowledged_by: None,
                            acknowledged_reason: None,
                        });
                    }
                    Ok(EffectBegin::AlreadyCompleted {
                        effect_id,
                        attempt: state.attempt,
                    })
                }
                EffectOutcome::Open if !state.acknowledged => Err(ledger_error(format!(
                    "effect {effect_id} attempt {} has no finished record; refusing to run it again",
                    state.attempt
                ))),
                EffectOutcome::Finished { ok: false } if !state.acknowledged => {
                    Err(ledger_error(format!(
                        "effect {effect_id} attempt {} failed; refusing to retry it without an acknowledgement",
                        state.attempt
                    )))
                }
                _ => {
                    let attempt = state.attempt.checked_add(1).ok_or_else(|| {
                        ledger_error(format!("effect {effect_id} exhausted its attempts"))
                    })?;
                    outgoing.push(ToolEffectRecord {
                        seq: 0,
                        run_id: String::new(),
                        effect_id: effect_id.clone(),
                        step: state.step.clone(),
                        call_id: call_id.to_owned(),
                        tool: tool.to_owned(),
                        args: Some(args.clone()),
                        phase: ToolEffectPhase::Started,
                        attempt,
                        ok: None,
                        error: None,
                        acknowledged_by: None,
                        acknowledged_reason: None,
                    });
                    Ok(EffectBegin::Execute { effect_id, attempt })
                }
            }
        })
    }

    /// Append and sync the matching `finished` record.
    ///
    /// # Errors
    /// I/O failure or a ledger that does not prove this exact attempt open.
    /// The tool already ran, so the caller must surface an
    /// unconfirmed-completion error and let the next preflight fail closed.
    pub fn record_finished(
        &self,
        effect_id: &str,
        attempt: u32,
        call_id: &str,
        tool: &str,
        ok: bool,
    ) -> Result<(), WorkflowError> {
        self.transact(|records, outgoing| {
            let state = effect_states(records)
                .into_iter()
                .find(|state| state.effect_id == effect_id)
                .ok_or_else(|| ledger_error(format!("effect {effect_id} has no start")))?;
            if state.attempt != attempt || state.outcome != EffectOutcome::Open {
                return Err(ledger_error(format!(
                    "effect {effect_id} attempt {attempt} is not open"
                )));
            }
            if state.call_id != call_id || state.tool != tool {
                return Err(ledger_error(format!(
                    "effect {effect_id} changed its call identity"
                )));
            }
            outgoing.push(ToolEffectRecord {
                seq: 0,
                run_id: String::new(),
                effect_id: effect_id.to_owned(),
                step: state.step.clone(),
                call_id: call_id.to_owned(),
                tool: tool.to_owned(),
                args: None,
                phase: ToolEffectPhase::Finished,
                attempt,
                ok: Some(ok),
                error: (!ok).then_some("tool reported an error".to_owned()),
                acknowledged_by: None,
                acknowledged_reason: None,
            });
            Ok(())
        })
    }

    /// Append one `acknowledged` record per still-risky attempt in `ids`.
    ///
    /// The record names the durable step/ask, the actor and the reason. It is
    /// idempotent: an already acknowledged attempt is skipped. An id the
    /// ledger does not contain is refused, because the evidence cannot be
    /// matched to a concrete effect.
    ///
    /// # Errors
    /// I/O failure, a corrupt ledger, or an unknown effect id.
    pub fn acknowledge_effects(
        &self,
        ids: &std::collections::BTreeSet<String>,
        acknowledged_by: &str,
        reason: &str,
    ) -> Result<usize, WorkflowError> {
        if acknowledged_by.trim().is_empty() {
            return Err(ledger_error("acknowledgement needs a non-empty actor"));
        }
        self.transact(|records, outgoing| {
            let states = effect_states(records);
            for id in ids {
                let state = states
                    .iter()
                    .find(|state| &state.effect_id == id)
                    .ok_or_else(|| {
                        ledger_error(format!(
                            "cannot acknowledge effect {id}: the ledger does not contain it"
                        ))
                    })?;
                if state.acknowledged {
                    continue;
                }
                outgoing.push(ToolEffectRecord {
                    seq: 0,
                    run_id: String::new(),
                    effect_id: id.clone(),
                    step: state.step.clone(),
                    call_id: state.call_id.clone(),
                    tool: state.tool.clone(),
                    args: None,
                    phase: ToolEffectPhase::Acknowledged,
                    attempt: state.attempt,
                    ok: None,
                    error: None,
                    acknowledged_by: Some(acknowledged_by.to_owned()),
                    acknowledged_reason: Some(reason.to_owned()),
                });
            }
            Ok(outgoing.len())
        })
    }

    /// Run one read/modify/append transaction under the per-run lock.
    fn transact<T>(
        &self,
        operation: impl FnOnce(
            &[ToolEffectRecord],
            &mut Vec<ToolEffectRecord>,
        ) -> Result<T, WorkflowError>,
    ) -> Result<T, WorkflowError> {
        let _lock = RunScopedLock::acquire(&self.lock_path)?;
        let existing = load_records_at(&self.path, &self.run_id)?;
        let mut outgoing = Vec::new();
        let outcome = operation(&existing, &mut outgoing)?;
        let mut next_seq = existing
            .last()
            .map_or(1, |record| record.seq.saturating_add(1));
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for mut record in outgoing {
            record.seq = next_seq;
            record.run_id.clone_from(&self.run_id);
            let mut line = serde_json::to_vec(&record).map_err(ledger_error)?;
            line.push(b'\n');
            file.write_all(&line)
                .and_then(|()| file.sync_all())
                .map_err(|source| {
                    ledger_error(format!(
                        "cannot append to {}: {source}",
                        self.path.display()
                    ))
                })?;
            next_seq = next_seq.saturating_add(1);
        }
        Ok(outcome)
    }
}

/// The per-run append lock, released on drop.
struct RunScopedLock {
    _file: fs::File,
}

impl RunScopedLock {
    fn acquire(path: &Path) -> Result<Self, WorkflowError> {
        check_file(path).map_err(ledger_error)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|source| ledger_error(format!("cannot open {}: {source}", path.display())))?;
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(LOCK_POLL);
                }
                Err(error) => {
                    return Err(ledger_error(format!(
                        "cannot lock {}: {error}",
                        path.display()
                    )))
                }
            }
        }
    }
}

/// Make the directory entry durable. Unix only; Windows has no userspace
/// directory `fsync`, which stays an exposed window (see the module docs).
fn sync_directory(path: &Path) {
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// A redacted argument summary for one side-effecting tool call.
///
/// Paths and sizes only: never file bodies, commands, keys or hashes of
/// content. Unknown tools report a size only.
#[must_use]
pub fn redacted_tool_arguments(tool: &str, arguments: &Value) -> Value {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(truncate_summary)
    };
    let bytes = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map_or(0, str::len)
    };
    match tool {
        "write_file" => serde_json::json!({
            "path": text("path"),
            "content_bytes": bytes("content"),
        }),
        "edit_file" => serde_json::json!({
            "path": text("path"),
            "old_text_bytes": bytes("old_text"),
            "new_text_bytes": bytes("new_text"),
            "replace_all": arguments.get("replace_all").and_then(Value::as_bool),
        }),
        "delete_file" => serde_json::json!({
            "path": text("path"),
        }),
        // Commands can carry secrets in an argument or redirect. Record shape,
        // not text.
        "bash" => serde_json::json!({
            "command_bytes": bytes("command"),
            "command_lines": arguments
                .get("command")
                .and_then(Value::as_str)
                .map_or(0, |command| command.lines().count()),
        }),
        _ => serde_json::json!({
            "argument_bytes": serde_json::to_string(arguments).map_or(0, |text| text.len()),
        }),
    }
}

fn truncate_summary(path: &str) -> String {
    if path.chars().count() <= MAX_SUMMARY_PATH_CHARS {
        return path.to_owned();
    }
    let mut short: String = path.chars().take(MAX_SUMMARY_PATH_CHARS).collect();
    short.push('…');
    short
}

/// Whether a tool name is side-effecting in a workflow ask.
#[must_use]
pub fn is_side_effecting_tool(tool: &str) -> bool {
    matches!(tool, "write_file" | "edit_file" | "delete_file" | "bash")
}
