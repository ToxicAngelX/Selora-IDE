//! `ThinkingOrb` — the thought indicator (Phase 6h; a native port of the
//! thinking-orbs web library's nine hand-tuned states).
//!
//! One small animated mark replaces the streaming bubble's `⠿⠿⠿` wave
//! (UI_SPEC.md §4 — same trigger: only after 800ms with no delta) and rides
//! the agent status line next to the [`crate::presence::AgentAvatar`]. The
//! nine states map onto the agent loop (ARCHITECTURE.md §17.3–17.4):
//!
//! | loop signal | orb state |
//! |---|---|
//! | `Thinking` (no recent delta) | [`OrbState::Working`] |
//! | `Thinking` (deltas streaming) | [`OrbState::Connecting`] |
//! | `RunningTool` read-type | [`OrbState::Searching`] |
//! | `RunningTool` bash/write | [`OrbState::Solving`] |
//! | `AwaitingApproval` | [`OrbState::Listening`] |
//! | idle / finished | [`OrbState::Breathing`] |
//! | reserved (search weaving, ghost text composing, tree refresh shaping) | `Weaving` / `Composing` / `Shaping` |
//!
//! The reserved arms are tuned and tested but not yet wired — their
//! surfaces (the editor's ghost text, the file tree) are outside this
//! phase's file ownership.
//!
//! Ink monochrome (§12): dots in `--fg-faint` / `--fg-muted`; `--ok`
//! appears only as the 64px breathing state's liveness dot (§1.3 sanctions
//! "live dot" status). Every loop runs on `--dur-loop` (1200ms) with
//! `--ease-orbital` (§7.1/§7.3) — no loop faster than 1.2s. Reduce-motion
//! (§7.4) freezes every state to a single static dot. The `speed` clock
//! multiplier is clamped to ≤ 1.0: a faster loop would break §12.
//!
//! The widget is a value type — no per-instance clock; the phase derives
//! from the egui frame clock (`Context::input(|i| i.time)`), so offscreen
//! or dropped orbs cost nothing and a swap never leaves a stale timer.

use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Stroke, Ui};

use crate::theme::{color, motion};

use super::{clock_s, ping_pong, pulse_eased, request_anim_frame, with_alpha, LOOP_S};

/// Inline size (the streaming bubble, the status line): 20px.
pub const ORB_INLINE: f32 = 20.0;
/// Avatar scale (a future roster surface): 64px.
pub const ORB_AVATAR: f32 = 64.0;

/// The most dots any state paints (connecting/working/shaping use 3;
/// breathing at avatar scale adds the liveness dot).
const MAX_DOTS: usize = 4;

/// The nine orb states. Six are wired to the agent loop; three are
/// reserved for future surfaces (see the module table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbState {
    /// Idle / finished: the §7.3 `pulse` — one dot breathing
    /// (scale 1.0↔1.08, alpha 1.0↔0.7). At [`ORB_AVATAR`] a small `--ok`
    /// satellite marks liveness (§1.3).
    Breathing,
    /// `Thinking` with no recent delta: the §7.3 `wave` — three dots
    /// bobbing on a 200ms stagger (the `⠿⠿⠿` successor).
    Working,
    /// Read-type tool: a scan — one dot sweeps a short track (ping-pong,
    /// `--ease-orbital`) between two faint end ticks.
    Searching,
    /// bash / write tool: the §7.3 `spin` — two dots orbiting the center
    /// 180° apart, linear (gears).
    Solving,
    /// Parked on an approval: one steady dot and a ring that expands and
    /// fades (attentive stillness).
    Listening,
    /// Deltas streaming: three dots converge from spread to cluster and
    /// back (the link forming).
    Connecting,
    /// Reserved (search weaving): two dots cross horizontally, opposite
    /// phases.
    Weaving,
    /// Reserved (ghost text composing): the §7.3 `comet` — a short streak
    /// sweeps left→right with the comet alpha envelope.
    Composing,
    /// Reserved (file tree refresh): three dots morph between a triangle
    /// and a row.
    Shaping,
}

/// One painted dot (the geometry is data — tests assert on it, the
/// painter is a thin shell).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbDot {
    pub center: Pos2,
    pub radius: f32,
    pub tone: Tone,
    pub alpha: f32,
}

/// The ink tone of a dot (§12: the whole palette the orb may use).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// `--fg-faint`.
    Faint,
    /// `--fg-muted`.
    Muted,
    /// `--ok` — the liveness dot only (§1.3).
    Ok,
}

impl Tone {
    fn color(self) -> Color32 {
        match self {
            Tone::Faint => color::FG_FAINT,
            Tone::Muted => color::FG_MUTED,
            Tone::Ok => color::OK,
        }
    }
}

/// One frame of orb geometry: up to [`MAX_DOTS`] dots, plus at most one
/// ring (listening) or one beam (composing's comet streak).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbGeometry {
    pub dots: [Option<OrbDot>; MAX_DOTS],
    /// Listening's expanding ring: (center, radius, alpha).
    pub ring: Option<(Pos2, f32, f32)>,
    /// Composing's streak: (tail, head, alpha).
    pub beam: Option<(Pos2, Pos2, f32)>,
}

impl OrbGeometry {
    fn empty() -> Self {
        Self {
            dots: [None; MAX_DOTS],
            ring: None,
            beam: None,
        }
    }

    /// The dots as an iterator (paint order).
    pub fn dots_iter(&self) -> impl Iterator<Item = &OrbDot> {
        self.dots.iter().filter_map(|d| d.as_ref())
    }
}

/// The thought indicator. Construct per paint site (it is a value type);
/// the phase derives from the frame clock.
#[derive(Clone, Copy, Debug)]
pub struct ThinkingOrb {
    state: OrbState,
    size: f32,
    /// Clock multiplier (the web library's `speed` prop), clamped to
    /// [0, 1] at use — §12 forbids loops faster than 1.2s.
    speed: f32,
    /// The web library's `paused` prop: freeze the clock at phase 0.
    paused: bool,
    reduce_motion: bool,
}

impl ThinkingOrb {
    pub fn new(state: OrbState, size: f32) -> Self {
        Self {
            state,
            size,
            speed: 1.0,
            paused: false,
            reduce_motion: false,
        }
    }

    /// Clock multiplier (default 1.0). Clamped to [0, 1]: a faster loop
    /// would break DESIGN_TOKENS.md §12's 1.2s floor.
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Freeze the animation at phase 0 (the web library's `paused`).
    pub fn paused(mut self, paused: bool) -> Self {
        self.paused = paused;
        self
    }

    /// DESIGN_TOKENS.md §7.4: a static dot, no motion, no repaint
    /// requests.
    pub fn reduce_motion(mut self, on: bool) -> Self {
        self.reduce_motion = on;
        self
    }

    pub fn state(&self) -> OrbState {
        self.state
    }

    /// The loop phase for `time_s` (seconds on the frame clock), honoring
    /// `speed` / `paused` / reduce-motion.
    fn phase(&self, time_s: f64) -> f32 {
        if self.paused || self.reduce_motion {
            return 0.0;
        }
        let speed = self.speed.clamp(0.0, 1.0);
        ((time_s as f32 * speed) / LOOP_S).rem_euclid(1.0)
    }

    /// Allocate a `size`×`size` slot in `ui` and paint. Returns the rect.
    pub fn show(&self, ui: &mut Ui) -> Rect {
        let (rect, _) = ui.allocate_exact_size(vec2(self.size, self.size), egui::Sense::hover());
        self.paint(ui, rect);
        rect
    }

    /// Paint into a pre-laid-out rect (the streaming bubble's reserved
    /// band): the orb centers itself in the rect.
    pub fn paint(&self, ui: &Ui, rect: Rect) {
        let now = clock_s(ui.ctx());
        self.paint_with(ui.painter(), rect, now);
        if !self.paused && !self.reduce_motion && self.speed > 0.0 {
            request_anim_frame(ui.ctx());
        }
    }

    /// Paint for an explicit clock value (tests; the streaming bubble's
    /// anchor-relative clock).
    pub fn paint_with(&self, painter: &Painter, rect: Rect, time_s: f64) {
        let geometry = if self.reduce_motion {
            static_geometry(rect)
        } else {
            geometry(self.state, self.phase(time_s), rect)
        };
        paint_geometry(painter, &geometry);
    }
}

/// The §7.4 reduce-motion rendering: one static `--fg-muted` dot, the
/// same for every state ("a static dot, no motion").
fn static_geometry(rect: Rect) -> OrbGeometry {
    let mut g = OrbGeometry::empty();
    g.dots[0] = Some(OrbDot {
        center: rect.center(),
        radius: rect.width() * 0.10,
        tone: Tone::Muted,
        alpha: 1.0,
    });
    g
}

/// The pure heart: one frame of geometry for `state` at loop `phase`
/// (0..1) inside `rect`. Every state's motion lives on the §7.1/§7.2
/// tokens; coordinates are fractions of the rect's size, so the two sizes
/// (20 / 64) share one tuning.
pub fn geometry(state: OrbState, phase: f32, rect: Rect) -> OrbGeometry {
    let c = rect.center();
    let s = rect.width().min(rect.height());
    let mut g = OrbGeometry::empty();
    let mut dot = |i: usize, x: f32, y: f32, r: f32, tone: Tone, alpha: f32| {
        g.dots[i] = Some(OrbDot {
            center: pos2(c.x + x * s, c.y + y * s),
            radius: r * s,
            tone,
            alpha,
        });
    };

    match state {
        OrbState::Breathing => {
            // §7.3 pulse: scale 1.0↔1.08, alpha 1.0↔0.7 on ease-orbital.
            let e = pulse_eased(phase);
            dot(
                0,
                0.0,
                0.0,
                0.12 * (1.0 + 0.08 * e),
                Tone::Muted,
                1.0 - 0.3 * e,
            );
            // Avatar scale only: the §1.3 liveness satellite.
            if s >= ORB_AVATAR {
                dot(1, 0.30, -0.30, 0.045, Tone::Ok, 0.9);
            }
        }
        OrbState::Working => {
            // §7.3 wave: three dots bob on a 1/6-loop (200ms) stagger,
            // rise 0–30%, fall 30–60%, rest — the ⠿⠿⠿ successor.
            for i in 0..3 {
                let p = (phase + i as f32 / 6.0).rem_euclid(1.0);
                let (dy, alpha) = wave_step(p);
                dot(
                    i,
                    (i as f32 - 1.0) * 0.35,
                    dy,
                    0.09,
                    if i == 1 { Tone::Muted } else { Tone::Faint },
                    alpha,
                );
            }
        }
        OrbState::Searching => {
            // A scan: the dot sweeps the track on ease-orbital ping-pong;
            // the end ticks mark the range.
            let x = -0.30 + 0.60 * ping_pong_eased(phase);
            dot(0, -0.34, 0.0, 0.05, Tone::Faint, 0.5);
            dot(1, 0.34, 0.0, 0.05, Tone::Faint, 0.5);
            dot(2, x, 0.0, 0.10, Tone::Muted, 1.0);
        }
        OrbState::Solving => {
            // §7.3 spin: two dots 180° apart, linear (gears turning).
            let theta = std::f32::consts::TAU * phase;
            for (i, angle) in [theta, theta + std::f32::consts::PI]
                .into_iter()
                .enumerate()
            {
                dot(
                    i,
                    0.26 * angle.cos(),
                    0.26 * angle.sin(),
                    0.10,
                    Tone::Muted,
                    1.0,
                );
            }
        }
        OrbState::Listening => {
            // Steady center dot + a ring expanding and fading away.
            dot(0, 0.0, 0.0, 0.11, Tone::Muted, 1.0);
            let e = motion::ease_bezier(motion::EASE_ORBITAL, phase);
            g.ring = Some((c, (0.16 + 0.28 * e) * s, 0.8 * (1.0 - e)));
        }
        OrbState::Connecting => {
            // Three dots converge spread → cluster → spread (ease-orbital
            // ping-pong); they brighten as they meet.
            let e = ping_pong_eased(phase);
            for i in 0..3 {
                let spread = (i as f32 - 1.0) * 0.36;
                let cluster = (i as f32 - 1.0) * 0.10;
                let x = spread + (cluster - spread) * e;
                dot(i, x, 0.0, 0.09, Tone::Muted, 0.55 + 0.45 * e);
            }
        }
        OrbState::Weaving => {
            // Two dots crossing horizontally, opposite phases.
            let theta = std::f32::consts::TAU * phase;
            dot(0, 0.30 * theta.cos(), -0.10, 0.10, Tone::Muted, 1.0);
            dot(1, -0.30 * theta.cos(), 0.10, 0.10, Tone::Faint, 1.0);
        }
        OrbState::Composing => {
            // §7.3 comet: the streak's head sweeps left→right on
            // ease-precise; alpha follows the comet envelope.
            let head_x = -0.34 + 0.68 * motion::ease_bezier(motion::EASE_PRECISE, phase);
            let alpha = comet_alpha(phase);
            let tail_x = (head_x - 0.18).max(-0.34);
            let head = pos2(c.x + head_x * s, c.y);
            let tail = pos2(c.x + tail_x * s, c.y);
            g.beam = Some((tail, head, alpha));
            dot(0, head_x, 0.0, 0.08, Tone::Muted, alpha);
        }
        OrbState::Shaping => {
            // Three dots morph triangle ↔ row on ease-orbital ping-pong.
            let e = ping_pong_eased(phase);
            let triangle = [(-0.26, 0.16), (0.26, 0.16), (0.0, -0.22)];
            let row = [(-0.30, 0.0), (0.0, 0.0), (0.30, 0.0)];
            for (i, ((tx, ty), (rx, ry))) in triangle.into_iter().zip(row).enumerate() {
                dot(
                    i,
                    tx + (rx - tx) * e,
                    ty + (ry - ty) * e,
                    0.09,
                    Tone::Muted,
                    0.7 + 0.3 * e,
                );
            }
        }
    }
    g
}

/// One `wave` dot's (dy, alpha) over the loop: rise 0–30%, fall 30–60%,
/// rest 60–100% (§7.3 — the same keyframes the ⠿⠿⠿ used).
fn wave_step(phase: f32) -> (f32, f32) {
    if phase < 0.3 {
        let t = phase / 0.3;
        (-0.16 * t, 0.5 + 0.5 * t)
    } else if phase < 0.6 {
        let t = (phase - 0.3) / 0.3;
        (-0.16 * (1.0 - t), 1.0 - 0.5 * t)
    } else {
        (0.0, 0.5)
    }
}

/// Ping-pong through `--ease-orbital` (the sweep states' shared easing).
fn ping_pong_eased(phase: f32) -> f32 {
    motion::ease_bezier(motion::EASE_ORBITAL, ping_pong(phase))
}

/// The §7.3 comet envelope: 0% → 0, 20% → 1, 100% → 0.
fn comet_alpha(phase: f32) -> f32 {
    let phase = phase.clamp(0.0, 1.0);
    if phase < 0.2 {
        phase / 0.2
    } else {
        1.0 - (phase - 0.2) / 0.8
    }
}

/// Paint one frame of geometry (thin shell over egui primitives — no
/// allocation, no per-frame `Vec`).
fn paint_geometry(painter: &Painter, geometry: &OrbGeometry) {
    if let Some((center, radius, alpha)) = geometry.ring {
        if alpha > 0.0 {
            painter.circle_stroke(
                center,
                radius,
                Stroke::new(1.0, with_alpha(color::FG_FAINT, alpha)),
            );
        }
    }
    if let Some((tail, head, alpha)) = geometry.beam {
        if alpha > 0.0 && head.x > tail.x {
            painter.line_segment(
                [tail, head],
                Stroke::new(1.0, with_alpha(color::FG_FAINT, alpha)),
            );
        }
    }
    for dot in geometry.dots_iter() {
        if dot.alpha > 0.0 && dot.radius > 0.0 {
            painter.circle_filled(
                dot.center,
                dot.radius,
                with_alpha(dot.tone.color(), dot.alpha),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [OrbState; 9] = [
        OrbState::Breathing,
        OrbState::Working,
        OrbState::Searching,
        OrbState::Solving,
        OrbState::Listening,
        OrbState::Connecting,
        OrbState::Weaving,
        OrbState::Composing,
        OrbState::Shaping,
    ];

    fn rect(size: f32) -> Rect {
        Rect::from_min_size(pos2(100.0, 100.0), vec2(size, size))
    }

    /// A geometry fingerprint: every painted primitive's kind + position,
    /// so two states compare unequal iff their pictures differ.
    fn fingerprint(g: &OrbGeometry) -> Vec<(i32, i32, i32, i32)> {
        let mut v: Vec<(i32, i32, i32, i32)> = g
            .dots_iter()
            .map(|d| {
                (
                    (d.center.x * 100.0) as i32,
                    (d.center.y * 100.0) as i32,
                    (d.radius * 100.0) as i32,
                    (d.alpha * 100.0) as i32,
                )
            })
            .collect();
        if let Some((c, r, a)) = g.ring {
            v.push((
                (c.x * 10.0) as i32,
                (r * 100.0) as i32,
                (a * 100.0) as i32,
                -1,
            ));
        }
        if let Some((t, h, a)) = g.beam {
            v.push((
                (t.x * 10.0) as i32,
                (h.x * 100.0) as i32,
                (a * 100.0) as i32,
                -2,
            ));
        }
        v
    }

    #[test]
    fn every_state_paints_distinct_geometry() {
        // At every sampled phase, each ordered pair of states must differ
        // somewhere in the sweep (distinct pictures, not just distinct
        // phases of one picture).
        let phases = [0.0, 0.13, 0.25, 0.37, 0.5, 0.63, 0.75, 0.88];
        for a in STATES {
            for b in STATES {
                if a == b {
                    continue;
                }
                let differs = phases.iter().any(|&p| {
                    fingerprint(&geometry(a, p, rect(ORB_INLINE)))
                        != fingerprint(&geometry(b, p, rect(ORB_INLINE)))
                });
                assert!(differs, "{a:?} and {b:?} paint identically");
            }
        }
    }

    #[test]
    fn every_state_animates_over_the_loop() {
        for state in STATES {
            let start = fingerprint(&geometry(state, 0.0, rect(ORB_INLINE)));
            let later = fingerprint(&geometry(state, 0.37, rect(ORB_INLINE)));
            assert_ne!(start, later, "{state:?} never moves");
        }
    }

    #[test]
    fn geometry_scales_with_size() {
        let small = geometry(OrbState::Solving, 0.25, rect(ORB_INLINE));
        let large = geometry(OrbState::Solving, 0.25, rect(ORB_AVATAR));
        let small_r = small.dots_iter().next().unwrap().radius;
        let large_r = large.dots_iter().next().unwrap().radius;
        assert!((large_r / small_r - ORB_AVATAR / ORB_INLINE).abs() < 0.01);
    }

    #[test]
    fn breathing_pulses_and_marks_liveness_at_avatar_scale() {
        let inline = geometry(OrbState::Breathing, 0.5, rect(ORB_INLINE));
        assert_eq!(inline.dots_iter().count(), 1, "inline: the dot only");
        let avatar = geometry(OrbState::Breathing, 0.5, rect(ORB_AVATAR));
        assert_eq!(avatar.dots_iter().count(), 2);
        assert!(avatar.dots_iter().any(|d| d.tone == Tone::Ok));
        // The pulse keyframes: alpha 1.0 at the edges, 0.7 mid-loop.
        let edge = geometry(OrbState::Breathing, 0.0, rect(ORB_INLINE));
        let mid = geometry(OrbState::Breathing, 0.5, rect(ORB_INLINE));
        let edge_alpha = edge.dots_iter().next().unwrap().alpha;
        let mid_alpha = mid.dots_iter().next().unwrap().alpha;
        assert!((edge_alpha - 1.0).abs() < 0.01);
        assert!((mid_alpha - 0.7).abs() < 0.05);
    }

    #[test]
    fn listening_ring_expands_and_fades() {
        let early = geometry(OrbState::Listening, 0.0, rect(ORB_INLINE));
        let late = geometry(OrbState::Listening, 0.9, rect(ORB_INLINE));
        let (_, r0, a0) = early.ring.expect("ring");
        let (_, r1, a1) = late.ring.expect("ring");
        assert!(r1 > r0);
        assert!(a1 < a0);
    }

    #[test]
    fn composing_follows_the_comet_envelope() {
        assert_eq!(comet_alpha(0.0), 0.0);
        assert!((comet_alpha(0.2) - 1.0).abs() < 1e-6);
        assert_eq!(comet_alpha(1.0), 0.0);
        let g = geometry(OrbState::Composing, 0.5, rect(ORB_INLINE));
        assert!(g.beam.is_some());
    }

    #[test]
    fn speed_is_a_clock_multiplier_clamped_below_one() {
        let orb = ThinkingOrb::new(OrbState::Working, ORB_INLINE);
        assert!((orb.phase(1.2) - 0.0).abs() < 1e-4, "one loop exactly");
        let half = ThinkingOrb::new(OrbState::Working, ORB_INLINE).speed(0.5);
        assert!((half.phase(1.2) - 0.5).abs() < 1e-4, "half speed");
        // §12: a speed above 1 clamps — no loop faster than 1.2s.
        let fast = ThinkingOrb::new(OrbState::Working, ORB_INLINE).speed(2.0);
        assert!((fast.phase(1.2) - 0.0).abs() < 1e-4);
    }

    #[test]
    fn paused_and_reduce_motion_freeze_the_clock() {
        let paused = ThinkingOrb::new(OrbState::Working, ORB_INLINE).paused(true);
        assert_eq!(paused.phase(99.0), 0.0);
        let reduced = ThinkingOrb::new(OrbState::Working, ORB_INLINE).reduce_motion(true);
        assert_eq!(reduced.phase(99.0), 0.0);
    }

    // -- headless paint -------------------------------------------------

    fn paint_frame(ctx: &egui::Context, time_s: f64, orb: &ThinkingOrb) -> egui::FullOutput {
        let input = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0))),
            time: Some(time_s),
            ..Default::default()
        };
        let orb = *orb;
        let mut out = ctx.run_ui(input, |ui| {
            orb.show(ui);
        });
        out.textures_delta.clear();
        out
    }

    fn circle_count(output: &egui::FullOutput) -> usize {
        output
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Circle(_)))
            .count()
    }

    #[test]
    fn every_state_paints_headlessly() {
        let ctx = egui::Context::default();
        for state in STATES {
            let orb = ThinkingOrb::new(state, ORB_INLINE);
            let out = paint_frame(&ctx, 0.42, &orb);
            assert!(
                circle_count(&out) >= 1,
                "{state:?} painted no dots: {:?}",
                out.shapes.len()
            );
        }
    }

    #[test]
    fn reduce_motion_paints_one_static_dot_and_requests_no_frames() {
        let ctx = egui::Context::default();
        // Warm the context (egui's first frames request follow-ups of
        // their own — the agent_cards measured_repaint_delay pattern).
        let orb = ThinkingOrb::new(OrbState::Working, ORB_INLINE).reduce_motion(true);
        for _ in 0..3 {
            paint_frame(&ctx, 0.0, &orb);
        }
        let a = paint_frame(&ctx, 1.0, &orb);
        let b = paint_frame(&ctx, 5.0, &orb);
        assert_eq!(circle_count(&a), 1);
        assert_eq!(circle_count(&b), 1);
        assert_eq!(a.shapes.len(), b.shapes.len(), "static: same shapes");
        let delay = a.viewport_output.values().next().map(|o| o.repaint_delay);
        assert_eq!(
            delay,
            Some(std::time::Duration::MAX),
            "reduce-motion requests no animation frames"
        );
    }

    #[test]
    fn an_animating_orb_requests_frames_a_paused_one_does_not() {
        let ctx = egui::Context::default();
        let live = ThinkingOrb::new(OrbState::Working, ORB_INLINE);
        let paused = live.paused(true);
        for _ in 0..3 {
            paint_frame(&ctx, 0.0, &live);
            paint_frame(&ctx, 0.0, &paused);
        }
        let live_delay = paint_frame(&ctx, 0.0, &live)
            .viewport_output
            .values()
            .next()
            .map(|o| o.repaint_delay);
        let paused_delay = paint_frame(&ctx, 0.0, &paused)
            .viewport_output
            .values()
            .next()
            .map(|o| o.repaint_delay);
        assert!(live_delay.is_some_and(|d| d < std::time::Duration::from_secs(1)));
        assert_eq!(paused_delay, Some(std::time::Duration::MAX));
    }
}
