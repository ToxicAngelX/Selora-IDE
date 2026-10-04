//! Selora IDE design tokens.
//! Single source of truth — every UI component reads from here.
//!
//! This module is the typed mirror of `DESIGN_TOKENS.md`. §11 of that
//! document is the contract for this file; §12 (the no-decoration rule)
//! applies to every value exposed here.

use egui::Color32;

pub mod color {
    use super::Color32;

    pub const INK_CANVAS: Color32 = Color32::from_rgb(0, 0, 0);
    pub const INK_FAINT: Color32 = Color32::from_rgb(10, 10, 10);
    pub const INK_LOW: Color32 = Color32::from_rgb(20, 20, 20);
    pub const INK_MID: Color32 = Color32::from_rgb(31, 31, 31);
    pub const INK_HI: Color32 = Color32::from_rgb(42, 42, 42);
    pub const INK_RULE: Color32 = Color32::from_rgb(58, 58, 58);

    pub const FG_DEFAULT: Color32 = Color32::from_rgb(229, 229, 229);
    pub const FG_MUTED: Color32 = Color32::from_rgb(138, 138, 138);
    pub const FG_FAINT: Color32 = Color32::from_rgb(90, 90, 90);
    pub const FG_DISABLED: Color32 = Color32::from_rgb(58, 58, 58);

    pub const OK: Color32 = Color32::from_rgb(34, 197, 94);
    pub const WARN: Color32 = Color32::from_rgb(212, 160, 23);
    pub const ERR: Color32 = Color32::from_rgb(220, 38, 38);
    pub const INFO: Color32 = Color32::from_rgb(107, 114, 128);

    // Syntax
    pub const SYN_DEFAULT: Color32 = FG_DEFAULT;
    pub const SYN_KEYWORD: Color32 = Color32::from_rgb(207, 207, 207);
    pub const SYN_FN: Color32 = Color32::WHITE;
    pub const SYN_TYPE: Color32 = Color32::from_rgb(184, 184, 184);
    pub const SYN_STRING: Color32 = Color32::from_rgb(154, 154, 154);
    pub const SYN_NUMBER: Color32 = Color32::from_rgb(208, 208, 208);
    pub const SYN_COMMENT: Color32 = FG_FAINT;
    pub const SYN_PUNCT: Color32 = FG_MUTED;
    pub const SYN_ATTR: Color32 = Color32::from_rgb(168, 168, 168);
}

pub mod space {
    pub const S0: f32 = 0.0;
    pub const S1: f32 = 4.0;
    pub const S2: f32 = 8.0;
    pub const S3: f32 = 12.0;
    pub const S4: f32 = 16.0;
    pub const S5: f32 = 24.0;
    pub const S6: f32 = 32.0;
}

pub mod radius {
    pub const R0: f32 = 0.0;
    pub const R1: f32 = 2.0;
    pub const R2: f32 = 4.0;
    pub const PILL: f32 = 999.0;
}

pub mod type_scale {
    pub const XS: f32 = 11.0;
    pub const SM: f32 = 12.0;
    pub const MD: f32 = 13.0;
    pub const LG: f32 = 15.0;
    pub const XL: f32 = 18.0;
    pub const XXL: f32 = 24.0;
}

pub mod font {
    use egui::FontFamily;

    pub const SANS: &str = "Space Grotesk";
    pub const MONO: &str = "JetBrains Mono";

    /// egui family backed by the bundled Space Grotesk Regular.
    pub const FAMILY_SANS: FontFamily = FontFamily::Proportional;
    /// egui family backed by the bundled JetBrains Mono Regular.
    pub const FAMILY_MONO: FontFamily = FontFamily::Monospace;

    /// Named family backed by the bundled Space Grotesk Medium (`--weight-medium`, 500).
    pub fn family_sans_medium() -> FontFamily {
        FontFamily::Name("SpaceGrotesk-Medium".into())
    }

    /// Named family backed by the bundled Space Grotesk SemiBold (`--weight-semibold`, 600).
    pub fn family_sans_semibold() -> FontFamily {
        FontFamily::Name("SpaceGrotesk-SemiBold".into())
    }

    /// Named family backed by the bundled JetBrains Mono Medium.
    pub fn family_mono_medium() -> FontFamily {
        FontFamily::Name("JetBrainsMono-Medium".into())
    }
}

pub mod motion {
    pub const DUR_INSTANT_MS: u32 = 80;
    pub const DUR_QUICK_MS: u32 = 160;
    pub const DUR_BASE_MS: u32 = 240;
    pub const DUR_LOOP_MS: u32 = 1200;
    pub const DUR_LONG_MS: u32 = 1800;

    // easing curves as tuples for egui / manual interpolation
    pub const EASE_PRECISE: (f32, f32, f32, f32) = (0.2, 0.0, 0.0, 1.0);
    pub const EASE_CALM: (f32, f32, f32, f32) = (0.4, 0.0, 0.2, 1.0);
    pub const EASE_ORBITAL: (f32, f32, f32, f32) = (0.65, 0.0, 0.35, 1.0);
    pub const EASE_FADE: (f32, f32, f32, f32) = (0.4, 0.0, 0.6, 1.0);
}

// -----------------------------------------------------------------------------
// Bundled fonts (DESIGN_TOKENS.md §2.1: both families are bundled in
// `assets/fonts/`; we do not download at runtime and do not depend on the
// OS having them).
// -----------------------------------------------------------------------------

const SPACE_GROTESK_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/SpaceGrotesk-Regular.ttf");
const SPACE_GROTESK_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/SpaceGrotesk-Medium.ttf");
const SPACE_GROTESK_SEMIBOLD: &[u8] =
    include_bytes!("../../../assets/fonts/SpaceGrotesk-SemiBold.ttf");
const JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
const JETBRAINS_MONO_MEDIUM: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Medium.ttf");

/// Register the bundled Space Grotesk + JetBrains Mono with egui.
/// Called once on app boot.
///
/// Space Grotesk Regular becomes the primary proportional face and JetBrains
/// Mono Regular the primary monospace face (each with the egui built-ins
/// kept as glyph fallbacks). The Medium / SemiBold weights are registered as
/// named egui families — see [`font::family_sans_medium`] and friends —
/// because egui resolves weight via family, not via a `FontId` weight field.
pub fn set_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fn put(
        fonts: &mut egui::FontDefinitions,
        name: &str,
        bytes: &'static [u8],
        family: egui::FontFamily,
        primary: bool,
    ) {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
        let family_fonts = fonts.families.entry(family).or_default();
        if primary {
            family_fonts.insert(0, name.to_owned());
        } else {
            family_fonts.push(name.to_owned());
        }
    }

    put(
        &mut fonts,
        "SpaceGrotesk-Regular",
        SPACE_GROTESK_REGULAR,
        font::FAMILY_SANS,
        true,
    );
    put(
        &mut fonts,
        "SpaceGrotesk-Medium",
        SPACE_GROTESK_MEDIUM,
        font::family_sans_medium(),
        false,
    );
    put(
        &mut fonts,
        "SpaceGrotesk-SemiBold",
        SPACE_GROTESK_SEMIBOLD,
        font::family_sans_semibold(),
        false,
    );
    put(
        &mut fonts,
        "JetBrainsMono-Regular",
        JETBRAINS_MONO_REGULAR,
        font::FAMILY_MONO,
        true,
    );
    put(
        &mut fonts,
        "JetBrainsMono-Medium",
        JETBRAINS_MONO_MEDIUM,
        font::family_mono_medium(),
        false,
    );

    ctx.set_fonts(fonts);
}

/// Apply the Selora dark visuals to the context. Called once on app boot.
///
/// Every value below is a token from [`color`] — nothing is invented here.
/// Surfaces not yet painted (menus, dropdowns, toasts) inherit these so they
/// land on-brand when later phases build them.
pub fn set_visuals(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    // Surfaces: workbench canvas, modal/dropdown fill, deepest edit field.
    visuals.panel_fill = color::INK_CANVAS;
    visuals.window_fill = color::INK_MID;
    visuals.extreme_bg_color = color::INK_LOW;
    visuals.faint_bg_color = color::INK_FAINT;

    // Hairlines: 1px, --ink-hi (DESIGN_TOKENS.md §5).
    visuals.window_stroke = egui::Stroke::new(1.0, color::INK_HI);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, color::INK_HI);

    // Selection: one ink-step lift, no saturated blue (UI_SPEC.md §3.2).
    visuals.selection.bg_fill = color::INK_MID;
    visuals.selection.stroke = egui::Stroke::new(1.0, color::FG_DEFAULT);

    // Text colors on the ink ramp.
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, color::FG_MUTED);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, color::FG_DEFAULT);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, color::FG_DEFAULT);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, color::FG_DEFAULT);
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, color::FG_DEFAULT);

    // Hover / active surfaces per DESIGN_TOKENS.md §10 (state matrix).
    visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
    visuals.widgets.hovered.bg_fill = color::INK_FAINT;
    visuals.widgets.active.bg_fill = color::INK_LOW;
    visuals.widgets.open.bg_fill = color::INK_LOW;

    // Text cursor: 2px, --fg-default, 500ms blink (UI_SPEC.md §3.2 — the only
    // blink in the app; the default on/off durations are already 0.5s).
    visuals.text_cursor.stroke = egui::Stroke::new(2.0, color::FG_DEFAULT);

    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals);
    ctx.set_style_of(egui::Theme::Dark, style());
}

fn style() -> egui::Style {
    egui::Style {
        text_styles: text_styles(),
        ..Default::default()
    }
}

/// The type scale (DESIGN_TOKENS.md §2.2) mapped onto egui's text-style
/// roles, with the default body size at `--type-md` (13px).
fn text_styles() -> std::collections::BTreeMap<egui::TextStyle, egui::FontId> {
    use egui::TextStyle::*;

    [
        // --type-lg, weight 500: headings, modal titles, panel titles.
        (
            Heading,
            egui::FontId::new(type_scale::LG, font::family_sans_medium()),
        ),
        // --type-md: default body, menus.
        (Body, egui::FontId::new(type_scale::MD, font::FAMILY_SANS)),
        // --type-md: buttons.
        (Button, egui::FontId::new(type_scale::MD, font::FAMILY_SANS)),
        // --type-sm: secondary UI.
        (Small, egui::FontId::new(type_scale::SM, font::FAMILY_SANS)),
        // --type-md: editor / code / terminal-style values.
        (
            Monospace,
            egui::FontId::new(type_scale::MD, font::FAMILY_MONO),
        ),
    ]
    .into_iter()
    .collect()
}
