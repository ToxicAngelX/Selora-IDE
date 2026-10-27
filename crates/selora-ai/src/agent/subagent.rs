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
use tokio::sync::{broadcast, watch};

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
    /// The stable wire spelling (`read_only` / `full`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Full => "full",
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
/// child; cancelled children emit nothing (there is no completion to claim).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum SubagentEvent {
    Completed(SubagentCompletion),
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
}

#[derive(Default)]
struct RuntimeState {
    children: BTreeMap<ChildId, ChildRecord>,
    cancelled_parents: BTreeSet<String>,
}

struct RuntimeInner {
    state: Mutex<RuntimeState>,
    events: broadcast::Sender<SubagentEvent>,
    /// Every accepted completion or cancellation bumps this watch. `wait`
    /// subscribes before reading the state, so no wakeup can be lost.
    changed: watch::Sender<u64>,
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
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(256);
        let (changed, _) = watch::channel(0);
        Self {
            inner: Arc::new(RuntimeInner {
                state: Mutex::new(RuntimeState::default()),
                events,
                changed,
            }),
        }
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
        if state.children.contains_key(&request.child_id) {
            return Err(SpawnError::Duplicate(request.child_id));
        }
        let handle = SubagentHandle {
            child_id: request.child_id.clone(),
            parent_session: request.parent_session.clone(),
            policy: request.policy,
        };
        state.children.insert(
            request.child_id.clone(),
            ChildRecord {
                request,
                state: ChildState::Running,
                result: None,
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
        tokio_handle.spawn(async move {
            let outcome = work.await;
            runtime.complete(&child_id, outcome);
        });
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
        let parent_cancelled = state.cancelled_parents.contains(&parent_session);
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
        self.bump(&state);
        drop(state);
        let _ = self.inner.events.send(SubagentEvent::Completed(completion));
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
        record.state = ChildState::Cancelled;
        record.result = None;
        self.bump(&state);
        true
    }

    /// Stop every child of one parent. Returns how many were still running.
    pub fn cancel_parent(&self, parent_session: &str) -> usize {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        state.cancelled_parents.insert(parent_session.to_owned());
        let mut cancelled = 0;
        for record in state.children.values_mut() {
            if record.request.parent_session == parent_session && !record.state.terminal() {
                record.state = ChildState::Cancelled;
                record.result = None;
                cancelled += 1;
            }
        }
        self.bump(&state);
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

    /// Every completed/failed result for a parent, in registration order.
    pub fn results_for_parent(&self, parent_session: &str) -> Vec<SubagentCompletion> {
        self.inner
            .state
            .lock()
            .expect("subagent state lock")
            .children
            .values()
            .filter(|record| record.request.parent_session == parent_session)
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
                let unknown: Vec<String> =
                    ids.iter()
                        .filter(|id| {
                            state.children.get(*id).is_none_or(|record| {
                                record.request.parent_session != parent_session
                            })
                        })
                        .cloned()
                        .collect();
                if !unknown.is_empty() {
                    return Err(WaitError::UnknownIds(unknown.join(", ")));
                }
                ids.clone()
            }
            None => self.child_ids(parent_session),
        };

        let bounded = timeout.min(MAX_WAIT_TIMEOUT);
        let started = tokio::time::Instant::now();
        let mut changed = self.inner.changed.subscribe();

        loop {
            {
                let state = self.inner.state.lock().expect("subagent state lock");
                if state.cancelled_parents.contains(parent_session) {
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

    fn request(parent: &str, child: &str) -> SpawnRequest {
        SpawnRequest::new(parent, child, SubagentPolicy::ReadOnly, "review")
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
        let SubagentEvent::Completed(completion) = event;
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
        let SubagentEvent::Completed(completion) = event;
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

        let SubagentEvent::Completed(completion) = events.recv().await.unwrap();
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
        assert_eq!(
            runtime.complete_detailed("child-1", ChildOutcome::success("late")),
            CompletionDisposition::Discarded
        );
        assert!(runtime.complete("child-2", ChildOutcome::success("sibling won")));

        let SubagentEvent::Completed(completion) = events.recv().await.unwrap();
        assert_eq!(completion.child_id, "child-2");
        assert!(completion.ok);
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
}
