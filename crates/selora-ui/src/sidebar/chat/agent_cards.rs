//! Agent Mode cards in the chat sidebar (UI_SPEC.md §4.2) — Phase 5d's
//! file.
//!
//! Everything the agent loop paints between bubbles: the collapsed
//! read-type tool card, the diff approval card (unified diff, `-`/`+`
//! in `--err`/`--ok` — the chat's only saturated uses, semantic), the
//! bash command card (live output, exit status, Stop while running),
//! the web cards, the agent status line (`thinking` / `reading …` /
//! `awaiting your approval` / `running …` + Stop), and the turn summary
//! footer.
//!
//! The state arrives as [`crate::dialogs`]-style events reduced into the
//! panel: `AppMessage::ToolCallStarted` / [`ToolResult`] /
//! [`ApprovalNeeded`] / [`AgentTurnDone`] (§17.6). Decisions go back out
//! as `Effect::AgentDecision`.
//!
//! Skeleton (the prep commit) — the rendering lands with 5d. The panel
//! wiring in [`super::mod`] routes events here once the loop is live.

use egui::Ui;

/// Paint the agent status line (UI_SPEC.md §4.2): the 24px line under
/// the last bubble while a turn runs — current state + the Stop button
/// on the right. `phase` is the state-line text (`thinking`, `reading
/// src/main.rs`, `awaiting your approval`, `running cargo test`).
///
/// Stub until Phase 5d lands.
pub fn status_line(_ui: &mut Ui, _phase: &str) {}
