//! Real workflow asks. The scheduler calls this executor only after acquiring
//! the named agent's FIFO slot and the workflow-wide concurrency permit.
//!
//! Keep `ChatMessage` history here, not in the scheduler's plain-text history:
//! dropping tool IDs, arguments or results would corrupt the next HTTP request.

use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
    time::{Duration, Instant},
};

use selora_ai::{
    run_agent_turn, tool_manifest, AgentDecision, AgentEvent, AgentToolSettings, AgentTurnReport,
    AgentTurnState, BashAbortHandle, ChatEvent, ChatMessage, ChatReceipt, ChatRequest, ChatStream,
    ClientChatStream, Model, Role, RouterToolRunner, SeloraClient, ToolCall, ToolContext,
    ToolOutcome, ToolResult, ToolRunner, ToolSpec, TurnCaps, TurnDecision, TurnInput,
};
use selora_workflow::{
    subagent::{AskError, AskExecutor, AskReply, AskRequest},
    HostError,
};
use tokio::sync::{mpsc, oneshot, Notify};
use uuid::Uuid;

use super::WorkflowHost;

#[cfg(test)]
pub(super) type StreamFactory = std::sync::Arc<dyn Fn() -> Box<dyn ChatStream> + Send + Sync>;

#[derive(Default)]
pub(super) struct LiveTurns {
    turns: BTreeMap<Uuid, LiveTurn>,
}

struct LiveTurn {
    decisions: mpsc::UnboundedSender<TurnDecision>,
    pending: Option<Uuid>,
    deadline: Option<Instant>,
}

impl LiveTurns {
    pub(super) fn cancel(&mut self) {
        for turn in self.turns.values_mut() {
            turn.pending = None;
            let _ = turn.decisions.send((Uuid::nil(), AgentDecision::Stop));
        }
    }

    pub(super) fn decide(
        &mut self,
        approval_id: Uuid,
        decision: AgentDecision,
    ) -> Result<(), HostError> {
        let turn = self
            .turns
            .values_mut()
            .find(|turn| turn.pending == Some(approval_id))
            .ok_or_else(|| {
                HostError::new("no matching live workflow approval (stale or already used)")
            })?;
        // Consume before sending: even a racing duplicate cannot widen a grant.
        turn.pending = None;
        if turn.decisions.is_closed()
            || turn
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(HostError::new("the workflow approval is no longer live"));
        }
        turn.decisions
            .send((approval_id, decision))
            .map_err(|_| HostError::new("the workflow approval is no longer live"))
    }
}

pub(super) struct WorkflowAskExecutor {
    host: Weak<WorkflowHost>,
}

impl WorkflowAskExecutor {
    pub(super) fn new(host: Weak<WorkflowHost>) -> Self {
        Self { host }
    }
}

impl AskExecutor for WorkflowAskExecutor {
    type Caps = TurnCaps;

    fn ask(&self, request: AskRequest<TurnCaps>) -> Result<AskReply, AskError> {
        let host = self
            .host
            .upgrade()
            .ok_or_else(|| AskError::new("workflow host was dropped"))?;
        host.execute_scheduled_ask(request)
            .map_err(|error| AskError::new(host.redact(&error.message)))
    }
}

/// Closed policy on BOTH router entry points, not merely the advertised tools.
/// Unknown future tools are disabled until explicitly included in this policy.
fn allowed_tool(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "list_dir"
            | "fuzzy_find"
            | "search_files"
            | "write_file"
            | "edit_file"
            | "delete_file"
            | "bash"
            | "web_search"
            | "web_fetch"
    )
}

fn manifest() -> Vec<ToolSpec> {
    tool_manifest()
        .into_iter()
        .filter(|tool| allowed_tool(&tool.name))
        .map(|mut tool| {
            if tool.name == "bash" {
                tool.description = "Run a workspace-rooted shell command. Every command requires \
                    explicit user approval; workflow confirmation grants no shell permission."
                    .to_owned();
            }
            tool
        })
        .collect()
}

/// Router filesystem functions are synchronous, even inside their async entry
/// points. Run each operation on a detached worker with its own runtime, so a
/// stalled read cannot stop the turn runtime from polling Stop or its deadline.
/// Do not spawn blocking work on the turn runtime: dropping that runtime would
/// wait for the blocked read and defeat cancellation.
struct WorkflowTools {
    router: RouterToolRunner,
    control: Arc<ToolWorkerControl>,
    #[cfg(test)]
    before_start: Option<Arc<dyn Fn() + Send + Sync>>,
}

struct ToolWorkerControl {
    cancelled: AtomicBool,
    wake: Notify,
    abort: BashAbortHandle,
}

impl ToolWorkerControl {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.abort.kill();
        self.wake.notify_waiters();
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    async fn cancelled(&self) {
        loop {
            let wake = self.wake.notified();
            if self.is_cancelled() {
                return;
            }
            wake.await;
        }
    }
}

/// Dropping a pending tool future must also revoke work not yet started on its
/// worker. The shared bash handle remains sticky for the rest of this turn.
struct ToolJobGuard {
    control: Arc<ToolWorkerControl>,
    armed: bool,
}

impl Drop for ToolJobGuard {
    fn drop(&mut self) {
        if self.armed {
            self.control.cancel();
        }
    }
}

impl WorkflowTools {
    fn new(context: ToolContext) -> Self {
        Self {
            control: Arc::new(ToolWorkerControl {
                cancelled: AtomicBool::new(false),
                wake: Notify::new(),
                abort: context.abort.clone(),
            }),
            router: RouterToolRunner::new(context),
            #[cfg(test)]
            before_start: None,
        }
    }

    fn on_worker<T, F, E>(&self, operation: F, error: E) -> Pin<Box<dyn Future<Output = T> + Send>>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
        E: Fn(String) -> T + Send + 'static,
    {
        let control = self.control.clone();
        #[cfg(test)]
        let before_start = self.before_start.clone();
        Box::pin(async move {
            let mut guard = ToolJobGuard {
                control: control.clone(),
                armed: true,
            };
            let (reply, result) = oneshot::channel();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let worker_control = control.clone();
            let spawned = std::thread::Builder::new()
                .name("selora-workflow-tool".to_owned())
                .spawn(move || {
                    // Signal only after the router future (and with it the bash
                    // child) has really stopped.
                    let _done = DoneSignal(Some(done_tx));
                    #[cfg(test)]
                    if let Some(before_start) = before_start {
                        before_start();
                    }
                    // Check AFTER queueing/runtime setup and immediately before
                    // polling the router. A revoked queued write must never run.
                    if worker_control.is_cancelled() || reply.is_closed() {
                        return;
                    }
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            let _ = reply.send(Err(format!("cannot start tool runtime: {error}")));
                            return;
                        }
                    };
                    // Do not drop a started router future: the bash tool must observe the
                    // abort handle and kill its child. `cancel()` already signalled
                    // it, so this returns promptly; non-preemptible filesystem work
                    // simply finishes without a receiver.
                    let outcome = if worker_control.is_cancelled() {
                        None
                    } else {
                        Some(runtime.block_on(operation))
                    };
                    if let Some(value) = outcome {
                        let _ = reply.send(Ok(value));
                    }
                });
            let outcome = match spawned {
                Err(failure) => Err(format!("cannot start tool worker: {failure}")),
                Ok(_worker) => match tokio::select! {
                    biased;
                    _ = control.cancelled() => None,
                    value = result => Some(value.unwrap_or_else(|_| Err("tool worker stopped without a result".to_owned()))),
                } {
                    Some(value) => value,
                    None => {
                        // A stop must not report "interrupted" before the child
                        // is actually dead. The bash path is bounded at 3 s.
                        let _ = tokio::task::spawn_blocking(move || {
                            let _ = done_rx.recv_timeout(Duration::from_secs(3));
                        })
                        .await;
                        Err("tool execution interrupted by cancellation".to_owned())
                    }
                },
            };
            guard.armed = false;
            outcome.unwrap_or_else(error)
        })
    }
}

struct DoneSignal(Option<std::sync::mpsc::Sender<()>>);

impl Drop for DoneSignal {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

impl ToolRunner for WorkflowTools {
    fn run(&mut self, call: ToolCall) -> Pin<Box<dyn Future<Output = ToolOutcome> + Send>> {
        if !allowed_tool(&call.name) {
            return Box::pin(async move {
                ToolOutcome::Ready(ToolResult::error(
                    call.id,
                    "tool disabled for workflow agents",
                ))
            });
        }
        let id = call.id.clone();
        let operation = self.router.run(call);
        self.on_worker(operation, move |error| {
            ToolOutcome::Ready(ToolResult::error(&id, error))
        })
    }

    fn run_approved(&mut self, call: ToolCall) -> Pin<Box<dyn Future<Output = ToolResult> + Send>> {
        if !allowed_tool(&call.name) {
            return Box::pin(async move {
                ToolResult::error(call.id, "tool disabled for workflow agents")
            });
        }
        let id = call.id.clone();
        let operation = self.router.run_approved(call);
        self.on_worker(operation, move |error| ToolResult::error(&id, error))
    }

    fn abort(&mut self) {
        self.control.cancel();
    }
}

/// Interrupted batches may retain an assistant tool call whose result never
/// reached the loop. Close every such batch before another user/assistant
/// message, preserving all actual results and their original order verbatim.
fn settle_interrupted_calls(report: &mut AgentTurnReport) {
    let interrupted = format!(
        "tool execution interrupted or failed: {}. Completion is unconfirmed; inspect the workspace before retrying.",
        report.end.toast(),
    );
    let mut pending = Vec::<String>::new();
    let mut history = Vec::with_capacity(report.conversation.len());
    for message in std::mem::take(&mut report.conversation) {
        if message.role != Role::Tool {
            for id in pending.drain(..) {
                history.push(ChatMessage::tool_result(id, &interrupted));
            }
        }
        if let Some(id) = &message.tool_call_id {
            pending.retain(|pending_id| pending_id != id);
        }
        if let Some(calls) = &message.tool_calls {
            pending.extend(calls.iter().map(|call| call.id.clone()));
        }
        history.push(message);
    }
    history.extend(
        pending
            .into_iter()
            .map(|id| ChatMessage::tool_result(id, &interrupted)),
    );
    report.conversation = history;
}

enum WorkflowChat {
    Client(ClientChatStream),
    #[cfg(test)]
    Injected(Box<dyn ChatStream>),
}

impl ChatStream for WorkflowChat {
    fn stream_round(
        &mut self,
        request: ChatRequest,
        events: mpsc::UnboundedSender<ChatEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<ChatReceipt, selora_core::SeloraError>> + Send + '_>>
    {
        match self {
            Self::Client(chat) => chat.stream_round(request, events),
            #[cfg(test)]
            Self::Injected(chat) => chat.stream_round(request, events),
        }
    }
}

/// Clear all pending decisions even if a worker or UI callback panics.
struct Registration<'a> {
    host: &'a WorkflowHost,
    turn_id: Uuid,
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.host
            .lock(&self.host.live_turns)
            .turns
            .remove(&self.turn_id);
    }
}

impl WorkflowHost {
    fn execute_scheduled_ask(&self, request: AskRequest<TurnCaps>) -> Result<AskReply, HostError> {
        self.ensure_running()?;
        let messages = {
            let mut conversations = self.lock(&self.conversations);
            let conversation = conversations
                .entry(request.agent.clone())
                .or_insert_with(|| {
                    vec![ChatMessage::system(
                        self.system_prompt(&request.agent, request.persona.as_deref()),
                    )]
                });
            conversation.push(ChatMessage::user(request.instructions));
            conversation.clone()
        };

        if let Some(chat) = &self.config.chat {
            let reply = chat(&self.config.model, &messages)?;
            self.lock(&self.conversations)
                .get_mut(&request.agent)
                .expect("the scheduler owns this agent's conversation")
                .push(ChatMessage::assistant(reply.clone()));
            self.ensure_running()?;
            return Ok(AskReply::new(reply));
        }

        let model = self.config.model_metadata.clone().unwrap_or_else(|| Model {
            id: self.config.model.clone(),
            display_name: self.config.model.clone(),
            context_window: 0,
            price_per_1k_input: 0.0,
            price_per_1k_output: 0.0,
            supports_streaming: true,
            supports_tools: true,
            family: String::new(),
        });
        let input = TurnInput::new(model, messages)
            .with_caps(request.caps)
            .with_manifest(manifest())
            // Never inherit the parent session's full-auto or script grant.
            .with_auto_approve_writes(false);

        // Host is blocking, including when called within a Tokio context. A
        // separate thread avoids nested Runtime::block_on and retains the report
        // on cancellation instead of abandoning a paid turn's receipt.
        let (mut report, approval_error) = std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("selora-workflow-agent".to_owned())
                .spawn_scoped(scope, || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            HostError::new(format!("cannot start ask runtime: {error}"))
                        })?;
                    runtime.block_on(self.run_workflow_turn(input))
                })
                .map_err(|error| HostError::new(format!("cannot start ask worker: {error}")))?
                .join()
                .map_err(|_| HostError::new("workflow agent worker panicked"))?
        })?;

        // Account before ANY failure checks, JSON parsing or engine validation.
        self.add_cost_usd(report.summary.cost_usd);
        let _ = self
            .tokens
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |tokens| {
                Some(tokens.saturating_add(report.summary.tokens))
            });
        settle_interrupted_calls(&mut report);
        self.lock(&self.conversations)
            .insert(request.agent, report.conversation);
        if let Some(error) = approval_error {
            return Err(error);
        }
        self.ensure_running()?;
        if !report.end.is_completed() {
            return Err(HostError::new(report.end.toast()));
        }
        Ok(AskReply::with_usage(
            report.final_message,
            report.summary.tokens,
            report.summary.cost_usd,
        ))
    }

    async fn run_workflow_turn(
        &self,
        input: TurnInput,
    ) -> Result<(AgentTurnReport, Option<HostError>), HostError> {
        let context = ToolContext::new(
            self.root.clone(),
            AgentToolSettings {
                // Even "git status" can be followed by shell syntax. No prefix
                // allowlist is safe as a workflow script's implicit grant.
                auto_approve_commands: Vec::new(),
                ..AgentToolSettings::default()
            },
        );
        let tools = WorkflowTools::new(context);
        let chat = WorkflowChat::Client(ClientChatStream::new(SeloraClient::new(
            &self.config.base_url,
            &self.config.api_key,
        )));
        #[cfg(test)]
        let chat = self
            .config
            .stream_factory
            .as_ref()
            .map_or(chat, |factory| WorkflowChat::Injected(factory()));
        let (decision_tx, decision_rx) = mpsc::unbounded_channel();
        let (event_tx, mut events) = mpsc::unbounded_channel();
        let turn_id = Uuid::new_v4();
        {
            let mut live = self.lock(&self.live_turns);
            self.ensure_running()?;
            live.turns.insert(
                turn_id,
                LiveTurn {
                    decisions: decision_tx.clone(),
                    pending: None,
                    deadline: (input.caps.max_turn_secs != 0).then(|| {
                        Instant::now() + std::time::Duration::from_secs(input.caps.max_turn_secs)
                    }),
                },
            );
        }
        let _registration = Registration {
            host: self,
            turn_id,
        };
        let turn = run_agent_turn(input, chat, tools, event_tx, decision_rx);
        tokio::pin!(turn);
        let mut approval_error = None;
        loop {
            tokio::select! {
                // A finished turn must not publish buffered stale cards.
                biased;
                report = &mut turn => return Ok((report, approval_error)),
                event = events.recv() => match event {
                    Some(AgentEvent::ApprovalNeeded { approval_id, card }) => {
                        let Some(handler) = &self.config.approval_handler else {
                            approval_error = Some(HostError::new(
                                "workflow subagent approval handler is not configured",
                            ));
                            let _ = decision_tx.send((Uuid::nil(), AgentDecision::Stop));
                            continue;
                        };
                        let installed = {
                            let mut live = self.lock(&self.live_turns);
                            if self.is_cancelled() {
                                false
                            } else if let Some(turn) = live.turns.get_mut(&turn_id) {
                                if turn.deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                                    false
                                } else {
                                    turn.pending = Some(approval_id);
                                    true
                                }
                            } else {
                                false
                            }
                        };
                        // Never hold the mutex across a callback: the callback may
                        // synchronously call decide, and must see its pending slot.
                        if installed && std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                            || handler(approval_id, card),
                        )).is_err() {
                            approval_error = Some(HostError::new("workflow approval handler panicked"));
                            if let Some(turn) = self.lock(&self.live_turns).turns.get_mut(&turn_id) {
                                turn.pending = None;
                            }
                            let _ = decision_tx.send((Uuid::nil(), AgentDecision::Stop));
                        }
                    }
                    Some(AgentEvent::State(state)) if !matches!(state, AgentTurnState::AwaitingApproval(_)) => {
                        if let Some(turn) = self.lock(&self.live_turns).turns.get_mut(&turn_id) {
                            turn.pending = None;
                        }
                    }
                    Some(_) => {}
                    None => return Err(HostError::new("workflow agent event channel closed")),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        fs,
        sync::{atomic::AtomicBool, mpsc as sync_mpsc, Arc, Mutex},
        time::Duration,
    };

    use selora_ai::ApprovalCard;
    use selora_workflow::{Host, WorkflowEvent, WorkflowSpec};
    use serde_json::{json, Value};

    use super::super::{WorkflowApprovalHandler, WorkflowHostConfig};

    const WAIT: Duration = Duration::from_secs(5);

    enum Round {
        Reply(Vec<ChatEvent>, ChatReceipt),
        Fail(&'static str),
        Pending(sync_mpsc::Sender<()>, Arc<AtomicBool>),
        Paused(sync_mpsc::Sender<()>, tokio::sync::oneshot::Receiver<()>),
    }

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[derive(Clone)]
    struct Script {
        rounds: Arc<Mutex<VecDeque<Round>>>,
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }

    impl Script {
        fn new(rounds: Vec<Round>) -> Self {
            Self {
                rounds: Arc::new(Mutex::new(rounds.into())),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn config(&self) -> WorkflowHostConfig {
            let script = self.clone();
            let mut config = WorkflowHostConfig::new("http://127.0.0.1:1", "test-key", "ignored")
                .with_model(model())
                .with_turn_caps(TurnCaps {
                    max_turn_secs: 3,
                    ..TurnCaps::spec_defaults(200_000)
                });
            config.stream_factory = Some(Arc::new(move || Box::new(script.clone())));
            config
        }

        fn seen(&self) -> Vec<ChatRequest> {
            self.requests.lock().expect("requests").clone()
        }
    }

    impl ChatStream for Script {
        fn stream_round(
            &mut self,
            request: ChatRequest,
            events: mpsc::UnboundedSender<ChatEvent>,
        ) -> Pin<Box<dyn Future<Output = Result<ChatReceipt, selora_core::SeloraError>> + Send + '_>>
        {
            self.requests.lock().expect("requests").push(request);
            let round = self
                .rounds
                .lock()
                .expect("rounds")
                .pop_front()
                .expect("scripted round");
            Box::pin(async move {
                match round {
                    Round::Reply(deltas, receipt) => {
                        for delta in deltas {
                            events.send(delta).expect("agent receives events");
                        }
                        Ok(receipt)
                    }
                    Round::Fail(message) => Err(selora_core::SeloraError::Ai(message.to_owned())),
                    Round::Pending(started, dropped) => {
                        let _probe = DropProbe(dropped);
                        started.send(()).expect("notify streaming");
                        std::future::pending().await
                    }
                    Round::Paused(started, resume) => {
                        started.send(()).expect("notify paused");
                        resume.await.expect("resume stream");
                        events
                            .send(ChatEvent::Delta("{}".to_owned()))
                            .expect("answer");
                        Ok(receipt())
                    }
                }
            })
        }
    }

    fn model() -> Model {
        Model {
            id: "authoritative-model".to_owned(),
            display_name: "Test model".to_owned(),
            context_window: 200_000,
            price_per_1k_input: 0.01,
            price_per_1k_output: 0.03,
            supports_streaming: true,
            supports_tools: true,
            family: "test".to_owned(),
        }
    }

    fn receipt() -> ChatReceipt {
        ChatReceipt {
            request_id: Uuid::new_v4(),
            model: "authoritative-model".to_owned(),
            prompt_tokens: 10,
            completion_tokens: 5,
            cost_usd: 0.0,
            duration_ms: 1,
            estimated: false,
        }
    }

    fn tools(calls: &[(&str, &str, Value)]) -> Round {
        Round::Reply(
            calls
                .iter()
                .enumerate()
                .map(|(index, (id, name, args))| ChatEvent::ToolCallDelta {
                    index,
                    id: Some((*id).to_owned()),
                    name: Some((*name).to_owned()),
                    arguments: args.to_string(),
                })
                .collect(),
            receipt(),
        )
    }

    fn read(id: &str) -> Round {
        tools(&[(id, "read_file", json!({"path": "evidence.txt"}))])
    }

    fn write(id: &str, path: &str) -> Round {
        tools(&[(
            id,
            "write_file",
            json!({"path": path, "content": "approved"}),
        )])
    }

    fn answer(text: &str) -> Round {
        Round::Reply(vec![ChatEvent::Delta(text.to_owned())], receipt())
    }

    fn cards() -> (
        WorkflowApprovalHandler,
        sync_mpsc::Receiver<(Uuid, ApprovalCard)>,
    ) {
        let (tx, rx) = sync_mpsc::channel();
        (
            Arc::new(move |id, card| tx.send((id, card)).expect("card receiver")),
            rx,
        )
    }

    fn ask(host: &Arc<WorkflowHost>, agent: &str) -> sync_mpsc::Receiver<Result<Value, HostError>> {
        let host = host.clone();
        let agent = agent.to_owned();
        let (tx, rx) = sync_mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(host.ask(&agent, None, "Gather evidence and answer JSON"));
        });
        rx
    }

    fn assert_usage(host: &WorkflowHost, rounds: u64) {
        assert_eq!(host.tokens(), rounds * 15);
        assert_eq!(Host::tokens(host), rounds * 15);
        assert!((host.cost_usd() - rounds as f64 * 0.00025).abs() < 1e-10);
    }

    #[test]
    fn real_read_tools_keep_lossless_history_per_name() {
        let root = tempfile::tempdir().expect("workspace");
        fs::write(root.path().join("evidence.txt"), "real evidence").expect("fixture");
        let script = Script::new(vec![
            read("read-1"),
            answer("{\"ok\":true}"),
            answer("{}"),
            answer("{}"),
        ]);
        let host = WorkflowHost::new(root.path().to_owned(), script.config());
        host.ask("reviewer", Some("terse"), "first")
            .expect("read turn");
        let first = host.conversation("reviewer").expect("history");
        assert_eq!(first.len(), 5);
        assert_eq!(first[2].tool_calls.as_ref().expect("calls")[0].id, "read-1");
        assert_eq!(first[3].role, Role::Tool);
        assert_eq!(first[3].tool_call_id.as_deref(), Some("read-1"));
        assert!(first[3].content.contains("real evidence"));
        host.ask("other", None, "isolated").expect("other");
        host.ask("reviewer", None, "second").expect("continued");
        let seen = script.seen();
        assert_eq!(seen[2].messages.len(), 2);
        assert_eq!(&seen[3].messages[..first.len()], first);
        assert_eq!(seen[3].messages.last().expect("user").content, "second");
        for request in seen {
            assert_eq!(request.model, "authoritative-model");
            let tools = request.tools.expect("manifest");
            assert!(tools.iter().any(|tool| tool.name == "read_file"));
            assert!(!tools
                .iter()
                .any(|tool| matches!(tool.name.as_str(), "run_workflow" | "manage_todo")));
        }
        assert_usage(&host, 4);
    }

    #[test]
    fn proposed_write_parks_deny_does_not_write_and_wrong_ids_reject() {
        let root = tempfile::tempdir().expect("workspace");
        fs::write(root.path().join("target.txt"), "original").expect("fixture");
        let script = Script::new(vec![write("w1", "target.txt"), answer("{}")]);
        let (handler, cards) = cards();
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script.config().with_approval_handler(handler),
        );
        let result = ask(&host, "writer");
        let (id, card) = cards.recv_timeout(WAIT).expect("parked");
        assert!(matches!(card, ApprovalCard::Diff { path, .. } if path == "target.txt"));
        assert!(matches!(
            result.try_recv(),
            Err(sync_mpsc::TryRecvError::Empty)
        ));
        assert_eq!(
            fs::read_to_string(root.path().join("target.txt")).expect("read"),
            "original"
        );
        assert!(host
            .decide(Uuid::new_v4(), AgentDecision::ApproveAllThisTurn)
            .is_err());
        assert!(host.decide(Uuid::new_v4(), AgentDecision::Stop).is_err());
        assert!(matches!(
            result.try_recv(),
            Err(sync_mpsc::TryRecvError::Empty)
        ));
        host.decide(id, AgentDecision::Deny).expect("deny");
        assert!(host.decide(id, AgentDecision::Approve).is_err());
        result
            .recv_timeout(WAIT)
            .expect("unparked")
            .expect("denial continues");
        assert_eq!(
            fs::read_to_string(root.path().join("target.txt")).expect("read"),
            "original"
        );
        assert!(script.seen()[1]
            .messages
            .iter()
            .any(|message| message.content == selora_ai::DENIED_MESSAGE));
        assert_usage(&host, 2);
    }

    #[test]
    fn explicit_approve_writes_once_and_rejects_duplicate_and_stale_ids() {
        let root = tempfile::tempdir().expect("workspace");
        let script = Script::new(vec![write("w1", "new.txt"), answer("{}")]);
        let (handler, cards) = cards();
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script.config().with_approval_handler(handler),
        );
        let result = ask(&host, "writer");
        let (id, _) = cards.recv_timeout(WAIT).expect("card");
        assert!(!root.path().join("new.txt").exists());
        host.decide(id, AgentDecision::Approve).expect("approve");
        assert!(host.decide(id, AgentDecision::Approve).is_err());
        result
            .recv_timeout(WAIT)
            .expect("unparked")
            .expect("completed");
        assert_eq!(
            fs::read_to_string(root.path().join("new.txt")).expect("read"),
            "approved"
        );
        assert!(host.decide(id, AgentDecision::Deny).is_err());
        assert_usage(&host, 2);
    }

    #[test]
    fn approval_slot_exists_before_synchronous_callback_decides() {
        let root = tempfile::tempdir().expect("workspace");
        let script = Script::new(vec![write("w1", "new.txt"), answer("{}")]);
        let reference = Arc::new(Mutex::new(Weak::<WorkflowHost>::new()));
        let callback_ref = reference.clone();
        let handler = Arc::new(move |id, _| {
            let host = callback_ref.lock().expect("host").upgrade().expect("live");
            host.decide(id, AgentDecision::Approve)
                .expect("slot already installed");
        });
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script.config().with_approval_handler(handler),
        );
        *reference.lock().expect("reference") = Arc::downgrade(&host);
        host.ask("writer", None, "write")
            .expect("synchronous approval does not deadlock");
        assert!(root.path().join("new.txt").exists());
    }

    #[test]
    fn approve_all_stays_local_to_one_turn_not_other_agent_or_next_ask() {
        let root = tempfile::tempdir().expect("workspace");
        let script = Script::new(vec![
            tools(&[
                ("a1", "write_file", json!({"path":"a1.txt","content":"one"})),
                ("a2", "write_file", json!({"path":"a2.txt","content":"two"})),
            ]),
            write("b1", "b1.txt"),
            answer("{}"),
            answer("{}"),
            write("a3", "a3.txt"),
            answer("{}"),
        ]);
        let (handler, cards) = cards();
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script.config().with_approval_handler(handler),
        );
        let a = ask(&host, "a");
        let (a_id, _) = cards.recv_timeout(WAIT).expect("a card");
        let b = ask(&host, "b");
        let (b_id, _) = cards.recv_timeout(WAIT).expect("b card");
        host.decide(a_id, AgentDecision::ApproveAllThisTurn)
            .expect("approve a only");
        a.recv_timeout(WAIT).expect("a done").expect("a completed");
        assert!(root.path().join("a1.txt").exists());
        assert!(root.path().join("a2.txt").exists());
        assert!(!root.path().join("b1.txt").exists());
        assert!(matches!(b.try_recv(), Err(sync_mpsc::TryRecvError::Empty)));
        host.decide(b_id, AgentDecision::Deny)
            .expect("b still has its own card");
        b.recv_timeout(WAIT).expect("b done").expect("b completed");
        let next = ask(&host, "a");
        let (next_id, _) = cards
            .recv_timeout(WAIT)
            .expect("new ask needs fresh approval");
        assert!(!root.path().join("a3.txt").exists());
        host.decide(next_id, AgentDecision::Deny)
            .expect("deny new ask");
        next.recv_timeout(WAIT)
            .expect("next done")
            .expect("next completed");
        assert_usage(&host, 6);
    }

    #[test]
    fn no_handler_fails_typed_before_side_effect_and_keeps_receipt() {
        let root = tempfile::tempdir().expect("workspace");
        let script = Script::new(vec![write("w1", "never.txt")]);
        let host = WorkflowHost::new(root.path().to_owned(), script.config());
        let error = host
            .ask("writer", None, "write")
            .expect_err("no approval route");
        assert!(
            error.message.contains("approval handler is not configured"),
            "{error}"
        );
        assert!(!root.path().join("never.txt").exists());
        assert_usage(&host, 1);
        assert!(host.lock(&host.live_turns).turns.is_empty());
    }

    #[test]
    fn cancel_unblocks_parked_approval_and_rejects_stale_decisions() {
        let root = tempfile::tempdir().expect("workspace");
        let script = Script::new(vec![write("w1", "never.txt")]);
        let (handler, cards) = cards();
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script.config().with_approval_handler(handler),
        );
        let result = ask(&host, "writer");
        let (id, _) = cards.recv_timeout(WAIT).expect("parked");
        let start = Instant::now();
        host.cancel();
        result
            .recv_timeout(Duration::from_secs(1))
            .expect("cancel promptly unparked")
            .expect_err("stopped");
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(Host::is_cancelled(host.as_ref()));
        assert!(host.decide(id, AgentDecision::Approve).is_err());
        assert!(!root.path().join("never.txt").exists());
        assert_usage(&host, 1);
    }

    #[test]
    fn stop_decision_unblocks_parked_turn_without_write() {
        let root = tempfile::tempdir().expect("workspace");
        let script = Script::new(vec![write("w1", "never.txt")]);
        let (handler, cards) = cards();
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script.config().with_approval_handler(handler),
        );
        let result = ask(&host, "writer");
        let (id, _) = cards.recv_timeout(WAIT).expect("parked");
        host.decide(id, AgentDecision::Stop)
            .expect("stop matching live card");
        result
            .recv_timeout(Duration::from_secs(1))
            .expect("stopped promptly")
            .expect_err("stop");
        assert!(!root.path().join("never.txt").exists());
        assert_usage(&host, 1);
    }

    #[test]
    fn cancel_drops_inflight_stream_and_keeps_prior_round_usage() {
        let root = tempfile::tempdir().expect("workspace");
        fs::write(root.path().join("evidence.txt"), "evidence").expect("fixture");
        let (started_tx, started_rx) = sync_mpsc::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let script = Script::new(vec![
            read("r1"),
            Round::Pending(started_tx, dropped.clone()),
        ]);
        let host = WorkflowHost::new(root.path().to_owned(), script.config());
        let result = ask(&host, "reader");
        started_rx
            .recv_timeout(WAIT)
            .expect("second HTTP stream in flight");
        host.cancel();
        result
            .recv_timeout(Duration::from_secs(1))
            .expect("stream cancelled promptly")
            .expect_err("stopped");
        assert!(dropped.load(Ordering::SeqCst), "stream future was dropped");
        assert_usage(&host, 1);
        assert_eq!(
            host.conversation("reader").expect("history")[3]
                .tool_call_id
                .as_deref(),
            Some("r1")
        );
    }

    #[test]
    fn disabled_hallucinated_tools_reject_on_both_runner_paths_and_all_bash_cards() {
        let root = tempfile::tempdir().expect("workspace");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let mut tools = WorkflowTools::new(ToolContext::new(
                root.path(),
                AgentToolSettings { auto_approve_commands: Vec::new(), ..AgentToolSettings::default() },
            ));
            for name in ["run_workflow", "manage_todo"] {
                let call = ToolCall { id: name.to_owned(), name: name.to_owned(), arguments: "{}".to_owned() };
                assert!(matches!(tools.run(call.clone()).await, ToolOutcome::Ready(result) if !result.ok && result.content.contains("disabled")));
                assert!(!tools.run_approved(call).await.ok);
            }
            for command in ["git status", "cargo test", "Get-Content evidence.txt", "git status; Set-Content evil.txt evil"] {
                let call = ToolCall { id: "bash".to_owned(), name: "bash".to_owned(), arguments: json!({"command":command}).to_string() };
                assert!(matches!(tools.run(call).await, ToolOutcome::NeedsApproval(ApprovalCard::Command { .. })));
            }
        });
        assert!(!root.path().join("evil.txt").exists());
    }

    #[test]
    fn caps_reset_per_ask_and_model_id_prices_and_context_are_authoritative() {
        let root = tempfile::tempdir().expect("workspace");
        fs::write(root.path().join("evidence.txt"), "evidence").expect("fixture");
        let script = Script::new(vec![read("r1"), answer("{}"), read("r2"), answer("{}")]);
        let mut config = script.config().with_turn_caps(TurnCaps {
            max_tool_iterations: 1,
            context_window: 1,
            max_turn_secs: 3,
            ..TurnCaps::spec_defaults(1)
        });
        config.model = "must-not-override-metadata".to_owned();
        let host = WorkflowHost::new(root.path().to_owned(), config);
        host.ask("reader", None, "first")
            .expect("one tool permitted");
        host.ask("reader", None, "second")
            .expect("fresh one-tool cap");
        assert_eq!(host.scheduler.caps().max_tool_iterations, 1);
        assert_eq!(host.scheduler.caps().context_window, 200_000);
        assert!(script
            .seen()
            .iter()
            .all(|request| request.model == "authoritative-model"));
        assert_usage(&host, 4);
    }

    #[test]
    fn failed_stream_and_invalid_json_preserve_paid_receipts_and_redact_errors() {
        let root = tempfile::tempdir().expect("workspace");
        fs::write(root.path().join("evidence.txt"), "evidence").expect("fixture");
        let script = Script::new(vec![
            read("r1"),
            Round::Fail("gateway echoed test-key"),
            answer("invalid test-key"),
        ]);
        let host = WorkflowHost::new(root.path().to_owned(), script.config());
        let error = host
            .ask("reader", None, "first")
            .expect_err("stream failed");
        assert!(!error.message.contains("test-key"));
        assert!(error.message.contains("[REDACTED]"));
        assert_usage(&host, 1);
        let failed = host.conversation("reader").expect("failed transcript");
        let error = host.ask("reader", None, "again").expect_err("invalid JSON");
        assert!(error.message.contains("not JSON"));
        assert!(!error.message.contains("test-key"));
        assert_eq!(&script.seen()[2].messages[..failed.len()], failed);
        assert_usage(&host, 2);
        assert!(!host
            .files_read("../test-key")
            .expect_err("bad path")
            .message
            .contains("test-key"));
        assert!(!host
            .git_diff(Some("--test-key"), None)
            .expect_err("bad revision")
            .message
            .contains("test-key"));
        let fake = WorkflowHost::new(
            root.path().to_owned(),
            script
                .config()
                .with_chat(|_, _| Err(HostError::new("test-key chat error"))),
        );
        assert!(!fake
            .ask("fake", None, "ask")
            .expect_err("fake failure")
            .message
            .contains("test-key"));
    }

    #[test]
    fn schema_failure_keeps_receipts_in_host_and_engine_finished_report() {
        let root = tempfile::tempdir().expect("workspace");
        let script = Script::new(vec![answer("{\"count\":\"wrong type\"}")]);
        let host = WorkflowHost::new(root.path().to_owned(), script.config());
        let spec = WorkflowSpec::new(
            "schema",
            r#"
            agent("reviewer").ask({ type: "object", properties: {count: {type: "number"}}, required: ["count"] }, "Count findings");
        "#,
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let capture = events.clone();
        let error = selora_workflow::run_workflow(
            &spec,
            host.clone(),
            Arc::new(move |event| capture.lock().expect("events").push(event)),
        )
        .expect_err("schema mismatch");
        assert!(error.to_string().contains("count"), "{error}");
        assert_usage(&host, 1);
        let events = events.lock().expect("events");
        assert!(
            matches!(events.last(), Some(WorkflowEvent::Finished { report }) if report.tokens == 15 && (report.cost_usd - 0.00025).abs() < 1e-10)
        );
        assert!(script.seen()[0]
            .messages
            .last()
            .expect("schema prompt")
            .content
            .contains("\"count\""));
    }

    #[test]
    fn concurrent_same_name_asks_snapshot_history_only_after_fifo_slot() {
        let root = tempfile::tempdir().expect("workspace");
        fs::write(root.path().join("evidence.txt"), "evidence").expect("fixture");
        let (started_tx, started_rx) = sync_mpsc::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        let script = Script::new(vec![
            read("r1"),
            Round::Paused(started_tx, resume_rx),
            answer("{}"),
        ]);
        let host = WorkflowHost::new(root.path().to_owned(), script.config());
        let first = ask(&host, "shared");
        started_rx
            .recv_timeout(WAIT)
            .expect("first ask holds FIFO slot after read");
        let second = ask(&host, "shared");
        resume_tx.send(()).expect("finish first turn");
        first
            .recv_timeout(WAIT)
            .expect("first returned")
            .expect("first completed");
        second
            .recv_timeout(WAIT)
            .expect("second returned")
            .expect("second completed");
        let seen = script.seen();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[2].messages.len(), 6);
        assert_eq!(
            seen[2].messages[2].tool_calls.as_ref().expect("full calls")[0].id,
            "r1"
        );
        assert_eq!(seen[2].messages[3].tool_call_id.as_deref(), Some("r1"));
        assert_eq!(seen[2].messages[4].content, "{}");
        assert_usage(&host, 3);
    }

    #[test]
    fn cancel_releases_real_scheduler_waiters_for_same_and_different_names() {
        let root = tempfile::tempdir().expect("workspace");
        let (started_tx, started_rx) = sync_mpsc::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let script = Script::new(vec![Round::Pending(started_tx, dropped.clone())]);
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script.config().with_max_concurrency(1),
        );
        let first = ask(&host, "shared");
        started_rx.recv_timeout(WAIT).expect("active HTTP stream");
        let same = ask(&host, "shared");
        let other = ask(&host, "other");
        host.cancel();
        for result in [first, same, other] {
            result
                .recv_timeout(Duration::from_secs(1))
                .expect("cancel released scheduler waiter")
                .expect_err("cancelled");
        }
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(script.seen().len(), 1, "queued asks never start HTTP");
        assert_usage(&host, 0);
    }

    #[test]
    fn real_tool_paths_reject_workspace_escape_without_approval_or_side_effect() {
        let root = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(outside.path().join("secret.txt"), "outside secret").expect("fixture");
        let script = Script::new(vec![
            tools(&[
                (
                    "r1",
                    "read_file",
                    json!({"path":outside.path().join("secret.txt").to_string_lossy()}),
                ),
                (
                    "w1",
                    "write_file",
                    json!({"path":outside.path().join("new.txt").to_string_lossy(), "content":"never"}),
                ),
            ]),
            answer("{}"),
        ]);
        let host = WorkflowHost::new(root.path().to_owned(), script.config());
        host.ask("reader", None, "try outside")
            .expect("model receives guarded tool errors");
        assert!(!outside.path().join("new.txt").exists());
        let messages = &script.seen()[1].messages;
        assert!(!messages[3].content.contains("outside secret"));
        assert!(messages[3].content.contains("outside") || messages[3].content.contains("escape"));
        assert!(messages[4].content.contains("outside") || messages[4].content.contains("escape"));
        assert_usage(&host, 2);
    }

    #[test]
    fn tool_cap_failure_and_expired_approval_preserve_receipts() {
        let root = tempfile::tempdir().expect("workspace");
        let script = Script::new(vec![write("w1", "never.txt")]);
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script.config().with_turn_caps(TurnCaps {
                max_tool_iterations: 0,
                ..TurnCaps::spec_defaults(200_000)
            }),
        );
        let error = host.ask("writer", None, "write").expect_err("tool cap");
        assert!(error.message.contains("iteration cap"));
        assert!(!root.path().join("never.txt").exists());
        assert_usage(&host, 1);

        let script = Script::new(vec![write("w2", "never.txt")]);
        let (handler, cards) = cards();
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script
                .config()
                .with_turn_caps(TurnCaps {
                    max_turn_secs: 1,
                    ..TurnCaps::spec_defaults(200_000)
                })
                .with_approval_handler(handler),
        );
        let result = ask(&host, "writer");
        let (id, _) = cards.recv_timeout(WAIT).expect("pending approval");
        let error = result
            .recv_timeout(WAIT)
            .expect("deadline releases park")
            .expect_err("time cap");
        assert!(error.message.contains("time cap"));
        assert!(host.decide(id, AgentDecision::Approve).is_err());
        assert!(!root.path().join("never.txt").exists());
        assert_usage(&host, 1);
    }

    #[test]
    #[cfg(windows)]
    fn cancel_aborts_approved_bash_tool_turn_promptly() {
        let root = tempfile::tempdir().expect("workspace");
        let script = Script::new(vec![tools(&[(
            "bash-1",
            "bash",
            json!({
            "command":"Set-Content -Path child.pid -Value $PID -Encoding Ascii; Start-Sleep -Seconds 30"
            }),
        )])]);
        let (handler, cards) = cards();
        let host = WorkflowHost::new(
            root.path().to_owned(),
            script
                .config()
                .with_turn_caps(TurnCaps {
                    max_turn_secs: 10,
                    ..TurnCaps::spec_defaults(200_000)
                })
                .with_approval_handler(handler),
        );
        let result = ask(&host, "runner");
        let (id, card) = cards
            .recv_timeout(WAIT)
            .expect("shell command needs approval");
        assert!(matches!(card, ApprovalCard::Command { .. }));
        assert!(!root.path().join("child.pid").exists());
        host.decide(id, AgentDecision::Approve)
            .expect("approve shell");
        let deadline = Instant::now() + WAIT;
        let pid = loop {
            if let Ok(text) = fs::read_to_string(root.path().join("child.pid")) {
                if let Ok(pid) = text.trim_start_matches('\u{feff}').trim().parse::<u32>() {
                    break pid;
                }
            }
            assert!(Instant::now() < deadline, "shell never published its PID");
            std::thread::sleep(Duration::from_millis(10));
        };
        host.cancel();
        result
            .recv_timeout(Duration::from_secs(1))
            .expect("running tool aborted promptly")
            .expect_err("cancelled");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let probe = std::process::Command::new(super::super::powershell_program())
                .args(["-NoProfile", "-NonInteractive", "-Command", &format!(
                    "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 1 }} else {{ exit 0 }}"
                )]).output().expect("probe process");
            if probe.status.success() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the direct shell child is still alive (pid {pid})"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_usage(&host, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn production_client_streams_tools_against_local_gateway() {
        use wiremock::{
            matchers::{body_partial_json, method, path},
            Mock, MockServer, ResponseTemplate,
        };

        let server = MockServer::start().await;
        let root = tempfile::tempdir().expect("workspace");
        fs::write(root.path().join("evidence.txt"), "wire evidence").expect("fixture");
        let first = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"wire-read\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"evidence.txt\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        let second = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"ok\\\":true}\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(first),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({"messages":[
                {"role":"system"}, {"role":"user"},
                {"role":"assistant","tool_calls":[{"id":"wire-read"}]},
                {"role":"tool","tool_call_id":"wire-read"}
            ]})))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(second),
            )
            .expect(1)
            .mount(&server)
            .await;
        let host = WorkflowHost::new(
            root.path().to_owned(),
            WorkflowHostConfig::new(server.uri(), "test-key", "ignored").with_model(model()),
        );
        let worker = host.clone();
        let value =
            tokio::task::spawn_blocking(move || worker.ask("reader", None, "read evidence"))
                .await
                .expect("worker")
                .expect("real streamed turn");
        assert_eq!(value, json!({"ok":true}));
        assert_usage(&host, 2);
        let requests = server.received_requests().await.expect("requests");
        let last: Value = serde_json::from_slice(&requests[1].body).expect("request JSON");
        assert!(last["messages"][3]["content"]
            .as_str()
            .expect("tool content")
            .contains("wire evidence"));
        assert_eq!(last["stream"], true);
    }
}
