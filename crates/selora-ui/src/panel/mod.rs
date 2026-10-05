//! Selora UI — dockable bottom/side panels (UI_SPEC.md §8 git, §9 search).
//!
//! Phase 3 orchestrator stub: `git` (subagent 3b) and `search` (subagent 3c)
//! each own one module file; this `mod.rs` only wires them in.

#![forbid(unsafe_code)]

pub mod git;
pub mod search;
