//! Selora AI — the client for the Selora gateway (`api.selora.lol`).
//!
//! Responsibility (PLAN.md §5): `GET /v1/models`,
//! `POST /v1/chat/completions` (streaming + non-streaming), the ghost-text
//! provider, and the cost meter. Depends only on `selora-core` (the error
//! contract) plus the networking stack.
//!
//! Phase 1b (this implementation) covers the gateway client and the SSE
//! stream: [`SeloraClient`], [`parse_sse_event`] and the §11 data model.
//! Ghost text and the settings-backed cost cache land in Phase 4.
//!
//! The SSE parser is a pure function over a byte buffer (`sse` module) and is
//! unit-tested against the mandatory RISKS.md §3 fixture list.

#![forbid(unsafe_code)]

mod agent;
mod chat_types;
mod client;
mod sse;
mod types;
mod updates;

pub use agent::{
    bound_summary, compact_conversation, compacted_conversation, denied_result, execute_approved,
    execute_tool, new_store, now_millis, plan, plan_approved_result, revise_plan_result,
    run_agent_turn, should_compact, spawn_agent_turn, summary_request, tool_manifest,
    transcript_conversation, AgentDecision, AgentEvent, AgentSessionMode, AgentSessionState,
    AgentSessionStore, AgentSessionSummary, AgentSummary, AgentToolSettings, AgentTurn,
    AgentTurnReport, AgentTurnState, ApprovalCard, ApprovalId, BashAbortHandle, ChatStream,
    ClientChatStream, CompactionConfig, CompactionCursor, CompactionError, CompactionOutcome,
    LoadedAgentSession, ParkSummary, ParkedCard, PlanDraft, PlanPhase, PlanSession, PlanSnapshot,
    PlanState, PlanStatus, PlanStore, RouterToolRunner, TodoItem, TodoStatus, ToolContext,
    ToolOutcome, ToolRunner, TranscriptEntry, TranscriptLoad, TurnCaps, TurnDecision, TurnEnd,
    TurnInput, AGENT_SESSIONS_DIR, COMPACTION_KEEP_LAST, COMPACTION_THRESHOLD, DENIED_MESSAGE,
    MAX_SUMMARY_CHARS, SESSION_SCHEMA, STATE_FILE, TODOS_FILE, TRANSCRIPT_FILE,
};
pub use chat_types::{ToolCall, ToolCallAccumulator, ToolCallId, ToolResult, ToolSpec};
pub use client::SeloraClient;
pub use sse::{parse_sse_event, SseEvent, MAX_DATA_FIELD_BYTES};
pub use types::{
    compute_cost_usd, ChatEvent, ChatMessage, ChatReceipt, ChatRequest, Model, ModelId, Role,
    SeloraStatus,
};
pub use updates::{check_latest_release, ReleaseInfo};
