//! Ghost text inline completion — the editor side of Phase 4d.
//!
//! The editor-side half of the ghost-text flow (PLAN.md §6.4): the shell
//! debounces edit input for 350 ms, sends the up-to-caret prefix to
//! `selora-ai::ghost_complete`, and hands the reply to
//! [`crate::Editor::set_ghost`] when it lands. From there this module owns
//! everything:
//!
//! - **state** — [`GhostText`] holds the suggestion, the caret position the
//!   request was made at (a reply for a moved caret is stale and dropped),
//!   and the "first appearance" clock the comet animation plays on;
//! - **rendering** — [`paint_ghost`] paints the suggestion at the caret in
//!   `--fg-faint` with a 2px `--ink-rule` underline beneath the ghosted span
//!   (UI_SPEC.md §3.5), without touching the buffer. Multi-line suggestions
//!   wrap naturally: the first row continues from the caret, later rows start
//!   at column 0, each on its own line band;
//! - **the comet** — a one-shot fade-in + streak sweep on first appearance
//!   (DESIGN_TOKENS.md §7.3 `comet`: `--dur-base` 240ms, `--ease-precise`
//!   for the streak's travel, `--ease-fade` for the text alpha). egui's
//!   repaint model is pull-driven, so the paint loop drives the clock from
//!   the frame time (`input.time`) and [`GhostText::needs_repaint`] tells
//!   the widget to `request_repaint_after` until the animation settles —
//!   the same pattern as the caret blink and the hover-hold gesture.
//!
//! The suggestion is paint-only until Tab accepts it (ARCHITECTURE.md §9.2);
//! acceptance, dismissal, and the idle seam live in `editor.rs`.

use std::time::Duration;

use egui::{pos2, Color32, FontId, Painter, Stroke};
use selora_core::Position;
use selora_tokens::motion;

use crate::layout;
use crate::theme;

/// The comet's duration (DESIGN_TOKENS.md §7.3: `comet` runs on
/// `--dur-base`). The spec's prose says "a 200ms streak" while its keyframes
/// bind `var(--dur-base)` (240ms); the token wins.
pub const COMET_DURATION: Duration = Duration::from_millis(motion::DUR_BASE_MS as u64);

/// How far the comet's streak trails behind its head (px). One `--space-5`
/// (DESIGN_TOKENS.md §3) — a short streak, not a banner.
const COMET_TRAIL: f32 = 24.0;

/// The underline beneath the ghosted span (UI_SPEC.md §3.5: 2px).
const GHOST_UNDERLINE_HEIGHT: f32 = 2.0;

/// The comet's travel easing, applied to the streak's progress (§7.3).
const EASE_PRECISE: (f32, f32, f32, f32) = motion::EASE_PRECISE;
/// The fade-in easing, applied to the ghost text's alpha (§7.3 `fade`).
const EASE_FADE: (f32, f32, f32, f32) = motion::EASE_FADE;

/// One ghost suggestion, ready to paint.
///
/// Built by [`crate::Editor::set_ghost`] when a reply lands on an unmoved
/// caret; consumed by [`crate::Editor::accept_ghost`] on Tab. The comet
/// clock (`first_seen`) stays `None` until the first paint — that is the
/// "first appearance" the animation plays on, so a ghost that arrives while
/// its pane is hidden doesn't burn its animation off-screen.
#[derive(Debug, Clone, PartialEq)]
pub struct GhostText {
    suggestion: String,
    /// The caret position the shell captured when it *sent* the request.
    /// The editor compares it against the live caret: a reply landing after
    /// the caret moved is stale and never becomes a ghost.
    request_caret: Position,
    /// When this ghost was first painted (egui frame clock, `input.time`).
    /// `None` until the first [`GhostText::frame`] call.
    first_seen: Option<f64>,
}

impl GhostText {
    /// A new ghost for `suggestion`, requested at `request_caret`.
    pub fn new(suggestion: String, request_caret: Position) -> Self {
        Self {
            suggestion,
            request_caret,
            first_seen: None,
        }
    }

    /// The suggestion text (the thing Tab inserts).
    pub fn suggestion(&self) -> &str {
        &self.suggestion
    }

    /// Consume the ghost, yielding the suggestion for insertion.
    pub fn into_suggestion(self) -> String {
        self.suggestion
    }

    /// Whether this ghost is stale: the caret has moved since the request
    /// was made, so the reply no longer matches the document.
    pub fn is_stale(&self, caret: Position) -> bool {
        self.request_caret != caret
    }

    /// Advance the animation clock to `now` and compute this frame's comet
    /// values. The first call stamps `first_seen` (the animation starts at
    /// the ghost's first appearance, not at its arrival).
    pub(crate) fn frame(&mut self, now: f64) -> GhostFrame {
        let start = *self.first_seen.get_or_insert(now);
        let progress =
            ((now - start).max(0.0) / COMET_DURATION.as_secs_f64()).clamp(0.0, 1.0) as f32;
        GhostFrame {
            text_alpha: ease_bezier(EASE_FADE, progress),
            streak_progress: ease_bezier(EASE_PRECISE, progress),
        }
    }

    /// How much longer the comet needs frames, or `None` once settled.
    /// The widget feeds this straight into
    /// `ui.ctx().request_repaint_after` (the repo's existing animation
    /// pattern — see the caret blink and hover-hold in `editor.rs`).
    pub(crate) fn needs_repaint(&self, now: f64) -> Option<Duration> {
        let start = self.first_seen?;
        let elapsed = Duration::from_secs_f64((now - start).max(0.0));
        COMET_DURATION.checked_sub(elapsed)
    }
}

/// One frame of the comet: the text's fade alpha and the streak's travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GhostFrame {
    /// 0..1 — the ghost text's opacity this frame (`fade`).
    pub(crate) text_alpha: f32,
    /// 0..1 — how far the streak's head has traveled (`precise`).
    pub(crate) streak_progress: f32,
}

impl GhostFrame {
    /// The streak's opacity at this progress: the comet keyframes are
    /// `0% opacity 0`, `20% opacity 1`, `100% opacity 0` (DESIGN_TOKENS
    /// §7.3) — brighten fast, fade to nothing.
    pub(crate) fn streak_alpha(&self) -> f32 {
        if self.streak_progress < 0.2 {
            self.streak_progress / 0.2
        } else {
            1.0 - (self.streak_progress - 0.2) / 0.8
        }
        .clamp(0.0, 1.0)
    }
}

/// Where the ghost paints, all in screen coordinates. The editor derives
/// this from the caret's painted position; the pure geometry lives here so
/// the paint math is one place.
pub(crate) struct GhostGeometry {
    /// x where the caret sits — the first ghost row continues from here.
    pub(crate) caret_x: f32,
    /// x of column 0 — continuation rows start here.
    pub(crate) line_start_x: f32,
    /// Top of the caret's line band.
    pub(crate) row_top: f32,
    /// How many line bands below the caret's are still on-screen
    /// (`0` = only the caret's row is visible).
    pub(crate) rows_below: usize,
}

/// Paint the ghost: the suggestion in `--fg-faint` at the caret, a 2px
/// `--ink-rule` underline beneath each painted row, and (while the comet
/// runs) a `--fg-muted` streak sweeping across the first row. Paint-only —
/// nothing here touches the buffer (ARCHITECTURE.md §9.2).
pub(crate) fn paint_ghost(
    painter: &Painter,
    ghost: &mut GhostText,
    now: f64,
    geometry: GhostGeometry,
    font: &FontId,
) {
    let frame = ghost.frame(now);
    let text_color = with_alpha(theme::color::FG_FAINT, frame.text_alpha);
    let underline_color = with_alpha(theme::color::INK_RULE, frame.text_alpha);

    // Multi-line suggestions wrap naturally as painted text: row 0 continues
    // from the caret, later rows each take a full line band from column 0.
    let lines: Vec<&str> = ghost.suggestion().split('\n').collect();
    let mut first_row_width = 0.0f32;
    for (row, line) in lines.iter().enumerate() {
        if row > geometry.rows_below {
            break; // the rest is below the viewport; the painter would clip it anyway
        }
        let x = if row == 0 {
            geometry.caret_x
        } else {
            geometry.line_start_x
        };
        let y = geometry.row_top + row as f32 * layout::LINE_HEIGHT;
        let galley = painter.layout((*line).to_owned(), font.clone(), text_color, f32::INFINITY);
        let galley_y = y + (layout::LINE_HEIGHT - galley.size().y) / 2.0;
        let width = galley.size().x;
        if row == 0 {
            first_row_width = width;
        }
        painter.galley(pos2(x, galley_y), galley, text_color);

        // The 2px underline beneath the ghosted span (UI_SPEC.md §3.5).
        if width > 0.0 {
            painter.rect_filled(
                egui::Rect::from_min_size(
                    pos2(x, y + layout::LINE_HEIGHT - GHOST_UNDERLINE_HEIGHT),
                    egui::vec2(width, GHOST_UNDERLINE_HEIGHT),
                ),
                theme::radius::R0,
                underline_color,
            );
        }
    }

    // The comet's streak: a short bright line whose head sweeps from just
    // before the suggestion's start to just past its end, fading to nothing
    // (DESIGN_TOKENS.md §7.3). Only during the animation, only over the
    // first row — the suggestion "arrives" along its first line.
    if frame.streak_progress < 1.0 && first_row_width > 0.0 {
        let span_start = geometry.caret_x;
        let span_end = geometry.caret_x + first_row_width;
        let head =
            span_start - COMET_TRAIL + frame.streak_progress * (first_row_width + COMET_TRAIL);
        let head = head.min(span_end);
        let tail = (head - COMET_TRAIL).max(span_start);
        if head > tail {
            let y = geometry.row_top + layout::LINE_HEIGHT / 2.0;
            painter.line_segment(
                [pos2(tail, y), pos2(head, y)],
                Stroke::new(
                    1.0,
                    with_alpha(theme::color::FG_MUTED, frame.streak_alpha()),
                ),
            );
        }
    }
}

/// `base` with its alpha set to `alpha` (0..1). The theme constants are
/// opaque; the fade-in is applied at paint time so no new color values
/// enter the system (DESIGN_TOKENS.md §12: all values from the tokens).
fn with_alpha(base: Color32, alpha: f32) -> Color32 {
    let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha)
}

/// Evaluate a cubic-bezier easing curve (DESIGN_TOKENS.md §7.1). The
/// evaluator lives in `selora_tokens::motion` (shared with selora-ui's
/// animated widgets); this alias keeps the module's call sites and tests
/// reading locally.
fn ease_bezier(curve: (f32, f32, f32, f32), p: f32) -> f32 {
    selora_tokens::motion::ease_bezier(curve, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ghost() -> GhostText {
        GhostText::new("    Theme::from_tokens(TOKENS)".into(), Position::new(2, 4))
    }

    /// The comet plays once, on first appearance: alpha ramps 0 → 1 over
    /// the 240ms token duration, and the clock starts at the first *paint*,
    /// not at construction.
    #[test]
    fn comet_fades_in_over_the_base_duration() {
        let mut ghost = ghost();
        // Constructed but never painted: the clock hasn't started.
        assert_eq!(ghost.needs_repaint(99.0), None);

        let half = COMET_DURATION.as_secs_f64() / 2.0;
        let first = ghost.frame(10.0);
        assert!(
            first.text_alpha.abs() < 1e-6,
            "the frame that stamps the clock is t=0"
        );
        assert_eq!(ghost.needs_repaint(10.0), Some(COMET_DURATION));

        let mid = ghost.frame(10.0 + half);
        assert!(
            mid.text_alpha > 0.0 && mid.text_alpha < 1.0,
            "mid-animation alpha: {}",
            mid.text_alpha
        );

        let done = ghost.frame(10.0 + COMET_DURATION.as_secs_f64() + 1.0);
        assert!((done.text_alpha - 1.0).abs() < 1e-6);
        assert!((done.streak_progress - 1.0).abs() < 1e-6);
        assert_eq!(
            ghost.needs_repaint(10.0 + COMET_DURATION.as_secs_f64() + 1.0),
            None
        );
    }

    /// The streak's opacity follows the comet keyframes: 0 at the start,
    /// 1 at 20% travel, back to 0 by the end.
    #[test]
    fn streak_opacity_peaks_early_and_fades_to_nothing() {
        let frame = |progress: f32| GhostFrame {
            text_alpha: 1.0,
            streak_progress: progress,
        };
        assert_eq!(frame(0.0).streak_alpha(), 0.0);
        assert!((frame(0.2).streak_alpha() - 1.0).abs() < 1e-6);
        assert!(frame(0.1).streak_alpha() > 0.45 && frame(0.1).streak_alpha() < 0.55);
        assert!(frame(0.6).streak_alpha() > 0.45 && frame(0.6).streak_alpha() < 0.55);
        assert_eq!(frame(1.0).streak_alpha(), 0.0);
        // Out-of-range progress clamps rather than extrapolating.
        assert_eq!(frame(-1.0).streak_alpha(), 0.0);
    }

    /// The bezier evaluator: exact endpoints, monotone in between, and the
    /// token curves' character (precise is front-loaded, fade is not).
    #[test]
    fn bezier_easing_hits_endpoints_and_curves_between() {
        for &curve in &[
            EASE_PRECISE,
            EASE_FADE,
            motion::EASE_CALM,
            motion::EASE_ORBITAL,
        ] {
            assert!(ease_bezier(curve, 0.0).abs() < 1e-6);
            assert!((ease_bezier(curve, 1.0) - 1.0).abs() < 1e-6);
            let mut last = 0.0f32;
            for step in 1..=20 {
                let p = step as f32 / 20.0;
                let eased = ease_bezier(curve, p);
                assert!(eased >= last, "monotone: {eased} < {last}");
                last = eased;
            }
        }
        // --ease-precise (0.2, 0, 0, 1) front-loads its motion.
        assert!(ease_bezier(EASE_PRECISE, 0.25) > 0.5);
    }

    /// Staleness is caret comparison, nothing more.
    #[test]
    fn ghost_is_stale_only_when_the_caret_moved() {
        let ghost = ghost();
        assert!(!ghost.is_stale(Position::new(2, 4)));
        assert!(ghost.is_stale(Position::new(2, 5)));
        assert!(ghost.is_stale(Position::new(3, 4)));
        assert_eq!(ghost.suggestion(), "    Theme::from_tokens(TOKENS)");
        assert_eq!(ghost.clone().into_suggestion(), ghost.suggestion());
    }

    /// The full paint path runs headlessly on egui's test UI (no fonts, no
    /// window): a multi-line ghost paints without panicking and without
    /// advancing past the viewport.
    #[test]
    fn ghost_painting_runs_headlessly() {
        let mut ghost = GhostText::new("line one\nline two\nline three".into(), Position::start());
        egui::__run_test_ui(|ui| {
            let painter = ui.painter();
            paint_ghost(
                painter,
                &mut ghost,
                0.0,
                GhostGeometry {
                    caret_x: 60.0,
                    line_start_x: 44.0,
                    row_top: 0.0,
                    rows_below: 1, // only two of the three rows fit
                },
                &FontId::new(layout::FONT_SIZE, theme::font::FAMILY_MONO),
            );
        });
        // The comet clock was stamped by the paint.
        assert!(ghost.frame(0.0 + COMET_DURATION.as_secs_f64()).text_alpha > 0.9);
    }
}
