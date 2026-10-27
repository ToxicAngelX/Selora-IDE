//! Durable execution and deterministic, index-keyed replay.
//!
//! Completed calls replay without host IO. An interrupted `world.run` is
//! ambiguous and cannot resume: its effects may already have happened.
//! Unfinished asks are dispatched again through the host with fresh approval
//! slots. Tools inside those asks may have partially applied effects before
//! interruption; renewed approval is not an exactly-once guarantee. Resume is
//! therefore fail-closed: when any ask would be dispatched again and
//! [`RunOptions::allow_incomplete_asks`] is false, the run is refused before
//! the host is called or the journal is appended to. [`incomplete_asks`] is
//! the shell's preflight for the confirmation that flag represents.
//! Recorded usage includes failed turns. A process crash before the host returns
//! its receipt cannot recover that unreported charge from this journal.
//!
//! Path checks reject existing symlinks/reparse points. As with ordinary
//! workspace IO, callers must exclude concurrent hostile filesystem mutation;
//! portable safe Rust path APIs cannot eliminate check/open races.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// Execute a confirmed workflow with durable, deterministic replay. Blocking.
///
/// Completed runs cannot be resumed. A run whose confirmed cost cap already
/// fired ([`RunStatus::BudgetExceeded`]) cannot resume either: the grant is
/// spent, resume never raises it silently, and the refusal happens before any
/// host call or journal append. Continuing needs a new confirmed run and
/// budget. Failed or stopped runs can resume, except when an interrupted
/// command has an ambiguous side effect. Unfinished asks are new host dispatches
/// and require fresh approval slots; their tools may already have partially
/// applied effects before interruption. A resume that would dispatch one again
/// is refused with [`WorkflowError::IncompleteAsks`] unless
/// [`RunOptions::allow_incomplete_asks`] is true, and the refusal happens before
/// any host call or journal append.
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

#[derive(Clone, Serialize, Deserialize)]
struct CallStart {
    step: WorkflowStep,
    inputs: Value,
}

#[derive(Clone, Serialize, Deserialize)]
struct CallFinish {
    step: WorkflowStep,
    value: Value,
    cost_usd: f64,
    tokens: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_agent_state"
    )]
    agent_state: Option<Value>,
}

// Preserve an explicitly supplied JSON null so the host can reject malformed
// checkpoints. A missing field alone means the older host had no checkpoint.
fn deserialize_agent_state<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

struct SavedCall {
    start: CallStart,
    finish: Option<CallFinish>,
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
                let start: CallStart =
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
                let finish: CallFinish =
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
        // Fail closed before any host call: a resume that would ask again may
        // re-run tools whose earlier effects already landed.
        if options.resume && !options.allow_incomplete_asks {
            let pending = incomplete_asks(&options.workspace_root, &options.run_id)?;
            if !pending.is_empty() {
                return Err(WorkflowError::IncompleteAsks {
                    count: pending.len(),
                });
            }
        }
        if !host.cost_usd().is_finite() || host.cost_usd() < 0.0 {
            return Err(persistence_error(
                "host usage counter must be finite and non-negative",
            ));
        }
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
    ) -> Result<(), WorkflowError> {
        let start = CallStart {
            step: step.clone(),
            inputs,
        };
        self.append(
            "engine_step_started",
            serde_json::to_value(&start).map_err(persistence_error)?,
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
        host: &dyn Host,
    ) -> Result<(), WorkflowError> {
        let cost = host.cost_usd();
        let tokens = host.tokens();
        if !cost.is_finite() || cost < self.host_cost || tokens < self.host_tokens {
            return Err(persistence_error(
                "host usage counters must be finite and monotonic",
            ));
        }
        let finish = CallFinish {
            step: step.clone(),
            value: value.unwrap_or(Value::Null),
            cost_usd: cost - self.host_cost,
            tokens: tokens - self.host_tokens,
            agent_state,
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
