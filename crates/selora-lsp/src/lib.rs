//! Selora LSP — the Language Server Protocol client.
//!
//! Responsibility (PLAN.md §5): one actor per LSP server (rust-analyzer,
//! typescript-language-server) over stdio JSON-RPC, built on `lsp-types`
//! and `tokio`. Depends only on `selora-core`.
//!
//! Phase 0: crate skeleton only. Implementation lands in Phase 3.

#![forbid(unsafe_code)]
