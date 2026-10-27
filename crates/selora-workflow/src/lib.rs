//! Selora Workflows — the engine core (ARCHITECTURE.md §18, ROADMAP.md 8a).
//!
//! A workflow is a small JavaScript script, authored by the chat agent (via
//! the `run_workflow` tool) or pasted by the user, that orchestrates
//! **subagents**, deterministic command gates, and journaled world reads. This
//! crate is the trusted half of that feature:
//!
//! * [`sandbox`] builds a boa realm that exposes *exactly* the §18.2 facade and
//!   nothing else — no `fetch`, no `fs`, no `process`, no timers, no `Date`.
//!   The facade is the whole attack surface (§18.1).
//! * [`facade`] is the JS-to-Rust bridge. It talks to the world only through
//!   the blocking [`Host`] trait, validates every `ask` result against the
//!   schema the script declared ([`schema`]), and gates `world.run` on the
//!   command set the user confirmed.
//! * [`schema`] is the JSON-schema-ish validator: a mismatch fails the run
//!   naming the offending field — never a silent coercion.
//! * [`journal`], [`budget`] and [`subagent`] are the 8b seams (stubs here).
//!   Durable engine execution is available through [`run_workflow_persisted`];
//!   [`load_workflow_spec`] loads its exact confirmed spec for resume.
//!
//! # Threading
//!
//! Nothing in this crate is async. [`run_workflow`] is a **blocking** call and
//! must run on a `tokio::task::spawn_blocking` worker (ARCHITECTURE.md §18.4),
//! never on the UI thread. The boa [`Context`] is created and dropped inside
//! that call, so it never has to cross a thread boundary.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use selora_workflow::{prepare, run_workflow, Host};
//!
//! # fn demo(host: Arc<dyn Host>) -> Result<(), Box<dyn std::error::Error>> {
//! let confirmation = prepare("review", r#"phase("plan"); phase("done");"#, 5.0)?;
//! let spec = confirmation.spec();
//! let events: selora_workflow::EventSink = Arc::new(|_event| { /* forward to the shell */ });
//! let outcome = run_workflow(&spec, host, events)?;
//! assert!(outcome.report.status.is_terminal_ok());
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod budget;
pub mod effects;
pub mod facade;
pub mod journal;
mod persisted;
pub mod sandbox;
pub mod schema;
pub mod subagent;

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use budget::Budget;
pub use effects::{
    is_side_effecting_tool, load_tool_effects, redacted_tool_arguments, tool_effects_path,
    ToolEffectLedger, ToolEffectPhase, ToolEffectRecord, ToolEffectStep,
};
pub use persisted::{
    incomplete_asks, load_workflow_spec, partial_effects, run_workflow_persisted, ArtifactRecord,
    IncompleteAsk, PartialEffects, PhaseUsageRecord, ReportItemRecord, RunOptions,
    StepFinishRecord, StepStartRecord,
};
pub use sandbox::{
    prepare, Confirmation, Limits, DEFAULT_MAX_COST_USD, MAX_SCRIPT_BYTES, MAX_STEPS,
};
pub use schema::{Schema, SchemaError};

/// Identifier of one workflow run. Also the directory name under
/// `.selora/workflow-runs/` (ARCHITECTURE.md §18.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Mint a fresh run id (UUID v4, hyphenated).
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for RunId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for RunId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// The confirmed commands a script may hand to `world.run`.
///
/// Extraction happens statically at confirmation time ([`prepare`]) from the
/// string literals in the script; the runtime re-checks every gate against
/// this set. A command assembled at runtime is therefore never silently
/// granted — it fails loudly (see [`WorkflowError::NonLiteralCommand`] and
/// [`WorkflowError::UnconfirmedCommand`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommandSet(Vec<String>);

impl CommandSet {
    /// Build a command set from anything stringy.
    pub fn new<I, S>(commands: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut set = Vec::new();
        for command in commands {
            let command = command.into();
            if !set.iter().any(|existing| existing == &command) {
                set.push(command);
            }
        }
        Self(set)
    }

    /// Whether `command` was confirmed.
    #[must_use]
    pub fn contains(&self, command: &str) -> bool {
        self.0.iter().any(|existing| existing == command)
    }

    /// Iterate the confirmed commands, in first-appearance order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// The confirmed commands as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    /// Number of confirmed commands.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether nothing was confirmed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<String>> for CommandSet {
    fn from(value: Vec<String>) -> Self {
        Self::new(value)
    }
}

impl FromIterator<String> for CommandSet {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        Self::new(iter)
    }
}

/// A confirmed workflow: the script text, the commands it may run, and the
/// run's cost ceiling. Produced by [`Confirmation::spec`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSpec {
    /// Human-readable run name (the panel's run list).
    pub name: String,
    /// The full script text the user approved.
    pub source: String,
    /// The commands extracted from the script at confirmation time.
    pub command_set: CommandSet,
    /// Workflow-level cost ceiling in USD (`workflow.max_cost_usd`, default $5).
    pub max_cost_usd: f64,
}

impl WorkflowSpec {
    /// A spec with the default cost cap and an empty command set. Callers
    /// normally go through [`prepare`] instead, which extracts the set.
    #[must_use]
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            command_set: CommandSet::default(),
            max_cost_usd: DEFAULT_MAX_COST_USD,
        }
    }
}

/// Run-level lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Created, not started.
    Pending,
    /// Evaluating the script.
    Running,
    /// Parked on an approval card (8b).
    AwaitingApproval,
    /// The script ran to completion.
    Completed,
    /// The script (or the engine contract) failed; see `WorkflowReport::error`.
    Failed,
    /// The user stopped the run.
    Stopped,
    /// The workflow-level cost cap was hit between steps.
    BudgetExceeded,
}

impl RunStatus {
    /// Whether the run finished successfully.
    #[must_use]
    pub fn is_terminal_ok(self) -> bool {
        matches!(self, Self::Completed)
    }

    /// Whether the run will produce no further steps.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Stopped | Self::BudgetExceeded
        )
    }
}

/// One step's lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Recorded but not started.
    Pending,
    /// In flight.
    Running,
    /// Returned normally.
    Ok,
    /// Failed; the run fails with it.
    Failed,
    /// Not run (journal replay / branch not taken).
    Skipped,
}

/// What kind of facade call a step is. One entry per JS-visible call, which is
/// also the unit counted against the 256-step cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// `phase(name)`
    Phase,
    /// `agent(...).ask(...)`
    Ask,
    /// `world.run(...)`
    WorldRun,
    /// `files.glob` / `files.grep` / `files.read`
    Files,
    /// `git.changedFiles` / `git.diff` / `git.status` / `git.log`
    Git,
    /// `report(item, artifactId?)`
    Report,
    /// `artifact.markdown` / `artifact.file`
    Artifact,
    /// `log(message)`
    Log,
}

/// One journaled step — the panel's step row and the journal's unit (8b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// 1-based, monotonic within the run.
    pub index: u64,
    /// Which facade call this is.
    pub kind: StepKind,
    /// The named agent for `ask` steps.
    pub agent: Option<String>,
    /// Lifecycle.
    pub status: StepStatus,
    /// Human-readable label (`cargo test`, `src/**/*.rs`, …).
    pub detail: String,
    /// Failure text, when `status == Failed`.
    pub error: Option<String>,
}

/// A progressive report item (`report(item, artifactId?)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportItem {
    /// 1-based index across the run.
    pub index: u64,
    /// The JSON item the script reported.
    pub item: serde_json::Value,
    /// Optional artifact this item belongs to.
    pub artifact_id: Option<String>,
}

/// What an artifact holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "body")]
pub enum ArtifactContent {
    /// Inline markdown (`artifact.markdown`).
    Markdown(String),
    /// A workspace file the panel renders (`artifact.file`).
    File(String),
}

/// An artifact produced by the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// Script-chosen id, unique within the run.
    pub id: String,
    /// Optional display title.
    pub title: Option<String>,
    /// Markdown or a file path.
    pub content: ArtifactContent,
}

/// The terminal summary of a run — the panel's run view and the journal's tail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowReport {
    /// Run id (also the journal directory).
    pub run_id: RunId,
    /// Run name.
    pub name: String,
    /// Terminal status.
    pub status: RunStatus,
    /// Wall time spent inside the engine, in milliseconds.
    pub elapsed_ms: u64,
    /// Phases in first-marker order (the panel's graph).
    pub phases: Vec<String>,
    /// Every facade step, in order.
    pub steps: Vec<WorkflowStep>,
    /// Every artifact, in creation order.
    pub artifacts: Vec<Artifact>,
    /// Cumulative spend reported by [`Host::cost_usd`].
    pub cost_usd: f64,
    /// Cumulative receipt tokens, including prior attempts on resume.
    #[serde(default)]
    pub tokens: u64,
    /// Failure text when `status` is not `Completed`.
    pub error: Option<String>,
}

/// What a completed run hands back: the report plus the script's completion
/// value (JSON; `null` when the script returns nothing).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowOutcome {
    /// Terminal summary.
    pub report: WorkflowReport,
    /// The script's last-expression value, as JSON.
    pub value: serde_json::Value,
}

/// Progress events, forwarded to the shell (ARCHITECTURE.md §18.4). One
/// `Finished` event per run, always last — including failed runs, so the panel
/// always gets a terminal event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum WorkflowEvent {
    /// The run started.
    Run {
        /// Run id.
        run_id: RunId,
        /// Run name.
        name: String,
    },
    /// A `phase(name)` marker.
    Phase {
        /// Phase name.
        name: String,
    },
    /// A step changed state (`Running`, then `Ok`/`Failed`).
    Step {
        /// The step snapshot.
        step: WorkflowStep,
    },
    /// A `report(item)` item.
    Report {
        /// The reported item.
        item: ReportItem,
    },
    /// An artifact was produced.
    Artifact {
        /// The artifact.
        artifact: Artifact,
    },
    /// A `log(message)` line.
    Log {
        /// The message.
        message: String,
    },
    /// The run ended. Always the last event.
    Finished {
        /// Terminal summary.
        report: WorkflowReport,
    },
}

/// Anything that aborts a run. Every variant is a hard failure: facade errors
/// are not catchable by the script (a `try`/`catch` may swallow the JS
/// exception, but the run still ends with this error recorded).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WorkflowError {
    /// The script is larger than the cap.
    #[error("workflow script is {size} bytes, over the {limit}-byte cap")]
    ScriptTooLarge {
        /// Actual size in bytes.
        size: usize,
        /// Configured cap.
        limit: usize,
    },
    /// More facade calls than the step cap allows.
    #[error("workflow exceeded its {limit}-step cap")]
    StepLimitExceeded {
        /// Configured cap.
        limit: usize,
    },
    /// The script is not valid JavaScript.
    #[error("workflow script is not valid JavaScript: {message}")]
    Syntax {
        /// The engine's parse error.
        message: String,
    },
    /// The script threw, or the engine rejected it at runtime.
    #[error("workflow script failed: {message}")]
    Script {
        /// The engine's exception text.
        message: String,
    },
    /// The script reached for a global the facade does not provide.
    #[error("`{name}` is not available in a workflow script — the facade (agent, phase, world, files, git, report, artifact, log) is the whole API")]
    AmbientGlobal {
        /// The identifier the script used.
        name: String,
    },
    /// `world.run` named a command that was not in the confirmed set.
    #[error("`world.run` command `{command}` was never confirmed — a workflow may only run the commands extracted at approval time")]
    UnconfirmedCommand {
        /// The unconfirmed command.
        command: String,
    },
    /// `world.run`'s command argument is not a literal, so it cannot be
    /// confirmed up front.
    #[error("`world.run` needs a string-literal command (found {found}); a command assembled at runtime cannot be confirmed")]
    NonLiteralCommand {
        /// What the scanner found instead of a literal.
        found: String,
    },
    /// The script passed a malformed `ask` type schema.
    #[error("invalid `ask` type schema: {message}")]
    InvalidSchema {
        /// What is wrong with the schema.
        message: String,
    },
    /// The model's answer did not match the declared schema. Never coerced.
    #[error("`ask` result field `{field}` failed validation: expected {expected}, got {actual}")]
    SchemaMismatch {
        /// JSON-path-ish field location (`$.findings[0].line`).
        field: String,
        /// What the schema declared.
        expected: String,
        /// What actually arrived.
        actual: String,
    },
    /// The workflow-level cost cap was hit between steps.
    #[error("workflow budget of ${limit:.2} exceeded (${spent:.2} spent)")]
    BudgetExceeded {
        /// The cap.
        limit: f64,
        /// Spend reported by the host at the check.
        spent: f64,
    },
    /// A generic cap (report items, artifact bytes, …).
    #[error("{what} exceeds its {limit} cap")]
    LimitExceeded {
        /// What tripped the cap.
        what: String,
        /// The cap.
        limit: usize,
    },
    /// The host refused or failed a facade call.
    #[error("{0}")]
    Host(String),
    /// The spec itself is unusable.
    #[error("invalid workflow: {message}")]
    InvalidSpec {
        /// What is wrong.
        message: String,
    },
    /// The user stopped the run.
    #[error("workflow stopped")]
    Stopped,
    /// A resume would dispatch ask steps again whose earlier tools may have
    /// already applied effects. Explicitly accepted via
    /// [`RunOptions::allow_incomplete_asks`].
    #[error("resume would re-dispatch {count} unfinished ask step(s) whose tools may already have applied effects; set allow_incomplete_asks to accept this risk")]
    IncompleteAsks {
        /// How many ask steps the resume would dispatch again.
        count: usize,
    },
    /// A resume would replay tool effects the ledger cannot prove finished, or
    /// effects that belong to an ask the resume would dispatch again.
    /// Explicitly accepted via [`RunOptions::acknowledge_partial_effects`].
    #[error("resume may replay {effects} tool effect(s) that already applied; set acknowledge_partial_effects to accept this risk")]
    PartialEffects {
        /// How many started effects lack a matching finish.
        effects: usize,
    },
    /// The per-run tool-effect ledger could not be read or trusted. Never
    /// treated as an empty ledger.
    #[error("tool effect ledger: {message}")]
    ToolEffectLedger {
        /// What could not be proven.
        message: String,
    },
    /// The engine could not build or harden its realm (an engine bug, not a
    /// script bug).
    #[error("workflow engine error: {message}")]
    Engine {
        /// What failed.
        message: String,
    },
}

impl From<HostError> for WorkflowError {
    fn from(value: HostError) -> Self {
        Self::Host(value.message)
    }
}

/// A blocking host failure, surfaced by [`Host`] methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    /// Human-readable failure.
    pub message: String,
}

impl HostError {
    /// Construct a host error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HostError {}

/// The result of one `world.run` gate (ARCHITECTURE.md §18.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    /// Process exit code; the script branches on it.
    pub exit_code: i32,
    /// Captured stdout (host-capped).
    pub stdout: String,
    /// Captured stderr (host-capped).
    pub stderr: String,
}

/// One `files.grep` hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepMatch {
    /// Workspace-relative path.
    pub path: String,
    /// 1-based line number.
    pub line: u64,
    /// The matching line's text.
    pub text: String,
}

/// One `git.log` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommit {
    /// Short or full SHA, as the host chooses.
    pub sha: String,
    /// First line of the commit message.
    pub summary: String,
}

/// Everything the engine is allowed to touch in the outside world.
///
/// The trait is **blocking** on purpose: the engine runs on a
/// `spawn_blocking` worker and boa never sees a future (§18.4). The app shell
/// implements this over `selora-ai`'s agent loop, the §17.5 bash spawn,
/// `selora-fs`, and `selora-git`. All methods are `&self`, so implementations
/// are expected to be cheap to share (`Arc<dyn Host>`).
///
/// `files.*` and `git.*` are read-only by contract: there is no write method
/// to implement. Writes inside a subagent still park on approval cards
/// (§18.1); the host's `ask` drives that loop unchanged.
pub trait Host: Send + Sync {
    /// Whether the user has requested Stop. Checked before every facade step.
    fn is_cancelled(&self) -> bool {
        false
    }

    /// Cumulative receipt tokens spent by this host instance.
    fn tokens(&self) -> u64 {
        0
    }

    /// Checkpoint a named agent after a successful, validated turn. Include
    /// the complete conversation, including tool calls and tool results.
    /// Hosts without checkpoint support may return `None`.
    ///
    /// # Errors
    /// Failure to capture the complete agent state aborts the workflow.
    fn snapshot_agent(&self, agent: &str) -> Result<Option<serde_json::Value>, HostError> {
        let _ = agent;
        Ok(None)
    }

    /// Restore a successful ask's checkpoint during replay, before the script
    /// can dispatch another call. Validate the entire snapshot before applying
    /// it; replace the conversation rather than appending duplicate messages.
    ///
    /// # Errors
    /// Reject malformed or incompatible state. A restore failure aborts replay.
    fn restore_agent(&self, agent: &str, snapshot: &serde_json::Value) -> Result<(), HostError> {
        let _ = (agent, snapshot);
        Ok(())
    }

    /// Called before a non-replayed step executes, so a host can attribute
    /// side effects raised by that step (for the durable tool-effect ledger).
    /// Read-only facade calls are included; implementations should only store
    /// the identity. Default is a no-op for hosts that do not need it.
    fn note_step_started(&self, step: &WorkflowStep) {
        let _ = step;
    }

    /// One subagent turn. `agent` is the run-unique identity the script used;
    /// `persona` is the optional persona the script declared. Returns the
    /// model's answer as JSON, which the engine validates against the schema.
    ///
    /// # Errors
    /// Any refusal/failure of the underlying turn.
    fn ask(
        &self,
        agent: &str,
        persona: Option<&str>,
        instructions: &str,
    ) -> Result<serde_json::Value, HostError>;

    /// Like [`Host::ask`] but also hands the declared result schema to the
    /// host, so the app can put the shape in the prompt. Defaults to
    /// [`Host::ask`]; override it when the model needs to see the schema.
    ///
    /// # Errors
    /// Any refusal/failure of the underlying turn.
    fn ask_with_schema(
        &self,
        agent: &str,
        persona: Option<&str>,
        instructions: &str,
        schema: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, HostError> {
        let _ = schema;
        self.ask(agent, persona, instructions)
    }

    /// A deterministic command gate over the §17.5 spawn (env-cleared,
    /// output-capped, killed on timeout). `cmd` is guaranteed to be in the
    /// confirmed command set; a non-zero exit is a value, not an error.
    ///
    /// # Errors
    /// Only when the process could not be run at all.
    fn world_run(
        &self,
        cmd: &str,
        args: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<CommandOutput, HostError>;

    /// Workspace-relative paths matching `pattern`.
    ///
    /// # Errors
    /// Host/IO failure.
    fn files_glob(&self, pattern: &str) -> Result<Vec<String>, HostError>;

    /// Content hits for `pattern`, optionally narrowed to a `glob`.
    ///
    /// # Errors
    /// Host/IO failure.
    fn files_grep(&self, pattern: &str, glob: Option<&str>) -> Result<Vec<GrepMatch>, HostError>;

    /// Read one workspace file. Host-capped; cap-reject, never truncate.
    ///
    /// # Errors
    /// Host/IO failure, oversized file.
    fn files_read(&self, path: &str) -> Result<String, HostError>;

    /// Paths changed against `base` (default: the working tree vs HEAD).
    ///
    /// # Errors
    /// Host/git failure.
    fn git_changed_files(&self, base: Option<&str>) -> Result<Vec<String>, HostError>;

    /// A unified diff, optionally limited to `base` and/or one `path`.
    ///
    /// # Errors
    /// Host/git failure.
    fn git_diff(&self, base: Option<&str>, path: Option<&str>) -> Result<String, HostError>;

    /// `git status` as a human-readable block.
    ///
    /// # Errors
    /// Host/git failure.
    fn git_status(&self) -> Result<String, HostError>;

    /// The most recent `count` commits (host default when `None`).
    ///
    /// # Errors
    /// Host/git failure.
    fn git_log(&self, count: Option<usize>) -> Result<Vec<GitCommit>, HostError>;

    /// A progressive result item, optionally tied to an artifact.
    ///
    /// # Errors
    /// Host failure.
    fn report(&self, item: &serde_json::Value, artifact_id: Option<&str>) -> Result<(), HostError>;

    /// Hand the panel an inline markdown artifact.
    ///
    /// # Errors
    /// Host failure.
    fn artifact_markdown(
        &self,
        id: &str,
        title: Option<&str>,
        content: &str,
    ) -> Result<(), HostError>;

    /// Hand the panel a file artifact.
    ///
    /// # Errors
    /// Host failure.
    fn artifact_file(&self, id: &str, title: Option<&str>, path: &str) -> Result<(), HostError>;

    /// A run-log line.
    fn log(&self, message: &str);

    /// Cumulative USD spent by this run so far, as the cost meter knows it.
    /// Checked against the workflow budget between every step (§18.1).
    fn cost_usd(&self) -> f64 {
        0.0
    }
}

/// Where [`run_workflow`] sends progress: an `Arc<dyn Fn>` so the shell can
/// hand the same sink to several layers (events → `AppMessage::Workflow*` →
/// `notify()`), per ARCHITECTURE.md §18.4.
pub type EventSink = Arc<dyn Fn(WorkflowEvent) + Send + Sync>;

/// Run a confirmed workflow. **Blocking** — call it from
/// `tokio::task::spawn_blocking`, never the UI thread.
///
/// Emits [`WorkflowEvent::Run`] first and exactly one
/// [`WorkflowEvent::Finished`] last once the script has stopped, on success
/// *and* on failure. Pre-flight rejections (size cap, unconfirmed command, bad
/// spec) return before `Run`, so a rejected run emits nothing. Errors abort the
/// run at the failing step.
///
/// # Errors
/// Any [`WorkflowError`] variant: caps, schema mismatches, unconfirmed
/// commands, host failures, budget, script errors.
pub fn run_workflow(
    spec: &WorkflowSpec,
    host: Arc<dyn Host>,
    sink: EventSink,
) -> Result<WorkflowOutcome, WorkflowError> {
    sandbox::run(spec, host, sink)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_set_dedupes_and_matches_exactly() {
        let set = CommandSet::new(["cargo", "cargo", "git"]);
        assert_eq!(set.as_slice(), ["cargo", "git"]);
        assert!(set.contains("cargo"));
        assert!(!set.contains("Cargo"));
        assert!(!set.contains("cargo test"));
    }

    #[test]
    fn run_id_is_unique_and_round_trips() {
        let a = RunId::new();
        let b = RunId::new();
        assert_ne!(a, b);
        let encoded = serde_json::to_string(&a).expect("run id serializes");
        let decoded: RunId = serde_json::from_str(&encoded).expect("run id deserializes");
        assert_eq!(a, decoded);
    }

    #[test]
    fn host_error_maps_into_workflow_error() {
        let err: WorkflowError = HostError::new("no gateway key").into();
        assert_eq!(err, WorkflowError::Host("no gateway key".into()));
    }
}
