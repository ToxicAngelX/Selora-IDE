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
use std::pin::Pin;
use std::time::Duration;

use selora_core::SeloraError;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::router::{self, ToolContext, ToolOutcome};
use super::{plan, AgentDecision, ApprovalCard, ApprovalId};
use crate::chat_types::{ToolCall, ToolCallAccumulator, ToolResult, ToolSpec};
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

/// When and how the turn compacts its conversation (ARCHITECTURE §19.5).
///
/// Opt-in: `TurnInput::new` leaves compaction off so a turn's request count
/// stays caller-controlled. The desktop enables it with the §19.5 threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionConfig {
    /// Fraction of the context budget that triggers the summarize round.
    pub threshold: f64,
    /// Messages carried after the summary.
    pub keep_last: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            threshold: super::session::COMPACTION_THRESHOLD,
            keep_last: super::session::COMPACTION_KEEP_LAST,
        }
    }
}

impl CompactionConfig {
    /// True when `estimated_tokens` has reached this config's threshold.
    pub fn should_compact(&self, estimated_tokens: u64, budget_tokens: u64) -> bool {
        if budget_tokens == 0 {
            return false;
        }
        let threshold = if self.threshold.is_finite() {
            self.threshold.clamp(0.0, 1.0)
        } else {
            super::session::COMPACTION_THRESHOLD
        };
        (estimated_tokens as f64) >= (budget_tokens as f64 * threshold)
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
    /// The ~80% summarize round failed. The typed error is also emitted as
    /// [`AgentEvent::CompactionFailed`]; the partial summary is retained so
    /// the caller surfaces it instead of silently truncating history.
    CompactionFailed {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial_summary: Option<String>,
    },
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
            TurnEnd::CompactionFailed { message, .. } => {
                format!("agent turn stopped: context compaction failed ({message})")
            }
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
    /// The context-compaction summarize round failed. Typed for the shell:
    /// `SummaryFailed` carries the partial summary (never silently dropped).
    CompactionFailed(super::session::CompactionError),
    /// One summarize round completed; the caller records the cursor (the
    /// full transcript on disk is untouched).
    Compacted(super::session::CompactionCursor),
    /// The turn ended: the summary, the reason, the (partial) answer, and
    /// the grown conversation.
    TurnDone(AgentTurnReport),
    /// Phase 9b: the session's plan phase or proposed plan changed (set by
    /// `enter_plan_mode`, or by approving `exit_plan_mode`). Additive: a
    /// pre-9b consumer can ignore it.
    PlanChanged {
        phase: plan::PlanPhase,
        plan: Option<plan::PlanDraft>,
    },
    /// Phase 9b: the session's stored todo list changed. Additive.
    TodosChanged(Vec<plan::TodoItem>),
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
    /// Phase 9b: the session's plan/todo snapshot, when this runner owns
    /// one. The loop turns changes into [`AgentEvent::PlanChanged`] /
    /// [`AgentEvent::TodosChanged`]; a runner without session state (the
    /// test doubles) returns `None` and no such events are emitted.
    fn plan_snapshot(&self) -> Option<plan::PlanSnapshot> {
        None
    }
}

/// Production tool backend: 5b's router. A thin adapter — the single
/// merge point for whatever entry points 5b lands.
pub struct RouterToolRunner {
    ctx: ToolContext,
}

impl RouterToolRunner {
    /// One runner per turn, over one [`ToolContext`] — the context's
    /// [`BashAbortHandle`] is turn-scoped and sticky (§17.3 Stop).
    pub fn new(ctx: ToolContext) -> Self {
        Self { ctx }
    }
}

impl ToolRunner for RouterToolRunner {
    fn run(&mut self, call: ToolCall) -> ToolRunFuture {
        // 5b's plan half: read tools resolve, side-effecting tools come
        // back as cards. `ToolContext` is `Clone` and the abort handle is
        // shared (Arc) — the clone keeps the Stop button wired to any
        // bash child this round spawns.
        let ctx = self.ctx.clone();
        Box::pin(async move { router::execute_tool(&ctx, call).await })
    }

    fn run_approved(&mut self, call: ToolCall) -> ToolApprovedFuture {
        // 5b's apply half — the ONLY executing path, and it re-validates.
        // The loop only routes card-producing calls here (routing a
        // Ready/allowlisted call would execute it a second time).
        let ctx = self.ctx.clone();
        Box::pin(async move { router::execute_approved(&ctx, call).await })
    }

    fn abort(&mut self) {
        // §17.3 Stop: sticky for the rest of the turn — a stopped turn
        // must not run further bash commands even if a straggler future
        // resolves.
        self.ctx.abort.kill();
    }

    fn plan_snapshot(&self) -> Option<plan::PlanSnapshot> {
        Some(plan::snapshot(&self.ctx.plan))
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
    pub temperature: Option<f64>,
    pub reasoning_effort: Option<String>,
    /// Session-scoped full access: file writes/edits/deletes apply without
    /// an approval card. `bash` still cards (the command is not inspected).
    /// Workflow cards always require an explicit decision.
    /// This is a property of the caller's session mode, never persisted by
    /// the engine.
    pub auto_approve_writes: bool,
    /// Opt-in context compaction (§19.5). `None` keeps the request sequence
    /// caller-controlled (tests, short turns).
    pub compaction: Option<CompactionConfig>,
    /// Open todos carried into the compacted summary.
    pub open_todos: Vec<super::session::TodoItem>,
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
            temperature: None,
            reasoning_effort: None,
            auto_approve_writes: false,
            compaction: None,
            open_todos: Vec::new(),
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

    /// Session full-access: file writes/edits/deletes apply without a card.
    /// Bash and workflows keep their approval gates.
    pub fn with_auto_approve_writes(mut self, on: bool) -> Self {
        self.auto_approve_writes = on;
        self
    }

    /// Opt in to §19.5 context compaction at the configured threshold.
    pub fn with_compaction(mut self, config: CompactionConfig) -> Self {
        self.compaction = Some(config);
        self
    }

    /// The session's open todos, carried into the compacted summary.
    pub fn with_open_todos(mut self, todos: Vec<super::session::TodoItem>) -> Self {
        self.open_todos = todos;
        self
    }

    /// Builder-style temperature override.
    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Builder-style reasoning effort override.
    pub fn with_reasoning_effort(mut self, reasoning_effort: impl Into<String>) -> Self {
        self.reasoning_effort = Some(reasoning_effort.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Streaming tool-call accumulation (§17.6) — 5a's
// `chat_types::ToolCallAccumulator`, fed every `ChatEvent` the round
// streams (it ignores the non-ToolCallDelta variants). A non-empty
// `finish()` IS the `finish_reason == "tool_calls"` signal: 5a does not
// surface finish_reason as data (adding it would break three crates),
// so the loop distinguishes tool-calls-vs-stop by the accumulator.
// ---------------------------------------------------------------------------

/// Chars/4 token estimate over the conversation (the same fallback the
/// client's receipts use, RISKS.md §3) — coarse, but it keeps the
/// context cap honest without shipping a tokenizer.
pub(crate) fn estimate_conversation_tokens(conversation: &[ChatMessage]) -> u64 {
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
    let sleep = tokio::time::sleep_until(deadline.unwrap_or_else(far_future));
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
    let sleep = tokio::time::sleep_until(deadline.unwrap_or_else(far_future));
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
        // The accumulator ignores everything but ToolCallDelta.
        other => accumulator.feed(&other),
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
    let sleep = tokio::time::sleep_until(deadline.unwrap_or_else(far_future));
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

/// Phase 9b: after each tool, compare the runner's session snapshot with
/// the last one and emit the additive plan/todo events. Todo events are
/// separate so the plan spine can update without re-rendering the plan
/// card (and vice versa).
fn emit_plan_changes(
    tools: &dyn ToolRunner,
    last: &mut Option<plan::PlanSnapshot>,
    events: &mpsc::UnboundedSender<AgentEvent>,
) {
    let Some(snapshot) = tools.plan_snapshot() else {
        return;
    };
    let (phase_changed, todos_changed) = match last.as_ref() {
        None => (true, true),
        Some(previous) => (
            previous.phase != snapshot.phase || previous.plan != snapshot.plan,
            previous.todos != snapshot.todos,
        ),
    };
    if phase_changed {
        let _ = emit(
            events,
            AgentEvent::PlanChanged {
                phase: snapshot.phase,
                plan: snapshot.plan.clone(),
            },
        );
    }
    if todos_changed {
        let _ = emit(events, AgentEvent::TodosChanged(snapshot.todos.clone()));
    }
    *last = Some(snapshot);
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
    let auto_approve_writes = input.auto_approve_writes;
    let compaction = input.compaction.clone();
    let open_todos = input.open_todos.clone();
    let mut compaction_attempted = false;
    let deadline = if caps.max_turn_secs == 0 {
        None
    } else {
        Some(tokio::time::Instant::now() + Duration::from_secs(caps.max_turn_secs))
    };

    let mut turn = AgentTurn::new();
    let mut conversation = input.conversation.clone();
    let mut final_message = String::new();
    let mut plan_state = tools.plan_snapshot();
    let end: Option<TurnEnd>;
    let mut ui_gone = false;

    'turn: loop {
        // --- the caps, checked before every round ------------------------
        if let Some(budget) = caps.context_budget_tokens() {
            let mut estimated = estimate_conversation_tokens(&conversation);
            // §19.5: at ~80% of the budget, run one summarize round
            // through this same chat backend and continue with the
            // summary + open todos + last N messages. One attempt per
            // turn; a failure is typed and preserves the partial summary.
            if let Some(config) = compaction.as_ref() {
                if !compaction_attempted && config.should_compact(estimated, budget) {
                    compaction_attempted = true;
                    match super::session::compact_conversation(
                        &mut chat,
                        &model.id,
                        &conversation,
                        &tools
                            .plan_snapshot()
                            .map(|snapshot| snapshot.todos)
                            .unwrap_or_else(|| open_todos.clone()),
                        config.keep_last,
                        0,
                    )
                    .await
                    {
                        Ok(outcome) => {
                            conversation = outcome.conversation;
                            estimated = estimate_conversation_tokens(&conversation);
                            let _ = emit(&events, AgentEvent::Compacted(outcome.cursor));
                        }
                        Err(error) => {
                            let _ = emit(&events, AgentEvent::CompactionFailed(error.clone()));
                            let partial_summary = match &error {
                                super::session::CompactionError::SummaryFailed {
                                    partial_summary,
                                    ..
                                } => partial_summary.clone(),
                                _ => None,
                            };
                            end = Some(TurnEnd::CompactionFailed {
                                message: error.to_string(),
                                partial_summary,
                            });
                            break 'turn;
                        }
                    }
                }
            }
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
        let mut request = ChatRequest::new(model.id.clone(), conversation.clone())
            .with_tools(input.manifest.clone());
        if let Some(temp) = input.temperature {
            request = request.with_temperature(temp as f32);
        }
        if let Some(ref effort) = input.reasoning_effort {
            request = request.with_reasoning_effort(Some(effort.clone()));
        }
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
                    // Session full-access auto-applies file writes, edits,
                    // and deletes. A bash command still cards: the flag
                    // never inspects the command line.
                    // A workflow or plan card is never pre-approved: the shell starts the
                    // run / flips the session from the card's explicit Approve, so
                    // an earlier `Approve all this turn` must not silently skip the
                    // confirmation.
                    let explicit_card = matches!(
                        &card,
                        crate::agent::ApprovalCard::Workflow { .. }
                            | crate::agent::ApprovalCard::Plan { .. }
                    );
                    let auto_write = auto_approve_writes
                        && !matches!(
                            &card,
                            crate::agent::ApprovalCard::Command { .. }
                                | crate::agent::ApprovalCard::Workflow { .. }
                                | crate::agent::ApprovalCard::Plan { .. }
                        );
                    if (turn.approve_all && !explicit_card) || auto_write {
                        // §17.4: ApproveAllThisTurn pre-approves the rest
                        // of this turn's write/bash cards; full access
                        // pre-approves the file side effects for the session.
                        turn.approved += 1;
                    } else {
                        let approval_id = Uuid::new_v4();
                        turn.state = AgentTurnState::AwaitingApproval(approval_id);
                        if !emit(&events, AgentEvent::State(turn.state.clone()))
                            || !emit(&events, AgentEvent::ApprovalNeeded { approval_id, card })
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
                                end = Some(TurnEnd::Failed("agent decision channel closed".into()));
                                break 'turn;
                            }
                            Parked::Decided(AgentDecision::Approve) => turn.approved += 1,
                            Parked::Decided(AgentDecision::ApproveAllThisTurn) => {
                                turn.approved += 1;
                                if !matches!(call.name.as_str(), "exit_plan_mode" | "run_workflow")
                                {
                                    turn.approve_all = true;
                                }
                            }
                            Parked::Decided(AgentDecision::Deny) => {
                                turn.denied += 1;
                                result = Some(router::denied_result(&call));
                            }
                            // Phase 9b: Revise is a plan-card-only answer.
                            // It keeps the session planning (nothing was
                            // flipped) and tells the model to resubmit.
                            Parked::Decided(AgentDecision::Answer(answer)) => {
                                result = Some(ToolResult::success(&call.id, answer));
                            }
                            Parked::Decided(AgentDecision::Revise) => {
                                turn.denied += 1;
                                result = Some(if call.name == "exit_plan_mode" {
                                    router::revise_plan_result(&call)
                                } else {
                                    router::denied_result(&call)
                                });
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

            // Approved (or pre-approved): apply the call. For workflows, the
            // router only acknowledges confirmation; the shell owns execution.
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
            emit_plan_changes(&tools, &mut plan_state, &events);
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
    use std::path::PathBuf;
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
            ToolSpec::new(
                "read_file",
                "Read a workspace file",
                serde_json::json!({"type": "object"}),
            ),
            ToolSpec::new(
                "bash",
                "Run a shell command",
                serde_json::json!({"type": "object"}),
            ),
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

    fn tool_delta(
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        arguments: &str,
    ) -> ChatEvent {
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
            error: None,
        }
    }

    /// A round whose response is a final answer (no tool calls).
    fn answer_round(text: &str, cost_usd: f64) -> ScriptedRound {
        let round_receipt = receipt(cost_usd, 100, 50);
        ScriptedRound {
            events: vec![text_delta(text), ChatEvent::Done(round_receipt.clone())],
            receipt: round_receipt,
            delay: None,
            error: None,
        }
    }

    // -----------------------------------------------------------------------
    // The mock gateway (ChatStream)
    // -----------------------------------------------------------------------

    /// A round that streams `text` then fails (the compaction summarize
    /// failure path).
    fn error_round(text: &str) -> ScriptedRound {
        let round_receipt = receipt(0.0, 10, 5);
        ScriptedRound {
            events: vec![text_delta(text)],
            receipt: round_receipt,
            delay: None,
            error: Some(SeloraError::Net("gateway down".into())),
        }
    }

    struct ScriptedRound {
        events: Vec<ChatEvent>,
        receipt: ChatReceipt,
        delay: Option<Duration>,
        /// When set, the round streams `events` then fails (the compaction
        /// failure path: a partial summary must not be dropped).
        error: Option<SeloraError>,
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
                match round.error {
                    Some(error) => Err(error),
                    None => Ok(round.receipt),
                }
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

    /// A recording event receiver: every event the test consumes while
    /// driving the turn (waiting for cards, waiting for tools) is kept,
    /// so the final assertions see the *whole* stream, not just the tail.
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

        /// Wait for the next approval card and return its id (the card
        /// event is kept in `seen` — the full stream is the record).
        async fn wait_for_approval(&mut self) -> ApprovalId {
            loop {
                let event = self.rx.recv().await.expect("events channel open");
                if let AgentEvent::ApprovalNeeded { approval_id, .. } = &event {
                    let approval_id = *approval_id;
                    self.seen.push(event);
                    return approval_id;
                }
                self.seen.push(event);
            }
        }

        /// Wait for the state event proving the approved tool is in flight.
        async fn wait_for_running_tool(&mut self) {
            loop {
                let event = self.rx.recv().await.expect("events channel open");
                if matches!(event, AgentEvent::State(AgentTurnState::RunningTool(_))) {
                    return;
                }
                self.seen.push(event);
            }
        }

        /// Everything seen so far, plus whatever is still buffered
        /// (cumulative — safe to call more than once per turn).
        fn all_events(&mut self) -> Vec<AgentEvent> {
            while let Ok(event) = self.rx.try_recv() {
                self.seen.push(event);
            }
            self.seen.clone()
        }
    }

    fn count<'a, I>(events: I, predicate: impl Fn(&AgentEvent) -> bool) -> usize
    where
        I: IntoIterator<Item = &'a AgentEvent>,
    {
        events.into_iter().filter(|event| predicate(event)).count()
    }

    /// Drain whatever is still buffered (for tests that never drive the
    /// channel while the turn runs).
    fn drain_remaining(rx: &mut mpsc::UnboundedReceiver<AgentEvent>) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
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

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(events_rx);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(input, chat, runner, events_tx, decision_rx));

        // The user approves the write card when it appears.
        let approval_id = recorder.wait_for_approval().await;
        decision_tx
            .send((approval_id, AgentDecision::Approve))
            .expect("send decision");

        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(
            report.final_message,
            "Fixed: src/main.rs now has the comment."
        );
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
        assert_eq!(third[5].tool_call_id.as_deref(), Some("call_write"));

        // The report's conversation is the full transcript.
        assert_eq!(report.conversation.len(), 7);

        // The write executed exactly once, after the approval.
        let state = runner_state.lock().expect("state lock");
        assert_eq!(state.routed, ["call_read", "call_list", "call_write"]);
        assert_eq!(state.approved_runs, ["call_write"]);

        // The event stream (the recorder kept everything, including the
        // events that arrived before the card): three tool starts, three
        // results, exactly one card, one TurnDone, and the final state
        // Done.
        let events = recorder.all_events();
        assert_eq!(
            count(&events, |event| matches!(
                event,
                AgentEvent::ToolCallStarted(_)
            )),
            3
        );
        assert_eq!(
            count(&events, |event| matches!(event, AgentEvent::ToolResult(_))),
            3
        );
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
        assert_eq!(
            count(&events, |event| *event
                == AgentEvent::State(AgentTurnState::Done)),
            1
        );
    }

    // -----------------------------------------------------------------------
    // Scenario 2: a cap-exceeded turn aborts
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn iteration_cap_from_settings_aborts_the_turn() {
        // The model never stops asking for tools; the settings cap is 3.
        let settings = AgentSettings {
            max_iterations: 3,
            ..AgentSettings::default()
        };
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
            script.push((id.clone(), ready_result(&id, &format!("file {n}"))));
            let split = arguments.len() / 2;
            rounds.push(ScriptedRound {
                events: vec![
                    tool_delta(0, Some(&id), Some("read_file"), &arguments[..split]),
                    tool_delta(0, None, None, &arguments[split..]),
                ],
                receipt: receipt(0.01, 100, 50),
                delay: None,
                error: None,
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
        assert_eq!(
            report.summary.iterations, 4,
            "the 4th round is the one capped"
        );
        assert!(report.end.toast().contains("iteration cap"));

        // The turn was aborted, not run to the script's end — and the
        // TurnDone event (the shell's toast trigger) still arrived.
        assert_eq!(requests.lock().expect("requests lock").len(), 4);
        let events = drain_remaining(&mut events_rx);
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
            &[("call_bash", "bash", r#"{"command":"ping -n 60 127.0.0.1"}"#)],
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

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(events_rx);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(input, chat, runner, events_tx, decision_rx));

        let approval_id = recorder.wait_for_approval().await;
        decision_tx
            .send((approval_id, AgentDecision::Approve))
            .expect("approve");
        // The state event proves the approved tool is in flight (the
        // child is spawned inside its run future).
        recorder.wait_for_running_tool().await;
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

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(events_rx);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(input, chat, runner, events_tx, decision_rx));

        let approval_id = recorder.wait_for_approval().await;
        decision_tx
            .send((approval_id, AgentDecision::Deny))
            .expect("deny");

        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.summary.denied, 1);
        assert_eq!(report.summary.approved, 0);
        assert_eq!(
            report.summary.tools_used, 1,
            "the denied call was still a tool step"
        );
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
        assert!(runner_state
            .lock()
            .expect("state lock")
            .approved_runs
            .is_empty());
    }

    // -----------------------------------------------------------------------
    // Session full access — file cards apply without parking
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn auto_approve_writes_applies_file_cards_without_a_card() {
        let rounds = vec![
            tool_round(
                &[("call_w", "write_file", r#"{"path":"a.rs","content":"1"}"#)],
                "",
                0.01,
            ),
            answer_round("Written.", 0.01),
        ];
        let (chat, _requests) = ScriptedChat::new(rounds);
        let (runner, _state) = MockRunner::new(vec![(
            "call_w".to_owned(),
            MockTool::Card {
                card: diff_card("src/main.rs"),
                approved: ToolResult {
                    id: "call_w".to_owned(),
                    ok: true,
                    content: "written".into(),
                },
            },
        )]);

        let input = TurnInput::new(model(), conversation())
            .with_manifest(manifest())
            .with_auto_approve_writes(true);
        let (report, mut events_rx) = run_turn(chat, runner, input).await;
        let mut events: Vec<AgentEvent> = Vec::new();
        while let Ok(event) = events_rx.try_recv() {
            events.push(event);
        }

        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.summary.approved, 1);
        assert_eq!(report.summary.denied, 0);
        assert_eq!(
            count(&events, |event| matches!(
                event,
                AgentEvent::ApprovalNeeded { .. }
            )),
            0,
            "full access must not park a file card"
        );
        assert_eq!(
            count(&events, |event| matches!(event, AgentEvent::ToolResult(_))),
            1
        );
    }

    #[tokio::test]
    async fn auto_approve_writes_never_pre_approves_bash() {
        // Session full access pre-approves the file side effects only. A
        // non-allowlisted command still parks; denying it never runs it.
        let rounds = vec![
            tool_round(
                &[("call_bash", "bash", r#"{"command":"cargo test"}"#)],
                "",
                0.01,
            ),
            answer_round("The command was denied.", 0.01),
        ];
        let (chat, _requests) = ScriptedChat::new(rounds);
        let (runner, state) = MockRunner::new(vec![(
            "call_bash".to_owned(),
            MockTool::Card {
                card: ApprovalCard::Command {
                    command: "cargo test".into(),
                },
                approved: ToolResult {
                    id: "call_bash".to_owned(),
                    ok: true,
                    content: "ran".into(),
                },
            },
        )]);

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(events_rx);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation())
            .with_manifest(manifest())
            .with_auto_approve_writes(true);
        let task = tokio::spawn(run_agent_turn(input, chat, runner, events_tx, decision_rx));

        let approval_id = recorder.wait_for_approval().await;
        decision_tx
            .send((approval_id, AgentDecision::Deny))
            .expect("deny the command");
        let report = task.await.expect("turn task joins");

        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.summary.denied, 1);
        assert_eq!(report.summary.approved, 0);
        assert!(
            state.lock().expect("runner").approved_runs.is_empty(),
            "the denied command never ran"
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

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(events_rx);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(
            input,
            chat,
            runner,
            events_tx.clone(),
            decision_rx,
        ));

        let approval_id = recorder.wait_for_approval().await;
        decision_tx
            .send((approval_id, AgentDecision::ApproveAllThisTurn))
            .expect("approve all");

        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(
            report.summary.approved, 3,
            "the card + the two pre-approvals"
        );
        assert_eq!(report.summary.denied, 0);

        // Exactly one card was ever shown this turn: w2 and w3 were
        // pre-approved, never parked.
        let events = recorder.all_events();
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

        let approval_id = recorder.wait_for_approval().await;
        decision_tx
            .send((approval_id, AgentDecision::Approve))
            .expect("approve");
        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.summary.approved, 1);
        // Two cards total across the two turns: the pre-approval did not
        // leak into this one.
        assert_eq!(
            count(&recorder.all_events(), |event| matches!(
                event,
                AgentEvent::ApprovalNeeded { .. }
            )),
            2
        );
    }

    // -----------------------------------------------------------------------
    // Workflow confirmation never inherits file/turn-wide approval
    // -----------------------------------------------------------------------

    async fn assert_workflow_approval_gate(
        approve_all_this_turn: bool,
        auto_approve_writes: bool,
        decision: AgentDecision,
    ) {
        let plan = router::WorkflowPlan {
            name: "audit".into(),
            script: "log('audit');".into(),
            commands: vec![],
            max_cost_usd: 2.5,
        };
        let card = ApprovalCard::Workflow {
            name: plan.name.clone(),
            script: plan.script.clone(),
            commands: plan.commands.clone(),
            max_cost_usd: plan.max_cost_usd,
        };
        let workflow_call = call(
            "call_workflow",
            "run_workflow",
            &serde_json::json!({
                "name": plan.name,
                "script": plan.script,
                "commands": plan.commands,
                "max_cost_usd": plan.max_cost_usd,
            })
            .to_string(),
        );
        let prior_write = approve_all_this_turn || auto_approve_writes;
        let mut rounds = Vec::new();
        let mut script = Vec::new();
        if prior_write {
            rounds.push(tool_round(
                &[(
                    "call_write",
                    "write_file",
                    r#"{"path":"a.rs","content":"1"}"#,
                )],
                "",
                0.01,
            ));
            script.push((
                "call_write",
                MockTool::Card {
                    card: diff_card("a.rs"),
                    approved: ToolResult::success("call_write", "written"),
                },
            ));
        }
        rounds.push(tool_round(
            &[(
                &workflow_call.id,
                &workflow_call.name,
                &workflow_call.arguments,
            )],
            "",
            0.01,
        ));
        rounds.push(answer_round("Confirmation handled.", 0.01));
        let confirmed = router::workflow_confirmed_result(&workflow_call.id, &plan);
        script.push((
            "call_workflow",
            MockTool::Card {
                card: card.clone(),
                approved: confirmed.clone(),
            },
        ));
        let (chat, requests) = ScriptedChat::new(rounds);
        let (runner, runner_state) = MockRunner::new(script);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(events_rx);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input =
            TurnInput::new(model(), conversation()).with_auto_approve_writes(auto_approve_writes);
        let task = tokio::spawn(run_agent_turn(input, chat, runner, events_tx, decision_rx));

        if approve_all_this_turn {
            let approval_id =
                tokio::time::timeout(Duration::from_secs(1), recorder.wait_for_approval())
                    .await
                    .expect("the first write must park");
            decision_tx
                .send((approval_id, AgentDecision::ApproveAllThisTurn))
                .expect("approve all writes this turn");
        }

        let approval_id =
            tokio::time::timeout(Duration::from_secs(1), recorder.wait_for_approval())
                .await
                .expect("workflow must park despite earlier approval shortcuts");
        assert_eq!(
            recorder.seen.last(),
            Some(&AgentEvent::ApprovalNeeded { approval_id, card })
        );
        assert!(recorder
            .seen
            .contains(&AgentEvent::State(AgentTurnState::AwaitingApproval(
                approval_id
            ))));
        let mut expected_runs = Vec::new();
        if prior_write {
            expected_runs.push("call_write".to_owned());
        }
        {
            let state = runner_state.lock().expect("state lock");
            assert_eq!(state.approved_runs, expected_runs);
            assert_eq!(
                state.routed.last().map(String::as_str),
                Some("call_workflow")
            );
        }
        assert!(
            !task.is_finished(),
            "workflow is waiting for its own decision"
        );
        assert!(!recorder.seen.iter().any(|event| matches!(
            event,
            AgentEvent::ToolResult(result) if result.id == workflow_call.id
        )));

        decision_tx
            .send((approval_id, decision.clone()))
            .expect("answer workflow card");
        let report = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("workflow decision must unpark the turn")
            .expect("turn task joins");
        let denied = decision == AgentDecision::Deny;
        let expected_result = if denied {
            router::denied_result(&workflow_call)
        } else {
            expected_runs.push(workflow_call.id.clone());
            confirmed
        };
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.summary.tools_used, u32::from(prior_write) + 1);
        assert_eq!(
            report.summary.approved,
            u32::from(prior_write) + u32::from(!denied)
        );
        assert_eq!(report.summary.denied, u32::from(denied));
        assert_eq!(
            runner_state.lock().expect("state lock").approved_runs,
            expected_runs,
            "denied workflows must never reach run_approved"
        );
        let events = recorder.all_events();
        assert_eq!(
            count(&events, |event| matches!(
                event,
                AgentEvent::ApprovalNeeded { .. }
            )),
            usize::from(approve_all_this_turn) + 1
        );
        assert!(events.contains(&AgentEvent::ToolResult(expected_result.clone())));
        let requests = requests.lock().expect("requests lock");
        let result_message = requests.last().unwrap().messages.last().unwrap();
        assert_eq!(result_message.role, Role::Tool);
        assert_eq!(
            result_message.tool_call_id.as_deref(),
            Some("call_workflow")
        );
        assert_eq!(result_message.content, expected_result.content);
    }

    #[tokio::test(start_paused = true)]
    async fn approve_all_this_turn_still_parks_later_workflow() {
        assert_workflow_approval_gate(true, false, AgentDecision::Approve).await;
    }

    #[tokio::test(start_paused = true)]
    async fn auto_approve_writes_still_parks_workflow() {
        assert_workflow_approval_gate(false, true, AgentDecision::Approve).await;
    }

    #[tokio::test(start_paused = true)]
    async fn denied_workflow_never_calls_run_approved() {
        assert_workflow_approval_gate(false, false, AgentDecision::Deny).await;
    }

    #[tokio::test(start_paused = true)]
    async fn approve_all_this_turn_still_parks_the_plan_card() {
        // A write first (pre-approved by the shortcut), then a plan card:
        // the plan is an explicit decision, never skipped by `Approve all`
        // or session full access.
        let plan_card = ApprovalCard::Plan {
            summary: Some("Do it".into()),
            steps: vec!["Inspect".into(), "Write".into()],
        };
        let (chat, _requests) = ScriptedChat::new(vec![
            tool_round(
                &[(
                    "call_write",
                    "write_file",
                    r#"{"path":"a.rs","content":"x"}"#,
                )],
                "Writing.",
                0.01,
            ),
            tool_round(
                &[(
                    "call_plan",
                    "exit_plan_mode",
                    r#"{"plan":["Inspect","Write"]}"#,
                )],
                "Proposing.",
                0.01,
            ),
            answer_round("Revised.", 0.01),
        ]);
        let (runner, runner_state) = MockRunner::new(vec![
            (
                "call_write",
                MockTool::Card {
                    card: diff_card("a.rs"),
                    approved: ToolResult {
                        id: "call_write".into(),
                        ok: true,
                        content: "written".into(),
                    },
                },
            ),
            (
                "call_plan",
                MockTool::Card {
                    card: plan_card.clone(),
                    approved: ToolResult {
                        id: "call_plan".into(),
                        ok: true,
                        content: "approved".into(),
                    },
                },
            ),
        ]);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(events_rx);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation());
        let task = tokio::spawn(run_agent_turn(input, chat, runner, events_tx, decision_rx));

        // Pre-approve the write with `Approve all this turn`.
        let write_id = tokio::time::timeout(Duration::from_secs(1), recorder.wait_for_approval())
            .await
            .expect("the write cards first");
        decision_tx
            .send((write_id, AgentDecision::ApproveAllThisTurn))
            .expect("approve all this turn");

        let id = tokio::time::timeout(Duration::from_secs(1), recorder.wait_for_approval())
            .await
            .expect("the plan must park despite approve-all");
        assert_eq!(
            recorder.seen.last(),
            Some(&AgentEvent::ApprovalNeeded {
                approval_id: id,
                card: plan_card,
            })
        );
        assert!(
            !task.is_finished(),
            "the plan card waits for its own explicit decision"
        );
        decision_tx
            .send((id, AgentDecision::Revise))
            .expect("revise the plan");
        let report = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("revise unpark the turn")
            .expect("turn joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(report.summary.denied, 1);
        assert_eq!(
            runner_state.lock().expect("state lock").approved_runs,
            vec!["call_write".to_owned()],
            "the write was pre-approved; the plan never was"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn auto_mode_still_parks_the_plan_card() {
        let plan_card = ApprovalCard::Plan {
            summary: None,
            steps: vec!["Inspect".into()],
        };
        let (chat, _requests) = ScriptedChat::new(vec![
            tool_round(
                &[("call_plan", "exit_plan_mode", r#"{"plan":["Inspect"]}"#)],
                "Proposing.",
                0.01,
            ),
            answer_round("Approved.", 0.01),
        ]);
        let (runner, runner_state) = MockRunner::new(vec![(
            "call_plan",
            MockTool::Card {
                card: plan_card.clone(),
                approved: ToolResult {
                    id: "call_plan".into(),
                    ok: true,
                    content: "approved".into(),
                },
            },
        )]);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(events_rx);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_auto_approve_writes(true);
        let task = tokio::spawn(run_agent_turn(input, chat, runner, events_tx, decision_rx));

        let id = tokio::time::timeout(Duration::from_secs(1), recorder.wait_for_approval())
            .await
            .expect("auto mode must not skip the plan card");
        assert_eq!(
            recorder.seen.last(),
            Some(&AgentEvent::ApprovalNeeded {
                approval_id: id,
                card: plan_card,
            })
        );
        decision_tx
            .send((id, AgentDecision::Approve))
            .expect("approve the plan");
        let report = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the decision unparks the turn")
            .expect("turn joins");
        assert_eq!(report.end, TurnEnd::Completed);
        assert_eq!(
            runner_state.lock().expect("state lock").approved_runs,
            vec!["call_plan".to_owned()]
        );
    }

    #[tokio::test]
    async fn a_structured_answer_is_model_input_not_a_tool_execution_grant() {
        let (chat, _) = ScriptedChat::new(vec![
            tool_round(&[("question-1", "question", "{}")], "", 0.0),
            answer_round("Understood.", 0.0),
        ]);
        let (runner, state) = MockRunner::new(vec![(
            "question-1",
            MockTool::Card {
                card: ApprovalCard::Command {
                    command: "choose".into(),
                },
                approved: ToolResult::success("question-1", "must not execute"),
            },
        )]);
        let (events, receiver) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(receiver);
        let (decisions, decision_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_agent_turn(
            TurnInput::new(model(), conversation()),
            chat,
            runner,
            events,
            decision_rx,
        ));
        let id = recorder.wait_for_approval().await;
        decisions
            .send((id, AgentDecision::Answer("Use src/main.rs".into())))
            .unwrap();
        let report = task.await.unwrap();
        assert_eq!(report.end, TurnEnd::Completed);
        assert!(state.lock().unwrap().approved_runs.is_empty());
        assert!(report.conversation.iter().any(
            |message| message.role == crate::Role::Tool && message.content == "Use src/main.rs"
        ));
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
            tool_round(
                &[("call_r1", "read_file", r#"{"path":"a.rs"}"#)],
                "text",
                0.0,
            ),
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
            error: None,
        };
        let (chat, requests) = ScriptedChat::new(vec![round]);
        let (runner, _state) = MockRunner::new(Vec::<(&str, MockTool)>::new());

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
            count(&drain_remaining(&mut events_rx), |event| matches!(
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
        let (runner, _state) = MockRunner::new(Vec::<(&str, MockTool)>::new());

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

    #[tokio::test]
    async fn context_compaction_runs_one_summary_round_and_carries_state_forward() {
        let mut conversation = vec![ChatMessage::user("x".repeat(3000))];
        conversation.push(ChatMessage::assistant("older answer"));
        conversation.push(ChatMessage::user("recent question"));
        conversation.push(ChatMessage::assistant("recent answer"));
        let (chat, requests) = ScriptedChat::new(vec![
            answer_round("summary of the work so far", 0.0),
            answer_round("all done", 0.01),
        ]);
        let (runner, _state) = MockRunner::new(Vec::<(&str, MockTool)>::new());
        let caps = TurnCaps {
            context_window: 1_000,
            ..caps()
        };
        let input = TurnInput::new(model(), conversation)
            .with_caps(caps)
            .with_manifest(manifest())
            .with_compaction(CompactionConfig {
                threshold: 0.8,
                keep_last: 2,
            })
            .with_open_todos(vec![crate::agent::session::TodoItem {
                id: "t1".into(),
                title: "keep this todo".into(),
                status: crate::agent::session::TodoStatus::InProgress,
            }]);
        let (report, mut rx) = run_turn(chat, runner, input).await;

        assert_eq!(report.end, TurnEnd::Completed);
        let events = drain_remaining(&mut rx);
        assert_eq!(
            count(&events, |event| matches!(event, AgentEvent::Compacted(_))),
            1,
            "the cursor event is emitted once"
        );
        let requests = requests.lock().expect("requests");
        assert_eq!(requests.len(), 2, "one summarize round, then the answer");
        assert!(requests[0].tools.is_none(), "the summary is tool-free");
        let carried = &requests[1].messages;
        assert!(carried[0].content.contains("summary of the work so far"));
        assert!(carried[0].content.contains("keep this todo"));
        assert_eq!(carried.len(), 3, "summary + keep_last 2");
        assert_eq!(carried[1].content, "recent question");
        assert_eq!(carried[2].content, "recent answer");
    }

    #[tokio::test]
    async fn context_compaction_failure_is_typed_and_keeps_the_partial_summary() {
        let mut conversation = vec![ChatMessage::user("x".repeat(3000))];
        conversation.push(ChatMessage::user("recent question"));
        conversation.push(ChatMessage::assistant("recent answer"));
        let (chat, requests) = ScriptedChat::new(vec![error_round("partial summary")]);
        let (runner, _state) = MockRunner::new(Vec::<(&str, MockTool)>::new());
        let caps = TurnCaps {
            context_window: 1_000,
            ..caps()
        };
        let input = TurnInput::new(model(), conversation)
            .with_caps(caps)
            .with_manifest(manifest())
            .with_compaction(CompactionConfig {
                threshold: 0.8,
                keep_last: 2,
            });
        let (report, mut rx) = run_turn(chat, runner, input).await;

        match &report.end {
            TurnEnd::CompactionFailed {
                message,
                partial_summary,
            } => {
                assert!(message.contains("gateway down"));
                assert_eq!(partial_summary.as_deref(), Some("partial summary"));
            }
            other => panic!("expected a typed compaction failure, got {other:?}"),
        }
        assert!(!report.summary.completed);
        let events = drain_remaining(&mut rx);
        assert_eq!(
            count(&events, |event| matches!(
                event,
                AgentEvent::CompactionFailed(
                    crate::agent::session::CompactionError::SummaryFailed {
                        partial_summary: Some(_),
                        ..
                    }
                )
            )),
            1
        );
        assert_eq!(
            requests.lock().expect("requests").len(),
            1,
            "the failed summary is not retried silently"
        );
    }

    // -----------------------------------------------------------------------
    // Unit: the accumulator
    // -----------------------------------------------------------------------

    #[test]
    fn tool_call_accumulator_merges_fragments_in_index_order() {
        let mut accumulator = ToolCallAccumulator::default();
        accumulator.push(1, Some("call_b".into()), Some("bash".into()), r#"{"comm"#);
        accumulator.push(
            0,
            Some("call_a".into()),
            Some("read_file".into()),
            r#"{"pa"#,
        );
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
        let settings = AgentSettings {
            max_iterations: 3,
            max_turn_secs: 12,
            max_cost_usd: 0.25,
            ..AgentSettings::default()
        };

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
    // The production adapter against 5b's real router (real fs, no network)
    // -----------------------------------------------------------------------

    fn unique_temp_root(label: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "selora-ai-agent-turn-{label}-{}-{n}",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn router_adapter_plans_applies_and_aborts_through_5b() {
        // Plan (run) resolves read tools; apply (run_approved) executes
        // cards and refuses non-card calls (the double-run guard);
        // abort kills the turn's bash handle — all through the same
        // ToolContext clone the loop hands to each round.
        let root = unique_temp_root("adapter");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("a.rs"), b"fn main() {}").expect("write a.rs");

        let ctx = ToolContext::new(&root, router::AgentToolSettings::default());
        let abort_handle = ctx.abort.clone();
        let mut runner = RouterToolRunner::new(ctx);

        // Plan: a read tool resolves straight to a ready result.
        let outcome = runner
            .run(call("call_x", "read_file", r#"{"path":"a.rs"}"#))
            .await;
        match outcome {
            ToolOutcome::Ready(result) => {
                assert!(result.ok, "read must succeed: {}", result.content);
                assert!(result.content.contains("fn main()"));
            }
            ToolOutcome::NeedsApproval(_) => panic!("read_file never produces a card"),
        }

        // Plan: a write tool comes back as a card (nothing on disk yet).
        let write_args = r#"{"path":"b.rs","content":"approved text"}"#;
        let outcome = runner.run(call("call_w", "write_file", write_args)).await;
        assert!(
            matches!(outcome, ToolOutcome::NeedsApproval(_)),
            "write_file must produce a diff card"
        );
        assert!(!root.join("b.rs").exists(), "planning must not write");

        // Apply: the approved call executes — the card is the save moment.
        let result = runner
            .run_approved(call("call_w", "write_file", write_args))
            .await;
        assert!(result.ok, "approved write must succeed: {}", result.content);
        assert_eq!(
            std::fs::read_to_string(root.join("b.rs")).expect("b.rs on disk"),
            "approved text"
        );

        // Apply refuses non-card calls (routing a Ready call here would
        // execute it a second time — 5b's guard).
        let result = runner
            .run_approved(call("call_x", "read_file", r#"{"path":"a.rs"}"#))
            .await;
        assert!(!result.ok, "read_file is not an approved-execution tool");
        assert!(result.content.contains("nothing to approve"));

        // Stop: the handle the runner shares with every cloned context
        // goes sticky.
        runner.abort();
        assert!(abort_handle.is_killed(), "the turn's bash handle is killed");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn turn_emits_plan_and_todo_events_and_gates_writes_until_approval() {
        // Round 1: enter plan mode + a write in the same batch — the write
        // must be refused structurally, before any card or disk touch.
        // Round 2: propose a plan; the user approves it.
        // Round 3: the same write now cards, and is approved.
        // Round 4: the final answer.
        let root = unique_temp_root("plan-turn");
        std::fs::create_dir_all(&root).expect("mkdir");
        let (chat, _requests) = ScriptedChat::new(vec![
            tool_round(
                &[
                    ("call_enter", "enter_plan_mode", "{}"),
                    (
                        "call_blocked",
                        "write_file",
                        r#"{"path":"blocked.txt","content":"nope"}"#,
                    ),
                ],
                "Planning first.",
                0.01,
            ),
            tool_round(
                &[(
                    "call_exit",
                    "exit_plan_mode",
                    r#"{"summary":"Make the file","plan":["Inspect","Write","Verify"]}"#,
                )],
                "Here is the plan.",
                0.01,
            ),
            tool_round(
                &[(
                    "call_write",
                    "write_file",
                    r#"{"path":"made.txt","content":"approved"}"#,
                )],
                "Executing.",
                0.01,
            ),
            answer_round("Done.", 0.01),
        ]);

        let store = plan::new_store();
        let ctx =
            ToolContext::new(&root, router::AgentToolSettings::default()).with_plan(store.clone());
        let runner = RouterToolRunner::new(ctx);

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let mut recorder = Recorder::new(events_rx);
        let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
        let input = TurnInput::new(model(), conversation()).with_manifest(manifest());
        let task = tokio::spawn(run_agent_turn(input, chat, runner, events_tx, decision_rx));

        // The only two cards in this turn are the plan and the write; the
        // blocked write never cards.
        let plan_card = recorder.wait_for_approval().await;
        decision_tx
            .send((plan_card, AgentDecision::Approve))
            .expect("approve the plan");
        let write_card = recorder.wait_for_approval().await;
        decision_tx
            .send((write_card, AgentDecision::Approve))
            .expect("approve the write");

        let report = task.await.expect("turn task joins");
        assert_eq!(report.end, TurnEnd::Completed);
        let seen = recorder.seen;

        // The structural rejection is model-visible, not a phantom success.
        let blocked = seen
            .iter()
            .find_map(|event| match event {
                AgentEvent::ToolResult(result) if result.id == "call_blocked" => Some(result),
                _ => None,
            })
            .expect("the blocked write has a tool result");
        assert!(!blocked.ok);
        assert!(blocked.content.contains("approve the plan first"));
        assert!(!root.join("blocked.txt").exists());

        // Plan/todo events are additive and carry real stored state.
        assert!(
            seen.iter().any(|event| matches!(
                event,
                AgentEvent::PlanChanged {
                    phase: plan::PlanPhase::Planning,
                    ..
                }
            )),
            "entering plan mode must emit a plan phase event"
        );
        assert!(
            seen.iter().any(|event| matches!(
                event,
                AgentEvent::PlanChanged {
                    phase: plan::PlanPhase::Executing,
                    plan: Some(plan),
                } if plan.steps.len() == 3
            )),
            "approving the plan must emit the execution phase with the plan"
        );
        assert!(
            seen.iter().any(|event| matches!(
                event,
                AgentEvent::TodosChanged(todos) if todos.len() == 3
            )),
            "an approved plan must seed the live todos"
        );

        // The approved write executed for real, and the session store
        // agrees about the phase.
        assert_eq!(
            std::fs::read_to_string(root.join("made.txt")).expect("written file"),
            "approved"
        );
        assert_eq!(plan::snapshot(&store).phase, plan::PlanPhase::Executing);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn denied_wording_cannot_drift_between_the_loop_and_the_router() {
        // §17.4's exact words: the loop feeds the router's
        // `denied_result` to the conversation, so both must agree.
        let denied = router::denied_result(&call("call_d", "bash", "{}"));
        assert_eq!(denied.content, DENIED_MESSAGE);
        assert!(!denied.ok);
        assert_eq!(denied.id, "call_d");
    }
}
