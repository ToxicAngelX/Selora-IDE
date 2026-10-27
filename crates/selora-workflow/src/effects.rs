//! Per-run tool-effect ledger: durable evidence for a fail-closed resume (P2).
//!
//! A workflow ask may run side-effecting tools (`write_file`, `edit_file`,
//! `delete_file`, `bash`). The engine's journal only proves that an ask
//! *started*; it cannot prove whether the tool's effect landed before a crash.
//! This ledger closes that gap for the resume preflight:
//!
//! * before a side-effecting tool executes, the host appends a `started`
//!   record and flushes it;
//! * after the tool returns (success, error or cancellation), it appends a
//!   matching `finished` record with the same `effect_id`.
//!
//! A crash therefore leaves `started` without `finished`. That is durable
//! evidence of a partial effect, so [`crate::run_workflow_persisted`] refuses
//! to resume without an explicit acknowledgement. A corrupt ledger is a typed
//! error, never treated as empty.
//!
//! # Format
//!
//! `.selora/workflow-runs/{run_id}/tool-effects.jsonl`, one JSON object per
//! line, contiguous `seq` starting at 1. Both records of a pair share
//! `effect_id`; `call_id` is the model's tool-call id. `step` names the durable
//! step/ask when the host knows it. `args` is a redacted summary: paths and
//! byte counts only, never file bodies, command text, or secrets.
//!
//! # Crash windows
//!
//! A process crash between the effect landing and the `finished` append leaves
//! an unmatched `started`, which is exactly the partial-effect evidence this
//! module records. A crash while appending may leave a torn last line; that is
//! corruption and the ledger fails closed. The ledger is written with
//! `flush`, not `sync_all`, so an OS/power loss can still lose the newest
//! record; no portable userspace protocol can close that window.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::journal::{check_file, checked_run_dir};
use crate::{RunId, WorkflowError};

/// The ledger file inside one run directory.
pub const TOOL_EFFECTS_FILE: &str = "tool-effects.jsonl";

/// Longest path kept in a redacted argument summary.
const MAX_SUMMARY_PATH_CHARS: usize = 256;
/// Longest ledger file accepted by the preflight (16 MiB).
const MAX_LEDGER_BYTES: u64 = 16 * 1024 * 1024;

/// The durable step/ask identity that owned a tool call, when known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEffectStep {
    /// `WorkflowStep::index`, as text (matches `IncompleteAsk::step_id`).
    pub step_id: String,
    /// The named workflow agent, when the step is an ask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// One half of a tool effect's lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolEffectPhase {
    /// Appended before the tool executes.
    Started,
    /// Appended after the tool returned (or was cancelled).
    Finished,
}

/// One line of the per-run tool-effect ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEffectRecord {
    /// Contiguous 1-based position in this run's ledger.
    pub seq: u64,
    /// Run identity; a record from another run is corruption, not evidence.
    pub run_id: String,
    /// Shared by the `started`/`finished` pair.
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
    /// Lifecycle half.
    pub phase: ToolEffectPhase,
    /// Present only on `finished`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// Present only on a failed `finished`; a fixed category, no tool output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The path of one run's tool-effect ledger.
#[must_use]
pub fn tool_effects_path(workspace: &Path, run_id: &RunId) -> PathBuf {
    crate::journal::run_dir(workspace, run_id).join(TOOL_EFFECTS_FILE)
}

fn ledger_error(message: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::ToolEffectLedger {
        message: message.to_string(),
    }
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
    let mut started: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut finished: BTreeSet<String> = BTreeSet::new();
    for (offset, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(ledger_error(format!(
                "{} has an empty record at line {}",
                path.display(),
                offset + 1
            )));
        }
        let record: ToolEffectRecord = serde_json::from_str(line).map_err(|error| {
            ledger_error(format!(
                "{} has an unreadable record at line {}: {error}",
                path.display(),
                offset + 1
            ))
        })?;
        if record.seq != offset as u64 + 1 {
            return Err(ledger_error(format!(
                "{} has a non-contiguous sequence at line {}",
                path.display(),
                offset + 1
            )));
        }
        if record.run_id != run_id {
            return Err(ledger_error(format!(
                "{} has a record for another run at line {}",
                path.display(),
                offset + 1
            )));
        }
        if record.effect_id.trim().is_empty()
            || record.call_id.trim().is_empty()
            || record.tool.trim().is_empty()
        {
            return Err(ledger_error(format!(
                "{} has an incomplete record at line {}",
                path.display(),
                offset + 1
            )));
        }
        match record.phase {
            ToolEffectPhase::Started => {
                if record.ok.is_some() || record.error.is_some() {
                    return Err(ledger_error(format!(
                        "{} has a finished field on a started record at line {}",
                        path.display(),
                        offset + 1
                    )));
                }
                if started
                    .insert(
                        record.effect_id.clone(),
                        (record.call_id.clone(), record.tool.clone()),
                    )
                    .is_some()
                {
                    return Err(ledger_error(format!(
                        "{} repeats an effect id at line {}",
                        path.display(),
                        offset + 1
                    )));
                }
            }
            ToolEffectPhase::Finished => {
                if record.ok.is_none() {
                    return Err(ledger_error(format!(
                        "{} has a finished record without an outcome at line {}",
                        path.display(),
                        offset + 1
                    )));
                }
                let Some((call_id, tool)) = started.get(&record.effect_id) else {
                    return Err(ledger_error(format!(
                        "{} has a finished record without a start at line {}",
                        path.display(),
                        offset + 1
                    )));
                };
                if *call_id != record.call_id || *tool != record.tool {
                    return Err(ledger_error(format!(
                        "{} changed the call identity at line {}",
                        path.display(),
                        offset + 1
                    )));
                }
                if !finished.insert(record.effect_id.clone()) {
                    return Err(ledger_error(format!(
                        "{} finishes one effect twice at line {}",
                        path.display(),
                        offset + 1
                    )));
                }
            }
        }
        records.push(record);
    }
    Ok(records)
}

/// Append-only writer for one run's ledger. One per host; the write is
/// serialized under a mutex and flushed before the caller runs the tool.
pub struct ToolEffectLedger {
    path: PathBuf,
    run_id: String,
    writer: Mutex<LedgerWriter>,
}

struct LedgerWriter {
    file: fs::File,
    next_seq: u64,
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
        let existing = load_records_at(&path, &run_id)?;
        let next_seq = existing.last().map_or(1, |record| record.seq + 1);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                ledger_error(format!("cannot create {}: {source}", parent.display()))
            })?;
        }
        check_file(&path).map_err(ledger_error)?;
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| ledger_error(format!("cannot open {}: {source}", path.display())))?;
        Ok(Self {
            path,
            run_id,
            writer: Mutex::new(LedgerWriter { file, next_seq }),
        })
    }

    /// The ledger path (for diagnostics; never printed with arguments).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append and flush a `started` record. Returns the shared `effect_id`.
    ///
    /// # Errors
    /// I/O failure. The caller must not execute the tool.
    pub fn record_started(
        &self,
        step: Option<&ToolEffectStep>,
        call_id: &str,
        tool: &str,
        args: Value,
    ) -> Result<String, WorkflowError> {
        let effect_id = uuid::Uuid::new_v4().to_string();
        self.append(ToolEffectRecord {
            seq: 0,
            run_id: self.run_id.clone(),
            effect_id: effect_id.clone(),
            step: step.cloned(),
            call_id: call_id.to_owned(),
            tool: tool.to_owned(),
            args: Some(args),
            phase: ToolEffectPhase::Started,
            ok: None,
            error: None,
        })?;
        Ok(effect_id)
    }

    /// Append and flush the matching `finished` record.
    ///
    /// # Errors
    /// I/O failure. The tool already ran, so the caller must surface an
    /// unconfirmed-completion error and let the next preflight fail closed.
    pub fn record_finished(
        &self,
        effect_id: &str,
        call_id: &str,
        tool: &str,
        ok: bool,
    ) -> Result<(), WorkflowError> {
        self.append(ToolEffectRecord {
            seq: 0,
            run_id: self.run_id.clone(),
            effect_id: effect_id.to_owned(),
            step: None,
            call_id: call_id.to_owned(),
            tool: tool.to_owned(),
            args: None,
            phase: ToolEffectPhase::Finished,
            ok: Some(ok),
            error: (!ok).then_some("tool reported an error".to_owned()),
        })
    }

    fn append(&self, mut record: ToolEffectRecord) -> Result<(), WorkflowError> {
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        record.seq = writer.next_seq;
        let mut line = serde_json::to_vec(&record).map_err(ledger_error)?;
        line.push(b'\n');
        writer
            .file
            .write_all(&line)
            .and_then(|()| writer.file.flush())
            .map_err(|source| {
                ledger_error(format!(
                    "cannot append to {}: {source}",
                    self.path.display()
                ))
            })?;
        writer.next_seq += 1;
        Ok(())
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
