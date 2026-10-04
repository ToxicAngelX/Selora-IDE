//! Selora Core — the lowest layer of the Selora IDE workspace.
//!
//! Responsibility (PLAN.md §5): `Buffer` (ropey wrapper), `Workspace`,
//! `PathId`, position types, undo/redo. Pure logic: no IO, no async, no UI.
//!
//! Phase 0: crate skeleton only. Implementation lands in Phase 1–2.

#![forbid(unsafe_code)]
