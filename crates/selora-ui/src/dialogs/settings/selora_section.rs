//! The Settings dialog's Selora section (UI_SPEC.md §6) — Phase 4a's file.
//!
//! Phase 2f shipped the section UI-only: a disconnected status line, a
//! password-masked key input whose "Save to keyring" button emits
//! [`SettingsEvent::SaveKeyring`], and the base-url / active-model fields.
//! Phase 4a (ROADMAP.md Phase 4) expands it into the real Connect flow:
//! validate the key via `list_models`, store it with
//! `selora_settings::keyring`, reflect the live connection state, and add
//! Disconnect.
//!
//! Layout lives in the parent (`settings::mod`) — this file paints the
//! right-hand pane for `Section::Selora` only.

use selora_tokens::{color, font, type_scale};

use super::{field_gap, label_style, note_style, value_style, SettingsDialog, SettingsEvent};

/// Paint the Selora pane. Phase 2f shape: status line, "Connect with API
/// key" (UI only — the keyring is Phase 4), base URL, active model.
pub(super) fn pane_selora(dialog: &mut SettingsDialog, ui: &mut egui::Ui) {
    // Status line: Phase 2 has no live connection state, so the dialog
    // shows the disconnected form (UI_SPEC.md §6).
    ui.label(
        egui::RichText::new("○ selora disconnected")
            .color(color::FG_MUTED)
            .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS)),
    );

    field_gap(ui);
    // The "Connect with API key" inline flow (UI_SPEC.md §6): a
    // password-masked input plus a "Save to keyring" button. UI only in
    // Phase 2 — the keyring is Phase 4.
    label_style(ui, "Connect with API key");
    let resp = ui.add(
        egui::TextEdit::singleline(&mut dialog.api_key)
            .password(true)
            .hint_text("sk-gw-…"),
    );
    if resp.changed() && dialog.api_key.contains(char::is_whitespace) {
        dialog.api_key.retain(|c| !c.is_whitespace());
    }
    let clicked = ui.button("Save to keyring").clicked();
    if clicked && !dialog.api_key.is_empty() {
        let key = std::mem::take(&mut dialog.api_key);
        dialog.events.push(SettingsEvent::SaveKeyring(key));
    }
    note_style(
        ui,
        "Secure keyring storage arrives in Phase 4; the key is not stored in Phase 2.",
    );

    field_gap(ui);
    label_style(ui, "Base URL");
    let resp = ui.text_edit_singleline(&mut dialog.settings.selora.base_url);
    dialog.commit_text_on_blur(&resp);

    field_gap(ui);
    // The active model is driven by the chat model picker (Phase 4) —
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
    note_style(
        ui,
        "Set from the model picker once Selora is connected (Phase 4).",
    );
}
