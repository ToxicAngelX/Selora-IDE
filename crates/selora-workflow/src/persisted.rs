//! Durable execution and deterministic, index-keyed replay.
//!
//! Completed calls replay without host IO. An interrupted `world.run` is
//! ambiguous and cannot resume: its effects may already have happened.
//! A per-run tool-effect ledger (`.selora/workflow-runs/{run_id}/tool-effects.jsonl`)
//! records `started`/`finished` pairs around side-effecting ask tools. A
//! `started` attempt without `finished`, a failed attempt, or an effect owned by
//! an unfinished ask makes the run non-resumable until
//! [`RunOptions::acknowledge_partial_effects`] accepts that effects may already
//! exist. That option first appends durable `acknowledged` evidence per effect
//! (step/ask, actor, reason) through [`acknowledge_partial_effects`], then the
//! preflight reads it back. A corrupt ledger is a typed error, never empty
//! evidence. [`partial_effects`] is the shell's preflight.
//!
//! Effect ids are stable (run, step/ask, model call id, tool). When a resumed
//! ask re-issues a call id the ledger proves completed, the host replays it
//! (one `replayed` record, no second execution) instead of running the tool
//! again. A retry of an unfinished or failed attempt needs that attempt's
//! acknowledgement. This narrows the duplicate-effect window; it is not
//! exactly-once across a process crash, OS crash or power loss (see
//! [`crate::effects`]).
//! Unfinished asks are dispatched again through the host with fresh approval
//! slots. When any ask would be dispatched again and neither
//! [`RunOptions::allow_incomplete_asks`] nor the partial-effect acknowledgement
//! is set, the run is refused before the host is called or the journal is
//! appended to. [`incomplete_asks`] is the shell's preflight for that flag.
//! Recorded usage includes failed turns. A process crash before the host returns
//! its receipt cannot recover that unreported charge from this journal.
//!
//! Path checks reject existing symlinks/reparse points. As with ordinary
//! workspace IO, callers must exclude concurrent hostile filesystem mutation;
//! portable safe Rust path APIs cannot eliminate check/open races.
//!
//! Record vocabulary (all additive; older journals stay readable):
//! `engine_run_started`, `engine_step_started` (with the phase in effect),
//! `engine_step_finished` (with the phase in effect at completion),
//! `engine_report_item` (one per `report` that reached the host, carrying its
//! run-wide ordinal), `engine_artifact` (full markdown/file content),
//! `engine_run_finished`, and `phase_started`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::effects::{effect_states, EffectOutcome, ToolEffectLedger, ToolEffectRecord};
use crate::journal::{check_file, checked_run_dir, load_verified, Journal, JournalWriter};
use crate::{
    EventSink, Host, RunId, RunStatus, StepKind, WorkflowError, WorkflowOutcome, WorkflowReport,
    WorkflowSpec, WorkflowStep,
};

const SPEC_FILE: &str = "spec.json";

/// Workspace and identity for a durable run. Resume requires the exact confirmed spec.
pub struct RunOptions {
    /// Trusted workspace root; persistence stays below its canonical path.
    pub workspace_root: PathBuf,
    /// Stable identity, reused unchanged for resume.
    pub run_id: RunId,
    /// Replay an existing run rather than creating a new confirmed run.
    pub resume: bool,
    /// Accept that resume may dispatch an `ask` again whose tools already ran.
    ///
    /// Only consulted when `resume` is true and the journal shows an ask
    /// without a successful result. The default (`false`) refuses such a
    /// resume with [`WorkflowError::IncompleteAsks`] before any host call or
    /// journal append, because a re-dispatched ask is not exactly-once.
    pub allow_incomplete_asks: bool,
    /// Accept that resume may replay tool effects the ledger proves may already
    /// have applied: an unfinished or failed attempt, or a side effect owned by
    /// an ask the resume would dispatch again.
    ///
    /// Only consulted when `resume` is true. Default (`false`) refuses with
    /// [`WorkflowError::PartialEffects`] before any host call or journal
    /// append. Set this only from a separate, explicit user action that names
    /// the existing-effect risk; it also accepts the unfinished-ask
    /// re-dispatch, because that ask is the owner of those effects. When set,
    /// durable `acknowledged` records are appended first, so the acceptance is
    /// auditable and permits one retry of each accepted attempt.
    pub acknowledge_partial_effects: bool,
}

/// One ask step a resume would dispatch again.
///
/// A completed (`Ok`) ask is never reported. A failed or interrupted ask is,
/// because resume retries it and its earlier tools may have had effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncompleteAsk {
    /// The durable step identity (`WorkflowStep::index`, as text).
    pub step_id: String,
    /// The named agent that would be asked again.
    pub agent: String,
    /// The step's human-readable label, when it has one.
    pub label: Option<String>,
}

pub(crate) fn persistence_error(error: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::Host(format!("workflow persistence: {error}"))
}

/// Load the exact confirmed spec from a contained run directory.
///
/// # Errors
/// Rejects invalid ids, untrusted paths, missing files, and malformed specs.
pub fn load_workflow_spec(root: &Path, run_id: &RunId) -> Result<WorkflowSpec, WorkflowError> {
    let dir = checked_run_dir(root, run_id, false).map_err(persistence_error)?;
    let path = dir.join(SPEC_FILE);
    check_file(&path).map_err(persistence_error)?;
    check_file(&dir.join(crate::journal::JOURNAL_FILE)).map_err(persistence_error)?;
    check_file(&dir.join("run.lock")).map_err(persistence_error)?;
    let bytes = fs::read(path).map_err(persistence_error)?;
    serde_json::from_slice(&bytes).map_err(persistence_error)
}

/// The asks a resume of this run would dispatch again, in original step order.
///
/// This is the shell's fail-closed preflight: call it before offering a resume,
/// so the user can be told exactly which asks may re-run tools that already had
/// effects. It loads the confirmed spec and the journal through the same
/// validation a resume uses (a strict load that only tolerates a torn tail,
/// plus the engine's identity and sequence checks), so a damaged interior
/// record, a changed confirmation, or any state it cannot prove is a typed
/// error — never an empty success.
///
/// A torn final line (the crash that interrupted the run) is recovered the way
/// the engine recovers it. Asks whose journaled result is `Ok` are not
/// returned; interrupted and failed asks are, because resume retries them.
///
/// # Errors
/// Invalid run id, untrusted path, unreadable or damaged journal, missing or
/// malformed spec, or a journal whose identity does not match that spec.
pub fn incomplete_asks(root: &Path, run_id: &RunId) -> Result<Vec<IncompleteAsk>, WorkflowError> {
    let spec = load_workflow_spec(root, run_id)?;
    let verified = load_verified(root, run_id).map_err(persistence_error)?;
    let scan = scan_journal(&verified.loaded.entries, &spec)?;
    scan.unfinished_asks()
}

/// Durable evidence that a resume may replay already-applied tool effects.
///
/// Non-empty when the run's tool-effect ledger proves at least one of:
///
/// * a `started` attempt has no matching `finished` record (the crash window
///   between the effect landing and its completion append);
/// * an attempt finished with a tool error (a partial effect is possible);
/// * a side effect is owned by an ask the resume would dispatch again (that
///   ask's tools may run a second time);
/// * a side effect cannot be attributed to any known step (fail closed, never
///   guess it is safe).
///
/// Attempts the owner already acknowledged in durable ledger evidence are
/// counted in `acknowledged_effects` and do not block. A successful effect
/// owned by a completed ask is not reported; a failed attempt is, because it
/// may have applied part of its effect. A read-only unfinished ask produces no
/// ledger records at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PartialEffects {
    /// `started` attempts with no matching `finished`.
    pub unfinished_effects: usize,
    /// Attempts that finished with a tool error.
    pub failed_effects: usize,
    /// Side effects owned by an ask the resume would dispatch again.
    pub unfinished_ask_effects: usize,
    /// Side effects whose step identity is unknown to the journal.
    pub unattributed_effects: usize,
    /// Risks the owner already accepted in durable ledger evidence.
    pub acknowledged_effects: usize,
}

impl PartialEffects {
    /// No unacknowledged durable partial-effect evidence.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.unfinished_effects == 0
            && self.failed_effects == 0
            && self.unfinished_ask_effects == 0
            && self.unattributed_effects == 0
    }

    /// Distinct side effects that make a silent resume unsafe.
    #[must_use]
    pub fn risk_count(&self) -> usize {
        self.unfinished_effects
            + self.failed_effects
            + self.unfinished_ask_effects
            + self.unattributed_effects
    }
}

/// Ledger effect ids grouped by why a resume would be unsafe.
#[derive(Debug, Default)]
struct EffectEvidence {
    unfinished: BTreeSet<String>,
    failed: BTreeSet<String>,
    unfinished_ask: BTreeSet<String>,
    unattributed: BTreeSet<String>,
    acknowledged: BTreeSet<String>,
}

impl EffectEvidence {
    /// Every effect id whose risk still needs an acknowledgement.
    fn risky_ids(&self) -> BTreeSet<String> {
        self.unfinished
            .iter()
            .chain(self.failed.iter())
            .chain(self.unfinished_ask.iter())
            .chain(self.unattributed.iter())
            .cloned()
            .collect()
    }
}

/// Append durable `acknowledged` evidence for every risky attempt of this run.
///
/// Called only from an explicit owner action: the `acknowledge_partial_effects`
/// resume option. One record per effect names its durable step/ask, the actor
/// and the reason, so a later reader can audit who accepted which risk. The
/// call is idempotent; already acknowledged attempts are skipped. It writes no
/// record when nothing is risky.
///
/// # Errors
/// Invalid run id, untrusted path, missing or malformed spec, damaged journal,
/// a corrupt/foreign ledger, or an append failure. In every error case no
/// host call happens and the run does not start.
pub fn acknowledge_partial_effects(
    root: &Path,
    run_id: &RunId,
    acknowledged_by: &str,
    reason: &str,
) -> Result<usize, WorkflowError> {
    let spec = load_workflow_spec(root, run_id)?;
    let verified = load_verified(root, run_id).map_err(persistence_error)?;
    let scan = scan_journal(&verified.loaded.entries, &spec)?;
    let records = crate::effects::load_tool_effects(root, run_id)?;
    let ids = scan.effect_evidence(&records).risky_ids();
    if ids.is_empty() {
        return Ok(0);
    }
    ToolEffectLedger::open_for_run(root, run_id)?.acknowledge_effects(&ids, acknowledged_by, reason)
}

/// Read the run's tool-effect ledger against its journal: the shell's
/// partial-effect preflight.
///
/// A missing ledger is empty evidence. A corrupt ledger is a typed
/// [`WorkflowError::ToolEffectLedger`] error, never an empty success. It also
/// runs the same strict spec/journal validation as a resume, so it cannot
/// answer for a run the engine would refuse.
///
/// # Errors
/// Invalid run id, untrusted path, missing or malformed spec, damaged journal,
/// or a corrupt/foreign ledger.
pub fn partial_effects(root: &Path, run_id: &RunId) -> Result<PartialEffects, WorkflowError> {
    let spec = load_workflow_spec(root, run_id)?;
    let verified = load_verified(root, run_id).map_err(persistence_error)?;
    let scan = scan_journal(&verified.loaded.entries, &spec)?;
    if !crate::effects::tool_effects_path(root, run_id).is_file()
        && scan
            .calls
            .values()
            .any(|call| call.start.step.kind == StepKind::Ask)
    {
        return Err(WorkflowError::ToolEffectLedger {
            message: "tool effect ledger is missing for a run with agent asks".into(),
        });
    }
    let records = crate::effects::load_tool_effects(root, run_id)?;
    Ok(scan.partial_effects(&records))
}

/// Execute a confirmed workflow with durable, deterministic replay. Blocking.
///
/// Completed runs cannot be resumed. A run whose confirmed cost cap already
/// fired ([`RunStatus::BudgetExceeded`]) cannot resume either: the grant is
/// spent, resume never raises it silently, and the refusal happens before any
/// host call or journal append. Continuing needs a new confirmed run and
/// budget. Failed or stopped runs can resume, except when an interrupted
/// command has an ambiguous side effect. The tool-effect ledger is consulted
/// first: an unfinished or failed attempt, or an effect owned by an unfinished
/// ask, is refused with [`WorkflowError::PartialEffects`] unless
/// [`RunOptions::acknowledge_partial_effects`] is true (which appends durable
/// per-effect acknowledgement evidence first). A corrupt ledger is
/// [`WorkflowError::ToolEffectLedger`]. Unfinished asks are new host dispatches
/// and require fresh approval slots; their tools may already have partially
/// applied effects before interruption. A resume that would dispatch one again
/// is refused with [`WorkflowError::IncompleteAsks`] unless
/// [`RunOptions::allow_incomplete_asks`] (or the partial-effect
/// acknowledgement) is true. Every refusal happens before any host call or
/// journal append.
///
/// # Errors
/// Returns workflow errors or persistence errors before any unjournaled effect.
pub fn run_workflow_persisted(
    spec: &WorkflowSpec,
    host: Arc<dyn Host>,
    sink: EventSink,
    options: RunOptions,
) -> Result<WorkflowOutcome, WorkflowError> {
    crate::sandbox::validate_spec(spec)?;
    let durable = Durable::open(spec, &options, host.as_ref())?;
    crate::sandbox::run_durable(spec, host, sink, options.run_id, durable)
}

/// One `engine_step_started` record.
///
/// `phase` is the phase marker in effect when the step started. It is additive:
/// journals written before it existed deserialize with `None`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepStartRecord {
    /// The step identity and its initial (running) state.
    pub step: WorkflowStep,
    /// The call's normalized inputs, used to prove replay identity.
    pub inputs: Value,
    /// The current phase name, when the script declared one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

/// One `engine_step_finished` record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepFinishRecord {
    /// The completed step identity.
    pub step: WorkflowStep,
    /// The step's value (`null` when it produced none).
    pub value: Value,
    /// Receipt cost for this step.
    pub cost_usd: f64,
    /// Receipt tokens for this step.
    pub tokens: u64,
    /// A successful ask's checkpoint, when the host supports one.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_agent_state"
    )]
    pub agent_state: Option<Value>,
    /// The current phase name when the step finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

/// One `engine_report_item` record: a `report(item, artifactId?)` that
/// actually reached the host. A report step that failed before the host call
/// leaves no record, so history can never claim it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportItemRecord {
    /// The report step's identity.
    pub step_id: u64,
    /// The phase in effect when the item was reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// The 1-based report ordinal across the run (the live `report-N` id).
    pub index: u64,
    /// The reported JSON item, verbatim.
    pub item: Value,
    /// Optional artifact the item belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
}

/// One `engine_artifact` record: an artifact with its full content, written
/// even when the terminal run report is missing or the step value is `null`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    /// The artifact step's identity.
    pub step_id: u64,
    /// The phase in effect when the artifact was produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// The artifact id, title and markdown/file content.
    pub artifact: crate::Artifact,
}

// Preserve an explicitly supplied JSON null so the host can reject malformed
// checkpoints. A missing field alone means the older host had no checkpoint.
fn deserialize_agent_state<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

struct SavedCall {
    start: StepStartRecord,
    finish: Option<StepFinishRecord>,
}

/// What a durable journal proves once every record has been validated.
struct JournalScan {
    calls: BTreeMap<u64, SavedCall>,
    spent_usd: f64,
    tokens: u64,
    /// The run's terminal record, when the run ended.
    finished: Option<RunStatus>,
    /// At least one `engine_run_started` record matched the confirmed spec.
    confirmed: bool,
}

/// Validate a durable journal the way resume does, without side effects.
///
/// This is the single source of truth for [`incomplete_asks`] and
/// [`Durable::open`]: same record vocabulary, same sequence and identity
/// checks, same confirmation check. Anything it cannot prove is a typed error,
/// so a damaged journal can never look like a run with no work left.
fn scan_journal(
    entries: &[crate::journal::JournalEntry],
    spec: &WorkflowSpec,
) -> Result<JournalScan, WorkflowError> {
    let mut scan = JournalScan {
        calls: BTreeMap::new(),
        spent_usd: 0.0,
        tokens: 0,
        finished: None,
        confirmed: false,
    };
    for (offset, entry) in entries.iter().enumerate() {
        if entry.seq != offset as u64 + 1 {
            return Err(persistence_error("non-contiguous journal sequence"));
        }
        match entry.kind.as_str() {
            "engine_step_started" => {
                let start: StepStartRecord =
                    serde_json::from_value(entry.payload.clone()).map_err(persistence_error)?;
                if let Some(saved) = scan.calls.get(&start.step.index) {
                    if saved.start.inputs != start.inputs || saved.start.step != start.step {
                        return Err(persistence_error("step identity changed in journal"));
                    }
                } else {
                    if start.step.index != scan.calls.len() as u64 + 1 {
                        return Err(persistence_error("non-contiguous step identity"));
                    }
                    scan.calls.insert(
                        start.step.index,
                        SavedCall {
                            start,
                            finish: None,
                        },
                    );
                }
            }
            "engine_step_finished" => {
                let finish: StepFinishRecord =
                    serde_json::from_value(entry.payload.clone()).map_err(persistence_error)?;
                if !finish.cost_usd.is_finite() || finish.cost_usd < 0.0 {
                    return Err(persistence_error("invalid usage delta"));
                }
                if finish.agent_state.is_some()
                    && (finish.step.kind != StepKind::Ask
                        || finish.step.status != crate::StepStatus::Ok
                        || finish.step.agent.is_none())
                {
                    return Err(persistence_error(
                        "agent checkpoint on an invalid or unsuccessful ask",
                    ));
                }
                scan.spent_usd += finish.cost_usd;
                scan.tokens = scan.tokens.saturating_add(finish.tokens);
                let saved = scan
                    .calls
                    .get_mut(&finish.step.index)
                    .ok_or_else(|| persistence_error("finish without start"))?;
                let mut identity = finish.step.clone();
                identity.status = crate::StepStatus::Running;
                identity.error = None;
                if saved.start.step != identity
                    || !matches!(
                        finish.step.status,
                        crate::StepStatus::Ok | crate::StepStatus::Failed
                    )
                {
                    return Err(persistence_error(
                        "invalid completed step identity or value",
                    ));
                }
                if saved
                    .finish
                    .as_ref()
                    .is_some_and(|f| f.step.status == crate::StepStatus::Ok)
                {
                    return Err(persistence_error("duplicate completed step"));
                }
                saved.finish = Some(finish);
            }
            "engine_run_finished" => {
                let report: WorkflowReport =
                    serde_json::from_value(entry.payload.clone()).map_err(persistence_error)?;
                scan.finished = Some(report.status);
            }
            "engine_run_started" => {
                let approved: WorkflowSpec = serde_json::from_value(entry.payload["spec"].clone())
                    .map_err(persistence_error)?;
                let limits: crate::Limits = serde_json::from_value(entry.payload["limits"].clone())
                    .map_err(persistence_error)?;
                if approved != *spec
                    || limits != crate::Limits::default()
                    || entry.payload["version"] != 1
                {
                    return Err(persistence_error(
                        "journal confirmation or engine caps changed",
                    ));
                }
                scan.confirmed = true;
            }
            "engine_report_item" => {
                let _: ReportItemRecord =
                    serde_json::from_value(entry.payload.clone()).map_err(persistence_error)?;
            }
            "engine_artifact" => {
                let _: ArtifactRecord =
                    serde_json::from_value(entry.payload.clone()).map_err(persistence_error)?;
            }
            "phase_started" => {}
            _ => {
                return Err(persistence_error(format!(
                    "unknown durable record {}",
                    entry.kind
                )))
            }
        }
    }
    if !entries.is_empty() && !scan.confirmed {
        return Err(persistence_error("journal has no confirmed run record"));
    }
    Ok(scan)
}

impl JournalScan {
    /// The asks a resume would dispatch again, in original step order.
    ///
    /// Failed asks count: replay only returns a successful result, so a failed
    /// or interrupted ask is the next thing the engine pays for.
    fn unfinished_asks(&self) -> Result<Vec<IncompleteAsk>, WorkflowError> {
        let mut asks = Vec::new();
        for saved in self.calls.values() {
            let step = &saved.start.step;
            if step.kind != StepKind::Ask
                || saved
                    .finish
                    .as_ref()
                    .is_some_and(|finish| finish.step.status == crate::StepStatus::Ok)
            {
                continue;
            }
            let agent = step.agent.clone().ok_or_else(|| {
                persistence_error(format!(
                    "unfinished ask step {} has no agent identity",
                    step.index
                ))
            })?;
            let label = step.detail.trim();
            asks.push(IncompleteAsk {
                step_id: step.index.to_string(),
                agent,
                label: (!label.is_empty()).then(|| label.to_owned()),
            });
        }
        Ok(asks)
    }

    /// Classify the ledger against this journal's proven step identities.
    fn effect_evidence(&self, records: &[ToolEffectRecord]) -> EffectEvidence {
        let unfinished_asks: BTreeSet<u64> = self
            .calls
            .values()
            .filter(|saved| {
                saved.start.step.kind == StepKind::Ask
                    && !saved
                        .finish
                        .as_ref()
                        .is_some_and(|finish| finish.step.status == crate::StepStatus::Ok)
            })
            .map(|saved| saved.start.step.index)
            .collect();
        let mut evidence = EffectEvidence::default();
        for state in effect_states(records) {
            if state.acknowledged {
                // The owner accepted this attempt's risk in durable evidence.
                evidence.acknowledged.insert(state.effect_id);
                continue;
            }
            match state.outcome {
                EffectOutcome::Open => {
                    evidence.unfinished.insert(state.effect_id);
                }
                EffectOutcome::Finished { ok: false } => {
                    evidence.failed.insert(state.effect_id);
                }
                EffectOutcome::Finished { ok: true } => {
                    let known = state.step.as_ref().and_then(|step| {
                        step.step_id
                            .parse::<u64>()
                            .ok()
                            .filter(|index| self.calls.contains_key(index))
                    });
                    match known {
                        Some(index) if unfinished_asks.contains(&index) => {
                            evidence.unfinished_ask.insert(state.effect_id);
                        }
                        Some(_) => {}
                        None => {
                            evidence.unattributed.insert(state.effect_id);
                        }
                    }
                }
            }
        }
        evidence
    }

    /// Count the risks that still need an acknowledgement.
    fn partial_effects(&self, records: &[ToolEffectRecord]) -> PartialEffects {
        let evidence = self.effect_evidence(records);
        PartialEffects {
            unfinished_effects: evidence.unfinished.len(),
            failed_effects: evidence.failed.len(),
            unfinished_ask_effects: evidence.unfinished_ask.len(),
            unattributed_effects: evidence.unattributed.len(),
            acknowledged_effects: evidence.acknowledged.len(),
        }
    }
}

pub(crate) struct Durable {
    _lock: fs::File,
    writer: JournalWriter,
    calls: BTreeMap<u64, SavedCall>,
    pub(crate) spent_usd: f64,
    pub(crate) tokens: u64,
    host_cost: f64,
    host_tokens: u64,
}

impl Durable {
    fn open(
        spec: &WorkflowSpec,
        options: &RunOptions,
        host: &dyn Host,
    ) -> Result<Self, WorkflowError> {
        let dir = checked_run_dir(&options.workspace_root, &options.run_id, !options.resume)
            .map_err(persistence_error)?;
        let path = dir.join(SPEC_FILE);
        check_file(&path).map_err(persistence_error)?;
        let lock_path = dir.join("run.lock");
        check_file(&lock_path).map_err(persistence_error)?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(persistence_error)?;
        lock.try_lock().map_err(persistence_error)?;
        // The explicit acknowledgement is written as durable per-effect
        // evidence before the preflight reads it back. One record names each
        // risky attempt's step/ask, actor and reason. A failed append starts
        // nothing.
        if options.resume && options.acknowledge_partial_effects {
            acknowledge_partial_effects(
                &options.workspace_root,
                &options.run_id,
                "workflow resume",
                "owner accepted that earlier tool effects may already exist",
            )?;
        }
        // Fail closed before any host call: a resume may replay tools whose
        // earlier effects already landed. The ledger is the durable evidence.
        if options.resume {
            let partial = partial_effects(&options.workspace_root, &options.run_id)?;
            if !partial.is_empty() && !options.acknowledge_partial_effects {
                return Err(WorkflowError::PartialEffects {
                    effects: partial.risk_count(),
                });
            }
            if !options.allow_incomplete_asks && !options.acknowledge_partial_effects {
                let pending = incomplete_asks(&options.workspace_root, &options.run_id)?;
                if !pending.is_empty() {
                    return Err(WorkflowError::IncompleteAsks {
                        count: pending.len(),
                    });
                }
            }
        }
        if !host.cost_usd().is_finite() || host.cost_usd() < 0.0 {
            return Err(persistence_error(
                "host usage counter must be finite and non-negative",
            ));
        }
        if options.resume {
            if load_workflow_spec(&options.workspace_root, &options.run_id)? != *spec {
                return Err(persistence_error(
                    "resume spec differs from confirmed script, grants, name or cost cap",
                ));
            }
        } else {
            // create_new prevents overwriting another run or following a pre-existing link.
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(persistence_error)?;
            file.write_all(&serde_json::to_vec(spec).map_err(persistence_error)?)
                .and_then(|()| file.sync_all())
                .map_err(persistence_error)?;
            crate::effects::ToolEffectLedger::open_for_run(
                &options.workspace_root,
                &options.run_id,
            )?;
        }
        let loaded = Journal::load(&options.workspace_root, options.run_id.clone())
            .map_err(persistence_error)?;
        if !options.resume && !loaded.entries.is_empty() {
            return Err(persistence_error("run already has journal records"));
        }
        let scan = scan_journal(&loaded.entries, spec)?;
        if scan.finished == Some(RunStatus::Completed) {
            return Err(persistence_error("completed run cannot resume"));
        }
        if scan.finished == Some(RunStatus::BudgetExceeded) {
            // The confirmed cap already fired. Resume never raises the grant:
            // a budget that is exhausted needs a new confirmed run and budget.
            return Err(WorkflowError::BudgetExceeded {
                limit: spec.max_cost_usd,
                spent: scan.spent_usd,
            });
        }
        let calls = scan.calls;
        if calls.values().any(|call| {
            call.start.step.kind == StepKind::WorldRun
                && call
                    .finish
                    .as_ref()
                    .is_none_or(|finish| finish.step.status != crate::StepStatus::Ok)
        }) {
            return Err(persistence_error(
                "interrupted world.run has ambiguous side effects; refusing to execute it again",
            ));
        }
        let writer = Journal::open(&options.workspace_root, options.run_id.clone())
            .map_err(persistence_error)?;
        if !scan.spent_usd.is_finite() {
            return Err(persistence_error("invalid historical spend"));
        }
        let mut durable = Self {
            _lock: lock,
            writer,
            calls,
            spent_usd: scan.spent_usd,
            tokens: scan.tokens,
            host_cost: host.cost_usd(),
            host_tokens: host.tokens(),
        };
        durable.append(
            "engine_run_started",
            serde_json::json!({"resume": options.resume, "spec": spec, "limits": crate::Limits::default(), "version": 1}),
        )?;
        Ok(durable)
    }

    pub(crate) fn append(&mut self, kind: &str, value: Value) -> Result<(), WorkflowError> {
        self.writer
            .append_raw(kind, value)
            .map_err(persistence_error)?;
        Ok(())
    }

    pub(crate) fn replay(
        &self,
        step: &WorkflowStep,
        inputs: &Value,
    ) -> Result<Option<Value>, WorkflowError> {
        if let Some(saved) = self.calls.get(&step.index) {
            if saved.start.step != *step || saved.start.inputs != *inputs {
                return Err(persistence_error(format!(
                    "step {} inputs or identity changed",
                    step.index
                )));
            }
            return Ok(saved
                .finish
                .as_ref()
                .filter(|finish| finish.step.status == crate::StepStatus::Ok)
                .map(|finish| finish.value.clone()));
        }
        Ok(None)
    }

    pub(crate) fn start(
        &mut self,
        step: &WorkflowStep,
        inputs: Value,
        phase: Option<String>,
    ) -> Result<(), WorkflowError> {
        let start = StepStartRecord {
            step: step.clone(),
            inputs,
            phase,
        };
        self.append(
            "engine_step_started",
            serde_json::to_value(&start).map_err(persistence_error)?,
        )
    }

    /// Append one report item that reached the host. Called only after the
    /// host accepted it, so the journal never claims a report that did not
    /// happen.
    pub(crate) fn report_item(&mut self, record: &ReportItemRecord) -> Result<(), WorkflowError> {
        self.append(
            "engine_report_item",
            serde_json::to_value(record).map_err(persistence_error)?,
        )
    }

    /// Append one artifact with its full content. Called only after the host
    /// accepted it.
    pub(crate) fn artifact(&mut self, record: &ArtifactRecord) -> Result<(), WorkflowError> {
        self.append(
            "engine_artifact",
            serde_json::to_value(record).map_err(persistence_error)?,
        )
    }

    /// Fetch only a successful ask's checkpoint. Identity probes remain pure;
    /// the facade invokes the host restore exactly once during actual replay.
    pub(crate) fn agent_state(&self, index: u64) -> Option<&Value> {
        self.calls
            .get(&index)
            .and_then(|call| call.finish.as_ref())
            .filter(|finish| {
                finish.step.kind == StepKind::Ask && finish.step.status == crate::StepStatus::Ok
            })
            .and_then(|finish| finish.agent_state.as_ref())
    }

    pub(crate) fn finish(
        &mut self,
        step: &WorkflowStep,
        value: Option<Value>,
        agent_state: Option<Value>,
        phase: Option<String>,
        host: &dyn Host,
    ) -> Result<(), WorkflowError> {
        let cost = host.cost_usd();
        let tokens = host.tokens();
        if !cost.is_finite() || cost < self.host_cost || tokens < self.host_tokens {
            return Err(persistence_error(
                "host usage counters must be finite and monotonic",
            ));
        }
        let finish = StepFinishRecord {
            step: step.clone(),
            value: value.unwrap_or(Value::Null),
            cost_usd: cost - self.host_cost,
            tokens: tokens - self.host_tokens,
            agent_state,
            phase,
        };
        self.append(
            "engine_step_finished",
            serde_json::to_value(&finish).map_err(persistence_error)?,
        )?;
        self.spent_usd += finish.cost_usd;
        self.tokens = self.tokens.saturating_add(finish.tokens);
        self.host_cost = cost;
        self.host_tokens = tokens;
        Ok(())
    }

    pub(crate) fn verify_replayed(&self, steps: u64) -> Result<(), WorkflowError> {
        if self
            .calls
            .last_key_value()
            .is_some_and(|(index, _)| *index > steps)
        {
            return Err(persistence_error(
                "script ended before historical steps were replayed",
            ));
        }
        Ok(())
    }
}
