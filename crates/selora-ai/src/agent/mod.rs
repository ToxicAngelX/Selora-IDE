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
//! This skeleton (the prep commit) fixes only the shared types so the
//! five Phase 5 subagents build in parallel; implementations land with
//! their phases.

pub mod router;
pub mod tools;
pub mod turn;

// Skeleton re-exports: unused until the Phase 5b/5c loop lands (the
// repo's `Effect` enum marks its pre-consumption seams the same way).
#[allow(unused_imports)]
pub use router::{execute_tool, tool_manifest, ToolOutcome};
pub use turn::{
    run_agent_turn, spawn_agent_turn, AgentEvent, AgentSummary, AgentTurn, AgentTurnReport,
    AgentTurnState, ChatStream, ClientChatStream, RouterToolRunner, ToolRunner, TurnCaps,
    TurnDecision, TurnEnd, TurnInput,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentDecision {
    Approve,
    ApproveAllThisTurn,
    Deny,
    Stop,
}

/// What a side-effecting tool needs the user to approve before the
/// router executes it (§17.4, UI_SPEC.md §4.2). The card is the *only*
/// moment a write reaches disk: approving it is the save.
#[derive(Debug, Clone, PartialEq)]
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
}
