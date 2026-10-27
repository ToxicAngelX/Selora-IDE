//! Phase 9 crash/soak coverage for the agent turn loop.
//!
//! These tests choose the crash window explicitly (the loop's Stop/abort seam
//! is the deterministic stand-in for a process kill) and then prove the
//! invariants a restart depends on:
//!
//! * a killed stream ends with the partial answer and never runs a tool;
//! * a killed approval park executes nothing;
//! * a killed approved write/bash never emits a tool result;
//! * stale decisions cannot unpark a card;
//! * the event stream is ordered, and `TurnDone` is the single last event,
//!   under repeated and concurrent load.
//!
//! One test (`process_kill_while_parked_leaves_the_workspace_untouched`) uses
//! a real child process: the child parks on a write card, the parent kills it,
//! and the workspace must be byte-identical. The child only runs when
//! `SELORA_PHASE9_CHILD` is set by its parent.
//!
//! PENDING (no public API on `integration` yet; tests land unchanged on the
//! branch that adds them):
//! * question-card parks and plan-card parks (`ask_user_question`,
//!   `enter_plan_mode`/`exit_plan_mode` — Phase 9b/9c);
//! * session JSONL restart and compaction rounds (Phase 9d).
//!
//! Those cases are tracked in `docs/PHASE9_SOAK.md` and must not be invented
//! here.

use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use selora_ai::{
    AgentDecision, AgentEvent, AgentTurn, AgentTurnState, ApprovalCard, ChatEvent, ChatMessage,
    ChatReceipt, ChatRequest, ChatStream, Model, RouterToolRunner, ToolCall, ToolContext,
    ToolOutcome, ToolResult, ToolRunner, TurnCaps, TurnDecision, TurnEnd, TurnInput,
};
use selora_core::SeloraError;
use tokio::sync::{mpsc, Notify};
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Deterministic fake gateway (a scripted `ChatStream`, no network)
// ---------------------------------------------------------------------------

enum Round {
    Answer {
        text: &'static str,
        tokens: (u32, u32, f64),
    },
    Tools {
        text: &'static str,
        calls: Vec<(&'static str, &'static str, &'static str)>,
        tokens: (u32, u32, f64),
    },
    /// Stream the deltas, then stay pending until the future is dropped. The
    /// drop flag proves cancellation reached the gateway future.
    Stall {
        deltas: Vec<&'static str>,
        dropped: Arc<AtomicBool>,
    },
}

struct FakeChat {
    rounds: Mutex<VecDeque<Round>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl FakeChat {
    fn new(rounds: Vec<Round>) -> Self {
        Self {
            rounds: Mutex::new(rounds.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn receipt(tokens: (u32, u32, f64)) -> ChatReceipt {
    ChatReceipt {
        request_id: Uuid::new_v4(),
        model: "test-model".to_string(),
        prompt_tokens: tokens.0,
        completion_tokens: tokens.1,
        cost_usd: tokens.2,
        duration_ms: 0,
        estimated: false,
    }
}

impl ChatStream for FakeChat {
    fn stream_round(
        &mut self,
        request: ChatRequest,
        events: mpsc::UnboundedSender<ChatEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<ChatReceipt, SeloraError>> + Send + '_>> {
        self.requests.lock().expect("requests lock").push(request);
        let round = self
            .rounds
            .lock()
            .expect("rounds lock")
            .pop_front()
            .expect("scripted round");
        Box::pin(async move {
            match round {
                Round::Answer { text, tokens } => {
                    let _ = events.send(ChatEvent::Delta(text.to_string()));
                    let receipt = receipt(tokens);
                    let _ = events.send(ChatEvent::Done(receipt.clone()));
                    Ok(receipt)
                }
                Round::Tools {
                    text,
                    calls,
                    tokens,
                } => {
                    if !text.is_empty() {
                        let _ = events.send(ChatEvent::Delta(text.to_string()));
                    }
                    for (index, (id, name, arguments)) in calls.into_iter().enumerate() {
                        let _ = events.send(ChatEvent::ToolCallDelta {
                            index,
                            id: Some(id.to_string()),
                            name: Some(name.to_string()),
                            arguments: arguments.to_string(),
                        });
                    }
                    let receipt = receipt(tokens);
                    let _ = events.send(ChatEvent::Done(receipt.clone()));
                    Ok(receipt)
                }
                Round::Stall { deltas, dropped } => {
                    let _guard = DropFlag(dropped);
                    for delta in deltas {
                        let _ = events.send(ChatEvent::Delta(delta.to_string()));
                    }
                    std::future::pending::<()>().await;
                    unreachable!("the stalled stream is always dropped");
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Scripted tool runner
// ---------------------------------------------------------------------------

struct RunnerState {
    card: bool,
    stall_approved: bool,
    routed: Mutex<Vec<String>>,
    approved_runs: Mutex<Vec<String>>,
    aborted: AtomicBool,
    approved_started: AtomicUsize,
    approved_started_notify: Notify,
    release_approved: Notify,
}

struct FakeRunner {
    state: Arc<RunnerState>,
}

impl FakeRunner {
    fn build(card: bool, stall_approved: bool) -> (Self, Arc<RunnerState>) {
        let state = Arc::new(RunnerState {
            card,
            stall_approved,
            routed: Mutex::new(Vec::new()),
            approved_runs: Mutex::new(Vec::new()),
            aborted: AtomicBool::new(false),
            approved_started: AtomicUsize::new(0),
            approved_started_notify: Notify::new(),
            release_approved: Notify::new(),
        });
        (
            Self {
                state: Arc::clone(&state),
            },
            state,
        )
    }

    fn card(stall_approved: bool) -> (Self, Arc<RunnerState>) {
        Self::build(true, stall_approved)
    }

    fn ready() -> (Self, Arc<RunnerState>) {
        Self::build(false, false)
    }
}

impl ToolRunner for FakeRunner {
    fn run(&mut self, call: ToolCall) -> Pin<Box<dyn Future<Output = ToolOutcome> + Send>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            state
                .routed
                .lock()
                .expect("routed lock")
                .push(call.id.clone());
            if state.card {
                ToolOutcome::NeedsApproval(match call.name.as_str() {
                    "bash" => ApprovalCard::Command {
                        command: "Write-Output touched".to_string(),
                    },
                    _ => ApprovalCard::Diff {
                        path: "src/main.rs".to_string(),
                        diff: "@@ -1 +1 @@\n-old\n+new\n".to_string(),
                    },
                })
            } else {
                ToolOutcome::Ready(ToolResult::success(call.id, "read-ok"))
            }
        })
    }

    fn run_approved(&mut self, call: ToolCall) -> Pin<Box<dyn Future<Output = ToolResult> + Send>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            state
                .approved_runs
                .lock()
                .expect("approved lock")
                .push(call.id.clone());
            state.approved_started.fetch_add(1, Ordering::SeqCst);
            state.approved_started_notify.notify_waiters();
            if state.stall_approved {
                state.release_approved.notified().await;
            }
            ToolResult::success(call.id, "applied")
        })
    }

    fn abort(&mut self) {
        self.state.aborted.store(true, Ordering::SeqCst);
        // A cooperating runner kills its child and returns; the loop discards
        // the outcome after its grace window either way.
        self.state.release_approved.notify_waiters();
    }
}

// ---------------------------------------------------------------------------
// Turn driver
// ---------------------------------------------------------------------------

struct Recorder {
    rx: mpsc::UnboundedReceiver<AgentEvent>,
    seen: Vec<AgentEvent>,
}

impl Recorder {
    fn new(rx: mpsc::UnboundedReceiver<AgentEvent>) -> Self {
        Self {
            rx,
            seen: Vec::new(),
        }
    }

    async fn next(&mut self) -> AgentEvent {
        let event = tokio::time::timeout(WAIT, self.rx.recv())
            .await
            .expect("event before timeout")
            .expect("event channel open");
        self.seen.push(event.clone());
        event
    }

    async fn wait_for_approval(&mut self) -> (Uuid, ApprovalCard) {
        loop {
            if let AgentEvent::ApprovalNeeded { approval_id, card } = self.next().await {
                return (approval_id, card);
            }
        }
    }

    async fn wait_for_delta(&mut self, expected: usize) {
        let mut deltas = 0;
        while deltas < expected {
            if matches!(self.next().await, AgentEvent::Delta(_)) {
                deltas += 1;
            }
        }
    }

    fn drain(&mut self) -> Vec<AgentEvent> {
        while let Ok(event) = self.rx.try_recv() {
            self.seen.push(event);
        }
        self.seen.clone()
    }
}

struct Harness {
    decisions: mpsc::UnboundedSender<TurnDecision>,
    recorder: Recorder,
    join: tokio::task::JoinHandle<selora_ai::AgentTurnReport>,
}

fn start(input: TurnInput, chat: FakeChat, runner: FakeRunner) -> Harness {
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let (decision_tx, decision_rx) = mpsc::unbounded_channel();
    let join = tokio::spawn(selora_ai::run_agent_turn(
        input,
        chat,
        runner,
        events_tx,
        decision_rx,
    ));
    Harness {
        decisions: decision_tx,
        recorder: Recorder::new(events_rx),
        join,
    }
}

fn model() -> Model {
    Model {
        id: "test-model".to_string(),
        display_name: "test".to_string(),
        context_window: 128_000,
        price_per_1k_input: 0.0,
        price_per_1k_output: 0.0,
        supports_streaming: true,
        supports_tools: true,
        family: "test".to_string(),
    }
}

fn input() -> TurnInput {
    TurnInput::new(model(), vec![ChatMessage::user("do the thing")])
        .with_caps(TurnCaps::spec_defaults(128_000))
        .with_manifest(Vec::new())
}

fn write_call() -> (&'static str, &'static str, &'static str) {
    (
        "call_write",
        "write_file",
        r#"{"path":"src/main.rs","content":"new"}"#,
    )
}

fn bash_call() -> (&'static str, &'static str, &'static str) {
    ("call_bash", "bash", r#"{"command":"Write-Output touched"}"#)
}

// ---------------------------------------------------------------------------
// Ordering invariants
// ---------------------------------------------------------------------------

/// Assert the whole per-turn stream obeys the UI contract.
fn assert_event_contract(events: &[AgentEvent], expect_completed: bool) {
    assert!(!events.is_empty(), "a turn always emits events");
    let done_positions: Vec<usize> = events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| matches!(event, AgentEvent::TurnDone(_)).then_some(index))
        .collect();
    assert_eq!(done_positions.len(), 1, "exactly one TurnDone: {events:?}");
    let done = done_positions[0];
    assert_eq!(done, events.len() - 1, "TurnDone must be last: {events:?}");
    assert!(
        events[..done].contains(&AgentEvent::State(AgentTurnState::Done)),
        "State(Done) must precede TurnDone"
    );

    let mut started: Vec<String> = Vec::new();
    let mut finished: Vec<String> = Vec::new();
    for (index, event) in events[..done].iter().enumerate() {
        match event {
            AgentEvent::ToolCallStarted(call) => {
                assert!(
                    matches!(
                        events.get(index.wrapping_sub(1)),
                        Some(AgentEvent::State(AgentTurnState::RunningTool(name))) if name == &call.name
                    ),
                    "ToolCallStarted must follow State(RunningTool): {events:?}"
                );
                assert!(
                    !started.contains(&call.id),
                    "duplicate ToolCallStarted for {}: {events:?}",
                    call.id
                );
                started.push(call.id.clone());
            }
            AgentEvent::ToolResult(result) => {
                assert!(
                    started.contains(&result.id),
                    "ToolResult before ToolCallStarted for {}: {events:?}",
                    result.id
                );
                assert!(
                    !finished.contains(&result.id),
                    "duplicate ToolResult for {}: {events:?}",
                    result.id
                );
                finished.push(result.id.clone());
            }
            AgentEvent::ApprovalNeeded { .. } => {
                assert!(
                    matches!(
                        events.get(index.wrapping_sub(1)),
                        Some(AgentEvent::State(AgentTurnState::AwaitingApproval(_)))
                    ),
                    "ApprovalNeeded must follow State(AwaitingApproval): {events:?}"
                );
            }
            AgentEvent::Delta(_) if expect_completed => {}
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 1. Kill mid-stream
// ---------------------------------------------------------------------------

#[tokio::test]
async fn kill_mid_stream_keeps_partial_answer_and_runs_nothing() {
    let dropped = Arc::new(AtomicBool::new(false));
    let chat = FakeChat::new(vec![Round::Stall {
        deltas: vec!["par", "tial"],
        dropped: Arc::clone(&dropped),
    }]);
    let (runner, state) = FakeRunner::ready();
    let mut harness = start(input(), chat, runner);

    harness.recorder.wait_for_delta(2).await;
    harness
        .decisions
        .send((Uuid::nil(), AgentDecision::Stop))
        .expect("stop decision");

    let report = harness.join.await.expect("turn joins");
    let events = harness.recorder.drain();
    assert_eq!(report.end, TurnEnd::StoppedByUser);
    assert_eq!(report.final_message, "partial");
    assert!(!report.summary.completed);
    assert_eq!(report.summary.tools_used, 0);
    assert_eq!(report.conversation.len(), 1, "no phantom assistant message");
    assert!(
        dropped.load(Ordering::SeqCst),
        "the gateway future was cancelled"
    );
    assert!(state.routed.lock().unwrap().is_empty());
    assert!(state.approved_runs.lock().unwrap().is_empty());
    assert_event_contract(&events, false);
}

// ---------------------------------------------------------------------------
// 2. Kill mid-approval-park (write and bash)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn kill_while_parked_on_a_write_card_executes_nothing() {
    let chat = FakeChat::new(vec![Round::Tools {
        text: "writing",
        calls: vec![write_call()],
        tokens: (10, 5, 0.001),
    }]);
    let (runner, state) = FakeRunner::card(false);
    let mut harness = start(input(), chat, runner);

    let (approval_id, card) = harness.recorder.wait_for_approval().await;
    assert!(matches!(card, ApprovalCard::Diff { .. }));
    harness
        .decisions
        .send((approval_id, AgentDecision::Stop))
        .expect("stop decision");

    let report = harness.join.await.expect("turn joins");
    assert_eq!(report.end, TurnEnd::StoppedByUser);
    assert_eq!(report.summary.approved, 0);
    assert_eq!(report.summary.tools_used, 0);
    assert_eq!(*state.routed.lock().unwrap(), vec!["call_write"]);
    assert!(state.approved_runs.lock().unwrap().is_empty());
    assert!(
        !state.aborted.load(Ordering::SeqCst),
        "no tool future was live"
    );
    let events = harness.recorder.drain();
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::ToolResult(_))));
    assert_event_contract(&events, false);
}

#[tokio::test]
async fn kill_while_parked_on_a_bash_card_executes_nothing() {
    let chat = FakeChat::new(vec![Round::Tools {
        text: "running",
        calls: vec![bash_call()],
        tokens: (10, 5, 0.001),
    }]);
    let (runner, state) = FakeRunner::card(false);
    let mut harness = start(input(), chat, runner);

    let (approval_id, card) = harness.recorder.wait_for_approval().await;
    assert!(matches!(card, ApprovalCard::Command { .. }));
    harness
        .decisions
        .send((approval_id, AgentDecision::Stop))
        .expect("stop decision");

    let report = harness.join.await.expect("turn joins");
    assert_eq!(report.end, TurnEnd::StoppedByUser);
    assert!(state.approved_runs.lock().unwrap().is_empty());
    let events = harness.recorder.drain();
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::ToolResult(_))));
    assert_event_contract(&events, false);
}

// ---------------------------------------------------------------------------
// 3. Kill mid-write / mid-bash execution (approved, future in flight)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn kill_during_approved_execution_aborts_and_emits_no_result() {
    for call in [write_call(), bash_call()] {
        let id = call.0;
        let chat = FakeChat::new(vec![Round::Tools {
            text: "applying",
            calls: vec![call],
            tokens: (10, 5, 0.001),
        }]);
        let (runner, state) = FakeRunner::card(true);
        let mut harness = start(input(), chat, runner);

        let (approval_id, _card) = harness.recorder.wait_for_approval().await;
        harness
            .decisions
            .send((approval_id, AgentDecision::Approve))
            .expect("approve decision");
        // Wait until the approved future is actually in flight.
        tokio::time::timeout(WAIT, async {
            loop {
                if state.approved_started.load(Ordering::SeqCst) == 1 {
                    break;
                }
                state.approved_started_notify.notified().await;
            }
        })
        .await
        .expect("approved execution starts");

        harness
            .decisions
            .send((Uuid::nil(), AgentDecision::Stop))
            .expect("stop decision");
        let report = harness.join.await.expect("turn joins");
        let events = harness.recorder.drain();

        assert_eq!(report.end, TurnEnd::StoppedByUser, "{id}");
        assert_eq!(report.summary.approved, 1, "{id}");
        assert_eq!(report.summary.tools_used, 0, "{id} must not count a result");
        assert!(
            state.aborted.load(Ordering::SeqCst),
            "{id} abort reached the runner"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::ToolResult(_))),
            "{id} emitted a result after cancellation: {events:?}"
        );
        assert_event_contract(&events, false);
    }
}

// ---------------------------------------------------------------------------
// 4. Stale decisions cannot unpark or execute
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_stale_approval_id_never_unparks_the_parked_card() {
    let chat = FakeChat::new(vec![
        Round::Tools {
            text: "writing",
            calls: vec![write_call()],
            tokens: (10, 5, 0.001),
        },
        Round::Answer {
            text: "acknowledged the denial",
            tokens: (8, 4, 0.001),
        },
    ]);
    let (runner, state) = FakeRunner::card(false);
    let mut harness = start(input(), chat, runner);

    let (approval_id, _) = harness.recorder.wait_for_approval().await;
    // A decision for another card must not unpark this one.
    harness
        .decisions
        .send((Uuid::new_v4(), AgentDecision::Approve))
        .expect("stale decision accepted by the channel");
    // The real decision denies; if the stale one had won, this would be too late
    // and the write would have run.
    harness
        .decisions
        .send((approval_id, AgentDecision::Deny))
        .expect("deny decision");

    let report = harness.join.await.expect("turn joins");
    assert_eq!(report.end, TurnEnd::Completed);
    assert_eq!(report.summary.denied, 1);
    assert_eq!(report.summary.approved, 0);
    assert!(state.approved_runs.lock().unwrap().is_empty());
    let events = harness.recorder.drain();
    assert_event_contract(&events, true);
}

// ---------------------------------------------------------------------------
// 5. Auto-approve is turn-local
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auto_approve_applies_only_inside_the_turn_that_enabled_it() {
    let chat = FakeChat::new(vec![
        Round::Tools {
            text: "writing",
            calls: vec![write_call()],
            tokens: (10, 5, 0.001),
        },
        Round::Answer {
            text: "done",
            tokens: (5, 2, 0.001),
        },
    ]);
    let (runner, state) = FakeRunner::card(false);
    let mut harness = start(input().with_auto_approve_writes(true), chat, runner);

    let report = harness.join.await.expect("turn joins");
    // No ApprovalNeeded event was needed.
    assert_eq!(report.summary.approved, 1);
    assert_eq!(*state.approved_runs.lock().unwrap(), vec!["call_write"]);
    let events = harness.recorder.drain();
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::ApprovalNeeded { .. })));
    assert_event_contract(&events, true);

    // The flag is a field of one `TurnInput`; a new turn starts off, and the
    // turn state's `approve_all` is off by construction.
    assert!(!TurnInput::new(model(), vec![ChatMessage::user("next")]).auto_approve_writes);
    assert!(!AgentTurn::new().approve_all);
}

// ---------------------------------------------------------------------------
// 6. Event ordering / soak
// ---------------------------------------------------------------------------

fn two_tool_round(id: &'static str) -> Round {
    Round::Tools {
        text: "working",
        calls: vec![
            (id, "read_file", r#"{"path":"src/main.rs"}"#),
            ("call_list", "list_dir", r#"{"path":"src"}"#),
        ],
        tokens: (20, 10, 0.002),
    }
}

#[tokio::test]
async fn repeated_turns_keep_the_event_contract() {
    for turn in 0..40 {
        let id = if turn % 2 == 0 {
            "call_read"
        } else {
            "call_scan"
        };
        let chat = FakeChat::new(vec![
            two_tool_round(id),
            Round::Answer {
                text: "answer",
                tokens: (4, 2, 0.0001),
            },
        ]);
        let (runner, _state) = FakeRunner::ready();
        let mut harness = start(input(), chat, runner);
        let report = tokio::time::timeout(WAIT, harness.join)
            .await
            .expect("turn finishes")
            .expect("turn joins");
        assert_eq!(report.end, TurnEnd::Completed, "turn {turn}");
        let events = harness.recorder.drain();
        assert_event_contract(&events, true);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentEvent::ToolResult(_)))
                .count(),
            2,
            "turn {turn}: {events:?}"
        );
    }
}

#[tokio::test]
async fn concurrent_turns_never_cross_events() {
    let mut joins = Vec::new();
    for index in 0..12 {
        let chat = FakeChat::new(vec![
            Round::Tools {
                text: "working",
                calls: vec![("call_read", "read_file", r#"{"path":"src/main.rs"}"#)],
                tokens: (index as u32 + 1, 2, 0.001),
            },
            Round::Answer {
                text: "answer",
                tokens: (3, 1, 0.0001),
            },
        ]);
        let (runner, _state) = FakeRunner::ready();
        let harness = start(input(), chat, runner);
        joins.push(async move {
            let mut harness = harness;
            let report = harness.join.await.expect("turn joins");
            let events = harness.recorder.drain();
            (index, report, events)
        });
    }

    for (index, report, events) in futures_util::future::join_all(joins).await {
        assert_eq!(report.end, TurnEnd::Completed, "turn {index}");
        assert_event_contract(&events, true);
        // Each stream carries only its own single read result.
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentEvent::ToolResult(_)))
                .count(),
            1,
            "turn {index}: {events:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 7. Real child-process kill while parked on a write card
// ---------------------------------------------------------------------------

fn child_root() -> PathBuf {
    PathBuf::from(std::env::var("SELORA_PHASE9_ROOT").expect("child root env"))
}

/// Child half of the process-kill test. It runs only when the parent sets
/// `SELORA_PHASE9_CHILD=approval`; otherwise it returns immediately so the
/// normal suite is unaffected.
#[tokio::test]
async fn child_process_parks_on_a_write_card() {
    if std::env::var("SELORA_PHASE9_CHILD").as_deref() != Ok("approval") {
        return;
    }
    let root = child_root();
    let chat = FakeChat::new(vec![Round::Tools {
        text: "about to write",
        calls: vec![write_call()],
        tokens: (10, 5, 0.001),
    }]);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (_decision_tx, decision_rx) = mpsc::unbounded_channel();
    let tools = RouterToolRunner::new(ToolContext::new(
        &root,
        selora_ai::AgentToolSettings::default(),
    ));
    let _turn = tokio::spawn(selora_ai::run_agent_turn(
        input(),
        chat,
        tools,
        events_tx,
        decision_rx,
    ));

    // Park, then tell the parent we are ready to be killed.
    loop {
        let event = tokio::time::timeout(WAIT, events_rx.recv())
            .await
            .expect("child event")
            .expect("child channel");
        if matches!(event, AgentEvent::ApprovalNeeded { .. }) {
            std::fs::write(root.join("child-ready"), b"parked").expect("marker");
            break;
        }
    }
    std::future::pending::<()>().await;
}

#[test]
fn process_kill_while_parked_leaves_the_workspace_untouched() {
    let root = std::env::temp_dir().join(format!(
        "selora-phase9-agent-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    std::fs::create_dir_all(root.join("src")).expect("root");
    std::fs::write(root.join("src/main.rs"), b"original\n").expect("seed");

    let exe = std::env::current_exe().expect("test binary");
    let mut child = std::process::Command::new(exe)
        .args([
            "--exact",
            "child_process_parks_on_a_write_card",
            "--nocapture",
        ])
        .env("SELORA_PHASE9_CHILD", "approval")
        .env("SELORA_PHASE9_ROOT", &root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn child");

    let marker = root.join("child-ready");
    let deadline = std::time::Instant::now() + WAIT;
    while !marker.is_file() {
        assert!(
            std::time::Instant::now() < deadline,
            "child never parked; stderr was suppressed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    child.kill().expect("kill child");
    let _ = child.wait();

    // The approval never arrived: the target and its recovery siblings are
    // untouched. A crash while parked is safe by construction.
    assert_eq!(
        std::fs::read_to_string(root.join("src/main.rs")).expect("seed survives"),
        "original\n"
    );
    let leftovers: Vec<String> = std::fs::read_dir(root.join("src"))
        .expect("src dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("selora-tmp") || name.contains("selora-bak"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    let _ = std::fs::remove_dir_all(root);
}
