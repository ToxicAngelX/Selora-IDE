//! `BorderBeam` — the in-flight beam riding the composer card's perimeter
//! (the sixth port; a native port of the border-beam web library — "an
//! animated glow that rides the border of any card, button or input").
//! REVAMP_SPEC §8.
//!
//! A bright arc segment (1/8 of the perimeter, painted as 24 short line
//! segments in four stepped alpha bands, head bright → tail gone) rides
//! the card's rounded-rect perimeter, arc-length parameterized: phase 0
//! is the top-left corner, traveling right along the top edge, then
//! clockwise. One lap is `--dur-lap` (2400ms — §8.3, the revamp's one new
//! token), the phase eased `--ease-orbital` exactly like the splash's
//! orbit: the wrap is seamless and the beam is calmest at the seam.
//!
//! The library's props map so: `colorVariant` — only `mono` maps (§12:
//! every segment is `--fg-default` at stepped alpha; no hue, no blur, no
//! gradient); `strength` (0–1) scales every alpha; `active` pauses the
//! lap — the beam freezes at its held phase and stays painted (a paused
//! instrument, not a hidden one — the composer simply never shows a
//! paused beam, §8.6). The library's `size` variant enum is subsumed by
//! rect-driven geometry: the beam rides whatever rect it is given at the
//! fixed 2px weight (the `'md'` reading); the two pulse variants would be
//! a second ambient loop on a resting control — the ready shimmer already
//! owns that job (§2.2) and §9's loop budget admits no more.
//!
//! Reduce-motion (§8.4, the owner's stricter glow rule — §11 bend 5): a
//! static segment at the top-left corner (phase 0), no repaint requests.
//! `strength = 0` paints nothing and requests nothing.
//!
//! Where it may paint (§8.5): the composer card only — one signature
//! surface, the MetalMark discipline (`metal.rs`).

use egui::{pos2, Pos2, Rect, Stroke, Ui};

use crate::theme::{color, motion, radius};

use super::{clock_s, request_anim_frame, with_alpha, LAP_S};

/// The beam's span as a fraction of the perimeter (§8.2 — ≈103px at the
/// default panel width).
const BEAM_ARC_FRAC: f32 = 0.125;
/// The segment count the arc is painted as (fixed arrays — the orb's
/// no-per-frame-`Vec` discipline, orb.rs).
const BEAM_SEGS: usize = 24;
/// Four stepped alpha bands over the segments, head bright → tail gone
/// (the `GOO_ALPHA` precedent, liquid.rs). `strength` multiplies these.
const BEAM_ALPHAS: [f32; 4] = [0.85, 0.50, 0.28, 0.14];
/// The stroke weight, centered on the perimeter path (the `'md'` reading
/// of the library's `size`, §8.1).
const BEAM_WIDTH: f32 = 2.0;

/// The composer card's in-flight perimeter beam. Stateful (the
/// `StreamGlow` precedent, not the value-type `ThinkingOrb`):
/// `held_phase` freezes the lap when `active` flips false. The composer
/// constructs one at `ChatPanel::new`, threads `set_reduce_motion` beside
/// the other presence widgets, and calls [`show`](Self::show) after the
/// card stroke, before the StreamGlow band (§2.3).
#[derive(Clone, Debug)]
pub struct BorderBeam {
    /// `strength` (0–1) scales every band's alpha.
    strength: f32,
    /// The lap runs while `active`; inactive paints at `held_phase`.
    active: bool,
    reduce_motion: bool,
    /// The linear lap phase of the last live frame — the freeze point.
    held_phase: f32,
}

impl BorderBeam {
    /// Strength 1.0, inactive (§8.1).
    pub fn new() -> Self {
        Self {
            strength: 1.0,
            active: false,
            reduce_motion: false,
            held_phase: 0.0,
        }
    }

    /// Scale every band's alpha; clamped to 0..1 (§8.1). Zero paints
    /// nothing and requests nothing (§8.4).
    pub fn set_strength(&mut self, s: f32) {
        self.strength = s.clamp(0.0, 1.0);
    }

    /// `active` runs the lap; flipping false freezes the beam at the last
    /// live frame's phase — still painted, no repaint requests (the
    /// library's `animation-play-state: paused`, §8.1/§8.4).
    pub fn set_active(&mut self, on: bool) {
        self.active = on;
    }

    /// §8.4: freeze to the static segment at the top-left corner
    /// (phase 0), no repaint requests.
    pub fn set_reduce_motion(&mut self, on: bool) {
        self.reduce_motion = on;
    }

    /// Paint the beam onto `rect`'s perimeter (the composer card, §8.5).
    /// Paint-only; requests animation frames only while the lap runs
    /// (`active` and not reduce-motion).
    pub fn show(&mut self, ui: &Ui, rect: Rect) {
        if self.strength <= 0.0 {
            return;
        }
        if self.active && !self.reduce_motion {
            // The eased lap phase derives from the frame clock; holding
            // the linear phase each live frame is what `active = false`
            // freezes.
            self.held_phase = (clock_s(ui.ctx()) as f32 / LAP_S).rem_euclid(1.0);
            request_anim_frame(ui.ctx());
        }
        let linear = if self.reduce_motion {
            0.0
        } else {
            self.held_phase
        };
        let head = motion::ease_bezier(motion::EASE_ORBITAL, linear);
        let path = Perimeter::new(rect);
        if path.total <= 0.0 {
            return; // a degenerate rect has no perimeter to ride
        }
        let span = BEAM_ARC_FRAC * path.total;
        let head_d = head * path.total;
        let painter = ui.painter();
        for i in 0..BEAM_SEGS {
            // Bands tail → head: segment 0 is the faintest, the last is
            // the brightest.
            let band = (BEAM_SEGS - 1 - i) / (BEAM_SEGS / BEAM_ALPHAS.len());
            let d0 = head_d - span + span * (i as f32 / BEAM_SEGS as f32);
            let d1 = head_d - span + span * ((i + 1) as f32 / BEAM_SEGS as f32);
            painter.line_segment(
                [path.at(d0), path.at(d1)],
                Stroke::new(
                    BEAM_WIDTH,
                    with_alpha(color::FG_DEFAULT, BEAM_ALPHAS[band] * self.strength),
                ),
            );
        }
    }
}

impl Default for BorderBeam {
    fn default() -> Self {
        Self::new()
    }
}

/// The rounded-rect perimeter path of a rect at corner `--radius-2`,
/// arc-length parameterized (§8.2): `d = 0` is the top-left corner — the
/// point where the top edge leaves the corner arc — traveling right, then
/// clockwise. Section order: top edge, top-right arc, right edge,
/// bottom-right arc, bottom edge (right→left), bottom-left arc, left edge
/// (bottom→top), top-left arc.
struct Perimeter {
    rect: Rect,
    /// The corner radius (`--radius-2`, clamped to the rect's extent).
    r: f32,
    /// One horizontal straight's length.
    top: f32,
    /// One vertical straight's length.
    side: f32,
    /// One quarter-arc's length.
    arc: f32,
    /// The full lap length.
    total: f32,
}

impl Perimeter {
    fn new(rect: Rect) -> Self {
        let r = radius::R2
            .min(rect.width() / 2.0)
            .min(rect.height() / 2.0)
            .max(0.0);
        let top = (rect.width() - 2.0 * r).max(0.0);
        let side = (rect.height() - 2.0 * r).max(0.0);
        let arc = std::f32::consts::FRAC_PI_2 * r;
        Self {
            rect,
            r,
            top,
            side,
            arc,
            total: 2.0 * (top + side) + 4.0 * arc,
        }
    }

    /// The point at arc distance `d` (wraps around the lap).
    fn at(&self, d: f32) -> Pos2 {
        let d = d.rem_euclid(self.total);
        let (l, t, rr, b) = (
            self.rect.left(),
            self.rect.top(),
            self.rect.right(),
            self.rect.bottom(),
        );
        let r = self.r;
        let mut rest = d;
        if rest < self.top {
            return pos2(l + r + rest, t); // top edge, left → right
        }
        rest -= self.top;
        if rest < self.arc {
            // Top-right arc: center (right−r, top+r), −90° → 0°.
            let a = rest / r - std::f32::consts::FRAC_PI_2;
            return pos2(rr - r + r * a.cos(), t + r + r * a.sin());
        }
        rest -= self.arc;
        if rest < self.side {
            return pos2(rr, t + r + rest); // right edge, top → bottom
        }
        rest -= self.side;
        if rest < self.arc {
            // Bottom-right arc: center (right−r, bottom−r), 0° → 90°.
            let a = rest / r;
            return pos2(rr - r + r * a.cos(), b - r + r * a.sin());
        }
        rest -= self.arc;
        if rest < self.top {
            return pos2(rr - r - rest, b); // bottom edge, right → left
        }
        rest -= self.top;
        if rest < self.arc {
            // Bottom-left arc: center (left+r, bottom−r), 90° → 180°.
            let a = std::f32::consts::FRAC_PI_2 + rest / r;
            return pos2(l + r + r * a.cos(), b - r + r * a.sin());
        }
        rest -= self.arc;
        if rest < self.side {
            return pos2(l, b - r - rest); // left edge, bottom → top
        }
        rest -= self.side;
        // Top-left arc: center (left+r, top+r), 180° → 270° — ends at
        // (left+r, top), the path's d = 0, so the lap wraps seamlessly.
        let a = std::f32::consts::PI + if r > 0.0 { rest / r } else { 0.0 };
        pos2(l + r + r * a.cos(), t + r + r * a.sin())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    /// The composer card's stand-in (§8.3's perimeter: 2·(392+92) + 2π·4
    /// ≈ 993px at `--radius-2`).
    fn card() -> Rect {
        Rect::from_min_size(pos2(10.0, 10.0), vec2(400.0, 100.0))
    }

    fn frame(ctx: &egui::Context, time_s: f64, beam: &mut BorderBeam) -> egui::FullOutput {
        let input = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 200.0))),
            time: Some(time_s),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            beam.show(ui, card());
        });
        out.textures_delta.clear();
        out
    }

    /// The painted beam segments (tail → head, the paint order).
    fn segments(out: &egui::FullOutput) -> Vec<(Pos2, Pos2, egui::Color32)> {
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::LineSegment { points, stroke } if stroke.width == BEAM_WIDTH => {
                    Some((points[0], points[1], stroke.color))
                }
                _ => None,
            })
            .collect()
    }

    fn repaint_delay(out: &egui::FullOutput) -> Option<std::time::Duration> {
        out.viewport_output.values().next().map(|o| o.repaint_delay)
    }

    /// The arc-fraction along the card's perimeter of a point on a
    /// straight section (the spec's section order, derived independently
    /// of the widget's path code — the sampled heads all land on
    /// straights).
    fn arc_fraction(p: Pos2) -> f32 {
        let rect = card();
        let r = radius::R2;
        let top = rect.width() - 2.0 * r;
        let side = rect.height() - 2.0 * r;
        let arc = std::f32::consts::FRAC_PI_2 * r;
        let total = 2.0 * (top + side) + 4.0 * arc;
        let (l, t, rr, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
        let d = if (p.y - t).abs() < 0.6 {
            p.x - l - r // top edge
        } else if (p.x - rr).abs() < 0.6 {
            top + arc + (p.y - t - r) // right edge
        } else if (p.y - b).abs() < 0.6 {
            top + arc + side + arc + (rr - r - p.x) // bottom edge
        } else if (p.x - l).abs() < 0.6 {
            top + arc + side + arc + top + arc + (b - r - p.y) // left edge
        } else {
            panic!("head off the straights: {p:?}");
        };
        d / total
    }

    #[test]
    fn beam_advances_one_eased_lap_per_dur_lap() {
        let ctx = egui::Context::default();
        let mut beam = BorderBeam::new();
        beam.set_active(true);
        let a = frame(&ctx, 0.0, &mut beam);
        // The eased travel (§8.3): head arc-fraction =
        // ease_bezier(--ease-orbital, t / --dur-lap) — ≈0.071 / 0.500 /
        // ≈0.929 at the lap's quarter / half / three-quarter marks
        // (tolerance ±0.02). NOT linear: linear would read 0.25 / 0.75 at
        // the quarter marks, 0.18 of arc (≈178px) away — far outside the
        // tolerance.
        for (t, expected) in [(0.6, 0.071), (1.2, 0.500), (1.8, 0.929)] {
            let out = frame(&ctx, t, &mut beam);
            let segs = segments(&out);
            let head = segs.last().expect("the beam paints").1;
            let frac = arc_fraction(head);
            assert!(
                (frac - expected).abs() < 0.02,
                "t={t}: head arc-fraction {frac:.3} ≈ {expected} (linear: {:.3})",
                t / 2.4
            );
        }
        // One lap = --dur-lap: identical pictures at t and t + 2400ms.
        let b = frame(&ctx, motion::DUR_LAP_MS as f64 / 1000.0, &mut beam);
        assert_eq!(segments(&a), segments(&b), "seamless wrap at one lap");
    }

    #[test]
    fn beam_paints_only_stepped_fg_alpha() {
        let ctx = egui::Context::default();
        let mut beam = BorderBeam::new();
        beam.set_active(true);
        beam.set_strength(0.5);
        let out = frame(&ctx, 0.3, &mut beam);
        let segs = segments(&out);
        assert_eq!(segs.len(), BEAM_SEGS, "24 segments");
        // The four bands × strength 0.5 (with_alpha's rounding).
        let bands = BEAM_ALPHAS.map(|a| (a * 0.5 * 255.0).round() as u8);
        let mut band_counts = [0usize; 4];
        for (i, (_, _, color)) in segs.iter().enumerate() {
            let band = bands
                .iter()
                .position(|b| *b == color.a())
                .unwrap_or_else(|| panic!("alpha {} ∈ the four bands × strength", color.a()));
            band_counts[band] += 1;
            // Head bright → tail gone: the band never rises going tailward.
            assert_eq!(band, (BEAM_SEGS - 1 - i) / (BEAM_SEGS / bands.len()));
        }
        assert_eq!(band_counts, [6, 6, 6, 6], "four bands over 24 segments");
        // Monochrome (§8.2/§12): every segment is a gray — and the gray
        // is --fg-default's (229,229,229), not white's. egui stores
        // premultiplied alpha: the head band (α108) reads 229·108/255 ≈
        // 97 in the channels; white would read 108.
        for (_, _, color) in &segs {
            assert!(
                color.r() == color.g() && color.g() == color.b() && color.a() > 0,
                "gray only: {color:?}"
            );
        }
        let head = segs.last().unwrap().2;
        let expected = (color::FG_DEFAULT.r() as f32 * head.a() as f32 / 255.0).round() as i32;
        assert!(
            (head.r() as i32 - expected).abs() <= 1,
            "the gray is --fg-default's: r={} ≈ {expected}",
            head.r()
        );
        // Head brightest.
        assert!(segs.last().unwrap().2.a() > segs.first().unwrap().2.a());
    }

    #[test]
    fn inactive_beam_freezes_in_place_and_requests_nothing() {
        let ctx = egui::Context::default();
        let mut beam = BorderBeam::new();
        beam.set_active(true);
        frame(&ctx, 0.0, &mut beam);
        frame(&ctx, 0.05, &mut beam);
        let live = frame(&ctx, 0.1, &mut beam); // mid-lap
        beam.set_active(false);
        let a = frame(&ctx, 2.0, &mut beam);
        let b = frame(&ctx, 5.0, &mut beam);
        assert_eq!(segments(&a), segments(&b), "frozen: identical shapes");
        assert_eq!(
            segments(&a),
            segments(&live),
            "frozen at the last live frame's phase, not rewound"
        );
        for out in [&a, &b] {
            assert_eq!(
                repaint_delay(out),
                Some(std::time::Duration::MAX),
                "a paused beam requests nothing"
            );
        }
    }

    #[test]
    fn reduce_motion_beam_is_a_static_top_left_segment() {
        let ctx = egui::Context::default();
        let mut beam = BorderBeam::new();
        beam.set_active(true);
        beam.set_reduce_motion(true);
        for _ in 0..3 {
            frame(&ctx, 0.0, &mut beam);
        }
        let a = frame(&ctx, 1.0, &mut beam);
        let b = frame(&ctx, 4.0, &mut beam);
        assert_eq!(segments(&a), segments(&b), "static: identical across times");
        let (l, t) = (card().left(), card().top());
        let segs = segments(&a);
        // Phase 0 = the top-left corner: the beam runs up the left edge,
        // around the corner arc, and its head touches the top edge.
        let on_left_edge = segs.iter().any(|(p0, p1, _)| {
            [p0, p1]
                .into_iter()
                .any(|p| (p.x - l).abs() < 1e-3 && p.y > t + radius::R2)
        });
        let on_top_edge = segs.iter().any(|(_, p1, _)| (p1.y - t).abs() < 1e-3);
        assert!(
            on_left_edge && on_top_edge,
            "the static segment straddles the top-left corner"
        );
        assert_eq!(
            repaint_delay(&a),
            Some(std::time::Duration::MAX),
            "reduce-motion requests nothing"
        );
    }

    #[test]
    fn zero_strength_paints_nothing() {
        let ctx = egui::Context::default();
        let mut beam = BorderBeam::new();
        beam.set_active(true);
        beam.set_strength(0.0);
        for _ in 0..3 {
            frame(&ctx, 0.0, &mut beam);
        }
        let out = frame(&ctx, 1.0, &mut beam);
        assert!(out.shapes.is_empty(), "nothing painted");
        assert_eq!(
            repaint_delay(&out),
            Some(std::time::Duration::MAX),
            "nothing requested"
        );
        // And the clamp (§8.1: strength clamps 0..1).
        let mut clamped = BorderBeam::new();
        clamped.set_strength(2.0);
        assert_eq!(clamped.strength, 1.0);
        clamped.set_strength(-1.0);
        assert_eq!(clamped.strength, 0.0);
    }
}
