//! Subagent turn adapter (ARCHITECTURE.md §19.3, ROADMAP Phase 9c).
//!
//! Phase 9e's [`SubagentRuntime`] owns run state, completion and `wait`; this
//! module owns *how a child turn runs*. There is no second loop: a child is
//! the ordinary §17 loop with a policy-scoped manifest armed, and a child
//! never inherits the parent's `ApproveAllThisTurn` or session auto-approve
//! grant.
//!
//! Two consumers share the adapter:
//!
//! * the interactive `spawn_subagent` tool, through [`spawn_child_turn`],
//!   which drives background children and relays their cards on the parent's
//!   normal approval rail;
//! * Phase 8's workflow host, which runs [`run_subagent_turn`] directly under
//!   its own scheduler.
//!
//! The backend seam is deliberately small: one fresh chat stream and one
//! policy-scoped tool runner per turn, so no mutable turn state is shared
//! between children.

use std::sync::Arc;

use tokio::sync::mpsc;

use super::subagent::{
    ChildOutcome, SpawnError, SpawnRequest, SubagentCard, SubagentHandle, SubagentPolicy,
    SubagentRuntime,
};
use super::turn::{
    run_agent_turn, AgentEvent, AgentTurnReport, ChatRoundFuture, ToolApprovedFuture,
    ToolRunFuture, TurnDecision,
};
use super::{ChatStream, ToolRunner, TurnCaps, TurnInput};
use crate::chat_types::{ToolCall, ToolResult};
use crate::types::{ChatEvent, ChatMessage, ChatRequest, Model};

/// The seam between the runtime and the app/desktop shell. A backend creates
/// one fresh chat stream and one policy-scoped tool runner per turn; it must
/// not share mutable turn state between children.
pub trait SubagentBackend: Send + Sync {
    /// A chat stream for a child turn.
    fn chat(&self) -> Result<Box<dyn ChatStream>, String>;

    /// A router runner whose tool context carries `policy`. The backend must
    /// set the policy on both router paths.
    fn tools(&self, policy: SubagentPolicy) -> Result<Box<dyn ToolRunner>, String>;
}

/// A policy-scoped, single-conversation child. Phase 8's host uses this
/// directly under its own scheduler. It is intentionally the ordinary §17
/// loop.
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
        let report = run_agent_turn(input, chat, tools, events, decisions).await;
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
    run_agent_turn(input, chat, tools, events, decisions).await
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

/// Spawn one child through the session runtime, driving its turns on the
/// tokio runtime. Cards park on the parent's normal rail
/// ([`super::subagent::SubagentEvent::ApprovalNeeded`]) and are answered
/// through [`SubagentRuntime::decide`]; steering messages queue FIFO between
/// turns.
pub fn spawn_child_turn(
    runtime: &SubagentRuntime,
    request: SpawnRequest,
    backend: Arc<dyn SubagentBackend>,
    model: Model,
    caps: TurnCaps,
) -> Result<SubagentHandle, SpawnError> {
    let child_id = request.child_id.clone();
    let parent_session = request.parent_session.clone();
    let policy = request.policy;
    let name = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("subagent-{}", short_id(&child_id)));
    let conversation = seed_conversation(&request, &name);
    let runtime_for_task = runtime.clone();
    runtime.spawn_background(request, async move {
        let (steer_tx, mut steer_rx) = mpsc::unbounded_channel::<String>();
        runtime_for_task.register_steering(&child_id, steer_tx);
        let mut conversation = conversation;
        loop {
            let (chat, tools) = match (backend.chat(), backend.tools(policy)) {
                (Ok(chat), Ok(tools)) => (chat, tools),
                (Err(message), _) | (_, Err(message)) => {
                    runtime_for_task.forget_steering(&child_id);
                    return ChildOutcome::failed(message);
                }
            };
            let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();
            let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
            let turn = run_subagent_turn(
                policy,
                model.clone(),
                caps.clone(),
                conversation.clone(),
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
                    event = event_rx.recv() => {
                        if let Some(event) = event {
                            relay_event(
                                &runtime_for_task,
                                &child_id,
                                &parent_session,
                                &decision_tx,
                                event,
                            );
                        }
                    }
                }
            };
            while let Ok(event) = event_rx.try_recv() {
                relay_event(
                    &runtime_for_task,
                    &child_id,
                    &parent_session,
                    &decision_tx,
                    event,
                );
            }
            runtime_for_task.forget_child_cards(&child_id);
            conversation = report.conversation;
            let end = report.end;
            if !end.is_completed() {
                runtime_for_task.forget_steering(&child_id);
                return ChildOutcome::failed(end.toast());
            }
            match steer_rx.try_recv() {
                Ok(text) => conversation.push(ChatMessage::user(text)),
                Err(_) => {
                    runtime_for_task.forget_steering(&child_id);
                    let final_message = conversation
                        .iter()
                        .rev()
                        .find(|message| message.role == crate::types::Role::Assistant)
                        .map(|message| message.content.clone())
                        .unwrap_or_default();
                    return ChildOutcome::success(final_message);
                }
            }
        }
    })
}

fn relay_event(
    runtime: &SubagentRuntime,
    child_id: &str,
    parent_session: &str,
    decisions: &mpsc::UnboundedSender<TurnDecision>,
    event: AgentEvent,
) {
    if let AgentEvent::ApprovalNeeded { approval_id, card } = event {
        runtime.register_decision(child_id, approval_id, decisions.clone());
        runtime.emit_child_card(SubagentCard {
            child_id: child_id.to_owned(),
            parent_session: parent_session.to_owned(),
            approval_id,
            card,
        });
    }
}

/// The child's initial conversation: policy system prompt, optional persona,
/// optional seed history, then the mission as the first user turn.
fn seed_conversation(request: &SpawnRequest, name: &str) -> Vec<ChatMessage> {
    let mut conversation = request.conversation.clone().unwrap_or_default();
    if conversation
        .first()
        .is_none_or(|message| message.role != crate::types::Role::System)
    {
        conversation.insert(
            0,
            ChatMessage::system(child_system_prompt(
                name,
                request.policy,
                request.persona.as_deref(),
            )),
        );
    }
    conversation.push(ChatMessage::user(request.mission.clone()));
    conversation
}

fn short_id(child_id: &str) -> &str {
    child_id.get(..8).unwrap_or(child_id)
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

/// Render a child's completion as the parent model's tool result. Only facts
/// the runtime actually observed appear; a background start is never reported
/// as a completed mission.
pub fn format_subagent_completion(completion: &super::subagent::SubagentCompletion) -> String {
    let status = if completion.ok {
        "completed"
    } else {
        "ended without completing"
    };
    let detail = completion
        .error
        .as_deref()
        .map(|error| format!("\nReason: {error}"))
        .unwrap_or_default();
    format!(
        "subagent `{}` (policy `{}`) {status}{detail}.\n\n{}",
        completion.child_id,
        completion.policy.as_str(),
        completion.final_message
    )
}

/// The tool result for a question card's answer. The model consumes this
/// verbatim; the answer is never reinterpreted as an approval.
pub fn question_answer_result(call: &ToolCall, answer: &str) -> ToolResult {
    ToolResult::success(&call.id, format!("User answer:\n{answer}"))
}
