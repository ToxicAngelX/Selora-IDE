//! Selora Editor — the custom-rendered editor pane.
//!
//! Responsibility (PLAN.md §5): editor widget — text paint, cursor,
//! selection, scroll, gestures, ghost text overlay, gutter. Painted by hand
//! on the [`egui::Painter`] canvas (UI_SPEC.md §3.2); egui itself is used
//! only for the surrounding chrome. No [`egui::TextEdit`], no syntax
//! highlighting, no minimap — those land in later phases.
//!
//! Layout of this crate:
//! - [`Editor`] — the egui widget: input, painting, buffer access.
//! - `motion` (private module) — pure cursor-movement rules over a
//!   [`selora_core::Buffer`]; unit-tested headlessly.
//! - `layout` (private module) — pure geometry: click-to-position mapping,
//!   selection extents, scroll clamping; unit-tested headlessly.

#![forbid(unsafe_code)]

mod editor;
mod layout;
mod motion;
mod theme;

pub use editor::Editor;
