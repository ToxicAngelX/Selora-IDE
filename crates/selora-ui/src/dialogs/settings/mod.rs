//! The Settings dialog (UI_SPEC.md §6).
//!
//! A centered 720×520 modal (`--ink-mid`, 1px `--ink-hi` border,
//! `--radius-2`, no shadow) with a 200px section list on the left and a
//! scrollable field pane on the right. Saving is implicit on field blur —
//! every field change writes via `Settings::save_to` (atomic) and emits a
//! [`SettingsEvent`] the shell turns into a "Settings saved" toast. There
//! are no Apply/Cancel buttons. Esc closes (emits [`SettingsEvent::Close`]).
//!
//! The Selora section's Connect flow is live as of Phase 4a
//! (`selora_section.rs`): the "Connect" button emits
//! [`SettingsEvent::Connect`] carrying the pasted key — the *shell*
//! validates it with `SeloraClient::list_models` and writes the OS keyring
//! (ARCHITECTURE.md §7.2); the dialog is a widget and never touches the
//! network or the keyring itself. "Disconnect" emits
//! [`SettingsEvent::Disconnect`]. The shell hands the live connection
//! state back with [`SettingsDialog::set_selora_status`] whenever it
//! changes (or once per frame, right before `show`) — the status line
//! paints exactly the state it is handed, including the gateway's own
//! error words, verbatim.
//!
//! Every color, font, and spacing value comes from `selora-tokens`
//! (DESIGN_TOKENS.md §12: the no-decoration rule).

use std::path::PathBuf;

use selora_ai::SeloraStatus;
use selora_settings::{find_on_path, Keybind, Settings, SETTINGS_FILE_NAME};
use selora_tokens::{color, font, radius, space, type_scale};

/// The Selora section's pane — Phase 4a's file (`selora_section.rs`).
mod selora_section;

/// Modal size (UI_SPEC.md §6): 720×520.
const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 520.0;

/// Left section-list column (UI_SPEC.md §6): 200px.
const SIDEBAR_WIDTH: f32 = 200.0;

/// Section-list row height — plain stacked labels, 28px rows (7× the 4px
/// grid; comfortable for `--type-md`).
const ROW_HEIGHT: f32 = 28.0;

/// Field-label typography: `--type-sm --fg-muted` above the control
/// (UI_SPEC.md §6).
fn label_style(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .color(color::FG_MUTED)
            .font(egui::FontId::new(type_scale::SM, font::FAMILY_SANS)),
    );
}

/// A muted inline note (e.g. "coming in Phase 3"), `--type-sm --fg-muted`.
fn note_style(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .color(color::FG_MUTED)
            .font(egui::FontId::new(type_scale::SM, font::FAMILY_SANS)),
    );
}

/// A read-only value: `--type-md` mono, `--fg-default`.
fn value_style(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .color(color::FG_DEFAULT)
            .font(egui::FontId::new(type_scale::MD, font::FAMILY_MONO)),
    );
}

/// Vertical gap between fields (a "section gap": `--space-4`).
fn field_gap(ui: &mut egui::Ui) {
    ui.add_space(space::S4);
}

/// One section in the left column. General and Git are Phase 3 placeholders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    Editor,
    Ui,
    Selora,
    Lsp,
    Git,
    Keybinds,
}

impl Section {
    const ALL: [(Section, &'static str); 7] = [
        (Section::General, "General"),
        (Section::Editor, "Editor"),
        (Section::Ui, "UI"),
        (Section::Selora, "Selora"),
        (Section::Lsp, "LSP"),
        (Section::Git, "Git"),
        (Section::Keybinds, "Keybinds"),
    ];
}

/// The keybind fields of `settings.json` (ARCHITECTURE.md §8.1) that are
/// rebindable in the Keybinds section (UI_SPEC.md §15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeybindField {
    CommandPalette,
    FuzzyFind,
    ToggleChat,
    Save,
}

impl KeybindField {
    const ALL: [(KeybindField, &'static str); 4] = [
        (KeybindField::CommandPalette, "Command palette"),
        (KeybindField::FuzzyFind, "Fuzzy find"),
        (KeybindField::ToggleChat, "Toggle chat"),
        (KeybindField::Save, "Save"),
    ];

    fn default_binding(self) -> &'static str {
        match self {
            KeybindField::CommandPalette => "Ctrl+Shift+P",
            KeybindField::FuzzyFind => "Ctrl+P",
            KeybindField::ToggleChat => "Ctrl+L",
            KeybindField::Save => "Ctrl+S",
        }
    }

    fn get(self, settings: &Settings) -> String {
        match self {
            KeybindField::CommandPalette => settings.keybinds.command_palette.clone(),
            KeybindField::FuzzyFind => settings.keybinds.fuzzy_find.clone(),
            KeybindField::ToggleChat => settings.keybinds.toggle_chat.clone(),
            KeybindField::Save => settings.keybinds.save.clone(),
        }
    }

    fn set(self, settings: &mut Settings, binding: String) {
        match self {
            KeybindField::CommandPalette => settings.keybinds.command_palette = binding,
            KeybindField::FuzzyFind => settings.keybinds.fuzzy_find = binding,
            KeybindField::ToggleChat => settings.keybinds.toggle_chat = binding,
            KeybindField::Save => settings.keybinds.save = binding,
        }
    }
}

/// Events the dialog emits to the shell. The dialog never toasts directly.
///
/// `Debug` is implemented by hand: the key-carrying variant redacts its
/// payload so the `sk-gw-…` key can never leak into logs, `dbg!` output,
/// or test-failure messages.
#[derive(Clone, PartialEq, Eq)]
pub enum SettingsEvent {
    /// Esc was pressed — close the modal.
    Close,
    /// Settings were written to disk; the shell toasts "Settings saved".
    Saved,
    /// Writing settings failed; the shell toasts the error message.
    SaveFailed(String),
    /// The user pressed "Connect" with a key in the masked input. The raw
    /// `sk-gw-…` key is carried here and nowhere else: the shell validates
    /// it with `SeloraClient::list_models` and writes it to the OS keyring
    /// (ARCHITECTURE.md §7.2). Replaces the Phase 2f `SaveKeyring`.
    Connect(String),
    /// The user pressed "Disconnect". The shell deletes the keyring entry
    /// and sets the connection state back to `Disconnected`.
    Disconnect,
}

impl std::fmt::Debug for SettingsEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Close => f.write_str("Close"),
            Self::Saved => f.write_str("Saved"),
            Self::SaveFailed(message) => f.debug_tuple("SaveFailed").field(message).finish(),
            // The key is a secret: never in Debug output.
            Self::Connect(_) => f.debug_tuple("Connect").field(&"[REDACTED]").finish(),
            Self::Disconnect => f.write_str("Disconnect"),
        }
    }
}

/// The Settings dialog. Owns a working copy of [`Settings`] and a snapshot
/// of the last state written to disk; a field blur that changed something
/// triggers a save (UI_SPEC.md §6: implicit save, no Apply/Cancel).
pub struct SettingsDialog {
    /// The working copy the fields edit.
    pub settings: Settings,
    /// The last state written to disk (implicit-save change detection).
    snapshot: Settings,
    section: Section,
    /// Where to write. `None` = the real `%APPDATA%\selora-ide` (via
    /// `Settings::default_path`). The override exists so tests (and
    /// sandboxed shells) never touch the user's real settings.
    save_dir: Option<PathBuf>,
    events: Vec<SettingsEvent>,
    /// The masked "Connect with API key" buffer. Never persisted, never
    /// written to a file, never in `Debug` output — the OS keyring (via
    /// the shell) is the key's only home. Cleared the moment the key is
    /// emitted in a [`SettingsEvent::Connect`] event.
    api_key: String,
    /// The live Selora connection state, handed in by the shell via
    /// [`SettingsDialog::set_selora_status`]. Painted verbatim by the
    /// Selora section's status line.
    selora_status: SeloraStatus,
    /// Which keybind row (if any) is waiting for the user to press a key.
    keybind_capture: Option<KeybindField>,
    /// Inline result note for the rust-analyzer "Auto-detect" button.
    rust_detect_note: String,
    /// Inline result note for the typescript "Auto-detect" button.
    ts_detect_note: String,
}

impl SettingsDialog {
    /// Create the dialog over the given settings. Saves go to the real
    /// settings path.
    pub fn new(settings: Settings) -> Self {
        Self::with_save_dir(settings, None)
    }

    /// Create the dialog with an explicit settings directory (tests and
    /// sandboxed shells; `None` keeps the real path).
    pub fn with_save_dir(settings: Settings, dir: Option<PathBuf>) -> Self {
        let snapshot = settings.clone();
        Self {
            settings,
            snapshot,
            section: Section::Editor,
            save_dir: dir,
            events: Vec::new(),
            api_key: String::new(),
            selora_status: SeloraStatus::Disconnected,
            keybind_capture: None,
            rust_detect_note: String::new(),
            ts_detect_note: String::new(),
        }
    }

    /// The settings the dialog is editing.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Replace the working copy (e.g. when the shell applies a
    /// `SettingsLoad` result). Resets the change-detection snapshot so the
    /// next blur only saves real edits.
    pub fn update(&mut self, settings: Settings) {
        self.snapshot = settings.clone();
        self.settings = settings;
    }

    /// Hand the dialog the live connection state (the shell calls this
    /// whenever it changes — e.g. on a `SeloraStatusChanged` message — or
    /// once per frame right before `show`). The Selora section's status
    /// line paints exactly this state, including the gateway's own error
    /// words in [`SeloraStatus::Error`], verbatim.
    ///
    /// On a transition to `Connected` the masked key buffer is cleared:
    /// the key now lives in the OS keyring and the dialog keeps no copy.
    pub fn set_selora_status(&mut self, status: SeloraStatus) {
        if matches!(status, SeloraStatus::Connected { .. }) {
            self.api_key.clear();
        }
        self.selora_status = status;
    }

    /// Paint the modal and return the events emitted this frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Vec<SettingsEvent> {
        // Esc closes the modal (UI_SPEC.md §6) — unless a keybind row is
        // capturing, in which case Esc cancels the capture first.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.keybind_capture.is_some() {
                self.keybind_capture = None;
            } else {
                self.events.push(SettingsEvent::Close);
            }
        }

        // The modal: --ink-mid fill, 1px --ink-hi hairline, --radius-2,
        // --space-5 inner margin, no shadow (DESIGN_TOKENS.md §4–§6).
        let frame = egui::Frame::default()
            .fill(color::INK_MID)
            .stroke(egui::Stroke::new(1.0, color::INK_HI))
            .corner_radius(radius::R2)
            .inner_margin(egui::Margin::same(space::S5 as i8));

        egui::Window::new("SETTINGS")
            .id(egui::Id::new("selora-settings-dialog"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([WIDTH, HEIGHT])
            .resizable(false)
            .collapsible(false)
            .frame(frame)
            .show(ctx, |ui| self.body(ui));

        std::mem::take(&mut self.events)
    }

    // -- layout -----------------------------------------------------------

    fn body(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Left column: the 200px section list.
            ui.allocate_ui(egui::vec2(SIDEBAR_WIDTH, ui.available_height()), |ui| {
                self.sidebar(ui);
            });

            // 1px hairline divider between the columns (DESIGN_TOKENS.md §5).
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(1.0, ui.available_height()), egui::Sense::hover());
            ui.painter().rect_filled(rect, radius::R0, color::INK_HI);

            // Right pane: the fields, scrollable.
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    self.pane(ui);
                });
        });
    }

    /// The section list (UI_SPEC.md §6 wireframe: plain text labels, no
    /// nested tree). Selected row: `--fg-default` on `--ink-low`; hover:
    /// `--ink-faint`; idle text `--fg-muted` (DESIGN_TOKENS.md §10).
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        for (section, name) in Section::ALL {
            let selected = self.section == section;
            let (rect, resp) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), ROW_HEIGHT),
                egui::Sense::click(),
            );
            if selected {
                ui.painter().rect_filled(rect, radius::R0, color::INK_LOW);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, radius::R0, color::INK_FAINT);
            }
            let text_color = if selected || resp.hovered() {
                color::FG_DEFAULT
            } else {
                color::FG_MUTED
            };
            let pos = egui::pos2(rect.left() + space::S3, rect.center().y);
            ui.painter().text(
                pos,
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
                text_color,
            );
            if resp.clicked() {
                self.section = section;
                self.keybind_capture = None;
            }
        }
    }

    fn pane(&mut self, ui: &mut egui::Ui) {
        ui.add_space(space::S1);
        match self.section {
            Section::General => {
                self.pane_placeholder(ui, "General settings are coming in Phase 3.")
            }
            Section::Editor => self.pane_editor(ui),
            Section::Ui => self.pane_ui(ui),
            Section::Selora => selora_section::pane_selora(self, ui),
            Section::Lsp => self.pane_lsp(ui),
            Section::Git => self.pane_placeholder(ui, "Git settings are coming in Phase 3."),
            Section::Keybinds => self.pane_keybinds(ui),
        }
    }

    fn pane_placeholder(&mut self, ui: &mut egui::Ui, text: &str) {
        note_style(ui, text);
    }

    // -- sections ---------------------------------------------------------

    fn pane_editor(&mut self, ui: &mut egui::Ui) {
        label_style(ui, "Font Family");
        let resp = ui.text_edit_singleline(&mut self.settings.editor.font_family);
        self.commit_text_on_blur(&resp);

        field_gap(ui);
        label_style(ui, "Font Size");
        let resp = ui.add(
            egui::DragValue::new(&mut self.settings.editor.font_size)
                // Nothing below --type-xs (11px) exists in the IDE.
                .range(11..=48),
        );
        self.commit_drag(&resp);

        field_gap(ui);
        label_style(ui, "Tab Size");
        let resp = ui.add(egui::DragValue::new(&mut self.settings.editor.tab_size).range(1..=16));
        self.commit_drag(&resp);

        field_gap(ui);
        let resp = ui.checkbox(&mut self.settings.editor.line_numbers, "Show line numbers");
        self.commit_if_changed(&resp);

        let resp = ui.checkbox(&mut self.settings.editor.minimap, "Minimap");
        self.commit_if_changed(&resp);

        let resp = ui.checkbox(&mut self.settings.editor.word_wrap, "Word wrap");
        self.commit_if_changed(&resp);
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui) {
        // The theme field exists in the schema but only "selora-dark" is
        // honored in Lite (Phase 2f brief) — rendered read-only.
        label_style(ui, "Theme");
        value_style(ui, &self.settings.ui.theme.clone());
        note_style(ui, "Only selora-dark is available in this edition.");

        field_gap(ui);
        label_style(ui, "Sidebar Width");
        let resp = ui.add(
            egui::DragValue::new(&mut self.settings.ui.sidebar_width)
                // Region ① min/max (UI_SPEC.md §2).
                .range(160..=480),
        );
        self.commit_drag(&resp);

        field_gap(ui);
        label_style(ui, "Chat Sidebar Width");
        let resp = ui.add(
            egui::DragValue::new(&mut self.settings.ui.chat_sidebar_width)
                // Region ③ min/max (UI_SPEC.md §2).
                .range(240..=600),
        );
        self.commit_drag(&resp);
    }

    fn pane_lsp(&mut self, ui: &mut egui::Ui) {
        label_style(ui, "rust-analyzer path");
        ui.horizontal(|ui| {
            let resp = ui.text_edit_singleline(
                self.settings
                    .lsp
                    .rust_analyzer_path
                    .get_or_insert_with(String::new),
            );
            if self.settings.lsp.rust_analyzer_path.as_deref() == Some("") {
                self.settings.lsp.rust_analyzer_path = None;
            }
            if ui.button("Auto-detect").clicked() {
                match find_on_path("rust-analyzer") {
                    Some(found) => {
                        self.settings.lsp.rust_analyzer_path = Some(found);
                        self.rust_detect_note.clear();
                        self.save_now();
                    }
                    None => self.rust_detect_note = "rust-analyzer was not found on PATH.".into(),
                }
            }
            self.commit_text_on_blur(&resp);
        });
        if !self.rust_detect_note.is_empty() {
            note_style(ui, &self.rust_detect_note.clone());
        }

        field_gap(ui);
        label_style(ui, "typescript-language-server path");
        ui.horizontal(|ui| {
            let resp = ui.text_edit_singleline(
                self.settings
                    .lsp
                    .typescript_server_path
                    .get_or_insert_with(String::new),
            );
            if self.settings.lsp.typescript_server_path.as_deref() == Some("") {
                self.settings.lsp.typescript_server_path = None;
            }
            if ui.button("Auto-detect").clicked() {
                match find_on_path("typescript-language-server") {
                    Some(found) => {
                        self.settings.lsp.typescript_server_path = Some(found);
                        self.ts_detect_note.clear();
                        self.save_now();
                    }
                    None => {
                        self.ts_detect_note =
                            "typescript-language-server was not found on PATH.".into()
                    }
                }
            }
            self.commit_text_on_blur(&resp);
        });
        if !self.ts_detect_note.is_empty() {
            note_style(ui, &self.ts_detect_note.clone());
        }

        field_gap(ui);
        note_style(
            ui,
            "Leave empty to resolve the server from PATH at startup.",
        );
    }

    fn pane_keybinds(&mut self, ui: &mut egui::Ui) {
        note_style(ui, "Click a binding, then press the new key. Esc cancels.");
        ui.add_space(space::S2);

        // A row is capturing: take the next non-modifier key event and bind
        // it. Escape never reaches here (show() cancels the capture first).
        if let Some(field) = self.keybind_capture {
            if let Some((key, modifiers)) = pressed_key(ui) {
                let keybind = Keybind {
                    ctrl: modifiers.ctrl || modifiers.command,
                    alt: modifiers.alt,
                    shift: modifiers.shift,
                    key,
                };
                field.set(&mut self.settings, keybind.to_string());
                self.keybind_capture = None;
                self.save_now();
            }
        }

        for (field, name) in KeybindField::ALL {
            label_style(ui, name);
            ui.horizontal(|ui| {
                let capturing = self.keybind_capture == Some(field);
                let binding = if capturing {
                    "Press a key…".to_owned()
                } else {
                    field.get(&self.settings)
                };
                let button = if capturing {
                    egui::RichText::new(binding)
                        .color(color::FG_DEFAULT)
                        .font(egui::FontId::new(type_scale::MD, font::FAMILY_MONO))
                } else {
                    egui::RichText::new(binding)
                        .color(color::FG_MUTED)
                        .font(egui::FontId::new(type_scale::MD, font::FAMILY_MONO))
                };
                if ui.button(button).clicked() {
                    // Clicking a row arms it; clicking another row re-arms.
                    self.keybind_capture = Some(field);
                }
                if ui.button("Reset").clicked() {
                    field.set(&mut self.settings, field.default_binding().to_owned());
                    self.save_now();
                }
            });
            ui.add_space(space::S2);
        }
    }

    // -- implicit save ----------------------------------------------------

    /// Text fields save on blur (UI_SPEC.md §6): when the field loses focus
    /// and the settings differ from the last written snapshot.
    fn commit_text_on_blur(&mut self, resp: &egui::Response) {
        if resp.lost_focus() && self.settings != self.snapshot {
            self.save_now();
        }
    }

    /// Checkboxes save immediately.
    fn commit_if_changed(&mut self, resp: &egui::Response) {
        if resp.changed() {
            self.save_now();
        }
    }

    /// Drag values save on release (or on keyboard nudges, which never hold
    /// the pointer).
    fn commit_drag(&mut self, resp: &egui::Response) {
        let pointer_down = resp.ctx.input(|i| i.pointer.primary_down());
        if resp.drag_stopped() || (resp.changed() && !pointer_down) {
            self.save_now();
        }
    }

    /// Write the settings atomically and emit the toast event.
    fn save_now(&mut self) {
        let path = match &self.save_dir {
            Some(dir) => dir.join(SETTINGS_FILE_NAME),
            None => Settings::default_path(),
        };
        match self.settings.save_to(&path) {
            Ok(()) => {
                self.snapshot = self.settings.clone();
                self.events.push(SettingsEvent::Saved);
            }
            Err(err) => self.events.push(SettingsEvent::SaveFailed(err.to_string())),
        }
    }
}

/// The next pressed key event, skipping pure modifier presses and repeats.
fn pressed_key(ui: &egui::Ui) -> Option<(egui::Key, egui::Modifiers)> {
    ui.input(|i| {
        i.events.iter().find_map(|event| match event {
            egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if !is_pure_modifier(*key) => Some((*key, *modifiers)),
            _ => None,
        })
    })
}

/// The physical modifier keys egui emits as distinct `Key` variants; they
/// must not be bindable themselves.
fn is_pure_modifier(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::ShiftLeft
            | egui::Key::ShiftRight
            | egui::Key::ControlLeft
            | egui::Key::ControlRight
            | egui::Key::AltLeft
            | egui::Key::AltRight
            | egui::Key::SuperLeft
            | egui::Key::SuperRight
    )
}

// ---------------------------------------------------------------------------
// Tests (headless paint via `egui::__run_test_ui`).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let n = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "selora-settings-dialog-test-{}-{}",
            std::process::id(),
            n
        ))
    }

    #[test]
    fn dialog_paints_every_section_without_events() {
        let dir = temp_dir();
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir.clone()));

        // Two passes per section so layout settles and any lazy state
        // (focus, scroll) round-trips.
        for section in [
            Section::General,
            Section::Editor,
            Section::Ui,
            Section::Selora,
            Section::Lsp,
            Section::Git,
            Section::Keybinds,
        ] {
            dialog.section = section;
            for _ in 0..2 {
                let mut events = Vec::new();
                egui::__run_test_ui(|ui| {
                    let ctx = ui.ctx().clone();
                    events = dialog.show(&ctx);
                });
                assert!(
                    events.is_empty(),
                    "section {section:?} painted without side effects, got {events:?}"
                );
            }
        }

        // Painting alone changes nothing → no implicit save happened.
        assert!(!dir.join(SETTINGS_FILE_NAME).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A `RawInput` with a 1280×800 screen (UI_SPEC.md §1) and one key
    /// press, for driving the dialog headlessly via `ctx.run_ui`.
    fn key_input(key: egui::Key, modifiers: egui::Modifiers) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn esc_emits_close() {
        let dir = temp_dir();
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir.clone()));

        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let raw = key_input(egui::Key::Escape, egui::Modifiers::default());
        ctx.run_ui(raw, |ui| {
            let events = dialog.show(ui.ctx());
            assert_eq!(events, vec![SettingsEvent::Close]);
        })
        .drop_without_applying_deltas();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn esc_cancels_keybind_capture_instead_of_closing() {
        let dir = temp_dir();
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir.clone()));
        dialog.section = Section::Keybinds;
        dialog.keybind_capture = Some(KeybindField::Save);

        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let raw = key_input(egui::Key::Escape, egui::Modifiers::default());
        ctx.run_ui(raw, |ui| {
            let events = dialog.show(ui.ctx());
            assert!(
                events.is_empty(),
                "Esc while capturing must not close: {events:?}"
            );
            assert_eq!(dialog.keybind_capture, None, "Esc must cancel the capture");
            // The binding was not rewritten by the Esc key.
            assert_eq!(dialog.settings.keybinds.save, "Ctrl+S");
        })
        .drop_without_applying_deltas();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keybind_capture_rebinds_on_key_press() {
        let dir = temp_dir();
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir.clone()));
        dialog.section = Section::Keybinds;
        dialog.keybind_capture = Some(KeybindField::FuzzyFind);

        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let raw = key_input(
            egui::Key::K,
            egui::Modifiers {
                alt: false,
                ctrl: true,
                shift: false,
                mac_cmd: false,
                command: true,
            },
        );
        ctx.run_ui(raw, |ui| {
            let events = dialog.show(ui.ctx());
            // The rebind saved implicitly.
            assert_eq!(events, vec![SettingsEvent::Saved]);
            assert_eq!(dialog.keybind_capture, None);
        })
        .drop_without_applying_deltas();

        // The binding was rewritten in canonical form and persisted.
        assert_eq!(dialog.settings.keybinds.fuzzy_find, "Ctrl+K");
        assert!(dir.join(SETTINGS_FILE_NAME).is_file());
        let loaded = Settings::load_from(&dir.join(SETTINGS_FILE_NAME)).expect("load");
        assert_eq!(loaded.keybinds.fuzzy_find, "Ctrl+K");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_now_writes_and_emits_saved() {
        let dir = temp_dir();
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir.clone()));
        dialog.settings.editor.font_size = 18;

        dialog.save_now();
        assert_eq!(dialog.events, vec![SettingsEvent::Saved]);

        let loaded = Settings::load_from(&dir.join(SETTINGS_FILE_NAME)).expect("load");
        assert_eq!(loaded.editor.font_size, 18);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_failure_emits_save_failed() {
        // A file where the settings directory should be — create_dir_all
        // fails, so the atomic write errors.
        let blocker = temp_dir();
        std::fs::create_dir_all(&blocker).expect("mkdir");
        let not_a_dir = blocker.join("blocker");
        std::fs::write(&not_a_dir, b"").expect("write blocker file");

        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(not_a_dir));
        dialog.settings.editor.font_size = 20;
        dialog.save_now();

        assert_eq!(dialog.events.len(), 1);
        assert!(matches!(&dialog.events[0], SettingsEvent::SaveFailed(_)));

        let _ = std::fs::remove_dir_all(blocker);
    }

    #[test]
    fn settings_event_debug_redacts_the_api_key() {
        let event = SettingsEvent::Connect("sk-gw-live-secret-key".into());
        let printed = format!("{event:?}");
        assert_eq!(printed, r#"Connect("[REDACTED]")"#);
        assert!(!printed.contains("sk-gw-live-secret-key"));
    }

    #[test]
    fn set_selora_status_updates_state_and_clears_the_key_on_connected() {
        let dir = temp_dir();
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir.clone()));
        dialog.api_key = "sk-gw-typed".into();

        dialog.set_selora_status(SeloraStatus::Connecting);
        assert_eq!(dialog.selora_status, SeloraStatus::Connecting);
        assert_eq!(dialog.api_key, "sk-gw-typed");

        // Failure keeps the buffer (the user may retry) — the key itself
        // is the shell's business now (it stayed in the keyring).
        dialog.set_selora_status(SeloraStatus::Error("HTTP 401".into()));
        assert_eq!(dialog.api_key, "sk-gw-typed");

        dialog.set_selora_status(SeloraStatus::Connected {
            model: Some("opus-5-5".into()),
            latency_ms: 142,
        });
        assert_eq!(
            dialog.selora_status,
            SeloraStatus::Connected {
                model: Some("opus-5-5".into()),
                latency_ms: 142,
            }
        );
        assert!(
            dialog.api_key.is_empty(),
            "connected dialog keeps no key copy"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
