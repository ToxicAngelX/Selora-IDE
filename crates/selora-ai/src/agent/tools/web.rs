//! Web tools (ARCHITECTURE.md §17.1) — Phase 5e's file.
//!
//! `web_search` — DuckDuckGo Lite HTML by default (no API key; Brave
//! Search API optional via a settings key). `web_fetch` — reqwest GET,
//! HTML stripped to text, 256KB cap, SSRF-guarded (§17.2: only
//! http/https; localhost, private, and reserved ranges refused).
//!
//! This is the only Phase 5 file allowed to add an HTTP dependency
//! outside `SeloraClient` (AGENT_ORCHESTRATION.md §10).
//!
//! Skeleton (the prep commit) — signatures only until 5e lands.

use crate::chat_types::ToolResult;

/// `web_search(query)` — the numbered result list the chat renders as a
/// web card (title + URL per hit).
///
/// Stub until Phase 5e lands.
#[allow(dead_code)] // skeleton — called by the 5b router when it lands
pub async fn web_search(_query: &str) -> ToolResult {
    ToolResult::error("", "web_search is not implemented yet (Phase 5e)")
}

/// `web_fetch(url)` — the URL's text, ≤256KB, truncated with a notice.
///
/// Stub until Phase 5e lands.
#[allow(dead_code)] // skeleton — called by the 5b router when it lands
pub async fn web_fetch(_url: &str) -> ToolResult {
    ToolResult::error("", "web_fetch is not implemented yet (Phase 5e)")
}
