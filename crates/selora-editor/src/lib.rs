//! Selora Editor — the custom-rendered editor pane.
//!
//! Responsibility (PLAN.md §5): editor widget — text paint, cursor,
//! selection, scroll, gestures, ghost text overlay, gutter, syntax
//! highlighting. Painted by hand on the [`egui::Painter`] canvas (UI_SPEC.md
//! §3.2) with per-glyph [`egui::text::LayoutJob`] sections (Phase 2d); egui
//! itself is used only for the surrounding chrome. No [`egui::TextEdit`],
//! no minimap — those land in later phases.
//!
//! Layout of this crate:
//! - [`Editor`] — the egui widget: input, painting, buffer access.
//! - [`syntax`] — tree-sitter highlighting: the Lite grammars, incremental
//!   re-parse on edit, SYN_* palette spans.
//! - `motion` (private module) — pure cursor-movement rules over a
//!   [`selora_core::Buffer`]; unit-tested headlessly.
//! - `layout` (private module) — pure geometry: click-to-position mapping,
//!   selection extents, scroll clamping; unit-tested headlessly.

#![forbid(unsafe_code)]

mod editor;
pub mod ghost_text;
mod layout;
pub mod lsp_overlay;
mod motion;
pub mod syntax;
mod theme;

pub use editor::Editor;
pub use syntax::{Language, LineSection, Span, SyntaxKind, SyntaxLayer};
