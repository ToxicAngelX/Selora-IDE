//! The agent turn state machine (ARCHITECTURE.md §17.3–17.4) — Phase
//! 5c's file.
//!
//! One [`AgentTurn`] is a user message turned into a tool loop: stream a
//! `ChatRequest` with the manifest armed; on `finish_reason ==
//! "tool_calls"` run the calls in order via the router (5b); on a card,
//! park in [`AgentTurnState::AwaitingApproval`] until the user's decision
//! arrives; repeat until a tool-free answer. Caps (25 tool iterations /
//! 10 minutes / $1 / the model's context window minus a 20% reserve —
//! user-tunable in settings `agent.*`) abort with the partial answer and
//! a toast-able reason.
//!
//! The loop never talks HTTP and never executes a tool itself. Both are
//! injected as traits ([`ChatStream`], [`ToolRunner`]) so the production
//! backends are 5a's `SeloraClient` and 5b's router behind thin adapters,
//! and the tests script both sides (the mock-gateway seam — no network).
//! The UI side is equally one channel: [`AgentEvent`] out (5d renders the
//! states and cards), [`TurnDecision`] in (the shell maps
//! `Effect::AgentDecision` onto it).

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use selora_core::SeloraError;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::router::{self, ToolOutcome};
use super::{AgentDecision, ApprovalCard, ApprovalId};
use crate::chat_types::{ToolCall, ToolResult, ToolSpec};
use crate::types::{compute_cost_usd, ChatEvent, ChatMessage, ChatReceipt, ChatRequest, Model};

// ---------------------------------------------------------------------------
// Caps (§17.3 — defaults; the user tunes them via settings `agent.*`)
// ---------------------------------------------------------------------------

/// Max tool iterations per turn (§17.3: 25; settings `agent.max_iterations`).
pub const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 25;
/// Max wall-clock seconds per turn (§17.3: 600 = 10 min; settings
/// `agent.max_turn_secs`). `0` disables the time cap.
pub const DEFAULT_MAX_TURN_SECS: u64 = 600;
/// Max estimated USD per turn (§17.3: $1.00; settings `agent.max_cost_usd`).
pub const DEFAULT_MAX_COST_USD: f64 = 1.0;
/// The context reserve: the conversation is capped at the model's context
/// window minus 20% (the acceptance contract; `agent.*` has no field for
/// it — see the module docs in the Phase 5c report).
pub const CONTEXT_RESERVE: f64 = 0.20;
/// How long the loop waits, after [`ToolRunner::abort`], for a running
/// tool future to settle before dropping it outright. A cooperating runner
/// kills its bash child and returns in milliseconds; the Stop button must
/// still end the turn within ~1s even if it does not.
const ABORT_GRACE: Duration = Duration::from_millis(500);
/// The §17.4 words a denied tool returns to the model (it usually adapts).
pub const DENIED_MESSAGE: &str = "user denied this action";

/// The loop's hard limits for one turn. The shell builds this from
/// `settings.agent` (the three §17.3 caps) plus the active model's
/// context window; the tests prove the mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnCaps {
    /// Max tool iterations per turn.
    pub max_tool_iterations: u32,
    /// Max wall-clock seconds per turn (`0` = no cap).
    pub max_turn_secs: u64,
    /// Max estimated USD per turn.
    pub max_cost_usd: f64,
    /// The model's context window in tokens (`0` = unknown → no context
    /// cap, never a false abort).
    pub context_window: u32,
    /// The fraction of the window held back as reserve (default 20%).
    pub context_reserve: f64,
}

impl TurnCaps {
    /// The §17.3 spec defaults for a model with `context_window` tokens.
    pub fn spec_defaults(context_window: u32) -> Self {
        Self {
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            max_turn_secs: DEFAULT_MAX_TURN_SECS,
            max_cost_usd: DEFAULT_MAX_COST_USD,
            context_window,
            context_reserve: CONTEXT_RESERVE,
        }
    }

    /// The conversation's token budget: the context window minus the
    /// reserve. `None` when the window is unknown.
    pub fn context_budget_tokens(&self) -> Option<u64> {
        if self.context_window == 0 {
            return None;
        }
        let reserve = self.context_reserve.clamp(0.0, 0.9);
        Some((f64::from(self.context_window) * (1.0 - reserve)).floor() as u64)
    }
}

// ---------------------------------------------------------------------------
// End states and events
// ---------------------------------------------------------------------------

/// How a turn ended. Everything but [`TurnEnd::Completed`] follows a
/// partial answer; the shell toasts [`TurnEnd::toast`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TurnEnd {
    /// A tool-free answer arrived (§17.3: the normal exit).
    Completed,
    /// The tool iteration cap fired before the model stopped calling tools.
    IterationCap { limit: u32 },
    /// The turn's wall-clock cap fired (streaming, a tool, or an approval
    /// park — the clock runs from the turn's start).
    TimeCap { limit_secs: u64 },
    /// The estimated-cost cap fired (from the streamed receipts, with the
    /// model's price table as the fallback estimate).
    CostCap { limit_usd: f64 },
    /// The conversation outgrew the model's context budget.
    ContextOverflow {
        estimated_tokens: u64,
        budget_tokens: u64,
    },
    /// The user pressed Stop (§17.4): any running bash child was killed,
    /// the stream cancelled.
    StoppedByUser,
    /// The stream or a tool failed; the message is the surfaced error.
    Failed(String),
}

impl TurnEnd {
    /// The words the shell toasts when a turn ends this way (§17.3:
    /// "abort the loop, surface a toast + final partial answer").
    pub fn toast(&self) -> String {
        match self {
            TurnEnd::Completed => "agent turn complete".to_owned(),
            TurnEnd::IterationCap { limit } => {
                format!("agent turn stopped: tool iteration cap ({limit}) reached")
            }
            TurnEnd::TimeCap { limit_secs } => {
                format!("agent turn stopped: {limit_secs}s time cap reached")
            }
            TurnEnd::CostCap { limit_usd } => {
                format!("agent turn stopped: ${limit_usd:.2} estimated cost cap reached")
            }
            TurnEnd::ContextOverflow {
                estimated_tokens,
                budget_tokens,
            } => format!(
                "agent turn stopped: conversation (~{estimated_tokens} tokens) exceeds the context budget ({budget_tokens} tokens)"
            ),
            TurnEnd::StoppedByUser => "agent turn stopped by user".to_owned(),
            TurnEnd::Failed(message) => format!("agent turn failed: {message}"),
        }
    }

    /// `true` only for the normal exit.
    pub fn is_completed(&self) -> bool {
        matches!(self, TurnEnd::Completed)
    }
}

/// A decision routed back into a running turn. The shell maps
/// `Effect::AgentDecision(chat_id, approval_id, decision)` onto this;
/// `Stop` is honored regardless of which card it names (the Stop button
/// is live for the whole turn, not just while a card is pending).
pub type TurnDecision = (ApprovalId, AgentDecision);

/// What the loop tells the UI. The shell reduces these onto
/// `AppMessage` (`ToolCallStarted` / `ToolResult` / `ApprovalNeeded` /
/// `AgentTurnDone`, §17.6); 5d renders them per UI_SPEC.md §4.2.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// A state transition — the agent status line under the last bubble.
    State(AgentTurnState),
    /// One content delta of the model's current reply (the live bubble).
    Delta(String),
    /// The loop started one tool call (the tool card header appears).
    ToolCallStarted(ToolCall),
    /// One tool finished (executed, denied, or errored) — the card body.
    ToolResult(ToolResult),
    /// The loop parked on an approval card (§17.4) — 5d paints the card.
    ApprovalNeeded {
        approval_id: ApprovalId,
        card: ApprovalCard,
    },
    /// The turn ended: the summary, the reason, the (partial) answer, and
    /// the grown conversation.
    TurnDone(AgentTurnReport),
}

/// The turn's result: what [`AgentSummary`] counts, how it ended, the
/// model's last (possibly partial) answer, and the grown transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentTurnReport {
    pub summary: AgentSummary,
    pub end: TurnEnd,
    /// The model's last answer text — complete on [`TurnEnd::Completed`],
    /// partial (the last streamed text) on any abort.
    pub final_message: String,
    /// The conversation with this turn's assistant tool-call and
    /// tool-result messages appended — what the shell persists as the
    /// transcript.
    pub conversation: Vec<ChatMessage>,
}

// ---------------------------------------------------------------------------
// The state machine's live state
// ---------------------------------------------------------------------------

/// One step of the loop — painted as the agent status line under the
/// last bubble (UI_SPEC.md §4.2: `thinking` / `reading src/main.rs` /
/// `awaiting your approval` / `running cargo test`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTurnState {
    /// Streaming the model's reply.
    Thinking,
    /// Executing one tool (its name rides along for the status line).
    RunningTool(String),
    /// Parked on an approval card.
    AwaitingApproval(crate::agent::ApprovalId),
    /// The turn ended (answer, cap, or user stop).
    Done,
}

/// The turn summary footer (UI_SPEC.md §4.2: `6 tools · 3 approved · 1
/// denied · 12.4k tokens · $0.041`) — carried by
/// `AppMessage::AgentTurnDone`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSummary {
    pub iterations: u32,
    pub tools_used: u32,
    pub approved: u32,
    pub denied: u32,
    pub tokens: u64,
    pub cost_usd: f64,
    /// `false` when a cap or the Stop button aborted it (the summary then
    /// follows a partial answer).
    pub completed: bool,
}

/// The state machine's live state, owned by the loop task. The UI never
/// holds this — it renders the [`AgentEvent`] stream.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentTurn {
    pub state: AgentTurnState,
    /// Streaming rounds completed so far this turn.
    pub iterations: u32,
    /// Tool calls processed (executed, denied, or errored).
    pub tools_used: u32,
    /// Cards the user approved — including `ApproveAllThisTurn` itself
    /// and every card it pre-approved.
    pub approved: u32,
    /// Cards the user denied.
    pub denied: u32,
    /// Tokens across every round (prompt + completion, per receipt).
    pub tokens: u64,
    /// Estimated USD across every round.
    pub cost_usd: f64,
    /// `ApproveAllThisTurn` is armed: the remaining write/bash cards this
    /// turn are pre-approved (§17.4). Turn-local by construction — a
    /// plain field on this turn's state, never persisted anywhere.
    pub approve_all: bool,
}

impl AgentTurn {
    /// A turn starting in `Thinking`.
    pub fn new() -> Self {
        Self {
            state: AgentTurnState::Thinking,
            iterations: 0,
            tools_used: 0,
            approved: 0,
            denied: 0,
            tokens: 0,
            cost_usd: 0.0,
            approve_all: false,
        }
    }
}

impl Default for AgentTurn {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// The two seams the loop is generic over
// ---------------------------------------------------------------------------

/// The boxed future shape of one streaming round.
pub type ChatRoundFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ChatReceipt, SeloraError>> + Send + 'a>>;

/// One streaming round against the chat backend (§17.3): a `ChatRequest`
/// with the manifest armed → `ChatEvent`s on `events` (deltas *and*
/// `ToolCallDelta` fragments — §17.6's accumulate-then-parse) → a receipt.
///
/// The production adapter is [`ClientChatStream`] over 5a's
/// `SeloraClient::stream_chat`; tests script it (the mock gateway).
pub trait ChatStream: Send {
    fn stream_round(
        &mut self,
        request: ChatRequest,
        events: mpsc::UnboundedSender<ChatEvent>,
    ) -> ChatRoundFuture<'_>;
}

/// Production chat backend: 5a's `SeloraClient`, unmodified.
pub struct ClientChatStream {
    client: crate::SeloraClient,
}

impl ClientChatStream {
    pub fn new(client: crate::SeloraClient) -> Self {
        Self { client }
    }
}

impl ChatStream for ClientChatStream {
    fn stream_round(
        &mut self,
        request: ChatRequest,
        events: mpsc::UnboundedSender<ChatEvent>,
    ) -> ChatRoundFuture<'_> {
        Box::pin(self.client.stream_chat(request, events))
    }
}

/// The boxed future shapes of the router's two entry points. Deliberately
/// `'static` (they own their inputs): the loop must be able to call
/// [`ToolRunner::abort`] while a run future is still alive.
pub type ToolRunFuture = Pin<Box<dyn Future<Output = ToolOutcome> + Send>>;
pub type ToolApprovedFuture = Pin<Box<dyn Future<Output = ToolResult> + Send>>;

/// The tool backend (5b's router), seen from the loop. The loop never
/// executes tools itself — it only sequences and gates (the Phase 5c
/// contract).
pub trait ToolRunner: Send {
    /// Route/execute one call. Read-type tools resolve immediately
    /// ([`ToolOutcome::Ready`]); side-effecting tools come back as
    /// [`ToolOutcome::NeedsApproval`] and the loop parks.
    fn run(&mut self, call: ToolCall) -> ToolRunFuture;
    /// Execute a call the user just approved — or that
    /// `ApproveAllThisTurn` pre-approves.
    fn run_approved(&mut self, call: ToolCall) -> ToolApprovedFuture;
    /// Kill any running bash child (§17.3's Stop button, §17.5's
    /// timeouts). Must be callable while a `run`/`run_approved` future is
    /// still in flight.
    fn abort(&mut self);
}

/// Production tool backend: 5b's router. A thin adapter — the single
/// merge point for whatever entry points 5b lands.
pub struct RouterToolRunner {
    root: PathBuf,
}

impl RouterToolRunner {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl ToolRunner for RouterToolRunner {
    fn run(&mut self, call: ToolCall) -> ToolRunFuture {
        let root = self.root.clone();
        Box::pin(async move { router::execute_tool(&root, call).await })
    }

    // SEAM(5b): the prep skeleton's router is stateless — it has neither
    // an approved-execution entry point nor a bash abort handle. This
    // adapter is where they plug in at merge time: `run_approved` calls
    // the router's approved path (the loop already collected the user's
    // decision), and `abort` forwards to its abort handle. Until then
    // both routes go through `execute_tool` (failing visibly, never
    // silently) and the loop's cancellation — dropping the in-flight run
    // future — is the stop mechanism.
    fn run_approved(&mut self, call: ToolCall) -> ToolApprovedFuture {
        let root = self.root.clone();
        Box::pin(async move {
            let id = call.id.clone();
            match router::execute_tool(&root, call).await {
                ToolOutcome::Ready(result) => result,
                ToolOutcome::NeedsApproval(_) => ToolResult::error(
                    id,
                    "router did not execute the approved call (Phase 5b seam not yet wired)",
                ),
            }
        })
    }

    fn abort(&mut self) {
        // SEAM(5b): no abort handle exists yet — see `run_approved`. The
        // loop additionally drops any in-flight run future on Stop.
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Everything one turn needs. `conversation` carries the prior agent-mode
/// turns plus the user's new message as the last entry.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnInput {
    pub conversation: Vec<ChatMessage>,
    pub model: Model,
    pub caps: TurnCaps,
    /// The fixed §17.1 manifest. Defaults to `router::tool_manifest()`
    /// (5b's); tests pass their own fixture.
    pub manifest: Vec<ToolSpec>,
}

impl TurnInput {
    /// A turn over `conversation` on `model`, with §17.3's default caps
    /// and the fixed manifest.
    pub fn new(model: Model, conversation: Vec<ChatMessage>) -> Self {
        let caps = TurnCaps::spec_defaults(model.context_window);
        Self {
            conversation,
            model,
            caps,
            manifest: router::tool_manifest(),
        }
    }

    /// Builder-style caps override (the shell maps `settings.agent`
    /// plus the active model onto this).
    pub fn with_caps(mut self, caps: TurnCaps) -> Self {
        self.caps = caps;
        self
    }

    /// Builder-style manifest override.
    pub fn with_manifest(mut self, manifest: Vec<ToolSpec>) -> Self {
        self.manifest = manifest;
        self
    }
}

// ---------------------------------------------------------------------------
// Streaming tool-call accumulation (§17.6)
// ---------------------------------------------------------------------------

/// Accumulates streaming `ChatEvent::ToolCallDelta` fragments into
/// complete [`ToolCall`]s — the same accumulate-then-parse pattern as
/// content deltas. `index` identifies which call of the response is
/// growing; `id`/`name` ride only the first fragment.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: Vec<ToolCall>,
}

impl ToolCallAccumulator {
    /// Merge one fragment into slot `index`.
    pub fn push(
        &mut self,
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: &str,
    ) {
        if self.calls.len() <= index {
            self.calls
                .resize_with(index + 1, || ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                });
        }
        let slot = &mut self.calls[index];
        if let Some(id) = id {
            slot.id = id;
        }
        if let Some(name) = name {
            slot.name = name;
        }
        slot.arguments.push_str(arguments);
    }

    /// The completed calls in index order; slots that never received a
    /// fragment are dropped.
    pub fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_iter()
            .filter(|call| {
                !call.id.is_empty() || !call.name.is_empty() || !call.arguments.is_empty()
            })
            .collect()
    }
}

/// Chars/4 token estimate over the conversation (the same fallback the
/// client's receipts use, RISKS.md §3) — coarse, but it keeps the
/// context cap honest without shipping a tokenizer.
fn estimate_conversation_tokens(conversation: &[ChatMessage]) -> u64 {
    conversation
        .iter()
        .map(|message| {
            let mut chars = message.content.chars().count();
            if let Some(calls) = &message.tool_calls {
                for call in calls {
                    chars += call.name.chars().count() + call.arguments.chars().count() + 8;
                }
            }
            (chars / 4) as u64
        })
        .sum()
}

// ---------------------------------------------------------------------------
// The gated awaits (Stop + the turn deadline race everything)
// ---------------------------------------------------------------------------

/// How a gated await ended.
#[derive(Debug, Clone, PartialEq)]
enum Gated<T> {
    /// The awaited future resolved normally.
    Done(T),
    /// The user pressed Stop. The runner was aborted; the future was
    /// given its grace period and its outcome discarded — the turn ends
    /// regardless.
    Stopped,
    /// The turn's wall-clock cap fired.
    TimeUp,
}

/// A stand-in deadline for "no cap": a year out is beyond any turn (the
/// caps themselves are bounded by `max_turn_secs`). `tokio`'s private
/// `Instant::far_future` is not reachable from here.
fn far_future() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(365 * 24 * 60 * 60)
}

/// Which abort path fired inside [`gated`].
enum AbortReason {
    Stop,
    Deadline,
}

/// Await `fut` while watching the Stop decisions and the turn deadline.
/// On Stop or the deadline the runner is aborted and the future gets
/// [`ABORT_GRACE`] to settle (a cooperating runner kills its child and
/// returns); past the grace it is simply dropped.
async fn gated<T, F>(
    fut: F,
    tools: &mut dyn ToolRunner,
    decisions: &mut mpsc::UnboundedReceiver<TurnDecision>,
    deadline: Option<tokio::time::Instant>,
) -> Gated<T>
where
    F: Future<Output = T> + Send + Unpin,
{
    let mut fut = fut;
    let mut decisions_open = true;
    let sleep =
        tokio::time::sleep_until(deadline.unwrap_or_else(|| far_future()));
    tokio::pin!(sleep);

    let reason = loop {
        tokio::select! {
            biased;
            decision = decisions.recv(), if decisions_open => match decision {
                Some((_, AgentDecision::Stop)) => break AbortReason::Stop,
                // A decision while no card is pending is not ours to apply
                // (the UI only enables card buttons while we are parked).
                Some(_) => {}
                None => { decisions_open = false; }
            },
            _ = &mut sleep, if deadline.is_some() => break AbortReason::Deadline,
            outcome = &mut fut => return Gated::Done(outcome),
        }
    };

    // The turn is over: abort the runner (the bash-child kill, §17.3) and
    // give it a grace period to clean up. The settled outcome is never
    // used — the loop ends with Stopped/TimeUp either way.
    tools.abort();
    let _ = tokio::time::timeout(ABORT_GRACE, &mut fut).await;

    match reason {
        AbortReason::Stop => Gated::Stopped,
        AbortReason::Deadline => Gated::TimeUp,
    }
}

/// How the approval park ended.
enum Parked {
    /// The user answered the pending card.
    Decided(AgentDecision),
    /// Stop (any card it names, or the Stop button's nil id).
    Stop,
    /// The turn's wall-clock cap fired while parked.
    TimeUp,
    /// The decision channel closed — the UI is gone; waiting is pointless.
    ChannelClosed,
}

/// The loop is parked on an approval card (§17.4): wait for the user's
/// decision, the turn deadline, or Stop. Decisions for *other* (stale)
/// cards are ignored.
async fn park_for_decision(
    decisions: &mut mpsc::UnboundedReceiver<TurnDecision>,
    approval_id: ApprovalId,
    deadline: Option<tokio::time::Instant>,
) -> Parked {
    let sleep =
        tokio::time::sleep_until(deadline.unwrap_or_else(|| far_future()));
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            biased;
            decision = decisions.recv() => match decision {
                Some((id, decision)) => {
                    // Stop applies regardless of which card it names.
                    if decision == AgentDecision::Stop {
                        return Parked::Stop;
                    }
                    // Only the pending card's decision unparks.
                    if id == approval_id {
                        return Parked::Decided(decision);
                    }
                }
                // The UI is gone: no decision can ever arrive.
                None => return Parked::ChannelClosed,
            },
            _ = &mut sleep, if deadline.is_some() => return Parked::TimeUp,
        }
    }
}

// ---------------------------------------------------------------------------
// One streaming round
// ---------------------------------------------------------------------------

/// How a streaming round ended.
enum StreamEnd {
    Receipt(Result<ChatReceipt, SeloraError>),
    Stopped,
    TimeUp,
}

/// What one streaming round produced.
struct RoundOutcome {
    end: StreamEnd,
    /// The round's content deltas, concatenated.
    text: String,
    /// The round's accumulated tool calls (empty → the round was a final
    /// answer, i.e. `finish_reason == "stop"`).
    tool_calls: Vec<ToolCall>,
    /// `true` when the UI dropped the event channel mid-stream.
    ui_gone: bool,
}

/// Absorb one `ChatEvent` into the round's state, forwarding deltas to
/// the UI.
fn absorb(
    event: ChatEvent,
    accumulator: &mut ToolCallAccumulator,
    text: &mut String,
    events: &mpsc::UnboundedSender<AgentEvent>,
    ui_gone: &mut bool,
) {
    match event {
        ChatEvent::Delta(delta) => {
            text.push_str(&delta);
            if events.send(AgentEvent::Delta(delta)).is_err() {
                *ui_gone = true;
            }
        }
        ChatEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments,
        } => accumulator.push(index, id, name, &arguments),
        // The receipt/error arrives on the stream's return value; the
        // terminal channel events are redundant here.
        ChatEvent::Done(_) | ChatEvent::Error(_) => {}
    }
}

/// Stream one round while gated on Stop and the turn deadline. Deltas and
/// tool-call fragments are absorbed (deltas forwarded live); on Stop or
/// the deadline the stream future is dropped, which cancels the in-flight
/// request.
async fn stream_round_gated<C: ChatStream>(
    chat: &mut C,
    request: ChatRequest,
    events: &mpsc::UnboundedSender<AgentEvent>,
    decisions: &mut mpsc::UnboundedReceiver<TurnDecision>,
    deadline: Option<tokio::time::Instant>,
) -> RoundOutcome {
    let (tx, mut rx) = mpsc::unbounded_channel::<ChatEvent>();
    let mut fut = chat.stream_round(request, tx);
    let mut accumulator = ToolCallAccumulator::default();
    let mut text = String::new();
    let mut rx_open = true;
    let mut decisions_open = true;
    let mut ui_gone = false;
    let sleep =
        tokio::time::sleep_until(deadline.unwrap_or_else(|| far_future()));
    tokio::pin!(sleep);

    let end = loop {
        tokio::select! {
            biased;
            event = rx.recv(), if rx_open => match event {
                Some(event) => absorb(event, &mut accumulator, &mut text, events, &mut ui_gone),
                None => { rx_open = false; }
            },
            decision = decisions.recv(), if decisions_open => match decision {
                Some((_, AgentDecision::Stop)) => break StreamEnd::Stopped,
                Some(_) => {}
                None => { decisions_open = false; }
            },
            _ = &mut sleep, if deadline.is_some() => break StreamEnd::TimeUp,
            receipt = &mut fut => break StreamEnd::Receipt(receipt),
        }
    };

    // The receipt arm can win while the round's last events are still
    // buffered: drain before consulting the accumulator.
    if matches!(end, StreamEnd::Receipt(_)) {
        while let Ok(event) = rx.try_recv() {
            absorb(event, &mut accumulator, &mut text, events, &mut ui_gone);
        }
    }

    RoundOutcome {
        end,
        text,
        tool_calls: accumulator.finish(),
        ui_gone,
    }
}

/// Send one event; `false` means the UI dropped the channel (the turn
/// should end rather than stream into the void).
fn emit(events: &mpsc::UnboundedSender<AgentEvent>, event: AgentEvent) -> bool {
    events.send(event).is_ok()
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

/// Run one agent turn to its end (answer, cap, or user stop) and return
/// the report. Also emits the full [`AgentEvent`] stream on `events` and
/// ends with [`AgentEvent::TurnDone`].
///
/// The conversation grows in place: each tool-call round appends the
/// assistant message (with `tool_calls`) and one tool-result message per
/// call; the next round's request carries all of it (§17.3).
pub async fn run_agent_turn<C, R>(
    input: TurnInput,
    mut chat: C,
    mut tools: R,
    events: mpsc::UnboundedSender<AgentEvent>,
    mut decisions: mpsc::UnboundedReceiver<TurnDecision>,
) -> AgentTurnReport
where
    C: ChatStream,
    R: ToolRunner,
{
    let caps = input.caps.clone();
    let model = input.model.clone();
    let deadline = if caps.max_turn_secs == 0 {
        None
    } else {
        Some(tokio::time::Instant::now() + Duration::from_secs(caps.max_turn_secs))
    };

    let mut turn = AgentTurn::new();
    let mut conversation = input.conversation.clone();
    let mut final_message = String::new();
    let end: Option<TurnEnd>;
    let mut ui_gone = false;

    'turn: loop {
        // --- the caps, checked before every round ------------------------
        if let Some(budget) = caps.context_budget_tokens() {
            let estimated = estimate_conversation_tokens(&conversation);
            if estimated > budget {
                end = Some(TurnEnd::ContextOverflow {
                    estimated_tokens: estimated,
                    budget_tokens: budget,
                });
                break 'turn;
            }
        }
        if deadline.is_some_and(|deadline| deadline <= tokio::time::Instant::now()) {
            end = Some(TurnEnd::TimeCap {
                limit_secs: caps.max_turn_secs,
            });
            break 'turn;
        }
        if ui_gone {
            end = Some(TurnEnd::Failed("agent event receiver dropped".into()));
            break 'turn;
        }

        // --- one streaming round (§17.3) --------------------------------
        turn.state = AgentTurnState::Thinking;
        if !emit(&events, AgentEvent::State(turn.state.clone())) {
            end = Some(TurnEnd::Failed("agent event receiver dropped".into()));
            break 'turn;
        }
        let request = ChatRequest::new(model.id.clone(), conversation.clone())
            .with_tools(input.manifest.clone());
        let round = stream_round_gated(&mut chat, request, &events, &mut decisions, deadline).await;
        ui_gone |= round.ui_gone;

        let receipt = match round.end {
            StreamEnd::Stopped => {
                if !round.text.is_empty() {
                    final_message = round.text.clone();
                }
                end = Some(TurnEnd::StoppedByUser);
                break 'turn;
            }
            StreamEnd::TimeUp => {
                if !round.text.is_empty() {
                    final_message = round.text.clone();
                }
                end = Some(TurnEnd::TimeCap {
                    limit_secs: caps.max_turn_secs,
                });
                break 'turn;
            }
            StreamEnd::Receipt(Err(error)) => {
                if !round.text.is_empty() {
                    final_message = round.text.clone();
                }
                end = Some(TurnEnd::Failed(error.to_string()));
                break 'turn;
            }
            StreamEnd::Receipt(Ok(receipt)) => receipt,
        };

        // --- bookkeeping from the receipt --------------------------------
        turn.iterations += 1;
        turn.tokens += u64::from(receipt.prompt_tokens) + u64::from(receipt.completion_tokens);
        let round_cost = if receipt.cost_usd > 0.0 {
            receipt.cost_usd
        } else {
            // The gateway did not report cost: fall back to the model's
            // price table — the same estimate the cost meter uses.
            compute_cost_usd(&model, receipt.prompt_tokens, receipt.completion_tokens)
        };
        turn.cost_usd += round_cost;
        if !round.text.trim().is_empty() {
            final_message = round.text.clone();
        }

        // --- a tool-free answer ends the turn ----------------------------
        if round.tool_calls.is_empty() {
            conversation.push(ChatMessage::assistant(round.text));
            end = Some(TurnEnd::Completed);
            break 'turn;
        }

        // The cost cap aborts the loop before this round's tools run; a
        // completed final answer (above) never trips it.
        if turn.cost_usd >= caps.max_cost_usd {
            end = Some(TurnEnd::CostCap {
                limit_usd: caps.max_cost_usd,
            });
            break 'turn;
        }

        let mut assistant = ChatMessage::assistant(round.text);
        assistant.tool_calls = Some(round.tool_calls.clone());
        conversation.push(assistant);

        // --- execute the calls in order (§17.3) --------------------------
        for call in round.tool_calls {
            if turn.tools_used >= caps.max_tool_iterations {
                end = Some(TurnEnd::IterationCap {
                    limit: caps.max_tool_iterations,
                });
                break 'turn;
            }

            turn.state = AgentTurnState::RunningTool(call.name.clone());
            if !emit(&events, AgentEvent::State(turn.state.clone()))
                || !emit(&events, AgentEvent::ToolCallStarted(call.clone()))
            {
                end = Some(TurnEnd::Failed("agent event receiver dropped".into()));
                break 'turn;
            }

            // Route through 5b's router: Ready, or a card to park on.
            let mut result: Option<ToolResult> = None;
            match gated(
                tools.run(call.clone()),
                &mut tools,
                &mut decisions,
                deadline,
            )
            .await
            {
                Gated::Stopped => {
                    end = Some(TurnEnd::StoppedByUser);
                    break 'turn;
                }
                Gated::TimeUp => {
                    end = Some(TurnEnd::TimeCap {
                        limit_secs: caps.max_turn_secs,
                    });
                    break 'turn;
                }
                Gated::Done(ToolOutcome::Ready(ready)) => result = Some(ready),
                Gated::Done(ToolOutcome::NeedsApproval(card)) => {
                    if turn.approve_all {
                        // §17.4: ApproveAllThisTurn pre-approves the rest
                        // of this turn's write/bash cards.
                        turn.approved += 1;
                    } else {
                        let approval_id = Uuid::new_v4();
                        turn.state = AgentTurnState::AwaitingApproval(approval_id);
                        if !emit(&events, AgentEvent::State(turn.state.clone()))
                            || !emit(
                                &events,
                                AgentEvent::ApprovalNeeded {
                                    approval_id,
                                    card,
                                },
                            )
                        {
                            end = Some(TurnEnd::Failed("agent event receiver dropped".into()));
                            break 'turn;
                        }
                        match park_for_decision(&mut decisions, approval_id, deadline).await {
                            Parked::Stop => {
                                end = Some(TurnEnd::StoppedByUser);
                                break 'turn;
                            }
                            Parked::TimeUp => {
                                end = Some(TurnEnd::TimeCap {
                                    limit_secs: caps.max_turn_secs,
                                });
                                break 'turn;
                            }
                            Parked::ChannelClosed => {
                                end = Some(TurnEnd::Failed(
                                    "agent decision channel closed".into(),
                                ));
                                break 'turn;
                            }
                            Parked::Decided(AgentDecision::Approve) => turn.approved += 1,
                            Parked::Decided(AgentDecision::ApproveAllThisTurn) => {
                                turn.approved += 1;
                                turn.approve_all = true;
                            }
                            Parked::Decided(AgentDecision::Deny) => {
                                turn.denied += 1;
                                result =
                                    Some(ToolResult::error(call.id.clone(), DENIED_MESSAGE));
                            }
                            // Defensive: park never returns Stop here (it
                            // is handled before the id match) — but if it
                            // ever did, stopping is the safe reading.
                            Parked::Decided(AgentDecision::Stop) => {
                                end = Some(TurnEnd::StoppedByUser);
                                break 'turn;
                            }
                        }
                    }
                }
            }

            // Approved (or pre-approved): execute the side effect.
            let result = match result {
                Some(result) => result,
                None => {
                    turn.state = AgentTurnState::RunningTool(call.name.clone());
                    let _ = emit(&events, AgentEvent::State(turn.state.clone()));
                    match gated(
                        tools.run_approved(call.clone()),
                        &mut tools,
                        &mut decisions,
                        deadline,
                    )
                    .await
                    {
                        Gated::Stopped => {
                            end = Some(TurnEnd::StoppedByUser);
                            break 'turn;
                        }
                        Gated::TimeUp => {
                            end = Some(TurnEnd::TimeCap {
                                limit_secs: caps.max_turn_secs,
                            });
                            break 'turn;
                        }
                        Gated::Done(result) => result,
                    }
                }
            };

            turn.tools_used += 1;
            conversation.push(ChatMessage::tool_result(call.id, result.content.clone()));
            let _ = emit(&events, AgentEvent::ToolResult(result));
        }
        // The batch finished: loop with the grown conversation.
    }

    // --- the report --------------------------------------------------------
    turn.state = AgentTurnState::Done;
    let _ = emit(&events, AgentEvent::State(AgentTurnState::Done));
    let end = end.unwrap_or_else(|| {
        // Every exit sets an end; this is a belt-and-braces default that
        // fails visibly instead of reporting a phantom success.
        TurnEnd::Failed("the loop exited without an end state".into())
    });
    let summary = AgentSummary {
        iterations: turn.iterations,
        tools_used: turn.tools_used,
        approved: turn.approved,
        denied: turn.denied,
        tokens: turn.tokens,
        cost_usd: turn.cost_usd,
        completed: end.is_completed(),
    };
    let report = AgentTurnReport {
        summary,
        end: end.clone(),
        final_message,
        conversation,
    };
    let _ = emit(&events, AgentEvent::TurnDone(report.clone()));
    report
}

/// Spawn the loop as its own task (the runtime-side pattern of the rest
/// of the app, §1.4): the shell keeps the decision sender (fed from
/// `Effect::AgentDecision`) and the join handle.
pub fn spawn_agent_turn<C, R>(
    handle: &tokio::runtime::Handle,
    input: TurnInput,
    chat: C,
    tools: R,
    events: mpsc::UnboundedSender<AgentEvent>,
) -> (
    mpsc::UnboundedSender<TurnDecision>,
    tokio::task::JoinHandle<AgentTurnReport>,
)
where
    C: ChatStream + 'static,
    R: ToolRunner + 'static,
{
    let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
    let join = handle.spawn(run_agent_turn(input, chat, tools, events, decision_rx));
    (decision_tx, join)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use selora_settings::AgentSettings;

    use crate::chat_types::ToolCallId;
    use crate::types::Role;

    const TEST_MODEL: &str = "test-model";

    // -----------------------------------------------------------------------
    // Fixtures
    // -----------------------------------------------------------------------

    fn model() -> Model {
        Model {
            id: TEST_MODEL.into(),
            display_name: "Test Model".into(),
            context_window: 200_000,
            price_per_1k_input: 0.010,
            price_per_1k_output: 0.030,
            supports_streaming: true,
            supports_tools: true,
            family: "test".into(),
        }
    }

    fn manifest() -> Vec<ToolSpec> {
        vec![
            ToolSpec::new("read_file", "Read a workspace file", serde_json::json!({"type": "object"})),
            ToolSpec::new("bash", "Run a shell command", serde_json::json!({"type": "object"})),
        ]
    }

    fn caps() -> TurnCaps {
        TurnCaps::spec_defaults(200_000)
    }

    fn receipt(cost_usd: f64, prompt_tokens: u32, completion_tokens: u32) -> ChatReceipt {
        ChatReceipt {
            request_id: Uuid::new_v4(),
            model: TEST_MODEL.into(),
            prompt_tokens,
            completion_tokens,
            cost_usd,
            duration_ms: 1,
            estimated: cost_usd <= 0.0,
        }
    }

    fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    fn ready_result(id: &str, content: &str) -> MockTool {
        MockTool::Ready(ToolResult {
            id: id.into(),
            ok: true,
            content: content.into(),
        })
    }

    fn diff_card(path: &str) -> ApprovalCard {
        ApprovalCard::Diff {
            path: path.into(),
            diff: format!("+fix {path}"),
        }
    }

    fn conversation() -> Vec<ChatMessage> {
        vec![ChatMessage::user("read src/main.rs and fix it")]
    }

    fn text_delta(text: &str) -> ChatEvent {
        ChatEvent::Delta(text.into())
    }

    fn tool_delta(index: usize, id: Option<&str>, name: Option<&str>, arguments: &str) -> ChatEvent {
        ChatEvent::ToolCallDelta {
            index,
            id: id.map(str::to_owned),
            name: name.map(str::to_owned),
            arguments: arguments.into(),
        }
    }

    /// A round whose response calls tools: one text delta, then each call
    /// streaming as two argument fragments (§17.6 accumulate-then-parse).
    fn tool_round(calls: &[(&str, &str, &str)], text: &str, cost_usd: f64) -> ScriptedRound {
        let mut events = Vec::new();
        if !text.is_empty() {
            events.push(text_delta(text));
        }
        for (index, (id, name, arguments)) in calls.iter().enumerate() {
            let split = arguments.len() / 2;
            events.push(tool_delta(index, Some(id), Some(name), &arguments[..split]));
            events.push(tool_delta(index, None, None, &arguments[split..]));
        }
        let round_receipt = receipt(cost_usd, 100, 50);
        // The terminal channel event mirrors what the real client sends.
        events.push(ChatEvent::Done(round_receipt.clone()));
        ScriptedRound {
            events,
            receipt: round_receipt,
            delay: None,
        }
    }

    /// A round whose response is a final answer (no tool calls).
    fn answer_round(text: &str, cost_usd: f64) -> ScriptedRound {
        let round_receipt = receipt(cost_usd, 100, 50);
        ScriptedRound {
            events: vec![text_delta(text), ChatEvent::Done(round_receipt.clone())],
            receipt: round_receipt,
            delay: None,
        }
    }

    // -----------------------------------------------------------------------
    // The mock gateway (ChatStream)
    // -----------------------------------------------------------------------

    struct ScriptedRound {
        events: Vec<ChatEvent>,
        receipt: ChatReceipt,
        delay: Option<Duration>,
    }

    /// The mock gateway: one [`ScriptedRound`] per request, every request
    /// recorded (so tests assert the conversation's growth).
    struct ScriptedChat {
        rounds: VecDeque<ScriptedRound>,
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }

    impl ScriptedChat {
        fn new(rounds: Vec<ScriptedRound>) -> (Self, Arc<Mutex<Vec<ChatRequest>>>) {
            let requests = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    rounds: rounds.into(),
                    requests: requests.clone(),
                },
                requests,
            )
        }
    }

    impl ChatStream for ScriptedChat {
        fn stream_round(
            &mut self,
            request: ChatRequest,
            events: mpsc::UnboundedSender<ChatEvent>,
        ) -> ChatRoundFuture<'_> {
            let round = self
                .rounds
                .pop_front()
                .expect("script exhausted before the turn ended");
            let requests = self.requests.clone();
            Box::pin(async move {
                requests.lock().expect("requests lock").push(request);
                if let Some(delay) = round.delay {
                    tokio::time::sleep(delay).await;
                }
                for event in round.events {
                    let _ = events.send(event);
                }
                Ok(round.receipt)
            })
        }
    }

    // -----------------------------------------------------------------------
    // The mock router (ToolRunner)
    // -----------------------------------------------------------------------

    /// What the mock router does with one call, keyed by call id.
    #[derive(Clone)]
    enum MockTool {
        /// Read-type: routes straight to a ready result.
        Ready(ToolResult),
        /// Side-effecting: routes to a card; executing the approval
        /// returns `approved`.
        Card {
            card: ApprovalCard,
            approved: ToolResult,
        },
        /// Side-effecting and slow: executing the approval spawns a real
        /// 60-second child and waits on it — the Stop-mid-tool case.
        Slow {
            card: ApprovalCard,
            approved: ToolResult,
        },
    }

    #[derive(Default)]
    struct MockRunnerState {
        routed: Vec<ToolCallId>,
        approved_runs: Vec<ToolCallId>,
        abort_called: bool,
        child_spawned: bool,
        child_killed: bool,
    }

    /// The mock router: 5b's `execute_tool`, scripted per call id.
    struct MockRunner {
        script: HashMap<ToolCallId, MockTool>,
        state: Arc<Mutex<MockRunnerState>>,
        abort_flag: Arc<tokio::sync::Notify>,
    }

    impl MockRunner {
        fn new<S: Into<String>>(script: Vec<(S, MockTool)>) -> (Self, Arc<Mutex<MockRunnerState>>) {
            let state = Arc::new(Mutex::new(MockRunnerState::default()));
            let abort_flag = Arc::new(tokio::sync::Notify::new());
            (
                Self {
                    script: script
                        .into_iter()
                        .map(|(id, tool)| (id.into(), tool))
                        .collect(),
                    state: state.clone(),
                    abort_flag,
                },
                state,
            )
        }
    }

    impl ToolRunner for MockRunner {
        fn run(&mut self, call: ToolCall) -> ToolRunFuture {
            let tool = self
                .script
                .get(&call.id)
                .expect("mock router: unscripted call")
                .clone();
            let state = self.state.clone();
            Box::pin(async move {
                state.lock().expect("state lock").routed.push(call.id);
                match tool {
                    MockTool::Ready(result) => ToolOutcome::Ready(result),
                    MockTool::Card { card, .. } | MockTool::Slow { card, .. } => {
                        ToolOutcome::NeedsApproval(card)
                    }
                }
            })
        }

        fn run_approved(&mut self, call: ToolCall) -> ToolApprovedFuture {
            let tool = self
                .script
                .get(&call.id)
                .expect("mock router: unscripted call")
                .clone();
            let state = self.state.clone();
            let abort_flag = self.abort_flag.clone();
            Box::pin(async move {
                state
                    .lock()
                    .expect("state lock")
                    .approved_runs
                    .push(call.id.clone());
                match tool {
                    MockTool::Ready(_) => {
                        ToolResult::error(call.id, "mock: read tool needs no approval")
                    }
                    MockTool::Card { approved, .. } => approved,
                    MockTool::Slow { approved, .. } => {
                        // A real child the Stop button must kill (§17.3).
                        let mut child = slow_child();
                        state.lock().expect("state lock").child_spawned = true;
                        tokio::select! {
                            exited = child.wait() => {
                                let _ = exited;
                            }
                            _ = abort_flag.notified() => {
                                child.start_kill().expect("kill the slow child");
                                // Reap: the kill is real only if the child exits.
                                let _ = child.wait().await;
                                let mut guard = state.lock().expect("state lock");
                                guard.child_killed = true;
                            }
                        }
                        approved
                    }
                }
            })
        }

        fn abort(&mut self) {
            self.state.lock().expect("state lock").abort_called = true;
            self.abort_flag.notify_waiters();
        }
    }

    /// A real long-running child (`ping` on Windows, `sleep` elsewhere) —
    /// the bash tool's stand-in.
    fn slow_child() -> tokio::process::Child {
        #[cfg(windows)]
        let mut command = {
            let mut command = tokio::process::Command::new("ping");
            command.arg("-n").arg("60").arg("127.0.0.1");
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = tokio::process::Command::new("sleep");
            command.arg("60");
            command
        };
        // The mock never reads its output; null the pipes so the child
        // never blocks on a full one.
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.spawn().expect("spawn the slow child")
    }

    // -----------------------------------------------------------------------
    // Event-channel helpers
    // -----------------------------------------------------------------------

    /// Wait for the next approval card and return its id.
    async fn wait_for_approval(rx: &mut mpsc::UnboundedReceiver<AgentEvent>) -> ApprovalId {
        loop {
            match rx.recv().await.expect("events channel open") {
                AgentEvent::ApprovalNeeded { approval_id, .. } => return approval_id,
                _ => continue,
            }
        }
    }

    /// Wait for the state event proving the approved tool is in flight.
    async fn wait_for_running_tool(rx: &mut mpsc::UnboundedReceiver<AgentEvent>) {
        loop {
            match rx.recv().await.expect("events channel open") {
                AgentEvent::State(AgentTurnState::RunningTool(_)) => return,
                _ => continue,
            }
        }
    }

    fn drain_events(rx: &mut mpsc::UnboundedReceiver<AgentEvent>) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    fn count<'a, I>(events: I, predicate: impl Fn(&AgentEvent) -> bool) -> usize
    where
        I: IntoIterator<Item = &'a AgentEvent>,
    {
        events.into_iter().filter(|event| predicate(event)).count()
    }

    /// Run a turn to completion on the current task (tests that never
    /// need to send decisions concurrently).
    async fn run_turn(
        chat: ScriptedChat,
        runner: MockRunner,
        input: TurnInput,
    ) -> (AgentTurnReport, mpsc::UnboundedReceiver<AgentEvent>) {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (_decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let report = run_agent_turn(input, chat, runner, events_tx, decision_rx).await;
        (report, events_rx)
    }

    // -----------------------------------------------------------------------
    // Scenario 1: a 3-tool scripted conversation completes
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn three_tool_scripted_conversation_completes() {
        // Round 1: two read-type calls in one batch (executed in order).
        // Round 2: one write — card, user approves, executes.
        // Round 3: the final answer.
        let (chat, requests) = ScriptedChat::new(vec![
            tool_round(
                &[
                    ("call_read", "read_file", r#"{"path":"src/main.rs"}"#),
                    ("call_list", "list_dir", r#"{"path":"src"}"#),
                ],
                "Reading the workspace first.",
                0.10,
            ),
            tool_round(
                &[(
                    "call_write",
                    "write_file",
                    r#"{"path":"src/main.rs","content":"fn main() {}"}"#,
                )],
                "Now writing the fix.",
                0.20,
            ),
            answer_round("Fixed: src/main.rs now has the comment.", 0.05),
        ]);
        let (runner, runner_state) = MockRunner::new(vec![
            ("call_read", ready_result("call_read", "fn main() {}")),
            ("call_list", ready_result("call_list", "src, tests")),
            (
                "call_write",
                MockTool::Card {
                    card: diff_card("src/main.rs"),
                    approved: ToolResult {
                        id: "call_write".into(),
                        ok: true,
                        content: "written".into(),
                    },
                },
            ),
        ]);

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(
            input,
            chat,
            runner,
            events_tx,
            decision_rx,
        ));

        // The user approves the write card when it appears.
        let approval_id = wait_for_approval(&mut events_rx).await;
        decision_tx
            .send((approval_id, AgentDecision::Approve))
            .expect("send decision");

        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.final_message, "Fixed: src/main.rs now has the comment.");
        assert!(report.summary.completed);
        assert_eq!(report.summary.iterations, 3);
        assert_eq!(report.summary.tools_used, 3);
        assert_eq!(report.summary.approved, 1);
        assert_eq!(report.summary.denied, 0);
        assert!((report.summary.cost_usd - 0.35).abs() < 1e-9);
        assert_eq!(report.summary.tokens, 3 * 150);

        // The conversation grew exactly per §17.3: the assistant's
        // tool-call message, then one tool result per call, in order.
        let requests = requests.lock().expect("requests lock").clone();
        assert_eq!(requests.len(), 3);
        assert!(requests
            .iter()
            .all(|req| req.tools.as_ref().is_some_and(|tools| !tools.is_empty())));
        let second = &requests[1].messages;
        assert_eq!(second.len(), 4); // user + assistant(tool_calls×2) + tool×2
        assert!(second[1]
            .tool_calls
            .as_ref()
            .is_some_and(|calls| calls.len() == 2));
        assert_eq!(second[2].tool_call_id.as_deref(), Some("call_read"));
        assert_eq!(second[3].tool_call_id.as_deref(), Some("call_list"));
        let third = &requests[2].messages;
        assert_eq!(third.len(), 6); // + assistant(tool_calls×1) + tool×1
        assert_eq!(third[4].tool_call_id.as_deref(), Some("call_write"));

        // The report's conversation is the full transcript.
        assert_eq!(report.conversation.len(), 7);

        // The write executed exactly once, after the approval.
        let state = runner_state.lock().expect("state lock");
        assert_eq!(state.routed, ["call_read", "call_list", "call_write"]);
        assert_eq!(state.approved_runs, ["call_write"]);

        // The event stream: three tool starts, three results, one card,
        // one TurnDone, and the final state is Done.
        let events = drain_events(&mut events_rx);
        assert_eq!(count(&events, |event| matches!(
            event,
            AgentEvent::ToolCallStarted(_)
        )), 3);
        assert_eq!(count(&events, |event| matches!(
            event,
            AgentEvent::ToolResult(_)
        )), 3);
        assert_eq!(
            count(&events, |event| matches!(
                event,
                AgentEvent::ApprovalNeeded { .. }
            )),
            1
        );
        assert_eq!(
            count(&events, |event| matches!(event, AgentEvent::TurnDone(_))),
            1
        );
        assert_eq!(count(&events, |event| *event
            == AgentEvent::State(AgentTurnState::Done)), 1);
    }

    // -----------------------------------------------------------------------
    // Scenario 2: a cap-exceeded turn aborts
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn iteration_cap_from_settings_aborts_the_turn() {
        // The model never stops asking for tools; the settings cap is 3.
        let mut settings = AgentSettings::default();
        settings.max_iterations = 3;
        // The shell's mapping: settings.agent.* + the model's window.
        let caps = TurnCaps {
            max_tool_iterations: settings.max_iterations,
            max_turn_secs: settings.max_turn_secs,
            max_cost_usd: settings.max_cost_usd,
            context_window: 200_000,
            context_reserve: CONTEXT_RESERVE,
        };
        assert_eq!(caps.max_tool_iterations, 3);

        // An endless tool loop: each round asks for one more read.
        let mut script = Vec::new();
        let mut rounds = Vec::new();
        for n in 0..10 {
            let id = format!("call_{n}");
            let arguments = format!(r#"{{"n":{n}}}"#);
            script.push((
                id.clone(),
                ready_result(&id, &format!("file {n}")),
            ));
            let split = arguments.len() / 2;
            rounds.push(ScriptedRound {
                events: vec![
                    tool_delta(0, Some(&id), Some("read_file"), &arguments[..split]),
                    tool_delta(0, None, None, &arguments[split..]),
                ],
                receipt: receipt(0.01, 100, 50),
                delay: None,
            });
        }
        let (chat, requests) = ScriptedChat::new(rounds);
        let (runner, _state) = MockRunner::new(script);

        let input = TurnInput::new(model(), conversation())
            .with_caps(caps)
            .with_manifest(manifest());
        let (report, mut events_rx) = run_turn(chat, runner, input).await;

        assert!(
            matches!(&report.end, TurnEnd::IterationCap { limit } if *limit == 3),
            "got: {:?}",
            report.end
        );
        assert!(!report.summary.completed);
        assert_eq!(report.summary.tools_used, 3);
        assert_eq!(report.summary.iterations, 4, "the 4th round is the one capped");
        assert!(report.end.toast().contains("iteration cap"));

        // The turn was aborted, not run to the script's end — and the
        // TurnDone event (the shell's toast trigger) still arrived.
        assert_eq!(requests.lock().expect("requests lock").len(), 4);
        let events = drain_events(&mut events_rx);
        assert_eq!(
            count(&events, |event| matches!(event, AgentEvent::TurnDone(_))),
            1
        );
    }

    // -----------------------------------------------------------------------
    // Scenario 3: a Stop mid-tool aborts and kills the child
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn stop_mid_tool_ends_the_turn_and_kills_the_child() {
        // Round 1: one bash call → command card → user approves → a real
        // 60s child starts. The Stop decision arrives while it runs: the
        // loop invokes the router's abort handle, the child is killed for
        // real, and the turn ends "stopped by user" (within the grace
        // period, not the child's 60s).
        let (chat, _requests) = ScriptedChat::new(vec![tool_round(
            &[(
                "call_bash",
                "bash",
                r#"{"command":"ping -n 60 127.0.0.1"}"#,
            )],
            "Running it.",
            0.10,
        )]);
        let (runner, runner_state) = MockRunner::new(vec![(
            "call_bash",
            MockTool::Slow {
                card: ApprovalCard::Command {
                    command: "ping -n 60 127.0.0.1".into(),
                },
                approved: ToolResult {
                    id: "call_bash".into(),
                    ok: true,
                    content: "done".into(),
                },
            },
        )]);

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(
            input,
            chat,
            runner,
            events_tx,
            decision_rx,
        ));

        let approval_id = wait_for_approval(&mut events_rx).await;
        decision_tx
            .send((approval_id, AgentDecision::Approve))
            .expect("approve");
        // The state event proves the approved tool is in flight (the
        // child is spawned inside its run future).
        wait_for_running_tool(&mut events_rx).await;
        // Stop: the nil approval id — the Stop button names no card.
        decision_tx
            .send((Uuid::nil(), AgentDecision::Stop))
            .expect("stop");

        let started = std::time::Instant::now();
        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::StoppedByUser);
        assert!(!report.summary.completed);
        assert_eq!(report.end.toast(), "agent turn stopped by user");
        // §17.3: the Stop button ends the turn within ~1s, not the
        // child's remaining 60.
        assert!(started.elapsed() < Duration::from_secs(5));

        let state = runner_state.lock().expect("state lock");
        assert!(state.child_spawned, "the child was running");
        assert!(state.abort_called, "the router's abort handle was invoked");
        assert!(
            state.child_killed,
            "the running child was killed (and reaped), not orphaned"
        );
    }

    // -----------------------------------------------------------------------
    // Scenario 4: a Deny continues the loop
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn deny_continues_the_loop_with_a_user_denied_tool_result() {
        let (chat, requests) = ScriptedChat::new(vec![
            tool_round(
                &[(
                    "call_write",
                    "write_file",
                    r#"{"path":"src/main.rs","content":"x"}"#,
                )],
                "I'll write it.",
                0.10,
            ),
            answer_round("Alright, I left the file alone.", 0.05),
        ]);
        let (runner, runner_state) = MockRunner::new(vec![(
            "call_write",
            MockTool::Card {
                card: diff_card("src/main.rs"),
                approved: ToolResult {
                    id: "call_write".into(),
                    ok: true,
                    content: "written".into(),
                },
            },
        )]);

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(
            input,
            chat,
            runner,
            events_tx,
            decision_rx,
        ));

        let approval_id = wait_for_approval(&mut events_rx).await;
        decision_tx
            .send((approval_id, AgentDecision::Deny))
            .expect("deny");

        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.summary.denied, 1);
        assert_eq!(report.summary.approved, 0);
        assert_eq!(report.summary.tools_used, 1, "the denied call was still a tool step");
        assert_eq!(report.final_message, "Alright, I left the file alone.");

        // §17.4: the denial went back as a tool result the model adapted to.
        let requests = requests.lock().expect("requests lock").clone();
        assert_eq!(requests.len(), 2, "the loop continued after the denial");
        let denied_message = &requests[1].messages[2];
        assert_eq!(denied_message.role, Role::Tool);
        assert_eq!(denied_message.tool_call_id.as_deref(), Some("call_write"));
        assert_eq!(denied_message.content, DENIED_MESSAGE);

        // The write never executed: the router was never asked for the
        // approved side effect.
        assert!(
            runner_state
                .lock()
                .expect("state lock")
                .approved_runs
                .is_empty()
        );
    }

    // -----------------------------------------------------------------------
    // ApproveAllThisTurn (§17.4) — turn-local, never persisted
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn approve_all_this_turn_pre_approves_only_this_turn() {
        // Round 1: two writes in one batch — the user answers the first
        // card with ApproveAllThisTurn, so the second never parks. Round
        // 2: another write in the SAME turn — also pre-approved. Round 3:
        // the final answer. Then a SECOND turn with a write: it must park
        // again (nothing persisted — the flag died with the first turn).
        // Owned ids: the closure's return type must not borrow its input.
        let write_script = |id: &str| {
            (
                id.to_owned(),
                MockTool::Card {
                    card: diff_card("src/main.rs"),
                    approved: ToolResult {
                        id: id.to_owned(),
                        ok: true,
                        content: "written".into(),
                    },
                },
            )
        };
        let rounds = vec![
            tool_round(
                &[
                    ("call_w1", "write_file", r#"{"path":"a.rs","content":"1"}"#),
                    ("call_w2", "write_file", r#"{"path":"b.rs","content":"2"}"#),
                ],
                "",
                0.10,
            ),
            tool_round(
                &[("call_w3", "write_file", r#"{"path":"c.rs","content":"3"}"#)],
                "",
                0.10,
            ),
            answer_round("All written.", 0.05),
        ];
        let (chat, _requests) = ScriptedChat::new(rounds);
        let (runner, _state) = MockRunner::new(vec![
            write_script("call_w1"),
            write_script("call_w2"),
            write_script("call_w3"),
            write_script("call_next_turn"),
        ]);

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(
            input,
            chat,
            runner,
            events_tx.clone(),
            decision_rx,
        ));

        let approval_id = wait_for_approval(&mut events_rx).await;
        decision_tx
            .send((approval_id, AgentDecision::ApproveAllThisTurn))
            .expect("approve all");

        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.summary.approved, 3, "the card + the two pre-approvals");
        assert_eq!(report.summary.denied, 0);

        // Exactly one card was ever shown this turn.
        let events = drain_events(&mut events_rx);
        assert_eq!(
            count(&events, |event| matches!(
                event,
                AgentEvent::ApprovalNeeded { .. }
            )),
            1
        );

        // A second turn: a fresh AgentTurn — the pre-approval does NOT
        // carry over (a card appears and must be answered again).
        let (chat, _requests) = ScriptedChat::new(vec![
            tool_round(
                &[(
                    "call_next_turn",
                    "write_file",
                    r#"{"path":"d.rs","content":"4"}"#,
                )],
                "",
                0.10,
            ),
            answer_round("Written on the second ask.", 0.05),
        ]);
        let (runner, _state) = MockRunner::new(vec![write_script("call_next_turn")]);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(input, chat, runner, events_tx, decision_rx));

        let approval_id = wait_for_approval(&mut events_rx).await;
        decision_tx
            .send((approval_id, AgentDecision::Approve))
            .expect("approve");
        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.summary.approved, 1);
    }

    // -----------------------------------------------------------------------
    // The cost cap
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn cost_cap_aborts_the_turn_with_the_partial_answer() {
        // Each round costs $0.60 (from the receipts); the $1.00 cap aborts
        // after the second receipt, before its tools run.
        let (chat, _requests) = ScriptedChat::new(vec![
            tool_round(
                &[("call_r1", "read_file", r#"{"path":"a.rs"}"#)],
                "First answer text.",
                0.60,
            ),
            tool_round(
                &[("call_r2", "read_file", r#"{"path":"b.rs"}"#)],
                "Second answer text.",
                0.60,
            ),
        ]);
        let (runner, _state) = MockRunner::new(vec![
            ("call_r1", ready_result("call_r1", "a")),
            ("call_r2", ready_result("call_r2", "b")),
        ]);

        let input = TurnInput::new(model(), conversation())
            .with_caps(caps())
            .with_manifest(manifest());
        let (report, _events_rx) = run_turn(chat, runner, input).await;

        assert!(
            matches!(&report.end, TurnEnd::CostCap { limit_usd } if (*limit_usd - 1.0).abs() < 1e-9),
            "got: {:?}",
            report.end
        );
        assert!(!report.summary.completed);
        assert_eq!(report.summary.iterations, 2);
        assert!((report.summary.cost_usd - 1.20).abs() < 1e-9);
        // The partial answer: the last streamed text before the abort.
        assert_eq!(report.final_message, "Second answer text.");
        // The second round's tool never ran.
        assert_eq!(report.summary.tools_used, 1);
        assert!(report.end.toast().contains("cost cap"));
    }

    #[tokio::test]
    async fn cost_falls_back_to_the_model_price_table_when_the_gateway_is_silent() {
        // No cost on the receipts: the estimate comes from the model's
        // prices (100 prompt + 50 completion tokens per round).
        let (chat, _requests) = ScriptedChat::new(vec![
            tool_round(&[("call_r1", "read_file", r#"{"path":"a.rs"}"#)], "text", 0.0),
            answer_round("done", 0.0),
        ]);
        let (runner, _state) = MockRunner::new(vec![("call_r1", ready_result("call_r1", "a"))]);

        let input = TurnInput::new(model(), conversation())
            .with_caps(caps())
            .with_manifest(manifest());
        let (report, _events_rx) = run_turn(chat, runner, input).await;

        assert_eq!(report.end, TurnEnd::Completed);
        let per_round = compute_cost_usd(&model(), 100, 50);
        assert!((report.summary.cost_usd - 2.0 * per_round).abs() < 1e-12);
    }

    // -----------------------------------------------------------------------
    // The time cap (paused clock: the deadline fires, not the 5s delay)
    // -----------------------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn time_cap_aborts_mid_stream() {
        // The scripted gateway sleeps 5s before answering; the turn cap is
        // 1s. The stream future is dropped (the request cancelled) and the
        // turn ends with the time-cap reason.
        let round = ScriptedRound {
            events: vec![text_delta("partial "), text_delta("answer")],
            receipt: receipt(0.10, 100, 50),
            delay: Some(Duration::from_secs(5)),
        };
        let (chat, requests) = ScriptedChat::new(vec![round]);
        let (runner, _state) = MockRunner::new(vec![]);

        let time_caps = TurnCaps {
            max_turn_secs: 1,
            ..caps()
        };
        let input = TurnInput::new(model(), conversation())
            .with_caps(time_caps)
            .with_manifest(manifest());
        let (report, mut events_rx) = run_turn(chat, runner, input).await;

        assert_eq!(report.end, TurnEnd::TimeCap { limit_secs: 1 });
        assert!(!report.summary.completed);
        assert!(report.end.toast().contains("time cap"));
        // The request was made and then cancelled with the stream future.
        assert_eq!(requests.lock().expect("requests lock").len(), 1);
        assert_eq!(
            count(&drain_events(&mut events_rx), |event| matches!(
                event,
                AgentEvent::TurnDone(_)
            )),
            1
        );
    }

    // -----------------------------------------------------------------------
    // The context cap
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn context_overflow_aborts_with_a_clear_error_before_any_request() {
        // A 400-token window with the 20% reserve: the conversation (a
        // 4000-char user message ≈ 1000 tokens) already exceeds the
        // 320-token budget, so the turn aborts before the first request.
        let (chat, requests) = ScriptedChat::new(vec![]);
        let (runner, _state) = MockRunner::new(vec![]);

        let small_caps = TurnCaps {
            context_window: 400,
            ..caps()
        };
        let input = TurnInput::new(model(), vec![ChatMessage::user("x".repeat(4000))])
            .with_caps(small_caps)
            .with_manifest(manifest());
        let (report, _events_rx) = run_turn(chat, runner, input).await;

        assert!(
            matches!(
                &report.end,
                TurnEnd::ContextOverflow {
                    estimated_tokens,
                    budget_tokens: 320,
                } if *estimated_tokens > 320
            ),
            "got: {:?}",
            report.end
        );
        assert!(!report.summary.completed);
        assert!(report.end.toast().contains("context"));
        assert_eq!(
            requests.lock().expect("requests lock").len(),
            0,
            "no request was made"
        );
    }

    // -----------------------------------------------------------------------
    // Unit: the accumulator
    // -----------------------------------------------------------------------

    #[test]
    fn tool_call_accumulator_merges_fragments_in_index_order() {
        let mut accumulator = ToolCallAccumulator::default();
        accumulator.push(1, Some("call_b".into()), Some("bash".into()), r#"{"comm"#);
        accumulator.push(0, Some("call_a".into()), Some("read_file".into()), r#"{"pa"#);
        accumulator.push(1, None, None, r#"and":"ls"}"#);
        accumulator.push(0, None, None, r#"th":"x"}"#);

        let calls = accumulator.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, r#"{"path":"x"}"#);
        assert_eq!(calls[1].id, "call_b");
        assert_eq!(calls[1].name, "bash");
        assert_eq!(calls[1].arguments, r#"{"command":"ls"}"#);

        // A slot that never received a fragment is dropped.
        let mut accumulator = ToolCallAccumulator::default();
        accumulator.push(3, Some("call_c".into()), Some("bash".into()), "{}");
        assert_eq!(accumulator.finish().len(), 1);
    }

    // -----------------------------------------------------------------------
    // The settings → caps mapping (acceptance: configurable + enforced)
    // -----------------------------------------------------------------------

    #[test]
    fn the_loop_caps_come_from_settings_and_the_model() {
        let mut settings = AgentSettings::default();
        settings.max_iterations = 3;
        settings.max_turn_secs = 12;
        settings.max_cost_usd = 0.25;

        // The shell's mapping (wiring): settings.agent.* + the model's
        // context window → TurnCaps.
        let mapped = TurnCaps {
            max_tool_iterations: settings.max_iterations,
            max_turn_secs: settings.max_turn_secs,
            max_cost_usd: settings.max_cost_usd,
            context_window: 128_000,
            context_reserve: CONTEXT_RESERVE,
        };
        assert_eq!(mapped.max_tool_iterations, 3);
        assert_eq!(mapped.max_turn_secs, 12);
        assert!((mapped.max_cost_usd - 0.25).abs() < 1e-12);
        // The fourth cap: the model's window minus the 20% reserve.
        assert_eq!(mapped.context_budget_tokens(), Some(102_400));

        // The §17.3 spec defaults, verbatim.
        let defaults = TurnCaps::spec_defaults(128_000);
        assert_eq!(defaults.max_tool_iterations, 25);
        assert_eq!(defaults.max_turn_secs, 600);
        assert!((defaults.max_cost_usd - 1.0).abs() < 1e-12);
        assert!((defaults.context_reserve - 0.20).abs() < 1e-12);

        // Unknown window → no context cap (never a false abort).
        assert_eq!(TurnCaps::spec_defaults(0).context_budget_tokens(), None);
        // `max_turn_secs: 0` disables the time cap (documented on TurnCaps).
        assert_eq!(
            TurnCaps {
                max_turn_secs: 0,
                ..defaults
            }
            .max_turn_secs,
            0
        );
    }

    // -----------------------------------------------------------------------
    // The production adapters compile and fail visibly against 5b's stub
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn router_adapter_routes_through_5b_and_marks_the_approval_seam() {
        // Against 5b's skeleton router every call resolves to an error
        // result — the loop's adapter must surface that, never fake
        // success. When 5b lands its approved-execution entry point, this
        // is the single seam that changes (see RouterToolRunner docs).
        let mut runner = RouterToolRunner::new("C:/definitely/not/a/workspace");

        let outcome = runner.run(call("call_x", "read_file", "{}")).await;
        match outcome {
            ToolOutcome::Ready(result) => {
                assert!(!result.ok, "the stub router's error must surface");
                assert!(result.content.contains("Phase 5b"));
            }
            ToolOutcome::NeedsApproval(_) => panic!("the stub router never builds cards"),
        }

        let result = runner.run_approved(call("call_x", "read_file", "{}")).await;
        assert!(!result.ok, "no fake success through the seam");

        // Must not panic even with no abort handle to forward to yet.
        runner.abort();
    }
}
