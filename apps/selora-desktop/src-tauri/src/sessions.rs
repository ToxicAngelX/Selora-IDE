//! Phase Z3 — the agent dock's engine (docs/ui/PHASE_Z_MIGRATION.md).
//!
//! Owns the chat/agent sessions behind the dock: the in-memory transcripts
//! (the bootstrap's `chat` snapshot), the in-flight requests keyed by
//! `RequestId`, the single-use approval slots, and the two relays that turn
//! `selora-ai`'s event streams into protocol events on `selora://event`
//! through [`crate::state::EventLog`].
//!
//! The routing vocabulary is the four `FrontendCommand` arms this slice
//! un-stubs:
//! - `SendChat` → [`SessionEngine::send_chat`] → `SeloraClient::stream_chat`
//!   behind a cancel watch; deltas arrive as `ChatDelta`, the terminal as
//!   `ChatCompleted`/`ChatFailed` (a cancelled stop ends *silently* with the
//!   partial bubble finalized into the transcript — a user stop is never an
//!   error, Phase R1's precedent).
//! - `SendAgent` → [`SessionEngine::send_agent`] →
//!   `selora_ai::spawn_agent_turn` over `ClientChatStream` +
//!   `RouterToolRunner` (the `ToolContext` rooted at the workspace root, so
//!   every tool path is contained, §17.2). The loop's events map to
//!   `ToolCallStarted` / `ToolResult` / `ApprovalNeeded` /
//!   `AgentStateChanged` / `ChatDelta` / `AgentTurnDone`.
//! - `AgentDecision` → [`SessionEngine::agent_decision`] → the parked
//!   approval is validated and consumed (`validate_and_consume_approval`)
//!   *before* the decision is forwarded on the turn's decision channel —
//!   single-use, strictly in Rust. `Stop` needs no pending card and is
//!   forwarded regardless (§17.4: the Stop button is live for the whole
//!   turn).
//! - `StopChat` routes to either kind: a chat's cancel watch flips (the
//!   request future is dropped, the SSE connection closes); an agent turn's
//!   decision channel gets `Stop` (the loop kills bash children through the
//!   router's abort handle). One stop story.
//!
//! Invariants (the Z3 brief's gate list):
//! - approval ids are consumed exactly once, on the matching decision only;
//! - the API key never crosses into an event (every error string that could
//!   carry it is redacted at the seam);
//! - one in-flight request per session (`session_busy` otherwise);
//! - every event rides the shared monotonic sequence (the `EventLog`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use selora_ai::{
    AgentSessionMode, AgentSessionState, AgentSessionStore, AgentSummary, AgentTurnState,
    ApprovalCard, ChatEvent, ChatMessage, ChatReceipt, ChatRequest, CompactionConfig,
    CompactionError, Model, ParkedCard, RouterToolRunner, ToolContext, ToolRunner, ToolSpec,
    TranscriptEntry, TurnCaps, TurnInput,
};
use selora_core::SeloraError;
use selora_protocol::{
    validate_and_consume_approval, AgentCompactionFailedDto, AgentModeDto, AgentParkSummaryDto,
    AgentPhase, AgentSessionSummaryDto, AgentStateDto, AgentTurnDoneDto, ApprovalDto, ApprovalId,
    ApprovalNeededDto, ChatCompletedDto, ChatDeltaDto, ChatFailedDto, ChatRequestDto, ChatStateDto,
    ConnectionStatus, DecisionDto, DecisionPromptDto, FrontendEvent, MessageDto, MessageId,
    MessageRole, ModelDto, ModelsDto, PlanApprovalDto, PlanPhaseDto, PlanStateDto,
    ProtocolErrorDto, QuestionPromptOptionDto, RequestId, SeloraStatusDto, SessionDto, SessionId,
    SubagentArtifactDto, SubagentCompletedDto, SubagentPolicyDto, TodoDto, TodoStatusDto, TodosDto,
    ToolCallStartedDto, ToolResultDto, TurnId, WorkflowApprovalDto,
};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::state::EventLog;

/// The read-only half of the §17.1 manifest — `explore` mode arms exactly
/// these (the model cannot request a side effect). Phase 9b's todo tools
/// are session state, not workspace writes, so they stay available.
const EXPLORE_TOOLS: [&str; 12] = [
    "read_file",
    "list_dir",
    "fuzzy_find",
    "search_files",
    "web_search",
    "web_fetch",
    "todo_read",
    "todo_write",
    "manage_todo",
    "wait",
    "enter_plan_mode",
    "exit_plan_mode",
];

/// Transcript bound (the egui panel's 100-entry cap, R3) on the web shell's
/// in-memory model.
const MAX_SESSION_MESSAGES: usize = 100;

/// How many trailing transcript messages ride each request (the egui
/// restore's 20-turn precedent keeps prompts bounded without a settings
/// field).
const REQUEST_HISTORY_MESSAGES: usize = 20;

/// Where the gateway key comes from. Production reads the OS keyring at
/// send time (the key's only home, ARCHITECTURE.md §7.2); the hermetic
/// tests pin a fixed key (`None` = first-run, never connected) so no test
/// ever touches the real Credential Manager.
#[derive(Clone)]
pub(crate) enum ApiKeySource {
    Keyring,
    #[cfg(test)]
    Fixed(Option<String>),
}

impl ApiKeySource {
    fn get(&self) -> Result<Option<String>, ProtocolErrorDto> {
        match self {
            Self::Keyring => {
                match selora_settings::keyring::api_key_get() {
                    Ok(Some(key)) => {
                        let trimmed = key.trim();
                        if !trimmed.is_empty() {
                            return Ok(Some(trimmed.to_string()));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return Err(ProtocolErrorDto {
                            code: "keyring_unavailable".into(),
                            message: format!("The OS keyring could not be read: {error}"),
                            retryable: true,
                        });
                    }
                }
                if let Ok(key) = std::env::var("SELORA_API_KEY") {
                    let trimmed = key.trim();
                    if !trimmed.is_empty() {
                        return Ok(Some(trimmed.to_string()));
                    }
                }
                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                    let trimmed = key.trim();
                    if !trimmed.is_empty() {
                        return Ok(Some(trimmed.to_string()));
                    }
                }
                Ok(None)
            }
            #[cfg(test)]
            Self::Fixed(key) => Ok(key.clone()),
        }
    }
}

/// Everything the engine needs from the loaded settings, extracted once at
/// construction (base URL and the `agent.*` caps are not editable through
/// the web shell in Z3).
pub(crate) struct EngineConfig {
    pub base_url: String,
    pub active_model: Option<String>,
    pub agent: selora_settings::AgentSettings,
    pub key_source: ApiKeySource,
}

/// One in-flight request. Registered synchronously in `dispatch` (so the
/// busy/approval guards are race-free) and removed by the relay at its
/// terminal event.
enum ActiveRequest {
    Chat {
        session_id: SessionId,
        turn_id: TurnId,
        cancel: watch::Sender<bool>,
    },
    Agent {
        session_id: SessionId,
        turn_id: TurnId,
        decisions: mpsc::UnboundedSender<selora_ai::TurnDecision>,
    },
}

impl ActiveRequest {
    fn session_id(&self) -> &SessionId {
        match self {
            Self::Chat { session_id, .. } | Self::Agent { session_id, .. } => session_id,
        }
    }
    fn turn_id(&self) -> &TurnId {
        match self {
            Self::Chat { turn_id, .. } | Self::Agent { turn_id, .. } => turn_id,
        }
    }
}

/// One finalized transcript entry (the streaming half lives in the relay
/// until it lands here at the terminal event).
struct StoredMessage {
    message_id: MessageId,
    role: MessageRole,
    text: String,
    /// How this user message was dispatched, so `RetryMessage` can re-send the
    /// same content through the same path exactly once. Assistant messages
    /// carry `None`.
    retry: Option<RetrySpec>,
    /// Assistant-side tool calls (restored sessions keep the pairing).
    tool_calls: Option<Vec<selora_ai::ToolCall>>,
    /// The call a tool-result message answers.
    tool_call_id: Option<String>,
    ok: bool,
    timestamp: u64,
}

/// The dispatch path a user message took. Capability fields are copied from
/// the original request — a retry must not silently change mode, model, or
/// reasoning depth.
#[derive(Clone)]
enum RetrySpec {
    Chat {
        model_id: Option<String>,
    },
    Agent {
        model_id: Option<String>,
        mode: AgentModeDto,
        reasoning_effort: Option<String>,
        system_instructions: Option<String>,
        temperature: Option<f64>,
    },
}

/// Which stored user message a retry re-dispatches. The send paths truncate
/// the tail and register the new turn under one engine lock, so a concurrent
/// send cannot slip between the truncation and the dispatch.
#[derive(Clone)]
struct RetryTarget {
    session_id: SessionId,
    message_id: MessageId,
}

struct ChatSession {
    title: String,
    messages: Vec<StoredMessage>,
    active_request: Option<RequestId>,
    /// The turn's parked approval (§17.4) — `None` while none is pending.
    /// Consumed exactly once by a matching `AgentDecision`.
    pending_approval: Option<ApprovalDto>,
    /// Phase 9b: session-scoped plan mode + todos. Shared with the turn's
    /// `ToolContext`, so tool writes land here and the bootstrap hydrates
    /// them after a reload.
    plan_store: selora_ai::PlanStore,
    /// The parked card itself (needed to persist/restore the resume view).
    pending_card: Option<ApprovalCard>,
    pending_tool_name: Option<String>,
    /// The restored park's underlying card (resume renders it; a decision
    /// expires without reaching any turn).
    restored_park: bool,
    /// Monotonic recency stamp (bootstrap's `active_session_id` = the most
    /// recently touched session).
    last_activity: u64,
    // ---- Phase 9d: durable session fields -----------------------------
    created_at: u64,
    updated_at: u64,
    model: Option<String>,
    mode: AgentModeDto,
    /// Session full access. Deliberately never written to disk (the store
    /// has no field for it) and reset on switch/new/restart.
    auto_approve: bool,
    /// How many `messages` entries have already been appended to the
    /// transcript. Tool-call/tool-result records are appended live and are
    /// not part of this count.
    durable_entries: usize,
    /// `true` for an agent session (persisted under `.selora/agent-sessions`).
    /// Plain chat sessions stay in memory.
    durable: bool,
    /// The last successful compaction cursor (full transcript stays on disk).
    compaction: selora_ai::CompactionCursor,
}

impl ChatSession {
    fn new(title: String, now: u64) -> Self {
        Self {
            title,
            messages: Vec::new(),
            active_request: None,
            pending_approval: None,
            pending_card: None,
            pending_tool_name: None,
            restored_park: false,
            last_activity: now,
            created_at: now,
            updated_at: now,
            model: None,
            mode: AgentModeDto::Execute,
            plan_store: selora_ai::new_store(),
            auto_approve: false,
            durable_entries: 0,
            durable: false,
            compaction: selora_ai::CompactionCursor::default(),
        }
    }
}

/// §19.3: one child card's routing facts. The runtime owns the single-use
/// decision slot; this record lets the engine refuse a stale session or the
/// wrong card type before forwarding.
#[derive(Clone)]
struct SubagentCardRoute {
    request_id: RequestId,
    session_id: SessionId,
    is_question: bool,
}

struct EngineInner {
    base_url: String,
    active_model: Option<String>,
    sessions: BTreeMap<SessionId, ChatSession>,
    active: BTreeMap<RequestId, ActiveRequest>,
    models: Vec<Model>,
    connection: ConnectionStatus,
    activity_counter: u64,
    /// The session the user explicitly selected (new/switch/send). While a
    /// turn is live, `chat_state` prefers that turn's session so the frontend
    /// always routes events to the stream it can see.
    active_session: Option<SessionId>,
}

/// The Z3 session engine. Every mutation locks [`EngineInner`] briefly on
/// the calling thread — never across an await (the relays only call the
/// `finish_*`/`set_*` seams between awaits).
pub(crate) struct SessionEngine {
    inner: Mutex<EngineInner>,
    config: EngineConfig,
    /// Phase 9e: the session-scoped background subagent runtime. It
    /// outlives individual turns, so a child started by one turn can finish
    /// while the user edits in another. The `wait` tool reads it through
    /// `ToolContext`.
    subagents: selora_ai::SubagentRuntime,
    /// §19.3: the one relay receiver for the runtime's completion and card
    /// events. Taken by the first agent turn; the relay then lives for the
    /// engine's lifetime so a child may finish while the user edits.
    subagent_events: Mutex<Option<mpsc::UnboundedReceiver<selora_ai::SubagentEvent>>>,
    /// Child cards waiting on the parent's decision rail. The runtime owns the
    /// single-use decision slot; this map only validates request/session/card
    /// type before forwarding.
    subagent_cards: Mutex<BTreeMap<ApprovalId, SubagentCardRoute>>,
    /// The agent tools' root. The Z3 shell has no folder picker (that lands
    /// in Z4 with the workbench), so `SELORA_WORKSPACE_ROOT` is the one way
    /// to point the tools at a real tree today; without it `SendAgent`
    /// answers the typed `no_workspace` error.
    workspace_root: Mutex<Option<PathBuf>>,
}

impl SessionEngine {
    pub(crate) fn new(config: EngineConfig) -> Self {
        let initial_key = config.key_source.get().ok().flatten();
        // The key stays in the keyring / engine config: it is never copied
        // into the process environment, so LSP servers and tool children
        // cannot inherit it.
        let (initial_connection, initial_models) = if initial_key.is_some() {
            let models = crate::commands::synthesize_provider_models(&config.base_url);
            (ConnectionStatus::Connected, models)
        } else {
            (ConnectionStatus::Disconnected, Vec::new())
        };

        let initial_active_model = config
            .active_model
            .clone()
            .filter(|m| {
                initial_models
                    .iter()
                    .any(|model| model.id == *m || model.id.eq_ignore_ascii_case(m))
            })
            .or_else(|| initial_models.first().map(|m| m.id.clone()))
            .or_else(|| config.active_model.clone());

        let runtime = selora_ai::SubagentRuntime::new();
        let (tx, rx) = mpsc::unbounded_channel();
        runtime.set_event_sink(tx);
        Self {
            inner: Mutex::new(EngineInner {
                base_url: config.base_url.clone(),
                active_model: initial_active_model,
                sessions: BTreeMap::new(),
                active: BTreeMap::new(),
                models: initial_models,
                connection: initial_connection,
                activity_counter: 0,
                active_session: None,
            }),
            config,
            workspace_root: Mutex::new(None),
            subagents: runtime,
            subagent_events: Mutex::new(Some(rx)),
            subagent_cards: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn set_workspace_root(&self, root: Option<PathBuf>) {
        *self.workspace_root.lock().expect("workspace root lock") = root;
    }

    // -----------------------------------------------------------------
    // Phase 9d — durable sessions
    // -----------------------------------------------------------------

    /// The store rooted at the live workspace. No workspace means no agent
    /// session can exist (the same rule as `SendAgent`).
    fn store(&self) -> Option<AgentSessionStore> {
        let root = self
            .workspace_root
            .lock()
            .expect("workspace root lock")
            .clone()?;
        let store = AgentSessionStore::new(root);
        match self.config.key_source.get().ok().flatten() {
            Some(key) => Some(store.with_redaction_key(key)),
            None => Some(store),
        }
    }

    /// The resume picker's list (read-only; newest first).
    pub(crate) fn agent_sessions(&self) -> Vec<AgentSessionSummaryDto> {
        let Some(store) = self.store() else {
            return Vec::new();
        };
        store
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|summary| {
                Some(AgentSessionSummaryDto {
                    session_id: SessionId::new(summary.session_id).ok()?,
                    title: summary.title,
                    updated_at: summary.updated_at,
                    model: summary.model,
                    mode: summary.mode.as_str().to_owned(),
                    pending: summary.pending.map(|pending| AgentParkSummaryDto {
                        kind: pending.kind,
                        summary: pending.summary,
                    }),
                    compacted: summary.compacted,
                    message_count: summary.message_count,
                })
            })
            .collect()
    }

    /// Persist one session's new messages, metadata, and todos.
    ///
    /// Appends are incremental (one fsynced record per new message); the
    /// state/todos files are atomic replacements. A failed append leaves the
    /// durable counter unchanged so the next flush retries it.
    fn flush_session(&self, session_id: &SessionId) {
        let Some(store) = self.store() else {
            return;
        };
        let (start, entries, state, todos) = {
            let inner = self.inner.lock().expect("engine lock");
            let Some(session) = inner.sessions.get(session_id) else {
                return;
            };
            if !session.durable {
                return;
            }
            let start = session.durable_entries.min(session.messages.len());
            let entries: Vec<TranscriptEntry> = session.messages[start..]
                .iter()
                .map(message_entry)
                .collect();
            (
                start,
                entries,
                session_state(session_id, session),
                selora_ai::plan::snapshot(&session.plan_store).todos,
            )
        };
        let mut appended = 0usize;
        for entry in &entries {
            if store.append(session_id.as_str(), entry).is_err() {
                break;
            }
            appended += 1;
        }
        let _ = store.save_state(&state);
        let _ = store.save_todos(session_id.as_str(), &todos);
        if appended > 0 {
            let mut inner = self.inner.lock().expect("engine lock");
            if let Some(session) = inner.sessions.get_mut(session_id) {
                let end = (start + appended).min(session.messages.len());
                session.durable_entries = end;
            }
        }
    }

    /// Append a live tool-call batch to the transcript (not part of the
    /// in-memory message tail).
    fn record_tool_calls(&self, session_id: &SessionId, calls: Vec<selora_ai::ToolCall>) {
        let Some(store) = self.store() else {
            return;
        };
        let _ = store.append(
            session_id.as_str(),
            &TranscriptEntry::ToolCalls {
                id: Uuid::new_v4().to_string(),
                timestamp: selora_ai::now_millis(),
                calls,
            },
        );
    }

    /// Append one tool result to the transcript.
    fn record_tool_result(&self, session_id: &SessionId, result: &selora_ai::ToolResult) {
        let Some(store) = self.store() else {
            return;
        };
        let _ = store.append(
            session_id.as_str(),
            &TranscriptEntry::Tool {
                id: Uuid::new_v4().to_string(),
                timestamp: selora_ai::now_millis(),
                call_id: result.id.clone(),
                ok: result.ok,
                content: result.content.clone(),
            },
        );
    }

    /// Record a successful compaction cursor. The transcript is not rewritten.
    fn record_compaction(&self, session_id: &SessionId, cursor: selora_ai::CompactionCursor) {
        {
            let mut inner = self.inner.lock().expect("engine lock");
            if let Some(session) = inner.sessions.get_mut(session_id) {
                session.compaction = cursor;
                session.updated_at = selora_ai::now_millis();
            }
        }
        self.flush_session(session_id);
    }

    /// Switch the dock to a session, restoring it from disk when it is not in
    /// memory. Every session's auto-approve flag is cleared first: trust never
    /// transfers across a switch.
    pub(crate) fn switch_session(&self, session_id: &SessionId) -> Result<(), ProtocolErrorDto> {
        let mut inner = self.inner.lock().expect("engine lock");
        if !inner.sessions.contains_key(session_id) {
            let loaded = self.load_session_from_disk(session_id)?;
            inner.sessions.insert(session_id.clone(), loaded);
        }
        inner.activity_counter += 1;
        let activity = inner.activity_counter;
        for session in inner.sessions.values_mut() {
            session.auto_approve = false;
        }
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.last_activity = activity;
        }
        inner.active_session = Some(session_id.clone());
        Ok(())
    }

    /// Create a fresh session and make it active. Its auto-approve flag is
    /// off by construction.
    pub(crate) fn new_session(&self) -> Result<SessionId, ProtocolErrorDto> {
        let store = self.store().ok_or_else(no_workspace)?;
        let session_id =
            SessionId::new(format!("sess-{}", Uuid::new_v4())).expect("uuid session id is valid");
        let title = "New Agent Session".to_owned();
        let now = selora_ai::now_millis();
        let state = store
            .create(
                session_id.as_str(),
                &title,
                self.active_model_id().as_deref(),
                selora_ai::AgentSessionMode::Execute,
            )
            .map_err(|error| session_persistence_error(&error))?;
        let mut session = ChatSession::new(title, now);
        session.mode = AgentModeDto::Execute;
        session.model = state.model;
        session.durable = true;
        let mut inner = self.inner.lock().expect("engine lock");
        inner.activity_counter += 1;
        session.last_activity = inner.activity_counter;
        for open in inner.sessions.values_mut() {
            open.auto_approve = false;
        }
        inner.sessions.insert(session_id.clone(), session);
        inner.active_session = Some(session_id.clone());
        Ok(session_id)
    }

    /// Delete a session. Durable agent sessions also lose their on-disk store,
    /// so a deleted tab cannot be resumed later. A running turn refuses the
    /// delete (the user must stop it first).
    ///
    /// The active session changes only when the deleted session *was* active:
    /// deleting a background tab leaves the user's selection alone. The
    /// replacement is a real session the engine still owns (the most recently
    /// touched one), never a synthesized placeholder.
    pub(crate) fn delete_session(&self, session_id: &SessionId) -> Result<(), ProtocolErrorDto> {
        let store = self.store();
        let mut inner = self.inner.lock().expect("engine lock");
        let open = inner.sessions.get(session_id);
        if let Some(session) = open {
            if session.active_request.is_some()
                || inner
                    .active
                    .values()
                    .any(|request| request.session_id() == session_id)
            {
                return Err(session_busy());
            }
        }
        let known = open.is_some();
        let durable = open.is_none_or(|session| session.durable);
        match store.as_ref() {
            Some(store) => {
                // A disk-only durable session is known through its store; a
                // session that exists in neither place is a stale tab.
                if !known && !store.exists(session_id.as_str()) {
                    return Err(unknown_session(session_id));
                }
                if durable {
                    store
                        .delete(session_id.as_str())
                        .map_err(|error| session_persistence_error(&error))?;
                }
            }
            None if !known => return Err(unknown_session(session_id)),
            None => {}
        }
        inner.sessions.remove(session_id);
        if inner.active_session.as_ref() == Some(session_id) {
            inner.active_session = inner
                .sessions
                .iter()
                .max_by_key(|(_, candidate)| candidate.last_activity)
                .map(|(id, _)| id.clone());
        }
        // Normalize any dangling pointer (a session deleted without an active
        // marker must not keep leaking the old id into the bootstrap).
        if inner
            .active_session
            .as_ref()
            .is_some_and(|id| !inner.sessions.contains_key(id))
        {
            inner.active_session = None;
        }
        Ok(())
    }

    /// Arm/disarm session full access. The flag is session memory only.
    pub(crate) fn set_auto_approve(
        &self,
        session_id: &SessionId,
        enabled: bool,
    ) -> Result<(), ProtocolErrorDto> {
        let mut inner = self.inner.lock().expect("engine lock");
        let Some(session) = inner.sessions.get_mut(session_id) else {
            return Err(ProtocolErrorDto {
                code: "session_not_found".into(),
                message: "That agent session is not open.".into(),
                retryable: false,
            });
        };
        session.auto_approve = enabled;
        Ok(())
    }

    /// Mirror the live plan/todo store into the durable session record after
    /// a plan or todo event. Phase 9b's events are the only writer of these
    /// fields, so this is where a reload's plan spine is captured.
    fn sync_plan_store(&self, session_id: &SessionId) {
        {
            let mut inner = self.inner.lock().expect("engine lock");
            let Some(session) = inner.sessions.get_mut(session_id) else {
                return;
            };
            session.updated_at = selora_ai::now_millis();
        }
        self.flush_session(session_id);
    }

    /// Restore one session from disk into the in-memory shape.
    fn load_session_from_disk(
        &self,
        session_id: &SessionId,
    ) -> Result<ChatSession, ProtocolErrorDto> {
        let store = self.store().ok_or_else(no_workspace)?;
        let loaded = store
            .load(session_id.as_str())
            .map_err(|error| session_persistence_error(&error))?
            .ok_or_else(|| ProtocolErrorDto {
                code: "session_not_found".into(),
                message: "That agent session does not exist on disk.".into(),
                retryable: false,
            })?;
        let now = selora_ai::now_millis();
        let mut session = ChatSession::new(loaded.state.title.clone(), now);
        session.created_at = loaded.state.created_at;
        session.updated_at = loaded.state.updated_at;
        session.durable = true;
        session.model = loaded.state.model.clone();
        session.mode = mode_from_disk(loaded.state.mode);
        // Phase 9b's live store hydrates from the durable record: the
        // resume picker must reopen the same plan phase, draft and todos.
        {
            let mut live = selora_ai::plan::lock(&session.plan_store);
            live.phase = if loaded.state.plan.status == selora_ai::PlanStatus::Proposed {
                selora_ai::PlanPhase::Planning
            } else {
                selora_ai::PlanPhase::Executing
            };
            live.plan = loaded
                .state
                .plan
                .plan
                .as_deref()
                .and_then(plan_draft_from_disk);
            live.todos = loaded.todos.clone();
        }
        session.compaction = loaded.state.compaction;
        session.messages = stored_transcript(&loaded.transcript.entries);
        session.durable_entries = session.messages.len();
        // A restored park is shown once. It has no live request, so any
        // decision expires instead of executing (see `agent_decision`).
        if let Some(park) = loaded.state.parks.first() {
            match park {
                ParkedCard::Approval {
                    approval_id,
                    tool_name,
                    card,
                    request_id,
                    turn_id,
                    ..
                } => {
                    if let Ok(approval_id) = ApprovalId::new(approval_id.clone()) {
                        let request_id = request_id
                            .as_deref()
                            .and_then(|id| RequestId::new(id).ok())
                            .unwrap_or_else(|| {
                                RequestId::new(format!("restored-{}", approval_id.as_str()))
                                    .expect("restored request id")
                            });
                        let turn_id = turn_id
                            .as_deref()
                            .and_then(|id| TurnId::new(id).ok())
                            .unwrap_or_else(|| {
                                TurnId::new(format!("restored-{}", approval_id.as_str()))
                                    .expect("restored turn id")
                            });
                        session.pending_approval = Some(ApprovalDto {
                            approval_id: approval_id.clone(),
                            request_id,
                            turn_id,
                            session_id: session_id.clone(),
                            summary: park.summary(),
                            can_approve_all_this_turn: false,
                            plan: approval_plan(card),
                            decision_prompt: approval_prompt(card),
                        });
                        session.pending_card = Some(card.clone());
                        session.pending_tool_name = Some(tool_name.clone());
                        session.restored_park = true;
                    }
                }
                ParkedCard::Question { .. } => {}
            }
        }
        Ok(session)
    }

    pub(crate) fn base_url(&self) -> String {
        self.inner.lock().expect("engine lock").base_url.clone()
    }

    /// The live agent request/turn for a session, when one is running. Child
    /// cards group under it; a background child may outlive the turn, in
    /// which case the relay mints its own ids.
    fn active_request_for_session(&self, session_id: &SessionId) -> Option<(RequestId, TurnId)> {
        let inner = self.inner.lock().expect("engine lock");
        inner
            .active
            .iter()
            .find_map(|(request_id, active)| match active {
                ActiveRequest::Agent {
                    session_id: owner,
                    turn_id,
                    ..
                } if owner == session_id => Some((request_id.clone(), turn_id.clone())),
                _ => None,
            })
    }

    pub(crate) fn set_base_url(&self, base_url: String) {
        self.inner.lock().expect("engine lock").base_url = base_url;
    }

    pub(crate) fn set_models_and_connection(
        &self,
        models: Vec<Model>,
        connection: ConnectionStatus,
        active_model: Option<String>,
    ) {
        let mut inner = self.inner.lock().expect("engine lock");
        inner.models = models;
        inner.connection = connection;
        inner.active_model = active_model;
    }

    // -----------------------------------------------------------------
    // Snapshot accessors (the bootstrap)
    // -----------------------------------------------------------------

    pub(crate) fn connection(&self) -> ConnectionStatus {
        self.inner.lock().expect("engine lock").connection
    }

    pub(crate) fn models_dto(&self) -> Vec<ModelDto> {
        self.inner
            .lock()
            .expect("engine lock")
            .models
            .iter()
            .map(model_dto)
            .collect()
    }

    pub(crate) fn active_model_id(&self) -> Option<String> {
        self.inner.lock().expect("engine lock").active_model.clone()
    }

    /// The bootstrap's `chat` block: every session, the flattened
    /// transcript, the live request/turn ids, the newest parked approval,
    /// and the active session's authoritative todos.
    pub(crate) fn chat_state(&self) -> ChatStateDto {
        let redaction_key = self
            .config
            .key_source
            .get()
            .ok()
            .flatten()
            .unwrap_or_default();
        let inner = self.inner.lock().expect("engine lock");
        let mut sessions = Vec::new();
        let mut messages = Vec::new();
        let mut active_session_id = inner
            .active_session
            .clone()
            .filter(|id| inner.sessions.contains_key(id));
        let mut active_request_id = None;
        let mut active_turn_id = None;
        let mut pending_approval = None;
        let mut pending_approvals: Vec<(SessionId, ApprovalDto)> = Vec::new();
        // Recency order (most recent last): `active_session_id` picks the
        // most recently touched session that is still running a request.
        let mut ordered: Vec<(&SessionId, &ChatSession)> = inner.sessions.iter().collect();
        ordered.sort_by_key(|(_, session)| session.last_activity);
        for (session_id, session) in ordered {
            let plan = selora_ai::plan::snapshot(&session.plan_store);
            sessions.push(SessionDto {
                session_id: session_id.clone(),
                title: session.title.clone(),
                message_ids: session
                    .messages
                    .iter()
                    .map(|message| message.message_id.clone())
                    .collect(),
                active: session.active_request.is_some(),
                plan_phase: plan_phase_dto(plan.phase),
                plan: plan
                    .plan
                    .as_ref()
                    .map(|draft| plan_approval_dto(draft, &redaction_key)),
                todos: plan.todos.iter().map(todo_dto).collect(),
            });
            for message in &session.messages {
                messages.push(MessageDto {
                    message_id: message.message_id.clone(),
                    session_id: session_id.clone(),
                    role: message.role,
                    text: message.text.clone(),
                });
            }
            if let Some(request_id) = &session.active_request {
                if active_session_id.is_none() {
                    active_session_id = Some(session_id.clone());
                }
                if let Some(active) = inner
                    .active
                    .get(request_id)
                    .filter(|_| active_session_id.as_ref() == Some(session_id))
                {
                    active_request_id = Some(request_id.clone());
                    active_turn_id = Some(active.turn_id().clone());
                }
            }
            if let Some(approval) = &session.pending_approval {
                pending_approvals.push((session_id.clone(), approval.clone()));
            }
        }
        // Prefer the active session's park; otherwise the first in recency
        // order (the pre-9d behaviour).
        if let Some(active) = &active_session_id {
            pending_approval = pending_approvals
                .iter()
                .find(|(session, _)| session == active)
                .map(|(_, approval)| approval.clone());
        }
        ChatStateDto {
            sessions,
            messages,
            active_session_id: active_session_id.clone(),
            active_request_id,
            active_turn_id,
            pending_approval,
            pending_approvals: pending_approvals
                .into_iter()
                .map(|(_, approval)| approval)
                .collect(),
            todos: active_session_id
                .as_ref()
                .and_then(|id| inner.sessions.get(id))
                .map(|session| {
                    selora_ai::plan::snapshot(&session.plan_store)
                        .todos
                        .iter()
                        .map(todo_dto)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The session the engine currently considers active (new/switch/send).
    pub(crate) fn active_session_id(&self) -> Option<SessionId> {
        let inner = self.inner.lock().expect("engine lock");
        inner.active_session.clone().or_else(|| {
            inner
                .sessions
                .iter()
                .max_by_key(|(_, session)| session.last_activity)
                .map(|(id, _)| id.clone())
        })
    }

    // -----------------------------------------------------------------
    // NewSession / SwitchSession
    // -----------------------------------------------------------------

    // -----------------------------------------------------------------
    // RetryMessage
    // -----------------------------------------------------------------

    /// Remove the failed tail (the named user message and everything after
    /// it) and re-dispatch that message's exact content once. The named
    /// message must be an existing user turn with a recorded dispatch path;
    /// a live turn or an unknown/non-user message is refused without
    /// touching the transcript.
    pub(crate) fn retry_message<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<RequestId, ProtocolErrorDto> {
        if envelope_session
            .as_ref()
            .is_some_and(|session| session != &session_id)
        {
            return Err(stale_session());
        }
        if self
            .inner
            .lock()
            .expect("engine lock")
            .sessions
            .get(&session_id)
            .is_none_or(|session| session.durable)
        {
            let request_id = RequestId::new(Uuid::new_v4().to_string()).expect("uuid is valid");
            self.retry_durable_message(log, app, request_id.clone(), session_id, message_id)?;
            return Ok(request_id);
        }
        let (spec, text) = {
            let inner = self.inner.lock().expect("engine lock");
            let session = inner
                .sessions
                .get(&session_id)
                .ok_or_else(|| unknown_session(&session_id))?;
            if session.active_request.is_some() {
                return Err(session_busy());
            }
            let message = session
                .messages
                .iter()
                .find(|message| message.message_id == message_id)
                .ok_or_else(|| unknown_message(&message_id))?;
            if message.role != MessageRole::User {
                return Err(not_a_user_message(&message_id));
            }
            let spec = message.retry.clone().ok_or_else(|| ProtocolErrorDto {
                code: "retry_unavailable".into(),
                message: "That message has no recorded dispatch to retry.".into(),
                retryable: false,
            })?;
            (spec, message.text.clone())
        };
        if envelope_session.is_some_and(|session| session != session_id) {
            return Err(stale_session());
        }

        let request_id = RequestId::new(Uuid::new_v4().to_string()).expect("uuid is valid");
        let target = RetryTarget {
            session_id: session_id.clone(),
            message_id,
        };
        match spec {
            RetrySpec::Chat { model_id } => self.send_chat_inner(
                log,
                app,
                Some(session_id.clone()),
                request_id.clone(),
                ChatRequestDto {
                    session_id,
                    message: text,
                    model_id,
                },
                Some(target),
            )?,
            RetrySpec::Agent {
                model_id,
                mode,
                reasoning_effort,
                system_instructions,
                temperature,
            } => self.send_agent_inner(
                log,
                app,
                Some(session_id.clone()),
                request_id.clone(),
                selora_protocol::AgentRequestDto {
                    session_id,
                    prompt: text,
                    model_id,
                    mode,
                    reasoning_effort,
                    system_instructions,
                    temperature,
                    editor_context: None,
                },
                Some(target),
            )?,
        }
        Ok(request_id)
    }

    // -----------------------------------------------------------------
    // SendChat
    // -----------------------------------------------------------------

    pub(crate) fn send_chat<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        request_id: RequestId,
        payload: ChatRequestDto,
    ) -> Result<(), ProtocolErrorDto> {
        self.send_chat_inner(log, app, envelope_session, request_id, payload, None)
    }

    fn send_chat_inner<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        request_id: RequestId,
        payload: ChatRequestDto,
        retry: Option<RetryTarget>,
    ) -> Result<(), ProtocolErrorDto> {
        if envelope_session.is_some_and(|session| session != payload.session_id) {
            return Err(stale_session());
        }
        let api_key = self.fetch_api_key()?;
        let model_id = payload
            .model_id
            .clone()
            .or_else(|| self.active_model_id())
            .ok_or_else(no_model_selected)?;

        let mut inner = self.inner.lock().expect("engine lock");
        if let Some(target) = &retry {
            truncate_for_retry(&mut inner, target)?;
        }
        let spec = RetrySpec::Chat {
            model_id: payload.model_id.clone(),
        };
        let session = begin_turn(
            &mut inner,
            &request_id,
            &payload.session_id,
            &payload.message,
            Some(spec),
        )?;
        // One turn, one streaming assistant message. The conversation is
        // the transcript's tail — `begin_turn` already landed the user's
        // message on it.
        let turn_id = fresh_turn_id();
        let message_id = fresh_message_id();
        let conversation: Vec<ChatMessage> = conversation_tail(session);
        let request = ChatRequest::new(model_id, conversation);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        inner.active.insert(
            request_id.clone(),
            ActiveRequest::Chat {
                session_id: payload.session_id.clone(),
                turn_id: turn_id.clone(),
                cancel: cancel_tx,
            },
        );
        drop(inner);

        tauri::async_runtime::spawn(chat_relay(ChatRelayCtx {
            log,
            engine: Arc::clone(self),
            app,
            request_id,
            session_id: payload.session_id,
            turn_id,
            message_id,
            base_url: self.base_url(),
            api_key,
            request,
            cancel_rx,
        }));
        Ok(())
    }

    // -----------------------------------------------------------------
    // SendAgent
    // -----------------------------------------------------------------

    pub(crate) fn send_agent<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        request_id: RequestId,
        payload: selora_protocol::AgentRequestDto,
    ) -> Result<(), ProtocolErrorDto> {
        self.send_agent_inner(log, app, envelope_session, request_id, payload, None)
    }

    fn send_agent_inner<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        envelope_session: Option<SessionId>,
        request_id: RequestId,
        payload: selora_protocol::AgentRequestDto,
        retry: Option<RetryTarget>,
    ) -> Result<(), ProtocolErrorDto> {
        if envelope_session.is_some_and(|session| session != payload.session_id) {
            return Err(stale_session());
        }
        let api_key = self.fetch_api_key()?;
        let model = self.resolve_model(payload.model_id.as_deref())?;
        let root = self
            .workspace_root
            .lock()
            .expect("workspace root lock")
            .clone()
            .ok_or_else(no_workspace)?;
        let turn_id = fresh_turn_id();

        let mut inner = self.inner.lock().expect("engine lock");
        if let Some(target) = &retry {
            truncate_for_retry(&mut inner, target)?;
        }
        let spec = RetrySpec::Agent {
            model_id: payload.model_id.clone(),
            mode: payload.mode,
            reasoning_effort: payload.reasoning_effort.clone(),
            system_instructions: payload.system_instructions.clone(),
            temperature: payload.temperature,
        };
        let session = begin_turn(
            &mut inner,
            &request_id,
            &payload.session_id,
            &payload.prompt,
            Some(spec),
        )?;
        // Mode and model are durable session facts. Full access is not: the
        // flag lives only in this `ChatSession` and is reset on switch/new.
        session.model = Some(model.id.clone());
        session.mode = persist_mode(payload.mode);
        session.auto_approve = matches!(payload.mode, AgentModeDto::Auto);
        session.durable = true;
        session.updated_at = selora_ai::now_millis();
        if session.restored_park {
            // A new turn supersedes a restored park: it can never execute.
            session.pending_approval = None;
            session.pending_card = None;
            session.pending_tool_name = None;
            session.restored_park = false;
        }
        let auto_approve = session.auto_approve;
        let open_todos = selora_ai::plan::snapshot(&session.plan_store).todos;
        let sys_prompt = system_prompt(&payload, &root);
        let mut conversation: Vec<ChatMessage> = vec![ChatMessage::system(sys_prompt)];
        conversation.extend(conversation_tail(session));
        let plan_store = session.plan_store.clone();
        self.subagents.begin_turn(payload.session_id.as_str());
        drop(inner);
        self.flush_session(&payload.session_id);

        // The turn: 5c's loop over 5a's client and 5b's router, rooted and
        // capped exactly like the egui shell's `spawn_agent` (§17.6).
        let caps = TurnCaps {
            max_tool_iterations: self.config.agent.max_iterations,
            max_turn_secs: self.config.agent.max_turn_secs,
            max_cost_usd: self.config.agent.max_cost_usd,
            ..TurnCaps::spec_defaults(model.context_window)
        };
        let mut input = TurnInput::new(model.clone(), conversation)
            .with_caps(caps.clone())
            .with_manifest(manifest_for(payload.mode))
            .with_auto_approve_writes(auto_approve)
            .with_compaction(CompactionConfig::default())
            .with_open_todos(open_todos);
        if let Some(temp) = payload.temperature {
            input = input.with_temperature(temp);
        }
        if let Some(ref effort) = payload.reasoning_effort {
            input = input.with_reasoning_effort(effort.clone());
        }
        let tool_settings = agent_tool_settings(&self.config.agent, self.brave_key());
        // §19.3: a child never inherits the parent's command allowlist or
        // session auto-approve grant: `full` means writes and bash still card
        // in the parent session.
        let child_settings = selora_ai::AgentToolSettings {
            auto_approve_commands: Vec::new(),
            ..tool_settings.clone()
        };
        let backend: Arc<dyn selora_ai::SubagentBackend> = Arc::new(DesktopSubagentBackend {
            root: root.clone(),
            settings: child_settings,
            base_url: self.base_url(),
            api_key: api_key.clone(),
        });
        let tools = RouterToolRunner::new(
            ToolContext::new(root, tool_settings)
                .with_plan(plan_store)
                .with_subagents(self.subagents.clone(), payload.session_id.as_str())
                .with_subagent_backend(backend, model.clone(), caps.clone()),
        );
        let chat = selora_ai::ClientChatStream::new(selora_ai::SeloraClient::new(
            &self.base_url(),
            &api_key,
        ));
        let (events_tx, events_rx) = mpsc::unbounded_channel::<selora_ai::AgentEvent>();
        // The loop spawns on tauri's async runtime; the decision sender
        // stays here (parked in the active map) for `AgentDecision`.
        let tokio_handle = tauri::async_runtime::handle().inner().clone();
        let (decision_tx, join) =
            selora_ai::spawn_agent_turn(&tokio_handle, input, chat, tools, events_tx);
        {
            let mut inner = self.inner.lock().expect("engine lock");
            inner.active.insert(
                request_id.clone(),
                ActiveRequest::Agent {
                    session_id: payload.session_id.clone(),
                    turn_id: turn_id.clone(),
                    decisions: decision_tx,
                },
            );
        }

        self.ensure_subagent_relay(Arc::clone(&log), app.clone());
        tauri::async_runtime::spawn(agent_relay(AgentRelayCtx {
            log,
            engine: Arc::clone(self),
            app,
            request_id,
            session_id: payload.session_id,
            turn_id,
            api_key,
            events_rx,
            join,
        }));
        Ok(())
    }

    /// Take the runtime's one event receiver and start the session relay.
    /// Called by the first agent turn; later turns reuse the running relay.
    fn ensure_subagent_relay<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
    ) {
        let receiver = self
            .subagent_events
            .lock()
            .expect("subagent events lock")
            .take();
        let Some(events_rx) = receiver else {
            return;
        };
        tauri::async_runtime::spawn(subagent_relay(SubagentRelayCtx {
            log,
            engine: Arc::clone(self),
            app,
            events_rx,
        }));
    }

    /// Phase 9e: start one background child and emit its single typed
    /// completion event. The runtime outlives the parent turn; the relay
    /// only maps the one accepted result onto `selora://event`. A cancelled
    /// child emits nothing, and a late result is discarded by the runtime.
    ///
    /// The host-facing spawn tool (Phase 9c) calls this seam; keeping the
    /// relay here means every consumer gets the same event contract.
    #[allow(dead_code)]
    pub(crate) fn spawn_background_subagent<R: tauri::Runtime, F>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        request: selora_ai::SpawnRequest,
        work: F,
    ) -> Result<selora_ai::SubagentHandle, selora_ai::SpawnError>
    where
        F: std::future::Future<Output = selora_ai::ChildOutcome> + Send + 'static,
    {
        let child_id = request.child_id.clone();
        let handle = self.subagents.spawn_background(request, work)?;
        let runtime = self.subagents.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(completion) = runtime.wait_for_child(&child_id).await {
                let Ok(parent) = SessionId::new(completion.parent_session.clone()) else {
                    return;
                };
                log.record_session(
                    &app,
                    FrontendEvent::SubagentCompleted(subagent_completed_dto(completion)),
                    None,
                    parent,
                );
            }
        });
        Ok(handle)
    }

    // -----------------------------------------------------------------
    // StopChat / AgentDecision
    // -----------------------------------------------------------------

    /// One stop story for both kinds: a chat's cancel watch flips (the
    /// request future is dropped); an agent turn's decision channel gets
    /// `Stop` (the loop kills bash children via the router's abort handle).
    /// Stopping an unknown or finished request is a silent success — the
    /// terminal event already told the frontend how it ended.
    pub(crate) fn stop_chat(
        &self,
        request_id: &RequestId,
        envelope_session: Option<SessionId>,
    ) -> Result<(), ProtocolErrorDto> {
        let inner = self.inner.lock().expect("engine lock");
        if let Some(active) = inner.active.get(request_id) {
            if envelope_session.is_some_and(|session| session != *active.session_id()) {
                return Err(stale_session());
            }
            match active {
                ActiveRequest::Chat { cancel, .. } => {
                    let _ = cancel.send(true);
                }
                ActiveRequest::Agent { decisions, .. } => {
                    // Phase 9e: stopping the parent stops its children. The
                    // runtime drops any result that arrives afterwards.
                    let _ = decisions.send((Uuid::nil(), selora_ai::AgentDecision::Stop));
                    self.subagents.cancel_parent(active.session_id().as_str());
                }
            }
        }
        Ok(())
    }

    /// The approval gate: the parked card is validated and consumed
    /// strictly here — only the matching decision gets through, exactly
    /// once. `Stop` needs no card and forwards to whatever is running.
    #[cfg(test)]
    pub(crate) fn agent_decision(
        &self,
        request_id: &RequestId,
        approval_id: &ApprovalId,
        decision: DecisionDto,
        envelope_session: Option<SessionId>,
    ) -> Result<(), ProtocolErrorDto> {
        self.agent_decision_with_resolution(
            request_id,
            approval_id,
            decision,
            envelope_session,
            None,
        )
    }

    pub(crate) fn agent_decision_with_resolution(
        &self,
        request_id: &RequestId,
        approval_id: &ApprovalId,
        decision: DecisionDto,
        envelope_session: Option<SessionId>,
        resolution: Option<selora_protocol::AgentDecisionResolutionDto>,
    ) -> Result<(), ProtocolErrorDto> {
        let answer = match (decision, resolution) {
            (
                DecisionDto::Answer,
                Some(selora_protocol::AgentDecisionResolutionDto::Question { option_id, text }),
            ) => {
                let answer = [option_id.unwrap_or_default(), text.unwrap_or_default()].join("\n");
                if answer.trim().is_empty() || answer.len() > 8000 {
                    return Err(ProtocolErrorDto {
                        code: "invalid_answer".into(),
                        message: "An answer must contain 1 to 8000 bytes.".into(),
                        retryable: false,
                    });
                }
                Some(answer)
            }
            (DecisionDto::Answer, _) => {
                return Err(ProtocolErrorDto {
                    code: "invalid_answer".into(),
                    message: "A structured question answer is required.".into(),
                    retryable: false,
                })
            }
            (
                DecisionDto::Revise,
                Some(selora_protocol::AgentDecisionResolutionDto::Plan {
                    feedback: Some(feedback),
                    ..
                }),
            ) => {
                if feedback.len() > 8000 {
                    return Err(ProtocolErrorDto {
                        code: "invalid_answer".into(),
                        message: "Revision feedback exceeds 8000 bytes.".into(),
                        retryable: false,
                    });
                }
                Some(format!(
                    "Revision requested; remain in plan mode. User feedback: {feedback}"
                ))
            }
            _ => None,
        };
        // Stop applies without a pending card (§17.4: honored regardless of
        // which card it names — the nil id carries no card).
        if decision == DecisionDto::Stop {
            return self.stop_chat(request_id, envelope_session);
        }

        // §19.3: a child card lives on its own rail. Validate the request,
        // session and card type, then let the runtime consume the single-use
        // id before forwarding to the child's turn.
        if let Some(route) = self
            .subagent_cards
            .lock()
            .expect("subagent card lock")
            .get(approval_id)
            .cloned()
        {
            if &route.request_id != request_id
                || envelope_session
                    .as_ref()
                    .is_some_and(|session| session != &route.session_id)
            {
                return Err(stale_session());
            }
            let is_answer = decision == DecisionDto::Answer;
            if route.is_question != is_answer {
                return Err(ProtocolErrorDto {
                    code: "approval_card_mismatch".into(),
                    message: if route.is_question {
                        "This subagent card needs a question answer, not an approval decision."
                            .into()
                    } else {
                        "A question answer cannot approve this subagent card.".into()
                    },
                    retryable: false,
                });
            }
            self.subagent_cards
                .lock()
                .expect("subagent card lock")
                .remove(approval_id);
            let loop_id = Uuid::parse_str(approval_id.as_str()).map_err(|_| ProtocolErrorDto {
                code: "approval_mismatch".into(),
                message: "The approval id does not name a pending approval.".into(),
                retryable: false,
            })?;
            return self
                .subagents
                .decide(loop_id, (loop_id, map_decision(decision, answer)))
                .map_err(|error| ProtocolErrorDto {
                    code: "approval_not_pending".into(),
                    message: error.to_string(),
                    retryable: false,
                });
        }

        let mut inner = self.inner.lock().expect("engine lock");
        // The session whose parked card belongs to this request.
        let Some((session_id, session)) = inner
            .sessions
            .iter_mut()
            .find(|(_, session)| {
                session
                    .pending_approval
                    .as_ref()
                    .is_some_and(|approval| &approval.request_id == request_id)
            })
            .map(|(id, session)| (id.clone(), session))
        else {
            return Err(ProtocolErrorDto {
                code: "approval_not_pending".into(),
                message: "No approval is pending for this request.".into(),
                retryable: false,
            });
        };
        let pending_turn = session
            .pending_approval
            .as_ref()
            .expect("found by pending approval")
            .turn_id
            .clone();
        if envelope_session
            .as_ref()
            .is_some_and(|owner| owner != &session_id)
        {
            return Err(stale_session());
        }
        // §19.4: a question card is answered, never approved; an approval
        // card is never answered. Refuse the wrong card type before the
        // single-use gate consumes the id.
        let pending_is_question = session
            .pending_approval
            .as_ref()
            .expect("found by pending approval")
            .decision_prompt
            .is_some();
        if pending_is_question != (decision == DecisionDto::Answer) {
            return Err(ProtocolErrorDto {
                code: "approval_card_mismatch".into(),
                message: if pending_is_question {
                    "This card needs a question answer, not an approval decision.".into()
                } else {
                    "A question answer cannot approve this card.".into()
                },
                retryable: false,
            });
        }
        if decision == DecisionDto::ApproveAllThisTurn
            && session
                .pending_approval
                .as_ref()
                .is_some_and(|approval| !approval.can_approve_all_this_turn)
        {
            return Err(ProtocolErrorDto {
                code: "invalid_decision".into(),
                message: "This card requires its own decision and cannot grant blanket approval."
                    .into(),
                retryable: false,
            });
        }
        // A restored park has no live turn. Consume it exactly once and tell
        // the caller it expired — never execute the restored action.
        if session.restored_park {
            let matches = session
                .pending_approval
                .as_ref()
                .is_some_and(|approval| &approval.approval_id == approval_id);
            if !matches {
                return Err(ProtocolErrorDto {
                    code: "approval_mismatch".into(),
                    message: "The decision does not match the restored approval.".into(),
                    retryable: false,
                });
            }
            session.pending_approval = None;
            session.pending_card = None;
            session.pending_tool_name = None;
            let sid = session_id.clone();
            drop(inner);
            self.flush_session(&sid);
            return Err(ProtocolErrorDto {
                code: "park_expired".into(),
                message: "This approval was restored from disk and has already expired. Ask again to start a new turn.".into(),
                retryable: false,
            });
        }
        // The single-use gate: a mismatch leaves the card parked — only the
        // matching decision consumes it.
        validate_and_consume_approval(
            &mut session.pending_approval,
            request_id,
            &session_id,
            &pending_turn,
            approval_id,
            decision,
        )
        .map_err(|_| ProtocolErrorDto {
            code: "approval_mismatch".into(),
            message: "The decision does not match the pending approval.".into(),
            retryable: false,
        })?;
        session.pending_card = None;
        session.pending_tool_name = None;
        // The loop keys decisions by its own uuid approval id; a decision
        // that survived the gate names a uuid by construction.
        let Ok(loop_id) = Uuid::parse_str(approval_id.as_str()) else {
            return Err(ProtocolErrorDto {
                code: "approval_mismatch".into(),
                message: "The approval id does not name a pending approval.".into(),
                retryable: false,
            });
        };
        let Some(ActiveRequest::Agent {
            session_id: owner,
            decisions,
            ..
        }) = inner.active.get(request_id)
        else {
            let sid = session_id.clone();
            drop(inner);
            self.flush_session(&sid);
            return Err(ProtocolErrorDto {
                code: "approval_expired".into(),
                message: "The turn this approval belongs to has ended.".into(),
                retryable: false,
            });
        };
        if envelope_session.is_some_and(|session| session != *owner) {
            return Err(stale_session());
        }
        let result = decisions
            .send((loop_id, map_decision(decision, answer)))
            .map_err(|_| ProtocolErrorDto {
                code: "approval_expired".into(),
                message: "The turn this approval belongs to has ended.".into(),
                retryable: false,
            });
        drop(inner);
        self.flush_session(&session_id);
        result
    }

    // -----------------------------------------------------------------
    // Phase 9d — RetryMessage
    // -----------------------------------------------------------------

    /// Rewind the session to just before `message_id` and re-send that user
    /// message as a new turn. The rewritten transcript drops the abandoned
    /// branch; the full history of *other* sessions is untouched.
    fn retry_durable_message<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
        request_id: RequestId,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<(), ProtocolErrorDto> {
        let store = self.store().ok_or_else(no_workspace)?;
        let (prompt, model, mode, entries) = {
            let mut inner = self.inner.lock().expect("engine lock");
            if inner
                .sessions
                .get(&session_id)
                .is_some_and(|session| session.active_request.is_some())
            {
                return Err(ProtocolErrorDto {
                    code: "session_busy".into(),
                    message: "A request is already running for this session — stop it first."
                        .into(),
                    retryable: false,
                });
            }
            if !inner.sessions.contains_key(&session_id) {
                let restored = self.load_session_from_disk(&session_id)?;
                inner.sessions.insert(session_id.clone(), restored);
            }
            let session = inner.sessions.get_mut(&session_id).expect("inserted above");
            let index = session
                .messages
                .iter()
                .position(|message| message.message_id == message_id)
                .ok_or_else(|| ProtocolErrorDto {
                    code: "message_not_found".into(),
                    message: "That message is not in this session.".into(),
                    retryable: false,
                })?;
            if session.messages[index].role != MessageRole::User {
                return Err(ProtocolErrorDto {
                    code: "retry_requires_user_message".into(),
                    message: "Only a user message can be retried.".into(),
                    retryable: false,
                });
            }
            let prompt = session.messages[index].text.clone();
            let loaded = store
                .load(session_id.as_str())
                .map_err(|error| session_persistence_error(&error))?
                .ok_or_else(|| unknown_session(&session_id))?;
            let durable_index = loaded
                .transcript
                .entries
                .iter()
                .position(|entry| entry.id() == message_id.as_str())
                .ok_or_else(|| unknown_message(&message_id))?;
            if loaded.transcript.entries[durable_index..]
                .iter()
                .any(|entry| {
                    let calls = match entry {
                        TranscriptEntry::ToolCalls { calls, .. } => Some(calls),
                        TranscriptEntry::Assistant { tool_calls, .. } => tool_calls.as_ref(),
                        _ => None,
                    };
                    calls.is_some_and(|calls| {
                        calls
                            .iter()
                            .any(|call| selora_ai::plan::is_side_effecting_tool(&call.name))
                    })
                })
            {
                return Err(ProtocolErrorDto {
                    code: "retry_partial_effects".into(),
                    message: "This turn attempted side effects. Start a new turn to inspect the result; retry could duplicate writes or commands.".into(),
                    retryable: false,
                });
            }
            let entries = loaded.transcript.entries[..durable_index].to_vec();
            store
                .rewrite_transcript(session_id.as_str(), &entries)
                .map_err(|error| session_persistence_error(&error))?;
            session.messages = stored_transcript(&entries);
            session.durable_entries = session.messages.len();
            session.pending_approval = None;
            session.pending_card = None;
            session.pending_tool_name = None;
            session.restored_park = false;
            session.updated_at = selora_ai::now_millis();
            let model = session.model.clone();
            let mode = session.mode;
            inner.active_session = Some(session_id.clone());
            (prompt, model, mode, entries)
        };
        let _ = entries;
        self.flush_session(&session_id);
        self.send_agent(
            log,
            app,
            Some(session_id.clone()),
            request_id,
            selora_protocol::AgentRequestDto {
                session_id: session_id.clone(),
                prompt,
                model_id: model,
                mode,
                reasoning_effort: None,
                system_instructions: None,
                temperature: None,
                editor_context: None,
            },
        )
    }

    // -----------------------------------------------------------------
    // The boot connection probe (the model picker's data path)
    // -----------------------------------------------------------------

    /// Read the key, probe the gateway (`list_models` — connected means up
    /// AND the key is valid), cache the catalog, and announce it through
    /// `ModelsChanged` + `SeloraStatusChanged`. A missing key or a failed
    /// probe is a silent `Disconnected` (the egui boot's precedent).
    pub(crate) async fn connect_probe<R: tauri::Runtime>(
        self: &Arc<Self>,
        log: Arc<EventLog>,
        app: tauri::AppHandle<R>,
    ) {
        let key = match self.fetch_api_key() {
            Ok(key) => key,
            Err(_) => {
                self.set_connection(ConnectionStatus::Disconnected);
                let active = self.active_model_id();
                log.record(
                    &app,
                    status_changed(ConnectionStatus::Disconnected, &active),
                    None,
                );
                return;
            }
        };
        let base_url = self.base_url();
        let client = selora_ai::SeloraClient::new(&base_url, &key);
        let models = match client.list_models().await {
            Ok(models) if !models.is_empty() => models,
            _ => crate::commands::synthesize_provider_models(&base_url),
        };
        let active_model = self
            .active_model_id()
            .filter(|current| {
                models
                    .iter()
                    .any(|m| m.id == *current || m.id.eq_ignore_ascii_case(current))
            })
            .or_else(|| models.first().map(|m| m.id.clone()));
        {
            let mut inner = self.inner.lock().expect("engine lock");
            inner.models = models.clone();
            inner.connection = ConnectionStatus::Connected;
            inner.active_model = active_model.clone();
        }
        log.record(
            &app,
            FrontendEvent::ModelsChanged(ModelsDto {
                models: models.iter().map(model_dto).collect(),
                active_model_id: active_model.clone(),
            }),
            None,
        );
        log.record(
            &app,
            status_changed(ConnectionStatus::Connected, &active_model),
            None,
        );
    }

    fn set_connection(&self, status: ConnectionStatus) {
        self.inner.lock().expect("engine lock").connection = status;
    }

    // -----------------------------------------------------------------
    // Relay-side seams (called between awaits, never across one)
    // -----------------------------------------------------------------

    /// The relay's terminal for a chat stream: unregisters the request and
    /// lands the (possibly partial) assistant message in the transcript.
    fn finish_chat(
        &self,
        request_id: &RequestId,
        session_id: &SessionId,
        assistant: Option<(MessageId, String)>,
    ) {
        {
            let mut inner = self.inner.lock().expect("engine lock");
            inner.active.remove(request_id);
            if let Some(session) = inner.sessions.get_mut(session_id) {
                if session.active_request.as_ref() == Some(request_id) {
                    session.active_request = None;
                }
                if let Some((message_id, text)) = assistant {
                    push_message(session, message_id, MessageRole::Assistant, text, None);
                }
                session.updated_at = selora_ai::now_millis();
            }
        }
        self.flush_session(session_id);
    }

    /// The relay's terminal for an agent turn: same teardown, plus clearing
    /// any parked approval (a decision for a dead turn can never apply).
    fn finish_agent(
        &self,
        request_id: &RequestId,
        session_id: &SessionId,
        assistant: Option<(MessageId, String)>,
    ) {
        {
            let mut inner = self.inner.lock().expect("engine lock");
            inner.active.remove(request_id);
            if let Some(session) = inner.sessions.get_mut(session_id) {
                if session.active_request.as_ref() == Some(request_id) {
                    session.active_request = None;
                }
                session.pending_approval = None;
                session.pending_card = None;
                session.pending_tool_name = None;
                session.restored_park = false;
                if let Some((message_id, text)) = assistant {
                    push_message(session, message_id, MessageRole::Assistant, text, None);
                }
                session.updated_at = selora_ai::now_millis();
            }
        }
        self.flush_session(session_id);
    }

    /// Park the turn's approval card (the single-use slot). The relay calls
    /// this *before* emitting `ApprovalNeeded`, so a decision can never race
    /// the announcement.
    fn set_pending_approval(
        &self,
        session_id: &SessionId,
        approval: ApprovalDto,
        card: ApprovalCard,
        tool_name: String,
    ) {
        {
            let mut inner = self.inner.lock().expect("engine lock");
            if let Some(session) = inner.sessions.get_mut(session_id) {
                session.pending_approval = Some(approval);
                session.pending_card = Some(card);
                session.pending_tool_name = Some(tool_name);
                session.restored_park = false;
                session.updated_at = selora_ai::now_millis();
            }
        }
        self.flush_session(session_id);
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    pub(crate) fn fetch_api_key(&self) -> Result<String, ProtocolErrorDto> {
        self.config.key_source.get().and_then(|key| {
            key.ok_or_else(|| ProtocolErrorDto {
                code: "not_connected".into(),
                message: "No Selora API key is stored — connect first.".into(),
                retryable: true,
            })
        })
    }

    pub(crate) fn resolve_model(&self, preferred: Option<&str>) -> Result<Model, ProtocolErrorDto> {
        let id = preferred
            .map(str::to_owned)
            .or_else(|| self.active_model_id())
            .ok_or_else(no_model_selected)?;
        let inner = self.inner.lock().expect("engine lock");
        if let Some(found) = inner.models.iter().find(|model| {
            model.id == id
                || model.id.eq_ignore_ascii_case(&id)
                || model.display_name.eq_ignore_ascii_case(&id)
        }) {
            return Ok(found.clone());
        }
        // An unknown id (no catalog yet, or a model the gateway no longer
        // lists): a bare model — no context cap (never a false abort), no
        // price fallback (cost stays receipt-verbatim).
        drop(inner);
        Ok(Model {
            display_name: id.clone(),
            id,
            context_window: 0,
            price_per_1k_input: 0.0,
            price_per_1k_output: 0.0,
            supports_streaming: true,
            supports_tools: true,
            family: String::new(),
        })
    }

    /// Workflow asks inherit the same per-turn limits, but not session approval grants.
    pub(crate) fn workflow_turn_caps(&self, context_window: u32) -> TurnCaps {
        TurnCaps {
            max_tool_iterations: self.config.agent.max_iterations,
            max_turn_secs: self.config.agent.max_turn_secs,
            max_cost_usd: self.config.agent.max_cost_usd,
            ..TurnCaps::spec_defaults(context_window)
        }
    }

    /// The test seam over [`SessionEngine::resolve_model`] (the boot probe
    /// test proves the cached catalog feeds model resolution).
    #[cfg(test)]
    pub(crate) fn resolve_model_for_test(
        &self,
        preferred: Option<&str>,
    ) -> Result<Model, ProtocolErrorDto> {
        self.resolve_model(preferred)
    }

    /// The Brave key (§17.1 `web_search`): the settings field first, the
    /// keyring entry second (R5's seam — the key never lives in a file once
    /// migrated). Never logged, never in an event.
    fn brave_key(&self) -> Option<String> {
        let from_settings = self
            .config
            .agent
            .brave_search_api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        from_settings.or_else(|| match self.config.key_source {
            ApiKeySource::Keyring => selora_settings::keyring::brave_key_get().ok().flatten(),
            #[cfg(test)]
            ApiKeySource::Fixed(_) => None,
        })
    }
}

// ---------------------------------------------------------------------------
// The chat relay
// ---------------------------------------------------------------------------

struct ChatRelayCtx<R: tauri::Runtime> {
    log: Arc<EventLog>,
    engine: Arc<SessionEngine>,
    app: tauri::AppHandle<R>,
    request_id: RequestId,
    session_id: SessionId,
    turn_id: TurnId,
    message_id: MessageId,
    base_url: String,
    api_key: String,
    request: ChatRequest,
    cancel_rx: watch::Receiver<bool>,
}

/// Relays `SeloraClient::stream_chat`'s events onto the protocol channel.
/// Mirrors the egui shell's `spawn_chat` (wiring.rs): the stream runs as
/// its own subtask gated on the cancel watch; the relay stops at the first
/// terminal event and always joins the subtask — an error that never
/// reached the channel is still surfaced (`ChatFailed`), a cancel is
/// silent, and the partial bubble is preserved either way.
async fn chat_relay<R: tauri::Runtime>(ctx: ChatRelayCtx<R>) {
    let ChatRelayCtx {
        log,
        engine,
        app,
        request_id,
        session_id,
        turn_id,
        message_id,
        base_url,
        api_key,
        request,
        mut cancel_rx,
    } = ctx;
    let client = selora_ai::SeloraClient::new(&base_url, &api_key);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<ChatEvent>();

    // The cancel watch: a user Stop flips it, the request future is dropped
    // (the SSE connection closes), and the subtask resolves `None` so the
    // relay stays silent about the cancel.
    let stream = tauri::async_runtime::spawn(async move {
        tokio::select! {
            _ = cancel_rx.changed() => None,
            result = client.stream_chat(request, events_tx) => Some(result),
        }
    });

    let mut delta_seq: u64 = 0;
    let mut text = String::new();
    let mut terminal: Option<Result<ChatReceipt, SeloraError>> = None;
    while let Some(event) = events_rx.recv().await {
        match event {
            ChatEvent::Delta(delta) => {
                text.push_str(&delta);
                delta_seq += 1;
                log.record_session(
                    &app,
                    FrontendEvent::ChatDelta(ChatDeltaDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        message_id: message_id.clone(),
                        sequence: delta_seq,
                        text: delta,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            // Plain chat never arms tools (`ChatRequest::tools` is `None`),
            // so a tool-call fragment cannot arrive here — defensive skip.
            ChatEvent::ToolCallDelta { .. } => continue,
            ChatEvent::Done(receipt) => {
                terminal = Some(Ok(receipt));
                break;
            }
            ChatEvent::Error(error) => {
                terminal = Some(Err(error));
                break;
            }
        }
    }

    // Always join the stream task: when it resolved normally its terminal
    // event was already relayed; a cancel resolved `None` (silent by
    // design); a panicking subtask is surfaced as a `ChatFailed` (never a
    // silent cancel, PLAN.md §8).
    let outcome = match stream.await {
        Ok(joined) => terminal.or(joined),
        Err(join_error) => Some(Err(SeloraError::Other(format!(
            "chat worker died: {join_error}"
        )))),
    };
    match outcome {
        Some(Ok(receipt)) => {
            // Finalize the bubble first, then announce: a bootstrap taken
            // after the event is consistent with the transcript.
            engine.finish_chat(
                &request_id,
                &session_id,
                Some((message_id.clone(), text.clone())),
            );
            log.record_session(
                &app,
                FrontendEvent::ChatCompleted(ChatCompletedDto {
                    request_id: request_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    message_id: message_id.clone(),
                    text,
                    total_tokens: u64::from(receipt.prompt_tokens)
                        + u64::from(receipt.completion_tokens),
                    cost_usd: receipt.cost_usd,
                }),
                Some(request_id.clone()),
                session_id.clone(),
            );
        }
        Some(Err(error)) => {
            // Keep the partial bubble (the frontend's Retry drops it; the
            // transcript keeps the user turn either way).
            let assistant = if text.is_empty() {
                None
            } else {
                Some((message_id, text.clone()))
            };
            engine.finish_chat(&request_id, &session_id, assistant);
            log.record_session(
                &app,
                FrontendEvent::ChatFailed(ChatFailedDto {
                    request_id: request_id.clone(),
                    session_id: session_id.clone(),
                    turn_id,
                    error: to_protocol_error(&error, &api_key),
                }),
                Some(request_id),
                session_id,
            );
        }
        // `None`: cancelled — the partial bubble is preserved (an empty
        // partial adds no transcript entry) and the stop is silent (Phase
        // R1: a user stop is never a ChatError).
        None => {
            let assistant = if text.is_empty() {
                None
            } else {
                Some((message_id.clone(), text.clone()))
            };
            engine.finish_chat(&request_id, &session_id, assistant);
            log.record_session(
                &app,
                FrontendEvent::ChatCompleted(ChatCompletedDto {
                    request_id: request_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    message_id: message_id.clone(),
                    text,
                    total_tokens: 0,
                    cost_usd: 0.0,
                }),
                Some(request_id.clone()),
                session_id.clone(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The agent relay
// ---------------------------------------------------------------------------

struct AgentRelayCtx<R: tauri::Runtime> {
    log: Arc<EventLog>,
    engine: Arc<SessionEngine>,
    app: tauri::AppHandle<R>,
    request_id: RequestId,
    session_id: SessionId,
    turn_id: TurnId,
    api_key: String,
    events_rx: mpsc::UnboundedReceiver<selora_ai::AgentEvent>,
    join: tokio::task::JoinHandle<selora_ai::AgentTurnReport>,
}

/// Relays the agent loop's `AgentEvent` stream onto the protocol channel —
/// the web-shell twin of the egui `spawn_agent` relay (wiring.rs). Every
/// round's `State(Thinking)` rotates the streaming `message_id` (one
/// message id per round, the same way the conversation grows a message per
/// round); the last `ToolCallStarted` names the card in
/// `ApprovalNeeded`; `TurnDone` finalizes the transcript, clears the
/// parked approval, and emits the summary footer.
async fn agent_relay<R: tauri::Runtime>(ctx: AgentRelayCtx<R>) {
    let AgentRelayCtx {
        log,
        engine,
        app,
        request_id,
        session_id,
        turn_id,
        api_key,
        mut events_rx,
        join,
    } = ctx;

    let mut current_message: Option<MessageId> = None;
    let mut delta_seq: u64 = 0;
    let mut last_call: Option<selora_ai::ToolCall> = None;
    let mut done_seen = false;
    let mut final_text = String::new();

    while let Some(event) = events_rx.recv().await {
        match event {
            selora_ai::AgentEvent::State(state) => {
                // A new round is a new streaming message.
                if state == AgentTurnState::Thinking {
                    current_message = Some(fresh_message_id());
                    delta_seq = 0;
                }
                let (phase, tool_name) = map_state(&state);
                log.record_session(
                    &app,
                    FrontendEvent::AgentStateChanged(AgentStateDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        state: phase,
                        tool_name,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            selora_ai::AgentEvent::Delta(delta) => {
                final_text.push_str(&delta);
                delta_seq += 1;
                let message_id = current_message.get_or_insert_with(fresh_message_id).clone();
                log.record_session(
                    &app,
                    FrontendEvent::ChatDelta(ChatDeltaDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        message_id,
                        sequence: delta_seq,
                        text: delta,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            selora_ai::AgentEvent::ToolCallStarted(call) => {
                engine.record_tool_calls(&session_id, vec![call.clone()]);
                log.record_session(
                    &app,
                    FrontendEvent::ToolCallStarted(ToolCallStartedDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        args_json: call.arguments.clone(),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
                last_call = Some(call);
            }
            selora_ai::AgentEvent::ToolResult(result) => {
                engine.record_tool_result(&session_id, &result);
                log.record_session(
                    &app,
                    FrontendEvent::ToolResult(ToolResultDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        call_id: result.id.clone(),
                        result_summary: result.content.clone(),
                        is_err: !result.ok,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            selora_ai::AgentEvent::ApprovalNeeded { approval_id, card } => {
                let card = redact_card(card, &api_key);
                let tool_name = last_call
                    .as_ref()
                    .map(|call| call.name.clone())
                    .unwrap_or_else(|| fallback_tool_name(&card));
                let (path_or_command, diff_or_input, workflow) = approval_body(&card);
                let plan = approval_plan(&card);
                let prompt = approval_prompt(&card);
                // Park before announcing: the single-use slot exists before
                // the frontend can possibly answer.
                engine.set_pending_approval(
                    &session_id,
                    ApprovalDto {
                        approval_id: ApprovalId::new(approval_id.to_string())
                            .expect("uuid string is a valid id"),
                        request_id: request_id.clone(),
                        turn_id: turn_id.clone(),
                        session_id: session_id.clone(),
                        summary: format!("{tool_name} {path_or_command}"),
                        can_approve_all_this_turn: !matches!(
                            &card,
                            ApprovalCard::Plan { .. }
                                | ApprovalCard::Workflow { .. }
                                | ApprovalCard::Question(_)
                        ),
                        plan,
                        decision_prompt: prompt.clone(),
                    },
                    card.clone(),
                    tool_name.clone(),
                );
                log.record_session(
                    &app,
                    FrontendEvent::ApprovalNeeded(ApprovalNeededDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        approval_id: ApprovalId::new(approval_id.to_string())
                            .expect("uuid string is a valid id"),
                        tool_name,
                        path_or_command,
                        diff_or_input,
                        workflow,
                        plan: approval_plan(&card),
                        prompt,
                        subagent_id: None,
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            selora_ai::AgentEvent::PlanChanged { phase, plan } => {
                engine.sync_plan_store(&session_id);
                log.record_session(
                    &app,
                    FrontendEvent::PlanStateChanged(PlanStateDto {
                        session_id: session_id.clone(),
                        phase: plan_phase_dto(phase),
                        plan: plan
                            .as_ref()
                            .map(|draft| plan_approval_dto(draft, &api_key)),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            selora_ai::AgentEvent::TodosChanged(todos) => {
                engine.sync_plan_store(&session_id);
                log.record_session(
                    &app,
                    FrontendEvent::TodosChanged(TodosDto {
                        session_id: session_id.clone(),
                        todos: todos.iter().map(todo_dto).collect(),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            selora_ai::AgentEvent::Compacted(cursor) => {
                engine.record_compaction(&session_id, cursor);
            }
            selora_ai::AgentEvent::CompactionFailed(error) => {
                let (message, partial_summary) = match &error {
                    CompactionError::SummaryFailed {
                        message,
                        partial_summary,
                    } => (message.clone(), partial_summary.clone()),
                    other => (other.to_string(), None),
                };
                log.record_session(
                    &app,
                    FrontendEvent::AgentCompactionFailed(AgentCompactionFailedDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        message: redact(&message, &api_key),
                        partial_summary: partial_summary.map(|summary| redact(&summary, &api_key)),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
            }
            selora_ai::AgentEvent::TurnDone(report) => {
                done_seen = true;
                // The final answer is the last round's message; an empty
                // final (a tools-only abort) adds nothing.
                let assistant = if report.final_message.trim().is_empty() {
                    None
                } else {
                    let message_id = current_message.get_or_insert_with(fresh_message_id).clone();
                    Some((message_id, report.final_message.clone()))
                };
                engine.finish_agent(&request_id, &session_id, assistant);
                if final_text.is_empty() && !report.final_message.trim().is_empty() {
                    let message_id = current_message.get_or_insert_with(fresh_message_id).clone();
                    log.record_session(
                        &app,
                        FrontendEvent::ChatDelta(ChatDeltaDto {
                            request_id: request_id.clone(),
                            session_id: session_id.clone(),
                            turn_id: turn_id.clone(),
                            message_id,
                            sequence: 1,
                            text: report.final_message.clone(),
                        }),
                        Some(request_id.clone()),
                        session_id.clone(),
                    );
                }
                log.record_session(
                    &app,
                    FrontendEvent::AgentTurnDone(AgentTurnDoneDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        total_tokens: report.summary.tokens,
                        cost_usd: report.summary.cost_usd,
                        num_tools: report.summary.tools_used,
                        approved: report.summary.approved,
                        denied: report.summary.denied,
                        completed: report.summary.completed,
                        end_message: redact(&report.end.toast(), &api_key),
                    }),
                    Some(request_id.clone()),
                    session_id.clone(),
                );
                break;
            }
        }
    }

    // Always join the loop task (the worker-died pattern, PLAN.md §8): a
    // turn whose event channel closed before `TurnDone` still gets one
    // terminal event — from the report if it completed, else a synthetic
    // `Failed`.
    if !done_seen {
        let (summary, end_message) = match join.await {
            Ok(report) => (report.summary, report.end.toast()),
            Err(join_error) => (
                AgentSummary {
                    iterations: 0,
                    tools_used: 0,
                    approved: 0,
                    denied: 0,
                    tokens: 0,
                    cost_usd: 0.0,
                    completed: false,
                },
                format!("agent worker died: {join_error}"),
            ),
        };
        let assistant = if final_text.is_empty() {
            None
        } else {
            Some((fresh_message_id(), final_text.clone()))
        };
        engine.finish_agent(&request_id, &session_id, assistant);
        log.record_session(
            &app,
            FrontendEvent::AgentTurnDone(AgentTurnDoneDto {
                request_id: request_id.clone(),
                session_id: session_id.clone(),
                turn_id,
                total_tokens: summary.tokens,
                cost_usd: summary.cost_usd,
                num_tools: summary.tools_used,
                approved: summary.approved,
                denied: summary.denied,
                completed: summary.completed,
                end_message: redact(&end_message, &api_key),
            }),
            Some(request_id),
            session_id,
        );
    }
}

// ---------------------------------------------------------------------------
// The subagent relay (completions + child cards)
// ---------------------------------------------------------------------------

struct SubagentRelayCtx<R: tauri::Runtime> {
    log: Arc<EventLog>,
    engine: Arc<SessionEngine>,
    app: tauri::AppHandle<R>,
    events_rx: mpsc::UnboundedReceiver<selora_ai::SubagentEvent>,
}

/// Map the shared runtime's events onto the normal session stream. A
/// completion lands in the roster once (the frontend dedupes by child id) and
/// a child card parks on the same approval rail as the parent's own cards.
async fn subagent_relay<R: tauri::Runtime>(ctx: SubagentRelayCtx<R>) {
    let SubagentRelayCtx {
        log,
        engine,
        app,
        mut events_rx,
    } = ctx;
    while let Some(event) = events_rx.recv().await {
        match event {
            selora_ai::SubagentEvent::Completed(completion) => {
                let Ok(parent) = SessionId::new(completion.parent_session.clone()) else {
                    continue;
                };
                log.record_session(
                    &app,
                    FrontendEvent::SubagentCompleted(subagent_completed_dto(completion)),
                    None,
                    parent,
                );
            }
            selora_ai::SubagentEvent::ApprovalNeeded(child_card) => {
                let Ok(session_id) = SessionId::new(child_card.parent_session.clone()) else {
                    continue;
                };
                let approval_id = ApprovalId::new(child_card.approval_id.to_string())
                    .expect("uuid string is a valid id");
                let child_id = child_card.child_id.clone();
                let card = child_card.card;
                let tool_name = fallback_tool_name(&card);
                let (path_or_command, diff_or_input, workflow) = approval_body(&card);
                let plan = approval_plan(&card);
                let prompt = approval_prompt(&card);
                let is_question = matches!(&card, ApprovalCard::Question(_));
                let (request_id, turn_id) = engine
                    .active_request_for_session(&session_id)
                    .unwrap_or_else(|| {
                        (
                            RequestId::new(format!("subagent-{child_id}"))
                                .expect("minted request id"),
                            TurnId::new(format!("turn-{child_id}")).expect("minted turn id"),
                        )
                    });
                engine
                    .subagent_cards
                    .lock()
                    .expect("subagent card lock")
                    .insert(
                        approval_id.clone(),
                        SubagentCardRoute {
                            request_id: request_id.clone(),
                            session_id: session_id.clone(),
                            is_question,
                        },
                    );
                log.record_session(
                    &app,
                    FrontendEvent::ApprovalNeeded(ApprovalNeededDto {
                        request_id: request_id.clone(),
                        session_id: session_id.clone(),
                        turn_id,
                        approval_id,
                        tool_name,
                        path_or_command,
                        diff_or_input,
                        workflow,
                        plan,
                        prompt,
                        subagent_id: Some(child_id),
                    }),
                    Some(request_id),
                    session_id,
                );
            }
        }
    }
}

/// The desktop subagent backend: one fresh chat stream and one policy-scoped
/// router per child turn. It deliberately carries no session approval grant.
struct DesktopSubagentBackend {
    root: PathBuf,
    settings: selora_ai::AgentToolSettings,
    base_url: String,
    api_key: String,
}

impl selora_ai::SubagentBackend for DesktopSubagentBackend {
    fn chat(&self) -> Result<Box<dyn selora_ai::ChatStream>, String> {
        Ok(Box::new(selora_ai::ClientChatStream::new(
            selora_ai::SeloraClient::new(&self.base_url, &self.api_key),
        )))
    }

    fn tools(&self, policy: selora_ai::SubagentPolicy) -> Result<Box<dyn ToolRunner>, String> {
        let context =
            ToolContext::new(self.root.clone(), self.settings.clone()).with_policy(policy);
        Ok(Box::new(RouterToolRunner::new(context)))
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The agent turn's system prompt: the mode-correct approval note, the
/// reasoning-depth line, the user's extra instructions, and the editor
/// reference (path/language/selection/dirty — never the file text; the
/// tools read the bytes).
fn system_prompt(payload: &selora_protocol::AgentRequestDto, root: &Path) -> String {
    let depth_instructions = match payload.reasoning_effort.as_deref() {
        Some("max") => "\nReasoning depth: Maximum. Provide deep, rigorous chain of thought. Analyze all edge cases, potential failure modes, and architectural implications before executing actions.",
        Some("quick") => "\nReasoning depth: Quick. Be concise and direct. Focus on immediate solution without excessive explanation.",
        _ => "\nReasoning depth: Standard. Balanced reasoning with clear explanations and thorough verification.",
    };

    let custom_sys = match payload.system_instructions.as_deref() {
        Some(custom) if !custom.trim().is_empty() => {
            format!("\nAdditional User Instructions:\n{}", custom.trim())
        }
        _ => String::new(),
    };

    // The prompt must match the mode: auto-applied writes never show the
    // approval card the default prompt describes.
    let approval_note = if matches!(payload.mode, AgentModeDto::Auto) {
        "File writes, edits, and deletes apply immediately in this session; bash commands still pause for approval."
    } else if matches!(payload.mode, AgentModeDto::Explore) {
        "This session is read-only: inspect and report, but do not attempt a write."
    } else {
        "Writes, deletes, and non-allowlisted commands pause for the user's approval card; explain what you are doing while the card is pending."
    };

    // Empty without editor context, so the prompt stays byte-identical to
    // the pre-editor-context one.
    let editor_note = match payload.editor_context.as_ref() {
        Some(ctx) => {
            let mut note = format!(
                "\n\nEditor context (as of when the user pressed send; it may have moved since):\n\
                 Active file (workspace-relative): {}\n\
                 Language: {}",
                ctx.path, ctx.language
            );
            if let Some(selection) = ctx.selection.as_ref() {
                let anchor = selection.anchor;
                let active = selection.active;
                // An empty range is no selection; a backwards one is
                // normalized so start is never past end.
                if anchor != active {
                    let (start, end) = if (anchor.line, anchor.utf16_column)
                        <= (active.line, active.utf16_column)
                    {
                        (anchor, active)
                    } else {
                        (active, anchor)
                    };
                    note.push_str(&format!(
                        "\nSelection: {}:{}-{}:{}",
                        start.line + 1,
                        start.utf16_column + 1,
                        end.line + 1,
                        end.utf16_column + 1
                    ));
                }
            }
            if ctx.dirty {
                note.push_str("\nStatus: has unsaved edits");
            }
            note.push_str("\nUse `read_file` with this workspace-relative path before concluding anything about this file. Do not assume the selection is still accurate, and do not claim to have read a file you have not read in this turn.");
            note
        }
        None => String::new(),
    };

    format!(
        "You are Selora, an autonomous coding agent in Selora IDE.\n\
         Operating System: Windows. Shell: PowerShell.\n\
         Workspace Root: {}\n\
         Available tools: read_file, list_dir, fuzzy_find, search_files, write_file, edit_file, delete_file, bash, web_search, web_fetch, run_workflow, enter_plan_mode, exit_plan_mode, todo_read, todo_write.\n\
         Instructions:\n\
         1. You can read and change the project. Use `list_dir`, `fuzzy_find`, `search_files`, and `read_file` to inspect it, then `write_file`, `edit_file`, or `delete_file` to make the change.\n\
         2. Do not stop at describing a change: when the user asks for a code change, perform it with the tools. {}\n\
         3. Always inspect relevant files before modifying them, and re-read after a failed edit rather than guessing.\n\
         4. Break complex work into clear steps and keep the live todo list current with `todo_write` (read it with `todo_read`).\n\
         5. For a non-trivial change, call `enter_plan_mode`, explore read-only, then propose the plan with `exit_plan_mode`; while planning, every write, delete, bash command, and workflow is blocked until the user approves the plan. Approving the plan still leaves each write behind its own diff card.\n\
         6. Make clean, accurate edits and verify your work (build, test, or read the result back).{}{}{}",
        root.display(),
        approval_note,
        depth_instructions,
        custom_sys,
        editor_note
    )
}

/// The `AgentToolSettings` mapping (R5's seam): the shell's settings into
/// the router's slice, the Brave key through the `skip_serializing` field
/// (never serialized back out, never logged, §17.2).
fn agent_tool_settings(
    settings: &selora_settings::AgentSettings,
    brave_api_key: Option<String>,
) -> selora_ai::AgentToolSettings {
    selora_ai::AgentToolSettings {
        shell: settings.shell.clone(),
        auto_approve_commands: settings.auto_approve_commands.clone(),
        brave_api_key,
    }
}

/// Look up (or create) the session, guard the busy rules, land the user's
/// message, and mark the request active — a refused send never mutates
/// the transcript, an accepted one is registered before the relay spawns.
fn begin_turn<'a>(
    inner: &'a mut EngineInner,
    request_id: &RequestId,
    session_id: &SessionId,
    user_text: &str,
    retry: Option<RetrySpec>,
) -> Result<&'a mut ChatSession, ProtocolErrorDto> {
    if inner.active.contains_key(request_id) {
        return Err(ProtocolErrorDto {
            code: "request_in_flight".into(),
            message: "This request id is already running.".into(),
            retryable: false,
        });
    }
    if let Some(session) = inner.sessions.get(session_id) {
        if session.active_request.is_some() {
            return Err(session_busy());
        }
    }
    inner.activity_counter += 1;
    let activity = inner.activity_counter;
    let now = selora_ai::now_millis();
    let session = inner
        .sessions
        .entry(session_id.clone())
        .or_insert_with(|| ChatSession::new(title_from(user_text), now));
    session.last_activity = activity;
    session.active_request = Some(request_id.clone());
    session.updated_at = now;
    inner.active_session = Some(session_id.clone());
    let user_message_id = fresh_message_id();
    push_message(
        session,
        user_message_id,
        MessageRole::User,
        user_text.to_owned(),
        retry,
    );
    Ok(session)
}

/// Validate and apply a `RetryMessage` truncation under the engine lock:
/// remove the named user message and everything after it. Called by the send
/// paths after their key/model/workspace preflight, so a refused preflight
/// never mutates the transcript.
fn truncate_for_retry(
    inner: &mut EngineInner,
    target: &RetryTarget,
) -> Result<(), ProtocolErrorDto> {
    let session = inner
        .sessions
        .get_mut(&target.session_id)
        .ok_or_else(|| unknown_session(&target.session_id))?;
    if session.active_request.is_some() {
        return Err(session_busy());
    }
    let index = session
        .messages
        .iter()
        .position(|message| message.message_id == target.message_id)
        .ok_or_else(|| unknown_message(&target.message_id))?;
    if session.messages[index].role != MessageRole::User {
        return Err(not_a_user_message(&target.message_id));
    }
    session.messages.truncate(index);
    Ok(())
}

fn push_message(
    session: &mut ChatSession,
    message_id: MessageId,
    role: MessageRole,
    text: String,
    retry: Option<RetrySpec>,
) {
    session.messages.push(StoredMessage {
        message_id,
        role,
        text,
        retry,
        tool_calls: None,
        tool_call_id: None,
        ok: true,
        timestamp: selora_ai::now_millis(),
    });
    if session.messages.len() > MAX_SESSION_MESSAGES {
        let overflow = session.messages.len() - MAX_SESSION_MESSAGES;
        session.messages.drain(..overflow);
        // The transcript keeps the full history; the in-memory tail is what
        // the request needs. Never re-append a message that was already
        // written.
        session.durable_entries = session.durable_entries.saturating_sub(overflow);
    }
}

/// The trailing transcript as request messages. Tool-call/tool-result
/// pairing is preserved for restored sessions (Z3's live-only behaviour
/// would lose the model's own tool context on resume).
fn conversation_tail(session: &ChatSession) -> Vec<ChatMessage> {
    session
        .messages
        .iter()
        .rev()
        .take(REQUEST_HISTORY_MESSAGES)
        .rev()
        .map(|message| {
            let mut chat = match message.role {
                MessageRole::User => ChatMessage::user(&message.text),
                MessageRole::Assistant => ChatMessage::assistant(&message.text),
                MessageRole::Tool => ChatMessage::tool_result(
                    message.tool_call_id.clone().unwrap_or_default(),
                    &message.text,
                ),
                MessageRole::System => ChatMessage::system(&message.text),
            };
            if message.role == MessageRole::Assistant {
                chat.tool_calls = message.tool_calls.clone();
            }
            chat
        })
        .collect()
}

fn title_from(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(32)
        .collect()
}

/// Persisted mode: `Auto` is session trust, not a durable mode.
fn persist_mode(mode: AgentModeDto) -> AgentModeDto {
    match mode {
        AgentModeDto::Auto => AgentModeDto::Execute,
        other => other,
    }
}

fn durable_mode(mode: AgentModeDto) -> AgentSessionMode {
    match persist_mode(mode) {
        AgentModeDto::Explore => AgentSessionMode::Explore,
        _ => AgentSessionMode::Execute,
    }
}

fn mode_from_disk(mode: AgentSessionMode) -> AgentModeDto {
    match mode {
        AgentSessionMode::Explore => AgentModeDto::Explore,
        AgentSessionMode::Execute | AgentSessionMode::Plan => AgentModeDto::Execute,
    }
}

/// One in-memory message as a durable transcript record.
fn message_entry(message: &StoredMessage) -> TranscriptEntry {
    let id = message.message_id.as_str().to_owned();
    match message.role {
        MessageRole::User => TranscriptEntry::User {
            id,
            timestamp: message.timestamp,
            content: message.text.clone(),
        },
        MessageRole::Assistant | MessageRole::System => TranscriptEntry::Assistant {
            id,
            timestamp: message.timestamp,
            content: message.text.clone(),
            tool_calls: message.tool_calls.clone(),
        },
        MessageRole::Tool => TranscriptEntry::Tool {
            id,
            timestamp: message.timestamp,
            call_id: message.tool_call_id.clone().unwrap_or_default(),
            ok: message.ok,
            content: message.text.clone(),
        },
    }
}

/// A restored transcript message in the in-memory shape.
fn stored_message(message: &ChatMessage, timestamp: u64) -> StoredMessage {
    StoredMessage {
        retry: None,
        message_id: fresh_message_id(),
        role: match message.role {
            selora_ai::Role::User => MessageRole::User,
            selora_ai::Role::Assistant => MessageRole::Assistant,
            selora_ai::Role::Tool => MessageRole::Tool,
            selora_ai::Role::System => MessageRole::System,
        },
        text: message.content.clone(),
        tool_calls: message.tool_calls.clone(),
        tool_call_id: message.tool_call_id.clone(),
        ok: true,
        timestamp,
    }
}

/// Restore every durable record, including call batches and result pairing.
fn stored_transcript(entries: &[TranscriptEntry]) -> Vec<StoredMessage> {
    entries
        .iter()
        .map(|entry| {
            let (message, ok) = match entry {
                TranscriptEntry::User { content, .. } => (ChatMessage::user(content), true),
                TranscriptEntry::Assistant {
                    content,
                    tool_calls,
                    ..
                } => {
                    let mut message = ChatMessage::assistant(content);
                    message.tool_calls = tool_calls.clone();
                    (message, true)
                }
                TranscriptEntry::ToolCalls { calls, .. } => {
                    let mut message = ChatMessage::assistant("");
                    message.tool_calls = Some(calls.clone());
                    (message, true)
                }
                TranscriptEntry::Tool {
                    call_id,
                    content,
                    ok,
                    ..
                } => (ChatMessage::tool_result(call_id, content), *ok),
            };
            let mut stored = stored_message(&message, entry.timestamp());
            stored.message_id = MessageId::new(entry.id()).unwrap_or_else(|_| fresh_message_id());
            stored.ok = ok;
            stored
        })
        .collect()
}

/// The metadata half of a session write. The auto-approve flag is absent by
/// construction.
fn session_state(session_id: &SessionId, session: &ChatSession) -> AgentSessionState {
    let parks = match (&session.pending_approval, &session.pending_card) {
        (Some(approval), Some(card)) => vec![ParkedCard::Approval {
            approval_id: approval.approval_id.as_str().to_owned(),
            tool_name: session
                .pending_tool_name
                .clone()
                .unwrap_or_else(|| fallback_tool_name(card)),
            card: card.clone(),
            request_id: Some(approval.request_id.as_str().to_owned()),
            turn_id: Some(approval.turn_id.as_str().to_owned()),
            created_at: selora_ai::now_millis(),
        }],
        _ => Vec::new(),
    };
    AgentSessionState {
        schema: selora_ai::SESSION_SCHEMA,
        session_id: session_id.as_str().to_owned(),
        title: session.title.clone(),
        created_at: session.created_at,
        updated_at: session.updated_at,
        model: session.model.clone(),
        mode: durable_mode(session.mode),
        plan: plan_state_from_live(&selora_ai::plan::snapshot(&session.plan_store)),
        parks,
        compaction: session.compaction,
    }
}

/// Parse a durable plan body back into a draft. Phase 9b writes the draft
/// as JSON; a record written by an earlier build holds free-form text and
/// loads as a single step rather than being dropped.
fn plan_draft_from_disk(raw: &str) -> Option<selora_ai::PlanDraft> {
    if raw.trim().is_empty() {
        return None;
    }
    match serde_json::from_str::<selora_ai::PlanDraft>(raw) {
        Ok(draft) if !draft.steps.is_empty() => Some(draft),
        _ => Some(selora_ai::PlanDraft {
            summary: None,
            steps: vec![raw.to_owned()],
        }),
    }
}

/// The durable half of the live plan store: status, draft (as JSON, so the
/// steps survive a round trip) and timestamp.
fn plan_state_from_live(snapshot: &selora_ai::PlanSnapshot) -> selora_ai::PlanState {
    let plan = snapshot
        .plan
        .as_ref()
        .map(|draft| serde_json::to_string(draft).unwrap_or_else(|_| draft.steps.join("\n")));
    let status = match (&snapshot.plan, snapshot.phase) {
        (_, selora_ai::PlanPhase::Planning) => selora_ai::PlanStatus::Proposed,
        (None, selora_ai::PlanPhase::Executing) => selora_ai::PlanStatus::None,
        (Some(_), selora_ai::PlanPhase::Executing) => selora_ai::PlanStatus::Approved,
    };
    selora_ai::PlanState {
        status,
        plan,
        updated_at: selora_ai::now_millis(),
    }
}

fn session_persistence_error(error: &SeloraError) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "session_persistence_failed".into(),
        message: format!("Could not read or write the agent session: {error}"),
        retryable: true,
    }
}

fn fresh_turn_id() -> TurnId {
    TurnId::new(Uuid::new_v4().to_string()).expect("uuid is valid")
}

fn fresh_message_id() -> MessageId {
    MessageId::new(Uuid::new_v4().to_string()).expect("uuid is valid")
}

pub(crate) fn model_dto(model: &Model) -> ModelDto {
    ModelDto {
        id: model.id.clone(),
        display_name: if model.display_name.is_empty() {
            model.id.clone()
        } else {
            model.display_name.clone()
        },
        available: model.supports_streaming,
    }
}

fn manifest_for(mode: AgentModeDto) -> Vec<ToolSpec> {
    match mode {
        AgentModeDto::Execute | AgentModeDto::Auto => selora_ai::tool_manifest(),
        AgentModeDto::Explore => selora_ai::tool_manifest()
            .into_iter()
            .filter(|spec| EXPLORE_TOOLS.contains(&spec.name.as_str()))
            .collect(),
    }
}

fn map_state(state: &AgentTurnState) -> (AgentPhase, Option<String>) {
    match state {
        AgentTurnState::Thinking => (AgentPhase::Thinking, None),
        AgentTurnState::RunningTool(name) => (AgentPhase::RunningTool, Some(name.clone())),
        AgentTurnState::AwaitingApproval(_) => (AgentPhase::AwaitingApproval, None),
        AgentTurnState::Done => (AgentPhase::Done, None),
    }
}

/// The `ApprovalNeeded` card body for one router card: the target line the
/// generic card's header shows, an optional diff, and — for `run_workflow` —
/// the §18.1 confirmation payload the WorkflowCard renders (script, confirmed
/// command set, cost cap). A workflow card's `path_or_command` is the workflow
/// *name*; the script rides `workflow.script`, never `diff_or_input` (it is not
/// a diff and must not be counted as one).
fn approval_body(card: &ApprovalCard) -> (String, Option<String>, Option<WorkflowApprovalDto>) {
    match card {
        ApprovalCard::Diff { path, diff } => (path.clone(), Some(diff.clone()), None),
        ApprovalCard::Delete { path } => (path.clone(), None, None),
        ApprovalCard::Command { command } => (command.clone(), None, None),
        ApprovalCard::Workflow {
            name,
            script,
            commands,
            max_cost_usd,
        } => (
            name.clone(),
            None,
            Some(WorkflowApprovalDto {
                name: name.clone(),
                script: script.clone(),
                commands: commands.clone(),
                max_cost_usd: *max_cost_usd,
            }),
        ),
        ApprovalCard::Plan { summary, steps } => (
            summary
                .clone()
                .unwrap_or_else(|| format!("plan with {} step(s)", steps.len())),
            None,
            None,
        ),
        ApprovalCard::Question(question_card) => (
            question_card
                .questions
                .first()
                .map(|question| question.question.clone())
                .unwrap_or_else(|| "question".to_owned()),
            None,
            None,
        ),
    }
}

/// The plan payload for the wire card (and the parked approval slot), when
/// the card is a plan.
fn approval_plan(card: &ApprovalCard) -> Option<PlanApprovalDto> {
    match card {
        ApprovalCard::Plan { summary, steps } => Some(PlanApprovalDto {
            summary: summary.clone(),
            steps: steps.clone(),
        }),
        _ => None,
    }
}

/// §19.4: one question card maps onto the single decision prompt the UI
/// renders (`prompt` on the event, `decision_prompt` on the parked approval).
/// The option id is the label: that is the value the answer echoes back.
fn approval_prompt(card: &ApprovalCard) -> Option<DecisionPromptDto> {
    let ApprovalCard::Question(question_card) = card else {
        return None;
    };
    let question = question_card.questions.first()?;
    Some(DecisionPromptDto::Question {
        header: None,
        question: question.question.clone(),
        options: question
            .options
            .iter()
            .map(|option| QuestionPromptOptionDto {
                id: option.label.clone(),
                label: option.label.clone(),
                description: (!option.description.is_empty()).then(|| option.description.clone()),
            })
            .collect(),
        allow_free_text: question.allow_free_text,
        free_text_placeholder: None,
    })
}

fn plan_phase_dto(phase: selora_ai::PlanPhase) -> PlanPhaseDto {
    match phase {
        selora_ai::PlanPhase::Planning => PlanPhaseDto::Planning,
        selora_ai::PlanPhase::Executing => PlanPhaseDto::Executing,
    }
}

fn plan_approval_dto(plan: &selora_ai::PlanDraft, api_key: &str) -> PlanApprovalDto {
    PlanApprovalDto {
        summary: plan
            .summary
            .as_deref()
            .map(|summary| redact(summary, api_key)),
        steps: plan
            .steps
            .iter()
            .map(|step| redact(step, api_key))
            .collect(),
    }
}

fn todo_dto(todo: &selora_ai::TodoItem) -> TodoDto {
    TodoDto {
        id: todo.id.clone(),
        title: todo.title.clone(),
        status: match todo.status {
            selora_ai::TodoStatus::Pending => TodoStatusDto::Pending,
            selora_ai::TodoStatus::InProgress => TodoStatusDto::InProgress,
            selora_ai::TodoStatus::Completed => TodoStatusDto::Completed,
        },
    }
}

fn fallback_tool_name(card: &ApprovalCard) -> String {
    match card {
        ApprovalCard::Diff { .. } => "write_file".into(),
        ApprovalCard::Delete { .. } => "delete_file".into(),
        ApprovalCard::Command { .. } => "bash".into(),
        ApprovalCard::Workflow { .. } => "run_workflow".into(),
        ApprovalCard::Plan { .. } => "exit_plan_mode".into(),
        ApprovalCard::Question(_) => "ask_user_question".into(),
    }
}

/// Map the runtime's completion onto the wire DTO (one shape for every
/// consumer; the frontend dedupes by `child_id`).
fn subagent_completed_dto(completion: selora_ai::SubagentCompletion) -> SubagentCompletedDto {
    SubagentCompletedDto {
        child_id: completion.child_id,
        parent_session_id: SessionId::new(completion.parent_session)
            .expect("runtime parent sessions originate from a SessionId"),
        policy: match completion.policy {
            selora_ai::SubagentPolicy::ReadOnly => SubagentPolicyDto::ReadOnly,
            selora_ai::SubagentPolicy::Full => SubagentPolicyDto::Full,
        },
        ok: completion.ok,
        error: completion.error,
        final_message: completion.final_message,
        artifacts: completion
            .artifacts
            .into_iter()
            .map(|artifact| SubagentArtifactDto {
                kind: artifact.kind,
                path: artifact.path,
            })
            .collect(),
    }
}

fn map_decision(decision: DecisionDto, answer: Option<String>) -> selora_ai::AgentDecision {
    match decision {
        DecisionDto::Answer => selora_ai::AgentDecision::Answer(answer.unwrap_or_default()),
        DecisionDto::Approve => selora_ai::AgentDecision::Approve,
        DecisionDto::Deny => selora_ai::AgentDecision::Deny,
        DecisionDto::ApproveAllThisTurn => selora_ai::AgentDecision::ApproveAllThisTurn,
        DecisionDto::Revise => answer
            .map(selora_ai::AgentDecision::Answer)
            .unwrap_or(selora_ai::AgentDecision::Revise),
        DecisionDto::Stop => selora_ai::AgentDecision::Stop,
    }
}

fn status_changed(status: ConnectionStatus, active_model: &Option<String>) -> FrontendEvent {
    FrontendEvent::SeloraStatusChanged(SeloraStatusDto {
        status,
        active_model_id: active_model.clone(),
    })
}

/// A Selora error into the protocol's typed error, with the key redacted
/// from the message (the client scrubs its own error bodies; this is the
/// second seam, for anything that slipped through).
fn to_protocol_error(error: &SeloraError, api_key: &str) -> ProtocolErrorDto {
    let (code, retryable) = match error {
        SeloraError::Auth(_) => ("auth_failed", false),
        SeloraError::Net(_) => ("network_error", true),
        SeloraError::Ai(_) => ("gateway_error", true),
        _ => ("chat_failed", false),
    };
    ProtocolErrorDto {
        code: code.into(),
        message: redact(&error.to_string(), api_key),
        retryable,
    }
}

fn redact(text: &str, api_key: &str) -> String {
    if api_key.is_empty() {
        text.to_owned()
    } else {
        text.replace(api_key, "[REDACTED]")
    }
}

/// Apply the same approval redaction to plan summaries and every plan step.
fn redact_card(card: ApprovalCard, api_key: &str) -> ApprovalCard {
    fn scrub(value: &mut serde_json::Value, key: &str) {
        match value {
            serde_json::Value::String(text) => *text = redact(text, key),
            serde_json::Value::Array(values) => {
                values.iter_mut().for_each(|value| scrub(value, key))
            }
            serde_json::Value::Object(values) => {
                values.values_mut().for_each(|value| scrub(value, key))
            }
            _ => {}
        }
    }
    let mut value = serde_json::to_value(card).expect("approval card serializes");
    scrub(&mut value, api_key);
    serde_json::from_value(value).expect("redaction preserves approval schema")
}

fn stale_session() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "stale_workspace_or_session".into(),
        message: "Request a fresh bootstrap snapshot.".into(),
        retryable: true,
    }
}

fn session_busy() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "session_busy".into(),
        message: "A request is already running for this session — stop it first.".into(),
        retryable: false,
    }
}

fn unknown_session(session_id: &SessionId) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "unknown_session".into(),
        message: format!(
            "No session `{}` exists; request a fresh bootstrap.",
            session_id.as_str()
        ),
        retryable: true,
    }
}

fn unknown_message(message_id: &MessageId) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "unknown_message".into(),
        message: format!(
            "No message `{}` exists in this session.",
            message_id.as_str()
        ),
        retryable: false,
    }
}

fn not_a_user_message(message_id: &MessageId) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "not_a_user_message".into(),
        message: format!(
            "Message `{}` is not a user turn; retry needs the user message.",
            message_id.as_str()
        ),
        retryable: false,
    }
}

fn no_model_selected() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "no_model_selected".into(),
        message: "No model selected — pass model_id or set an active model.".into(),
        retryable: false,
    }
}

fn no_workspace() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "no_workspace".into(),
        message: "Agent mode needs a workspace root — open a folder so the agent can \
                  read and write files."
            .into(),
        retryable: false,
    }
}

// ---------------------------------------------------------------------------
// Tests — the Z3 vertical slice, hermetic: a loopback wiremock gateway (the
// selora-ai test pattern), a fixed engine key (the real Credential Manager
// is never touched), and tempdir workspaces. The Tauri-side seams run
// through `tauri::test::mock_app` (the Z2 pattern), the relays through
// `tauri::async_runtime`'s global runtime.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DesktopState;
    use selora_protocol::{
        AgentRequestDto, ChatRequestDto, DecisionDto, EditorContextDto, EventEnvelope,
        FrontendCommand, RequestEnvelope, Selection, TextPosition, WorkspaceId, PROTOCOL_VERSION,
    };
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_TIMEOUT: Duration = Duration::from_secs(10);

    // ------------------------------------------------------------------
    // Fixtures and helpers
    // ------------------------------------------------------------------

    fn mock_app_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
        tauri::test::mock_app().handle().clone()
    }

    fn gateway_settings(base_url: String) -> selora_settings::Settings {
        let mut settings = selora_settings::Settings::default();
        settings.selora.base_url = base_url;
        settings
    }

    fn envelope(
        state: &DesktopState,
        command: FrontendCommand,
    ) -> RequestEnvelope<FrontendCommand> {
        RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            workspace_id: Some(WorkspaceId::new("no-workspace").expect("valid id")),
            core_instance_id: Some(state.core_instance_id()),
            request_id: RequestId::new("transport-1").expect("valid id"),
            session_id: None,
            payload: command,
        }
    }

    fn send_chat(request_id: &str, session: &str, message: &str) -> FrontendCommand {
        FrontendCommand::SendChat {
            request_id: RequestId::new(request_id).expect("valid id"),
            request: ChatRequestDto {
                session_id: SessionId::new(session).expect("valid id"),
                message: message.into(),
                model_id: Some("test-model".into()),
            },
        }
    }

    fn send_agent(request_id: &str, session: &str, prompt: &str) -> FrontendCommand {
        FrontendCommand::SendAgent {
            request_id: RequestId::new(request_id).expect("valid id"),
            request: AgentRequestDto {
                session_id: SessionId::new(session).expect("valid id"),
                prompt: prompt.into(),
                model_id: Some("test-model".into()),
                mode: AgentModeDto::Execute,
                reasoning_effort: None,
                system_instructions: None,
                temperature: None,
                editor_context: None,
            },
        }
    }

    fn agent_decision(
        request_id: &str,
        approval_id: &str,
        decision: DecisionDto,
    ) -> FrontendCommand {
        FrontendCommand::AgentDecision {
            resolution: None,
            request_id: RequestId::new(request_id).expect("valid id"),
            approval_id: ApprovalId::new(approval_id).expect("valid id"),
            decision,
        }
    }

    fn stop_chat(request_id: &str) -> FrontendCommand {
        FrontendCommand::StopChat {
            request_id: RequestId::new(request_id).expect("valid id"),
        }
    }

    /// A tempdir workspace with a seeded NOTES.md the tools are rooted at.
    fn temp_workspace(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("selora-z3-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("workspace dir");
        std::fs::write(root.join("NOTES.md"), "notes-body-xyz\n").expect("seed file");
        root
    }

    /// The SSE chunk builders (the client's own test fixtures' shapes).
    fn content_chunk(text: &str) -> serde_json::Value {
        serde_json::json!({ "choices": [ { "delta": { "content": text } } ] })
    }
    fn fragment_chunk(index: usize, id: &str, name: &str, arguments: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [ { "delta": { "tool_calls": [ {
                "index": index, "id": id, "type": "function",
                "function": { "name": name, "arguments": arguments }
            } ] } } ]
        })
    }
    fn usage_chunk(reason: &str, prompt: u32, completion: u32, cost: f64) -> serde_json::Value {
        serde_json::json!({
            "choices": [ { "delta": {}, "finish_reason": reason } ],
            "usage": { "prompt_tokens": prompt, "completion_tokens": completion, "cost_usd": cost }
        })
    }
    fn sse(chunks: &[serde_json::Value]) -> String {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str("data: ");
            body.push_str(&chunk.to_string());
            body.push_str("\n\n");
        }
        body.push_str("data: [DONE]\n\n");
        body
    }

    /// One streaming round that calls tools: a content marker (the mock
    /// gateway's body matcher), one fragment per call, then `tool_calls`.
    fn tool_round_sse(calls: &[(&str, &str, &str)], marker: &str, cost: f64) -> String {
        let mut chunks = vec![content_chunk(marker)];
        for (index, (id, name, arguments)) in calls.iter().enumerate() {
            chunks.push(fragment_chunk(index, id, name, arguments));
        }
        chunks.push(usage_chunk("tool_calls", 100, 50, cost));
        sse(&chunks)
    }

    /// One streaming round that is the final answer.
    fn answer_round_sse(text: &str, cost: f64) -> String {
        sse(&[content_chunk(text), usage_chunk("stop", 100, 50, cost)])
    }

    /// Wait until a parked approval differs from `previous` (a decision
    /// unparks the turn, which then parks again on the next card).
    fn wait_for_new_approval(
        state: &DesktopState,
        previous: Option<&str>,
    ) -> selora_protocol::ApprovalDto {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            if let Some(pending) = state.bootstrap().chat.pending_approval {
                if previous != Some(pending.approval_id.as_str()) {
                    return pending;
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for a new parked approval"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// A mock gateway: one SSE round per `(body marker, response)`, mounted
    /// in the given order (wiremock matches in mount order — mount the most
    /// specific marker first). `expect_hits` arms a hit-count verification
    /// per round (wiremock verifies on drop) — false for turns that are
    /// stopped before their script plays out.
    fn mock_gateway(rounds: Vec<(&str, String)>, expect_hits: bool) -> MockServer {
        tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            for (marker, body) in rounds {
                let mut mock = Mock::given(method("POST"))
                    .and(path("/v1/chat/completions"))
                    .and(body_string_contains(marker.to_owned()))
                    .respond_with(ResponseTemplate::new(200).set_body_string(body));
                if expect_hits {
                    mock = mock.expect(1);
                }
                mock.mount(&server).await;
            }
            server
        })
    }

    /// Wait until the mock gateway has received at least one request — the
    /// stop tests must not race their cancellation against the relay's
    /// first request (the egui tests' synchronization rule).
    fn sync_with_server(server: &MockServer) {
        let deadline = Instant::now() + TEST_TIMEOUT;
        while tauri::async_runtime::block_on(async { server.received_requests().await })
            .is_none_or(|requests| requests.is_empty())
        {
            assert!(
                Instant::now() < deadline,
                "the request never reached the mock gateway"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Poll the recorded event log until an event matching the predicate
    /// lands (the relays run on tauri::async_runtime's workers; the poll
    /// never blocks the runtime).
    fn wait_for(
        state: &DesktopState,
        predicate: impl Fn(&EventEnvelope<FrontendEvent>) -> bool,
    ) -> Vec<EventEnvelope<FrontendEvent>> {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            let events = state.events_after(0);
            if events.iter().any(&predicate) {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the event; recorded so far:\n{events:#?}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// A compact event tag for order assertions.
    fn tag(event: &EventEnvelope<FrontendEvent>) -> String {
        match &event.payload {
            FrontendEvent::AgentStateChanged(dto) => match dto.state {
                AgentPhase::Thinking => "state:thinking".into(),
                AgentPhase::RunningTool => {
                    format!("state:running:{}", dto.tool_name.as_deref().unwrap_or("?"))
                }
                AgentPhase::AwaitingApproval => "state:awaiting".into(),
                AgentPhase::Done => "state:done".into(),
            },
            FrontendEvent::ChatDelta(dto) => format!("delta:{}", dto.text),
            FrontendEvent::ToolCallStarted(dto) => format!("tool_start:{}", dto.tool_name),
            FrontendEvent::ToolResult(dto) => format!("tool_result:{}", dto.call_id),
            FrontendEvent::ApprovalNeeded(dto) => format!("approval:{}", dto.tool_name),
            FrontendEvent::AgentTurnDone(_) => "turn_done".into(),
            FrontendEvent::ChatCompleted(_) => "completed".into(),
            FrontendEvent::ChatFailed(dto) => format!("failed:{}", dto.error.code),
            other => format!("other:{other:?}"),
        }
    }

    fn find_payload<T>(
        events: &[EventEnvelope<FrontendEvent>],
        predicate: impl Fn(&FrontendEvent) -> Option<T> + Copy,
    ) -> T {
        events
            .iter()
            .filter_map(|event| predicate(&event.payload))
            .next_back()
            .expect("the event is in the log")
    }

    /// A raw TCP "gateway" that streams one delta, then stalls — wiremock
    /// cannot hold a body open mid-stream. The thread parks on the socket
    /// until the client cancels and drops it.
    fn stalling_gateway(first_delta: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the stalling gateway");
        let uri = format!("http://{}", listener.local_addr().expect("addr"));
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\ndata: {}\n\n",
            content_chunk(first_delta)
        );
        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                // Drain the request first: closing a Windows socket with
                // unread data sends RST mid-request and the client reports
                // a send failure instead of a stream.
                let mut sink = [0u8; 8192];
                let _ = socket.read(&mut sink);
                let _ = socket.write_all(response.as_bytes());
                let _ = socket.flush();
                // The stall: hold the connection open (this read parks)
                // until the client cancels and drops it.
                let _ = socket.read(&mut sink);
            }
        });
        uri
    }

    // ------------------------------------------------------------------
    // Typed refusals (no key / no workspace / stale envelopes)
    // ------------------------------------------------------------------

    #[test]
    fn send_chat_without_a_key_fails_not_connected() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings_no_key(selora_settings::Settings::default());
        let error = state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "hi")),
            )
            .expect_err("no key is a typed refusal");
        assert_eq!(error.code, "not_connected");
        assert!(error.retryable);
        assert!(state.events_after(0).is_empty(), "refusals record nothing");
    }

    #[test]
    fn send_agent_without_a_workspace_fails_no_workspace() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let error = state
            .dispatch(
                &app,
                envelope(&state, send_agent("agent-1", "session-1", "fix it")),
            )
            .expect_err("no root is a typed refusal");
        assert_eq!(error.code, "no_workspace");
        assert!(!error.retryable);
        assert!(state.events_after(0).is_empty());
    }

    #[test]
    fn session_commands_reject_stale_envelopes() {
        let app = mock_app_handle();
        for command in [
            send_chat("chat-1", "session-1", "hi"),
            stop_chat("chat-1"),
            send_agent("agent-1", "session-1", "fix it"),
            agent_decision("agent-1", "a-valid-id", DecisionDto::Approve),
        ] {
            let state = DesktopState::with_settings(selora_settings::Settings::default());
            let base = envelope(&state, command.clone());

            // (1) protocol version mismatch.
            let mut bad = base.clone();
            bad.protocol_version = PROTOCOL_VERSION + 1;
            assert_eq!(
                state
                    .dispatch(&app, bad)
                    .expect_err("version mismatch")
                    .code,
                "protocol_version_mismatch"
            );

            // (2) stale core instance.
            let mut bad = base.clone();
            bad.core_instance_id =
                Some(selora_protocol::CoreInstanceId::new("other-core").expect("valid id"));
            assert_eq!(
                state.dispatch(&app, bad).expect_err("stale core").code,
                "stale_core_instance"
            );

            // (3) a foreign workspace id.
            let mut bad = base.clone();
            bad.workspace_id = Some(WorkspaceId::new("a-real-workspace").expect("valid id"));
            assert_eq!(
                state.dispatch(&app, bad).expect_err("stale workspace").code,
                "stale_workspace_or_session"
            );

            // (4) an envelope session that disagrees with the payload's.
            // (Stop/AgentDecision have no payload session: they are refused
            // by the engine instead — the request never existed.)
            let mut bad = base;
            bad.session_id = Some(SessionId::new("another-session").expect("valid id"));
            if matches!(
                command,
                FrontendCommand::SendChat { .. } | FrontendCommand::SendAgent { .. }
            ) {
                assert_eq!(
                    state
                        .dispatch(&app, bad)
                        .expect_err("session mismatch")
                        .code,
                    "stale_workspace_or_session"
                );
            }
            assert!(state.events_after(0).is_empty());
        }
    }

    #[test]
    fn session_commands_accept_their_envelope_session() {
        // A matching envelope session id passes the guards (the engine's
        // own payload comparison is the next line of defense) — proven by
        // reaching the not_connected refusal instead of a stale envelope.
        let app = mock_app_handle();
        let state = DesktopState::with_settings_no_key(selora_settings::Settings::default());
        let mut accepted = envelope(&state, send_chat("chat-1", "session-1", "hi"));
        accepted.session_id = Some(SessionId::new("session-1").expect("valid id"));
        assert_eq!(
            state
                .dispatch(&app, accepted)
                .expect_err("past the envelope, refused at the key")
                .code,
            "not_connected"
        );
    }

    // ------------------------------------------------------------------
    // SendChat: stream, complete, fail, stop, busy
    // ------------------------------------------------------------------

    #[test]
    fn send_chat_streams_deltas_and_lands_the_transcript() {
        let app = mock_app_handle();
        let server = mock_gateway(
            vec![(
                "list 3 primes",
                sse(&[
                    content_chunk("Hello"),
                    content_chunk(" world"),
                    usage_chunk("stop", 9, 2, 0.0013),
                ]),
            )],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "list 3 primes")),
            )
            .expect("send_chat accepted");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });

        // The wire order: two deltas then the completion, monotonic, all
        // bound to the request and the session.
        assert_eq!(
            events.iter().map(tag).collect::<Vec<_>>(),
            vec!["delta:Hello", "delta: world", "completed"]
        );
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        for event in &events {
            assert_eq!(
                event.request_id,
                Some(RequestId::new("chat-1").expect("valid id"))
            );
            assert_eq!(
                event.session_id,
                Some(SessionId::new("session-1").expect("valid id"))
            );
        }

        // The deltas share one message id and count 1, 2 within it.
        let deltas: Vec<&ChatDeltaDto> = events
            .iter()
            .filter_map(|event| match &event.payload {
                FrontendEvent::ChatDelta(dto) => Some(dto),
                _ => None,
            })
            .collect();
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].message_id, deltas[1].message_id);
        assert_eq!(deltas[0].sequence, 1);
        assert_eq!(deltas[1].sequence, 2);

        // The completion carries the whole text and the receipt's metering.
        let completed = find_payload(&events, |payload| match payload {
            FrontendEvent::ChatCompleted(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(completed.text, "Hello world");
        assert_eq!(completed.total_tokens, 11);
        assert!((completed.cost_usd - 0.0013).abs() < 1e-12);

        // The transcript: one session, auto-titled, user + assistant.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.sessions.len(), 1);
        assert_eq!(chat.sessions[0].title, "list 3 primes");
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, MessageRole::User);
        assert_eq!(chat.messages[0].text, "list 3 primes");
        assert_eq!(chat.messages[1].role, MessageRole::Assistant);
        assert_eq!(chat.messages[1].text, "Hello world");
        assert!(chat.active_request_id.is_none(), "the stream ended");
        assert!(chat.pending_approval.is_none());

        // Stopping a finished request is a silent idempotent success.
        state
            .dispatch(&app, envelope(&state, stop_chat("chat-1")))
            .expect("stopping a finished request is fine");
        assert_eq!(state.events_after(0).len(), 3, "no new events");
    }

    #[test]
    fn chat_failure_maps_to_typed_error_and_keeps_the_user_turn() {
        let app = mock_app_handle();
        let server = tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
                .expect(1) // 401 is never retried
                .mount(&server)
                .await;
            server
        });
        let state = DesktopState::with_settings(gateway_settings(server.uri()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "hello?")),
            )
            .expect("send accepted");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatFailed(_))
        });
        let failed = find_payload(&events, |payload| match payload {
            FrontendEvent::ChatFailed(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(failed.error.code, "auth_failed");
        assert!(!failed.error.retryable);
        // The user turn survives for the frontend's Retry.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].text, "hello?");
        // The key never rode an event.
        let json = serde_json::to_string(&events).expect("events serialize");
        assert!(!json.contains("sk-gw-desktop-test-key"));
    }

    #[test]
    fn stop_chat_preserves_the_partial_bubble_silently() {
        let app = mock_app_handle();
        let uri = stalling_gateway("Hello ");
        let state = DesktopState::with_settings(gateway_settings(uri));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "write a poem")),
            )
            .expect("send accepted");
        // The first delta lands while the gateway stalls.
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatDelta(_))
        });

        state
            .dispatch(&app, envelope(&state, stop_chat("chat-1")))
            .expect("stop accepted");

        // A user stop is never a ChatError: the partial bubble completes.
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
        assert!(
            events
                .iter()
                .all(|event| !matches!(event.payload, FrontendEvent::ChatFailed(_))),
            "no failure event for a user stop"
        );
        let completed = find_payload(&events, |payload| match payload {
            FrontendEvent::ChatCompleted(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(completed.text, "Hello ");
        // The partial answer is a transcript entry the restart keeps.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[1].text, "Hello ");
    }

    #[test]
    fn session_busy_refuses_a_second_stream_without_mutating() {
        let app = mock_app_handle();
        // The response stalls server-side: the first stream stays in flight.
        let server = tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .and(body_string_contains("first question".to_owned()))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(sse(&[content_chunk("…"), usage_chunk("stop", 1, 1, 0.0)]))
                        .set_delay(Duration::from_secs(10)),
                )
                .expect(1)
                .mount(&server)
                .await;
            server
        });
        let state = DesktopState::with_settings(gateway_settings(server.uri()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-1", "session-1", "first question")),
            )
            .expect("first send accepted");
        let error = state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-2", "session-1", "second question")),
            )
            .expect_err("the session is busy");
        assert_eq!(error.code, "session_busy");
        assert!(!error.retryable);

        // The refusal never touched the transcript: one user message.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].text, "first question");
        assert_eq!(
            chat.active_request_id.expect("still streaming").as_str(),
            "chat-1"
        );

        // Synchronize before stopping: the first request must have reached
        // the (delayed) mock, or the stop would race it (the egui tests'
        // rule — the relay's request is async).
        sync_with_server(&server);

        // Teardown: stop the stalled stream so the test ends promptly.
        state
            .dispatch(&app, envelope(&state, stop_chat("chat-1")))
            .expect("stop accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
    }

    // ------------------------------------------------------------------
    // SendAgent: the full vertical — tools, approval, decision, turn done
    // ------------------------------------------------------------------

    /// The 3-round script: read NOTES.md (a read tool, auto-runs), write a
    /// line (a diff card — parked until approved), then the final answer.
    fn agent_rounds(new_content: &str) -> Vec<(&'static str, String)> {
        let write_arguments =
            serde_json::json!({ "path": "NOTES.md", "content": new_content }).to_string();
        vec![
            // Round 3 (mount first — its marker only exists once the write
            // result joined the conversation).
            (
                "bytes to",
                sse(&[
                    content_chunk("Done: the line is appended."),
                    usage_chunk("stop", 100, 50, 0.05),
                ]),
            ),
            // Round 2 (the write card round; the request now carries the
            // read result).
            (
                "notes-body-xyz",
                sse(&[
                    fragment_chunk(0, "call_write", "write_file", &write_arguments),
                    usage_chunk("tool_calls", 100, 50, 0.20),
                ]),
            ),
            // Round 1 (only the prompt distinguishes it).
            (
                "append a line",
                sse(&[
                    content_chunk("Reading the notes first."),
                    fragment_chunk(0, "call_read", "read_file", r#"{"path":"NOTES.md"}"#),
                    usage_chunk("tool_calls", 100, 50, 0.10),
                ]),
            ),
        ]
    }

    #[test]
    fn agent_turn_streams_tools_approvals_and_completes() {
        let app = mock_app_handle();
        let root = temp_workspace("full-turn");
        let new_content = "notes-body-xyz\nappended line\n";
        let server = mock_gateway(agent_rounds(new_content), true);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(&state, send_agent("agent-1", "session-1", "append a line")),
            )
            .expect("send_agent accepted");

        // The card parks; its announcement carries the write's diff.
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ApprovalNeeded(_))
        });
        let approval = find_payload(&events, |payload| match payload {
            FrontendEvent::ApprovalNeeded(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(approval.tool_name, "write_file");
        assert_eq!(approval.path_or_command, "NOTES.md");
        let diff = approval.diff_or_input.expect("a diff for a write card");
        assert!(diff.contains("appended line"));
        // The file is untouched — the card IS the save moment.
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("seeded file"),
            "notes-body-xyz\n"
        );

        // The user approves through the command path.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision(
                        "agent-1",
                        approval.approval_id.as_str(),
                        DecisionDto::Approve,
                    ),
                ),
            )
            .expect("the approval resolves");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        // The exact wire order (the contract the dock renders against).
        assert_eq!(
            events.iter().map(tag).collect::<Vec<_>>(),
            vec![
                "state:thinking",
                "delta:Reading the notes first.",
                "state:running:read_file",
                "tool_start:read_file",
                "tool_result:call_read",
                "state:thinking",
                "state:running:write_file",
                "tool_start:write_file",
                "state:awaiting",
                "approval:write_file",
                "state:running:write_file",
                "tool_result:call_write",
                "state:thinking",
                "delta:Done: the line is appended.",
                "state:done",
                "turn_done",
            ]
        );
        // Every event rides the request and the session, one monotonic
        // sequence with no gaps.
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event.sequence, index as u64 + 1);
            assert_eq!(
                event.request_id,
                Some(RequestId::new("agent-1").expect("valid id"))
            );
            assert_eq!(
                event.session_id,
                Some(SessionId::new("session-1").expect("valid id"))
            );
        }

        // The footer: 3 rounds × 150 tokens, the summed cost, 2 tools,
        // 1 approved, the normal exit.
        let done = find_payload(&events, |payload| match payload {
            FrontendEvent::AgentTurnDone(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(done.total_tokens, 450);
        assert!((done.cost_usd - 0.35).abs() < 1e-9);
        assert_eq!(done.num_tools, 2);
        assert_eq!(done.approved, 1);
        assert_eq!(done.denied, 0);
        assert!(done.completed);
        assert_eq!(done.end_message, "agent turn complete");

        // The approval executed for real: the file on disk changed.
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("written file"),
            new_content
        );

        // Single-use: the consumed id can never resolve again.
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision(
                        "agent-1",
                        approval.approval_id.as_str(),
                        DecisionDto::Approve,
                    ),
                ),
            )
            .expect_err("the approval was consumed");
        assert_eq!(error.code, "approval_not_pending");

        // The transcript: the user turn + the final answer; no parked card.
        let chat = state.bootstrap().chat;
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[1].text, "Done: the line is appended.");
        assert!(chat.pending_approval.is_none());
        assert!(chat.active_request_id.is_none());

        // The API key never crossed into an event.
        let json = serde_json::to_string(&events).expect("events serialize");
        assert!(!json.contains("sk-gw-desktop-test-key"));

        // Every scripted round was actually hit.
        tauri::async_runtime::block_on(async {
            server.verify().await;
        });

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_mode_and_todos_ride_the_agent_stream_and_the_chat_snapshot() {
        let app = mock_app_handle();
        let root = temp_workspace("plan-hydrate");
        let server = mock_gateway(
            vec![
                // Mounted most specific first: the next request is
                // identified by the previous round's tool result / prompt.
                (
                    "Stored 2 todo(s)",
                    answer_round_sse("Planning is ready.", 0.01),
                ),
                (
                    "PLAN_HYDRATE_PROMPT",
                    tool_round_sse(
                        &[
                            ("call_enter", "enter_plan_mode", "{}"),
                            (
                                "call_todo",
                                "todo_write",
                                r#"{"todos":[{"title":"Audit the router","status":"completed"},{"title":"Write the plan"}]}"#,
                            ),
                        ],
                        "Reading the workspace first.",
                        0.01,
                    ),
                ),
            ],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_agent("plan-h1", "session-plan", "PLAN_HYDRATE_PROMPT"),
                ),
            )
            .expect("send_agent accepted");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        // The live events are additive and carry the stored state.
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::PlanStateChanged(dto) if dto.phase == PlanPhaseDto::Planning
        )));
        let todos = find_payload(&events, |payload| match payload {
            FrontendEvent::TodosChanged(dto) => Some(dto.todos.clone()),
            _ => None,
        });
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].title, "Audit the router");
        assert_eq!(todos[0].status, TodoStatusDto::Completed);

        // Hydration: the bootstrap snapshot is the source of truth after a
        // reload — the session carries the phase and the same stored list.
        let chat = state.bootstrap().chat;
        let session = chat.sessions.first().expect("the session exists");
        assert_eq!(session.plan_phase, PlanPhaseDto::Planning);
        assert_eq!(session.todos.len(), 2);
        assert_eq!(session.todos[1].title, "Write the plan");
        assert_eq!(session.todos[1].status, TodoStatusDto::Pending);
        assert!(session.plan.is_none(), "no plan was approved yet");
        // The tool results are real: entering plan mode and storing todos
        // succeeded, and the write tools were never called.
        let final_state = state.bootstrap().chat;
        assert!(final_state.pending_approval.is_none());

        tauri::async_runtime::block_on(async {
            server.verify().await;
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_card_revise_keeps_planning_then_approval_flips_and_writes_still_card() {
        let app = mock_app_handle();
        let root = temp_workspace("plan-revise-approve");
        let plan_args = r#"{"summary":"Make the file","plan":["Inspect","Write","Verify"]}"#;
        let server = mock_gateway(
            vec![
                ("wrote 8 bytes", answer_round_sse("Done.", 0.01)),
                (
                    "plan approved",
                    tool_round_sse(
                        &[(
                            "call_write",
                            "write_file",
                            r#"{"path":"made.txt","content":"approved"}"#,
                        )],
                        "Executing the approved plan.",
                        0.01,
                    ),
                ),
                (
                    "asked to revise the plan",
                    tool_round_sse(
                        &[("call_exit_2", "exit_plan_mode", plan_args)],
                        "Revising the plan.",
                        0.01,
                    ),
                ),
                (
                    "PLAN_FLOW_PROMPT",
                    tool_round_sse(
                        &[
                            ("call_enter", "enter_plan_mode", "{}"),
                            ("call_exit", "exit_plan_mode", plan_args),
                        ],
                        "Planning first.",
                        0.01,
                    ),
                ),
            ],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_agent("plan-a", "session-approve", "PLAN_FLOW_PROMPT"),
                ),
            )
            .expect("send_agent accepted");

        // First card: the plan, with its steps on the parked approval slot
        // (so a reload can rebuild the same card).
        let first = wait_for_new_approval(&state, None);
        assert_eq!(first.summary, "exit_plan_mode Make the file");
        let plan = first.plan.expect("the parked card carries the plan");
        assert_eq!(plan.steps, ["Inspect", "Write", "Verify"]);
        assert!(!first.can_approve_all_this_turn);

        // Revise: the turn continues in plan mode and proposes again.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision("plan-a", first.approval_id.as_str(), DecisionDto::Revise),
                ),
            )
            .expect("revise resolves the card");
        let second = wait_for_new_approval(&state, Some(first.approval_id.as_str()));
        assert_eq!(
            plan_phase(&state, "session-approve"),
            PlanPhaseDto::Planning
        );

        // Approve: execution, with the approved plan stored and its steps
        // seeded as todos.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision("plan-a", second.approval_id.as_str(), DecisionDto::Approve),
                ),
            )
            .expect("approve resolves the plan");
        let third = wait_for_new_approval(&state, Some(second.approval_id.as_str()));
        assert_eq!(third.summary, "write_file made.txt");
        let session = state
            .bootstrap()
            .chat
            .sessions
            .into_iter()
            .find(|session| session.session_id.as_str() == "session-approve")
            .expect("session");
        assert_eq!(session.plan_phase, PlanPhaseDto::Executing);
        assert_eq!(session.plan.expect("approved plan").steps.len(), 3);
        assert_eq!(session.todos.len(), 3);
        // The plan approval never wrote the file; its own card is the save.
        assert!(!root.join("made.txt").exists());

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision("plan-a", third.approval_id.as_str(), DecisionDto::Approve),
                ),
            )
            .expect("approve the write");

        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        assert_eq!(
            std::fs::read_to_string(root.join("made.txt")).expect("written file"),
            "approved"
        );

        tauri::async_runtime::block_on(async {
            server.verify().await;
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    fn plan_phase(state: &DesktopState, session_id: &str) -> PlanPhaseDto {
        state
            .bootstrap()
            .chat
            .sessions
            .into_iter()
            .find(|session| session.session_id.as_str() == session_id)
            .expect("session")
            .plan_phase
    }

    #[test]
    fn auto_mode_applies_file_writes_without_an_approval_card() {
        let app = mock_app_handle();
        let root = temp_workspace("auto-mode");
        let new_content = "notes-body-xyz\nauto-applied line\n";
        let server = mock_gateway(agent_rounds(new_content), true);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state.sessions().set_workspace_root(Some(root.clone()));

        let mut command = send_agent("agent-auto", "session-auto", "append a line");
        if let FrontendCommand::SendAgent { request, .. } = &mut command {
            request.mode = AgentModeDto::Auto;
        }
        state
            .dispatch(&app, envelope(&state, command))
            .expect("send_agent accepted");

        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        // No card was ever parked for the write.
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.payload, FrontendEvent::ApprovalNeeded(_))),
            "auto mode must not emit an approval card"
        );
        let done = find_payload(&events, |payload| match payload {
            FrontendEvent::AgentTurnDone(dto) => Some(dto.clone()),
            _ => None,
        });
        assert!(done.completed);
        assert_eq!(done.approved, 1, "the auto-applied write is counted");
        assert_eq!(done.denied, 0);

        // The write executed for real.
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("written file"),
            new_content
        );

        tauri::async_runtime::block_on(async {
            server.verify().await;
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn approval_mismatch_keeps_the_card_parked() {
        let app = mock_app_handle();
        let root = temp_workspace("mismatch");
        let new_content = "notes-body-xyz\nappended line\n";
        let server = mock_gateway(agent_rounds(new_content), true);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(&state, send_agent("agent-1", "session-1", "append a line")),
            )
            .expect("send_agent accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ApprovalNeeded(_))
        });
        let approval = find_payload(&events, |payload| match payload {
            FrontendEvent::ApprovalNeeded(dto) => Some(dto.clone()),
            _ => None,
        });

        // A decision naming a different id is refused — and the real card
        // stays parked (not consumed).
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision("agent-1", "not-the-approval", DecisionDto::Deny),
                ),
            )
            .expect_err("the ids do not line up");
        assert_eq!(error.code, "approval_mismatch");
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("seeded file"),
            "notes-body-xyz\n",
            "a mismatched decision executes nothing"
        );

        // The matching decision still resolves afterwards.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    agent_decision(
                        "agent-1",
                        approval.approval_id.as_str(),
                        DecisionDto::Approve,
                    ),
                ),
            )
            .expect("the real approval resolves");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("written file"),
            new_content
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stop_chat_mid_agent_turn_ends_it_stopped_by_user() {
        let app = mock_app_handle();
        let root = temp_workspace("stop-mid-turn");
        let new_content = "notes-body-xyz\nappended line\n";
        let server = mock_gateway(agent_rounds(new_content), false);
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(&state, send_agent("agent-1", "session-1", "append a line")),
            )
            .expect("send_agent accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ApprovalNeeded(_))
        });

        // StopChat routes to the agent turn too (one stop story): the loop
        // ends without the write ever executing.
        state
            .dispatch(&app, envelope(&state, stop_chat("agent-1")))
            .expect("stop accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });
        let done = find_payload(&events, |payload| match payload {
            FrontendEvent::AgentTurnDone(dto) => Some(dto.clone()),
            _ => None,
        });
        assert!(!done.completed);
        assert_eq!(done.end_message, "agent turn stopped by user");
        assert_eq!(
            std::fs::read_to_string(root.join("NOTES.md")).expect("seeded file"),
            "notes-body-xyz\n",
            "a stopped turn never writes"
        );
        // No card stays parked for a dead turn.
        assert!(state.bootstrap().chat.pending_approval.is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn explore_mode_arms_the_read_only_manifest() {
        let app = mock_app_handle();
        let root = temp_workspace("explore");
        // One tool round (a read), then the answer.
        let server = mock_gateway(
            vec![
                (
                    "NOTES.md (lines",
                    sse(&[
                        content_chunk("The notes are read."),
                        usage_chunk("stop", 100, 50, 0.05),
                    ]),
                ),
                (
                    "explore the notes",
                    sse(&[
                        fragment_chunk(0, "call_read", "read_file", r#"{"path":"NOTES.md"}"#),
                        usage_chunk("tool_calls", 100, 50, 0.10),
                    ]),
                ),
            ],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state.sessions().set_workspace_root(Some(root.clone()));

        let mut command = send_agent("agent-1", "session-1", "explore the notes");
        if let FrontendCommand::SendAgent { request, .. } = &mut command {
            request.mode = AgentModeDto::Explore;
        }
        state
            .dispatch(&app, envelope(&state, command))
            .expect("send_agent accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        // The manifest the loop armed carries only read-only tools: the
        // model cannot even request a side effect.
        let requests = tauri::async_runtime::block_on(async { server.received_requests().await })
            .expect("the two rounds were recorded");
        assert_eq!(requests.len(), 2, "two rounds");
        for request in &requests {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("request body is JSON");
            let tools: Vec<&str> = body["tools"]
                .as_array()
                .expect("the manifest rides every request")
                .iter()
                .map(|tool| tool["function"]["name"].as_str().expect("named tool"))
                .collect();
            assert!(tools.contains(&"read_file"), "read tools armed: {tools:?}");
            for side_effect in ["write_file", "edit_file", "delete_file", "bash"] {
                assert!(
                    !tools.contains(&side_effect),
                    "explore mode never arms {side_effect}: {tools:?}"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // The boot probe (the model picker's data path)
    // ------------------------------------------------------------------

    #[test]
    fn boot_probe_announces_models_and_connection() {
        let server = tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/models"))
                .and(header("authorization", "Bearer sk-gw-desktop-test-key"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({ "object": "list", "data": [
                        {
                            "id": "glm-5.3", "display_name": "GLM 5.3",
                            "context_window": 200000,
                            "price_per_1k_input": 0.01, "price_per_1k_output": 0.03,
                            "supports_streaming": true, "supports_tools": true,
                            "family": "glm"
                        }
                    ]}),
                ))
                .expect(1)
                .mount(&server)
                .await;
            server
        });
        let state = DesktopState::with_settings(gateway_settings(server.uri()));

        tauri::async_runtime::block_on(async {
            state
                .sessions()
                .connect_probe(state.event_log(), mock_app_handle())
                .await;
        });

        let events = state.events_after(0);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].payload, FrontendEvent::ModelsChanged(_)));
        assert!(matches!(
            events[1].payload,
            FrontendEvent::SeloraStatusChanged(_)
        ));
        let boot = state.bootstrap();
        assert_eq!(boot.connection, ConnectionStatus::Connected);
        assert_eq!(boot.models.len(), 1);
        assert_eq!(boot.models[0].display_name, "GLM 5.3");
        // The catalog feeds model resolution: a catalog hit carries the
        // context window (the caps' source).
        assert_eq!(
            state
                .sessions()
                .resolve_model_for_test(Some("glm-5.3"))
                .expect("catalog model resolves")
                .context_window,
            200_000
        );
    }

    // ------------------------------------------------------------------
    // Phase 9 session slice — todos, new/switch isolation, retry
    // ------------------------------------------------------------------

    fn session_command(command: FrontendCommand) -> FrontendCommand {
        command
    }

    fn new_session_command() -> FrontendCommand {
        session_command(FrontendCommand::NewSession)
    }

    fn switch_session_command(session: &str) -> FrontendCommand {
        FrontendCommand::SwitchSession {
            session_id: SessionId::new(session).expect("valid id"),
        }
    }

    fn delete_session_command(session: &str) -> FrontendCommand {
        FrontendCommand::DeleteSession {
            session_id: SessionId::new(session).expect("valid id"),
        }
    }

    fn retry_command(session: &str, message: MessageId) -> FrontendCommand {
        FrontendCommand::RetryMessage {
            session_id: SessionId::new(session).expect("valid id"),
            message_id: message,
        }
    }

    fn messages_for(
        snapshot: &selora_protocol::BootstrapState,
        session: &str,
    ) -> Vec<selora_protocol::MessageDto> {
        snapshot
            .chat
            .messages
            .iter()
            .filter(|message| message.session_id.as_str() == session)
            .cloned()
            .collect()
    }

    #[test]
    fn manage_todo_stores_session_scoped_todos_and_hydrates_the_chat_state() {
        let app = mock_app_handle();
        let root = temp_workspace("todos");
        // Round 1: the model writes a plan. Round 2: the tool result is in the
        // request body, so that marker only matches after the store.
        let server = mock_gateway(
            vec![
                (
                    "Stored 2 todo(s)",
                    sse(&[
                        content_chunk("The plan is stored."),
                        usage_chunk("stop", 10, 5, 0.01),
                    ]),
                ),
                (
                    "plan the work",
                    sse(&[
                        fragment_chunk(
                            0,
                            "call_todo",
                            "manage_todo",
                            r#"{"todos":[
                                {"id":"t1","text":"inspect the router","status":"completed"},
                                {"id":"t2","text":"store the todos","status":"in_progress"}
                            ]}"#,
                        ),
                        usage_chunk("tool_calls", 10, 5, 0.01),
                    ]),
                ),
            ],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state.sessions().set_workspace_root(Some(root.clone()));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_agent("agent-todo", "session-todo", "plan the work"),
                ),
            )
            .expect("send_agent accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        // The live announcement carries the stored list (not the raw args).
        let changed = find_payload(&events, |payload| match payload {
            FrontendEvent::TodosChanged(dto) => Some(dto.clone()),
            _ => None,
        });
        assert_eq!(changed.session_id.as_str(), "session-todo");
        assert_eq!(changed.todos.len(), 2);
        assert_eq!(changed.todos[0].id, "t1");
        assert_eq!(
            changed.todos[0].status,
            selora_protocol::TodoStatusDto::Completed
        );
        assert_eq!(changed.todos[1].title, "store the todos");

        // The bootstrap (the hydrate path) carries the same list, both on
        // the active session and per session.
        let chat = state.bootstrap().chat;
        assert_eq!(
            chat.active_session_id.expect("active").as_str(),
            "session-todo"
        );
        assert_eq!(chat.todos.len(), 2);
        assert_eq!(
            chat.todos[1].status,
            selora_protocol::TodoStatusDto::InProgress
        );
        let session = chat
            .sessions
            .iter()
            .find(|session| session.session_id.as_str() == "session-todo")
            .expect("the session is listed");
        assert_eq!(session.todos.len(), 2);

        // New sessions start empty; switching back restores the same list.
        state
            .dispatch(&app, envelope(&state, new_session_command()))
            .expect("new session");
        let fresh = state.bootstrap().chat;
        assert_eq!(fresh.todos.len(), 0, "todos are session-scoped");
        let fresh_id = fresh.active_session_id.expect("new session active");
        assert_ne!(fresh_id.as_str(), "session-todo");

        state
            .dispatch(
                &app,
                envelope(&state, switch_session_command("session-todo")),
            )
            .expect("switch back");
        let back = state.bootstrap().chat;
        assert_eq!(
            back.active_session_id.expect("active").as_str(),
            "session-todo"
        );
        assert_eq!(back.todos.len(), 2);
        assert_eq!(back.todos[0].title, "inspect the router");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn new_and_switch_sessions_keep_independent_transcripts() {
        let app = mock_app_handle();
        let server = mock_gateway(
            vec![(
                "session A",
                sse(&[content_chunk("answer A"), usage_chunk("stop", 1, 1, 0.0)]),
            )],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        let root = tempfile::tempdir().unwrap();
        state
            .sessions()
            .set_workspace_root(Some(root.path().to_path_buf()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-a", "session-a", "session A")),
            )
            .expect("send accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
        assert_eq!(messages_for(&state.bootstrap(), "session-a").len(), 2);

        // NewSession is a real, empty session; A's transcript stays with A.
        state
            .dispatch(&app, envelope(&state, new_session_command()))
            .expect("new session");
        let created = state.bootstrap();
        let new_id = created.chat.active_session_id.clone().expect("active");
        assert_ne!(new_id.as_str(), "session-a");
        assert!(messages_for(&created, new_id.as_str()).is_empty());
        assert_eq!(messages_for(&created, "session-a").len(), 2);
        // The transition emitted the hydrate snapshot.
        assert!(state.events_after(0).iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::BootstrapState(snapshot)
                if snapshot.chat.active_session_id.as_ref() == Some(&new_id)
        )));

        // Switching back returns A's own transcript and marks A active.
        state
            .dispatch(&app, envelope(&state, switch_session_command("session-a")))
            .expect("switch back");
        let back = state.bootstrap();
        assert_eq!(
            back.chat
                .active_session_id
                .as_ref()
                .expect("active")
                .as_str(),
            "session-a"
        );
        assert_eq!(messages_for(&back, "session-a").len(), 2);
        assert!(messages_for(&back, new_id.as_str()).is_empty());

        // An unknown id is refused, never created implicitly.
        let error = state
            .dispatch(
                &app,
                envelope(&state, switch_session_command("no-such-session")),
            )
            .expect_err("unknown session");
        assert_eq!(error.code, "session_not_found");
        assert_eq!(
            state
                .bootstrap()
                .chat
                .active_session_id
                .expect("active")
                .as_str(),
            "session-a",
            "a refused switch changes nothing"
        );
    }

    #[test]
    fn delete_session_removes_open_and_disk_only_sessions_and_refreshes_bootstrap() {
        let app = mock_app_handle();
        let root = temp_workspace("delete-session");
        seed_session_store(&root, "sess-open");
        seed_session_store(&root, "sess-disk");
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.clone()));
        let open = SessionId::new("sess-open").expect("id");
        engine.switch_session(&open).expect("open the session");

        let before = state.bootstrap();
        assert!(before
            .agent_sessions
            .iter()
            .any(|session| session.session_id.as_str() == "sess-disk"));

        // A disk-only durable session is not in the dock, but delete still
        // removes its store and the next bootstrap no longer offers it.
        state
            .dispatch(&app, envelope(&state, delete_session_command("sess-disk")))
            .expect("delete disk-only");
        assert!(!root.join(".selora/agent-sessions/sess-disk").exists());
        let after_disk = state.bootstrap();
        assert!(!after_disk
            .agent_sessions
            .iter()
            .any(|session| session.session_id.as_str() == "sess-disk"));
        assert!(after_disk
            .chat
            .sessions
            .iter()
            .any(|session| session.session_id.as_str() == "sess-open"));

        // Deleting the open durable session removes both the tab and its
        // transcript, and leaves no stale active id behind.
        state
            .dispatch(&app, envelope(&state, delete_session_command("sess-open")))
            .expect("delete open");
        assert!(!root.join(".selora/agent-sessions/sess-open").exists());
        let after_open = state.bootstrap();
        assert!(after_open.chat.sessions.is_empty());
        assert!(after_open.agent_sessions.is_empty());
        assert!(after_open.chat.active_session_id.is_none());
        assert!(state.events_after(0).iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::BootstrapState(snapshot)
                if snapshot.chat.sessions.is_empty() && snapshot.agent_sessions.is_empty()
        )));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delete_session_keeps_a_non_active_tab_and_selects_a_real_replacement() {
        let root = temp_workspace("delete-active");
        for id in ["sess-a", "sess-b", "sess-c", "sess-disk"] {
            seed_session_store(&root, id);
        }
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.clone()));
        let a = SessionId::new("sess-a").expect("id");
        let b = SessionId::new("sess-b").expect("id");
        let c = SessionId::new("sess-c").expect("id");
        let disk = SessionId::new("sess-disk").expect("id");
        engine.switch_session(&a).expect("open a");
        engine.switch_session(&b).expect("open b");
        engine.switch_session(&c).expect("open c");

        // Deleting another tab must not yank the user's active selection.
        engine.delete_session(&a).expect("delete a");
        engine.delete_session(&disk).expect("delete disk-only");
        let active = state.bootstrap().chat;
        assert_eq!(
            active
                .active_session_id
                .as_ref()
                .expect("still active")
                .as_str(),
            "sess-c"
        );
        assert_eq!(
            active
                .sessions
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-b", "sess-c"]
        );

        // Deleting the active tab selects a real remaining session, not a
        // synthetic row and not the stale id.
        engine.delete_session(&c).expect("delete c");
        let remaining = state.bootstrap().chat;
        assert_eq!(
            remaining
                .active_session_id
                .as_ref()
                .expect("b replaces c")
                .as_str(),
            "sess-b"
        );
        assert_eq!(remaining.sessions.len(), 1);
        assert_eq!(remaining.sessions[0].session_id.as_str(), "sess-b");

        engine.delete_session(&b).expect("delete b");
        assert!(state.bootstrap().chat.active_session_id.is_none());
        assert!(state.bootstrap().chat.sessions.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delete_session_refuses_a_live_turn_without_removing_anything() {
        let app = mock_app_handle();
        let uri = stalling_gateway("delete-live ");
        let state = DesktopState::with_settings(gateway_settings(uri));
        let root = tempfile::tempdir().unwrap();
        state
            .sessions()
            .set_workspace_root(Some(root.path().to_path_buf()));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    send_chat("chat-delete", "session-delete", "stay live"),
                ),
            )
            .expect("send accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatDelta(_))
        });

        let error = state
            .dispatch(
                &app,
                envelope(&state, delete_session_command("session-delete")),
            )
            .expect_err("a live turn refuses delete");
        assert_eq!(error.code, "session_busy");
        let chat = state.bootstrap().chat;
        assert!(chat
            .sessions
            .iter()
            .any(|session| session.session_id.as_str() == "session-delete"));
        assert_eq!(
            chat.active_session_id
                .as_ref()
                .expect("still active")
                .as_str(),
            "session-delete"
        );

        state
            .dispatch(&app, envelope(&state, stop_chat("chat-delete")))
            .expect("stop accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
    }

    #[test]
    fn switch_and_new_preserve_a_live_turn() {
        let app = mock_app_handle();
        let uri = stalling_gateway("first ");
        let state = DesktopState::with_settings(gateway_settings(uri));
        let root = tempfile::tempdir().unwrap();
        state
            .sessions()
            .set_workspace_root(Some(root.path().to_path_buf()));

        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-live", "session-live", "stay live")),
            )
            .expect("send accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatDelta(_))
        });

        state
            .dispatch(&app, envelope(&state, new_session_command()))
            .expect("new session does not cancel another session's turn");
        assert_ne!(
            state.bootstrap().chat.active_session_id.unwrap().as_str(),
            "session-live"
        );
        state
            .dispatch(
                &app,
                envelope(&state, switch_session_command("session-live")),
            )
            .expect("switch returns to the live turn");
        assert_eq!(
            state.bootstrap().chat.active_request_id.unwrap().as_str(),
            "chat-live"
        );

        state
            .dispatch(&app, envelope(&state, stop_chat("chat-live")))
            .expect("stop accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
    }

    #[test]
    fn retry_truncates_the_failed_tail_and_dispatches_once() {
        let app = mock_app_handle();
        // The first attempt fails at the gateway (401 is never retried).
        let failing = tauri::async_runtime::block_on(async {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .and(body_string_contains("retry me"))
                .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
                .expect(1)
                .mount(&server)
                .await;
            server
        });
        let state = DesktopState::with_settings(gateway_settings(failing.uri()));
        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-first", "session-retry", "retry me")),
            )
            .expect("send accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatFailed(_))
        });
        assert_eq!(
            find_payload(&events, |payload| match payload {
                FrontendEvent::ChatFailed(dto) => Some(dto.error.code.clone()),
                _ => None,
            }),
            "auth_failed"
        );
        let failed = state.bootstrap();
        let user_message = messages_for(&failed, "session-retry")
            .into_iter()
            .find(|message| message.role == MessageRole::User)
            .expect("the user turn survives a failure");
        assert_eq!(messages_for(&failed, "session-retry").len(), 1);

        // The retry re-dispatches the same content exactly once against the
        // healthy gateway.
        let success = mock_gateway(
            vec![(
                "retry me",
                sse(&[
                    content_chunk("answer on retry"),
                    usage_chunk("stop", 2, 3, 0.0),
                ]),
            )],
            true,
        );
        state.sessions().set_base_url(success.uri());
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    retry_command("session-retry", user_message.message_id.clone()),
                ),
            )
            .expect("retry accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
        assert_eq!(
            find_payload(&events, |payload| match payload {
                FrontendEvent::ChatCompleted(dto) => Some(dto.text.clone()),
                _ => None,
            }),
            "answer on retry"
        );
        tauri::async_runtime::block_on(async { success.verify().await });

        // Truncation: one user turn, then the retried answer — no duplicate
        // user bubble, no leftover failed bubble.
        let after = state.bootstrap();
        let transcript = messages_for(&after, "session-retry");
        assert_eq!(transcript.len(), 2, "{transcript:#?}");
        assert_eq!(transcript[0].role, MessageRole::User);
        assert_eq!(transcript[0].text, "retry me");
        assert_eq!(transcript[1].role, MessageRole::Assistant);
        assert_eq!(transcript[1].text, "answer on retry");
        assert!(after.chat.active_request_id.is_none());

        // The old user id is gone (it was re-created), and an assistant id
        // is not a retry target.
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    retry_command("session-retry", user_message.message_id),
                ),
            )
            .expect_err("the old id was truncated");
        assert_eq!(error.code, "unknown_message");
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    retry_command("session-retry", transcript[1].message_id.clone()),
                ),
            )
            .expect_err("only user messages retry");
        assert_eq!(error.code, "not_a_user_message");
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    retry_command(
                        "session-retry",
                        MessageId::new("no-such-message").expect("valid id"),
                    ),
                ),
            )
            .expect_err("unknown id");
        assert_eq!(error.code, "unknown_message");
    }

    #[test]
    fn retry_is_refused_during_a_live_turn() {
        let app = mock_app_handle();
        let uri = stalling_gateway("partial ");
        let state = DesktopState::with_settings(gateway_settings(uri));
        state
            .dispatch(
                &app,
                envelope(&state, send_chat("chat-live", "session-live", "stay live")),
            )
            .expect("send accepted");
        let events = wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatDelta(_))
        });
        let live_user = events
            .iter()
            .find(|event| matches!(event.payload, FrontendEvent::ChatDelta(_)))
            .and_then(|_| {
                state
                    .bootstrap()
                    .chat
                    .messages
                    .into_iter()
                    .find(|message| message.role == MessageRole::User)
            })
            .expect("the live user turn");
        let error = state
            .dispatch(
                &app,
                envelope(&state, retry_command("session-live", live_user.message_id)),
            )
            .expect_err("a live turn cannot retry");
        assert_eq!(error.code, "session_busy");
        state
            .dispatch(&app, envelope(&state, stop_chat("chat-live")))
            .expect("stop accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::ChatCompleted(_))
        });
    }

    // ------------------------------------------------------------------
    // The approval card body mapping (the WorkflowCard's data path)
    // ------------------------------------------------------------------

    #[test]
    fn approval_body_maps_the_workflow_card_to_the_confirmation_payload() {
        let card = ApprovalCard::Workflow {
            name: "repo-audit".into(),
            script: "phase('scan');\nawait world.run('cargo', ['test']);\n".into(),
            commands: vec!["cargo test".into(), "git status".into()],
            max_cost_usd: 2.5,
        };

        let (path_or_command, diff_or_input, workflow) = approval_body(&card);
        // The header target stays the workflow name; the script is NOT a diff.
        assert_eq!(path_or_command, "repo-audit");
        assert!(diff_or_input.is_none());
        // `tool_name` stays `run_workflow` so the frontend routes the card.
        assert_eq!(fallback_tool_name(&card), "run_workflow");

        let workflow = workflow.expect("a workflow card carries the payload");
        assert_eq!(workflow.name, "repo-audit");
        assert_eq!(
            workflow.script,
            "phase('scan');\nawait world.run('cargo', ['test']);\n"
        );
        assert_eq!(workflow.commands, vec!["cargo test", "git status"]);
        assert_eq!(workflow.max_cost_usd, 2.5);
    }

    #[test]
    fn approval_body_leaves_non_workflow_cards_unchanged() {
        let (path, diff, workflow) = approval_body(&ApprovalCard::Diff {
            path: "NOTES.md".into(),
            diff: "+line\n".into(),
        });
        assert_eq!(path, "NOTES.md");
        assert_eq!(diff.as_deref(), Some("+line\n"));
        assert!(workflow.is_none());

        let (path, diff, workflow) = approval_body(&ApprovalCard::Delete {
            path: "NOTES.md".into(),
        });
        assert_eq!(path, "NOTES.md");
        assert!(diff.is_none());
        assert!(workflow.is_none());

        let (command, diff, workflow) = approval_body(&ApprovalCard::Command {
            command: "cargo test".into(),
        });
        assert_eq!(command, "cargo test");
        assert!(diff.is_none());
        assert!(workflow.is_none());
        assert_eq!(
            fallback_tool_name(&ApprovalCard::Command {
                command: "cargo test".into()
            }),
            "bash"
        );
    }

    // ------------------------------------------------------------------
    // The system prompt's editor context (path/language/selection/dirty —
    // never the file text)
    // ------------------------------------------------------------------

    fn agent_payload(editor_context: Option<EditorContextDto>) -> AgentRequestDto {
        AgentRequestDto {
            session_id: SessionId::new("session-1").expect("valid id"),
            prompt: "do the thing".into(),
            model_id: Some("test-model".into()),
            mode: AgentModeDto::Execute,
            reasoning_effort: None,
            system_instructions: None,
            temperature: None,
            editor_context,
        }
    }

    #[test]
    fn system_prompt_includes_editor_context() {
        let payload = agent_payload(Some(EditorContextDto {
            path: "crates/selora-ai/src/agent/turn.rs".into(),
            language: "rust".into(),
            selection: Some(Selection {
                anchor: TextPosition {
                    line: 6,
                    utf16_column: 0,
                },
                active: TextPosition {
                    line: 6,
                    utf16_column: 13,
                },
                version: 3,
            }),
            dirty: true,
        }));

        let prompt = system_prompt(&payload, Path::new("C:/repo"));
        assert!(
            prompt.contains("Active file (workspace-relative): crates/selora-ai/src/agent/turn.rs")
        );
        assert!(prompt.contains("Language: rust"));
        // 0-based protocol coordinates print 1-based, like `read_file` does.
        assert!(prompt.contains("Selection: 7:1-7:14"));
        assert!(prompt.contains("Status: has unsaved edits"));
        assert!(prompt.contains("do not claim to have read a file you have not read in this turn"));
    }

    #[test]
    fn system_prompt_without_editor_context_is_unchanged() {
        let payload = agent_payload(None);

        let prompt = system_prompt(&payload, Path::new("C:/repo"));
        assert!(!prompt.contains("Editor context"));

        // Today's prompt, hard-coded: root `C:/repo`, execute mode, standard
        // depth, no custom instructions, and the empty editor slot.
        let expected = format!(
            "You are Selora, an autonomous coding agent in Selora IDE.\n\
             Operating System: Windows. Shell: PowerShell.\n\
             Workspace Root: {}\n\
             Available tools: read_file, list_dir, fuzzy_find, search_files, write_file, edit_file, delete_file, bash, web_search, web_fetch, run_workflow, enter_plan_mode, exit_plan_mode, todo_read, todo_write.\n\
             Instructions:\n\
             1. You can read and change the project. Use `list_dir`, `fuzzy_find`, `search_files`, and `read_file` to inspect it, then `write_file`, `edit_file`, or `delete_file` to make the change.\n\
             2. Do not stop at describing a change: when the user asks for a code change, perform it with the tools. {}\n\
             3. Always inspect relevant files before modifying them, and re-read after a failed edit rather than guessing.\n\
             4. Break complex work into clear steps and keep the live todo list current with `todo_write` (read it with `todo_read`).\n\
             5. For a non-trivial change, call `enter_plan_mode`, explore read-only, then propose the plan with `exit_plan_mode`; while planning, every write, delete, bash command, and workflow is blocked until the user approves the plan. Approving the plan still leaves each write behind its own diff card.\n\
             6. Make clean, accurate edits and verify your work (build, test, or read the result back).{}{}",
            "C:/repo",
            "Writes, deletes, and non-allowlisted commands pause for the user's approval card; explain what you are doing while the card is pending.",
            "\nReasoning depth: Standard. Balanced reasoning with clear explanations and thorough verification.",
            ""
        );
        assert_eq!(prompt, expected);
    }

    #[test]
    fn system_prompt_omits_empty_selection() {
        let payload = agent_payload(Some(EditorContextDto {
            path: "apps/selora-desktop/src/main.rs".into(),
            language: "rust".into(),
            selection: Some(Selection {
                anchor: TextPosition {
                    line: 3,
                    utf16_column: 2,
                },
                active: TextPosition {
                    line: 3,
                    utf16_column: 2,
                },
                version: 1,
            }),
            dirty: false,
        }));

        let prompt = system_prompt(&payload, Path::new("C:/repo"));
        // A collapsed caret is not a selection, and no edits means no
        // status line — never `0:0-0:0` or an empty range.
        assert!(!prompt.contains("Selection:"));
        assert!(!prompt.contains("Status: has unsaved edits"));
        assert!(
            prompt.contains("Active file (workspace-relative): apps/selora-desktop/src/main.rs")
        );
        assert!(prompt.contains("do not claim to have read a file you have not read in this turn"));
    }

    // ------------------------------------------------------------------
    // Phase 9d — durable sessions, resume, retry, session trust
    // ------------------------------------------------------------------

    fn seed_session_store(root: &Path, session: &str) {
        let store = selora_ai::AgentSessionStore::new(root);
        let mut state = store
            .create(
                session,
                "Persisted session",
                Some("test-model"),
                selora_ai::AgentSessionMode::Execute,
            )
            .expect("create session");
        store
            .append(
                session,
                &selora_ai::TranscriptEntry::User {
                    id: "m1".into(),
                    timestamp: 1,
                    content: "first prompt".into(),
                },
            )
            .expect("append user");
        store
            .append(
                session,
                &selora_ai::TranscriptEntry::Assistant {
                    id: "m2".into(),
                    timestamp: 2,
                    content: "first answer".into(),
                    tool_calls: None,
                },
            )
            .expect("append assistant");
        store
            .append(
                session,
                &selora_ai::TranscriptEntry::User {
                    id: "m3".into(),
                    timestamp: 3,
                    content: "second prompt".into(),
                },
            )
            .expect("append user");
        store
            .append(
                session,
                &selora_ai::TranscriptEntry::Assistant {
                    id: "m4".into(),
                    timestamp: 4,
                    content: "second answer old".into(),
                    tool_calls: None,
                },
            )
            .expect("append old second answer");
        state.plan = selora_ai::PlanState {
            status: selora_ai::PlanStatus::Proposed,
            plan: Some("keep the plan".into()),
            updated_at: 4,
        };
        state.parks = vec![selora_ai::ParkedCard::Approval {
            approval_id: Uuid::nil().to_string(),
            tool_name: "bash".into(),
            card: ApprovalCard::Command {
                command: "cargo test".into(),
            },
            request_id: Some("req-park".into()),
            turn_id: Some("turn-park".into()),
            created_at: 5,
        }];
        store.save_state(&state).expect("save state");
        store
            .save_todos(
                session,
                &[selora_ai::TodoItem {
                    id: "t1".into(),
                    title: "restore me".into(),
                    status: selora_ai::TodoStatus::InProgress,
                }],
            )
            .expect("save todos");
    }

    #[test]
    fn switch_restores_a_session_and_its_park_expires_once() {
        let root = temp_workspace("resume");
        seed_session_store(&root, "sess-restore");
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        state.sessions().set_workspace_root(Some(root.clone()));

        let listed = state.sessions().agent_sessions();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Persisted session");
        assert_eq!(listed[0].message_count, 4);
        assert_eq!(listed[0].mode, "execute");
        assert_eq!(listed[0].pending.as_ref().expect("park").kind, "approval");

        // A fresh process has not restored anything yet.
        let before = state.bootstrap().chat;
        assert!(before.sessions.is_empty());
        assert!(before.messages.is_empty());

        state
            .sessions()
            .switch_session(&SessionId::new("sess-restore").expect("id"))
            .expect("switch restores");
        let chat = state.bootstrap().chat;
        assert_eq!(chat.sessions.len(), 1);
        assert_eq!(chat.messages.len(), 4);
        assert_eq!(chat.messages[0].text, "first prompt");
        assert_eq!(chat.messages[1].text, "first answer");
        assert_eq!(chat.messages[2].text, "second prompt");
        assert_eq!(chat.messages[3].text, "second answer old");
        let park = chat.pending_approval.expect("the parked card is restored");
        assert_eq!(park.request_id.as_str(), "req-park");

        // A restored park is actionable exactly once and never executes.
        let error = state
            .sessions()
            .agent_decision(
                &RequestId::new("req-park").expect("id"),
                &ApprovalId::new(Uuid::nil().to_string()).expect("id"),
                DecisionDto::Approve,
                None,
            )
            .expect_err("a restored park expires");
        assert_eq!(error.code, "park_expired");
        assert!(state.bootstrap().chat.pending_approval.is_none());
        let error = state
            .sessions()
            .agent_decision(
                &RequestId::new("req-park").expect("id"),
                &ApprovalId::new(Uuid::nil().to_string()).expect("id"),
                DecisionDto::Approve,
                None,
            )
            .expect_err("the second decision has no card");
        assert_eq!(error.code, "approval_not_pending");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn switch_restores_the_plan_phase_draft_and_todos() {
        let root = temp_workspace("resume-plan");
        seed_session_store(&root, "sess-plan");
        let store = selora_ai::AgentSessionStore::new(&root);
        let mut record = store
            .load("sess-plan")
            .expect("load")
            .expect("the seeded session exists")
            .state;
        record.plan = selora_ai::PlanState {
            status: selora_ai::PlanStatus::Proposed,
            plan: Some(
                serde_json::to_string(&selora_ai::PlanDraft {
                    summary: Some("ship it".into()),
                    steps: vec!["Read the router".into(), "Write the plan".into()],
                })
                .expect("draft json"),
            ),
            updated_at: 7,
        };
        store.save_state(&record).expect("save state");
        store
            .save_todos(
                "sess-plan",
                &[selora_ai::TodoItem {
                    id: "t1".into(),
                    title: "Read the router".into(),
                    status: selora_ai::TodoStatus::Completed,
                }],
            )
            .expect("save todos");

        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.clone()));
        let session_id = SessionId::new("sess-plan").expect("id");
        engine.switch_session(&session_id).expect("switch restores");

        let chat = state.bootstrap().chat;
        let session = chat.sessions.first().expect("the session exists");
        assert_eq!(session.plan_phase, PlanPhaseDto::Planning);
        assert_eq!(session.todos.len(), 1);
        assert_eq!(session.todos[0].title, "Read the router");
        assert_eq!(session.todos[0].status, TodoStatusDto::Completed);
        let plan = session.plan.as_ref().expect("the draft hydrates");
        assert_eq!(plan.steps, vec!["Read the router", "Write the plan"]);
        assert_eq!(plan.summary.as_deref(), Some("ship it"));

        // The router's structural gate sees the restored phase.
        let planning = {
            let inner = engine.inner.lock().expect("engine lock");
            selora_ai::plan::is_planning(
                &inner.sessions.get(&session_id).expect("session").plan_store,
            )
        };
        assert!(planning, "a proposed plan reopens as planning");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn session_auto_approve_is_off_by_default_and_resets_on_switch_and_new() {
        let root = temp_workspace("auto-scope");
        seed_session_store(&root, "sess-a");
        seed_session_store(&root, "sess-b");
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.clone()));

        let a = SessionId::new("sess-a").expect("id");
        let b = SessionId::new("sess-b").expect("id");
        engine.switch_session(&a).expect("switch a");
        let flag = |session: &SessionId| {
            engine
                .inner
                .lock()
                .expect("engine")
                .sessions
                .get(session)
                .expect("session")
                .auto_approve
        };
        assert!(!flag(&a), "off by default");
        engine.set_auto_approve(&a, true).expect("arm a");
        assert!(flag(&a));

        // Switching cannot inherit the flag, and it clears the source too.
        engine.switch_session(&b).expect("switch b");
        assert!(!flag(&a), "the source session is disarmed");
        assert!(!flag(&b), "the target never inherits trust");

        engine.set_auto_approve(&b, true).expect("arm b");
        engine.new_session().expect("new session");
        assert!(!flag(&b), "a new session resets trust");

        // Restart: the flag has no on-disk representation, so a fresh engine
        // restores the session with trust off.
        let restarted = DesktopState::with_settings(selora_settings::Settings::default());
        restarted.sessions().set_workspace_root(Some(root.clone()));
        restarted
            .sessions()
            .switch_session(&a)
            .expect("restore after restart");
        assert!(
            !restarted
                .sessions()
                .inner
                .lock()
                .expect("engine")
                .sessions
                .get(&a)
                .expect("session")
                .auto_approve
        );
        let name = ".selora/agent-sessions/sess-a/state.json";
        let text = std::fs::read_to_string(root.join(name)).expect("state file");
        assert!(!text.contains("auto_approve"), "{name} persisted trust");
        assert!(
            !text.contains("sk-gw-desktop-test-key"),
            "{name} leaked the key"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn planning_without_a_draft_survives_restart_and_refuses_all_effects_until_approval() {
        let root = tempfile::tempdir().unwrap();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let engine = state.sessions();
        engine.set_workspace_root(Some(root.path().to_path_buf()));
        let id = engine.new_session().unwrap();
        let live = engine.inner.lock().unwrap().sessions[&id]
            .plan_store
            .clone();
        selora_ai::plan::lock(&live).phase = selora_ai::PlanPhase::Planning;
        engine.sync_plan_store(&id);
        drop(engine);
        drop(state);

        let restarted = DesktopState::with_settings(selora_settings::Settings::default());
        let engine = restarted.sessions();
        engine.set_workspace_root(Some(root.path().to_path_buf()));
        engine.switch_session(&id).unwrap();
        let live = engine.inner.lock().unwrap().sessions[&id]
            .plan_store
            .clone();
        assert!(selora_ai::plan::is_planning(&live));
        let context = ToolContext::new(root.path(), selora_ai::AgentToolSettings::default())
            .with_plan(live.clone());
        tauri::async_runtime::block_on(async {
            for (name, arguments) in [
                ("write_file", r#"{"path":"never.txt","content":"no"}"#),
                ("bash", r#"{"command":"Set-Content never.txt no"}"#),
                ("run_workflow", r#"{"name":"no","script":"log('no');"}"#),
            ] {
                let call = selora_ai::ToolCall {
                    id: name.into(),
                    name: name.into(),
                    arguments: arguments.into(),
                };
                let selora_ai::ToolOutcome::Ready(result) =
                    selora_ai::execute_tool(&context, call.clone()).await
                else {
                    panic!("planning must refuse before a card");
                };
                assert!(!result.ok);
                assert!(!selora_ai::execute_approved(&context, call).await.ok);
            }
            let todos = selora_ai::ToolCall {
                id: "todos".into(),
                name: "manage_todo".into(),
                arguments: r#"{"todos":[{"id":"t1","text":"Inspect","status":"pending"}]}"#.into(),
            };
            assert!(
                matches!(selora_ai::execute_tool(&context, todos).await, selora_ai::ToolOutcome::Ready(result) if result.ok)
            );
            let exit = selora_ai::ToolCall {
                id: "exit".into(),
                name: "exit_plan_mode".into(),
                arguments: r#"{"summary":"Review","plan":["Inspect","Write"]}"#.into(),
            };
            assert!(matches!(
                selora_ai::execute_tool(&context, exit.clone()).await,
                selora_ai::ToolOutcome::NeedsApproval(ApprovalCard::Plan { .. })
            ));
            engine.sync_plan_store(&id);
            let restored = engine.load_session_from_disk(&id).unwrap();
            let snapshot = selora_ai::plan::snapshot(&restored.plan_store);
            assert_eq!(snapshot.phase, selora_ai::PlanPhase::Planning);
            assert_eq!(snapshot.todos[0].title, "Inspect");
            assert_eq!(snapshot.plan.unwrap().steps, ["Inspect", "Write"]);
            let context = ToolContext::new(root.path(), selora_ai::AgentToolSettings::default())
                .with_plan(restored.plan_store);
            assert!(selora_ai::execute_approved(&context, exit).await.ok);
            let write = selora_ai::ToolCall {
                id: "write".into(),
                name: "write_file".into(),
                arguments: r#"{"path":"approved.txt","content":"yes"}"#.into(),
            };
            assert!(matches!(
                selora_ai::execute_tool(&context, write.clone()).await,
                selora_ai::ToolOutcome::NeedsApproval(ApprovalCard::Diff { .. })
            ));
            assert!(selora_ai::execute_approved(&context, write).await.ok);
        });
        assert!(!root.path().join("never.txt").exists());
        assert_eq!(
            std::fs::read_to_string(root.path().join("approved.txt")).unwrap(),
            "yes"
        );
    }

    #[test]
    fn retry_rewinds_the_durable_transcript_and_resends() {
        let app = mock_app_handle();
        let root = temp_workspace("retry");
        seed_session_store(&root, "sess-retry");
        let durable = selora_ai::AgentSessionStore::new(&root);
        let mut prefix = durable.load_transcript("sess-retry").unwrap().entries;
        let split = prefix.iter().position(|entry| matches!(entry, TranscriptEntry::User { content, .. } if content == "second prompt")).unwrap();
        prefix.splice(
            split..split,
            [
                TranscriptEntry::ToolCalls {
                    id: "batch-original".into(),
                    timestamp: 1,
                    calls: vec![
                        selora_ai::ToolCall {
                            id: "write-original".into(),
                            name: "write_file".into(),
                            arguments: r#"{"path":"once.txt","content":"once"}"#.into(),
                        },
                        selora_ai::ToolCall {
                            id: "bash-original".into(),
                            name: "bash".into(),
                            arguments: r#"{"command":"Add-Content once.txt duplicate"}"#.into(),
                        },
                    ],
                },
                TranscriptEntry::Tool {
                    id: "write-result".into(),
                    timestamp: 2,
                    call_id: "write-original".into(),
                    ok: true,
                    content: "written once".into(),
                },
                TranscriptEntry::Tool {
                    id: "bash-result".into(),
                    timestamp: 3,
                    call_id: "bash-original".into(),
                    ok: true,
                    content: "ran once".into(),
                },
            ],
        );
        durable.rewrite_transcript("sess-retry", &prefix).unwrap();
        std::fs::write(root.join("once.txt"), "once").unwrap();
        let server = mock_gateway(
            vec![(
                "second prompt",
                sse(&[
                    content_chunk("second answer"),
                    usage_chunk("stop", 10, 5, 0.001),
                ]),
            )],
            true,
        );
        let state = DesktopState::with_settings(gateway_settings(server.uri()));
        state.sessions().set_workspace_root(Some(root.clone()));
        let session = SessionId::new("sess-retry").expect("id");
        state
            .sessions()
            .switch_session(&session)
            .expect("restore before retry");

        let chat = state.bootstrap().chat;
        let retry_target = chat
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User && message.text == "second prompt")
            .expect("the second user message")
            .message_id
            .clone();

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::RetryMessage {
                        session_id: session.clone(),
                        message_id: retry_target,
                    },
                ),
            )
            .expect("retry accepted");
        wait_for(&state, |event| {
            matches!(event.payload, FrontendEvent::AgentTurnDone(_))
        });

        let store = selora_ai::AgentSessionStore::new(&root);
        let transcript = store.load_transcript("sess-retry").expect("transcript");
        let user_texts: Vec<&str> = transcript
            .entries
            .iter()
            .filter_map(|entry| match entry {
                selora_ai::TranscriptEntry::User { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            user_texts,
            vec!["first prompt", "second prompt"],
            "the abandoned branch is dropped and the retried message re-sent"
        );
        assert!(
            transcript.entries.iter().any(|entry| matches!(
                entry,
                selora_ai::TranscriptEntry::Assistant { content, .. }
                    if content == "first answer"
            )),
            "history before the retried message is kept"
        );
        assert!(
            !transcript.entries.iter().any(|entry| matches!(
                entry,
                selora_ai::TranscriptEntry::Assistant { content, .. }
                    if content == "second answer old"
            )),
            "the abandoned branch is gone from the durable transcript"
        );
        assert!(transcript.entries.iter().any(|entry| matches!(entry, TranscriptEntry::ToolCalls { calls, .. } if calls.len() == 2 && calls[1].id == "bash-original")));
        assert!(transcript.entries.iter().any(|entry| matches!(entry, TranscriptEntry::Tool { call_id, .. } if call_id == "write-original")));
        let requests = tauri::async_runtime::block_on(server.received_requests()).unwrap();
        assert_eq!(requests.len(), 1, "retry sends exactly one request");
        let request: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["tool_call_id"] == "bash-original"));
        assert_eq!(
            std::fs::read_to_string(root.join("once.txt")).unwrap(),
            "once"
        );

        // An abandoned turn with attempted effects cannot be retried blindly.
        durable
            .append(
                "sess-retry",
                &TranscriptEntry::ToolCalls {
                    id: "unfinished-effect".into(),
                    timestamp: 4,
                    calls: vec![selora_ai::ToolCall {
                        id: "unsafe-bash".into(),
                        name: "bash".into(),
                        arguments: "{}".into(),
                    }],
                },
            )
            .unwrap();
        let target = transcript
            .entries
            .iter()
            .rev()
            .find(|entry| matches!(entry, TranscriptEntry::User { .. }))
            .unwrap();
        let error = state
            .sessions()
            .retry_message(
                state.event_log(),
                app.clone(),
                None,
                session,
                MessageId::new(target.id()).unwrap(),
            )
            .unwrap_err();
        assert_eq!(error.code, "retry_partial_effects");
        assert_eq!(
            std::fs::read_to_string(root.join("once.txt")).unwrap(),
            "once"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn plan_text_is_redacted_on_approval_events_and_bootstrap() {
        const KEY: &str = "sk-gw-desktop-test-key";
        let redacted = redact_card(
            ApprovalCard::Plan {
                summary: Some(format!("use {KEY}")),
                steps: vec![format!("run {KEY}")],
            },
            KEY,
        );
        let ApprovalCard::Plan { summary, steps } = redacted else {
            panic!("redaction preserves the plan card");
        };
        assert_eq!(summary.as_deref(), Some("use [REDACTED]"));
        assert_eq!(steps, vec!["run [REDACTED]"]);

        let root = tempfile::tempdir().unwrap();
        let state = DesktopState::with_settings(gateway_settings("http://127.0.0.1:9".into()));
        state
            .sessions()
            .set_workspace_root(Some(root.path().to_path_buf()));
        let id = state.sessions().new_session().unwrap();
        let live = state.sessions().inner.lock().unwrap().sessions[&id]
            .plan_store
            .clone();
        {
            let mut plan = selora_ai::plan::lock(&live);
            plan.phase = selora_ai::PlanPhase::Planning;
            plan.plan = Some(selora_ai::PlanDraft {
                summary: Some(KEY.into()),
                steps: vec![KEY.into()],
            });
        }
        let session = state
            .bootstrap()
            .chat
            .sessions
            .into_iter()
            .find(|session| session.session_id == id)
            .unwrap();
        let plan = session.plan.expect("plan hydrates");
        assert_eq!(plan.summary.as_deref(), Some("[REDACTED]"));
        assert_eq!(plan.steps, vec!["[REDACTED]"]);
    }

    #[test]
    fn session_state_on_disk_never_contains_the_api_key() {
        let root = temp_workspace("no-key");
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        state.sessions().set_workspace_root(Some(root.clone()));
        let session = state.sessions().new_session().expect("new session");
        // The engine writes the state through its redacting store; even a
        // title that echoed the key must not survive.
        let engine = state.sessions();
        {
            let mut inner = engine.inner.lock().expect("engine");
            if let Some(open) = inner.sessions.get_mut(&session) {
                open.title = "sk-gw-desktop-test-key".into();
            }
        }
        engine.record_compaction(
            &session,
            selora_ai::CompactionCursor {
                compacted_at: 1,
                ..Default::default()
            },
        );
        let dir = root.join(".selora/agent-sessions").join(session.as_str());
        let state_json = std::fs::read_to_string(dir.join("state.json")).expect("state");
        assert!(!state_json.contains("sk-gw-desktop-test-key"));
        assert!(state_json.contains("[REDACTED]"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn phase_9e_background_completion_emits_one_typed_event() {
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let log = Arc::new(EventLog::new(state.core_instance_id()));
        let app = mock_app_handle();
        let engine = Arc::new(SessionEngine::new(EngineConfig {
            base_url: "http://127.0.0.1:1".into(),
            active_model: None,
            agent: selora_settings::AgentSettings::default(),
            key_source: ApiKeySource::Fixed(None),
        }));

        let spawn = tauri::async_runtime::block_on(async {
            engine.spawn_background_subagent(
                Arc::clone(&log),
                app,
                selora_ai::SpawnRequest::new(
                    "session-1",
                    "child-desktop-1",
                    selora_ai::SubagentPolicy::ReadOnly,
                    "review",
                ),
                async {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    selora_ai::ChildOutcome::success("review complete")
                },
            )
        });
        spawn.expect("spawn");

        let deadline = Instant::now() + TEST_TIMEOUT;
        let events = loop {
            let events = log.events_after(0);
            if events
                .iter()
                .any(|event| matches!(event.payload, FrontendEvent::SubagentCompleted(_)))
                || Instant::now() >= deadline
            {
                break events;
            }
            tauri::async_runtime::block_on(async {
                tokio::time::sleep(Duration::from_millis(5)).await;
            });
        };

        let completions: Vec<_> = events
            .into_iter()
            .filter_map(|event| match event.payload {
                FrontendEvent::SubagentCompleted(completion) => Some(completion),
                _ => None,
            })
            .collect();
        assert_eq!(completions.len(), 1, "one event, never a duplicate");
        let completion = &completions[0];
        assert_eq!(completion.child_id, "child-desktop-1");
        assert_eq!(
            completion.parent_session_id,
            SessionId::new("session-1").unwrap()
        );
        assert!(completion.ok);
        assert_eq!(completion.final_message, "review complete");
    }
}
