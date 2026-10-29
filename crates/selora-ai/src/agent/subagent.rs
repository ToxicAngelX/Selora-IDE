//! Background subagent runs (ARCHITECTURE.md §19.3, Phase 9e).
//!
//! A subagent is a conversation, not a process. This module owns the *run
//! state* around one child conversation: who the parent is, the policy it
//! was granted, whether it is still running, and the one terminal result.
//! The conversation itself still runs through Phase 5c's loop (or Phase 8's
//! engine); the runtime keeps that work alive after the parent turn's future
//! has ended.
//!
//! The contract the rest of the app consumes:
//!
//! * [`SubagentRuntime::spawn_background`] starts work on the tokio runtime
//!   and does **not** join it into the parent turn. The runtime must outlive
//!   the turn; the desktop session engine owns one runtime for the whole
//!   session.
//! * [`SubagentRuntime::complete`] accepts exactly one terminal outcome per
//!   child. A duplicate is dropped. A result for a cancelled child (or a
//!   child whose parent was cancelled) is discarded, never resurrected.
//! * Every accepted terminal outcome emits one [`SubagentEvent::Completed`]
//!   carrying child id, parent session, policy, success/error, final message
//!   and artifact refs.
//! * [`SubagentRuntime::wait`] blocks on a watch channel, not a spin loop.
//!   It returns completed results, a truthful timeout, or an unknown-id
//!   error, and wakes on parent cancellation.
//!
//! The `wait` tool is routed here through [`wait_tool_result`] so the model
//! gets a bounded, structured answer instead of inventing one.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, watch};

use super::{ApprovalCard, ApprovalId, TurnDecision};
use crate::chat_types::ToolResult;

/// A subagent is identified by an opaque id minted by the host. Script
/// names (Phase 8) and model-supplied ids are validated by the host before
/// they reach this runtime.
pub type ChildId = String;

/// The default `wait` bound. A parent turn is bounded anyway; this keeps the
/// tool from parking forever when a child never reports.
pub const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// The hard ceiling on a single `wait` call (`timeout` arguments clamp here).
pub const MAX_WAIT_TIMEOUT: Duration = Duration::from_secs(300);

/// The tool name added to the manifest for Phase 9e.
pub const WAIT_TOOL_NAME: &str = "wait";

/// §19.3's two policies. `ReadOnly` is structural at the router: the child
/// is constructed without write/bash tools. `Full` keeps the manifest and
/// still parks writes on approval cards in the parent's window.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentPolicy {
    #[default]
    ReadOnly,
    Full,
}

impl SubagentPolicy {
    /// The tools a read-only child may ever see or route.
    pub const READ_ONLY_TOOLS: [&'static str; 6] = [
        "read_file",
        "list_dir",
        "fuzzy_find",
        "search_files",
        "web_search",
        "web_fetch",
    ];

    /// The full child surface: §17.1 only. Orchestration tools
    /// (`spawn_subagent`, `ask_user_question`, `wait`, `send_message`) are
    /// intentionally absent, so a subagent cannot fan out nested children.
    pub const FULL_TOOLS: [&'static str; 10] = [
        "read_file",
        "list_dir",
        "fuzzy_find",
        "search_files",
        "write_file",
        "edit_file",
        "delete_file",
        "bash",
        "web_search",
        "web_fetch",
    ];

    /// The stable wire spelling (`read_only` / `full`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Full => "full",
        }
    }

    /// Whether this policy permits `tool` on either router path.
    #[must_use]
    pub fn allows(self, tool: &str) -> bool {
        match self {
            Self::ReadOnly => Self::READ_ONLY_TOOLS.contains(&tool),
            Self::Full => Self::FULL_TOOLS.contains(&tool),
        }
    }

    /// The manifest this policy arms. Filtering the canonical manifest keeps
    /// names/descriptions in one place while making the absent tools a fact.
    #[must_use]
    pub fn manifest(self) -> Vec<crate::chat_types::ToolSpec> {
        super::router::tool_manifest()
            .into_iter()
            .filter(|spec| self.allows(&spec.name))
            .collect()
    }

    /// Parse the model-facing spelling. Unknown spellings are rejected (never
    /// widened to `full`).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "read-only" | "read_only" | "readonly" => Some(Self::ReadOnly),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

/// One artifact a child produced. `kind` is deliberately a string: Phase 8's
/// engine mints artifact kinds and this type must not become a second,
/// conflicting enum.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub kind: String,
    /// Workspace-relative path, or an engine-owned artifact id when the
    /// artifact is virtual.
    pub path: String,
}

impl ArtifactRef {
    pub fn new(kind: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            path: path.into(),
        }
    }
}

/// What a child conversation returns when it finishes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChildOutcome {
    pub ok: bool,
    /// Present exactly when `ok` is false (a cancelled child never reaches
    /// this type - cancellation is handled by the runtime).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub final_message: String,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

impl ChildOutcome {
    pub fn success(final_message: impl Into<String>) -> Self {
        Self {
            ok: true,
            error: None,
            final_message: final_message.into(),
            artifacts: Vec::new(),
        }
    }

    pub fn failed(error: impl Into<String>) -> Self {
        let error = error.into();
        Self {
            ok: false,
            error: Some(error),
            final_message: String::new(),
            artifacts: Vec::new(),
        }
    }

    pub fn with_artifacts(mut self, artifacts: Vec<ArtifactRef>) -> Self {
        self.artifacts = artifacts;
        self
    }
}

/// The payload of the one completion event. `parent_session` is explicit:
/// the toast/badge route by it even when the turn that spawned the child is
/// long gone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentCompletion {
    pub child_id: ChildId,
    pub parent_session: String,
    pub policy: SubagentPolicy,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub final_message: String,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

impl SubagentCompletion {
    fn from_outcome(record: &ChildRecord, outcome: ChildOutcome) -> Self {
        Self {
            child_id: record.request.child_id.clone(),
            parent_session: record.request.parent_session.clone(),
            policy: record.request.policy,
            ok: outcome.ok,
            error: if outcome.ok {
                None
            } else {
                Some(
                    outcome
                        .error
                        .unwrap_or_else(|| "subagent failed without a message".to_owned()),
                )
            },
            final_message: outcome.final_message,
            artifacts: outcome.artifacts,
        }
    }
}

/// The typed event stream. Exactly one `Completed` is emitted per accepted
/// child, and exactly one `Cancelled` per running child that is stopped;
/// `Activity` carries the live transcript mirrored from the child's turn.
/// `ApprovalNeeded` is the §19.3 child-card rail: a child's write/bash or
/// question parks here and is answered through [`SubagentRuntime::decide`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum SubagentEvent {
    Completed(SubagentCompletion),
    ApprovalNeeded(SubagentCard),
    Activity(SubagentActivity),
    Cancelled {
        child_id: ChildId,
        parent_session: String,
    },
}

/// One live step of a child conversation. The parent session scopes the view;
/// `child_id` is the stable child session key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentActivity {
    pub child_id: ChildId,
    pub parent_session: String,
    pub kind: SubagentActivityKind,
}

impl SubagentActivity {
    pub fn new(
        child_id: impl Into<ChildId>,
        parent_session: impl Into<String>,
        kind: SubagentActivityKind,
    ) -> Self {
        Self {
            child_id: child_id.into(),
            parent_session: parent_session.into(),
            kind,
        }
    }
}

/// The live child transcript vocabulary. `Delta` text is incremental and its
/// `sequence` is monotonic across the child's whole run; `message_id` changes
/// once per assistant output round.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubagentActivityKind {
    Started {
        name: String,
        policy: SubagentPolicy,
        mission: String,
    },
    Delta {
        message_id: String,
        sequence: u64,
        text: String,
    },
    ToolCallStarted {
        call_id: String,
        tool_name: String,
        args_json: String,
    },
    ToolResult {
        call_id: String,
        result_summary: String,
        is_err: bool,
    },
    StateChanged {
        state: SubagentRunState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },
}

/// The child's live status line. `AwaitingApproval` mirrors the turn parking
/// on a child card; `Done` is the turn-end state, not the terminal result.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRunState {
    Thinking,
    RunningTool,
    AwaitingApproval,
    Done,
}

/// A child card waiting on the parent session's normal decision rail. The id
/// is the single-use key; `decide` consumes it before forwarding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentCard {
    pub child_id: ChildId,
    pub parent_session: String,
    pub approval_id: ApprovalId,
    pub card: ApprovalCard,
}

/// Why steering or answering a child card was refused. Every message is safe
/// to show the parent model.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SubagentControlError {
    #[error("subagent `{0}` is unknown")]
    Unknown(ChildId),
    #[error("subagent `{0}` is not running")]
    NotRunning(ChildId),
    #[error("no matching live subagent approval (stale or already used)")]
    StaleApproval,
    #[error("the subagent turn is no longer live")]
    Closed,
}

/// One spawn request. `parent_turn` is diagnostic only (the session is the
/// cancellation scope); it lets a host audit which turn started a child.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpawnRequest {
    pub child_id: ChildId,
    pub parent_session: String,
    pub policy: SubagentPolicy,
    pub mission: String,
    /// False for an inline ask that the caller joins itself.
    pub background: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn: Option<String>,
    /// Optional stable name for the transcript; the driver mints one when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional persona/system supplement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    /// Optional seed conversation (Phase 8 passes its per-agent history).
    /// The mission is appended as a user message regardless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<Vec<crate::types::ChatMessage>>,
}

impl SpawnRequest {
    pub fn new(
        parent_session: impl Into<String>,
        child_id: impl Into<String>,
        policy: SubagentPolicy,
        mission: impl Into<String>,
    ) -> Self {
        Self {
            child_id: child_id.into(),
            parent_session: parent_session.into(),
            policy,
            mission: mission.into(),
            background: true,
            parent_turn: None,
            name: None,
            persona: None,
            conversation: None,
        }
    }

    pub fn background(mut self, background: bool) -> Self {
        self.background = background;
        self
    }

    pub fn parent_turn(mut self, turn: impl Into<String>) -> Self {
        self.parent_turn = Some(turn.into());
        self
    }

    /// Builder-style name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Builder-style persona.
    pub fn with_persona(mut self, persona: impl Into<String>) -> Self {
        self.persona = Some(persona.into());
        self
    }

    /// Builder-style seed conversation.
    pub fn with_conversation(mut self, conversation: Vec<crate::types::ChatMessage>) -> Self {
        self.conversation = Some(conversation);
        self
    }
}

/// A registered run handle. The id is the dedupe key for completion and the
/// key `wait` accepts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubagentHandle {
    pub child_id: ChildId,
    pub parent_session: String,
    pub policy: SubagentPolicy,
}

impl SubagentHandle {
    pub fn id(&self) -> &str {
        &self.child_id
    }
}

/// The states a child can be in. Cancelled is terminal but produces no
/// result (the UI must never show a cancellation as a completion).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl ChildState {
    fn terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// What `complete` did with the outcome. The bool-returning convenience
/// surface is [`SubagentRuntime::complete`]; hosts that need to audit
/// duplicate/late deliveries use this.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionDisposition {
    /// Stored and one event was emitted.
    Emitted,
    /// The child already had a terminal outcome (dedupe).
    Duplicate,
    /// The child or its parent was cancelled; the result was discarded.
    Discarded,
    /// No child with that id is registered.
    Unknown,
}

impl CompletionDisposition {
    pub fn emitted(self) -> bool {
        matches!(self, Self::Emitted)
    }
}

/// A spawn refused up front: duplicate id or an empty id/mission.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SpawnError {
    #[error("subagent id is empty")]
    EmptyId,
    #[error("subagent `{0}` already exists")]
    Duplicate(ChildId),
    #[error("subagent mission is empty")]
    EmptyMission,
    #[error("no tokio runtime is running for background subagent work")]
    NoRuntime,
    #[error("the parent turn has been cancelled")]
    Cancelled,
}

/// The `wait` answer. A timeout is not an error: it tells the parent the
/// truth so it can continue or wait again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WaitOutcome {
    /// Every requested child reached a terminal result.
    Completed { results: Vec<SubagentCompletion> },
    /// At least one child is still running at the bound.
    Timeout {
        pending: Vec<ChildId>,
        waited_ms: u64,
    },
}

/// Why a `wait` could not answer. Both are model-visible errors.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WaitError {
    #[error("unknown subagent id(s): {0}")]
    UnknownIds(String),
    #[error("wait cancelled: the parent turn is no longer running")]
    Cancelled,
}

/// A cooperative cancellation flag for one parent turn. The host flips it
/// when Stop is pressed; `wait` wakes immediately instead of waiting out its
/// bound. The session-level `cancel_parent` remains authoritative for late
/// results.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    inner: Arc<CancelState>,
}

#[derive(Debug, Default)]
struct CancelState {
    flag: std::sync::atomic::AtomicBool,
    notify: tokio::sync::Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if !self
            .inner
            .flag
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            self.inner.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Resolve when cancelled (immediately when already set).
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.inner.notify.notified().await;
    }
}

struct ChildRecord {
    request: SpawnRequest,
    state: ChildState,
    result: Option<SubagentCompletion>,
    generation: u64,
    task: Option<tokio::task::AbortHandle>,
}

#[derive(Default)]
struct RuntimeState {
    children: BTreeMap<ChildId, ChildRecord>,
    cancelled_parents: BTreeSet<String>,
    generations: BTreeMap<String, u64>,
}

struct RuntimeInner {
    state: Mutex<RuntimeState>,
    events: broadcast::Sender<SubagentEvent>,
    /// Every accepted completion or cancellation bumps this watch. `wait`
    /// subscribes before reading the state, so no wakeup can be lost.
    changed: watch::Sender<u64>,
    /// §19.3: the desktop relay's event sink. Broadcast alone can drop an
    /// event emitted before its receiver subscribes, so the session installs
    /// one sink at construction time.
    sink: Mutex<Option<mpsc::UnboundedSender<SubagentEvent>>>,
    /// Pending child cards: the single-use decision route. `decide` removes
    /// the entry before forwarding, so a duplicate is inert.
    decisions: Mutex<BTreeMap<ApprovalId, mpsc::UnboundedSender<TurnDecision>>>,
    /// Every card id registered by a child, so the driver can forget its
    /// stale entries when the turn ends.
    child_cards: Mutex<BTreeMap<ChildId, BTreeSet<ApprovalId>>>,
    /// FIFO steering channel per running child. A child drains it between
    /// turns; a finished child has no channel and refuses steering.
    steering: Mutex<BTreeMap<ChildId, mpsc::UnboundedSender<String>>>,
}

/// The session-owned background subagent runtime. Cloning is cheap and
/// shares the same state (the desktop engine and its tool contexts use the
/// same instance).
#[derive(Clone)]
pub struct SubagentRuntime {
    inner: Arc<RuntimeInner>,
}

impl Default for SubagentRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SubagentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.inner.state.lock().expect("subagent state lock");
        f.debug_struct("SubagentRuntime")
            .field("children", &state.children.len())
            .field("cancelled_parents", &state.cancelled_parents)
            .finish()
    }
}

impl SubagentRuntime {
    /// Start a new turn without reviving children cancelled by an earlier turn.
    pub fn begin_turn(&self, parent_session: &str) {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        *state
            .generations
            .entry(parent_session.to_owned())
            .or_default() += 1;
        state.cancelled_parents.remove(parent_session);
        self.bump(&state);
    }

    pub fn new() -> Self {
        let (events, _) = broadcast::channel(256);
        let (changed, _) = watch::channel(0);
        Self {
            inner: Arc::new(RuntimeInner {
                state: Mutex::new(RuntimeState::default()),
                events,
                changed,
                sink: Mutex::new(None),
                decisions: Mutex::new(BTreeMap::new()),
                child_cards: Mutex::new(BTreeMap::new()),
                steering: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    /// Install the session relay's event sink. Every event is also broadcast;
    /// the sink guarantees delivery even when no broadcast receiver exists
    /// yet.
    pub fn set_event_sink(&self, sink: mpsc::UnboundedSender<SubagentEvent>) {
        *self.inner.sink.lock().expect("subagent sink lock") = Some(sink);
    }

    fn emit(&self, event: SubagentEvent) {
        if let Some(sink) = self.inner.sink.lock().expect("subagent sink lock").as_ref() {
            let _ = sink.send(event.clone());
        }
        let _ = self.inner.events.send(event);
    }

    /// Register a child's steer queue. The driver owns the receiver and
    /// clears the entry when the run ends.
    pub fn register_steering(&self, child_id: &str, tx: mpsc::UnboundedSender<String>) {
        self.inner
            .steering
            .lock()
            .expect("subagent steering lock")
            .insert(child_id.to_owned(), tx);
    }

    /// Queue one steering message for a running child (FIFO).
    pub fn send_message(
        &self,
        child_id: &str,
        text: impl Into<String>,
    ) -> Result<(), SubagentControlError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(SubagentControlError::NotRunning(child_id.to_owned()));
        }
        if self.child_state(child_id).is_none() {
            return Err(SubagentControlError::Unknown(child_id.to_owned()));
        }
        let steering = self.inner.steering.lock().expect("subagent steering lock");
        let Some(sender) = steering.get(child_id) else {
            return Err(SubagentControlError::NotRunning(child_id.to_owned()));
        };
        sender
            .send(text)
            .map_err(|_| SubagentControlError::NotRunning(child_id.to_owned()))
    }

    /// Register one parked child card. Called by the driver before the event
    /// is emitted, so the card cannot be answered before it exists.
    pub fn register_decision(
        &self,
        child_id: &str,
        approval_id: ApprovalId,
        tx: mpsc::UnboundedSender<TurnDecision>,
    ) {
        self.inner
            .decisions
            .lock()
            .expect("subagent decision lock")
            .insert(approval_id, tx);
        self.inner
            .child_cards
            .lock()
            .expect("subagent card lock")
            .entry(child_id.to_owned())
            .or_default()
            .insert(approval_id);
    }

    /// Answer one parked child card exactly once. The entry is consumed
    /// before forwarding, so a stale or duplicate id can never apply.
    pub fn decide(
        &self,
        approval_id: ApprovalId,
        decision: TurnDecision,
    ) -> Result<(), SubagentControlError> {
        let sender = self
            .inner
            .decisions
            .lock()
            .expect("subagent decision lock")
            .remove(&approval_id)
            .ok_or(SubagentControlError::StaleApproval)?;
        sender
            .send(decision)
            .map_err(|_| SubagentControlError::Closed)
    }

    /// Drop a child's stale card entries once its turn is no longer live.
    pub fn forget_child_cards(&self, child_id: &str) {
        let cards = self
            .inner
            .child_cards
            .lock()
            .expect("subagent card lock")
            .remove(child_id)
            .unwrap_or_default();
        let mut decisions = self.inner.decisions.lock().expect("subagent decision lock");
        for approval_id in cards {
            decisions.remove(&approval_id);
        }
    }

    /// Drop a child's steer queue when its run ends.
    pub fn forget_steering(&self, child_id: &str) {
        self.inner
            .steering
            .lock()
            .expect("subagent steering lock")
            .remove(child_id);
    }

    /// Emit the one `ApprovalNeeded` event for a child card.
    pub fn emit_child_card(&self, card: SubagentCard) {
        self.emit(SubagentEvent::ApprovalNeeded(card));
    }

    /// Emit one live child transcript step.
    pub fn emit_activity(&self, activity: SubagentActivity) {
        self.emit(SubagentEvent::Activity(activity));
    }

    /// Subscribe to the one-event-per-child stream. Late subscribers do not
    /// receive history; the host also persists the completion itself.
    pub fn subscribe(&self) -> broadcast::Receiver<SubagentEvent> {
        self.inner.events.subscribe()
    }

    /// Register a child without running it. Hosts that own their own task
    /// (Phase 8's engine) call this, run the ask, then [`Self::complete`].
    pub fn register(&self, request: SpawnRequest) -> Result<SubagentHandle, SpawnError> {
        if request.child_id.trim().is_empty() {
            return Err(SpawnError::EmptyId);
        }
        if request.mission.trim().is_empty() {
            return Err(SpawnError::EmptyMission);
        }
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if state.cancelled_parents.contains(&request.parent_session) {
            return Err(SpawnError::Cancelled);
        }
        if state.children.contains_key(&request.child_id) {
            return Err(SpawnError::Duplicate(request.child_id));
        }
        let handle = SubagentHandle {
            child_id: request.child_id.clone(),
            parent_session: request.parent_session.clone(),
            policy: request.policy,
        };
        let generation = *state.generations.get(&request.parent_session).unwrap_or(&0);
        state.children.insert(
            request.child_id.clone(),
            ChildRecord {
                request,
                state: ChildState::Running,
                result: None,
                generation,
                task: None,
            },
        );
        self.bump(&state);
        Ok(handle)
    }

    /// Alias for [`Self::register`] (the spawn tool's natural spelling).
    pub fn spawn(&self, request: SpawnRequest) -> Result<SubagentHandle, SpawnError> {
        self.register(request)
    }

    /// Register and run a child on the tokio runtime. The task is *not*
    /// joined into the parent turn; dropping the parent's future leaves the
    /// child running. Use a runtime that lives as long as the session.
    /// Returns [`SpawnError::NoRuntime`] outside a tokio runtime instead of
    /// panicking.
    pub fn spawn_background<F>(
        &self,
        request: SpawnRequest,
        work: F,
    ) -> Result<SubagentHandle, SpawnError>
    where
        F: Future<Output = ChildOutcome> + Send + 'static,
    {
        let handle = self.register(request)?;
        let Ok(tokio_handle) = tokio::runtime::Handle::try_current() else {
            self.cancel_child(&handle.child_id);
            return Err(SpawnError::NoRuntime);
        };
        let runtime = self.clone();
        let child_id = handle.child_id.clone();
        let (start, ready) = tokio::sync::oneshot::channel();
        let task = tokio_handle.spawn(async move {
            if ready.await.is_err() {
                return;
            }
            let outcome = work.await;
            runtime.complete(&child_id, outcome);
        });
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.children.get_mut(&handle.child_id) {
            if record.state == ChildState::Cancelled {
                task.abort();
            } else if !record.state.terminal() {
                record.task = Some(task.abort_handle());
                let _ = start.send(());
            }
        }
        Ok(handle)
    }

    /// Accept one terminal outcome. Returns `true` only for a result that
    /// was stored and emitted exactly once.
    pub fn complete(&self, child_id: &str, outcome: ChildOutcome) -> bool {
        self.complete_detailed(child_id, outcome).emitted()
    }

    /// The auditable form of [`Self::complete`].
    pub fn complete_detailed(
        &self,
        child_id: &str,
        outcome: ChildOutcome,
    ) -> CompletionDisposition {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        let Some(parent_session) = state
            .children
            .get(child_id)
            .map(|record| record.request.parent_session.clone())
        else {
            return CompletionDisposition::Unknown;
        };
        let parent_cancelled = state.cancelled_parents.contains(&parent_session)
            && state.children[child_id].generation
                == *state.generations.get(&parent_session).unwrap_or(&0);
        let record = state
            .children
            .get_mut(child_id)
            .expect("checked by the lookup above");
        if record.state == ChildState::Cancelled {
            return CompletionDisposition::Discarded;
        }
        if record.state.terminal() {
            return CompletionDisposition::Duplicate;
        }
        if parent_cancelled {
            // A late result from a cancelled child/parent is discarded, not
            // resurrected. Marking the child cancelled is idempotent.
            record.state = ChildState::Cancelled;
            record.result = None;
            self.bump(&state);
            return CompletionDisposition::Discarded;
        }
        let completion = SubagentCompletion::from_outcome(record, outcome);
        record.state = if completion.ok {
            ChildState::Completed
        } else {
            ChildState::Failed
        };
        record.result = Some(completion.clone());
        record.task = None;
        self.bump(&state);
        drop(state);
        self.forget_child_cards(child_id);
        self.forget_steering(child_id);
        self.emit(SubagentEvent::Completed(completion));
        CompletionDisposition::Emitted
    }

    /// Convenience failure path; still one event with `ok: false`.
    pub fn fail(&self, child_id: &str, error: impl Into<String>) -> bool {
        self.complete(child_id, ChildOutcome::failed(error))
    }

    /// Cancel one child. Siblings keep running. A result that arrives later
    /// is discarded. Returns whether this call changed the state.
    pub fn cancel_child(&self, child_id: &str) -> bool {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        let Some(record) = state.children.get_mut(child_id) else {
            return false;
        };
        if record.state.terminal() {
            return false;
        }
        let parent_session = record.request.parent_session.clone();
        record.state = ChildState::Cancelled;
        record.result = None;
        if let Some(task) = record.task.take() {
            task.abort();
        }
        self.bump(&state);
        drop(state);
        self.forget_child_cards(child_id);
        self.forget_steering(child_id);
        self.emit(SubagentEvent::Cancelled {
            child_id: child_id.to_owned(),
            parent_session,
        });
        true
    }

    /// Stop every child of one parent. Returns how many were still running.
    pub fn cancel_parent(&self, parent_session: &str) -> usize {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        state.cancelled_parents.insert(parent_session.to_owned());
        let generation = *state.generations.get(parent_session).unwrap_or(&0);
        let mut cancelled = 0;
        let mut cancelled_ids = Vec::new();
        for record in state.children.values_mut() {
            if record.request.parent_session == parent_session
                && record.generation == generation
                && !record.state.terminal()
            {
                record.state = ChildState::Cancelled;
                record.result = None;
                cancelled_ids.push(record.request.child_id.clone());
                if let Some(task) = record.task.take() {
                    task.abort();
                }
                cancelled += 1;
            }
        }
        self.bump(&state);
        drop(state);
        for child_id in cancelled_ids {
            self.forget_child_cards(&child_id);
            self.forget_steering(&child_id);
            self.emit(SubagentEvent::Cancelled {
                child_id,
                parent_session: parent_session.to_owned(),
            });
        }
        cancelled
    }

    /// True when the parent session was cancelled. Late results are dropped.
    pub fn is_parent_cancelled(&self, parent_session: &str) -> bool {
        self.inner
            .state
            .lock()
            .expect("subagent state lock")
            .cancelled_parents
            .contains(parent_session)
    }

    /// The child's current state, or `None` for an unknown id.
    pub fn child_state(&self, child_id: &str) -> Option<ChildState> {
        self.inner
            .state
            .lock()
            .expect("subagent state lock")
            .children
            .get(child_id)
            .map(|record| record.state)
    }

    /// The stored terminal result for `child_id` (never a cancelled child).
    pub fn result(&self, child_id: &str) -> Option<SubagentCompletion> {
        self.inner
            .state
            .lock()
            .expect("subagent state lock")
            .children
            .get(child_id)
            .and_then(|record| record.result.clone())
    }

    /// Wait for one child's terminal state, including cancellation, without retaining a broadcast task.
    pub async fn wait_for_child(&self, child_id: &str) -> Option<SubagentCompletion> {
        let mut changed = self.inner.changed.subscribe();
        loop {
            {
                let state = self.inner.state.lock().expect("subagent state lock");
                let record = state.children.get(child_id)?;
                if record.state.terminal() {
                    return record.result.clone();
                }
            }
            if changed.changed().await.is_err() {
                return None;
            }
        }
    }

    /// Every completed/failed result for a parent, in registration order.
    pub fn results_for_parent(&self, parent_session: &str) -> Vec<SubagentCompletion> {
        let state = self.inner.state.lock().expect("subagent state lock");
        let generation = *state.generations.get(parent_session).unwrap_or(&0);
        state
            .children
            .values()
            .filter(|record| {
                record.request.parent_session == parent_session && record.generation == generation
            })
            .filter_map(|record| record.result.clone())
            .collect()
    }

    /// All ids belonging to `parent_session`, in registration order.
    pub fn child_ids(&self, parent_session: &str) -> Vec<ChildId> {
        self.inner
            .state
            .lock()
            .expect("subagent state lock")
            .children
            .values()
            .filter(|record| record.request.parent_session == parent_session)
            .map(|record| record.request.child_id.clone())
            .collect()
    }

    /// Block until `child_ids` (or every child of the parent when `None`)
    /// has a terminal result, the bound expires, or the parent is cancelled.
    ///
    /// This uses a watch channel, never a spin loop. The timeout is capped by
    /// [`MAX_WAIT_TIMEOUT`]; `Duration::ZERO` still performs one state check.
    pub async fn wait(
        &self,
        parent_session: &str,
        child_ids: Option<Vec<ChildId>>,
        timeout: Duration,
    ) -> Result<WaitOutcome, WaitError> {
        self.wait_with_cancel(parent_session, child_ids, timeout, None)
            .await
    }

    /// [`Self::wait`] plus a turn's cooperative cancellation token.
    pub async fn wait_with_cancel(
        &self,
        parent_session: &str,
        child_ids: Option<Vec<ChildId>>,
        timeout: Duration,
        cancel: Option<&CancelToken>,
    ) -> Result<WaitOutcome, WaitError> {
        let generation = *self
            .inner
            .state
            .lock()
            .expect("subagent state lock")
            .generations
            .get(parent_session)
            .unwrap_or(&0);
        let requested = child_ids.map(|ids| {
            let mut seen = BTreeSet::new();
            ids.into_iter()
                .filter(|id| seen.insert(id.clone()))
                .collect::<Vec<_>>()
        });

        // Validate explicitly requested ids before parking. An unknown id is
        // a caller bug, not something a timeout should hide.
        let ids = match &requested {
            Some(ids) => {
                let state = self.inner.state.lock().expect("subagent state lock");
                let unknown: Vec<String> = ids
                    .iter()
                    .filter(|id| {
                        state.children.get(*id).is_none_or(|record| {
                            record.request.parent_session != parent_session
                                || record.generation != generation
                        })
                    })
                    .cloned()
                    .collect();
                if !unknown.is_empty() {
                    return Err(WaitError::UnknownIds(unknown.join(", ")));
                }
                ids.clone()
            }
            None => self
                .inner
                .state
                .lock()
                .expect("subagent state lock")
                .children
                .values()
                .filter(|record| {
                    record.request.parent_session == parent_session
                        && record.generation == generation
                })
                .map(|record| record.request.child_id.clone())
                .collect(),
        };

        let bounded = timeout.min(MAX_WAIT_TIMEOUT);
        let started = tokio::time::Instant::now();
        let mut changed = self.inner.changed.subscribe();

        loop {
            {
                let state = self.inner.state.lock().expect("subagent state lock");
                if state.cancelled_parents.contains(parent_session)
                    || *state.generations.get(parent_session).unwrap_or(&0) != generation
                {
                    return Err(WaitError::Cancelled);
                }
                let explicit = requested.is_some();
                let records: Vec<&ChildRecord> =
                    ids.iter().filter_map(|id| state.children.get(id)).collect();
                let cancelled: Vec<&str> = records
                    .iter()
                    .filter(|record| record.state == ChildState::Cancelled)
                    .map(|record| record.request.child_id.as_str())
                    .collect();
                if explicit && !cancelled.is_empty() {
                    return Err(WaitError::Cancelled);
                }
                let pending: Vec<ChildId> = records
                    .iter()
                    .filter(|record| !record.state.terminal())
                    .map(|record| record.request.child_id.clone())
                    .collect();
                if pending.is_empty() {
                    let results = records
                        .iter()
                        .filter_map(|record| record.result.clone())
                        .collect::<Vec<_>>();
                    return Ok(WaitOutcome::Completed { results });
                }
                let elapsed = started.elapsed();
                if elapsed >= bounded {
                    return Ok(WaitOutcome::Timeout {
                        pending,
                        waited_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
                    });
                }
            }

            let remaining = bounded.saturating_sub(started.elapsed());
            let deadline = tokio::time::sleep(remaining);
            tokio::pin!(deadline);

            match cancel {
                Some(cancel) => {
                    if cancel.is_cancelled() {
                        return Err(WaitError::Cancelled);
                    }
                    tokio::select! {
                        changed = changed.changed() => {
                            if changed.is_err() {
                                // The runtime dropped its sender; fall back to
                                // the bound rather than spinning.
                                tokio::time::sleep(remaining).await;
                            }
                        }
                        _ = cancel.cancelled() => return Err(WaitError::Cancelled),
                        _ = &mut deadline => {
                            // Re-check once at the bound (a result may have
                            // landed concurrently with the timer).
                        }
                    }
                }
                None => {
                    tokio::select! {
                        changed = changed.changed() => {
                            if changed.is_err() {
                                tokio::time::sleep(remaining).await;
                            }
                        }
                        _ = &mut deadline => {}
                    }
                }
            }
        }
    }

    fn bump(&self, _state: &RuntimeState) {
        self.inner.changed.send_modify(|sequence| *sequence += 1);
    }
}

// ---------------------------------------------------------------------------
// The `wait` tool
// ---------------------------------------------------------------------------

/// The manifest entry for `wait`. Manifest v3 (Phase 9) adds this one tool.
pub fn wait_tool_spec() -> crate::chat_types::ToolSpec {
    crate::chat_types::ToolSpec::new(
        WAIT_TOOL_NAME,
        "Wait for background subagents to finish. Omit child_ids to wait for every \
         child of this session. Returns each completed child's final message and \
         artifacts, or a truthful timeout with the still-running ids. Unknown ids \
         are an error. The wait is bounded (default 30s, max 300s).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "child_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Subagent ids to wait for. Omit to wait for every child of this session."
                },
                "timeout_secs": {
                    "type": "number",
                    "description": "Bound in seconds (default 30, capped at 300). Fractions are allowed."
                },
                "timeout_ms": {
                    "type": "number",
                    "description": "Bound in milliseconds (alternative to timeout_secs)."
                },
                "timeout": {
                    "type": "number",
                    "description": "Bound in seconds (alias of timeout_secs)."
                }
            },
            "required": []
        }),
    )
}

/// Parse and execute the `wait` tool against a runtime. Used by the router;
/// kept public so hosts with their own tool loops get the same contract.
pub async fn wait_tool_result(
    runtime: &SubagentRuntime,
    parent_session: &str,
    call_id: &str,
    arguments: &str,
) -> ToolResult {
    let args: serde_json::Value = match serde_json::from_str(arguments) {
        Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
        Ok(_) => return ToolResult::error(call_id, "wait: arguments must be a JSON object"),
        Err(error) => {
            return ToolResult::error(call_id, format!("wait: invalid JSON arguments: {error}"))
        }
    };

    let child_ids = match args.get("child_ids") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Array(values)) => {
            let mut ids = Vec::with_capacity(values.len());
            for value in values {
                match value {
                    serde_json::Value::String(id) => ids.push(id.clone()),
                    _ => {
                        return ToolResult::error(
                            call_id,
                            "wait: child_ids must be an array of strings",
                        )
                    }
                }
            }
            Some(ids)
        }
        Some(_) => {
            return ToolResult::error(call_id, "wait: child_ids must be an array of strings")
        }
    };

    let timeout = match parse_timeout(&args) {
        Ok(timeout) => timeout,
        Err(message) => return ToolResult::error(call_id, message),
    };

    match runtime.wait(parent_session, child_ids, timeout).await {
        Ok(WaitOutcome::Completed { results }) => {
            let body = serde_json::json!({
                "status": "completed",
                "children": results,
            });
            ToolResult::success(call_id, body.to_string())
        }
        Ok(WaitOutcome::Timeout { pending, waited_ms }) => {
            // A timeout is truthful, not an error: the child is still alive.
            let body = serde_json::json!({
                "status": "timeout",
                "pending": pending,
                "waited_ms": waited_ms,
            });
            ToolResult::success(call_id, body.to_string())
        }
        Err(WaitError::UnknownIds(ids)) => {
            ToolResult::error(call_id, format!("wait: unknown subagent id(s): {ids}"))
        }
        Err(WaitError::Cancelled) => ToolResult::error(
            call_id,
            "wait: cancelled - the parent turn is no longer running",
        ),
    }
}

fn parse_timeout(args: &serde_json::Value) -> Result<Duration, String> {
    let number = |value: &serde_json::Value| -> Result<f64, String> {
        value
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .ok_or_else(|| "wait: timeout must be a non-negative number".to_owned())
    };
    let timeout = if let Some(value) = args.get("timeout_ms") {
        Duration::from_millis(number(value)? as u64)
    } else if let Some(value) = args.get("timeout_secs") {
        let secs = number(value)?.min(MAX_WAIT_TIMEOUT.as_secs_f64() + 1.0);
        Duration::from_secs_f64(secs)
    } else if let Some(value) = args.get("timeout") {
        let secs = number(value)?.min(MAX_WAIT_TIMEOUT.as_secs_f64() + 1.0);
        Duration::from_secs_f64(secs)
    } else {
        DEFAULT_WAIT_TIMEOUT
    };
    Ok(timeout.min(MAX_WAIT_TIMEOUT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentDecision;

    fn request(parent: &str, child: &str) -> SpawnRequest {
        SpawnRequest::new(parent, child, SubagentPolicy::ReadOnly, "review")
    }

    #[tokio::test]
    async fn cancellation_is_turn_scoped_and_wait_ignores_old_children() {
        let runtime = SubagentRuntime::new();
        runtime.begin_turn("session");
        runtime
            .register(request("session", "cancelled-child"))
            .unwrap();
        runtime.cancel_parent("session");
        runtime.begin_turn("session");
        assert!(!runtime.is_parent_cancelled("session"));
        assert!(!runtime.complete("cancelled-child", ChildOutcome::success("late")));
        runtime.register(request("session", "new-child")).unwrap();
        assert!(runtime.complete("new-child", ChildOutcome::success("current")));
        let WaitOutcome::Completed { results } =
            runtime.wait("session", None, Duration::ZERO).await.unwrap()
        else {
            panic!("current turn has finished");
        };
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].child_id, "new-child");
        runtime.begin_turn("session");
        runtime
            .register(request("session", "previous-running"))
            .unwrap();
        runtime.begin_turn("session");
        assert!(
            matches!(runtime.wait("session", None, Duration::ZERO).await,
            Ok(WaitOutcome::Completed { results }) if results.is_empty())
        );
    }

    #[tokio::test]
    async fn cancelling_a_parent_drops_background_work() {
        struct DropFlag(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let runtime = SubagentRuntime::new();
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = DropFlag(dropped.clone());
        runtime
            .spawn_background(request("session", "pending"), async move {
                let _probe = probe;
                std::future::pending().await
            })
            .unwrap();
        tokio::task::yield_now().await;
        runtime.cancel_parent("session");
        tokio::task::yield_now().await;
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn background_child_completes_after_the_parent_turn_ends() {
        let runtime = SubagentRuntime::new();
        let mut events = runtime.subscribe();
        let handle = runtime
            .spawn_background(request("parent-1", "child-1"), async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                ChildOutcome::success("the review is done")
                    .with_artifacts(vec![ArtifactRef::new("diff", "src/lib.rs")])
            })
            .expect("spawn");

        // Drop every parent-scope value: only the runtime holds the work.
        drop(handle);

        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("completion arrives")
            .expect("event");
        let completion = match event {
            SubagentEvent::Completed(completion) => completion,
            other => panic!("expected a completion, got {other:?}"),
        };
        assert_eq!(completion.child_id, "child-1");
        assert_eq!(completion.parent_session, "parent-1");
        assert_eq!(completion.policy, SubagentPolicy::ReadOnly);
        assert!(completion.ok);
        assert_eq!(completion.final_message, "the review is done");
        assert_eq!(completion.artifacts[0].path, "src/lib.rs");
    }

    #[tokio::test]
    async fn exactly_one_event_per_child_even_when_completed_twice() {
        let runtime = SubagentRuntime::new();
        let mut events = runtime.subscribe();
        runtime.register(request("parent-1", "child-1")).unwrap();

        assert!(runtime.complete("child-1", ChildOutcome::success("done")));
        assert!(!runtime.complete("child-1", ChildOutcome::success("done again")));
        assert_eq!(
            runtime.complete_detailed("child-1", ChildOutcome::success("third")),
            CompletionDisposition::Duplicate
        );

        let event = events.recv().await.expect("one event");
        let completion = match event {
            SubagentEvent::Completed(completion) => completion,
            other => panic!("expected a completion, got {other:?}"),
        };
        assert_eq!(completion.final_message, "done");
        assert!(
            tokio::time::timeout(Duration::from_millis(30), events.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn failed_child_is_not_reported_as_success() {
        let runtime = SubagentRuntime::new();
        let mut events = runtime.subscribe();
        runtime.register(request("parent-1", "child-1")).unwrap();
        assert!(runtime.fail("child-1", "gateway said no"));

        let completion = match events.recv().await.unwrap() {
            SubagentEvent::Completed(completion) => completion,
            other => panic!("expected a completion, got {other:?}"),
        };
        assert!(!completion.ok);
        assert_eq!(completion.error.as_deref(), Some("gateway said no"));
        assert_eq!(completion.final_message, "");
    }

    #[tokio::test]
    async fn wait_returns_immediately_when_results_exist() {
        let runtime = SubagentRuntime::new();
        runtime.register(request("parent-1", "child-1")).unwrap();
        runtime.complete("child-1", ChildOutcome::success("ready"));

        let started = std::time::Instant::now();
        let outcome = runtime
            .wait(
                "parent-1",
                Some(vec!["child-1".into()]),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_millis(100));
        match outcome {
            WaitOutcome::Completed { results } => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].final_message, "ready");
            }
            other => panic!("expected completion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_times_out_truthfully_and_does_not_spin() {
        let runtime = SubagentRuntime::new();
        runtime.register(request("parent-1", "child-1")).unwrap();
        let outcome = runtime
            .wait(
                "parent-1",
                Some(vec!["child-1".into()]),
                Duration::from_millis(30),
            )
            .await
            .unwrap();
        match outcome {
            WaitOutcome::Timeout { pending, waited_ms } => {
                assert_eq!(pending, vec!["child-1".to_owned()]);
                assert!(waited_ms >= 20, "truthful bound, got {waited_ms}ms");
            }
            other => panic!("expected timeout, got {other:?}"),
        }
        // The child can still finish afterwards.
        runtime.complete("child-1", ChildOutcome::success("late but alive"));
        assert_eq!(
            runtime.result("child-1").unwrap().final_message,
            "late but alive"
        );
    }

    #[tokio::test]
    async fn wait_rejects_unknown_ids_before_waiting() {
        let runtime = SubagentRuntime::new();
        let error = runtime
            .wait(
                "parent-1",
                Some(vec!["ghost".into()]),
                Duration::from_secs(5),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, WaitError::UnknownIds(ids) if ids.contains("ghost")));
    }

    #[tokio::test]
    async fn wait_respects_turn_cancellation() {
        let runtime = SubagentRuntime::new();
        runtime.register(request("parent-1", "child-1")).unwrap();
        let cancel = CancelToken::new();
        let waiter = {
            let runtime = runtime.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                runtime
                    .wait_with_cancel(
                        "parent-1",
                        Some(vec!["child-1".into()]),
                        Duration::from_secs(30),
                        Some(&cancel),
                    )
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("wait wakes")
            .expect("join");
        assert_eq!(result.unwrap_err(), WaitError::Cancelled);
    }

    #[tokio::test]
    async fn cancel_parent_discards_late_results_and_wakes_wait() {
        let runtime = SubagentRuntime::new();
        let mut events = runtime.subscribe();
        runtime.register(request("parent-1", "child-1")).unwrap();

        let waiter = {
            let runtime = runtime.clone();
            tokio::spawn(async move {
                runtime
                    .wait("parent-1", None, Duration::from_secs(30))
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(runtime.cancel_parent("parent-1"), 1);

        let result = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("wait wakes")
            .expect("join");
        assert_eq!(result.unwrap_err(), WaitError::Cancelled);

        assert_eq!(
            runtime.complete_detailed("child-1", ChildOutcome::success("too late")),
            CompletionDisposition::Discarded
        );
        assert!(runtime.result("child-1").is_none());
        let cancelled = match events.recv().await.expect("cancellation event") {
            SubagentEvent::Cancelled {
                child_id,
                parent_session,
            } => (child_id, parent_session),
            other => panic!("expected a cancellation, got {other:?}"),
        };
        assert_eq!(cancelled.0, "child-1");
        assert_eq!(cancelled.1, "parent-1");
        assert!(
            tokio::time::timeout(Duration::from_millis(30), events.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn cancelling_one_child_leaves_siblings_running() {
        let runtime = SubagentRuntime::new();
        let mut events = runtime.subscribe();
        runtime.register(request("parent-1", "child-1")).unwrap();
        runtime.register(request("parent-1", "child-2")).unwrap();

        assert!(runtime.cancel_child("child-1"));
        assert!(
            !runtime.cancel_child("child-1"),
            "cancellation is exactly once"
        );
        assert_eq!(
            runtime.complete_detailed("child-1", ChildOutcome::success("late")),
            CompletionDisposition::Discarded
        );
        assert!(runtime.complete("child-2", ChildOutcome::success("sibling won")));

        let cancelled = match events.recv().await.unwrap() {
            SubagentEvent::Cancelled {
                child_id,
                parent_session,
            } => (child_id, parent_session),
            other => panic!("expected a cancellation, got {other:?}"),
        };
        assert_eq!(cancelled, ("child-1".into(), "parent-1".into()));
        let completion = match events.recv().await.unwrap() {
            SubagentEvent::Completed(completion) => completion,
            other => panic!("expected a completion, got {other:?}"),
        };
        assert_eq!(completion.child_id, "child-2");
        assert!(completion.ok);
        // Cancellation is one lifecycle event, never repeated by the late result.
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn wait_tool_returns_structured_json_and_model_visible_errors() {
        let runtime = SubagentRuntime::new();
        runtime.register(request("parent-1", "child-1")).unwrap();
        runtime.complete("child-1", ChildOutcome::failed("boom"));

        let ok = wait_tool_result(
            &runtime,
            "parent-1",
            "call-1",
            r#"{"child_ids":["child-1"],"timeout_secs":1}"#,
        )
        .await;
        assert!(ok.ok);
        let body: serde_json::Value = serde_json::from_str(&ok.content).unwrap();
        assert_eq!(body["status"], "completed");
        assert_eq!(body["children"][0]["ok"], false);
        assert_eq!(body["children"][0]["error"], "boom");

        let timeout = wait_tool_result(&runtime, "parent-1", "call-2", r#"{"timeout_ms":5}"#).await;
        assert!(timeout.ok);
        let body: serde_json::Value = serde_json::from_str(&timeout.content).unwrap();
        assert!(body["status"] == "completed" || body["status"] == "timeout");

        let unknown =
            wait_tool_result(&runtime, "parent-1", "call-3", r#"{"child_ids":["ghost"]}"#).await;
        assert!(!unknown.ok);
        assert!(unknown.content.contains("unknown subagent id"));
    }

    #[test]
    fn child_card_is_answered_exactly_once() {
        let runtime = SubagentRuntime::new();
        runtime.register(request("session", "child-1")).unwrap();
        let approval_id = uuid::Uuid::new_v4();
        let (tx, mut rx) = mpsc::unbounded_channel::<TurnDecision>();
        runtime.register_decision("child-1", approval_id, tx);
        runtime.emit_child_card(SubagentCard {
            child_id: "child-1".into(),
            parent_session: "session".into(),
            approval_id,
            card: ApprovalCard::Command {
                command: "cargo test".into(),
            },
        });
        assert!(runtime
            .decide(approval_id, (approval_id, AgentDecision::Approve))
            .is_ok());
        let (id, decision) = rx.try_recv().expect("decision forwarded");
        assert_eq!(id, approval_id);
        assert_eq!(decision, AgentDecision::Approve);
        // A stale or duplicate answer can never reach the child twice.
        assert_eq!(
            runtime.decide(approval_id, (approval_id, AgentDecision::Approve)),
            Err(SubagentControlError::StaleApproval)
        );
        runtime.forget_child_cards("child-1");
    }

    #[test]
    fn steering_requires_a_registered_running_child() {
        let runtime = SubagentRuntime::new();
        runtime.register(request("session", "child-1")).unwrap();
        assert_eq!(
            runtime.send_message("child-1", "look closer"),
            Err(SubagentControlError::NotRunning("child-1".into()))
        );
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        runtime.register_steering("child-1", tx);
        runtime.send_message("child-1", "look closer").unwrap();
        assert_eq!(rx.try_recv().unwrap(), "look closer");
        runtime.forget_steering("child-1");
        assert_eq!(
            runtime.send_message("ghost", "hi"),
            Err(SubagentControlError::Unknown("ghost".into()))
        );
    }

    #[test]
    fn event_sink_receives_cards_without_a_broadcast_subscriber() {
        let runtime = SubagentRuntime::new();
        let (tx, mut rx) = mpsc::unbounded_channel::<SubagentEvent>();
        runtime.set_event_sink(tx);
        runtime.emit_child_card(SubagentCard {
            child_id: "child-1".into(),
            parent_session: "session".into(),
            approval_id: uuid::Uuid::new_v4(),
            card: ApprovalCard::Plan {
                summary: None,
                steps: vec!["Inspect".into()],
            },
        });
        assert!(matches!(
            rx.try_recv().unwrap(),
            SubagentEvent::ApprovalNeeded(card) if card.child_id == "child-1"
        ));
    }
}
