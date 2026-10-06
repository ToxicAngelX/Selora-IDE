//! The agent turn state machine (ARCHITECTURE.md §17.3–17.4) — Phase
//! 5c's file.
//!
//! One `AgentTurn` is a user message turned into a tool loop: stream a
//! ChatRequest with the manifest armed; on `finish_reason ==
//! "tool_calls"` run the calls in order via the router; on a card, park
//! in `AwaitingApproval` until `Effect::AgentDecision` arrives; repeat
//! until a tool-free answer. Caps (25 iterations / 10 minutes / $1,
//! user-tunable in settings `agent.*`) abort with the partial answer.
//!
//! This skeleton (the prep commit) fixes only the shared types; 5c
//! implements the loop, caps, Stop, and the summary.

use serde::{Deserialize, Serialize};

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
    /// How the turn ended — `false` when a cap or the Stop button
    /// aborted it (the summary then follows a partial answer).
    pub completed: bool,
}

/// The state machine's public face — enough for the prep skeleton; 5c
/// owns the transitions, caps, and aborts.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentTurn {
    pub state: AgentTurnState,
}

impl AgentTurn {
    /// A turn starting in `Thinking`.
    pub fn new() -> Self {
        Self {
            state: AgentTurnState::Thinking,
        }
    }
}

impl Default for AgentTurn {
    fn default() -> Self {
        Self::new()
    }
}
