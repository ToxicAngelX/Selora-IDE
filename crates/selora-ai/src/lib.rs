//! Selora AI — the client for the Selora gateway (`api.selora.lol`).
//!
//! Responsibility (PLAN.md §5): `GET /v1/models`,
//! `POST /v1/chat/completions` (streaming + non-streaming), the ghost-text
//! provider, and the cost meter. Depends only on `selora-core` and
//! `selora-settings`.
//!
//! Phase 0: crate skeleton only. Implementation lands in Phase 1 (spike) and
//! Phase 4.

#![forbid(unsafe_code)]
