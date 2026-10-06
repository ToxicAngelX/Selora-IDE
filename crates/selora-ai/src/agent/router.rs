//! The tool router (ARCHITECTURE.md §17.1, §17.2) — Phase 5b's file.
//!
//! Maps a [`ToolCall`] to an implementation over `selora-core`,
//! `selora-fs`, runtime fs, and `tokio::process`. Read-type tools
//! execute immediately; `write_file`/`edit_file`/`delete_file`/non-
//! allowlisted `bash` return [`ToolOutcome::NeedsApproval`] and the loop
//! parks until the user decides.
//!
//! Every guard lives here and is unit-tested (path escape rejected,
//! localhost fetch rejected, oversize read rejected):
//! - **Path rooting** (§17.2): paths resolve against the workspace root;
//!   `..` traversal or absolute paths outside the root are rejected as a
//!   tool-result error the model can see and correct.
//! - **SSRF** (§17.2): `web_fetch` accepts only http/https and refuses
//!   localhost, `127.0.0.0/8`, `10.0.0.0/8`, `172.16.0.0/12`,
//!   `192.168.0.0/16`, `169.254.0.0/16`, and non-routable/reserved
//!   ranges.
//! - **Bounded I/O** (§17.2): reads ≤4MB; fetch and bash output ≤256KB,
//!   truncated with a notice.
//! - **Bash env** (§17.5): the child runs with a stripped environment —
//!   never the Selora key or the app's secrets.
//!
//! This skeleton (the prep commit) fixes only the signatures the other
//! subagents consume; 5b implements the tools and guards.

use std::path::Path;

use crate::chat_types::{ToolCall, ToolResult, ToolSpec};

use super::ApprovalCard;

/// What the router produced for one tool call: a finished result, or a
/// card the user must approve before execution proceeds.
#[allow(dead_code)] // skeleton — consumed when the Phase 5c loop lands
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutcome {
    /// The tool ran (read tools, auto-approved allowlisted bash) — carry
    /// the result straight into the conversation.
    Ready(ToolResult),
    /// The tool is side-effecting: build the card, park the loop
    /// (§17.4). Execution happens on the approval reply.
    NeedsApproval(ApprovalCard),
}

/// The fixed tool manifest (ARCHITECTURE.md §17.1 — the list is closed;
/// adding a tool is an architecture change, not a code change).
///
/// Stub until Phase 5b lands.
#[allow(dead_code)] // skeleton — consumed when the Phase 5c loop lands
pub fn tool_manifest() -> Vec<ToolSpec> {
    Vec::new()
}

/// Execute one tool call against the workspace `root`.
///
/// Stub until Phase 5b lands — every call resolves to an error result so
/// any early integration fails visibly, never silently.
#[allow(dead_code)] // skeleton — consumed when the Phase 5c loop lands
pub async fn execute_tool(root: &Path, call: ToolCall) -> ToolOutcome {
    let _ = root;
    ToolOutcome::Ready(ToolResult::error(
        call.id,
        format!("tool '{}' is not implemented yet (Phase 5b)", call.name),
    ))
}
