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
    ChildOutcome, SpawnError, SpawnRequest, SubagentActivity, SubagentActivityKind, SubagentCard,
    SubagentHandle, SubagentPolicy, SubagentRunState, SubagentRuntime,
};
use super::turn::{
    run_agent_turn, AgentEvent, AgentTurnReport, AgentTurnState, ChatRoundFuture,
    ToolApprovedFuture, ToolRunFuture, TurnDecision,
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
    let name = normalize_name(&request);
    let conversation = seed_conversation(&request, &name);
    let started = SubagentActivity::new(
        child_id.clone(),
        parent_session.clone(),
        SubagentActivityKind::Started {
            name: name.clone(),
            policy,
            mission: request.mission.clone(),
        },
    );
    let runtime_for_task = runtime.clone();
    runtime.spawn_background(request, async move {
        // One Started per spawned child, before the first gateway/tool event.
        runtime_for_task.emit_activity(started);
        let (steer_tx, mut steer_rx) = mpsc::unbounded_channel::<String>();
        runtime_for_task.register_steering(&child_id, steer_tx);
        let mut conversation = conversation;
        let mut transcript = ChildActivity::default();
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
                                &mut transcript,
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
                    &mut transcript,
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

/// Per-child live transcript state. A new `Thinking` round mints a new
/// assistant message; the delta sequence stays monotonic across turns.
#[derive(Default)]
struct ChildActivity {
    message_id: Option<String>,
    sequence: u64,
}

fn relay_event(
    runtime: &SubagentRuntime,
    child_id: &str,
    parent_session: &str,
    decisions: &mpsc::UnboundedSender<TurnDecision>,
    event: AgentEvent,
    transcript: &mut ChildActivity,
) {
    let activity = match event {
        AgentEvent::State(state) => {
            if state == AgentTurnState::Thinking {
                transcript.message_id = None;
            }
            let (state, tool_name) = activity_state(&state);
            SubagentActivityKind::StateChanged { state, tool_name }
        }
        AgentEvent::Delta(text) => {
            transcript.sequence += 1;
            let message_id = transcript
                .message_id
                .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
                .clone();
            SubagentActivityKind::Delta {
                message_id,
                sequence: transcript.sequence,
                text,
            }
        }
        AgentEvent::ToolCallStarted(call) => SubagentActivityKind::ToolCallStarted {
            call_id: call.id,
            tool_name: call.name,
            args_json: call.arguments,
        },
        AgentEvent::ToolResult(result) => SubagentActivityKind::ToolResult {
            call_id: result.id,
            result_summary: result.content,
            is_err: !result.ok,
        },
        AgentEvent::ApprovalNeeded { approval_id, card } => {
            runtime.register_decision(child_id, approval_id, decisions.clone());
            runtime.emit_child_card(SubagentCard {
                child_id: child_id.to_owned(),
                parent_session: parent_session.to_owned(),
                approval_id,
                card,
            });
            return;
        }
        _ => return,
    };
    runtime.emit_activity(SubagentActivity::new(
        child_id.to_owned(),
        parent_session.to_owned(),
        activity,
    ));
}

fn activity_state(state: &AgentTurnState) -> (SubagentRunState, Option<String>) {
    match state {
        AgentTurnState::Thinking => (SubagentRunState::Thinking, None),
        AgentTurnState::RunningTool(name) => (SubagentRunState::RunningTool, Some(name.clone())),
        AgentTurnState::AwaitingApproval(_) => (SubagentRunState::AwaitingApproval, None),
        AgentTurnState::Done => (SubagentRunState::Done, None),
    }
}

fn normalize_name(request: &SpawnRequest) -> String {
    if let Some(name) = request.name.as_deref() {
        let name = plain_line(name);
        if !name.is_empty() {
            return truncate_ellipsis(&name, 80);
        }
    }
    if let Some(line) = request.mission.lines().find(|line| !line.trim().is_empty()) {
        let line = plain_line(line);
        if !line.is_empty() {
            return truncate_ellipsis(&line, 48);
        }
    }
    format!("subagent-{}", short_id(&request.child_id))
}

/// Trim, collapse whitespace, and drop control characters so the name is one
/// readable line in the child transcript/pet UI.
fn plain_line(text: &str) -> String {
    text.split_whitespace()
        .map(|part| {
            part.chars()
                .filter(|ch| {
                    !ch.is_control()
                        && !matches!(
                            *ch,
                            '\u{200B}'..='\u{200F}'
                                | '\u{202A}'..='\u{202E}'
                                | '\u{2060}'..='\u{206F}'
                                | '\u{FEFF}'
                        )
                })
                .collect::<String>()
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_ellipsis(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::router::ToolOutcome;
    use selora_ai_types_test_support::receipt;

    struct ScriptedChat {
        deltas: Vec<&'static str>,
    }

    impl Default for ScriptedChat {
        fn default() -> Self {
            Self {
                deltas: vec!["hello"],
            }
        }
    }

    impl ChatStream for ScriptedChat {
        fn stream_round(
            &mut self,
            _request: ChatRequest,
            events: mpsc::UnboundedSender<ChatEvent>,
        ) -> ChatRoundFuture<'_> {
            let deltas = self.deltas.clone();
            Box::pin(async move {
                for delta in deltas {
                    let _ = events.send(ChatEvent::Delta(delta.to_owned()));
                }
                let receipt = receipt();
                let _ = events.send(ChatEvent::Done(receipt.clone()));
                Ok(receipt)
            })
        }
    }

    #[derive(Default)]
    struct HangingChat;

    impl ChatStream for HangingChat {
        fn stream_round(
            &mut self,
            _request: ChatRequest,
            _events: mpsc::UnboundedSender<ChatEvent>,
        ) -> ChatRoundFuture<'_> {
            Box::pin(async {
                std::future::pending::<Result<crate::types::ChatReceipt, selora_core::SeloraError>>(
                )
                .await
            })
        }
    }

    struct UnusedTools;

    impl ToolRunner for UnusedTools {
        fn run(&mut self, call: ToolCall) -> ToolRunFuture {
            Box::pin(async move {
                ToolOutcome::Ready(ToolResult {
                    id: call.id,
                    ok: false,
                    content: "unused".into(),
                })
            })
        }

        fn run_approved(&mut self, call: ToolCall) -> ToolApprovedFuture {
            Box::pin(async move {
                ToolResult {
                    id: call.id,
                    ok: false,
                    content: "unused".into(),
                }
            })
        }

        fn abort(&mut self) {}
    }

    struct Backend<C> {
        chat: std::marker::PhantomData<fn() -> C>,
    }

    impl<C: ChatStream + Default + 'static> SubagentBackend for Backend<C> {
        fn chat(&self) -> Result<Box<dyn ChatStream>, String> {
            Ok(Box::new(C::default()))
        }

        fn tools(&self, _policy: SubagentPolicy) -> Result<Box<dyn ToolRunner>, String> {
            Ok(Box::new(UnusedTools))
        }
    }

    mod selora_ai_types_test_support {
        use crate::types::{ChatReceipt, ModelId};
        use uuid::Uuid;

        pub fn receipt() -> ChatReceipt {
            ChatReceipt {
                request_id: Uuid::nil(),
                model: ModelId::from("test-model"),
                prompt_tokens: 1,
                completion_tokens: 1,
                cost_usd: 0.0,
                duration_ms: 1,
                estimated: false,
            }
        }
    }

    fn model() -> Model {
        serde_json::from_value(serde_json::json!({
            "id": "test-model",
            "supports_tools": true,
            "context_window": 128000
        }))
        .unwrap()
    }

    fn activities(events: &[super::super::subagent::SubagentEvent]) -> Vec<SubagentActivity> {
        events
            .iter()
            .filter_map(|event| match event {
                super::super::subagent::SubagentEvent::Activity(activity) => Some(activity.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn started_precedes_child_delta_and_completion_with_ids() {
        let runtime = SubagentRuntime::new();
        let mut events = runtime.subscribe();
        spawn_child_turn(
            &runtime,
            SpawnRequest::new(
                "parent-1",
                "child-1",
                SubagentPolicy::ReadOnly,
                "Review parser",
            ),
            Arc::new(Backend::<ScriptedChat> {
                chat: std::marker::PhantomData,
            }),
            model(),
            TurnCaps::spec_defaults(128000),
        )
        .unwrap();
        let completion = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            runtime.wait_for_child("child-1"),
        )
        .await
        .expect("completion")
        .expect("child");
        assert!(completion.ok, "{completion:?}");
        let mut seen = Vec::new();
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                .await
                .expect("event")
                .expect("stream open");
            let done = matches!(event, super::super::subagent::SubagentEvent::Completed(_));
            seen.push(event);
            if done {
                break;
            }
        }
        let activities = activities(&seen);
        assert!(matches!(
            activities.first().map(|activity| &activity.kind),
            Some(SubagentActivityKind::Started { name, mission, .. })
                if name == "Review parser" && mission == "Review parser"
        ));
        let delta = activities
            .iter()
            .find(|activity| matches!(&activity.kind, SubagentActivityKind::Delta { .. }))
            .expect("delta activity");
        let SubagentActivityKind::Delta { text, sequence, .. } = &delta.kind else {
            unreachable!()
        };
        assert_eq!(text, "hello");
        assert_eq!(*sequence, 1);
        assert!(activities.iter().all(|activity| {
            activity.child_id == "child-1" && activity.parent_session == "parent-1"
        }));
    }

    #[tokio::test]
    async fn cancellation_emits_one_lifecycle_event_and_no_completion() {
        let runtime = SubagentRuntime::new();
        let mut events = runtime.subscribe();
        spawn_child_turn(
            &runtime,
            SpawnRequest::new(
                "parent-1",
                "child-1",
                SubagentPolicy::ReadOnly,
                "Wait forever",
            ),
            Arc::new(Backend::<HangingChat> {
                chat: std::marker::PhantomData,
            }),
            model(),
            TurnCaps::spec_defaults(128000),
        )
        .unwrap();
        // Wait for Started so cancellation cannot race task startup.
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                .await
                .expect("started")
                .expect("stream open");
            if matches!(
                event,
                super::super::subagent::SubagentEvent::Activity(SubagentActivity {
                    kind: SubagentActivityKind::Started { .. },
                    ..
                })
            ) {
                break;
            }
        }
        assert!(runtime.cancel_child("child-1"));
        assert_eq!(
            runtime.child_state("child-1"),
            Some(super::super::subagent::ChildState::Cancelled)
        );
        let mut cancellations = 0;
        loop {
            match tokio::time::timeout(std::time::Duration::from_millis(150), events.recv()).await {
                Ok(Ok(super::super::subagent::SubagentEvent::Cancelled {
                    child_id,
                    parent_session,
                })) => {
                    assert_eq!(child_id, "child-1");
                    assert_eq!(parent_session, "parent-1");
                    cancellations += 1;
                }
                Ok(Ok(super::super::subagent::SubagentEvent::Completed(_))) => {
                    panic!("cancelled child must not complete")
                }
                Ok(Ok(_)) => continue,
                _ => break,
            }
        }
        assert_eq!(cancellations, 1);
    }

    #[test]
    fn relay_maps_tool_and_state_activities_with_stable_message_ids() {
        let runtime = SubagentRuntime::new();
        let mut events = runtime.subscribe();
        let (decisions, _decision_rx) = mpsc::unbounded_channel();
        let mut transcript = ChildActivity::default();
        for event in [
            AgentEvent::State(AgentTurnState::Thinking),
            AgentEvent::Delta("a".into()),
            AgentEvent::Delta("b".into()),
            AgentEvent::ToolCallStarted(ToolCall {
                id: "call-1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"src/lib.rs"}"#.into(),
            }),
            AgentEvent::ToolResult(ToolResult {
                id: "call-1".into(),
                ok: false,
                content: "denied".into(),
            }),
            AgentEvent::State(AgentTurnState::RunningTool("read_file".into())),
            AgentEvent::State(AgentTurnState::Done),
            AgentEvent::State(AgentTurnState::Thinking),
            AgentEvent::Delta("c".into()),
        ] {
            relay_event(
                &runtime,
                "child-1",
                "parent-1",
                &decisions,
                event,
                &mut transcript,
            );
        }
        let activities = activities(&[
            events.try_recv().unwrap(),
            events.try_recv().unwrap(),
            events.try_recv().unwrap(),
            events.try_recv().unwrap(),
            events.try_recv().unwrap(),
            events.try_recv().unwrap(),
            events.try_recv().unwrap(),
            events.try_recv().unwrap(),
            events.try_recv().unwrap(),
        ]);
        let deltas: Vec<_> = activities
            .iter()
            .filter_map(|activity| match &activity.kind {
                SubagentActivityKind::Delta {
                    message_id,
                    sequence,
                    ..
                } => Some((message_id.clone(), *sequence)),
                _ => None,
            })
            .collect();
        assert_eq!(deltas[0].0, deltas[1].0);
        assert_eq!(deltas[0].1, 1);
        assert_eq!(deltas[1].1, 2);
        assert_ne!(deltas[2].0, deltas[0].0, "new Thinking round new message");
        assert_eq!(deltas[2].1, 3);
        assert!(activities.iter().any(|activity| matches!(
            &activity.kind,
            SubagentActivityKind::ToolCallStarted {
                call_id,
                tool_name,
                args_json,
            } if call_id == "call-1"
                && tool_name == "read_file"
                && args_json.contains("src/lib.rs")
        )));
        assert!(activities.iter().any(|activity| matches!(
            &activity.kind,
            SubagentActivityKind::ToolResult {
                call_id,
                result_summary,
                is_err: true,
            } if call_id == "call-1" && result_summary == "denied"
        )));
        assert!(activities.iter().any(|activity| matches!(
            &activity.kind,
            SubagentActivityKind::StateChanged {
                state: SubagentRunState::RunningTool,
                tool_name: Some(name),
            } if name == "read_file"
        )));
        assert!(activities.iter().any(|activity| matches!(
            &activity.kind,
            SubagentActivityKind::StateChanged {
                state: SubagentRunState::Done,
                tool_name: None,
            }
        )));
    }

    #[test]
    fn display_name_drops_bidi_formatting_and_controls() {
        let mut request =
            SpawnRequest::new("parent-1", "child-1", SubagentPolicy::ReadOnly, "mission");
        request.name = Some("Safe \u{202E}evil\u{200F} name\u{0007}".into());
        assert_eq!(normalize_name(&request), "Safe evil name");
    }
}
