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

use crate::theme::{color, font, radius, space, type_scale};
use selora_ai::SeloraStatus;
use selora_settings::{find_on_path, Keybind, Settings, SETTINGS_FILE_NAME};

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

/// One section in the left column. The Git placeholder went in Phase R5
/// (nothing to configure — git2 reads the repository's own config); the
/// Agent section arrived with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    Editor,
    Ui,
    Selora,
    Agent,
    Lsp,
    Keybinds,
}

impl Section {
    const ALL: [(Section, &'static str); 7] = [
        (Section::General, "General"),
        (Section::Editor, "Editor"),
        (Section::Ui, "UI"),
        (Section::Selora, "Selora"),
        (Section::Agent, "Agent"),
        (Section::Lsp, "LSP"),
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
    /// The masked Brave Search key buffer (Phase R5). On commit it goes
    /// to the OS keyring and is cleared — settings.json never sees it.
    brave_key: String,
    /// Inline note for the Brave key row.
    brave_note: String,
    /// The allowlist editor's add field (Phase R5).
    allowlist_add: String,
    /// Whether one of the panes' text fields held keyboard focus this
    /// frame (re-set by every field as it paints). Read by `show`'s Esc
    /// handler — egui surrenders focus on Esc before `show` can see it.
    field_focused: bool,
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
            brave_key: String::new(),
            brave_note: String::new(),
            allowlist_add: String::new(),
            field_focused: false,
        }
    }

    /// The settings the dialog is editing.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Jump to the Selora section (the status bar's "○ selora disconnected
    /// · connect" click affordance promises the Connect form, not a
    /// generic settings page). Cancels any armed keybind capture.
    pub fn open_selora_section(&mut self) {
        self.section = Section::Selora;
        self.keybind_capture = None;
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
        // capturing (Esc cancels the capture first) or a text field held
        // keyboard focus last frame. egui's own Esc handling (Focus's
        // begin_pass) has *already* surrendered the field's focus by the
        // time this runs, so a fresh `Memory::focused()` read is always
        // `None` here — the flag is what the fields reported last frame.
        // The first Esc belongs to the field (keeping the dialog, and a
        // half-typed API key, alive); the second closes.
        let field_was_focused = self.field_focused;
        self.field_focused = false;
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.keybind_capture.is_some() {
                self.keybind_capture = None;
            } else if !field_was_focused {
                self.events.push(SettingsEvent::Close);
            }
        }

        // The modal (UI_SPEC.md §6): `egui::Modal` — Foreground order with
        // an input-blocking backdrop. A plain `egui::Window` (the old
        // paint) let every click and keystroke around the dialog fall
        // through to the workbench behind it: the file tree opened files,
        // the editor took typing, the status bar toggled — the dialog
        // looked "fully bugged". The Modal confines pointer and keyboard
        // focus to itself. Toasts still show above the scrim: they are
        // Foreground-order Areas painted after this in the frame, and egui
        // stacks same-order layers by paint order.
        //
        // The panel: --ink-mid fill, 1px --ink-hi hairline, --radius-2,
        // --space-5 inner margin, no shadow (DESIGN_TOKENS.md §4–§6). The
        // backdrop keeps egui's default black-alpha scrim (no token
        // exists — the design-token gap is noted in DESIGN_TOKENS.md §12
        // follow-ups).
        let frame = egui::Frame::default()
            .fill(color::INK_MID)
            .stroke(egui::Stroke::new(1.0, color::INK_HI))
            .corner_radius(radius::R2)
            .inner_margin(egui::Margin::same(space::S5 as i8));

        let modal = egui::Modal::new(egui::Id::new("selora-settings-dialog"))
            .backdrop_color(color::SCRIM)
            .frame(frame)
            .show(ctx, |ui| {
                // Pin the geometry with an exact allocation: the Modal
                // sizes to its content, and neither a placer pin
                // (set_min_size == set_max_size) nor a size-based
                // allocate_ui bound the horizontal body's children — the
                // ScrollArea's greedy width (`auto_shrink(false)`) grew the
                // modal toward the full screen. Allocating the exact rect
                // tells the modal the content's size (centering works) and
                // the body paints into explicit child rects.
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(WIDTH, HEIGHT), egui::Sense::hover());
                self.body(ui, rect);
            });
        // A backdrop click dismisses — Esc's mouse twin.
        if modal.backdrop_response.clicked() {
            self.events.push(SettingsEvent::Close);
        }

        std::mem::take(&mut self.events)
    }

    // -- layout -----------------------------------------------------------

    /// The two panes, positioned explicitly: a `horizontal` layout in a
    /// modal sized to content mis-measures (the ScrollArea's greedy width
    /// escapes placer pins, and `available_height` inside a fresh
    /// horizontal strip reads the strip's 18px row height, collapsing the
    /// sidebar). Explicit child rects are deterministic.
    fn body(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        // Left column: the 200px section list.
        let sidebar_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(SIDEBAR_WIDTH, rect.height()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(sidebar_rect), |ui| {
            self.sidebar(ui)
        });

        // 1px hairline divider between the columns (DESIGN_TOKENS.md §5).
        let divider = egui::Rect::from_min_size(
            egui::pos2(rect.left() + SIDEBAR_WIDTH, rect.top()),
            egui::vec2(1.0, rect.height()),
        );
        ui.painter().rect_filled(divider, radius::R0, color::INK_HI);

        // Right pane: the fields, scrollable.
        let pane_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + SIDEBAR_WIDTH + 1.0, rect.top()),
            egui::vec2(rect.width() - SIDEBAR_WIDTH - 1.0, rect.height()),
        );
        let mut pane_ui = ui.new_child(egui::UiBuilder::new().max_rect(pane_rect));
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(&mut pane_ui, |ui| {
                ui.set_min_width(ui.available_width());
                self.pane(ui);
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
            Section::General => self.pane_general(ui),
            Section::Editor => self.pane_editor(ui),
            Section::Ui => self.pane_ui(ui),
            Section::Selora => selora_section::pane_selora(self, ui),
            Section::Agent => self.pane_agent(ui),
            Section::Lsp => self.pane_lsp(ui),
            Section::Keybinds => self.pane_keybinds(ui),
        }
    }

    /// General (Phase R5, real at last): the motion discipline. The
    /// theme stays in UI (only selora-dark exists).
    fn pane_general(&mut self, ui: &mut egui::Ui) {
        let resp = ui.checkbox(
            &mut self.settings.ui.reduce_motion,
            "Reduce motion (static dots instead of loops)",
        );
        self.commit_if_changed(&resp);
        note_style(
            ui,
            "Freezes the loops (pulse/spin/orbit); essential fades stay.",
        );
    }

    /// The Agent section (Phase R5): the turn caps, the shell, the
    /// auto-approve allowlist, and the Brave key (OS keyring — never in
    /// settings.json).
    fn pane_agent(&mut self, ui: &mut egui::Ui) {
        label_style(ui, "Max Iterations");
        let resp =
            ui.add(egui::DragValue::new(&mut self.settings.agent.max_iterations).range(1..=100));
        self.commit_drag(&resp);

        field_gap(ui);
        label_style(ui, "Max Turn Seconds");
        let resp =
            ui.add(egui::DragValue::new(&mut self.settings.agent.max_turn_secs).range(10..=3600));
        self.commit_drag(&resp);

        field_gap(ui);
        label_style(ui, "Max Cost (USD)");
        let resp = ui.add(
            egui::DragValue::new(&mut self.settings.agent.max_cost_usd)
                .range(0.01..=50.0)
                .speed(0.01),
        );
        self.commit_drag(&resp);

        field_gap(ui);
        label_style(ui, "Shell");
        let resp = ui.text_edit_singleline(&mut self.settings.agent.shell);
        self.commit_text_on_blur(&resp);

        field_gap(ui);
        label_style(ui, "Auto-approve Commands");
        note_style(ui, "Read-only prefixes that run without an approval card.");
        self.pane_allowlist(ui);

        field_gap(ui);
        label_style(ui, "Brave Search API Key");
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.brave_key)
                    .hint_text("stored in the OS keyring")
                    .password(true)
                    .desired_width(240.0),
            );
            if resp.lost_focus() && !self.brave_key.trim().is_empty() {
                match selora_settings::keyring::brave_key_set(self.brave_key.trim()) {
                    Ok(()) => {
                        self.brave_note =
                            "stored in the OS keyring — never written to settings.json".into();
                        // The plaintext field (legacy pre-R5 files) is
                        // scrubbed with the save.
                        self.settings.agent.brave_search_api_key = None;
                        self.brave_key.clear();
                        self.save_now();
                    }
                    Err(error) => self.brave_note = format!("{error}"),
                }
            }
            if ui.button("Remove").clicked() {
                match selora_settings::keyring::brave_key_delete() {
                    Ok(()) => {
                        self.brave_note = "removed from the keyring".into();
                        self.settings.agent.brave_search_api_key = None;
                        self.save_now();
                    }
                    Err(error) => self.brave_note = format!("{error}"),
                }
            }
        });
        if !self.brave_note.is_empty() {
            note_style(ui, &self.brave_note.clone());
        }
    }

    /// The auto-approve allowlist editor: one row per command prefix with
    /// a remove `×`, plus an add field.
    fn pane_allowlist(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("agent_allowlist")
            .max_height(96.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let mut remove: Option<usize> = None;
                let count = self.settings.agent.auto_approve_commands.len();
                for index in 0..count {
                    ui.horizontal(|ui| {
                        let mut text = self.settings.agent.auto_approve_commands[index].clone();
                        let resp =
                            ui.add(egui::TextEdit::singleline(&mut text).desired_width(220.0));
                        if resp.changed() {
                            self.settings.agent.auto_approve_commands[index] = text;
                            self.save_now();
                        }
                        if ui
                            .button(egui::RichText::new("×").color(color::FG_FAINT))
                            .clicked()
                        {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove {
                    self.settings.agent.auto_approve_commands.remove(index);
                    self.save_now();
                }
            });
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.allowlist_add)
                    .hint_text("add a command prefix")
                    .desired_width(220.0),
            );
            if ui.button("Add").clicked() && !self.allowlist_add.trim().is_empty() {
                let command = self.allowlist_add.trim().to_owned();
                if !self.settings.agent.auto_approve_commands.contains(&command) {
                    self.settings.agent.auto_approve_commands.push(command);
                    self.save_now();
                }
                self.allowlist_add.clear();
            }
            let _ = resp;
        });
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

        // The Minimap checkbox went in Phase R5 — no minimap exists, and
        // a no-op control is a lie (the settings field stays for the
        // future feature).

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
    ///
    /// This is also where the fields report focus for `show`'s Esc handler
    /// (see [`SettingsDialog::field_focused`]).
    fn commit_text_on_blur(&mut self, resp: &egui::Response) {
        self.field_focused |= resp.has_focus();
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
    fn open_selora_section_jumps_from_any_section_and_cancels_capture() {
        let dir = temp_dir();
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir.clone()));
        dialog.section = Section::Keybinds;
        dialog.keybind_capture = Some(KeybindField::Save);

        dialog.open_selora_section();
        assert_eq!(dialog.section, Section::Selora);
        assert_eq!(dialog.keybind_capture, None);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Phase R5: the Agent section paints its allowlist rows (and the
    /// caps/shell/Brave fields) without side effects; the General
    /// section paints the motion toggle.
    #[test]
    fn agent_section_paints_the_allowlist_rows() {
        let dir = temp_dir();
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir.clone()));
        dialog.settings.agent.auto_approve_commands =
            vec!["ls".into(), "cargo check".into(), "dir".into()];
        dialog.section = Section::Agent;
        for _ in 0..2 {
            let mut events = Vec::new();
            egui::__run_test_ui(|ui| {
                let ctx = ui.ctx().clone();
                events = dialog.show(&ctx);
            });
            assert!(events.is_empty(), "events: {events:?}");
        }
        assert_eq!(
            dialog.settings.agent.auto_approve_commands.len(),
            3,
            "painting alone never edits the list"
        );
        assert!(!dir.join(SETTINGS_FILE_NAME).exists(), "no implicit save");
        let _ = std::fs::remove_dir_all(dir);
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
            Section::Agent,
            Section::Lsp,
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

#[cfg(test)]
mod geometry_probe2 {
    use super::*;

    /// TEMPORARY: the real body inside the bare scope (no Modal).
    #[test]
    fn scope_max_rect_probe() {
        let dir = std::env::temp_dir().join(format!("selora-geom2-{}", std::process::id()));
        let mut dialog = SettingsDialog::with_save_dir(Settings::default(), Some(dir.clone()));
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                ui.set_min_size(egui::vec2(1280.0, 800.0));
                let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(720.0, 520.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    eprintln!("scoped available: {:?}", ui.available_size());
                    // Bisect: the pane alone, vertical, no scroll area.
                    dialog.pane(ui);
                    eprintln!("after pane, cursor y advance: {:?}", ui.cursor().min);
                });
            },
        );
        for clipped in &output.shapes {
            if let egui::epaint::Shape::Rect(r) = &clipped.shape {
                if r.rect.width() > 600.0 {
                    eprintln!("  wide rect {:?} fill={:?}", r.rect, r.fill);
                }
            }
        }
        output.drop_without_applying_deltas();
        let _ = std::fs::remove_dir_all(dir);
    }
}
