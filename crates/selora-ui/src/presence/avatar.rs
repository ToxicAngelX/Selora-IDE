//! `AgentAvatar` — the agent identity glyph (Phase 6h; a native port of
//! the bot-avatars web library's roster bots).
//!
//! One ink silhouette — a rounded body with a visor and two eyes — painted
//! in the ink steps (`--ink-mid` body, `--ink-hi` hairline, `--ink-low`
//! visor, `--ink-faint` base band; the web version's fur/plastic shading
//! becomes ink steps). No accessories, no saturated color, no emoji (§12).
//!
//! Three states:
//!
//! - [`AvatarState::Default`] — idle: the eyes glance around on a seeded,
//!   stepped schedule (a 160ms `--dur-quick` shift per `--dur-loop` dwell)
//!   and blink every fourth loop (a 160ms lid dip) — the bot-avatars
//!   library's signature idle behavior, on token durations only.
//! - [`AvatarState::Working`] — a hop synced to the
//!   [`crate::presence::ThinkingOrb`] clock (both read the frame clock's
//!   `--dur-loop` phase, so the status line's pair moves together).
//! - [`AvatarState::Sleeping`] — head down, eyes closed, a slow breath
//!   (scale pulse on `--dur-loop`; a truly slower breath would exceed
//!   §7.2's 2000ms duration ceiling). Only when the gateway is
//!   disconnected.
//!
//! `seed` (0..1) offsets the blink/wander clocks so a future roster never
//! blinks in unison. State changes cross-fade over `--dur-quick` on
//! `--ease-calm` by interpolating the pose (a morph, not a double
//! exposure). Reduce-motion (§7.4) freezes the pose (eyes open, centered).
//!
//! 20px in the agent status line and tool-card headers; 64px is reserved
//! for a future multi-agent roster (the geometry is size-parameterized).

use egui::{pos2, vec2, Painter, Rect, Stroke, Ui};

use crate::theme::{color, motion, radius};

use super::{clock_s, pulse_eased, request_anim_frame, with_alpha, LOOP_S};

/// Inline size (the status line, card headers): 20px.
pub const AVATAR_INLINE: f32 = 20.0;
/// Roster scale (reserved for a future multi-agent surface): 64px.
pub const AVATAR_ROSTER: f32 = 64.0;

/// The cross-fade's duration (`--dur-quick`, §7.2).
const TRANSITION_S: f32 = motion::DUR_QUICK_MS as f32 / 1000.0;
/// The blink's length (one `--dur-quick` per four-loop cycle).
const BLINK_S: f32 = motion::DUR_QUICK_MS as f32 / 1000.0;
/// Blink cadence: one blink per this many loops.
const BLINK_CYCLE_LOOPS: f32 = 4.0;

/// The avatar's three states (the module docs' table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarState {
    /// Idle: look-around + blink.
    Default,
    /// Mid-turn: the hop (synced to the orb's clock).
    Working,
    /// Gateway disconnected: head down, eyes closed, slow breath.
    Sleeping,
}

/// One frame of the avatar, as data (tests assert on it; the painter is a
/// thin shell). All offsets are fractions of the widget's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvatarPose {
    /// Vertical body offset (the hop's lift, the sleeping sag).
    pub body_dy: f32,
    /// Body height scale (the hop's squash, the sleeping slump).
    pub body_squash: f32,
    /// Breath scale (sleeping) applied to the whole body.
    pub body_scale: f32,
    /// Eye offset (the look-around glance), signed.
    pub eye_dx: f32,
    /// 1.0 = open dots, 0.0 = closed lids (painted as hairlines).
    pub eye_open: f32,
}

impl AvatarPose {
    /// The neutral pose (upright, eyes open and centered).
    fn neutral() -> Self {
        Self {
            body_dy: 0.0,
            body_squash: 1.0,
            body_scale: 1.0,
            eye_dx: 0.0,
            eye_open: 1.0,
        }
    }

    /// The cross-fade: linear interpolation of every channel (the paint
    /// is a morph between poses, never a double exposure).
    pub fn lerp(a: &AvatarPose, b: &AvatarPose, t: f32) -> AvatarPose {
        let t = t.clamp(0.0, 1.0);
        let l = |x: f32, y: f32| x + (y - x) * t;
        AvatarPose {
            body_dy: l(a.body_dy, b.body_dy),
            body_squash: l(a.body_squash, b.body_squash),
            body_scale: l(a.body_scale, b.body_scale),
            eye_dx: l(a.eye_dx, b.eye_dx),
            eye_open: l(a.eye_open, b.eye_open),
        }
    }
}

/// The agent identity glyph. Owns the cross-fade clock; the loop phases
/// derive from the egui frame clock (no per-instance timer).
#[derive(Clone, Debug)]
pub struct AgentAvatar {
    state: AvatarState,
    /// 0..1 — offsets the blink/wander clocks (roster de-unison).
    seed: f32,
    reduce_motion: bool,
    /// The state being cross-faded FROM, with the transition's start
    /// (frame-clock seconds).
    previous: Option<(AvatarState, f64)>,
}

impl Default for AgentAvatar {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl AgentAvatar {
    /// An avatar in [`AvatarState::Default`] with the given seed (0..1).
    pub fn new(seed: f32) -> Self {
        Self {
            state: AvatarState::Default,
            seed: seed.clamp(0.0, 1.0),
            reduce_motion: false,
            previous: None,
        }
    }

    pub fn set_reduce_motion(&mut self, on: bool) {
        self.reduce_motion = on;
    }

    pub fn state(&self) -> AvatarState {
        self.state
    }

    /// Transition to `state` (a no-op when already there). The cross-fade
    /// plays over `--dur-quick` from `now` (frame-clock seconds).
    pub fn set_state(&mut self, state: AvatarState, now: f64) {
        if state == self.state {
            return;
        }
        self.previous = Some((self.state, now));
        self.state = state;
    }

    /// Allocate `size`×`size` in `ui` and paint (full alpha, live
    /// animation). Returns the rect.
    pub fn show(&mut self, ui: &mut Ui, size: f32) -> Rect {
        let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
        self.paint(ui, rect, 1.0, true);
        rect
    }

    /// Paint into a pre-laid-out rect. `alpha` multiplies the whole glyph
    /// (a card's fade-in); `live` is whether the surface is alive (the
    /// status line, a card of a running turn): a dead surface paints the
    /// current pose frozen and requests no frames — a frozen instrument
    /// reads frozen, and a sleeping glyph on a finished turn must not
    /// stream repaints forever.
    pub fn paint(&mut self, ui: &Ui, rect: Rect, alpha: f32, live: bool) {
        let now = clock_s(ui.ctx());
        let pose = self.current_pose(now);
        paint_pose(ui.painter(), rect, &pose, alpha);
        if self.previous.is_some() || (live && !self.reduce_motion) {
            request_anim_frame(ui.ctx());
        }
    }

    /// The pose for this frame: the state's own pose, cross-faded from
    /// the previous state's while the `--dur-quick` transition runs.
    fn current_pose(&mut self, now: f64) -> AvatarPose {
        let frozen_t = 0.0;
        let (t, seed) = if self.reduce_motion {
            // Frozen: the clocks pin at 0 and the blink/wander seeds
            // neutralize (no motion, so no unison problem) — every frozen
            // avatar reads eyes-open, centered.
            (frozen_t, 0.0)
        } else {
            (now as f32, self.seed)
        };
        let current = pose(self.state, t, seed);
        let Some((previous, started)) = self.previous else {
            return current;
        };
        let raw = ((now as f32) - started as f32) / TRANSITION_S;
        if raw >= 1.0 {
            self.previous = None;
            return current;
        }
        let eased = motion::ease_bezier(motion::EASE_CALM, raw);
        AvatarPose::lerp(&pose(previous, t, seed), &current, eased)
    }
}

/// The pure heart: `state`'s pose at `t_s` seconds. Every motion is on
/// the §7.2 tokens: loops are `--dur-loop`, the blink and the glance
/// shift are `--dur-quick` moves inside a loop-long dwell — nothing runs
/// faster than 80ms or drags past 2000ms.
pub fn pose(state: AvatarState, t_s: f32, seed: f32) -> AvatarPose {
    let mut pose = AvatarPose::neutral();
    let loop_phase = (t_s / LOOP_S).rem_euclid(1.0);
    match state {
        AvatarState::Default => {
            pose.eye_dx = glance(t_s, seed);
            pose.eye_open = blink(t_s, seed);
        }
        AvatarState::Working => {
            // The hop: quick lift, soft landing, one per --dur-loop (the
            // orb's clock — the status line's pair moves together).
            let hop = if loop_phase < 0.35 {
                motion::ease_bezier(motion::EASE_ORBITAL, loop_phase / 0.35)
            } else {
                motion::ease_bezier(motion::EASE_ORBITAL, 1.0 - (loop_phase - 0.35) / 0.65)
            };
            pose.body_dy = -0.07 * hop;
            pose.body_squash = 1.0 - 0.05 * hop;
        }
        AvatarState::Sleeping => {
            // Head down, lids shut, a slow breath (the pulse on
            // --dur-loop; §7.2's ceiling forbids a slower loop).
            pose.body_dy = 0.06;
            pose.body_squash = 0.94;
            pose.body_scale = 1.0 + 0.03 * pulse_eased(loop_phase);
            pose.eye_open = 0.0;
        }
    }
    pose
}

/// The idle glance: a stepped look-around — dwell one `--dur-loop` per
/// direction (left / center / right / center), shift over `--dur-quick`
/// on `--ease-calm`, seeded so a roster never glances in unison.
fn glance(t_s: f32, seed: f32) -> f32 {
    // Center first: the frozen pose (t = 0) reads eyes-forward.
    const DIRS: [f32; 4] = [0.0, 1.0, 0.0, -1.0];
    let step = t_s / LOOP_S + seed * BLINK_CYCLE_LOOPS;
    let index = step.floor().max(0.0) as u32;
    let frac = step.fract();
    let from = DIRS[(index % 4) as usize];
    let to = DIRS[((index + 1) % 4) as usize];
    let shift = ((frac * LOOP_S) / BLINK_S).clamp(0.0, 1.0);
    0.05 * (from + (to - from) * motion::ease_bezier(motion::EASE_CALM, shift))
}

/// The blink: a `--dur-quick` lid dip once per four-loop cycle, seeded.
/// The window sits a quarter-cycle in so the frozen pose (t = 0) and its
/// neighborhood keep their eyes open.
fn blink(t_s: f32, seed: f32) -> f32 {
    let cycle = (t_s / (LOOP_S * BLINK_CYCLE_LOOPS) + seed).rem_euclid(1.0);
    let seconds = cycle * LOOP_S * BLINK_CYCLE_LOOPS;
    let start = LOOP_S * BLINK_CYCLE_LOOPS * 0.25;
    if !(start..start + BLINK_S).contains(&seconds) {
        return 1.0;
    }
    let p = (seconds - start) / BLINK_S;
    // Down fast, back up on the same quick ease: 1 → 0.1 → 1.
    let dip = if p < 0.5 { p * 2.0 } else { (1.0 - p) * 2.0 };
    1.0 - 0.9 * motion::ease_bezier(motion::EASE_CALM, dip)
}

/// Paint one pose (thin shell over egui primitives — fixed geometry, no
/// per-frame allocation).
fn paint_pose(painter: &Painter, rect: Rect, pose: &AvatarPose, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let center = pos2(c.x, c.y + pose.body_dy * s);

    // The body: a rounded rect in --ink-mid with the 1px --ink-hi
    // hairline; the corner radius caps at --radius-2 (§4: the brand is
    // sharp — no radius larger than 4px even at roster scale).
    let body_w = 0.70 * s * pose.body_scale;
    let body_h = 0.58 * s * pose.body_squash * pose.body_scale;
    let body = Rect::from_center_size(center, vec2(body_w, body_h));
    let body_corner = (0.15 * s).min(radius::R2);
    painter.rect_filled(body, body_corner, with_alpha(color::INK_MID, alpha));
    painter.rect(
        body,
        body_corner,
        egui::Color32::TRANSPARENT,
        Stroke::new(1.0, with_alpha(color::INK_HI, alpha)),
        egui::StrokeKind::Inside,
    );

    // The base band (the shading's ink step) and the visor (the face
    // plate) — both clipped to the body so the silhouette stays crisp.
    let body_painter = painter.with_clip_rect(body);
    let band = Rect::from_min_max(
        pos2(body.left(), body.bottom() - 0.10 * s),
        body.right_bottom(),
    );
    body_painter.rect_filled(
        band,
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: body_corner as u8,
            se: body_corner as u8,
        },
        with_alpha(color::INK_FAINT, alpha),
    );

    let visor = Rect::from_center_size(
        pos2(center.x, body.top() + body_h * 0.38),
        vec2(body_w - 0.16 * s, 0.30 * s),
    );
    let visor_corner = (0.08 * s).min(radius::R1);
    body_painter.rect_filled(visor, visor_corner, with_alpha(color::INK_LOW, alpha));

    // The eyes: open = --fg-muted dots on the visor centerline; closed
    // (sleeping, mid-blink) = 1px hairlines.
    let eye_y = visor.center().y;
    for side in [-1.0f32, 1.0] {
        let eye = pos2(center.x + (side * 0.13 + pose.eye_dx) * s, eye_y);
        if pose.eye_open > 0.3 {
            body_painter.circle_filled(
                eye,
                0.055 * s * pose.eye_open,
                with_alpha(color::FG_MUTED, alpha),
            );
        } else {
            let half = 0.045 * s;
            body_painter.line_segment(
                [pos2(eye.x - half, eye.y), pos2(eye.x + half, eye.y)],
                Stroke::new(1.0, with_alpha(color::FG_FAINT, alpha)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(size: f32) -> Rect {
        Rect::from_min_size(pos2(50.0, 50.0), vec2(size, size))
    }

    #[test]
    fn the_three_states_pose_distinctly() {
        let t = 3.7;
        let idle = pose(AvatarState::Default, t, 0.0);
        let working = pose(AvatarState::Working, 0.2, 0.0);
        let sleeping = pose(AvatarState::Sleeping, t, 0.0);
        assert_ne!(idle, working);
        assert_ne!(working, sleeping);
        assert_ne!(idle, sleeping);
        // Sleeping: head down, eyes shut.
        assert!(sleeping.body_dy > 0.0);
        assert_eq!(sleeping.eye_open, 0.0);
        // Working mid-hop: lifted.
        assert!(working.body_dy < 0.0);
    }

    #[test]
    fn the_hop_is_synced_to_the_orb_clock() {
        // The hop's period is exactly --dur-loop (the orb's loop): lifted
        // early, landed by the loop's end, identical at loop boundaries.
        let up = pose(AvatarState::Working, 0.1, 0.0);
        let down = pose(AvatarState::Working, 1.1, 0.0);
        assert!(up.body_dy < down.body_dy, "up {up:?} vs down {down:?}");
        let a = pose(AvatarState::Working, 0.3, 0.0);
        let b = pose(AvatarState::Working, 0.3 + LOOP_S, 0.0);
        assert_eq!(a, b, "the hop repeats every --dur-loop");
    }

    #[test]
    fn the_seed_offsets_blink_and_glance() {
        // Two seeds never share a blink phase; the glance differs too.
        let a = pose(AvatarState::Default, 2.0, 0.1);
        let b = pose(AvatarState::Default, 2.0, 0.7);
        assert_ne!(a.eye_dx, b.eye_dx);
        // Somewhere in a cycle each seed blinks (eye_open dips below 1)…
        let blinks = |seed: f32| {
            (0..48)
                .map(|i| pose(AvatarState::Default, i as f32 * 0.1, seed).eye_open)
                .any(|open| open < 0.9)
        };
        assert!(blinks(0.0));
        assert!(blinks(0.5));
    }

    #[test]
    fn blink_and_glance_moves_stay_on_token_durations() {
        // The blink dip lasts exactly --dur-quick inside its cycle.
        let cycle_s = LOOP_S * BLINK_CYCLE_LOOPS;
        let mut open_streak = 0.0;
        for i in 0..(cycle_s * 20.0) as usize {
            let t = i as f32 * 0.05;
            if pose(AvatarState::Default, t, 0.0).eye_open < 1.0 {
                open_streak += 0.05;
            }
        }
        assert!(
            (0.05..=BLINK_S + 0.1).contains(&open_streak),
            "one short blink per cycle (sampled at 50ms): {open_streak}"
        );
    }

    #[test]
    fn the_cross_fade_morphs_halfway() {
        let mut avatar = AgentAvatar::new(0.0);
        avatar.set_state(AvatarState::Sleeping, 10.0);
        let mid = avatar.current_pose(10.0 + TRANSITION_S as f64 / 2.0);
        let from = pose(AvatarState::Default, 10.0, 0.0);
        let to = pose(AvatarState::Sleeping, 10.0, 0.0);
        // Mid-cross-fade: every channel sits strictly between its ends
        // (ease-calm's midpoint is ~0.5).
        let between = |mid: f32, a: f32, b: f32| {
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            mid > lo && mid < hi
        };
        assert!(between(mid.body_dy, from.body_dy, to.body_dy));
        assert!(between(mid.eye_open, from.eye_open, to.eye_open));
        // The transition completes and the clock clears.
        let done_t = 10.0 + TRANSITION_S as f64 + 1.0;
        let done = avatar.current_pose(done_t);
        assert_eq!(done, pose(AvatarState::Sleeping, done_t as f32, 0.0));
        assert!(avatar.previous.is_none());
    }

    #[test]
    fn set_state_is_a_no_op_when_already_there() {
        let mut avatar = AgentAvatar::new(0.0);
        avatar.set_state(AvatarState::Default, 1.0);
        assert!(avatar.previous.is_none());
        avatar.set_state(AvatarState::Working, 1.0);
        assert_eq!(avatar.previous, Some((AvatarState::Default, 1.0)));
    }

    // -- headless paint -------------------------------------------------

    fn paint_frame(
        ctx: &egui::Context,
        time_s: f64,
        avatar: &mut AgentAvatar,
        size: f32,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0))),
            time: Some(time_s),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            avatar.show(ui, size);
        });
        out.textures_delta.clear();
        out
    }

    fn count(output: &egui::FullOutput, pred: impl Fn(&egui::Shape) -> bool) -> usize {
        output.shapes.iter().filter(|s| pred(&s.shape)).count()
    }

    #[test]
    fn every_state_paints_headlessly_at_both_sizes() {
        let ctx = egui::Context::default();
        for state in [
            AvatarState::Default,
            AvatarState::Working,
            AvatarState::Sleeping,
        ] {
            for size in [AVATAR_INLINE, AVATAR_ROSTER] {
                let mut avatar = AgentAvatar::new(0.3);
                avatar.set_state(state, -1.0); // long ago: no cross-fade
                let out = paint_frame(&ctx, 2.4, &mut avatar, size);
                let rects = count(
                    &out,
                    |s| matches!(s, egui::Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT),
                );
                assert!(
                    rects >= 3,
                    "{state:?} at {size}px paints body+band+visor: {rects}"
                );
            }
        }
    }

    #[test]
    fn sleeping_paints_closed_eyes_as_hairlines() {
        let ctx = egui::Context::default();
        let mut sleeping = AgentAvatar::new(0.0);
        sleeping.set_state(AvatarState::Sleeping, -1.0);
        let out = paint_frame(&ctx, 2.0, &mut sleeping, AVATAR_INLINE);
        let lines = count(&out, |s| matches!(s, egui::Shape::LineSegment { .. }));
        assert!(lines >= 2, "closed eyes are two hairlines: {lines}");
        // No eye dots when asleep.
        let mut awake = AgentAvatar::new(0.0);
        let out = paint_frame(&ctx, 2.0, &mut awake, AVATAR_INLINE);
        let eye_dots = count(
            &out,
            |s| matches!(s, egui::Shape::Circle(c) if c.fill == color::FG_MUTED),
        );
        assert_eq!(eye_dots, 2, "open eyes are two dots");
    }

    #[test]
    fn reduce_motion_freezes_the_pose_and_requests_no_frames() {
        let ctx = egui::Context::default();
        let mut avatar = AgentAvatar::new(0.3);
        avatar.set_reduce_motion(true);
        for _ in 0..3 {
            paint_frame(&ctx, 0.0, &mut avatar, AVATAR_INLINE);
        }
        let a = paint_frame(&ctx, 1.0, &mut avatar, AVATAR_INLINE);
        let b = paint_frame(&ctx, 7.0, &mut avatar, AVATAR_INLINE);
        assert_eq!(a.shapes.len(), b.shapes.len(), "frozen: same shapes");
        let delay = a.viewport_output.values().next().map(|o| o.repaint_delay);
        assert_eq!(delay, Some(std::time::Duration::MAX));
    }

    #[test]
    fn a_dead_surface_paints_frozen_and_requests_no_frames() {
        // A sleeping avatar on a finished turn's card (live = false):
        // the pose paints, but no repaint stream — the frozen instrument.
        let ctx = egui::Context::default();
        let mut avatar = AgentAvatar::new(0.3);
        avatar.set_state(AvatarState::Sleeping, -10.0);
        let warm = |avatar: &mut AgentAvatar| {
            let input = egui::RawInput {
                focused: true,
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0))),
                time: Some(3.0),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                avatar.paint(ui, rect(AVATAR_INLINE), 1.0, false);
            });
            out.textures_delta.clear();
            out
        };
        for _ in 0..3 {
            warm(&mut avatar);
        }
        let out = warm(&mut avatar);
        let delay = out.viewport_output.values().next().map(|o| o.repaint_delay);
        assert_eq!(delay, Some(std::time::Duration::MAX));
        // Live: the same sleeping avatar animates (the breath).
        let mut live = AgentAvatar::new(0.3);
        live.set_state(AvatarState::Sleeping, -10.0);
        let input = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0))),
            time: Some(3.0),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            live.paint(ui, rect(AVATAR_INLINE), 1.0, true);
        });
        let delay = out.viewport_output.values().next().map(|o| o.repaint_delay);
        out.textures_delta.clear();
        assert!(delay.is_some_and(|d| d < std::time::Duration::from_secs(1)));
    }
}
