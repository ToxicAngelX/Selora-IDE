//! AI presence & motion effects (Phase 6h) — native egui ports.
//!
//! The six reference web libraries (thinking-orbs, bot-avatars,
//! voice-glow, liquid-gooey, metal-fx, border-beam) are JavaScript;
//! Selora IDE is pure egui (PLAN.md §2 non-goals — no webview, no React,
//! no JS). These modules port the libraries' *behavior* — their state
//! sets, reaction curves, and motion semantics — as painted widgets in
//! the Selora monochrome design language. No package is installed; the
//! libraries were reference material.
//!
//! Every widget here is paint-only (ARCHITECTURE.md §1: no threads, no
//! channels), driven by the egui frame clock (`Context::input(|i| i.time)`)
//! and `request_repaint_after` while animating — the `ghost_text.rs` /
//! `agent_cards.rs` pattern. Every color, duration, and curve comes from
//! [`crate::theme`] (DESIGN_TOKENS.md §11/§12): ink ramp only, `--ok`
//! reserved for liveness/semantics, loops on `--dur-loop` (no loop faster
//! than 1.2s, §12), and `ui.reduce_motion` (§7.4) freezes every loop.
//!
//! - [`orb`] — `ThinkingOrb`: the nine-state thought indicator (replaces the
//!   streaming bubble's `⠿⠿⠿` wave; rides the agent status line).
//! - [`avatar`] — `AgentAvatar`: the agent identity glyph (status line and
//!   tool-card headers).
//! - [`stream_glow`] — `StreamGlow`: the composer's bottom-edge glow, driven
//!   by streaming activity (Lite has no microphone — the voice-glow port is
//!   reactive to deltas, not audio).
//! - [`liquid`] — `LiquidPill`: the gooey melt on the chat header's
//!   agent-mode pill.
//! - [`metal`] — `MetalMark`: the liquid-metal sheen on the splash brand
//!   mark, behind `metal::METAL_MARK_ENABLED` (owner-decision item).
//! - [`beam`] — `BorderBeam`: the in-flight beam riding the composer
//!   card's perimeter (REVAMP_SPEC §8; the border-beam port).

use std::time::Duration;

use crate::theme::motion;

pub mod avatar;
pub mod beam;
pub mod liquid;
pub mod metal;
pub mod orb;
pub mod stream_glow;

pub use avatar::{AgentAvatar, AvatarState};
pub use beam::BorderBeam;
pub use liquid::{LiquidPill, PillTransition};
pub use orb::{OrbState, ThinkingOrb};
pub use stream_glow::StreamGlow;

/// The repaint cadence while a presence widget animates (one `--dur-loop`
/// sixtieth ≈ 20ms — the `agent_cards.rs` ANIM_FRAME_MS precedent: smooth
/// without busy-looping).
pub(crate) const ANIM_FRAME: Duration = Duration::from_millis(motion::DUR_LOOP_MS as u64 / 60);

/// The one animation loop period in seconds (`--dur-loop`, §7.2).
pub(crate) const LOOP_S: f32 = motion::DUR_LOOP_MS as f32 / 1000.0;

/// The border beam's perimeter lap in seconds (`--dur-lap`, REVAMP_SPEC
/// §8.3 — exactly 2 × `--dur-loop`, the §11 bend-6a token).
pub(crate) const LAP_S: f32 = motion::DUR_LAP_MS as f32 / 1000.0;

/// The egui frame clock in seconds (the `agent_cards.rs` status-line
/// pattern; `RawInput.time` drives it headlessly).
pub(crate) fn clock_s(ctx: &egui::Context) -> f64 {
    ctx.input(|i| i.time)
}

/// Ask for the next animation frame while a widget's motion is live.
pub(crate) fn request_anim_frame(ctx: &egui::Context) {
    ctx.request_repaint_after(ANIM_FRAME);
}

/// `base` with its alpha scaled (0..1). The token colors are opaque; alpha
/// is applied at paint time so no new color values enter the system
/// (§12) — the `ghost_text.rs` / `chat/mod.rs` precedent.
pub(crate) fn with_alpha(base: egui::Color32, alpha: f32) -> egui::Color32 {
    let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha)
}

/// The §7.3 `pulse` keyframes' easing position: a symmetric triangle wave
/// (0%↔100% and 50%) through `--ease-orbital` (the `agent_cards.rs`
/// `pulse_eased` formula, hoisted so the presence widgets share it).
pub(crate) fn pulse_eased(phase: f32) -> f32 {
    let phase = phase.rem_euclid(1.0);
    let triangle = if phase < 0.5 {
        phase * 2.0
    } else {
        (1.0 - phase) * 2.0
    };
    motion::ease_bezier(motion::EASE_ORBITAL, triangle)
}

/// A triangle ping-pong (0→1→0) over `phase` — back-and-forth sweeps
/// (searching's scan, the processing beam) never wrap-jump.
pub(crate) fn ping_pong(phase: f32) -> f32 {
    let phase = phase.rem_euclid(1.0);
    if phase < 0.5 {
        phase * 2.0
    } else {
        (1.0 - phase) * 2.0
    }
}

/// The welcome chips' stagger step (REVAMP_SPEC §1/§4: `--dur-instant`).
pub(crate) const STAGGER_S: f32 = motion::DUR_INSTANT_MS as f32 / 1000.0;
/// The welcome chips' per-chip bloom run (REVAMP_SPEC §4: `--dur-base`).
pub(crate) const BLOOM_S: f32 = motion::DUR_BASE_MS as f32 / 1000.0;

/// One welcome chip's §7.3 `bloom` progress (eased, 0..1) at frame-clock
/// `now`: chip `step` starts `step` × `--dur-instant` after `anchor` and
/// blooms over `--dur-base` on `--ease-precise` (alpha 0→1, scale
/// 0.96→1.0 are the caller's lerp of this value). A one-shot, so §7.4
/// keeps it under reduce-motion.
pub(crate) fn bloom_progress(now: f64, anchor: f64, step: usize) -> f32 {
    let t = ((now - anchor) as f32 - step as f32 * STAGGER_S) / BLOOM_S;
    motion::ease_bezier(motion::EASE_PRECISE, t.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_eased_is_symmetric_and_bounded() {
        assert!(pulse_eased(0.0).abs() < 1e-6);
        assert!((pulse_eased(0.5) - 1.0).abs() < 1e-6);
        assert!((pulse_eased(1.0)).abs() < 1e-6);
        // The loop wraps seamlessly.
        assert!((pulse_eased(1.25) - pulse_eased(0.25)).abs() < 1e-6);
        for step in 0..=16 {
            let v = pulse_eased(step as f32 / 16.0);
            assert!((0.0..=1.0).contains(&v));
        }
    }

    #[test]
    fn ping_pong_never_wrap_jumps() {
        assert_eq!(ping_pong(0.0), 0.0);
        assert_eq!(ping_pong(0.5), 1.0);
        assert_eq!(ping_pong(1.0), 0.0);
        assert!((ping_pong(0.99) - 0.02).abs() < 1e-6);
        assert!((ping_pong(-0.25) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn with_alpha_scales_and_clamps() {
        let c = with_alpha(crate::theme::color::FG_MUTED, 0.5);
        assert_eq!(c.a(), 128);
        assert_eq!(with_alpha(crate::theme::color::FG_MUTED, 2.0).a(), 255);
        assert_eq!(with_alpha(crate::theme::color::FG_MUTED, -1.0).a(), 0);
    }

    #[test]
    fn the_welcome_bloom_staggers_each_chip_by_dur_instant() {
        let anchor = 10.0;
        // 40ms in: chip 0 mid-bloom, chip 2 has not started (the §9
        // welcome_chips_bloom_in_three_staggered_steps contract).
        let mid = bloom_progress(anchor + 0.04, anchor, 0);
        assert!(mid > 0.0 && mid < 1.0, "chip 0 mid-bloom: {mid}");
        assert!(
            bloom_progress(anchor + 0.04, anchor, 2) < 1e-6,
            "chip 2 still dark"
        );
        // Each chip starts one --dur-instant after the previous.
        assert!(bloom_progress(anchor + STAGGER_S as f64 - 0.01, anchor, 1) < 1e-6);
        assert!(bloom_progress(anchor + STAGGER_S as f64 + 0.01, anchor, 1) > 0.0);
        // Every bloom completes --dur-base after its own start.
        for step in 0..3 {
            let done = anchor + (step as f32 * STAGGER_S + BLOOM_S) as f64 + 0.01;
            assert!(
                bloom_progress(done, anchor, step) > 1.0 - 1e-4,
                "chip {step} bloomed"
            );
        }
        // Before the anchor: nothing.
        assert!(bloom_progress(anchor - 0.5, anchor, 0) < 1e-6);
    }

    /// The 60fps budget: 100 live presence widgets (orbs across all nine
    /// states, avatars, glows, melting pills) paint in one headless frame
    /// in well under 16ms. Paint-only, zero per-frame allocation: this is
    /// the discipline's proof.
    #[test]
    fn a_hundred_widgets_paint_within_the_frame_budget() {
        use egui::{pos2, vec2, Rect};

        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0));
        let orb_states = [
            orb::OrbState::Breathing,
            orb::OrbState::Working,
            orb::OrbState::Searching,
            orb::OrbState::Solving,
            orb::OrbState::Listening,
            orb::OrbState::Connecting,
            orb::OrbState::Weaving,
            orb::OrbState::Composing,
            orb::OrbState::Shaping,
        ];

        // The widget set is built once (the widgets are value types; the
        // avatar/pill hold their cross-fade clocks).
        let mut avatars: Vec<avatar::AgentAvatar> = (0..30)
            .map(|i| {
                let mut a = avatar::AgentAvatar::new(i as f32 / 30.0);
                a.set_state(avatar::AvatarState::Working, -1.0);
                a
            })
            .collect();
        let mut glows: Vec<stream_glow::StreamGlow> = (0..15)
            .map(|_| {
                let mut g = stream_glow::StreamGlow::new();
                for _ in 0..6 {
                    g.note_delta();
                }
                g
            })
            .collect();
        let mut pills: Vec<liquid::LiquidPill> = (0..15)
            .map(|_| {
                let mut p = liquid::LiquidPill::default();
                p.set_on(false, -1.0);
                p.set_on(true, 0.0); // mid-melt during the timed frame
                p
            })
            .collect();

        let paint = |avatars: &mut [avatar::AgentAvatar],
                     glows: &mut [stream_glow::StreamGlow],
                     pills: &mut [liquid::LiquidPill],
                     time: f64| {
            let input = egui::RawInput {
                focused: true,
                screen_rect: Some(screen),
                time: Some(time),
                ..Default::default()
            };
            let started = std::time::Instant::now();
            let mut out = ctx.run_ui(input, |ui| {
                for i in 0..40 {
                    let x = 8.0 + (i % 10) as f32 * 28.0;
                    let y = 8.0 + (i / 10) as f32 * 28.0;
                    let rect = Rect::from_min_size(pos2(x, y), vec2(20.0, 20.0));
                    orb::ThinkingOrb::new(orb_states[i % 9], orb::ORB_INLINE).paint(ui, rect);
                }
                for (i, avatar) in avatars.iter_mut().enumerate() {
                    let x = 8.0 + (i % 10) as f32 * 28.0;
                    let y = 130.0 + (i / 10) as f32 * 28.0;
                    avatar.paint(
                        ui,
                        Rect::from_min_size(pos2(x, y), vec2(20.0, 20.0)),
                        1.0,
                        true,
                    );
                }
                for (i, glow) in glows.iter_mut().enumerate() {
                    let x = 8.0 + (i % 5) as f32 * 240.0;
                    let y = 240.0 + (i / 5) as f32 * 40.0;
                    glow.show(
                        ui,
                        Rect::from_min_size(pos2(x, y), vec2(220.0, 36.0)),
                        i % 2 == 0,
                    );
                }
                for (i, pill) in pills.iter_mut().enumerate() {
                    let x = 240.0 + (i % 5) as f32 * 240.0;
                    let y = 380.0 + (i / 5) as f32 * 36.0;
                    pill.show(ui, pos2(x, y), "agent");
                }
            });
            out.textures_delta.clear();
            started.elapsed()
        };

        paint(&mut avatars, &mut glows, &mut pills, 0.0); // warmup (fonts, galley cache)
        let elapsed = paint(&mut avatars, &mut glows, &mut pills, 0.13);
        assert!(
            elapsed < std::time::Duration::from_millis(16),
            "100 live widgets painted in {elapsed:?} (budget: 16ms)"
        );
    }
}
