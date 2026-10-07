//! Toast notifications (UI_SPEC.md §13): bottom-right above the status
//! bar, `--ink-mid` fill, 1px `--ink-hi` hairline, radius-1, 12px padding,
//! max 3, info dismisses after 4s / error after 8s. Phase R0: the
//! `Toast`/`Severity` types and the stack painter moved from
//! `selora-app`'s `app_state.rs`/`shell.rs` — the *state* (the `Vec`, the
//! expiry clock) still lives in `AppState`; this module is the
//! presentation.
//!
//! One deliberate addition over the Phase 2 inline paint: the info glyph
//! is a real `--info` dot (`i`) rather than a bare letter — same token
//! family, same geometry, no §12 bend.

use std::time::{Duration, Instant};

use egui::Context;

use crate::theme::{color, font, radius, space, type_scale};

/// Toast severity (UI_SPEC.md §13): info dismisses after 4s, error after 8s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Error,
}

impl Severity {
    /// How long the toast stays on screen.
    pub fn lifetime(self) -> Duration {
        match self {
            Self::Info => Duration::from_secs(4),
            Self::Error => Duration::from_secs(8),
        }
    }
}

/// One toast notification (UI_SPEC.md §13).
#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub severity: Severity,
    pub expires_at: Instant,
    /// An optional action link, painted as a trailing hyperlink (Phase 6b:
    /// the update-available toast's "download"). eframe opens it in the
    /// system browser.
    pub link: Option<(String, String)>,
}

/// Toast anchoring (UI_SPEC.md §13): 12px from each edge, above the 24px
/// status bar.
const TOAST_EDGE: f32 = 12.0;
/// The status bar the stack clears (UI_SPEC.md §1.2: 24px).
const STATUS_BAR_HEIGHT: f32 = 24.0;
/// The stack's maximum width (long messages wrap instead of stretching).
const MAX_WIDTH: f32 = 420.0;

/// Paint the toast stack (bottom-right, above the status bar) and remove
/// any toast whose `×` was clicked. The caller owns the `Vec` — expiry is
/// its clock ([`show`] never reads or mutates `expires_at`).
pub fn show(ctx: &Context, toasts: &mut Vec<Toast>) {
    if toasts.is_empty() {
        return;
    }

    let mut dismissed = None;
    egui::Area::new(egui::Id::new("selora_toasts"))
        .order(egui::Order::Foreground)
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::vec2(-TOAST_EDGE, -(TOAST_EDGE + STATUS_BAR_HEIGHT)),
        )
        .show(ctx, |ui| {
            ui.set_max_width(MAX_WIDTH);
            ui.spacing_mut().item_spacing.y = space::S2;
            for (index, toast) in toasts.iter().enumerate() {
                let (glyph, color) = match toast.severity {
                    Severity::Info => ("i", color::INFO),
                    Severity::Error => ("!", color::ERR),
                };
                egui::Frame::default()
                    .fill(color::INK_MID)
                    .stroke(egui::Stroke::new(1.0, color::INK_HI))
                    .corner_radius(radius::R1 as u8)
                    .inner_margin(egui::Margin::same(space::S3 as i8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(glyph)
                                    .color(color)
                                    .font(egui::FontId::new(type_scale::XS, font::FAMILY_MONO)),
                            );
                            ui.label(
                                egui::RichText::new(toast.text.as_str())
                                    .color(color::FG_DEFAULT)
                                    .font(egui::FontId::new(type_scale::SM, font::FAMILY_SANS)),
                            );
                            // The optional action link (Phase 6b: "download"
                            // on the update-available toast).
                            if let Some((label, url)) = &toast.link {
                                ui.hyperlink_to(
                                    egui::RichText::new(label.as_str())
                                        .color(color::FG_MUTED)
                                        .font(egui::FontId::new(type_scale::SM, font::FAMILY_SANS)),
                                    url.as_str(),
                                );
                            }
                            if ui
                                .button(egui::RichText::new("×").color(color::FG_FAINT))
                                .clicked()
                            {
                                dismissed = Some(index);
                            }
                        });
                    });
            }
        });
    if let Some(index) = dismissed {
        toasts.remove(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        }
    }

    /// One headless frame with its texture deltas handled.
    fn run(ctx: &egui::Context, raw: egui::RawInput, toasts: &mut Vec<Toast>) {
        let mut output = ctx.run_ui(raw, |ui| show(ui.ctx(), toasts));
        output.textures_delta.clear();
    }

    #[test]
    fn severity_lifetimes_match_the_spec() {
        assert_eq!(Severity::Info.lifetime(), Duration::from_secs(4));
        assert_eq!(Severity::Error.lifetime(), Duration::from_secs(8));
    }

    /// The stack paints headlessly and the `×` dismisses exactly the
    /// clicked toast (the app_state tests previously covered the cap and
    /// expiry; this covers the paint+dismiss the shell did inline).
    #[test]
    fn clicking_the_close_glyph_dismisses_that_toast() {
        let ctx = egui::Context::default();
        let now = Instant::now();
        let mut toasts = vec![
            Toast {
                text: "first".into(),
                severity: Severity::Info,
                expires_at: now + Duration::from_secs(4),
                link: None,
            },
            Toast {
                text: "second".into(),
                severity: Severity::Error,
                expires_at: now + Duration::from_secs(8),
                link: None,
            },
        ];
        // Two settle frames so the stack's layout is stable.
        for _ in 0..2 {
            run(&ctx, screen(), &mut toasts);
        }
        assert_eq!(toasts.len(), 2, "both toasts paint");

        // The stack anchors 12px off the bottom-right, above the 24px
        // status bar; the `×` closes each row's right end. Sweep a small
        // grid around it with the four-frame click pattern (move, move,
        // press, release) — the first hit must dismiss exactly one toast.
        let anchor = egui::pos2(1280.0 - TOAST_EDGE, 800.0 - TOAST_EDGE - STATUS_BAR_HEIGHT);
        'probe: for dx in (10..=40).step_by(6) {
            for dy in (6..=64).step_by(6) {
                let pos = egui::pos2(anchor.x - dx as f32, anchor.y - dy as f32);
                let frames = [
                    vec![egui::Event::PointerMoved(pos)],
                    vec![egui::Event::PointerMoved(pos)],
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ],
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Default::default(),
                        },
                    ],
                ];
                for events in frames {
                    let mut raw = screen();
                    raw.events = events;
                    run(&ctx, raw, &mut toasts);
                }
                if toasts.len() == 1 {
                    break 'probe;
                }
                assert_eq!(
                    toasts.len(),
                    2,
                    "a probe never dismisses more than one toast"
                );
            }
        }
        assert_eq!(toasts.len(), 1, "the × dismissed one toast");
        assert_eq!(
            toasts[0].text, "first",
            "the surviving toast is the other one"
        );
    }
}
