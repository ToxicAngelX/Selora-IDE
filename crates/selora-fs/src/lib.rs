//! Selora FS — file tree, fuzzy finder, search.
//!
//! Responsibility (PLAN.md §5): file tree traversal (with `.gitignore`
//! support via the `ignore` crate), fuzzy find, search-in-files.
//!
//! Module layout (fixed by the orchestrator before Phase 2's parallel
//! subagents; `mod` declarations in this file are owned by the orchestrator):
//! - `tree` (Phase 2b) — directory walking → `FileTreeNode`.
//! - `fuzzy` (Phase 2c) — `nucleo-matcher` fuzzy matching.

#![forbid(unsafe_code)]

pub mod fuzzy;
pub mod tree;
