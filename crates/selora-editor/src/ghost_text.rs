//! Ghost text — inline AI completions in the editor (UI_SPEC.md §3.5,
//! PLAN.md §6.4) — Phase 4d's file.
//!
//! The flow (ARCHITECTURE.md §2): editor idle 350ms → the shell sends the
//! up-to-caret prefix as a debounced effect → the reply lands as
//! `GhostTextReady(path, Option<String>)` → this module stores and paints
//! the suggestion at the caret in `--fg-faint` with a 2px `--ink-rule`
//! underline (the `comet` fade-in on first appearance). `Tab` accepts
//! (inserts into the buffer), any other keypress dismisses. The buffer
//! itself is never touched until acceptance.
//!
//! This file owns the `GhostText` state + paint + accept/dismiss gesture
//! handling inside the editor; the network half belongs to the shell
//! (`Effect`/`AppMessage`), not here.
