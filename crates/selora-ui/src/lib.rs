//! Selora UI — the egui chrome of Selora IDE.
//!
//! Responsibility (PLAN.md §5): top-level egui UI — panel layout, command
//! palette, file tree, chat sidebar, status bar, modals. Every color, font,
//! and spacing value comes from [`theme`] (the typed mirror of
//! `DESIGN_TOKENS.md`); no component hardcodes a visual value.
//!
//! Phase 0: design tokens, bundled font registration, the brand mark, and
//! the Selora dark visuals. Panels and widgets arrive in later phases.

#![forbid(unsafe_code)]

pub mod brand;
pub mod dialogs;
pub mod editor;
pub mod palette;
pub mod sidebar;
pub mod theme;
