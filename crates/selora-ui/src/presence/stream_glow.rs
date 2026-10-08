//! `StreamGlow` — the composer's bottom-edge glow (Phase 6h; a native
//! port of the voice-glow web library's `VoiceBeam`).
//!
//! The web original reacts to a microphone; Lite has no voice input, so
//! this port reacts to **streaming activity** — the panel notes each
//! SSE delta and the glow rises and blooms with the delta rate (a
//! deliberate deviation from the source library, mandated by the phase
//! brief). While a tool runs, `processing` gathers the glow into a beam
//! that travels the composer's width (ping-pong on `--ease-orbital`, one
//! `--dur-loop` per traverse). A turn's completion fires the one-shot
//! `--ok` spark (the §7.3 `spark` keyframes over `--dur-long`) — the only
//! green here, semantic per §1.3.
//!
//! Monochrome (§12): the bloom is three stepped alpha bands in
//! `--fg-faint` / `--fg-muted` — the §12-lawful analogue of the web
//! version's blur (no smooth gradient). The envelope is attack/release,
//! tuned to §7.2: each delta is one attack step; the release decays to
//! zero over one `--dur-loop`. The band's end caps follow the host card's
//! corner radius — [`set_cap_radius`](Self::set_cap_radius) when the
//! caller knows it (the composer card is `--radius-2`), else the style's
//! widget radius, capped at `--radius-2` (the web version reads the host
//! element's `border-radius`).
//!
//! The rest state (REVAMP_SPEC §2.2): while the composer is focused and
//! the gateway connected but nothing streams (`set_ready`), the line band
//! breathes a low-alpha shimmer (alpha 0.06↔0.12 — the §7.3 `pulse`
//! glyph at low alpha, one `--dur-loop`).
//!
//! Reduce-motion (§7.4): the rise/decay is fade-family (kept); the
//! traveling beam freezes to a static centered segment; the spark becomes
//! a static flash (a `--dur-quick` full-alpha hold); the ready shimmer
//! freezes to a static α0.08 line.

use egui::{pos2, vec2, CornerRadius, Rect, Stroke, Ui};

use crate::theme::{color, motion, radius};

use super::{clock_s, ping_pong, pulse_eased, request_anim_frame, with_alpha, LOOP_S};

/// One delta's attack step: ~8 deltas in a frame saturate the envelope.
const ATTACK: f32 = 0.125;
/// The release: 1 → 0 over one `--dur-loop` (§7.2).
const RELEASE_S: f32 = LOOP_S;
/// The spark's run (§7.3: `spark` is `--dur-long`).
const SPARK_S: f32 = motion::DUR_LONG_MS as f32 / 1000.0;
/// The beam's length as a fraction of the composer's width.
const BEAM_LEN: f32 = 0.18;
/// The spark segment's length as a fraction of the width (before scale).
const SPARK_LEN: f32 = 0.12;

/// The band stack's heights (bottom line, mid, top — hairline-scale
/// strokes like the 2px cursor and the 3px squiggle, §3.2/§3.6).
const LINE_H: f32 = 1.0;
const MID_H: f32 = 2.0;
const TOP_H: f32 = 3.0;

/// The ready shimmer's breathing alpha bounds (REVAMP_SPEC §2.2 — a
/// `pulse` at low alpha, never louder than the streaming envelope).
const READY_SHIMMER_MIN: f32 = 0.06;
const READY_SHIMMER_MAX: f32 = 0.12;
/// The ready shimmer's static alpha under reduce-motion (§2.2).
const READY_SHIMMER_STATIC: f32 = 0.08;

/// The composer's streaming-activity glow. The panel owns one; deltas and
/// turn completions are noted through [`note_delta`](Self::note_delta) /
/// [`note_turn_done`](Self::note_turn_done); [`show`](Self::show) paints
/// along the composer bar's bottom edge every frame.
#[derive(Clone, Debug, Default)]
pub struct StreamGlow {
    /// The envelope level (0..1) — delta-rate-driven.
    level: f32,
    /// Attack steps noted since the last tick (deltas arrive outside
    /// frames — the panel's `push_assistant_delta` has no frame clock;
    /// the attack lands at the next tick's `now`).
    pending: f32,
    /// The release anchor: (frame-clock time, level) at the last applied
    /// attack. The level falls from the anchored level to exactly zero
    /// over one `--dur-loop` after the last delta (§7.2).
    anchor: Option<(f64, f32)>,
    /// A turn completion arrived; the spark starts on the next shown
    /// frame (the ghost-text "first paint" clock precedent).
    pending_spark: bool,
    /// The spark's start (frame-clock seconds).
    spark_at: Option<f64>,
    /// The §2.2 ready shimmer: the composer is focused and the gateway
    /// connected — the idle band breathes.
    ready: bool,
    /// The band end caps' corner radius when the caller knows the host
    /// card's radius (§2.2 — the composer card is `--radius-2`); `None`
    /// falls back to the style's widget radius.
    cap_radius: Option<f32>,
    reduce_motion: bool,
}

impl StreamGlow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_reduce_motion(&mut self, on: bool) {
        self.reduce_motion = on;
    }

    /// REVAMP_SPEC §2.2: while `ready` (the composer is focused and the
    /// gateway connected), the otherwise-idle band breathes the low-alpha
    /// ready shimmer.
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
    }

    /// REVAMP_SPEC §2.2: the band's end caps follow the host card's corner
    /// radius (the composer passes `--radius-2` so the caps match the
    /// card's corners); unset = the style's widget radius.
    pub fn set_cap_radius(&mut self, r: f32) {
        self.cap_radius = Some(r);
    }

    /// One streamed delta arrived (an attack step, applied at the next
    /// tick). Time-free: the release runs on the frame clock in
    /// [`tick`](Self::tick).
    pub fn note_delta(&mut self) {
        self.pending = (self.pending + ATTACK).min(1.0);
    }

    /// The turn completed — queue the one-shot `--ok` spark.
    pub fn note_turn_done(&mut self) {
        self.pending_spark = true;
    }

    /// The envelope level (tests, the panel's repaint discipline).
    pub fn level(&self) -> f32 {
        self.level
    }

    /// Whether the completion spark is queued or playing (the panel's
    /// integration tests).
    #[cfg(test)]
    pub(crate) fn spark_pending(&self) -> bool {
        self.pending_spark || self.spark_at.is_some()
    }

    /// Advance the envelope to `now` (frame-clock seconds): apply any
    /// pending attack (re-anchoring the release), then fall linearly from
    /// the anchored level to exactly zero over one `--dur-loop`. Returns
    /// the level.
    pub(crate) fn tick(&mut self, now: f64) -> f32 {
        if self.pending > 0.0 {
            self.level = (self.level + self.pending).min(1.0);
            self.pending = 0.0;
            self.anchor = Some((now, self.level));
        }
        if let Some((started, level)) = self.anchor {
            let t = ((now - started).max(0.0) as f32 / RELEASE_S).min(1.0);
            self.level = level * (1.0 - t);
        }
        self.level
    }

    /// Paint along `edge`'s bottom border (the composer card's rect).
    /// `processing` is "a tool is running": the glow gathers into the
    /// traveling beam.
    pub fn show(&mut self, ui: &Ui, edge: Rect, processing: bool) {
        let now = clock_s(ui.ctx());
        self.tick(now);
        if self.pending_spark {
            self.pending_spark = false;
            self.spark_at = Some(now);
        }
        let spark = self.spark_frame(now);

        // The band's end caps follow the host card's corner radius —
        // `set_cap_radius` when set (the composer card is --radius-2),
        // else the style's widget radius; capped at --radius-2.
        let cap = self
            .cap_radius
            .unwrap_or_else(|| {
                let widget_radius = ui.visuals().widgets.inactive.corner_radius;
                f32::from(widget_radius.sw.max(widget_radius.se))
            })
            .min(radius::R2);
        let end_caps = CornerRadius {
            nw: 0,
            ne: 0,
            sw: cap as u8,
            se: cap as u8,
        };

        if self.level <= 0.001 && !processing && spark.is_none() {
            // Idle: only the §2.2 ready shimmer may paint.
            self.paint_ready_shimmer(ui, edge, end_caps, now);
            return;
        }

        let painter = ui.painter();
        let bottom = edge.bottom();
        let band = |height_up: f32, height: f32| {
            Rect::from_min_size(
                pos2(edge.left(), bottom - height_up - height),
                vec2(edge.width(), height),
            )
        };

        // The bloom: three stepped bands. The lower ones carry the base
        // level; the upper ones emerge as the level blooms (continuous
        // thresholds — a stepped ramp, not a pop).
        let emerge = |floor: f32| ((self.level - floor) / (1.0 - floor)).clamp(0.0, 1.0);
        let line_alpha = 0.10 + 0.55 * self.level;
        if self.level > 0.001 {
            painter.rect_filled(
                band(0.0, LINE_H),
                end_caps,
                with_alpha(color::FG_MUTED, line_alpha),
            );
            let mid = emerge(0.10);
            if mid > 0.0 {
                painter.rect_filled(
                    band(LINE_H, MID_H),
                    end_caps,
                    with_alpha(color::FG_FAINT, 0.35 * mid),
                );
            }
            let top = emerge(0.45);
            if top > 0.0 {
                painter.rect_filled(
                    band(LINE_H + MID_H, TOP_H),
                    end_caps,
                    with_alpha(color::FG_FAINT, 0.18 * top),
                );
            }
        }

        // The processing beam: a bright segment traveling the width
        // (ping-pong on --ease-orbital, one --dur-loop per traverse).
        if processing {
            let y = bottom - LINE_H / 2.0;
            let seg = BEAM_LEN * edge.width();
            let (center_x, alpha) = if self.reduce_motion {
                (edge.center().x, 0.6)
            } else {
                let phase = (now as f32 / LOOP_S).rem_euclid(1.0);
                let t = motion::ease_bezier(motion::EASE_ORBITAL, ping_pong(phase));
                let travel = (edge.width() - seg).max(0.0);
                (edge.left() + seg / 2.0 + travel * t, 0.9)
            };
            painter.line_segment(
                [pos2(center_x - seg / 2.0, y), pos2(center_x + seg / 2.0, y)],
                Stroke::new(2.0, with_alpha(color::FG_MUTED, alpha)),
            );
        }

        // The completion spark: the §7.3 keyframes (0%: scale 0.6 alpha 0;
        // 40%: scale 1.1 alpha 1; 100%: scale 1 alpha 0) on --dur-long.
        if let Some((scale, alpha)) = spark {
            let half = SPARK_LEN * edge.width() * scale / 2.0;
            let y = bottom - LINE_H;
            painter.line_segment(
                [
                    pos2(edge.center().x - half, y),
                    pos2(edge.center().x + half, y),
                ],
                Stroke::new(2.0, with_alpha(color::OK, alpha)),
            );
        }

        // Repaint discipline: while the envelope, the beam, or the spark
        // is live (a static beam under reduce-motion needs no frames).
        let animating =
            self.level > 0.001 || spark.is_some() || (processing && !self.reduce_motion);
        if animating {
            request_anim_frame(ui.ctx());
        }
    }

    /// The §2.2 ready shimmer: with the composer focused and connected but
    /// idle, the `LINE_H` band breathes alpha 0.06↔0.12 on the §7.3
    /// `pulse` (one `--dur-loop`). Reduce-motion: a static α0.08 line, no
    /// repaints. Not ready: nothing painted, no frames requested (the
    /// pre-revamp idle contract).
    fn paint_ready_shimmer(&self, ui: &Ui, edge: Rect, end_caps: CornerRadius, now: f64) {
        if !self.ready {
            return;
        }
        let alpha = if self.reduce_motion {
            READY_SHIMMER_STATIC
        } else {
            let phase = now as f32 / LOOP_S;
            READY_SHIMMER_MIN + (READY_SHIMMER_MAX - READY_SHIMMER_MIN) * pulse_eased(phase)
        };
        let band = Rect::from_min_size(
            pos2(edge.left(), edge.bottom() - LINE_H),
            vec2(edge.width(), LINE_H),
        );
        ui.painter()
            .rect_filled(band, end_caps, with_alpha(color::FG_MUTED, alpha));
        if !self.reduce_motion {
            request_anim_frame(ui.ctx());
        }
    }

    /// The spark's (scale, alpha) at `now`, or `None` when it has run its
    /// `--dur-long` (and clears the clock). Under reduce-motion: a static
    /// full-alpha flash for `--dur-quick` (§7.4).
    fn spark_frame(&mut self, now: f64) -> Option<(f32, f32)> {
        let started = self.spark_at?;
        let t = ((now - started) as f32 / SPARK_S).clamp(0.0, 1.0);
        if self.reduce_motion {
            let held = (now - started) as f32 <= motion::DUR_QUICK_MS as f32 / 1000.0;
            if held {
                return Some((1.0, 1.0));
            }
            self.spark_at = None;
            return None;
        }
        if t >= 1.0 {
            self.spark_at = None;
            return None;
        }
        Some(spark_keyframe(t))
    }
}

/// The §7.3 `spark` keyframes: 0% scale 0.6 alpha 0 → 40% scale 1.1
/// alpha 1 → 100% scale 1.0 alpha 0.
pub(crate) fn spark_keyframe(t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    if t < 0.4 {
        let p = t / 0.4;
        (0.6 + 0.5 * p, p)
    } else {
        let p = (t - 0.4) / 0.6;
        (1.1 - 0.1 * p, 1.0 - p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    #[test]
    fn a_delta_burst_raises_the_envelope_and_it_decays_to_zero() {
        let mut glow = StreamGlow::new();
        assert_eq!(glow.level(), 0.0);
        // A scripted burst: eight deltas saturate the envelope (the
        // attack lands at the next tick — deltas carry no clock).
        for _ in 0..8 {
            glow.note_delta();
        }
        glow.tick(10.0);
        assert!((glow.level() - 1.0).abs() < 1e-6);
        // More deltas never overflow.
        glow.note_delta();
        glow.tick(10.0);
        assert_eq!(glow.level(), 1.0);

        // The release: from the anchored level to zero over one
        // --dur-loop.
        let mid = glow.tick(10.0 + RELEASE_S as f64 / 2.0);
        assert!((mid - 0.5).abs() < 0.01, "half-way through the release");
        let done = glow.tick(10.0 + RELEASE_S as f64 + 0.1);
        assert_eq!(done, 0.0, "decayed to zero");
    }

    #[test]
    fn a_single_delta_fades_quickly_without_repaint_pressure() {
        let mut glow = StreamGlow::new();
        glow.note_delta();
        glow.tick(0.0);
        let after = glow.tick(0.6);
        assert!((after - ATTACK / 2.0).abs() < 0.01);
        assert_eq!(glow.tick(2.0), 0.0);
    }

    #[test]
    fn the_spark_follows_the_spec_keyframes() {
        assert_eq!(spark_keyframe(0.0), (0.6, 0.0));
        let (scale, alpha) = spark_keyframe(0.4);
        assert!((scale - 1.1).abs() < 1e-6 && (alpha - 1.0).abs() < 1e-6);
        assert_eq!(spark_keyframe(1.0), (1.0, 0.0));
    }

    // -- headless paint -------------------------------------------------

    fn edge() -> Rect {
        Rect::from_min_size(pos2(0.0, 540.0), vec2(360.0, 60.0))
    }

    fn frame(
        ctx: &egui::Context,
        time_s: f64,
        glow: &mut StreamGlow,
        processing: bool,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(360.0, 600.0))),
            time: Some(time_s),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            glow.show(ui, edge(), processing);
        });
        out.textures_delta.clear();
        out
    }

    fn filled_bands(out: &egui::FullOutput) -> usize {
        out.shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT))
            .count()
    }

    #[test]
    fn an_idle_glow_paints_nothing_and_requests_nothing() {
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        for _ in 0..3 {
            frame(&ctx, 0.0, &mut glow, false);
        }
        let out = frame(&ctx, 5.0, &mut glow, false);
        assert_eq!(filled_bands(&out), 0);
        let delay = out.viewport_output.values().next().map(|o| o.repaint_delay);
        assert_eq!(delay, Some(std::time::Duration::MAX));
    }

    #[test]
    fn a_burst_blooms_the_bands_then_they_fade() {
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        frame(&ctx, 0.0, &mut glow, false); // stamp the decay clock
        for _ in 0..8 {
            glow.note_delta();
        }
        let bloomed = frame(&ctx, 0.1, &mut glow, false);
        assert_eq!(filled_bands(&bloomed), 3, "level 1: all three bands");
        // Past the release: back to nothing.
        let faded = frame(&ctx, 0.1 + RELEASE_S as f64 + 0.5, &mut glow, false);
        assert_eq!(filled_bands(&faded), 0);
        assert_eq!(glow.level(), 0.0);
    }

    #[test]
    fn processing_paints_the_traveling_beam() {
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        // Two frames a quarter-loop apart: the beam moved.
        let a = frame(&ctx, 1.0, &mut glow, true);
        let b = frame(&ctx, 1.3, &mut glow, true);
        let beam_x = |out: &egui::FullOutput| {
            out.shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::LineSegment { points, stroke } if stroke.width == 2.0 => {
                        Some((points[0].x + points[1].x) / 2.0)
                    }
                    _ => None,
                })
                .expect("the beam paints")
        };
        assert_ne!(beam_x(&a), beam_x(&b), "the beam travels");

        // Reduce-motion: a static centered segment.
        let mut calm = StreamGlow::new();
        calm.set_reduce_motion(true);
        let c = frame(&ctx, 1.0, &mut calm, true);
        let d = frame(&ctx, 1.3, &mut calm, true);
        assert_eq!(beam_x(&c), beam_x(&d), "reduce-motion: static beam");
        assert!((beam_x(&c) - edge().center().x).abs() < 1e-4);
    }

    #[test]
    fn turn_done_fires_the_one_shot_ok_spark() {
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        frame(&ctx, 0.0, &mut glow, false);
        glow.note_turn_done();
        frame(&ctx, 0.1, &mut glow, false); // the spark's clock starts here
        let during = frame(&ctx, 0.1 + SPARK_S as f64 * 0.4, &mut glow, false);
        let has_green = during.shapes.iter().any(|s| match &s.shape {
            egui::Shape::LineSegment { stroke, .. } => stroke.color == color::OK,
            _ => false,
        });
        assert!(has_green, "the one-shot --ok spark paints");
        // After --dur-long the spark is spent (no more green, no frames).
        let after = frame(&ctx, 0.1 + SPARK_S as f64 + 0.5, &mut glow, false);
        let still_green = after.shapes.iter().any(|s| match &s.shape {
            egui::Shape::LineSegment { stroke, .. } => stroke.color == color::OK,
            _ => false,
        });
        assert!(!still_green, "one-shot: the spark is spent");
    }

    #[test]
    fn reduce_motion_spark_is_a_static_flash() {
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        glow.set_reduce_motion(true);
        glow.note_turn_done();
        let flash = frame(&ctx, 5.0, &mut glow, false);
        let green = flash.shapes.iter().any(|s| match &s.shape {
            egui::Shape::LineSegment { stroke, .. } => stroke.color == color::OK,
            _ => false,
        });
        assert!(green, "the static flash paints");
        let gone = frame(
            &ctx,
            5.0 + motion::DUR_QUICK_MS as f64 / 1000.0 + 0.05,
            &mut glow,
            false,
        );
        let green = gone.shapes.iter().any(|s| match &s.shape {
            egui::Shape::LineSegment { stroke, .. } => stroke.color == color::OK,
            _ => false,
        });
        assert!(!green, "the flash ends after --dur-quick");
    }

    #[test]
    fn the_band_caps_follow_the_style_corner_radius() {
        // egui's default widget radius is 2 (= --radius-1, the composer
        // input's spec): the bottom band's end caps adopt it.
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        for _ in 0..8 {
            glow.note_delta();
        }
        let out = frame(&ctx, 0.0, &mut glow, false);
        let band = out.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT => Some(r.corner_radius),
            _ => None,
        });
        assert_eq!(
            band,
            Some(CornerRadius {
                nw: 0,
                ne: 0,
                sw: 2,
                se: 2
            }),
            "the bottom corners follow the widget radius (2 = --radius-1)"
        );
    }

    #[test]
    fn ready_shimmer_breathes_within_bounds() {
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        glow.set_ready(true);
        let band_alpha = |out: &egui::FullOutput| {
            out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT => Some(r.fill.a()),
                _ => None,
            })
        };
        // Sweep one --dur-loop: the shimmer band is always present, its
        // alpha stays inside 0.06..0.12, and it actually breathes.
        let mut min = u8::MAX;
        let mut max = u8::MIN;
        for i in 0..12 {
            let out = frame(&ctx, i as f64 * 0.1, &mut glow, false);
            let a = band_alpha(&out).expect("the ready shimmer band paints");
            min = min.min(a);
            max = max.max(a);
            let delay = out.viewport_output.values().next().map(|o| o.repaint_delay);
            assert!(
                delay.is_some_and(|d| d <= crate::presence::ANIM_FRAME),
                "the breathing shimmer requests animation frames"
            );
        }
        // with_alpha rounds: 0.06·255 ≈ 15, 0.12·255 ≈ 31.
        assert!(min >= 15, "never below the 0.06 floor: {min}");
        assert!(max <= 31, "never above the 0.12 ceiling: {max}");
        assert!(max > min, "the shimmer breathes (min {min}, max {max})");
    }

    #[test]
    fn ready_shimmer_freezes_under_reduce_motion() {
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        glow.set_ready(true);
        glow.set_reduce_motion(true);
        for _ in 0..3 {
            frame(&ctx, 0.0, &mut glow, false);
        }
        let band_alpha = |out: &egui::FullOutput| {
            out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT => Some(r.fill.a()),
                _ => None,
            })
        };
        let a = frame(&ctx, 1.0, &mut glow, false);
        let b = frame(&ctx, 5.0, &mut glow, false);
        let expected = (READY_SHIMMER_STATIC * 255.0).round() as u8;
        assert_eq!(band_alpha(&a), Some(expected), "the static α0.08 line");
        assert_eq!(band_alpha(&b), Some(expected), "static: no breathing");
        let delay = a.viewport_output.values().next().map(|o| o.repaint_delay);
        assert_eq!(
            delay,
            Some(std::time::Duration::MAX),
            "reduce-motion requests no animation frames"
        );
    }

    #[test]
    fn band_caps_follow_the_set_cap_radius() {
        // §2.2: the composer passes --radius-2 so the caps match the
        // card's corners (the style default is 2, the card is 4).
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        glow.set_ready(true);
        glow.set_cap_radius(radius::R2);
        let out = frame(&ctx, 0.0, &mut glow, false);
        let band = out.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT => Some(r.corner_radius),
            _ => None,
        });
        assert_eq!(
            band,
            Some(CornerRadius {
                nw: 0,
                ne: 0,
                sw: radius::R2 as u8,
                se: radius::R2 as u8
            }),
            "the caps follow set_cap_radius, not the style"
        );
    }

    #[test]
    fn not_ready_idle_still_paints_nothing() {
        // The pre-shimmer idle contract is untouched while `ready` is unset.
        let ctx = egui::Context::default();
        let mut glow = StreamGlow::new();
        for _ in 0..3 {
            frame(&ctx, 0.0, &mut glow, false);
        }
        let out = frame(&ctx, 5.0, &mut glow, false);
        assert_eq!(filled_bands(&out), 0);
        let delay = out.viewport_output.values().next().map(|o| o.repaint_delay);
        assert_eq!(delay, Some(std::time::Duration::MAX));
    }
}
