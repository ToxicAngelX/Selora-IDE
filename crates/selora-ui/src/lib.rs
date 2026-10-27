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
pub mod diff_view;
pub mod editor;
pub mod empty_state;
pub mod palette;
pub mod panel;
pub mod picker;
pub mod presence;
pub mod sidebar;
pub mod splash;
pub mod status_bar;
pub mod theme;
pub mod title_bar;
pub mod toasts;
