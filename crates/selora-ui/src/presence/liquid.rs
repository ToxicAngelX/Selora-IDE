//! `LiquidPill` — the gooey melt on the chat header's agent-mode pill
//! (Phase 6h; a native port of the liquid-gooey web library, applied to
//! ONE seam: the `agent` pill, UI_SPEC.md §4.2).
//!
//! Off → on: the `--ok` dot slides into its slot (with a goo trail — the
//! web version's SVG blur/contrast merge becomes ≤ 4 layered alpha
//! circles, under the brief's ≤12 cap) while the pill's background melts
//! between `--ink-low` and `--ink-mid`. On → off: the reverse. The
//! [`PillTransition`] prop selects one of the four §7.1 curves; the melt
//! runs on `--dur-base` (240ms).
//!
//! Two deliberate deviations from 5d's pill, both mandated by the brief
//! and flagged for the owner: the pill gains a resting background fill
//! (a melt needs something to melt between — 5d's chip was border-only),
//! and the pill's width is constant (the dot slot is always reserved —
//! the gooey metaphor needs a fixed track; 5d's chip resized on toggle).
//!
//! Reduce-motion (§7.4): the fill's cross-fade stays (`fade` is kept);
//! the dot fades in place — no slide, no trail.

use egui::{pos2, vec2, Pos2, Rect, Sense, Stroke, Ui};

use crate::theme::{color, font, motion, radius, space, type_scale};

use super::{clock_s, request_anim_frame, with_alpha};

/// The pill's height (the 5d header chip).
pub const PILL_HEIGHT: f32 = 24.0;
/// The melt's duration (`--dur-base`, §7.2).
const MELT_S: f32 = motion::DUR_BASE_MS as f32 / 1000.0;
/// The dot's diameter (the 5d `--ok` dot).
const DOT: f32 = 6.0;
/// The goo trail's circle count (the brief's cap is 12; four suffice).
const GOO_STEPS: usize = 4;
/// The goo trail's alpha falloff (head to tail).
const GOO_ALPHA: [f32; GOO_STEPS] = [1.0, 0.5, 0.28, 0.14];
/// The goo trail's radius falloff (head to tail).
const GOO_RADIUS: [f32; GOO_STEPS] = [1.0, 0.85, 0.7, 0.55];

/// The four §7.1 curves, as the pill's transition flavors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PillTransition {
    /// `--ease-precise` — snappy.
    Sharp,
    /// `--ease-orbital`.
    Pulse,
    /// `--ease-calm` — the default (§7.1's default curve).
    #[default]
    Soft,
    /// `--ease-fade`.
    Bouncy,
}

impl PillTransition {
    /// The token curve for this transition.
    pub fn curve(self) -> (f32, f32, f32, f32) {
        match self {
            PillTransition::Sharp => motion::EASE_PRECISE,
            PillTransition::Pulse => motion::EASE_ORBITAL,
            PillTransition::Soft => motion::EASE_CALM,
            PillTransition::Bouncy => motion::EASE_FADE,
        }
    }
}

/// One frame of the melt, as data (tests assert on it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PillFrame {
    /// 0 = `--ink-low`, 1 = `--ink-mid` (the background melt).
    pub fill_t: f32,
    /// 0 = the dot at the pill's left edge, 1 = the dot in its slot.
    pub dot_t: f32,
    /// The dot's alpha (it fades in over the first 35% of the slide).
    pub dot_alpha: f32,
}

/// The agent-mode pill. The panel owns one; `set_on` is driven from the
/// panel's `agent_mode` every frame (the first call snaps — a boot
/// restore must not animate).
#[derive(Clone, Debug, Default)]
pub struct LiquidPill {
    on: bool,
    transition: PillTransition,
    reduce_motion: bool,
    /// The melt's clock: (frame-clock start, animating toward on).
    anim: Option<(f64, bool)>,
    /// Whether `set_on` has run once (the first call snaps, no melt).
    initialized: bool,
}

/// What one [`LiquidPill::show`] produced.
#[derive(Clone, Copy, Debug)]
pub struct PillOutcome {
    /// The pill's rect (the panel's `agent_pill_rect` test aid).
    pub rect: Rect,
    /// The pill was clicked this frame.
    pub clicked: bool,
}

impl LiquidPill {
    pub fn new(transition: PillTransition) -> Self {
        Self {
            transition,
            ..Default::default()
        }
    }

    pub fn set_reduce_motion(&mut self, on: bool) {
        self.reduce_motion = on;
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Drive the pill from the source of truth (the panel's agent-mode
    /// flag). The first call snaps without animating (boot restore); a
    /// later change starts the melt.
    pub fn set_on(&mut self, on: bool, now: f64) {
        if !self.initialized {
            self.initialized = true;
            self.on = on;
            return;
        }
        if on != self.on {
            self.on = on;
            self.anim = Some((now, on));
        }
    }

    /// The melt's frame at `now` (and clears a finished clock).
    fn frame(&mut self, now: f64) -> PillFrame {
        let Some((started, toward_on)) = self.anim else {
            return PillFrame::at(if self.on { 1.0 } else { 0.0 });
        };
        let raw = ((now - started) as f32 / MELT_S).clamp(0.0, 1.0);
        if raw >= 1.0 {
            self.anim = None;
        }
        let eased = if self.reduce_motion {
            // The dot pops in place; the fill still eases (fade is kept,
            // §7.4) — dot_t jumps, fill eases.
            motion::ease_bezier(motion::EASE_FADE, raw)
        } else {
            motion::ease_bezier(self.transition.curve(), raw)
        };
        let t = if toward_on { eased } else { 1.0 - eased };
        PillFrame::at(t)
    }

    /// Paint the pill, right-aligned at `right_top` (its top-right
    /// corner), and sense clicks on it. `label` is the chip's text
    /// ("agent").
    pub fn show(&mut self, ui: &Ui, right_top: Pos2, label: &str) -> PillOutcome {
        let now = clock_s(ui.ctx());
        let frame = self.frame(now);
        if self.anim.is_some() {
            request_anim_frame(ui.ctx());
        }

        let painter = ui.painter();
        let galley = painter.layout_no_wrap(
            label.to_owned(),
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_MUTED,
        );
        // Constant width: the dot slot is always reserved (the gooey
        // track); the melt never churns the header's layout.
        let width = space::S2 + DOT + space::S1 + galley.size().x + space::S2;
        let rect = Rect::from_min_size(
            pos2(right_top.x - width, right_top.y),
            vec2(width, PILL_HEIGHT),
        );
        let response = ui.interact(
            rect,
            egui::Id::new("selora_chat_agent_pill"),
            Sense::click(),
        );

        paint_pill(
            ui,
            rect,
            &galley,
            &frame,
            response.hovered(),
            self.reduce_motion,
        );

        PillOutcome {
            rect,
            clicked: response.clicked(),
        }
    }
}

impl PillFrame {
    /// The frame at melt progress `t` (0 = off, 1 = on).
    pub fn at(t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        // The dot appears over the first 35% of the slide, and is gone by
        // 65% on the way out.
        let dot_alpha = (t / 0.35).clamp(0.0, 1.0);
        Self {
            fill_t: t,
            dot_t: t,
            dot_alpha,
        }
    }
}

/// The melt's fill color: `--ink-low` ↔ `--ink-mid`, raised one ink step
/// on hover (§10's hover rule: hover lifts the surface one step).
fn fill_color(t: f32, hovered: bool) -> egui::Color32 {
    let base = lerp_color(color::INK_LOW, color::INK_MID, t);
    let hovered_color = lerp_color(color::INK_MID, color::INK_HI, t);
    if hovered {
        hovered_color
    } else {
        base
    }
}

/// Channel-wise lerp between two token colors (the melt — no new color
/// values, just the two endpoints mixed at paint time).
fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Paint the pill: the melting background, the 1px `--ink-hi` border, the
/// sliding `--ok` dot with its goo trail, the `--type-xs` label.
fn paint_pill(
    ui: &Ui,
    rect: Rect,
    galley: &std::sync::Arc<egui::Galley>,
    frame: &PillFrame,
    hovered: bool,
    reduce_motion: bool,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, radius::PILL, fill_color(frame.fill_t, hovered));
    painter.rect(
        rect,
        radius::PILL,
        egui::Color32::TRANSPARENT,
        Stroke::new(1.0, color::INK_HI),
        egui::StrokeKind::Inside,
    );

    // The dot's travel: from the pill's left edge to its slot (a short,
    // deliberate slide — the goo follows it).
    let target = rect.left() + space::S2 + DOT / 2.0;
    let origin = rect.left() + 1.0;
    let dot_x = if reduce_motion {
        target // in place: no slide under reduce-motion
    } else {
        origin + (target - origin) * frame.dot_t
    };
    let dot_center = pos2(dot_x, rect.center().y);
    if frame.dot_alpha > 0.0 {
        if !reduce_motion && frame.dot_t > 0.0 && frame.dot_t < 1.0 {
            // The goo trail: GOO_STEPS layered circles along the slide
            // path, fading and shrinking toward the tail (the web
            // version's blur merge, stepped — no smooth gradient, §12).
            let travel = (target - origin).abs();
            for step in 1..GOO_STEPS {
                let back = step as f32 / GOO_STEPS as f32;
                let x = dot_x - (target - origin).signum() * travel * 0.18 * back;
                painter.circle_filled(
                    pos2(x, rect.center().y),
                    DOT / 2.0 * GOO_RADIUS[step],
                    with_alpha(color::OK, frame.dot_alpha * GOO_ALPHA[step]),
                );
            }
        }
        painter.circle_filled(
            dot_center,
            DOT / 2.0,
            with_alpha(color::OK, frame.dot_alpha),
        );
    }

    // The label, right of the dot slot.
    painter.galley(
        pos2(
            rect.left() + space::S2 + DOT + space::S1,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley.clone(),
        color::FG_MUTED,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_transitions_map_to_the_four_token_curves() {
        assert_eq!(PillTransition::Sharp.curve(), motion::EASE_PRECISE);
        assert_eq!(PillTransition::Pulse.curve(), motion::EASE_ORBITAL);
        assert_eq!(PillTransition::Soft.curve(), motion::EASE_CALM);
        assert_eq!(PillTransition::Bouncy.curve(), motion::EASE_FADE);
    }

    #[test]
    fn pill_frame_endpoints_and_dot_window() {
        let off = PillFrame::at(0.0);
        assert_eq!((off.fill_t, off.dot_t, off.dot_alpha), (0.0, 0.0, 0.0));
        let on = PillFrame::at(1.0);
        assert_eq!(on.fill_t, 1.0);
        assert_eq!(on.dot_t, 1.0);
        assert_eq!(on.dot_alpha, 1.0);
        let mid = PillFrame::at(0.5);
        assert!(mid.dot_alpha >= 1.0, "the dot is fully in by mid-melt");
        assert!((mid.fill_t - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_first_set_on_snaps_without_animating() {
        let mut pill = LiquidPill::default();
        pill.set_on(true, 5.0);
        assert!(pill.is_on());
        assert!(pill.anim.is_none(), "boot restore never melts");
        // A later change melts.
        pill.set_on(false, 6.0);
        assert!(pill.anim.is_some());
    }

    #[test]
    fn the_melt_runs_on_dur_base_and_clears() {
        let mut pill = LiquidPill::new(PillTransition::Soft);
        pill.set_on(false, 0.0); // initialize off
        pill.set_on(true, 1.0);
        let mid = pill.frame(1.0 + MELT_S as f64 / 2.0);
        assert!(mid.fill_t > 0.0 && mid.fill_t < 1.0);
        assert!(pill.anim.is_some());
        let done = pill.frame(1.0 + MELT_S as f64 + 0.1);
        assert_eq!(done.fill_t, 1.0);
        assert!(pill.anim.is_none(), "the clock clears at the end");
        // And melts back.
        pill.set_on(false, 5.0);
        let back = pill.frame(5.0 + MELT_S as f64 + 0.1);
        assert_eq!(back.fill_t, 0.0);
    }

    // -- headless paint -------------------------------------------------

    fn frame(
        ctx: &egui::Context,
        time_s: f64,
        pill: &mut LiquidPill,
    ) -> (egui::FullOutput, PillOutcome) {
        let input = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0))),
            time: Some(time_s),
            ..Default::default()
        };
        let mut outcome = PillOutcome {
            rect: Rect::ZERO,
            clicked: false,
        };
        let mut out = ctx.run_ui(input, |ui| {
            outcome = pill.show(ui, pos2(360.0, 8.0), "agent");
        });
        out.textures_delta.clear();
        (out, outcome)
    }

    fn pill_fill(out: &egui::FullOutput) -> Option<egui::Color32> {
        out.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT => Some(r.fill),
            _ => None,
        })
    }

    /// Circles in `--ok`'s hue. egui's `Color32` stores premultiplied
    /// alpha, so the goo trail's rgb channels scale with their alpha —
    /// compare the hue ratio (premultiply-invariant), not the raw
    /// channels.
    fn ok_circles(out: &egui::FullOutput) -> usize {
        out.shapes
            .iter()
            .filter(|s| {
                matches!(&s.shape, egui::Shape::Circle(c)
                    if c.fill.a() > 0 && c.fill.g() > c.fill.r() && c.fill.g() > c.fill.b())
            })
            .count()
    }

    #[test]
    fn off_on_and_mid_melt_paint_distinctly() {
        let ctx = egui::Context::default();
        let mut pill = LiquidPill::new(PillTransition::Soft);
        pill.set_on(false, 0.0);

        let (off_out, off) = frame(&ctx, 0.0, &mut pill);
        assert_eq!(pill_fill(&off_out), Some(color::INK_LOW));
        assert_eq!(ok_circles(&off_out), 0, "off: no dot");

        pill.set_on(true, 0.5);
        let (mid_out, _) = frame(&ctx, 0.5 + MELT_S as f64 / 2.0, &mut pill);
        let mid_fill = pill_fill(&mid_out).expect("fill");
        assert_ne!(mid_fill, color::INK_LOW, "mid-melt: the fill moved");
        assert_ne!(mid_fill, color::INK_MID, "mid-melt: not there yet");
        assert!(
            ok_circles(&mid_out) > 1,
            "mid-melt: the dot plus its goo trail"
        );

        let (on_out, on) = frame(&ctx, 0.5 + MELT_S as f64 + 0.1, &mut pill);
        assert_eq!(pill_fill(&on_out), Some(color::INK_MID));
        assert_eq!(ok_circles(&on_out), 1, "settled: just the dot");
        // Constant width across the toggle (no layout churn).
        assert_eq!(off.rect.width(), on.rect.width());
    }

    #[test]
    fn reduce_motion_fades_the_dot_in_place_without_a_trail() {
        let ctx = egui::Context::default();
        let mut pill = LiquidPill::new(PillTransition::Soft);
        pill.set_reduce_motion(true);
        pill.set_on(false, 0.0);
        pill.set_on(true, 1.0);
        let (mid_out, _) = frame(&ctx, 1.0 + MELT_S as f64 / 2.0, &mut pill);
        assert!(
            ok_circles(&mid_out) <= 1,
            "no goo trail under reduce-motion"
        );
        // The fill still eases (fade is kept, §7.4).
        let mid_fill = pill_fill(&mid_out).expect("fill");
        assert_ne!(mid_fill, color::INK_LOW);
        assert_ne!(mid_fill, color::INK_MID);
    }

    #[test]
    fn each_transition_curve_reaches_the_midpoint_differently() {
        // Sharp (precise) is well past the midpoint at t=0.5; bouncy
        // (fade) trails it — the prop is a real curve switch.
        let mut fills = Vec::new();
        for transition in [
            PillTransition::Sharp,
            PillTransition::Pulse,
            PillTransition::Soft,
            PillTransition::Bouncy,
        ] {
            let mut pill = LiquidPill::new(transition);
            pill.set_on(false, 0.0);
            pill.set_on(true, 0.0);
            fills.push(pill.frame(MELT_S as f64 / 2.0).fill_t);
        }
        assert!(fills[0] > fills[3], "sharp leads bouncy: {fills:?}");
        assert!(fills.iter().any(|f| *f != fills[0]), "curves differ");
    }

    #[test]
    fn the_pill_reports_clicks() {
        let ctx = egui::Context::default();
        let mut pill = LiquidPill::default();
        pill.set_on(false, 0.0);
        let (_, outcome) = frame(&ctx, 0.0, &mut pill);
        let center = outcome.rect.center();
        let click = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0))),
            time: Some(0.1),
            events: vec![
                egui::Event::PointerMoved(center),
                egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        };
        let mut out = PillOutcome {
            rect: Rect::ZERO,
            clicked: false,
        };
        let mut full = ctx.run_ui(click, |ui| {
            out = pill.show(ui, pos2(360.0, 8.0), "agent");
        });
        full.textures_delta.clear();
        assert!(out.clicked, "the click lands");
    }
}
