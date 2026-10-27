//! Selora IDE design tokens.
//! Single source of truth — every UI component reads from here.
//!
//! The constants live in the leaf crate `selora-tokens` (the typed
//! DESIGN_TOKENS.md §11 contract) and are re-exported below, so that both
//! `selora-ui` (chrome) and `selora-editor` (the pane `selora-ui` depends on)
//! share one source of truth instead of mirrors. This module additionally
//! owns the pieces that need the egui context and the bundled font files:
//! [`set_fonts`] and [`set_visuals`], both called once on app boot.
//! DESIGN_TOKENS.md §12 (the no-decoration rule) applies to every value.

pub use selora_tokens::{color, font, motion, radius, space, type_scale};

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
    ctx.set_fonts(build_fonts());
}

/// Build the font definitions (split from `set_fonts` so tests can inspect
/// the fallback chains without a context).
fn build_fonts() -> egui::FontDefinitions {
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

    // Symbol coverage: Space Grotesk has no geometric shapes (●/○ — the
    // status run's dots, UI_SPEC.md §1.2) and the egui built-ins below it in
    // the chain cover ○ but not ● (a tofu box shipped once). JetBrains Mono
    // has both; insert it as the sans fallback so symbols resolve in one
    // face instead of split-rendering across families.
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        if let Some(pos) = family
            .iter()
            .position(|name| name == "SpaceGrotesk-Regular")
        {
            family.insert(pos + 1, "JetBrainsMono-Regular".to_owned());
        }
    }

    fonts
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

#[cfg(test)]
mod tests {
    /// The status run's dots (● U+25CF / ○ U+25CB, UI_SPEC.md §1.2) are not
    /// in Space Grotesk; they resolve through the sans fallback chain. If
    /// JetBrains Mono ever drops out of that chain, ● renders as tofu.
    #[test]
    fn sans_fallback_resolves_geometric_dots_in_jetbrains_mono() {
        let fonts = super::build_fonts();
        let sans = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .expect("sans family");
        let sg = sans
            .iter()
            .position(|n| n == "SpaceGrotesk-Regular")
            .expect("Space Grotesk is the primary sans");
        let jb = sans
            .iter()
            .position(|n| n == "JetBrainsMono-Regular")
            .expect("JetBrains Mono is the sans symbol fallback");
        assert!(sg < jb, "JetBrains Mono follows Space Grotesk");
    }
}
