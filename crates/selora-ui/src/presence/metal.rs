//! `MetalMark` — the liquid-metal sheen on the splash's brand mark
//! (Phase 6h; a native port of the metal-fx web library's wandering
//! halo). **OWNER-DECISION ITEM**: shipped behind
//! [`METAL_MARK_ENABLED`] (default `false`); the owner flips the flag or
//! this file is deleted.
//!
//! The web version runs a WebGL liquid-metal shader; the port is a sheen
//! band swept across the mark, built on the §7.3 `comet` vocabulary
//! (ease-precise travel, the comet alpha envelope) — monochrome white on
//! ink, the single permitted specular highlight in the app. Do not extend
//! it anywhere else.
//!
//! Cadence: one `--dur-loop` sweep every four loops (the sweep itself is
//! 1200ms — inside §7.2's 2000ms ceiling; the idle gap between sweeps is
//! not an animation duration). The splash already repaints at its orbit
//! cadence, so the sheen costs no extra frames; under reduce-motion (the
//! comet family is §7.4-disabled) the caller paints nothing.

use egui::{pos2, vec2, Painter, Rect, Stroke};

use crate::theme::motion;

use super::LOOP_S;

/// The owner-decision flag (Phase 6h brief): `false` = the sheen never
/// paints. Wired in `splash.rs` behind this constant.
pub const METAL_MARK_ENABLED: bool = true; // the owner enabled it (Phase R7)

/// One sweep every four loops: calm, not a strobe.
const SWEEP_PERIOD_S: f32 = 4.0 * LOOP_S;
/// The sheen band's tilt — the brand mark's ring tilt (§9), so the sweep
/// reads as light running along the ring.
const SHEEN_TILT_DEG: f32 = 20.0;
/// Peak alpha of the sheen's core hairline (a sheen, not a flare).
const PEAK_ALPHA: f32 = 0.30;

/// One frame of the sheen, as data (tests assert on it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheenFrame {
    /// The band's horizontal position across the mark (0 = left edge,
    /// 1 = right edge), eased `--ease-precise`.
    pub x: f32,
    /// The core hairline's alpha (the comet envelope).
    pub alpha: f32,
}

/// The sheen's frame at `elapsed_s` (the splash's orbit clock), or `None`
/// during the idle three loops of the period.
pub fn sheen_frame(elapsed_s: f32) -> Option<SheenFrame> {
    let phase = (elapsed_s / SWEEP_PERIOD_S).rem_euclid(1.0);
    if phase >= 0.25 {
        return None; // the idle three loops
    }
    let sweep = phase * 4.0; // 0..1 over the sweep loop
    let x = motion::ease_bezier(motion::EASE_PRECISE, sweep);
    // The §7.3 comet envelope: 0% → 0, 20% → 1, 100% → 0.
    let alpha = if sweep < 0.2 {
        sweep / 0.2
    } else {
        1.0 - (sweep - 0.2) / 0.8
    }
    .clamp(0.0, 1.0);
    Some(SheenFrame {
        x,
        alpha: alpha * PEAK_ALPHA,
    })
}

/// Paint the sheen across `mark_rect` (the splash's 64px mark). Three
/// parallel 1px hairlines, stepped alpha (the §12-lawful analogue of a
/// soft band — no gradient), tilted at the ring's 20°, clipped to the
/// mark's rect. A no-op during the idle gap or at alpha 0.
pub fn paint_sheen(painter: &Painter, mark_rect: Rect, elapsed_s: f32) {
    let Some(frame) = sheen_frame(elapsed_s) else {
        return;
    };
    if frame.alpha <= 0.0 {
        return;
    }
    let tilt = SHEEN_TILT_DEG.to_radians();
    // The band spans the mark's height, tilted: the bottom endpoint trails
    // right by h·tan(20°). The sweep travels the width plus the trail.
    let trail = mark_rect.height() * tilt.tan();
    let x = mark_rect.left() - trail + frame.x * (mark_rect.width() + trail);
    let top = pos2(x, mark_rect.top() - 1.0);
    let bottom = pos2(x + trail, mark_rect.bottom() + 1.0);
    let clipped = painter.with_clip_rect(mark_rect);
    // Stepped alpha: flank, core, flank (no smooth gradient, §12).
    for (offset, alpha) in [(-1.5_f32, 0.35_f32), (0.0, 1.0), (1.5, 0.35)] {
        let shift = vec2(offset, 0.0);
        clipped.line_segment(
            [top + shift, bottom + shift],
            Stroke::new(1.0, with_white(frame.alpha * alpha)),
        );
    }
}

/// White at `alpha` — the brand mark's own color (brand.rs precedent:
/// the mark is monochrome white, so the sheen is too).
fn with_white(alpha: f32) -> egui::Color32 {
    let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(255, 255, 255, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::assertions_on_constants)] // the test's point IS the constant's value
    fn the_owner_enabled_the_flag() {
        assert!(METAL_MARK_ENABLED, "the owner flipped this (Phase R7)");
    }

    #[test]
    fn the_sheen_sweeps_once_per_four_loops() {
        // Idle for the last three loops of the period.
        for t in [1.3, 2.4, 3.6, 4.5] {
            assert_eq!(sheen_frame(t), None, "idle at {t}");
        }
        // The sweep occupies the first loop.
        let start = sheen_frame(0.0).expect("sweep starts");
        // (the bezier evaluator's bisection residue is ~1e-15 at p=0)
        assert!(start.x.abs() < 1e-6, "starts at the left edge");
        assert!(start.alpha.abs() < 1e-6, "fades in from zero");
        let mid = sheen_frame(LOOP_S * 0.5).expect("mid-sweep");
        assert!(mid.x > 0.0 && mid.x < 1.0);
        assert!(mid.alpha > 0.0);
        // Monotone travel across the sweep (staying inside it: the
        // boundary t = --dur-loop is exactly the sweep's end).
        let mut last = 0.0;
        for i in 1..10 {
            let f = sheen_frame(i as f32 * LOOP_S / 10.0).expect("in sweep");
            assert!(f.x >= last, "monotone: {} < {}", f.x, last);
            last = f.x;
        }
        assert_eq!(sheen_frame(LOOP_S), None, "the sweep ends at its loop");
        // The envelope peaks at 20% of the sweep (the comet keyframes).
        let peak = sheen_frame(LOOP_S * 0.2).expect("peak");
        assert!((peak.alpha - PEAK_ALPHA).abs() < 1e-4);
        // The period wraps: the next period's sweep starts fresh.
        assert!(sheen_frame(SWEEP_PERIOD_S).is_some());
    }

    #[test]
    fn paint_sheen_is_a_no_op_when_idle() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0))),
            ..Default::default()
        };
        let mark = Rect::from_center_size(pos2(100.0, 100.0), vec2(64.0, 64.0));
        let mut out = ctx.run_ui(input, |ui| {
            paint_sheen(ui.painter(), mark, 2.4); // the idle gap
        });
        out.textures_delta.clear();
        assert!(out.shapes.is_empty(), "idle: nothing painted");
    }

    #[test]
    fn paint_sheen_paints_three_stepped_hairlines_when_sweeping() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0))),
            ..Default::default()
        };
        let mark = Rect::from_center_size(pos2(100.0, 100.0), vec2(64.0, 64.0));
        let mut out = ctx.run_ui(input, |ui| {
            paint_sheen(ui.painter(), mark, LOOP_S * 0.5); // mid-sweep
        });
        out.textures_delta.clear();
        let lines = out
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::LineSegment { .. }))
            .count();
        assert_eq!(lines, 3, "flank, core, flank");
        // Monochrome white only (egui stores premultiplied alpha: white
        // stays grayscale — r == g == b — at any alpha).
        for s in &out.shapes {
            if let egui::Shape::LineSegment { stroke, .. } = &s.shape {
                let c = stroke.color;
                assert!(
                    c.r() == c.g() && c.g() == c.b() && c.a() > 0,
                    "the sheen is white on ink: {c:?}"
                );
            }
        }
    }
}
