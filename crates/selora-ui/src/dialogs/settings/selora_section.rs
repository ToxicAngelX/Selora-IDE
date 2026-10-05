//! The Settings dialog's Selora section (UI_SPEC.md §6) — Phase 4a's file.
//!
//! The real Connect flow (ROADMAP.md Phase 4): a password-masked key
//! input, a "Connect" button that emits [`SettingsEvent::Connect`] with
//! the pasted key (the *shell* validates it via `SeloraClient::list_models`
//! and writes the OS keyring — ARCHITECTURE.md §7.2; this dialog is a
//! widget and never touches the network or the keyring), a status line
//! that paints the live [`SeloraStatus`] the shell hands it verbatim
//! (including the gateway's own error words — never paraphrased,
//! PLAN.md §8), and a "Disconnect" button that emits
//! [`SettingsEvent::Disconnect`].
//!
//! Layout lives in the parent (`settings::mod`) — this file paints the
//! right-hand pane for `Section::Selora` only. Every color comes from
//! `selora-tokens`; the only saturated uses are `--ok` (connected) and
//! `--err` (error), both semantic (DESIGN_TOKENS.md §12).

use selora_ai::SeloraStatus;
use selora_tokens::{color, font, radius, space, type_scale};

use super::{field_gap, label_style, note_style, value_style, SettingsDialog, SettingsEvent};

/// Absolute widget id of the "Connect" button. The buttons are
/// hand-painted with `Ui::interact` and explicit ids (the repo's
/// git-panel button idiom) so the headless tests can find and click them
/// via `Context::read_response`.
const CONNECT_ID: &str = "selora-settings-connect";

/// Absolute widget id of the "Disconnect" button.
const DISCONNECT_ID: &str = "selora-settings-disconnect";

/// Button height — the settings row height (comfortable on the 4px grid).
const BUTTON_HEIGHT: f32 = 28.0;

/// The status line (UI_SPEC.md §6): the live connection state, painted
/// exactly as handed. Returns the text and its token color.
///
/// `Connected` is `--ok`, `Error` is `--err`; everything else is muted —
/// the only saturated colors in this pane, both semantic.
pub(super) fn status_line(status: &SeloraStatus) -> (String, egui::Color32) {
    match status {
        SeloraStatus::Disconnected => ("○ selora disconnected".to_owned(), color::FG_MUTED),
        // In-flight validation feedback (the Phase 4a brief: "connecting…"
        // while validation runs, the gateway's own words on completion).
        SeloraStatus::Connecting => ("○ selora connecting…".to_owned(), color::FG_MUTED),
        SeloraStatus::Connected { model, latency_ms } => (
            format!(
                "● selora connected · {} · {}ms",
                model.as_deref().unwrap_or("no model"),
                latency_ms
            ),
            color::OK,
        ),
        // The message is the gateway's own error string — printed
        // verbatim, never paraphrased (PLAN.md §8).
        SeloraStatus::Error(message) => (format!("✕ selora error — {message}"), color::ERR),
    }
}

/// Paint the Selora pane: live status line, Connect flow, Base URL, and
/// the read-only Active Model.
pub(super) fn pane_selora(dialog: &mut SettingsDialog, ui: &mut egui::Ui) {
    // -- status line -------------------------------------------------------
    let (text, status_color) = status_line(&dialog.selora_status);
    ui.label(
        egui::RichText::new(text)
            .color(status_color)
            .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS)),
    );

    field_gap(ui);

    // -- connect flow (UI_SPEC.md §6: masked input + inline buttons) ------
    label_style(ui, "Connect with API key");
    let resp = ui.add(
        egui::TextEdit::singleline(&mut dialog.api_key)
            .password(true)
            .hint_text("sk-gw-…"),
    );
    if resp.changed() && dialog.api_key.contains(char::is_whitespace) {
        dialog.api_key.retain(|c| !c.is_whitespace());
    }

    // While validation is in flight the Connect button goes display-only
    // (the status line carries the "connecting…" feedback).
    let connecting = matches!(dialog.selora_status, SeloraStatus::Connecting);
    // Disconnect is offered whenever there is something to disconnect
    // (a live connection, or a stored key whose validation failed).
    let can_disconnect = matches!(
        dialog.selora_status,
        SeloraStatus::Connected { .. } | SeloraStatus::Error(_)
    );
    let key_empty = dialog.api_key.is_empty();

    let mut connect_clicked = false;
    let mut disconnect_clicked = false;
    ui.horizontal(|ui| {
        connect_clicked = paint_connect_button(ui, connecting);
        if can_disconnect {
            disconnect_clicked = paint_disconnect_button(ui);
        }
    });
    if connect_clicked && !key_empty {
        // The key leaves the dialog here and lives only in the event;
        // the buffer is emptied so no copy lingers in widget state.
        let key = std::mem::take(&mut dialog.api_key);
        dialog.events.push(SettingsEvent::Connect(key));
    }
    if disconnect_clicked {
        dialog.events.push(SettingsEvent::Disconnect);
    }
    note_style(
        ui,
        "Validated against the gateway, then stored in the OS keyring — never on disk.",
    );

    field_gap(ui);
    label_style(ui, "Base URL");
    let resp = ui.text_edit_singleline(&mut dialog.settings.selora.base_url);
    dialog.commit_text_on_blur(&resp);

    field_gap(ui);
    // The active model is driven by the chat sidebar's model picker —
    // shown read-only here.
    label_style(ui, "Active Model");
    let model = dialog
        .settings
        .selora
        .active_model
        .clone()
        .unwrap_or_default();
    if model.is_empty() {
        value_style(ui, "—");
    } else {
        value_style(ui, &model);
    }
    note_style(ui, "Set from the model picker in the chat sidebar.");
}

/// The primary "Connect" button: `--ink-mid` fill, `--radius-1`,
/// `--fg-default` label; disabled while a validation is in flight
/// (transparent fill, `--fg-disabled` label — DESIGN_TOKENS.md §10).
/// Returns whether it was clicked.
fn paint_connect_button(ui: &mut egui::Ui, connecting: bool) -> bool {
    let text_color = if connecting {
        color::FG_DISABLED
    } else {
        color::FG_DEFAULT
    };
    let galley = ui.painter().layout_no_wrap(
        "Connect".to_owned(),
        egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
        text_color,
    );
    let size = egui::vec2(galley.size().x + space::S3 * 2.0, BUTTON_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let response = ui.interact(rect, egui::Id::new(CONNECT_ID), egui::Sense::click());

    if !connecting {
        ui.painter().rect_filled(rect, radius::R1, color::INK_MID);
        if response.hovered() {
            // Focused-control border (DESIGN_TOKENS.md §5): 1px
            // --fg-default, painted inside the fill so it stays a full
            // hairline.
            ui.painter().rect_stroke(
                rect,
                radius::R1,
                egui::Stroke::new(1.0, color::FG_DEFAULT),
                egui::StrokeKind::Inside,
            );
        }
    }
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, text_color);

    !connecting && response.clicked()
}

/// The ghost "Disconnect" button (the "Deny" idiom, UI_SPEC.md §4.2):
/// no fill, `--fg-muted` label lifting to `--fg-default` on hover.
/// Returns whether it was clicked.
fn paint_disconnect_button(ui: &mut egui::Ui) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(110.0, BUTTON_HEIGHT), egui::Sense::hover());
    let response = ui.interact(rect, egui::Id::new(DISCONNECT_ID), egui::Sense::click());
    let text_color = if response.hovered() {
        color::FG_DEFAULT
    } else {
        color::FG_MUTED
    };
    let galley = ui.painter().layout_no_wrap(
        "Disconnect".to_owned(),
        egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
        text_color,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, text_color);
    response.clicked()
}

// ---------------------------------------------------------------------------
// Tests (headless paint via `ctx.run_ui`, the settings-dialog pattern:
// two settle frames, then a frame whose `RawInput` carries the pointer
// press/release at the button the previous frames registered).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs::settings::{Section, SettingsDialog};
    use selora_settings::{Settings, SETTINGS_FILE_NAME};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

    /// The local twin of the parent module's temp-dir helper (each test
    /// gets its own directory so implicit saves never collide).
    fn temp_dir() -> PathBuf {
        let n = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "selora-settings-selora-section-test-{}-{}",
            std::process::id(),
            n
        ))
    }

    /// A dialog on the Selora section, saving into a scratch directory.
    fn selora_dialog() -> SettingsDialog {
        let dir = temp_dir();
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir));
        dialog.section = Section::Selora;
        dialog
    }

    /// The screen the dialog is laid out against (UI_SPEC.md §1 default).
    fn screen() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))
    }

    /// One paint pass over the dialog. Returns the events it emitted.
    fn paint(
        dialog: &mut SettingsDialog,
        ctx: &egui::Context,
        raw: egui::RawInput,
    ) -> Vec<SettingsEvent> {
        let mut events = Vec::new();
        let mut output = ctx.run_ui(raw, |ui| {
            events = dialog.show(ui.ctx());
        });
        output.textures_delta.clear();
        events
    }

    /// Paint settle frames (window anchor, scroll state) on a fresh
    /// context with empty fonts — the established settings-dialog harness.
    /// Four passes: the anchored `egui::Window` takes two extra frames to
    /// land on its final rect (verified headlessly — the button rect is
    /// stable from the third pass on).
    fn settle(dialog: &mut SettingsDialog) -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        for _ in 0..4 {
            let events = paint(
                dialog,
                &ctx,
                egui::RawInput {
                    screen_rect: Some(screen()),
                    ..Default::default()
                },
            );
            assert!(events.is_empty(), "idle frame emitted {events:?}");
        }
        ctx
    }

    /// Click the widget registered under `id`: one pass carrying the
    /// pointer move + press, one pass carrying the release (the release
    /// pass is where egui registers the click and the dialog emits).
    fn click_id(dialog: &mut SettingsDialog, ctx: &egui::Context, id: &str) -> Vec<SettingsEvent> {
        let rect = ctx
            .read_response(egui::Id::new(id))
            .unwrap_or_else(|| panic!("{id} was not painted"))
            .rect;
        let pos = rect.center();
        let press = egui::RawInput {
            screen_rect: Some(screen()),
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        };
        let events = paint(dialog, ctx, press);
        assert!(
            events.is_empty(),
            "the press pass alone must not emit: {events:?}"
        );
        let release = egui::RawInput {
            screen_rect: Some(screen()),
            events: vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
            ..Default::default()
        };
        paint(dialog, ctx, release)
    }

    #[test]
    fn status_line_covers_all_four_states() {
        let (text, color) = status_line(&SeloraStatus::Disconnected);
        assert_eq!(text, "○ selora disconnected");
        assert_eq!(color, color::FG_MUTED);

        let (text, color) = status_line(&SeloraStatus::Connecting);
        assert_eq!(text, "○ selora connecting…");
        assert_eq!(color, color::FG_MUTED);

        let (text, color) = status_line(&SeloraStatus::Connected {
            model: Some("opus-5-5".into()),
            latency_ms: 142,
        });
        assert_eq!(text, "● selora connected · opus-5-5 · 142ms");
        assert_eq!(color, color::OK);

        let (text, _color) = status_line(&SeloraStatus::Connected {
            model: None,
            latency_ms: 0,
        });
        assert_eq!(text, "● selora connected · no model · 0ms");

        // The gateway's own error words, verbatim (PLAN.md §8).
        let (text, color) = status_line(&SeloraStatus::Error(
            "HTTP 401 Unauthorized: invalid key".into(),
        ));
        assert_eq!(text, "✕ selora error — HTTP 401 Unauthorized: invalid key");
        assert_eq!(color, color::ERR);
    }

    #[test]
    fn painting_every_status_form_emits_no_events() {
        for status in [
            SeloraStatus::Disconnected,
            SeloraStatus::Connecting,
            SeloraStatus::Connected {
                model: Some("kimi-k3".into()),
                latency_ms: 90,
            },
            SeloraStatus::Error("gateway unreachable".into()),
        ] {
            let mut dialog = selora_dialog();
            dialog.set_selora_status(status);
            let _ = settle(&mut dialog);
        }
    }

    #[test]
    fn connect_click_emits_connect_with_the_typed_key() {
        let mut dialog = selora_dialog();
        dialog.api_key = "sk-gw-test-1234".into();
        let ctx = settle(&mut dialog);

        let events = click_id(&mut dialog, &ctx, CONNECT_ID);
        assert_eq!(
            events,
            vec![SettingsEvent::Connect("sk-gw-test-1234".into())]
        );

        // The key lives only in the event — the buffer is emptied.
        assert!(dialog.api_key.is_empty());

        // No further frames emit anything (the click is not sticky).
        let events = paint(
            &mut dialog,
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen()),
                ..Default::default()
            },
        );
        assert!(events.is_empty(), "events: {events:?}");
        assert!(!dialog
            .save_dir
            .is_some_and(|dir| dir.join(SETTINGS_FILE_NAME).exists()));
    }

    #[test]
    fn connect_click_with_an_empty_key_emits_nothing() {
        let mut dialog = selora_dialog();
        assert!(dialog.api_key.is_empty());
        let ctx = settle(&mut dialog);

        let events = click_id(&mut dialog, &ctx, CONNECT_ID);
        assert!(events.is_empty(), "events: {events:?}");
    }

    #[test]
    fn connect_button_is_inert_while_connecting() {
        let mut dialog = selora_dialog();
        dialog.api_key = "sk-gw-test-1234".into();
        dialog.set_selora_status(SeloraStatus::Connecting);
        let ctx = settle(&mut dialog);

        let events = click_id(&mut dialog, &ctx, CONNECT_ID);
        assert!(events.is_empty(), "events: {events:?}");
    }

    #[test]
    fn disconnect_click_emits_disconnect() {
        let mut dialog = selora_dialog();
        dialog.set_selora_status(SeloraStatus::Connected {
            model: Some("opus-5-5".into()),
            latency_ms: 120,
        });
        let ctx = settle(&mut dialog);

        let events = click_id(&mut dialog, &ctx, DISCONNECT_ID);
        assert_eq!(events, vec![SettingsEvent::Disconnect]);
    }

    #[test]
    fn disconnect_is_offered_after_a_failed_validation() {
        let mut dialog = selora_dialog();
        dialog.set_selora_status(SeloraStatus::Error("HTTP 401".into()));
        let ctx = settle(&mut dialog);

        let events = click_id(&mut dialog, &ctx, DISCONNECT_ID);
        assert_eq!(events, vec![SettingsEvent::Disconnect]);
    }

    #[test]
    fn no_disconnect_button_while_disconnected_or_connecting() {
        for status in [SeloraStatus::Disconnected, SeloraStatus::Connecting] {
            let mut dialog = selora_dialog();
            dialog.set_selora_status(status.clone());
            let ctx = settle(&mut dialog);
            assert!(
                ctx.read_response(egui::Id::new(DISCONNECT_ID)).is_none(),
                "disconnect painted for {status:?}"
            );
        }
    }
}
