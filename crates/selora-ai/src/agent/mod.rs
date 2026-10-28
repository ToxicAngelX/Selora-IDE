//! Agent Mode (ARCHITECTURE.md §17) — the tool loop's shared vocabulary.
//!
//! Phase 5's module split (AGENT_ORCHESTRATION.md §10):
//! - [`router`] + [`tools`] — Phase 5b: mapping tool calls to
//!   implementations, every §17.2 guard (path rooting, SSRF, caps), the
//!   approval cards for side-effecting tools.
//! - [`turn`] — Phase 5c: the `AgentTurn` state machine, loop caps,
//!   decisions.
//! - [`tools::web`] — Phase 5e: `web_search` + `web_fetch`.
//!
//! §17.2's invariants are non-negotiable and rest on this module:
//! every path resolves against the workspace root; `web_fetch` refuses
//! private/reserved ranges; the API key never appears in any tool
//! result, message, bash environment, or log; reads ≤4MB and fetch/bash
//! output ≤256KB (truncated with a notice).
//!
//! Phase 5b (router + tools) has landed: the ten §17.1 tools, every
//! guard, and the approval split. 5c's loop controller and 5e's web
//! implementations build on these seams.

pub mod plan;
pub mod router;
pub mod session;
pub mod subagent;
pub mod subagent_turn;
pub mod tools;
pub mod turn;

// Re-exports of the Phase 5 surfaces (5c/5d consume these): the router
// (5b), the loop (5c), the bash abort handle. The shared types' shapes
// are unchanged from the prep commit — these are additive.
pub use plan::{
    new_store, PlanDraft, PlanPhase, PlanSession, PlanSnapshot, PlanStore, TodoItem, TodoStatus,
};
pub use router::{
    denied_result, execute_approved, execute_tool, plan_approved_result, revise_plan_result,
    tool_manifest, AgentToolSettings, ToolContext, ToolOutcome,
};
pub use session::{
    bound_summary, compact_conversation, compacted_conversation, now_millis, should_compact,
    summary_request, transcript_conversation, AgentSessionMode, AgentSessionState,
    AgentSessionStore, AgentSessionSummary, CompactionCursor, CompactionError, CompactionOutcome,
    LoadedAgentSession, ParkSummary, ParkedCard, PlanState, PlanStatus, TranscriptEntry,
    TranscriptLoad, AGENT_SESSIONS_DIR, COMPACTION_KEEP_LAST, COMPACTION_THRESHOLD,
    MAX_SUMMARY_CHARS, SESSION_SCHEMA, STATE_FILE, TODOS_FILE, TRANSCRIPT_FILE,
};
#[allow(unused_imports)]
pub use subagent::{
    wait_tool_result, wait_tool_spec, ArtifactRef, CancelToken, ChildId, ChildOutcome, ChildState,
    CompletionDisposition, SpawnError, SpawnRequest, SubagentCard, SubagentCompletion,
    SubagentControlError, SubagentEvent, SubagentHandle, SubagentPolicy, SubagentRuntime,
    WaitError, WaitOutcome, DEFAULT_WAIT_TIMEOUT, MAX_WAIT_TIMEOUT, WAIT_TOOL_NAME,
};
pub use subagent_turn::{
    format_subagent_completion, question_answer_result, run_subagent_turn,
    run_subagent_turn_blocking, spawn_child_turn, SubagentBackend, SubagentSession,
};
pub use tools::bash::BashAbortHandle;
pub use turn::{
    run_agent_turn, spawn_agent_turn, AgentEvent, AgentSummary, AgentTurn, AgentTurnReport,
    AgentTurnState, ChatStream, ClientChatStream, CompactionConfig, RouterToolRunner, ToolRunner,
    TurnCaps, TurnDecision, TurnEnd, TurnInput, DENIED_MESSAGE,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies one approval card in flight (§17.4): the UI's decision
/// comes back as `Effect::AgentDecision(request_id, approval_id,
/// decision)`.
pub type ApprovalId = Uuid;

/// The user's answer to an approval card (§17.4).
///
/// `ApproveAllThisTurn` pre-approves remaining writes and
/// non-allowlisted bash for the rest of this turn only — never persisted
/// across turns or sessions. `Deny` continues the loop with a
/// "user denied this action" tool result; `Stop` aborts the turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentDecision {
    Approve,
    /// A structured answer is model input, never an effect grant.
    Answer(String),
    ApproveAllThisTurn,
    Deny,
    /// Phase 9b: reject the *shape* of a proposed plan while keeping the
    /// session in plan mode. The model sees a "revise the plan" tool result
    /// and calls `exit_plan_mode` again. Never a grant.
    Revise,
    Stop,
}

/// One option on a §19.4 escalation question. The description is shown under
/// the label so the options are decide-able without extra prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

/// One question on a §19.4 escalation card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuestion {
    pub question: String,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    /// Free text is available unless the model explicitly disables it.
    #[serde(default = "default_allow_free_text")]
    pub allow_free_text: bool,
}

fn default_allow_free_text() -> bool {
    true
}

/// The question card's payload. It rides [`ApprovalCard::Question`] and is
/// answered through the same single-use `AgentDecision` route as an approval
/// (`AgentDecision::Answer`); an answer is model input, never an effect grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionCard {
    pub questions: Vec<AgentQuestion>,
}

/// What a side-effecting tool needs the user to approve before the
/// router executes it (§17.4, UI_SPEC.md §4.2). The card is the *only*
/// moment a write reaches disk: approving it is the save.
///
/// (Serialize/Deserialize: Phase R3's session persistence stores parked
/// cards so a restored session keeps its timeline.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ApprovalCard {
    /// `write_file` / `edit_file` — a unified diff (create = all
    /// additions) against the workspace file.
    Diff {
        /// Workspace-relative path.
        path: String,
        /// The unified diff body (+/- lines, the 5d diff card renders it
        /// verbatim).
        diff: String,
    },
    /// `delete_file`.
    Delete {
        /// Workspace-relative path.
        path: String,
    },
    /// `bash` — the command about to run (auto-approved allowlisted
    /// commands never build a card).
    Command { command: String },
    /// `exit_plan_mode` (Phase 9b): the plan approval card. Approving flips
    /// the session to execution (writes still card) and seeds the step list
    /// as todos when the session has none; Revise/Deny keeps planning.
    Plan {
        /// Optional one-line context the model supplied.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        /// The ordered plan steps, validated and bounded by
        /// [`plan::MAX_PLAN_STEPS`].
        steps: Vec<String>,
    },
    /// `run_workflow` (§18.1): the confirmation moment for a workflow
    /// script. Shows the full script body, the extracted `world.run`
    /// command set, and the workflow-level cost cap. Approving it starts
    /// the run; nothing before that — and approving a script never
    /// pre-approves a subagent's writes (§18.1).
    Workflow {
        /// The workflow's name (unique within a run; resume's step key).
        name: String,
        /// The complete JS script body, rendered verbatim by the card.
        script: String,
        /// The extracted `world.run` command literals the script may
        /// spawn; a command assembled at runtime is not in this set.
        commands: Vec<String>,
        /// The workflow-level cost cap in USD (`workflow.max_cost_usd`),
        /// checked between every step.
        max_cost_usd: f64,
    },
    /// `ask_user_question` (§19.4): a structured escalation. It parks on the
    /// same §17.4 channel as an approval; the user's answer comes back as this
    /// tool call's result (never an approval).
    Question(QuestionCard),
}
