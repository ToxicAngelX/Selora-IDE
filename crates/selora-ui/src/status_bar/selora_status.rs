//! The Selora connection status (UI_SPEC.md §1.2 right zone) — Phase 4f's
//! file.
//!
//! "● selora connected · <model>" or "○ selora disconnected · connect".
//! The state is [`selora_ai::SeloraStatus`] (the shell pings every 30s via
//! `SeloraClient::status` and hands the result here); the disconnected
//! form is a click target for opening the Settings dialog's Selora
//! section. On a disconnect mid-chat, in-flight chats complete — new ones
//! get the toast (the shell's call, via `ChatError`).
//!
//! Pure widget: paints the state it is handed, emits a click event. No
//! network, no timers — the ping loop lives in the shell's frame pipeline.
//!
//! Per-frame API for the shell:
//!
//! ```ignore
//! let events = selora_status::show(ui, &self.state.selora, active_model);
//! // ui: the status-bar panel's Ui — the widget paints into its
//! // max_rect(), right-aligned 12px from the right edge (the exact spot
//! // shell.rs paints "○ selora disconnected" today).
//! // active_model: Option<&str> — the model of the chat sidebar's model
//! // picker, preferred over SeloraStatus::Connected::model (the ping
//! // itself does not know the active model).
//! ```
//!
//! Handle [`StatusEvent::OpenConnect`] by opening the Settings dialog's
//! Selora section.

use egui::{Id, Sense, Ui};
use selora_ai::SeloraStatus;

use crate::presence::orb::{OrbState, ThinkingOrb, ORB_INLINE};
use crate::theme::{color, font, radius, space, type_scale};

/// The run's id — one interactive element per frame, so a plain constant
/// (the LSP-status restart targets in shell.rs use the same scheme).
const ID: &str = "selora_connection_status";

/// The clickable run's hover-target vertical pad (REVAMP_SPEC.md §7): the
/// galley bounds expanded by (`--space-1`, 2px), `--radius-1`,
/// `--ink-faint` fill + `--fg-default` text on hover.
const RUN_PAD_V: f32 = 2.0;

/// The dot glyph of the connected run (UI_SPEC.md §1.2).
const DOT_CONNECTED: &str = "●";
/// The dot glyph of the disconnected / error runs.
const DOT_OFF: &str = "○";
/// The glyph of the connecting run (a ping is in flight).
const DOT_CONNECTING: &str = "…";

/// The longest gateway error words painted before eliding — the status bar
/// is one 24px line, not an error surface (toasts carry the full text).
const ERROR_MAX_CHARS: usize = 64;

/// The width the activity orb zone takes (the orb + its gap) — the
/// shell clears this much more room for the cost meter when an activity
/// is live (Phase R4).
pub const ACTIVITY_ZONE: f32 = ORB_INLINE + space::S2;

/// What the shell must do in response to a click on the status run.
///
/// Only the disconnected form is clickable; the click opens the Settings
/// dialog's Selora section (4a's dialog, `Section::Selora`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEvent {
    /// "○ selora disconnected · connect" was clicked → open Settings →
    /// Selora.
    OpenConnect,
}

/// The exact string the run paints for `status` (exposed for tests and
/// for the shell's accessible label / status reporting).
///
/// - Connected with a model (the `active_model` override wins over the
///   status's own): `"● selora connected · <model>"`, else
///   `"● selora connected"`.
/// - Disconnected: `"○ selora disconnected · connect"`.
/// - Connecting: `"… selora connecting"`.
/// - Error: `"○ selora error · <the gateway's own words>"`, elided at 64
///   characters.
pub fn label(status: &SeloraStatus, active_model: Option<&str>) -> String {
    match status {
        SeloraStatus::Connected { model, .. } => {
            // The shell's override (the chat picker's model) knows better
            // than the ping, which cannot see the active model.
            let model = active_model.or(model.as_deref());
            match model {
                Some(model) => format!("{DOT_CONNECTED} selora connected · {model}"),
                None => format!("{DOT_CONNECTED} selora connected"),
            }
        }
        SeloraStatus::Disconnected => format!("{DOT_OFF} selora disconnected · connect"),
        SeloraStatus::Connecting => format!("{DOT_CONNECTING} selora connecting"),
        SeloraStatus::Error(message) => {
            format!(
                "{DOT_OFF} selora error · {}",
                elide(message, ERROR_MAX_CHARS)
            )
        }
    }
}

/// The run's measured width (the orb's anchor).
fn galley_zone(painter: &egui::Painter, text: &str, font: &egui::FontId) -> f32 {
    painter
        .layout_no_wrap(text.to_owned(), font.clone(), color::FG_MUTED)
        .size()
        .x
}

/// Elide `text` to at most `max` characters (on a char boundary), ending
/// in `…` when something was cut.
fn elide(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// Paint the connection status into `ui`'s right edge (UI_SPEC.md §1.2
/// right zone) and return this frame's events.
///
/// The widget paints into `ui.max_rect()` — right-aligned, 12px
/// (`--space-3`) from the right edge, vertically centered — exactly where
/// `shell.rs` paints its hardcoded "○ selora disconnected" today, so the
/// shell swaps that block for this call inside the same panel closure.
/// It consumes no layout space.
///
/// Colors (all from `selora-tokens`): the connected run is `--ok`
/// (UI_SPEC §1.2), disconnected and connecting are `--fg-muted` with the
/// "connect" word overlaid in `--fg-default` (the click affordance), the
/// error run's prefix is `--err` and the gateway's words stay
/// `--fg-muted`. Typography: `--type-xs` sans, the status bar's right-zone
/// run.
/// `activity` (Phase R4) paints the ThinkingOrb's reserved arms left of
/// the connection run: `Weaving` while a ghost-text completion is in
/// flight, `Composing` while a workspace search runs, `Shaping` while
/// the file tree rebuilds — the Phase 6h states' documented surfaces,
/// wired at last.
pub fn show(
    ui: &mut Ui,
    status: &SeloraStatus,
    active_model: Option<&str>,
    activity: Option<OrbState>,
    reduce_motion: bool,
) -> Vec<StatusEvent> {
    let rect = ui.max_rect();
    let painter = ui.painter_at(rect);
    let run_font = egui::FontId::new(type_scale::XS, font::FAMILY_SANS);
    let right = (rect.right() - space::S3, rect.center().y);
    let text = label(status, active_model);

    // The reserved arms' orb, left of the run.
    if let Some(orb_state) = activity {
        let orb_rect = egui::Rect::from_min_size(
            egui::pos2(
                right.0 - galley_zone(&painter, &text, &run_font) - ACTIVITY_ZONE,
                rect.center().y - ORB_INLINE / 2.0,
            ),
            egui::vec2(ORB_INLINE, ORB_INLINE),
        );
        ThinkingOrb::new(orb_state, ORB_INLINE)
            .reduce_motion(reduce_motion)
            .paint(ui, orb_rect);
    }

    // The full run laid out once with a PLACEHOLDER color, so the
    // per-state color is applied at paint time (`Painter::galley` treats
    // it as a fallback — the editor tabs' pattern); the overlays below
    // reuse offsets measured with the same font, the same technique as
    // the shell's --warn dirty-dot overlay.
    let galley = painter.layout_no_wrap(text.clone(), run_font.clone(), egui::Color32::PLACEHOLDER);
    let run_size = galley.size();
    let run_pos = egui::pos2(right.0 - run_size.x, right.1 - run_size.y / 2.0);
    let run_rect = egui::Rect::from_min_size(
        egui::pos2(right.0 - run_size.x, rect.top()),
        egui::vec2(run_size.x, rect.height()),
    );

    match status {
        SeloraStatus::Connected { latency_ms, .. } => {
            // Animated radar ripple ring around the connected dot
            if !reduce_motion {
                let time = ui.input(|i| i.time);
                let cycle = (time * 1.5).fract() as f32;
                let dot_pos = egui::pos2(run_pos.x + 3.5, right.1);
                let ring_r = 3.0 + cycle * 6.5;
                let ring_alpha = ((1.0 - cycle) * 110.0) as u8;
                painter.circle_stroke(
                    dot_pos,
                    ring_r,
                    egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_unmultiplied(34, 197, 94, ring_alpha),
                    ),
                );
                ui.ctx().request_repaint();
            }

            // One --ok run (UI_SPEC.md §1.2); not a click target. The ping
            // latency rides along as the hover tooltip.
            painter.galley(run_pos, galley, color::OK);
            let response = ui.interact(run_rect, Id::new(ID), Sense::hover());
            response.on_hover_text(format!("gateway latency: {latency_ms} ms"));
        }
        SeloraStatus::Disconnected => {
            // Muted base run with the "connect" word lifted to
            // --fg-default (the click affordance, UI_SPEC.md §1.2). The
            // defined hover target (REVAMP_SPEC.md §7): the galley bounds
            // expanded by (--space-1, 2px), --radius-1, --ink-faint fill +
            // the whole run in --fg-default on hover.
            let target = egui::Rect::from_min_size(run_pos, run_size)
                .expand2(egui::vec2(space::S1, RUN_PAD_V));
            let response = ui.interact(target, Id::new(ID), Sense::click());
            if response.hovered() {
                painter.rect_filled(target, radius::R1, color::INK_FAINT);
            }
            painter.galley(
                run_pos,
                galley,
                if response.hovered() {
                    color::FG_DEFAULT
                } else {
                    color::FG_MUTED
                },
            );
            let connect = painter.layout_no_wrap("connect".to_owned(), run_font, color::FG_DEFAULT);
            painter.galley(
                egui::pos2(right.0 - connect.size().x, right.1 - connect.size().y / 2.0),
                connect,
                color::FG_DEFAULT,
            );
            if response.clicked() {
                return vec![StatusEvent::OpenConnect];
            }
        }
        SeloraStatus::Connecting => {
            // Transient: muted, not interactive.
            painter.galley(run_pos, galley, color::FG_MUTED);
        }
        // The gateway's own words (already inside the base galley via
        // `label`) stay muted; only the "○ selora error ·" prefix runs hot
        // (--err, the one saturated use here) as an overlay — the same
        // technique as the dirty-dot overlay.
        SeloraStatus::Error(_) => {
            painter.galley(run_pos, galley, color::FG_MUTED);
            let prefix = format!("{DOT_OFF} selora error · ");
            let prefix_galley = painter.layout_no_wrap(prefix, run_font, color::ERR);
            painter.galley(
                egui::pos2(run_pos.x, right.1 - prefix_galley.size().y / 2.0),
                prefix_galley,
                color::ERR,
            );
        }
    }

    Vec::new()
}

// ---------------------------------------------------------------------------
// Tests (headless paint via `egui::__run_test_ui`; pointer clicks driven
// through `ctx.run_ui` with `RawInput` events, the panel/search.rs harness).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{vec2, Pos2, RawInput};

    #[test]
    fn label_connected_prefers_the_active_model_override() {
        let status = SeloraStatus::Connected {
            model: Some("ping-model".into()),
            latency_ms: 142,
        };
        assert_eq!(
            label(&status, Some("opus-5-5")),
            "● selora connected · opus-5-5"
        );
        assert_eq!(label(&status, None), "● selora connected · ping-model");
    }

    #[test]
    fn label_connected_without_model_drops_the_suffix() {
        let status = SeloraStatus::Connected {
            model: None,
            latency_ms: 0,
        };
        assert_eq!(
            label(&status, Some("opus-5-5")),
            "● selora connected · opus-5-5"
        );
        assert_eq!(label(&status, None), "● selora connected");
    }

    #[test]
    fn label_disconnected_connecting_and_error_use_their_exact_strings() {
        assert_eq!(
            label(&SeloraStatus::Disconnected, None),
            "○ selora disconnected · connect"
        );
        assert_eq!(
            label(&SeloraStatus::Connecting, None),
            "… selora connecting"
        );
        assert_eq!(
            label(&SeloraStatus::Error("401 unauthorized".into()), None),
            "○ selora error · 401 unauthorized"
        );
    }

    #[test]
    fn label_elides_long_gateway_errors() {
        let long = "x".repeat(200);
        let text = label(&SeloraStatus::Error(long), None);
        let message = text
            .strip_prefix("○ selora error · ")
            .expect("error label keeps its prefix");
        assert_eq!(message.chars().count(), ERROR_MAX_CHARS);
        assert!(message.ends_with('…'));
        // Short errors pass through untouched.
        assert_eq!(
            label(&SeloraStatus::Error("no route".into()), None),
            "○ selora error · no route"
        );
    }

    #[test]
    fn all_four_states_paint_headlessly_without_events() {
        let states = vec![
            SeloraStatus::Connected {
                model: Some("glm-5.3".into()),
                latency_ms: 88,
            },
            SeloraStatus::Disconnected,
            SeloraStatus::Connecting,
            SeloraStatus::Error("gateway unreachable".into()),
        ];
        for status in states {
            egui::__run_test_ui(|ui| {
                ui.set_min_size(vec2(400.0, 24.0));
                let events = show(ui, &status, Some("glm-5.3"), None, false);
                assert!(
                    events.is_empty(),
                    "{status:?} paints without side effects, got {events:?}"
                );
            });
        }
    }

    /// A click landed on the run's rect: move + press in one frame, release
    /// in the next (egui's `clicked()` fires on the release frame).
    ///
    /// The default (built-in) fonts stay installed — an empty
    /// `FontDefinitions` measures every galley at zero width, which would
    /// collapse the click target.
    fn click_run(status: &SeloraStatus) -> Vec<StatusEvent> {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 24.0));
        let screen_rect = Some(screen);

        // Frame 1: settle the layout, capture where the run landed.
        let mut spot = screen.right_center();
        ctx.run_ui(
            RawInput {
                screen_rect,
                ..Default::default()
            },
            |ui| {
                ui.set_min_size(screen.size());
                // The run hugs the right edge minus --space-3; aim a bit
                // further in to land inside the text itself.
                spot = Pos2::new(screen.right() - space::S3 - 40.0, screen.center().y);
                let _ = show(ui, status, None, Some(OrbState::Weaving), false);
            },
        )
        .drop_without_applying_deltas();

        // Frame 2: pointer moves onto the run and presses.
        ctx.run_ui(
            RawInput {
                screen_rect,
                focused: true,
                events: vec![
                    egui::Event::PointerMoved(spot),
                    egui::Event::PointerButton {
                        pos: spot,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| {
                ui.set_min_size(screen.size());
                let _ = show(ui, status, None, Some(OrbState::Weaving), false);
            },
        )
        .drop_without_applying_deltas();

        // Frame 3: release — the click registers here.
        let mut events = Vec::new();
        ctx.run_ui(
            RawInput {
                screen_rect,
                focused: true,
                events: vec![egui::Event::PointerButton {
                    pos: spot,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |ui| {
                ui.set_min_size(screen.size());
                events = show(ui, status, None, None, false);
            },
        )
        .drop_without_applying_deltas();
        events
    }

    #[test]
    fn click_on_disconnected_emits_open_connect() {
        assert_eq!(
            click_run(&SeloraStatus::Disconnected),
            vec![StatusEvent::OpenConnect]
        );
    }

    /// Phase R4: an activity paints the reserved-arm orb left of the run
    /// without touching the click behavior.
    #[test]
    fn an_activity_paints_the_orb_and_stays_silent() {
        for activity in [OrbState::Weaving, OrbState::Composing, OrbState::Shaping] {
            egui::__run_test_ui(|ui| {
                ui.set_min_size(vec2(400.0, 24.0));
                let events = show(
                    ui,
                    &SeloraStatus::Connected {
                        model: None,
                        latency_ms: 5,
                    },
                    None,
                    Some(activity),
                    false,
                );
                assert!(events.is_empty(), "{activity:?} painted silently");
            });
        }
    }

    #[test]
    fn click_on_connected_emits_nothing() {
        // The connected form is not a click target (UI_SPEC.md §1.2).
        let status = SeloraStatus::Connected {
            model: Some("opus-5-5".into()),
            latency_ms: 12,
        };
        assert!(click_run(&status).is_empty());
    }

    /// REVAMP_SPEC.md §7: the connect affordance's hover target is the
    /// run's galley bounds expanded by (`--space-1`, 2px), `--radius-1`,
    /// `--ink-faint` fill — and the run's text lifts to `--fg-default`.
    #[test]
    fn connect_run_hover_paints_the_defined_target() {
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 24.0));
        let raw = |events: Vec<egui::Event>| RawInput {
            focused: true,
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let paint = |events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(raw(events), |ui| {
                ui.set_min_size(screen.size());
                let _ = show(ui, &SeloraStatus::Disconnected, None, None, false);
            });
            output.textures_delta.clear();
            output
        };

        // Frame 1: settle; recover the run's galley bounds from its text
        // shape.
        let output = paint(vec![]);
        let galley_rect = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text().contains("disconnected") => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .expect("the disconnected run paints its galley");

        // Frame 2: the pointer parked on the run paints the hover target.
        let output = paint(vec![egui::Event::PointerMoved(galley_rect.center())]);
        let target = galley_rect.expand2(egui::vec2(space::S1, RUN_PAD_V));
        let hover_fill = output.shapes.iter().any(|clipped| {
            matches!(
                &clipped.shape,
                egui::Shape::Rect(shape)
                    if shape.fill == color::INK_FAINT
                        && (shape.rect.left() - target.left()).abs() < 0.5
                        && (shape.rect.right() - target.right()).abs() < 0.5
                        && (shape.rect.top() - target.top()).abs() < 0.5
                        && (shape.rect.bottom() - target.bottom()).abs() < 0.5
            )
        });
        assert!(
            hover_fill,
            "hover paints --ink-faint at the galley bounds + (--space-1, 2px)"
        );
        let lifted = output.shapes.iter().any(|clipped| {
            matches!(
                &clipped.shape,
                egui::Shape::Text(text)
                    if text.galley.text().contains("disconnected")
                        && text.fallback_color == color::FG_DEFAULT
            )
        });
        assert!(lifted, "the hovered run's text lifts to --fg-default");
    }

    #[test]
    fn status_event_is_copy_and_debug() {
        // The shell matches on it and logs it; both must compile.
        let event = StatusEvent::OpenConnect;
        let copy = event;
        assert_eq!(event, copy);
        assert_eq!(format!("{event:?}"), "OpenConnect");
    }
}
