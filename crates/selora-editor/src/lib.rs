//! Selora Editor — the custom-rendered editor pane.
//!
//! Responsibility (PLAN.md §5): editor widget — text paint, cursor,
//! selection, scroll, gestures, ghost text overlay, gutter. Painted by hand
//! on the egui canvas; egui itself is used only for the surrounding chrome.
//!
//! Phase 0: crate skeleton only. Implementation lands in Phase 2.

#![forbid(unsafe_code)]
