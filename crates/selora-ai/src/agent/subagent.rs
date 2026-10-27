//! The shared subagent runtime (ARCHITECTURE.md §19.3, ROADMAP Phase 9c).
//!
//! One runtime, two consumers: the interactive `spawn_subagent` tool and
//! Phase 8's workflow `agent()` host. The pieces that matter are deliberately
//! small:
//!
//! * [`SubagentPolicy`] is a structural tool policy. `read-only` has no write
//!   or bash tools at all; the router rejects them on **both** entry points
//!   (`execute_tool` and `execute_approved`). `full` restores the §17.1 tools,
//!   but writes and bash still build approval cards — a child never inherits
//!   the parent's `ApproveAllThisTurn` or session auto-approve grant.
//! * [`SubagentRuntime`] owns background children, their conversations, FIFO
//!   steering through [`SubagentRuntime::send_message`], cancellation, and
//!   exactly one completion event per child.
//! * [`run_subagent_turn`] is the turn-level adapter both the runtime and the
//!   Phase 8 host use. It is the ordinary §17 loop with the policy manifest
//!   armed; there is no second loop and no process-level trick.
//!
//! A child's approval cards are surfaced as [`SubagentEvent::ApprovalNeeded`]
//! carrying its id, so the parent session can render them on the normal card
//! rail. [`SubagentRuntime::decide`] consumes a card exactly once before
//! forwarding the decision, which is what makes stale and duplicate answers
//! inert.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::router;
use super::turn::{self, AgentTurnState, ChatRoundFuture, ToolApprovedFuture, ToolRunFuture};
use super::{
    AgentDecision, AgentEvent, AgentSummary, AgentTurnReport, ApprovalCard, ApprovalId, ChatStream,
    ToolRunner, TurnCaps, TurnDecision, TurnEnd, TurnInput,
};
use crate::chat_types::{ToolCall, ToolResult};
use crate::types::{ChatEvent, ChatMessage, ChatRequest, Model};

/// A subagent's structural tool policy (§19.3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubagentPolicy {
    /// Read tools only. The router has no write or bash arm available.
    ReadOnly,
    /// The §17.1 tool set. File writes and every non-allowlisted bash command
    /// still park on an approval card.
    #[default]
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
        router::tool_manifest()
            .into_iter()
            .filter(|spec| self.allows(&spec.name))
            .collect()
    }

    /// The policy's wire spelling (`read-only` / `full`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Full => "full",
        }
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

/// Stable child identity. It is a UUID string on the wire and in the parent
/// transcript.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubagentId(String);

impl SubagentId {
    /// Mint a fresh identity.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse a model/session supplied id.
    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(Self(trimmed.to_owned()))
        }
    }
}

impl Default for SubagentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SubagentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A child's live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Working,
    Idle,
    Done,
    Cancelled,
    Failed,
}

/// One request to spawn a child.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentRequest {
    /// Optional caller-chosen name; a stable child-<short-id> name is minted
    /// otherwise.
    pub name: Option<String>,
    /// The structural policy.
    pub policy: SubagentPolicy,
    /// The mission/instructions for the child.
    pub mission: String,
    /// `true` returns immediately (a completion event follows); `false`
    /// waits for the first completion in [`SubagentRuntime::spawn`].
    pub background: bool,
    /// Optional persona/system supplement.
    pub persona: Option<String>,
    /// Optional seed conversation (Phase 8 passes its per-agent history).
    /// The mission is appended as a user message regardless.
    pub conversation: Option<Vec<ChatMessage>>,
}

impl SubagentRequest {
    /// A foreground/background request with the required fields.
    pub fn new(policy: SubagentPolicy, mission: impl Into<String>, background: bool) -> Self {
        Self {
            name: None,
            policy,
            mission: mission.into(),
            background,
            persona: None,
            conversation: None,
        }
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
    pub fn with_conversation(mut self, conversation: Vec<ChatMessage>) -> Self {
        self.conversation = Some(conversation);
        self
    }
}

/// A completed child turn's result, delivered once.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentCompletion {
    pub id: SubagentId,
    /// The parent turn/session identity supplied to the runtime, for grouping.
    pub parent_id: Option<String>,
    pub name: String,
    pub policy: SubagentPolicy,
    pub mission: String,
    pub final_message: String,
    pub summary: AgentSummary,
    pub end: TurnEnd,
    /// The child's complete conversation (tool calls and results included).
    pub conversation: Vec<ChatMessage>,
    pub cancelled: bool,
}

/// A child status snapshot (the parent's roster/panel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: SubagentId,
    pub parent_id: Option<String>,
    pub name: String,
    pub policy: SubagentPolicy,
    pub mission: String,
    pub status: SubagentStatus,
    pub tokens: u64,
    pub cost_usd_bits: u64,
}

impl SubagentInfo {
    /// Estimated USD spent by this child so far.
    #[must_use]
    pub fn cost_usd(&self) -> f64 {
        f64::from_bits(self.cost_usd_bits)
    }
}

/// Everything the runtime tells its parent. All events carry the child id;
/// `Started`/`Completed` also carry the parent identity so two children from
/// one parent group together.
#[derive(Debug, Clone, PartialEq)]
pub enum SubagentEvent {
    Started {
        id: SubagentId,
        parent_id: Option<String>,
        name: String,
        policy: SubagentPolicy,
        mission: String,
    },
    State {
        id: SubagentId,
        state: AgentTurnState,
    },
    /// The child's live text. The parent may render it under the child card.
    Delta {
        id: SubagentId,
        text: String,
    },
    ToolCallStarted {
        id: SubagentId,
        call: ToolCall,
    },
    ToolResult {
        id: SubagentId,
        result: ToolResult,
    },
    /// A child write/bash (or question) card. The parent renders it on the
    /// normal card rail and answers through [`SubagentRuntime::decide`].
    ApprovalNeeded {
        id: SubagentId,
        approval_id: ApprovalId,
        card: ApprovalCard,
    },
    /// The child finished a mission (or was cancelled). Exactly one per
    /// child run; `send_message` after completion starts a new run.
    Completed(SubagentCompletion),
    /// The child could not start or the loop failed before a report.
    Failed {
        id: SubagentId,
        parent_id: Option<String>,
        message: String,
    },
}

/// Runtime errors. Every variant is safe to show to the parent model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentError {
    Unknown(SubagentId),
    Cancelled(SubagentId),
    Timeout(SubagentId),
    Backend(String),
    Closed,
}

impl fmt::Display for SubagentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(formatter, "subagent `{id}` is unknown"),
            Self::Cancelled(id) => write!(formatter, "subagent `{id}` was cancelled"),
            Self::Timeout(id) => write!(
                formatter,
                "subagent `{id}` did not finish within the requested time"
            ),
            Self::Backend(message) => write!(formatter, "subagent backend failed: {message}"),
            Self::Closed => formatter.write_str("subagent runtime is shutting down"),
        }
    }
}

impl std::error::Error for SubagentError {}

/// The seam between the runtime and the app/desktop shell. A backend creates
/// one fresh chat stream and one policy-scoped tool runner per turn; it must
/// not share mutable turn state between children.
pub trait SubagentBackend: Send + Sync {
    /// A chat stream for a child turn.
    fn chat(&self) -> Result<Box<dyn ChatStream>, String>;

    /// A router runner whose [`super::ToolContext`] carries `policy`. The
    /// backend must set the policy on both router paths.
    fn tools(&self, policy: SubagentPolicy) -> Result<Box<dyn ToolRunner>, String>;
}

/// A policy-scoped, single-conversation child. Phase 8's host uses this
/// directly under its own scheduler; the interactive runtime uses it for
/// background execution. It is intentionally the ordinary §17 loop.
pub struct SubagentSession {
    policy: SubagentPolicy,
    model: Model,
    caps: TurnCaps,
    conversation: Vec<ChatMessage>,
}

impl SubagentSession {
    /// A new child conversation. The mission becomes its first user message.
    pub fn new(
        policy: SubagentPolicy,
        name: &str,
        persona: Option<&str>,
        model: Model,
        caps: TurnCaps,
        system_prompt: Option<String>,
        mission: &str,
    ) -> Self {
        let system = system_prompt.unwrap_or_else(|| child_system_prompt(name, policy, persona));
        Self {
            policy,
            model,
            caps,
            conversation: vec![ChatMessage::system(system), ChatMessage::user(mission)],
        }
    }

    /// Seed a session from an existing per-agent history and append the new
    /// mission. An empty history gets the policy system prompt.
    pub fn from_history(
        policy: SubagentPolicy,
        name: &str,
        persona: Option<&str>,
        model: Model,
        caps: TurnCaps,
        mut conversation: Vec<ChatMessage>,
        mission: &str,
    ) -> Self {
        if conversation
            .first()
            .is_none_or(|message| message.role != crate::types::Role::System)
        {
            conversation.insert(
                0,
                ChatMessage::system(child_system_prompt(name, policy, persona)),
            );
        }
        conversation.push(ChatMessage::user(mission));
        Self {
            policy,
            model,
            caps,
            conversation,
        }
    }

    /// The manifest this child arms.
    #[must_use]
    pub fn manifest(&self) -> Vec<crate::chat_types::ToolSpec> {
        self.policy.manifest()
    }

    /// The conversation this session currently holds.
    #[must_use]
    pub fn conversation(&self) -> &[ChatMessage] {
        &self.conversation
    }

    /// Run one turn. The turn's grown conversation replaces this session's
    /// conversation, so a caller may run another turn later.
    pub async fn run(
        &mut self,
        chat: Box<dyn ChatStream>,
        tools: Box<dyn ToolRunner>,
        events: mpsc::UnboundedSender<AgentEvent>,
        decisions: mpsc::UnboundedReceiver<TurnDecision>,
    ) -> AgentTurnReport {
        let input = TurnInput::new(self.model.clone(), self.conversation.clone())
            .with_caps(self.caps.clone())
            .with_manifest(self.manifest())
            .with_auto_approve_writes(false);
        let report = turn::run_agent_turn(input, chat, tools, events, decisions).await;
        self.conversation = report.conversation.clone();
        report
    }
}

/// Run one policy-scoped subagent turn. This is the shared turn adapter the
/// interactive runtime and the Phase 8 workflow host both call.
#[allow(clippy::too_many_arguments)]
pub async fn run_subagent_turn(
    policy: SubagentPolicy,
    model: Model,
    caps: TurnCaps,
    conversation: Vec<ChatMessage>,
    chat: Box<dyn ChatStream>,
    tools: Box<dyn ToolRunner>,
    events: mpsc::UnboundedSender<AgentEvent>,
    decisions: mpsc::UnboundedReceiver<TurnDecision>,
) -> AgentTurnReport {
    let input = TurnInput::new(model, conversation)
        .with_caps(caps)
        .with_manifest(policy.manifest())
        // A child never inherits the parent session's auto-approve grant.
        .with_auto_approve_writes(false);
    turn::run_agent_turn(input, chat, tools, events, decisions).await
}

// ---------------------------------------------------------------------------
// Blanket impls so the backend can hand the loop boxed seams.
// ---------------------------------------------------------------------------

impl ChatStream for Box<dyn ChatStream> {
    fn stream_round(
        &mut self,
        request: ChatRequest,
        events: mpsc::UnboundedSender<ChatEvent>,
    ) -> ChatRoundFuture<'_> {
        (**self).stream_round(request, events)
    }
}

impl ToolRunner for Box<dyn ToolRunner> {
    fn run(&mut self, call: ToolCall) -> ToolRunFuture {
        (**self).run(call)
    }

    fn run_approved(&mut self, call: ToolCall) -> ToolApprovedFuture {
        (**self).run_approved(call)
    }

    fn abort(&mut self) {
        (**self).abort();
    }
}

// ---------------------------------------------------------------------------
// The runtime
// ---------------------------------------------------------------------------

struct ChildState {
    id: SubagentId,
    parent_id: Option<String>,
    name: String,
    policy: SubagentPolicy,
    mission: String,
    status: SubagentStatus,
    conversation: Vec<ChatMessage>,
    pending_messages: VecDeque<String>,
    tokens: u64,
    cost_bits: u64,
    iterations: u32,
    tools_used: u32,
    approved: u32,
    denied: u32,
    running: bool,
    cancelled: bool,
    /// The card currently parked, if any. Consumed before a decision is
    /// forwarded.
    pending_approval: Option<ApprovalId>,
    decisions: Option<mpsc::UnboundedSender<TurnDecision>>,
    completion: Option<SubagentCompletion>,
    /// Set once the one completion event has been emitted.
    completion_emitted: bool,
}

struct RuntimeInner {
    children: BTreeMap<SubagentId, ChildState>,
}

/// The shared background subagent runtime.
pub struct SubagentRuntime {
    backend: Arc<dyn SubagentBackend>,
    model: Model,
    caps: TurnCaps,
    handle: tokio::runtime::Handle,
    events: mpsc::UnboundedSender<SubagentEvent>,
    parent_id: Option<String>,
    shared: Arc<(Mutex<RuntimeInner>, Condvar)>,
    next_serial: AtomicU64,
}

impl fmt::Debug for SubagentRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubagentRuntime")
            .field("model", &self.model.id)
            .field("parent_id", &self.parent_id)
            .field("children", &self.children())
            .finish_non_exhaustive()
    }
}

impl SubagentRuntime {
    /// Build a runtime. `events` is the parent's completion/approval rail;
    /// `handle` is the runtime that owns child tasks.
    pub fn new(
        backend: Arc<dyn SubagentBackend>,
        model: Model,
        caps: TurnCaps,
        handle: tokio::runtime::Handle,
        events: mpsc::UnboundedSender<SubagentEvent>,
    ) -> Self {
        Self {
            backend,
            model,
            caps,
            handle,
            events,
            parent_id: None,
            shared: Arc::new((
                Mutex::new(RuntimeInner {
                    children: BTreeMap::new(),
                }),
                Condvar::new(),
            )),
            next_serial: AtomicU64::new(1),
        }
    }

    /// Group this runtime's events under a parent turn/session identity.
    #[must_use]
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    /// The grouped parent identity, if any.
    #[must_use]
    pub fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }

    /// Spawn a child. A foreground request waits for the first completion;
    /// a background one returns as soon as the child task is registered.
    pub async fn spawn(
        &self,
        request: SubagentRequest,
    ) -> Result<SubagentCompletion, SubagentError> {
        if request.mission.trim().is_empty() {
            return Err(SubagentError::Backend(
                "subagent mission must not be empty".into(),
            ));
        }
        let id = SubagentId::new();
        let serial = self.next_serial.fetch_add(1, Ordering::SeqCst);
        let name = request
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("subagent-{serial}"));
        let policy = request.policy;
        let mission = request.mission;
        let mut conversation = request.conversation.unwrap_or_default();
        if conversation
            .first()
            .is_none_or(|message| message.role != crate::types::Role::System)
        {
            conversation.insert(
                0,
                ChatMessage::system(child_system_prompt(
                    &name,
                    policy,
                    request.persona.as_deref(),
                )),
            );
        }
        let parent_id = self.parent_id.clone();
        {
            let mut inner = self.lock();
            inner.children.insert(
                id.clone(),
                ChildState {
                    id: id.clone(),
                    parent_id: parent_id.clone(),
                    name: name.clone(),
                    policy,
                    mission: mission.clone(),
                    status: SubagentStatus::Working,
                    conversation,
                    // The mission is this child's first user turn. Steering
                    // messages join the same FIFO.
                    pending_messages: VecDeque::from([mission.clone()]),
                    tokens: 0,
                    cost_bits: 0.0f64.to_bits(),
                    iterations: 0,
                    tools_used: 0,
                    approved: 0,
                    denied: 0,
                    running: true,
                    cancelled: false,
                    pending_approval: None,
                    decisions: None,
                    completion: None,
                    completion_emitted: false,
                },
            );
        }
        let _ = self.events.send(SubagentEvent::Started {
            id: id.clone(),
            parent_id: parent_id.clone(),
            name,
            policy,
            mission,
        });
        self.start_child(id.clone());

        if request.background {
            let info = self
                .info(&id)
                .ok_or_else(|| SubagentError::Unknown(id.clone()))?;
            return Ok(SubagentCompletion {
                id: info.id,
                parent_id,
                name: info.name,
                policy: info.policy,
                mission: info.mission,
                final_message: "started in the background".to_owned(),
                summary: AgentSummary {
                    iterations: 0,
                    tools_used: 0,
                    approved: 0,
                    denied: 0,
                    tokens: 0,
                    cost_usd: 0.0,
                    completed: false,
                },
                end: TurnEnd::Completed,
                conversation: Vec::new(),
                cancelled: false,
            });
        }

        self.wait(&id, None).await
    }

    /// Queue a steering message for a running child, or resume a completed
    /// one. FIFO: a child drains queued messages in arrival order before it
    /// is marked done.
    pub fn send_message(
        &self,
        id: &SubagentId,
        text: impl Into<String>,
    ) -> Result<(), SubagentError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(SubagentError::Backend(
                "send_message text must not be empty".into(),
            ));
        }
        let (start, cancelled) = {
            let mut inner = self.lock();
            let child = inner
                .children
                .get_mut(id)
                .ok_or_else(|| SubagentError::Unknown(id.clone()))?;
            if child.cancelled {
                return Err(SubagentError::Cancelled(id.clone()));
            }
            child.pending_messages.push_back(text.clone());
            let start = !child.running;
            if start {
                child.running = true;
                child.status = SubagentStatus::Working;
                child.completion = None;
                // A resumed run gets its own single completion event.
                child.completion_emitted = false;
            }
            (start, child.cancelled)
        };
        if cancelled {
            return Err(SubagentError::Cancelled(id.clone()));
        }
        if start {
            self.start_child(id.clone());
        }
        Ok(())
    }

    /// Answer one child's parked card. The pending slot is consumed before
    /// the decision is sent, so a duplicate or stale id can never apply.
    pub fn decide(
        &self,
        approval_id: ApprovalId,
        decision: AgentDecision,
    ) -> Result<(), SubagentError> {
        let mut inner = self.lock();
        let Some(child) = inner
            .children
            .values_mut()
            .find(|child| child.pending_approval == Some(approval_id))
        else {
            return Err(SubagentError::Backend(
                "no matching live subagent approval (stale or already used)".into(),
            ));
        };
        child.pending_approval = None;
        let sender = child
            .decisions
            .as_ref()
            .ok_or_else(|| SubagentError::Backend("the subagent turn is no longer live".into()))?;
        if sender.is_closed() {
            return Err(SubagentError::Backend(
                "the subagent turn is no longer live".into(),
            ));
        }
        sender
            .send((approval_id, decision))
            .map_err(|_| SubagentError::Backend("the subagent turn is no longer live".into()))
    }

    /// The child's current card, if one is parked.
    #[must_use]
    pub fn pending_approval(&self, id: &SubagentId) -> Option<ApprovalId> {
        self.lock()
            .children
            .get(id)
            .and_then(|child| child.pending_approval)
    }

    /// Wait for a child's completion. `None` waits indefinitely.
    pub async fn wait(
        &self,
        id: &SubagentId,
        timeout: Option<Duration>,
    ) -> Result<SubagentCompletion, SubagentError> {
        if self.info(id).is_none() {
            return Err(SubagentError::Unknown(id.clone()));
        }
        let deadline = timeout.map(|limit| tokio::time::Instant::now() + limit);
        loop {
            if let Some(completion) = self.completion(id) {
                return Ok(completion);
            }
            if let Some(deadline) = deadline {
                if tokio::time::Instant::now() >= deadline {
                    return Err(SubagentError::Timeout(id.clone()));
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Wait for every current child and return their completions, ordered by
    /// id. A `wait` tool call without an id uses this.
    pub async fn wait_all(
        &self,
        timeout: Option<Duration>,
    ) -> Result<Vec<SubagentCompletion>, SubagentError> {
        let ids: Vec<SubagentId> = self.children().into_iter().map(|info| info.id).collect();
        let deadline = timeout.map(|limit| tokio::time::Instant::now() + limit);
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            let remaining = deadline.map(|deadline| {
                deadline
                    .saturating_duration_since(tokio::time::Instant::now())
                    .max(Duration::from_millis(1))
            });
            results.push(self.wait(&id, remaining).await?);
        }
        Ok(results)
    }

    /// Blocking-safe adapter for the Phase 8 host (which is synchronous by
    /// contract). It waits on the runtime's condvar, never on a nested Tokio
    /// runtime, so it is safe on a `spawn_blocking` worker.
    pub fn wait_blocking(
        &self,
        id: &SubagentId,
        timeout: Option<Duration>,
    ) -> Result<SubagentCompletion, SubagentError> {
        let (lock, wake) = &*self.shared;
        let deadline = timeout.map(|limit| std::time::Instant::now() + limit);
        let mut inner = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !inner.children.contains_key(id) {
            return Err(SubagentError::Unknown(id.clone()));
        }
        loop {
            if let Some(completion) = inner
                .children
                .get(id)
                .and_then(|child| child.completion.clone())
            {
                return Ok(completion);
            }
            if let Some(deadline) = deadline {
                let now = std::time::Instant::now();
                if now >= deadline {
                    return Err(SubagentError::Timeout(id.clone()));
                }
                let (next, _) = wake
                    .wait_timeout(inner, deadline.saturating_duration_since(now))
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                inner = next;
            } else {
                inner = wake
                    .wait(inner)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
    }

    /// Cancel one child: queued steering is discarded and a live turn gets
    /// the normal Stop decision (which kills a bash child through the
    /// router's abort handle).
    pub fn cancel(&self, id: &SubagentId) -> Result<(), SubagentError> {
        let sender = {
            let mut inner = self.lock();
            let child = inner
                .children
                .get_mut(id)
                .ok_or_else(|| SubagentError::Unknown(id.clone()))?;
            child.cancelled = true;
            child.pending_messages.clear();
            child.pending_approval = None;
            if child.running {
                child.decisions.clone()
            } else {
                None
            }
        };
        if let Some(sender) = sender {
            let _ = sender.send((Uuid::nil(), AgentDecision::Stop));
        }
        self.wake();
        Ok(())
    }

    /// Cancel every child. The parent Stop path calls this.
    pub fn cancel_all(&self) {
        let senders: Vec<mpsc::UnboundedSender<TurnDecision>> = {
            let mut inner = self.lock();
            let mut senders = Vec::new();
            for child in inner.children.values_mut() {
                child.cancelled = true;
                child.pending_messages.clear();
                child.pending_approval = None;
                if child.running {
                    if let Some(sender) = child.decisions.clone() {
                        senders.push(sender);
                    }
                }
            }
            senders
        };
        for sender in senders {
            let _ = sender.send((Uuid::nil(), AgentDecision::Stop));
        }
        self.wake();
    }

    /// The child roster, ordered by id.
    #[must_use]
    pub fn children(&self) -> Vec<SubagentInfo> {
        self.lock()
            .children
            .values()
            .map(|child| SubagentInfo {
                id: child.id.clone(),
                parent_id: child.parent_id.clone(),
                name: child.name.clone(),
                policy: child.policy,
                mission: child.mission.clone(),
                status: child.status,
                tokens: child.tokens,
                cost_usd_bits: child.cost_bits,
            })
            .collect()
    }

    /// One child's info.
    #[must_use]
    pub fn info(&self, id: &SubagentId) -> Option<SubagentInfo> {
        self.lock().children.get(id).map(|child| SubagentInfo {
            id: child.id.clone(),
            parent_id: child.parent_id.clone(),
            name: child.name.clone(),
            policy: child.policy,
            mission: child.mission.clone(),
            status: child.status,
            tokens: child.tokens,
            cost_usd_bits: child.cost_bits,
        })
    }

    /// One completion, if the child has finished.
    #[must_use]
    pub fn completion(&self, id: &SubagentId) -> Option<SubagentCompletion> {
        self.lock()
            .children
            .get(id)
            .and_then(|child| child.completion.clone())
    }

    fn start_child(&self, id: SubagentId) {
        let chat = match self.backend.chat() {
            Ok(chat) => chat,
            Err(message) => {
                self.finish_failed(&id, message);
                return;
            }
        };
        let policy = match self.lock().children.get(&id) {
            Some(child) => child.policy,
            None => return,
        };
        let tools = match self.backend.tools(policy) {
            Ok(tools) => tools,
            Err(message) => {
                self.finish_failed(&id, message);
                return;
            }
        };
        let runtime = self.clone_handle();
        self.handle.spawn(async move {
            runtime.drive_child(id, chat, tools).await;
        });
    }

    fn clone_handle(&self) -> RuntimeHandle {
        RuntimeHandle {
            backend: Arc::clone(&self.backend),
            model: self.model.clone(),
            caps: self.caps.clone(),
            events: self.events.clone(),
            shared: Arc::clone(&self.shared),
            parent_id: self.parent_id.clone(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, RuntimeInner> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wake(&self) {
        self.shared.1.notify_all();
    }

    fn finish_failed(&self, id: &SubagentId, message: String) {
        let (parent_id, name, policy, mission, conversation) = {
            let mut inner = self.lock();
            let Some(child) = inner.children.get_mut(id) else {
                return;
            };
            child.status = SubagentStatus::Failed;
            child.running = false;
            (
                child.parent_id.clone(),
                child.name.clone(),
                child.policy,
                child.mission.clone(),
                child.conversation.clone(),
            )
        };
        let _ = self.events.send(SubagentEvent::Failed {
            id: id.clone(),
            parent_id: parent_id.clone(),
            message: message.clone(),
        });
        let failed = SubagentCompletion {
            id: id.clone(),
            parent_id,
            name,
            policy,
            mission,
            final_message: message.clone(),
            summary: empty_summary(false),
            end: TurnEnd::Failed(message),
            conversation,
            cancelled: false,
        };
        self.store_completion(id, failed);
        self.wake();
    }

    fn store_completion(&self, id: &SubagentId, completion: SubagentCompletion) {
        let emit = {
            let mut inner = self.lock();
            if let Some(child) = inner.children.get_mut(id) {
                child.completion = Some(completion.clone());
                if !child.completion_emitted {
                    child.completion_emitted = true;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if emit {
            let _ = self.events.send(SubagentEvent::Completed(completion));
        }
        self.wake();
    }
}

struct RuntimeHandle {
    backend: Arc<dyn SubagentBackend>,
    model: Model,
    caps: TurnCaps,
    events: mpsc::UnboundedSender<SubagentEvent>,
    shared: Arc<(Mutex<RuntimeInner>, Condvar)>,
    parent_id: Option<String>,
}

impl RuntimeHandle {
    async fn drive_child(
        &self,
        id: SubagentId,
        chat: Box<dyn ChatStream>,
        tools: Box<dyn ToolRunner>,
    ) {
        let mut chat = chat;
        let mut tools = tools;
        let mut iterations = 0u64;
        loop {
            let (policy, conversation, message) = {
                let mut inner = self.lock();
                let Some(child) = inner.children.get_mut(&id) else {
                    return;
                };
                if child.cancelled {
                    child.status = SubagentStatus::Cancelled;
                    child.running = false;
                    let cancelled = SubagentCompletion {
                        id: id.clone(),
                        parent_id: child.parent_id.clone(),
                        name: child.name.clone(),
                        policy: child.policy,
                        mission: child.mission.clone(),
                        final_message: String::new(),
                        summary: AgentSummary {
                            iterations: child.iterations,
                            tools_used: child.tools_used,
                            approved: child.approved,
                            denied: child.denied,
                            tokens: child.tokens,
                            cost_usd: f64::from_bits(child.cost_bits),
                            completed: false,
                        },
                        end: TurnEnd::StoppedByUser,
                        conversation: child.conversation.clone(),
                        cancelled: true,
                    };
                    child.completion = Some(cancelled.clone());
                    if !child.completion_emitted {
                        child.completion_emitted = true;
                        drop(inner);
                        let _ = self.events.send(SubagentEvent::Completed(cancelled));
                    }
                    self.wake();
                    return;
                }
                let Some(message) = child.pending_messages.pop_front() else {
                    if iterations == 0 {
                        drop(inner);
                        let runtime = self.runtime();
                        runtime.finish_failed(&id, "subagent had no mission to run".into());
                        return;
                    }
                    child.status = SubagentStatus::Done;
                    child.running = false;
                    let completion = SubagentCompletion {
                        id: id.clone(),
                        parent_id: child.parent_id.clone(),
                        name: child.name.clone(),
                        policy: child.policy,
                        mission: child.mission.clone(),
                        final_message: child
                            .conversation
                            .iter()
                            .rev()
                            .find(|message| message.role == crate::types::Role::Assistant)
                            .map(|message| message.content.clone())
                            .unwrap_or_default(),
                        summary: AgentSummary {
                            iterations: child.iterations,
                            tools_used: child.tools_used,
                            approved: child.approved,
                            denied: child.denied,
                            tokens: child.tokens,
                            cost_usd: f64::from_bits(child.cost_bits),
                            completed: true,
                        },
                        end: TurnEnd::Completed,
                        conversation: child.conversation.clone(),
                        cancelled: false,
                    };
                    child.completion = Some(completion.clone());
                    if !child.completion_emitted {
                        child.completion_emitted = true;
                        drop(inner);
                        let _ = self.events.send(SubagentEvent::Completed(completion));
                    }
                    self.wake();
                    return;
                };
                child.conversation.push(ChatMessage::user(&message));
                (child.policy, child.conversation.clone(), message)
            };
            let _ = message;
            let _ = iterations;
            let (decision_tx, decision_rx) = mpsc::unbounded_channel();
            {
                let mut inner = self.lock();
                if let Some(child) = inner.children.get_mut(&id) {
                    child.decisions = Some(decision_tx.clone());
                }
            }
            let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();
            let turn = run_subagent_turn(
                policy,
                self.model.clone(),
                self.caps.clone(),
                conversation,
                chat,
                tools,
                event_tx,
                decision_rx,
            );
            tokio::pin!(turn);
            let report = loop {
                tokio::select! {
                    biased;
                    report = &mut turn => break report,
                    event = event_rx.recv() => match event {
                        Some(event) => self.relay_event(&id, event),
                        None => {
                            // The turn future still owns the sender until it
                            // returns; keep polling it.
                        }
                    },
                }
            };
            while let Ok(event) = event_rx.try_recv() {
                self.relay_event(&id, event);
            }
            {
                let mut inner = self.lock();
                if let Some(child) = inner.children.get_mut(&id) {
                    child.decisions = None;
                    child.pending_approval = None;
                    child.tokens = child.tokens.saturating_add(report.summary.tokens);
                    child.cost_bits =
                        (f64::from_bits(child.cost_bits) + report.summary.cost_usd).to_bits();
                    child.iterations = child.iterations.saturating_add(report.summary.iterations);
                    child.tools_used = child.tools_used.saturating_add(report.summary.tools_used);
                    child.approved = child.approved.saturating_add(report.summary.approved);
                    child.denied = child.denied.saturating_add(report.summary.denied);
                    child.conversation = report.conversation.clone();
                } else {
                    return;
                }
            }
            iterations += 1;

            // `run_subagent_turn` consumed the boxes by value; the backend
            // creates a fresh pair for the next steered turn.
            match self.backend.chat() {
                Ok(next) => chat = next,
                Err(message) => {
                    self.runtime().finish_failed(&id, message);
                    return;
                }
            }
            match self.backend.tools(policy) {
                Ok(next) => tools = next,
                Err(message) => {
                    self.runtime().finish_failed(&id, message);
                    return;
                }
            }
        }
    }

    fn relay_event(&self, id: &SubagentId, event: AgentEvent) {
        match event {
            AgentEvent::State(state) => {
                if !matches!(state, AgentTurnState::AwaitingApproval(_)) {
                    if let Some(child) = self.lock().children.get_mut(id) {
                        child.pending_approval = None;
                    }
                }
                let _ = self.events.send(SubagentEvent::State {
                    id: id.clone(),
                    state,
                });
            }
            AgentEvent::Delta(text) => {
                let _ = self.events.send(SubagentEvent::Delta {
                    id: id.clone(),
                    text,
                });
            }
            AgentEvent::ToolCallStarted(call) => {
                let _ = self.events.send(SubagentEvent::ToolCallStarted {
                    id: id.clone(),
                    call,
                });
            }
            AgentEvent::ToolResult(result) => {
                let _ = self.events.send(SubagentEvent::ToolResult {
                    id: id.clone(),
                    result,
                });
            }
            AgentEvent::ApprovalNeeded { approval_id, card } => {
                let installed = {
                    let mut inner = self.lock();
                    if let Some(child) = inner.children.get_mut(id) {
                        if child.cancelled {
                            false
                        } else {
                            child.pending_approval = Some(approval_id);
                            true
                        }
                    } else {
                        false
                    }
                };
                if installed {
                    let _ = self.events.send(SubagentEvent::ApprovalNeeded {
                        id: id.clone(),
                        approval_id,
                        card,
                    });
                }
            }
            AgentEvent::TurnDone(_) => {}
        }
    }

    fn lock(&self) -> MutexGuard<'_, RuntimeInner> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wake(&self) {
        self.shared.1.notify_all();
    }

    fn runtime(&self) -> RuntimeHandle {
        RuntimeHandle {
            backend: Arc::clone(&self.backend),
            model: self.model.clone(),
            caps: self.caps.clone(),
            events: self.events.clone(),
            shared: Arc::clone(&self.shared),
            parent_id: self.parent_id.clone(),
        }
    }
}

impl RuntimeHandle {
    fn finish_failed(&self, id: &SubagentId, message: String) {
        let (parent_id, name, policy, mission) = {
            let mut inner = self.lock();
            let Some(child) = inner.children.get_mut(id) else {
                return;
            };
            child.status = SubagentStatus::Failed;
            child.running = false;
            (
                child.parent_id.clone(),
                child.name.clone(),
                child.policy,
                child.mission.clone(),
            )
        };
        let _ = self.events.send(SubagentEvent::Failed {
            id: id.clone(),
            parent_id: parent_id.clone(),
            message: message.clone(),
        });
        let completion = SubagentCompletion {
            id: id.clone(),
            parent_id,
            name,
            policy,
            mission,
            final_message: message.clone(),
            summary: empty_summary(false),
            end: TurnEnd::Failed(message),
            conversation: Vec::new(),
            cancelled: false,
        };
        let emit = {
            let mut inner = self.lock();
            if let Some(child) = inner.children.get_mut(id) {
                child.completion = Some(completion.clone());
                if !child.completion_emitted {
                    child.completion_emitted = true;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if emit {
            let _ = self.events.send(SubagentEvent::Completed(completion));
        }
        self.wake();
    }
}

fn empty_summary(completed: bool) -> AgentSummary {
    AgentSummary {
        iterations: 0,
        tools_used: 0,
        approved: 0,
        denied: 0,
        tokens: 0,
        cost_usd: 0.0,
        completed,
    }
}

fn child_system_prompt(name: &str, policy: SubagentPolicy, persona: Option<&str>) -> String {
    let policy_line = match policy {
        SubagentPolicy::ReadOnly => {
            "Policy: read-only. Only read tools exist: read_file, list_dir, fuzzy_find, \
             search_files, web_search, web_fetch. You cannot write, edit, delete, or run shell \
             commands, and no approval can widen this policy."
        }
        SubagentPolicy::Full => {
            "Policy: full. The §17.1 read and write tools are available, but every file write, \
             edit, delete, and non-allowlisted shell command parks on an approval card owned by \
             the parent session. Approval is not a grant you can assume."
        }
    };
    let mut prompt = format!(
        "You are the `{name}` subagent in Selora IDE. You were spawned by a parent agent to \
         complete one mission.\n\n{policy_line}\n\n\
         Stay inside the workspace root. Do not spawn nested subagents or ask the user direct \
         questions; report evidence and answer the mission. Never claim to have changed a file \
         or run a command you did not actually run."
    );
    if let Some(persona) = persona.map(str::trim).filter(|value| !value.is_empty()) {
        prompt.push_str("\n\nPersona:\n");
        prompt.push_str(persona);
    }
    prompt
}

/// A blocking-safe helper for hosts that run on a `spawn_blocking` worker.
/// It drives one policy-scoped turn on a dedicated current-thread runtime;
/// callers must not already be inside a Tokio runtime context.
#[allow(clippy::too_many_arguments)]
pub fn run_subagent_turn_blocking(
    policy: SubagentPolicy,
    model: Model,
    caps: TurnCaps,
    conversation: Vec<ChatMessage>,
    chat: Box<dyn ChatStream>,
    tools: Box<dyn ToolRunner>,
    events: mpsc::UnboundedSender<AgentEvent>,
    decisions: mpsc::UnboundedReceiver<TurnDecision>,
) -> Result<AgentTurnReport, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot start subagent runtime: {error}"))?;
    Ok(runtime.block_on(run_subagent_turn(
        policy,
        model,
        caps,
        conversation,
        chat,
        tools,
        events,
        decisions,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;

    use tokio::sync::oneshot;

    use crate::agent::tools::testutil::temp_root;
    use crate::agent::{ApprovalCard, QuestionAnswer, QuestionResponse, ToolOutcome};
    use crate::types::{ChatEvent, ChatMessage, ChatReceipt, Role};

    #[derive(Default)]
    struct BackendState {
        rounds: Mutex<VecDeque<Round>>,
        requests: Mutex<Vec<ChatRequest>>,
    }

    enum Round {
        Reply(String),
        Calls(Vec<(String, String, String)>),
        Paused {
            started: oneshot::Sender<()>,
            resume: oneshot::Receiver<()>,
            answer: String,
        },
    }

    struct MockBackend {
        state: Arc<BackendState>,
        root: std::path::PathBuf,
    }

    impl MockBackend {
        fn new(root: std::path::PathBuf, rounds: Vec<Round>) -> Arc<Self> {
            Arc::new(Self {
                state: Arc::new(BackendState {
                    rounds: Mutex::new(rounds.into()),
                    requests: Mutex::new(Vec::new()),
                }),
                root,
            })
        }

        fn requests(&self) -> Vec<ChatRequest> {
            self.state.requests.lock().expect("requests lock").clone()
        }
    }

    struct MockChat {
        state: Arc<BackendState>,
    }

    impl ChatStream for MockChat {
        fn stream_round(
            &mut self,
            request: ChatRequest,
            events: mpsc::UnboundedSender<ChatEvent>,
        ) -> ChatRoundFuture<'_> {
            self.state
                .requests
                .lock()
                .expect("requests lock")
                .push(request);
            let round = self
                .state
                .rounds
                .lock()
                .expect("rounds lock")
                .pop_front()
                .expect("scripted round");
            Box::pin(async move {
                match round {
                    Round::Reply(text) => {
                        let _ = events.send(ChatEvent::Delta(text));
                        Ok(receipt())
                    }
                    Round::Calls(calls) => {
                        for (index, (id, name, args)) in calls.into_iter().enumerate() {
                            let _ = events.send(ChatEvent::ToolCallDelta {
                                index,
                                id: Some(id),
                                name: Some(name),
                                arguments: args,
                            });
                        }
                        Ok(receipt())
                    }
                    Round::Paused {
                        started,
                        resume,
                        answer,
                    } => {
                        let _ = started.send(());
                        let _ = resume.await;
                        let _ = events.send(ChatEvent::Delta(answer));
                        Ok(receipt())
                    }
                }
            })
        }
    }

    impl SubagentBackend for MockBackend {
        fn chat(&self) -> Result<Box<dyn ChatStream>, String> {
            Ok(Box::new(MockChat {
                state: Arc::clone(&self.state),
            }))
        }

        fn tools(&self, policy: SubagentPolicy) -> Result<Box<dyn ToolRunner>, String> {
            let ctx = router::ToolContext::new(
                self.root.clone(),
                router::AgentToolSettings {
                    auto_approve_commands: Vec::new(),
                    ..router::AgentToolSettings::default()
                },
            )
            .with_policy(policy);
            Ok(Box::new(turn::RouterToolRunner::new(ctx)))
        }
    }

    fn receipt() -> ChatReceipt {
        ChatReceipt {
            request_id: Uuid::new_v4(),
            model: "test-model".into(),
            prompt_tokens: 10,
            completion_tokens: 5,
            cost_usd: 0.0,
            duration_ms: 1,
            estimated: false,
        }
    }

    fn model() -> Model {
        Model {
            id: "test-model".into(),
            display_name: "Test".into(),
            context_window: 200_000,
            price_per_1k_input: 0.0,
            price_per_1k_output: 0.0,
            supports_streaming: true,
            supports_tools: true,
            family: "test".into(),
        }
    }

    fn caps() -> TurnCaps {
        TurnCaps::spec_defaults(200_000)
    }

    fn answer(text: &str) -> QuestionAnswer {
        QuestionAnswer {
            answers: vec![QuestionResponse {
                question: "Which file?".into(),
                answer: text.into(),
            }],
        }
    }

    fn runtime(
        backend: Arc<MockBackend>,
        handle: tokio::runtime::Handle,
    ) -> (SubagentRuntime, mpsc::UnboundedReceiver<SubagentEvent>) {
        let (events, events_rx) = mpsc::unbounded_channel();
        (
            SubagentRuntime::new(backend, model(), caps(), handle, events)
                .with_parent("parent-turn"),
            events_rx,
        )
    }

    #[tokio::test]
    async fn background_child_receives_fifo_steering_and_completes_once() {
        let root = temp_root("subagent-steering");
        let (started_tx, started_rx) = oneshot::channel();
        let (resume_tx, resume_rx) = oneshot::channel();
        let backend = MockBackend::new(
            root,
            vec![
                Round::Paused {
                    started: started_tx,
                    resume: resume_rx,
                    answer: "first".into(),
                },
                Round::Reply("second".into()),
            ],
        );
        let (runtime, mut events) = runtime(backend.clone(), tokio::runtime::Handle::current());
        let started = runtime
            .spawn(SubagentRequest::new(
                SubagentPolicy::ReadOnly,
                "inspect the diff",
                true,
            ))
            .await
            .expect("background spawn");
        started_rx.await.expect("first turn started");
        runtime
            .send_message(&started.id, "now check the tests")
            .expect("steering queued");
        resume_tx.send(()).expect("finish first round");
        let completion = runtime
            .wait(&started.id, Some(Duration::from_secs(2)))
            .await
            .expect("steered child finishes");
        assert_eq!(completion.final_message, "second");
        assert_eq!(completion.policy, SubagentPolicy::ReadOnly);
        let requests = backend.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].messages.last().expect("steering user").content,
            "now check the tests"
        );
        // One completion event, not one per turn.
        let mut completed = 0;
        while let Ok(event) = events.try_recv() {
            if matches!(event, SubagentEvent::Completed(_)) {
                completed += 1;
            }
        }
        assert_eq!(completed, 1);
        assert_eq!(
            runtime.info(&started.id).expect("child roster").status,
            SubagentStatus::Done
        );
    }

    #[tokio::test]
    async fn two_children_group_under_one_parent_and_deliver_once_each() {
        let root = temp_root("subagent-two");
        let backend = MockBackend::new(
            root,
            vec![Round::Reply("left".into()), Round::Reply("right".into())],
        );
        let (runtime, mut events) = runtime(backend, tokio::runtime::Handle::current());
        let left = runtime
            .spawn(SubagentRequest::new(
                SubagentPolicy::ReadOnly,
                "review left",
                true,
            ))
            .await
            .expect("left spawn");
        let right = runtime
            .spawn(SubagentRequest::new(
                SubagentPolicy::ReadOnly,
                "review right",
                true,
            ))
            .await
            .expect("right spawn");
        let all = runtime
            .wait_all(Some(Duration::from_secs(2)))
            .await
            .expect("both children finish");
        assert_eq!(all.len(), 2);
        assert_ne!(left.id, right.id);

        let mut started = Vec::new();
        let mut completed = Vec::new();
        while let Ok(event) = events.try_recv() {
            match event {
                SubagentEvent::Started { id, parent_id, .. } => {
                    assert_eq!(parent_id.as_deref(), Some("parent-turn"));
                    started.push(id);
                }
                SubagentEvent::Completed(completion) => completed.push(completion.id),
                _ => {}
            }
        }
        started.sort();
        completed.sort();
        let mut expected = vec![left.id.clone(), right.id.clone()];
        expected.sort();
        assert_eq!(started, expected);
        assert_eq!(completed, expected);
    }

    #[tokio::test]
    async fn wait_returns_and_cancel_kills_a_running_child() {
        let root = temp_root("subagent-cancel");
        let (started_tx, started_rx) = oneshot::channel();
        let (_resume_tx, resume_rx) = oneshot::channel();
        let backend = MockBackend::new(
            root,
            vec![Round::Paused {
                started: started_tx,
                resume: resume_rx,
                answer: "never".into(),
            }],
        );
        let (runtime, _events) = runtime(backend, tokio::runtime::Handle::current());
        let child = runtime
            .spawn(SubagentRequest::new(
                SubagentPolicy::Full,
                "wait forever",
                true,
            ))
            .await
            .expect("spawn");
        started_rx.await.expect("turn started");
        runtime.cancel(&child.id).expect("cancel");
        let completion = runtime
            .wait(&child.id, Some(Duration::from_secs(1)))
            .await
            .expect("cancel finishes promptly");
        assert!(completion.cancelled);
        assert_eq!(completion.end, TurnEnd::StoppedByUser);
    }

    async fn ask_turn(
        backend: Arc<MockBackend>,
        decisions: mpsc::UnboundedReceiver<TurnDecision>,
        events: mpsc::UnboundedSender<AgentEvent>,
    ) -> AgentTurnReport {
        // The question card is a parent-session tool (manifest v3). Use the
        // ordinary unrestricted Agent Mode context, not a child policy.
        let ctx =
            router::ToolContext::new(backend.root.clone(), router::AgentToolSettings::default());
        let input = TurnInput::new(
            model(),
            vec![
                ChatMessage::system("ask"),
                ChatMessage::user("ask the user"),
            ],
        )
        .with_caps(caps());
        turn::run_agent_turn(
            input,
            backend.chat().expect("chat"),
            turn::RouterToolRunner::new(ctx),
            events,
            decisions,
        )
        .await
    }

    #[tokio::test]
    async fn question_answer_resumes_and_stale_id_is_inert() {
        let root = temp_root("subagent-question");
        let backend = MockBackend::new(
            root,
            vec![
                Round::Calls(vec![(
                    "q1".into(),
                    "ask_user_question".into(),
                    serde_json::json!({
                        "questions": [{
                            "question": "Which file?",
                            "options": [
                                {"label": "src/a.rs", "description": "left"},
                                {"label": "src/b.rs", "description": "right"}
                            ],
                            "allow_free_text": true
                        }]
                    })
                    .to_string(),
                )]),
                Round::Reply("using src/b.rs".into()),
            ],
        );
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (decision_tx, decision_rx) = mpsc::unbounded_channel();
        let mut task = tokio::spawn(ask_turn(backend.clone(), decision_rx, event_tx));
        let approval_id = loop {
            if let AgentEvent::ApprovalNeeded { approval_id, card } =
                event_rx.recv().await.expect("event")
            {
                assert!(matches!(card, ApprovalCard::Question(_)));
                break approval_id;
            }
        };
        // A stale answer with a different id must not unpark the turn.
        decision_tx
            .send((
                Uuid::new_v4(),
                AgentDecision::Answer(Box::new(answer("src/a.rs"))),
            ))
            .expect("stale answer");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut task)
                .await
                .is_err(),
            "a stale answer cannot resume the turn"
        );
        decision_tx
            .send((
                approval_id,
                AgentDecision::Answer(Box::new(answer("src/b.rs"))),
            ))
            .expect("answer");
        let report = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("answer resumes")
            .expect("turn joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.final_message, "using src/b.rs");
        let requests = backend.requests();
        let tool_result = requests[1]
            .messages
            .iter()
            .find(|message| message.role == Role::Tool)
            .expect("answer tool result");
        assert!(tool_result.content.contains("Which file?"));
        assert!(tool_result.content.contains("src/b.rs"));
    }

    #[test]
    fn blocking_adapter_runs_the_shared_turn_for_a_synchronous_host() {
        let root = temp_root("subagent-blocking");
        let backend = MockBackend::new(root, vec![Round::Reply("sync".into())]);
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (_decision_tx, decision_rx) = mpsc::unbounded_channel();
        let report = run_subagent_turn_blocking(
            SubagentPolicy::ReadOnly,
            model(),
            caps(),
            vec![ChatMessage::system("sync"), ChatMessage::user("go")],
            backend.chat().expect("chat"),
            backend.tools(SubagentPolicy::ReadOnly).expect("tools"),
            event_tx,
            decision_rx,
        )
        .expect("blocking adapter");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.final_message, "sync");
    }

    #[tokio::test]
    async fn read_only_policy_has_no_writes_on_either_router_path() {
        let root = temp_root("subagent-readonly");
        let readonly_manifest = SubagentPolicy::ReadOnly.manifest();
        for denied in [
            "write_file",
            "edit_file",
            "delete_file",
            "bash",
            "spawn_subagent",
        ] {
            assert!(
                !readonly_manifest.iter().any(|tool| tool.name == denied),
                "`{denied}` must be absent from the read-only manifest"
            );
        }
        let full_manifest = SubagentPolicy::Full.manifest();
        for present in ["read_file", "write_file", "bash"] {
            assert!(
                full_manifest.iter().any(|tool| tool.name == present),
                "`{present}` must be present in the full manifest"
            );
        }
        assert!(!full_manifest
            .iter()
            .any(|tool| tool.name == "spawn_subagent"));
        let readonly = router::ToolContext::new(&root, router::AgentToolSettings::default())
            .with_policy(SubagentPolicy::ReadOnly);
        let write = crate::chat_types::ToolCall {
            id: "w1".into(),
            name: "write_file".into(),
            arguments: serde_json::json!({"path": "x.txt", "content": "x"}).to_string(),
        };
        let first = router::execute_tool(&readonly, write.clone()).await;
        assert!(matches!(
            first,
            ToolOutcome::Ready(result) if !result.ok && result.content.contains("read-only")
        ));
        let second = router::execute_approved(&readonly, write).await;
        assert!(!second.ok);
        assert!(second.content.contains("read-only"));
        assert!(!root.join("x.txt").exists());

        let full = router::ToolContext::new(&root, router::AgentToolSettings::default());
        let card = router::execute_tool(
            &full,
            crate::chat_types::ToolCall {
                id: "w2".into(),
                name: "write_file".into(),
                arguments: serde_json::json!({"path": "x.txt", "content": "x"}).to_string(),
            },
        )
        .await;
        assert!(matches!(
            card,
            ToolOutcome::NeedsApproval(ApprovalCard::Diff { .. })
        ));
        let bash = router::execute_tool(
            &full,
            crate::chat_types::ToolCall {
                id: "b1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({"command": "Set-Content evil.txt evil"}).to_string(),
            },
        )
        .await;
        assert!(matches!(
            bash,
            ToolOutcome::NeedsApproval(ApprovalCard::Command { .. })
        ));
    }
}
