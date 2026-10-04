//! Selora Core — the lowest layer of the Selora IDE workspace.
//!
//! Responsibility (PLAN.md §5): `Buffer` (ropey wrapper), position types,
//! buffer identity, and the error enum every crate above reuses. Pure logic:
//! no IO, no async, no UI (ARCHITECTURE.md §10.1).
//!
//! `Workspace` and undo/redo land in Phase 2 (ARCHITECTURE.md §3.3, §3.4);
//! the Phase 1 spike only needs the buffer + error contract.

#![forbid(unsafe_code)]

mod buffer;
mod error;
mod workspace;

pub use buffer::{Buffer, BufferId, Position};
pub use error::SeloraError;
pub use workspace::Workspace;
