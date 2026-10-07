//! Selora IDE design tokens — the leaf source of truth.
//!
//! This is the typed mirror of `DESIGN_TOKENS.md` §11, hoisted out of
//! `selora-ui` so that `selora-editor` (which `selora-ui` depends on, per
//! PLAN.md §5 — a cargo cycle the other direction) can import the tokens
//! without mirroring them. `selora-ui`'s `theme` module re-exports everything
//! here, so the documented path `crates/selora-ui/src/theme.rs` remains the
//! module every UI component reads from (DESIGN_TOKENS.md §11).
//!
//! No component hardcodes a color, font, or spacing value. All values come
//! from here (DESIGN_TOKENS.md §12: the no-decoration rule).
//!
//! Only the constants live here. The font registration (`set_fonts`) and the
//! visuals mapping (`set_visuals`) need the egui context and the bundled font
//! files, so they stay in `selora-ui::theme`.

#![forbid(unsafe_code)]

pub mod color {
    use egui::Color32;

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

    // Syntax (DESIGN_TOKENS.md §1.5)
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

    /// Evaluate a cubic-bezier easing curve (DESIGN_TOKENS.md §7.1 control
    /// points; endpoints fixed at (0,0) and (1,1)) at linear progress `p`.
    ///
    /// Bisection on x(t): every token curve keeps its x control points in
    /// [0,1], so x(t) is monotone and 24 halvings pin y far tighter than a
    /// 1/255 alpha step needs.
    pub fn ease_bezier(curve: (f32, f32, f32, f32), p: f32) -> f32 {
        let (x1, y1, x2, y2) = curve;
        let p = p.clamp(0.0, 1.0);
        let coordinate = |t: f32, c1: f32, c2: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * c1 + 3.0 * u * t * t * c2 + t * t * t
        };
        let (mut low, mut high) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (low + high) / 2.0;
            if coordinate(mid, x1, x2) < p {
                low = mid;
            } else {
                high = mid;
            }
        }
        coordinate((low + high) / 2.0, y1, y2)
    }
}
