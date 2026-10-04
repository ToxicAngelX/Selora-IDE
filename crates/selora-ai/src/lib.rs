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

mod client;
mod sse;
mod types;

pub use client::SeloraClient;
pub use sse::{parse_sse_event, SseEvent, MAX_DATA_FIELD_BYTES};
pub use types::{
    compute_cost_usd, ChatEvent, ChatMessage, ChatReceipt, ChatRequest, Model, ModelId, Role,
    SeloraStatus,
};
