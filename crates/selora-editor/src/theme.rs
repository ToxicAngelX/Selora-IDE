//! Design tokens for the editor pane.
//!
//! **This module is a value-exact mirror of `crates/selora-ui/src/theme.rs`
//! (DESIGN_TOKENS.md §11).** It exists because of the inter-crate dependency
//! direction in PLAN.md §5: `selora-ui` depends on `selora-editor`, so
//! `selora-editor` cannot depend on `selora-ui` (cargo rejects the cycle),
//! yet the editor pane must paint with the theme tokens and nothing else.
//!
//! Every constant below is copied verbatim from the selora-ui theme module
//! with the same name and value; keep the two in sync (or, in a later phase,
//! hoist the tokens into a leaf crate both can import — see RISKS.md §1).
//!
//! Only the tokens the editor needs are mirrored. Anything not here is not
//! used by this crate.

pub mod color {
    use egui::Color32;

    pub const INK_CANVAS: Color32 = Color32::from_rgb(0, 0, 0);
    pub const INK_FAINT: Color32 = Color32::from_rgb(10, 10, 10);
    pub const INK_MID: Color32 = Color32::from_rgb(31, 31, 31);
    pub const INK_RULE: Color32 = Color32::from_rgb(58, 58, 58);

    pub const FG_DEFAULT: Color32 = Color32::from_rgb(229, 229, 229);
    pub const FG_FAINT: Color32 = Color32::from_rgb(90, 90, 90);
}

pub mod radius {
    pub const R0: f32 = 0.0;
}

pub mod type_scale {
    pub const SM: f32 = 12.0;
}

pub mod font {
    use egui::FontFamily;

    /// egui family backed by the bundled JetBrains Mono Regular
    /// (`FontFamily::Monospace`; registered by `selora_ui::theme::set_fonts`).
    /// Mirrors `selora_ui::theme::font::FAMILY_MONO`.
    pub const FAMILY_MONO: FontFamily = FontFamily::Monospace;
}
