//! The Phase 2 workbench shell — the `eframe::App` that wires every Phase 2
//! component together (PLAN.md §9 Phase 2, UI_SPEC.md §2).
//!
//! The frame pipeline follows ARCHITECTURE.md §2's seven stages, in order:
//!
//! 1. drain worker messages → [`AppState::apply`][crate::app_state::AppState::apply]
//!    (the `mpsc` receiver plus the Phase 1c notify seam — workers call
//!    `ctx.request_repaint()` after every send so results always reach the
//!    screen);
//! 2. timeouts — the 5s `workspace.json` autosave (§8.2) and toast expiry;
//! 3. derived state — the sequential restore pump (tab order) and the
//!    per-frame dirty derivation fed by [`AppState::note_editor_frame`];
//! 4. layout — title strip, status bar, activity bar + file tree, central
//!    editor area (UI_SPEC.md §2);
//! 5. chrome — tree, palettes, settings dialog, toasts, empty states;
//! 6. editor panes — the split tree's per-pane tab bars, then the active
//!    tab's `Editor` painted into each pane's content rect;
//! 7. effects — saves/opens/pickers the interactions above enqueued onto
//!    the runtime via the `Effect` channel (dispatched by
//!    [`crate::wiring::spawn_dispatcher`]).
//!
//! Keybinds (UI_SPEC.md §15) are routed through `selora-settings`'
//! [`Keybind`] with `settings.keybinds` as the source for the rebindable
//! four; the rest of the Phase 2 table is fixed. While any palette or
//! dialog is open — or any text edit (the tree's inline rename, a palette
//! input) has keyboard focus — the global keybinds stay quiet so they never
//! fire into a modal surface.
//!
//! Blocking `rfd` pickers run on worker threads, never the UI thread.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use selora_editor::{Editor, Language};
use selora_settings::{Keybind, KeybindsSettings};
use selora_ui::dialogs::settings::{SettingsDialog, SettingsEvent};
use selora_ui::editor::split::{self, PaneView};
use selora_ui::palette::registry::CommandId;
use selora_ui::panel::git::GitAction;
use selora_ui::panel::search::SearchEvent;
use selora_ui::sidebar::{activity_bar, Activity, ActivityBar, TreeEvent};
use selora_ui::{brand, theme};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::app_state::{AppMessage, AppState, BufferShape, Effect, Severity};
use crate::persistence::{self, WorkspaceState};
use crate::wiring;

/// Status bar height (UI_SPEC.md §1.2).
const STATUS_BAR_HEIGHT: f32 = 24.0;
/// Title bar height (UI_SPEC.md §1.1).
const TITLE_BAR_HEIGHT: f32 = 32.0;
/// Brand mark size in the title bar (UI_SPEC.md §1.1: 16px).
const MARK_SIZE: f32 = 16.0;
/// Brand mark inset from the window edge (UI_SPEC.md §1.1: 8px).
const EDGE_PADDING: f32 = 8.0;
/// Gap between the mark and the app title (UI_SPEC.md §1.1: 8px).
const TITLE_GAP: f32 = 8.0;
/// File-tree panel bounds (UI_SPEC.md §2: 240px default, 160–480 resizable).
const TREE_MIN_WIDTH: f32 = 160.0;
const TREE_MAX_WIDTH: f32 = 480.0;
/// The `workspace.json` cadence (ARCHITECTURE.md §8.2: "every 5s").
const WORKSPACE_SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// Bottom panel (region ④, UI_SPEC.md §2): 200px default, 0–50% of the
/// workbench height. Shown when the git (Phase 3b) / search (Phase 3c)
/// activity is selected.
const PANEL_HEIGHT: f32 = 200.0;
const PANEL_MIN_HEIGHT: f32 = 0.0;
const PANEL_MAX_HEIGHT_FRAC: f32 = 0.5;
/// Toast anchoring (UI_SPEC.md §13): 12px from each edge, above the 24px
/// status bar.
const TOAST_EDGE: f32 = 12.0;
/// How long the first stroke of `Ctrl+K …` stays armed (UI_SPEC.md §15
/// chords).
const CHORD_WINDOW: Duration = Duration::from_millis(500);
/// The Selora connection ping cadence (Phase 4f, ROADMAP.md: "live ping
/// every 30s").
const SELORA_PING_INTERVAL: Duration = Duration::from_secs(30);
/// The ghost-text idle threshold (Phase 4d, ROADMAP.md: "debounce 350ms
/// idle") — seconds on the egui frame clock.
const GHOST_IDLE_SECS: f64 = 0.350;

/// The Phase 2 shell. Owns the (main-thread-only) [`AppState`], the worker
/// channels, and the transient view state that is not part of the document
/// model (dialog visibility, tree toggle).
pub struct SeloraApp {
    /// Cloned egui context — the notify seam handed to workers so they can
    /// `request_repaint()` after every message.
    ctx: egui::Context,
    /// Handle onto the tokio runtime the workers are spawned onto.
    handle: Handle,
    /// Worker → main-thread messages (drained at the top of every frame).
    msg_rx: mpsc::UnboundedReceiver<AppMessage>,
    /// Main-thread → worker messages (cloned to workers by the dispatcher).
    msg_tx: mpsc::UnboundedSender<AppMessage>,
    /// The main-thread → runtime effect channel.
    effect_tx: mpsc::UnboundedSender<Effect>,
    /// The repaint callback workers invoke after each message send.
    notify: wiring::Notify,
    /// All document/IDE state. Main-thread only (ARCHITECTURE.md §3.1).
    state: AppState,
    /// The activity-bar strip's selection (UI_SPEC.md §2.1).
    activity_bar: ActivityBar,
    /// Whether the file tree panel is visible (`Ctrl+B`, UI_SPEC.md §2).
    show_tree: bool,
    /// Whether region ④ (the bottom panel) is visible (`Ctrl+J`,
    /// UI_SPEC.md §2). The *content* follows the activity bar: git shows
    /// the git panel, search the search panel. Selecting one of those
    /// activities also shows the panel.
    show_bottom_panel: bool,
    /// Result rows whose file was not open yet: opened via
    /// `Effect::OpenFile`, the line jump happens when the buffer loads.
    goto_after_load: VecDeque<(PathBuf, usize)>,
    /// The LSP manager's effect receiver, held until the manager can start
    /// (settings + workspace root known — both arrive via messages after
    /// boot). The dispatcher forwards `Effect::Lsp` onto the sender.
    lsp_effect_rx: Option<mpsc::UnboundedReceiver<selora_lsp::LspEffect>>,
    /// Whether the LSP manager task has been spawned (once per session).
    lsp_started: bool,
    /// The buffer *shape* (char + line counts — O(log n) rope queries) as
    /// of the last `LspEffect::Change` per path, plus the opened-guard:
    /// present = didOpen sent. Per *path*, not per pane — split panes
    /// share one buffer, and the shape is identical from either pane, so
    /// this cannot flap between them the way a per-editor counter would.
    lsp_shapes: BTreeMap<PathBuf, BufferShape>,
    /// When the `Ctrl+K` chord stroke was pressed, if armed.
    chord_ctrl_k: Option<Instant>,
    /// When the last Selora ping effect left (Phase 4f): `None` until
    /// settings arrive (the base URL is theirs), then every
    /// [`SELORA_PING_INTERVAL`].
    last_selora_ping: Option<Instant>,
    /// The last ghost-text request key (Phase 4d): `(path, caret, prefix)`.
    /// A steady-state idle editor re-requests nothing — only a change of
    /// prefix or caret (i.e. an edit) triggers the next request.
    ghost_last_request: Option<(PathBuf, selora_core::Position, String)>,
    /// Settings have arrived (the tree panel's initial width is taken from
    /// them, so the panel first shows *after* the load resolves).
    settings_loaded: bool,
    /// The Settings dialog, while open (UI_SPEC.md §6).
    settings_dialog: Option<SettingsDialog>,
    /// Where `workspace.json` is persisted; `None` = no APPDATA (headless).
    workspace_dir: Option<PathBuf>,
    /// When `workspace.json` was last written (the 5s autosave clock).
    last_workspace_save: Instant,
    /// The last persisted snapshot (skip no-op writes).
    last_workspace_snapshot: Option<WorkspaceState>,
    /// Files to reopen on restore, in tab order. Opens are issued one at a
    /// time — unbounded-channel ordering is only guaranteed per sender, and
    /// tab order *is* the restore contract (ARCHITECTURE.md §8.2).
    restore_queue: VecDeque<PathBuf>,
    /// The restore open currently in flight.
    restore_in_flight: Option<PathBuf>,
    /// The file-tree panel width as of the previous frame (for persistence).
    last_tree_width: Option<f32>,
}

impl SeloraApp {
    /// Build the shell: spawn the effect dispatcher, request the settings
    /// load, and restore the last session if its root still exists
    /// (ARCHITECTURE.md §8.2).
    pub fn new(
        ctx: egui::Context,
        handle: Handle,
        msg_tx: mpsc::UnboundedSender<AppMessage>,
        msg_rx: mpsc::UnboundedReceiver<AppMessage>,
        effect_tx: mpsc::UnboundedSender<Effect>,
        effect_rx: mpsc::UnboundedReceiver<Effect>,
    ) -> Self {
        let mut app = Self::shell(ctx, handle, msg_tx, msg_rx, effect_tx, effect_rx);

        // Settings first (§8.1) — the tree panel waits on the width.
        let _ = app.effect_tx.send(Effect::SettingsLoad);

        // Restore the last session (§8.2): a root that still exists reopens
        // the workspace and requeues its files in tab order.
        if let Some(dir) = app.workspace_dir.clone() {
            if let Some(saved) = persistence::load(&dir) {
                let saved = persistence::restorable(&saved);
                if let Some(root) = saved.root {
                    app.state.workspace = Some(selora_core::Workspace::new(root.clone()));
                    let _ = app.effect_tx.send(Effect::OpenFolder(root));
                    app.restore_queue = saved.open_files.into();
                }
            }
        }

        app
    }

    /// The bare shell: dispatcher spawned, channels wired, nothing loaded,
    /// nothing restored. The boot sequence in [`Self::new`] layers the
    /// settings load and session restore on top; the headless smoke test
    /// builds this directly.
    #[cfg_attr(not(test), allow(dead_code))]
    fn shell(
        ctx: egui::Context,
        handle: Handle,
        msg_tx: mpsc::UnboundedSender<AppMessage>,
        msg_rx: mpsc::UnboundedReceiver<AppMessage>,
        effect_tx: mpsc::UnboundedSender<Effect>,
        effect_rx: mpsc::UnboundedReceiver<Effect>,
    ) -> Self {
        let notify: wiring::Notify = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        // The LSP manager's effect channel: the dispatcher forwards
        // `Effect::Lsp` here; the manager itself is spawned lazily (see
        // `maybe_start_lsp`) once settings and the workspace root are known.
        let (lsp_effect_tx, lsp_effect_rx) = mpsc::unbounded_channel::<selora_lsp::LspEffect>();
        wiring::spawn_dispatcher(
            &handle,
            notify.clone(),
            effect_rx,
            msg_tx.clone(),
            wiring::DEFAULT_BASE_URL.to_owned(),
            lsp_effect_tx,
        );

        Self {
            ctx,
            handle,
            msg_rx,
            msg_tx,
            effect_tx,
            notify,
            state: AppState::default(),
            activity_bar: ActivityBar::default(),
            show_tree: true,
            show_bottom_panel: false,
            goto_after_load: VecDeque::new(),
            lsp_effect_rx: Some(lsp_effect_rx),
            lsp_started: false,
            lsp_shapes: BTreeMap::new(),
            chord_ctrl_k: None,
            settings_loaded: false,
            settings_dialog: None,
            workspace_dir: persistence::state_dir(),
            last_workspace_save: Instant::now(),
            last_workspace_snapshot: None,
            restore_queue: VecDeque::new(),
            restore_in_flight: None,
            last_tree_width: None,
            last_selora_ping: None,
            ghost_last_request: None,
        }
    }

    // -----------------------------------------------------------------------
    // Frame stages 1–3: drain, timeouts, derived state
    // -----------------------------------------------------------------------

    /// §2 stage 1: drain every worker message into the AppState.
    fn drain_messages(&mut self) {
        while let Ok(message) = self.msg_rx.try_recv() {
            let is_settings = matches!(message, AppMessage::SettingsLoaded(_));
            let failed_path = match &message {
                AppMessage::BufferLoadFailed { path, .. } => Some(path.clone()),
                _ => None,
            };
            // A picked folder re-roots the workspace and needs a follow-up
            // tree build (the effect can't be sent from inside `apply`).
            let folder_picked = match &message {
                AppMessage::DialogFolderPicked(Some(path)) => Some(path.clone()),
                _ => None,
            };
            // A new tree (folder opened or refreshed) and every save both
            // re-snapshot git status: the tree's badges and the panel's
            // rows must follow the disk (Phase 3b).
            let tree_or_save = match &message {
                AppMessage::FileTreeUpdated { root, .. } => Some(root.clone()),
                AppMessage::BufferSaved(path) => Some(
                    self.state
                        .workspace
                        .as_ref()
                        .map(|workspace| workspace.root.clone())
                        .unwrap_or_else(|| path.clone()),
                ),
                _ => None,
            };
            self.state.apply(message);
            if is_settings {
                self.settings_loaded = true;
                // Phase 4a: restore the last session's connection — a key
                // in the OS keyring is validated and becomes the session
                // key (the base URL is the freshly loaded settings').
                let base_url = self.state.settings.selora.base_url.clone();
                let _ = self.effect_tx.send(Effect::SeloraAutoConnect { base_url });
            }
            if let Some(root) = folder_picked {
                // The old session's buffers all close with the re-root —
                // tell the language servers before the workspace switches.
                for path in self.state.buffers.keys().cloned().collect::<Vec<_>>() {
                    self.close_lsp_doc(&path);
                }
                let _ = self.effect_tx.send(Effect::OpenFolder(root));
            }
            if let Some(root) = tree_or_save {
                let _ = self.effect_tx.send(Effect::GitStatus(root));
            }
            if let Some(path) = failed_path {
                // A queued search-result jump whose file failed to load:
                // drop it (the load failure already became a toast).
                self.goto_after_load.retain(|(queued, _)| queued != &path);
                // A restore open that failed leaves the queue so the next
                // one can proceed.
                if self.restore_in_flight.as_deref() == Some(path.as_path()) {
                    self.restore_in_flight = None;
                    self.restore_queue.retain(|queued| *queued != path);
                }
            }
            // An F12 definition resolved (set by apply_lsp): jump — opening
            // the target file first when it is not loaded (the same
            // open-then-jump queue the search results use).
            if let Some(location) = self.state.pending_definition.take() {
                self.open_at_line(location.path, location.range.start.line as usize + 1);
            }
        }
    }

    /// §2 stage 2: the 5s workspace-state autosave (§8.2). Writes only when
    /// the snapshot changed since the last write.
    fn maybe_save_workspace(&mut self) {
        if self.last_workspace_save.elapsed() < WORKSPACE_SAVE_INTERVAL {
            return;
        }
        self.last_workspace_save = Instant::now();
        self.save_workspace_now();
    }

    /// Persist the session snapshot synchronously (a tiny atomic write; also
    /// the graceful-close path). `None` workspace_dir = persistence off.
    fn save_workspace_now(&mut self) {
        let Some(dir) = self.workspace_dir.clone() else {
            return;
        };
        let snapshot = self.state.workspace_snapshot();
        if self.last_workspace_snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        if let Err(error) = persistence::save(&dir, &snapshot) {
            self.state.toast(
                format!("workspace state not saved: {error}"),
                Severity::Error,
            );
        }
        self.last_workspace_snapshot = Some(snapshot);
    }

    /// §2 stage 3 (restore half): issue the queued opens one at a time so
    /// arrival order — and therefore tab order — matches the persisted
    /// `open_files` order exactly.
    fn pump_restore(&mut self) {
        if let Some(in_flight) = self.restore_in_flight.clone() {
            if self.state.buffers.contains_key(&in_flight) {
                self.restore_in_flight = None;
            }
        }
        while let Some(next) = self.restore_queue.front().cloned() {
            if self.state.buffers.contains_key(&next) {
                // Already open (e.g. re-opened by hand during restore).
                self.restore_queue.pop_front();
                continue;
            }
            if self.restore_in_flight.is_some() {
                break; // wait for the current open to land
            }
            self.restore_in_flight = Some(next.clone());
            let _ = self.effect_tx.send(Effect::OpenFile(next));
            break;
        }
    }

    /// §2 stage 3 (LSP half): spawn the LSP manager once its two inputs
    /// exist — the loaded settings (server paths) and the workspace root
    /// (handed to servers at `initialize`). Both arrive as messages after
    /// boot, so this runs every frame until it fires, once. Effects sent
    /// before the manager exists queue in the channel.
    fn maybe_start_lsp(&mut self) {
        if self.lsp_started || !self.settings_loaded {
            return;
        }
        let Some(root) = self.state.workspace.as_ref().map(|w| w.root.clone()) else {
            return;
        };
        self.lsp_started = true;
        let registry = selora_lsp::LspRegistry::new(selora_lsp::LspServerPaths {
            rust_analyzer: self.state.settings.lsp.rust_analyzer_path.clone(),
            typescript_language_server: self.state.settings.lsp.typescript_server_path.clone(),
        });
        let config = selora_lsp::LspManagerConfig::new(registry).with_root(Some(root));
        let effect_rx = self
            .lsp_effect_rx
            .take()
            .expect("the manager starts at most once");
        wiring::spawn_lsp_manager(
            &self.handle,
            self.notify.clone(),
            effect_rx,
            self.msg_tx.clone(),
            config,
        );
    }

    /// §2 stage 2 (Selora half): the 30s connection ping loop (Phase 4f)
    /// and the cost meter's day boundary (Phase 4e). The first ping leaves
    /// as soon as settings arrive — the base URL is theirs — so the status
    /// bar reflects reality within one reply, not after 30s. Sending the
    /// effect flips the indicator to `Connecting` until the worker's
    /// [`AppMessage::SeloraStatus`] lands.
    fn drive_selora_ping(&mut self) {
        // The cost meter's "today" window (Phase 4e): clears at the day
        // boundary even with no traffic.
        self.state
            .cost_meter
            .sync_day(selora_ui::status_bar::cost_meter::day_number(
                std::time::SystemTime::now(),
            ));

        if !self.settings_loaded {
            return;
        }
        let due = self
            .last_selora_ping
            .is_none_or(|last| last.elapsed() >= SELORA_PING_INTERVAL);
        if due {
            self.last_selora_ping = Some(Instant::now());
            self.state.selora_status = selora_ai::SeloraStatus::Connecting;
            let _ = self.effect_tx.send(Effect::SeloraPing);
        }
    }

    /// §2 stage 2 (ghost half, Phase 4d): when the active editor has been
    /// idle for the 350ms debounce, send its up-to-caret prefix for a
    /// ghost completion. The request key `(path, caret, prefix)` guards
    /// the steady state — an editor that keeps idling never re-requests;
    /// only an edit (a new prefix or caret) triggers the next one. The
    /// reply lands as [`AppMessage::GhostTextReady`], which offers the
    /// suggestion to the editors (each stale-drops it against its own
    /// caret).
    fn drive_ghost_text(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        let Some(path) = self.state.active_path() else {
            return;
        };
        let path = path.to_path_buf();
        let pane = self.state.editor_area.active_pane_id();
        let Some(editor) = self.state.editors.get(&(pane, path.clone())) else {
            return;
        };
        // A suggestion is already on screen — nothing to request.
        if editor.ghost().is_some() {
            return;
        }
        // No model picked → nothing to complete with (the picker sets it).
        let Some(model) = self.state.settings.selora.active_model.clone() else {
            return;
        };
        let idle = editor
            .last_edit_time()
            .is_none_or(|last_edit| now - last_edit >= GHOST_IDLE_SECS);
        if !idle {
            return;
        }
        let caret = editor.cursor();
        let prefix = editor.text_to_caret();
        if prefix.is_empty() {
            return; // nothing to complete
        }
        let key = (path.clone(), caret, prefix.clone());
        if self.ghost_last_request.as_ref() == Some(&key) {
            return; // this exact request is already in flight / answered
        }
        self.ghost_last_request = Some(key);
        let _ = self.effect_tx.send(Effect::SeloraGhostText {
            path,
            prefix,
            model,
            request_caret: caret,
        });
    }

    /// A buffer was fully dropped (no tab anywhere): close its LSP document
    /// and forget the sync state. No-op for paths never opened with a
    /// server (plain-text files have no entry in `lsp_shapes`).
    fn close_lsp_doc(&mut self, path: &Path) {
        if self.lsp_shapes.remove(path).is_some() {
            let _ = self
                .effect_tx
                .send(Effect::Lsp(selora_lsp::LspEffect::Close {
                    path: path.to_path_buf(),
                }));
        }
    }

    // -----------------------------------------------------------------------
    // Keybinds (UI_SPEC.md §15)
    // -----------------------------------------------------------------------

    /// Whether an overlay owns the keyboard right now: any palette, the
    /// settings dialog, or a focused text edit (the tree's inline rename).
    fn overlay_open(&self, ctx: &egui::Context) -> bool {
        self.state.palette.is_open()
            || self.state.fuzzy.is_open()
            || self.settings_dialog.is_some()
            || ctx.text_edit_focused()
    }

    fn handle_keybinds(&mut self, ctx: &egui::Context) {
        // While the completion popup is open it owns the keyboard: its
        // keys are consumed here (removed from the input the editors will
        // see this frame) and the global binds stay quiet.
        if self.state.lsp.completion.is_some() {
            self.drive_completion_popup(ctx);
            return;
        }
        if self.overlay_open(ctx) {
            self.chord_ctrl_k = None;
            return;
        }
        let keybinds = self.state.settings.keybinds.clone();
        let (mut actions, chord_events) = ctx.input(|input| {
            let mut actions = Vec::new();
            let mut ctrl_k = false;
            let mut plain_i = false;
            for event in &input.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                {
                    actions.push(key_action(
                        &keybinds,
                        &egui::KeyboardShortcut::new(*modifiers, *key),
                    ));
                    // The `Ctrl+K Ctrl+I` hover chord (UI_SPEC.md §15): the
                    // first stroke arms, a plain `I` within 500ms completes.
                    let plain = !modifiers.ctrl && !modifiers.alt && !modifiers.shift;
                    if *key == egui::Key::K && modifiers.ctrl && !modifiers.shift && !modifiers.alt
                    {
                        ctrl_k = true;
                    }
                    if plain && *key == egui::Key::I {
                        plain_i = true;
                    }
                }
            }
            (actions, (ctrl_k, plain_i))
        });
        if chord_events.0 {
            self.chord_ctrl_k = Some(Instant::now());
        }
        if chord_events.1
            && self
                .chord_ctrl_k
                .is_some_and(|at| at.elapsed() < CHORD_WINDOW)
        {
            self.chord_ctrl_k = None;
            actions.push(Some(KeyAction::Hover));
        } else if chord_events.1 {
            self.chord_ctrl_k = None;
        }
        for action in actions.into_iter().flatten() {
            self.run_key_action(action);
        }
    }

    fn run_key_action(&mut self, action: KeyAction) {
        match action {
            KeyAction::FuzzyFind => self.open_fuzzy(),
            KeyAction::CommandPalette => self.toggle_command_palette(),
            KeyAction::Save => self.save_active(),
            KeyAction::ToggleChat => {
                self.state
                    .toast("chat sidebar lands in Phase 4", Severity::Info);
            }
            KeyAction::ToggleFileTree => {
                self.show_tree = !self.show_tree;
            }
            KeyAction::SplitEditor => self.state.split_editor(),
            KeyAction::CloseEditor => {
                if let Some(closed) = self.state.close_editor() {
                    if self.state.prune_closed(&closed) {
                        self.close_lsp_doc(&closed);
                    }
                }
            }
            KeyAction::OpenSettings => self.open_settings(),
            KeyAction::OpenFile => self.pick_file(),
            KeyAction::TogglePanel => self.toggle_bottom_panel(),
            KeyAction::SearchInFiles => self.open_search(),
            KeyAction::GoToDefinition => self.request_lsp(selora_lsp::LspEffect::Definition {
                path: PathBuf::new(),
                position: Default::default(),
            }),
            KeyAction::Completions => self.request_lsp(selora_lsp::LspEffect::Completion {
                path: PathBuf::new(),
                position: Default::default(),
            }),
            KeyAction::Hover => self.request_lsp(selora_lsp::LspEffect::Hover {
                path: PathBuf::new(),
                position: Default::default(),
            }),
        }
    }

    /// F12 / Ctrl+Space / Ctrl+K Ctrl+I: send an LSP request for the active
    /// tab's caret position. The blank path/position are replaced with the
    /// active editor's cursor (the enum carries them; the shell fills them
    /// from focus).
    fn request_lsp(&mut self, template: selora_lsp::LspEffect) {
        let Some(path) = self.state.active_path() else {
            return; // no file open — nothing to ask about
        };
        let pane = self.state.editor_area.active_pane_id();
        let Some(position) = self
            .state
            .editors
            .get(&(pane, path.clone()))
            .map(selora_editor::Editor::cursor)
        else {
            return;
        };
        let effect = match template {
            selora_lsp::LspEffect::Definition { .. } => {
                selora_lsp::LspEffect::Definition { path, position }
            }
            selora_lsp::LspEffect::Hover { .. } => selora_lsp::LspEffect::Hover { path, position },
            selora_lsp::LspEffect::Completion { .. } => {
                selora_lsp::LspEffect::Completion { path, position }
            }
            other => other,
        };
        let _ = self.effect_tx.send(Effect::Lsp(effect));
    }

    /// While the completion popup is open (Phase 3a): its keys are consumed
    /// from the frame's input (so the editor never sees Enter as a newline
    /// or the arrows as caret moves). Enter/Tab accept the selected entry —
    /// replacing the typed identifier prefix — and Esc dismisses. Typing
    /// stays with the editor; the filter re-derives from the caret word
    /// every frame (see `lsp_popups`).
    fn drive_completion_popup(&mut self, ctx: &egui::Context) {
        let Some((path, popup)) = self.state.lsp.completion.take() else {
            return;
        };
        let mut delta: i32 = 0;
        let mut accept = false;
        let mut dismiss = false;
        ctx.input_mut(|input| {
            input.events.retain(|event| match event {
                egui::Event::Key {
                    key, pressed: true, ..
                } => match key {
                    egui::Key::ArrowUp => {
                        delta -= 1;
                        false
                    }
                    egui::Key::ArrowDown => {
                        delta += 1;
                        false
                    }
                    egui::Key::Enter | egui::Key::Tab => {
                        accept = true;
                        false
                    }
                    egui::Key::Escape => {
                        dismiss = true;
                        false
                    }
                    _ => true,
                },
                _ => true,
            });
        });
        let mut popup = popup;
        if delta != 0 {
            popup.move_selection(delta);
        }
        if accept {
            let label = popup
                .filtered()
                .get(popup.selected_index())
                .map(|entry| entry.label.clone());
            if let Some(label) = label {
                let pane = self.state.editor_area.active_pane_id();
                if let Some(editor) = self.state.editors.get_mut(&(pane, path)) {
                    editor.insert_completion(&label);
                }
            }
            return; // popup consumed by acceptance
        }
        if dismiss {
            return; // popup dropped
        }
        self.state.lsp.completion = Some((path, popup));
    }

    /// `Ctrl+J` / View→Toggle Panel (UI_SPEC.md §2 region ④). Toggling on
    /// with the files activity selected defaults the content to search.
    fn toggle_bottom_panel(&mut self) {
        self.show_bottom_panel = !self.show_bottom_panel;
        if self.show_bottom_panel
            && !matches!(self.activity_bar.selected, Activity::Git | Activity::Search)
        {
            self.activity_bar.selected = Activity::Search;
            if let Some(root) = self.state.workspace.as_ref().map(|w| w.root.clone()) {
                self.state.search.set_root(root);
            }
        }
    }

    /// `Ctrl+Shift+F`: switch to the search activity, show region ④, and
    /// focus the query bar.
    fn open_search(&mut self) {
        self.activity_bar.selected = Activity::Search;
        self.show_bottom_panel = true;
        if let Some(root) = self.state.workspace.as_ref().map(|w| w.root.clone()) {
            self.state.search.set_root(root);
        }
        self.state.search.focus_query();
    }

    // -----------------------------------------------------------------------
    // Layout (§2 stages 4–6)
    // -----------------------------------------------------------------------

    /// The left region: the 48px activity strip (always visible) plus the
    /// resizable file tree panel (UI_SPEC.md §2.1, §2.2). The panels' built-in
    /// 1px separators are the `--ink-hi` hairlines between regions.
    fn left_region(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("activity_strip")
            .exact_size(activity_bar::STRIP_WIDTH)
            .resizable(false)
            .show_separator_line(true)
            .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
            .show(ui, |ui| {
                if let Some(activity) = self.activity_bar.show(ui) {
                    self.route_activity(activity);
                }
            });

        // The tree panel first appears with the *loaded* sidebar width, so
        // it waits for the settings effect to resolve (a frame or two).
        // Region ① shows the tree only while the "files" activity is
        // selected (UI_SPEC.md §2.1); search/git take over region ④.
        if self.show_tree && self.settings_loaded && self.activity_bar.selected == Activity::Files {
            let default_width = self.state.settings.ui.sidebar_width as f32;
            let response = egui::Panel::left("file_tree_panel")
                .resizable(true)
                .default_size(default_width)
                .min_size(TREE_MIN_WIDTH)
                .max_size(TREE_MAX_WIDTH)
                .show_separator_line(true)
                .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
                .show(ui, |ui| {
                    let outcome = self.state.file_tree.show(ui);
                    for event in outcome.events {
                        self.route_tree_event(event);
                    }
                });
            self.persist_tree_width(response.response.rect.width());
        }
    }

    fn route_activity(&mut self, activity: Activity) {
        match activity {
            Activity::Files => {}
            Activity::Search => {
                // Selecting search takes region ④ with the search panel
                // and focuses the query bar (Phase 3c).
                self.show_bottom_panel = true;
                if let Some(root) = self.state.workspace.as_ref().map(|w| w.root.clone()) {
                    self.state.search.set_root(root);
                }
                self.state.search.focus_query();
            }
            Activity::Git => {
                // Selecting the git activity shows the panel (region ④) and
                // refreshes its data — a status snapshot is cheap and always
                // current on click (Phase 3b).
                self.show_bottom_panel = true;
                if let Some(root) = self.state.workspace.as_ref().map(|w| w.root.clone()) {
                    let _ = self.effect_tx.send(Effect::GitStatus(root));
                }
            }
            Activity::Chat => {
                self.state
                    .toast("chat sidebar lands in Phase 4", Severity::Info);
            }
        }
    }

    fn route_tree_event(&mut self, event: TreeEvent) {
        match event {
            TreeEvent::OpenFile(path) => self.open_path(path),
            TreeEvent::TreeChanged => {
                if let Some(root) = self.state.workspace.as_ref().map(|w| w.root.clone()) {
                    let _ = self.effect_tx.send(Effect::RefreshFileTree(root));
                }
            }
            TreeEvent::Error(message) => {
                self.state.toast(message, Severity::Error);
            }
        }
    }

    /// Persist the dragged sidebar width into `settings.json` (on release,
    /// not per pixel). Runs off the frame path via `Effect::SettingsSave`.
    fn persist_tree_width(&mut self, width: f32) {
        let width = width.round().clamp(TREE_MIN_WIDTH, TREE_MAX_WIDTH);
        if self
            .last_tree_width
            .is_some_and(|last| (last - width).abs() < 0.5)
        {
            return;
        }
        if self.ctx.input(|input| input.pointer.primary_down()) {
            return; // mid-drag; persist on release
        }
        self.last_tree_width = Some(width);
        let mut settings = self.state.settings.clone();
        settings.ui.sidebar_width = width as u32;
        self.state.settings = settings.clone();
        let _ = self.effect_tx.send(Effect::SettingsSave(settings));
    }

    /// Region ④ (UI_SPEC.md §2): the bottom panel, visible while
    /// `Ctrl+J`/an activity selection turned it on AND a panel-bearing
    /// activity (git, search) is selected. Painted above the status bar
    /// (which is added first and keeps the lowest strip). Resizable,
    /// 200px default, capped at half the workbench.
    fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        let selected = self.activity_bar.selected;
        if !self.show_bottom_panel || !matches!(selected, Activity::Git | Activity::Search) {
            return;
        }
        let max_height = ui.max_rect().height() * PANEL_MAX_HEIGHT_FRAC;
        egui::Panel::bottom("bottom_panel")
            .resizable(true)
            .default_size(PANEL_HEIGHT)
            .min_size(PANEL_MIN_HEIGHT)
            .max_size(max_height)
            .show_separator_line(true)
            .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
            .show(ui, |ui| match selected {
                Activity::Git => {
                    let outcome = self
                        .state
                        .git_panel
                        .show(ui, self.state.git_status.as_ref());
                    for action in outcome.actions {
                        self.route_git_action(action);
                    }
                }
                Activity::Search => {
                    let outcome = self.state.search.show(ui);
                    for event in outcome.events {
                        self.route_search_event(event);
                    }
                }
                _ => {}
            });
    }

    /// A search-panel event → `Effect` / editor jump (Phase 3c).
    fn route_search_event(&mut self, event: SearchEvent) {
        match event {
            SearchEvent::RunSearch { id, query } => {
                match self.state.workspace.as_ref().map(|w| w.root.clone()) {
                    Some(root) => {
                        let _ = self.effect_tx.send(Effect::Search { id, root, query });
                    }
                    None => {
                        self.state.toast("open a folder to search", Severity::Info);
                    }
                }
            }
            SearchEvent::OpenResultAtLine { path, line } => {
                self.open_at_line(path, line);
            }
            SearchEvent::Info(text) => {
                self.state.toast(text, Severity::Info);
            }
        }
    }

    /// Open `path` and jump to a **1-based** line (a search result; later
    /// also problems and LSP go-to-definition). A buffer that has to load
    /// first queues the jump — its pane editor is created at first paint.
    fn open_at_line(&mut self, path: PathBuf, line: usize) {
        if self.state.buffers.contains_key(&path) {
            self.state.open_file(path.clone());
            self.state.goto_line(&path, line);
        } else {
            self.goto_after_load.push_back((path.clone(), line));
            let _ = self.effect_tx.send(Effect::OpenFile(path));
        }
    }

    /// A git-panel action → `Effect` (the panel never touches the runtime).
    fn route_git_action(&mut self, action: GitAction) {
        match action {
            GitAction::Stage(path) => {
                let _ = self.effect_tx.send(Effect::GitStage(path));
            }
            GitAction::Unstage(path) => {
                let _ = self.effect_tx.send(Effect::GitUnstage(path));
            }
            GitAction::Commit(message) => {
                if let Some(root) = self.state.workspace.as_ref().map(|w| w.root.clone()) {
                    let _ = self.effect_tx.send(Effect::GitCommit(root, message));
                } else {
                    self.state
                        .toast("no folder open to commit", Severity::Error);
                }
            }
            GitAction::OpenDiff(path) => {
                // Open the file itself for now (Lite Phase 3); the
                // read-only diff view is a v1.x refinement (UI_SPEC.md §8).
                self.open_path(path);
            }
        }
    }

    /// The 24px status bar (UI_SPEC.md §1.2, Phase 2 subset): workspace root
    /// name (or "no folder") left, cursor line:col + language center, a
    /// muted "○ selora disconnected" right (live status is Phase 4).
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar")
            .exact_size(STATUS_BAR_HEIGHT)
            .resizable(false)
            .show_separator_line(true)
            .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let painter = ui.painter_at(rect);
                let sans = || egui::FontId::new(theme::type_scale::XS, theme::font::FAMILY_SANS);
                let mono = || egui::FontId::new(theme::type_scale::XS, theme::font::FAMILY_MONO);

                // Left: the git branch + "●" + change count (UI_SPEC.md §1.2,
                // the Phase 3b subset — ahead/behind need remotes, out of
                // Lite). Outside a repository: the workspace root's name.
                let left = match self.state.git_status.as_ref() {
                    Some(status) => {
                        let branch = status.branch.clone().unwrap_or_default();
                        format!("{branch} ● {}", status.changed_count())
                    }
                    None => self
                        .state
                        .workspace
                        .as_ref()
                        .and_then(|workspace| workspace.root.file_name())
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "no folder".to_owned()),
                };
                let left_font = sans();
                // In a repo with changes, the dot runs hot (--warn); the
                // rest of the run stays --fg-muted. Painted as one text run
                // in --fg-muted plus a warn-colored "●" glyph over it at the
                // branch-name offset (measured with the same font).
                let dirty = self
                    .state
                    .git_status
                    .as_ref()
                    .is_some_and(|status| status.changed_count() > 0);
                let dot_pos = if dirty {
                    let branch = self
                        .state
                        .git_status
                        .as_ref()
                        .and_then(|status| status.branch.clone())
                        .unwrap_or_default();
                    let galley = painter.layout_no_wrap(
                        format!("{branch} "),
                        left_font.clone(),
                        theme::color::FG_MUTED,
                    );
                    Some(egui::pos2(
                        rect.left() + theme::space::S3 + galley.size().x,
                        rect.center().y,
                    ))
                } else {
                    None
                };
                painter.text(
                    egui::pos2(rect.left() + theme::space::S3, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    left.clone(),
                    left_font.clone(),
                    theme::color::FG_MUTED,
                );
                if let Some(pos) = dot_pos {
                    // Same font as the run it overlays, so the x offset
                    // measured above lands exactly on the dot.
                    painter.text(
                        pos,
                        egui::Align2::LEFT_CENTER,
                        "●",
                        left_font,
                        theme::color::WARN,
                    );
                }

                // Left, after the git run: per-server LSP status
                // ("rust-analyzer: ready", Phase 3a). A dead server is a
                // click target for `Effect::Lsp(Restart)` (§5.6).
                let mut status_x = painter
                    .layout_no_wrap(
                        format!("{left}  "),
                        egui::FontId::new(theme::type_scale::XS, theme::font::FAMILY_SANS),
                        theme::color::FG_MUTED,
                    )
                    .size()
                    .x
                    + rect.left()
                    + theme::space::S3;
                let servers: Vec<(selora_lsp::LanguageId, selora_lsp::ServerStatus)> = self
                    .state
                    .lsp
                    .status
                    .iter()
                    .map(|(language, status)| (*language, status.clone()))
                    .collect();
                for (language, status) in servers {
                    let label = match &status {
                        selora_lsp::ServerStatus::Starting => "starting".to_owned(),
                        selora_lsp::ServerStatus::Ready => "ready".to_owned(),
                        selora_lsp::ServerStatus::Dead(_) => "dead — click to restart".to_owned(),
                    };
                    let text = format!("{}: {label}", language.server_name());
                    let color = match &status {
                        selora_lsp::ServerStatus::Dead(_) => theme::color::ERR,
                        _ => theme::color::FG_MUTED,
                    };
                    let font = sans();
                    let galley = painter.layout_no_wrap(text.clone(), font.clone(), color);
                    let text_rect = egui::Rect::from_min_size(
                        egui::pos2(status_x, rect.top()),
                        egui::vec2(galley.size().x + theme::space::S3, rect.height()),
                    );
                    painter.galley(
                        egui::pos2(status_x, rect.center().y - galley.size().y / 2.0),
                        galley,
                        color,
                    );
                    if matches!(status, selora_lsp::ServerStatus::Dead(_)) {
                        let response = ui.interact(
                            text_rect,
                            egui::Id::new(("lsp_restart", language)),
                            egui::Sense::click(),
                        );
                        if response.clicked() {
                            let _ = self
                                .effect_tx
                                .send(Effect::Lsp(selora_lsp::LspEffect::Restart(language)));
                        }
                    }
                    status_x = text_rect.right() + theme::space::S3;
                }

                // Center: cursor line:col + language of the active tab, and
                // the LSP work-done progress line while a server indexes
                // ("rust-analyzer: Indexing 42%", Phase 3a).
                let pane = self.state.editor_area.active_pane_id();
                let mut center = self
                    .state
                    .active_path()
                    .and_then(|path| self.state.editors.get(&(pane, path)))
                    .map(|editor| {
                        let cursor = editor.cursor();
                        format!(
                            "Ln {}, Col {}   {}",
                            cursor.line + 1,
                            cursor.col + 1,
                            language_name(editor.language()),
                        )
                    })
                    .unwrap_or_default();
                if let Some(progress) = self.state.lsp.progress.clone() {
                    if !center.is_empty() {
                        center.push_str("   ");
                    }
                    center.push_str(&progress);
                }
                if !center.is_empty() {
                    let center_font = mono();
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        center,
                        center_font,
                        theme::color::FG_MUTED,
                    );
                }

                // Right: the live Selora connection (Phase 4f) at the very
                // edge, the `Σ today` cost meter (Phase 4e) left of it.
                // The active model from settings wins over the ping's own
                // (the ping cannot see the picker's choice).
                let active_model = self.state.settings.selora.active_model.clone();
                let status_events = selora_ui::status_bar::selora_status::show(
                    ui,
                    &self.state.selora_status,
                    active_model.as_deref(),
                );
                // Measure the connection run to anchor the meter left of
                // it without overlap (same font the widget paints with).
                let connection_run = selora_ui::status_bar::selora_status::label(
                    &self.state.selora_status,
                    active_model.as_deref(),
                );
                let run_width = painter
                    .layout_no_wrap(
                        connection_run,
                        egui::FontId::new(theme::type_scale::XS, theme::font::FAMILY_SANS),
                        theme::color::FG_MUTED,
                    )
                    .size()
                    .x;
                let meter_zone = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rect.top()),
                    egui::pos2(
                        rect.right() - theme::space::S3 - run_width - theme::space::S4,
                        rect.bottom(),
                    ),
                );
                selora_ui::status_bar::cost_meter::CostMeter::new(&self.state.cost_meter)
                    .show(ui, meter_zone);
                for event in status_events {
                    match event {
                        selora_ui::status_bar::selora_status::StatusEvent::OpenConnect => {
                            self.open_settings();
                        }
                    }
                }
            });
    }

    /// The central editor area: the split tree with its tab bars, the
    /// per-pane editors, the empty states, and the fuzzy-file overlay.
    fn editor_region(&mut self, ui: &mut egui::Ui, workbench: &mut egui::Rect) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
            .show(ui, |ui| {
                *workbench = ui.max_rect();
                if self.state.workspace.is_none() {
                    self.empty_no_folder(ui);
                    return;
                }

                // Stages 5–6: the split chrome, then the editors it reports.
                let response = split::show(ui, &mut self.state.editor_area);
                for closed in response.closed.clone() {
                    if let Some(workspace) = &mut self.state.workspace {
                        workspace.close(&closed);
                    }
                    if self.state.prune_closed(&closed) {
                        self.close_lsp_doc(&closed);
                    }
                }
                for _pane in &response.new_tab {
                    self.open_fuzzy(); // "+" opens quick-open (UI_SPEC.md §3.1)
                }
                for view in response.panes {
                    self.paint_pane(ui, view);
                }

                // The Ctrl+P overlay is scoped to the workbench (UI_SPEC §5.1).
                if let Some(path) = self.state.fuzzy.show_ui(ui) {
                    self.open_path(path);
                }
            });
    }

    /// Paint one pane's active-tab editor into the pane's content rect
    /// (UI_SPEC.md §3.2). Each (pane, tab) keeps its own `Editor` — its own
    /// cursor and scroll — over the buffer shared by path.
    fn paint_pane(&mut self, ui: &mut egui::Ui, view: PaneView) {
        let Some(path) = self
            .state
            .editor_area
            .pane(view.pane)
            .and_then(|tabs| tabs.active_path().map(Path::to_path_buf))
        else {
            self.empty_folder_no_file(ui, view.content);
            return;
        };
        let Some(buffer) = self.state.buffers.get(&path).cloned() else {
            return; // a tab never points at an unloaded buffer
        };
        let is_new = !self.state.editors.contains_key(&(view.pane, path.clone()));
        let editor = self
            .state
            .editors
            .entry((view.pane, path.clone()))
            .or_insert_with(|| Editor::new(buffer.clone()));
        if is_new {
            // Syntax highlighting by extension (Phase 2d).
            editor.set_language_from_path(&path);
            // A queued search-result jump for this freshly-created editor:
            // the line was known before the buffer loaded.
            if let Some(index) = self
                .goto_after_load
                .iter()
                .position(|(queued, _)| queued == &path)
            {
                let (_, line) = self.goto_after_load.remove(index).expect("indexed entry");
                editor.goto_line(line);
            }
        }
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(view.content), |ui| {
                ui.add(editor)
            })
            .inner;
        // Editor interaction focuses its pane (the split's own hit-test only
        // sees the background beneath the widget painted on top).
        if response.clicked() {
            self.state.editor_area.set_active_pane(view.pane);
        }

        // §2 stage 3, per frame: dirty derivation. The shape is O(log n);
        // the edit signal covers same-length replacements the shape cannot
        // see. See app_state's module docs.
        let shape = buffer
            .read()
            .map(|buffer| BufferShape::of(&buffer))
            .unwrap_or_default();
        let edit_input = response.has_focus() && edit_typed_this_frame(ui.ctx());
        self.state.note_editor_frame(&path, shape, edit_input);

        // LSP document sync (Phase 3a, deliverable 4): didOpen on the first
        // paint of the *path* (per-path guard — split panes share one
        // document), didChange when the buffer's shape moved or the focused
        // pane took edit input (same detection as the dirty derivation —
        // the shape alone cannot see same-length replacements), and hover
        // requests from the 600ms pointer-hold gesture. The manager
        // debounces changes 250ms and drops no-op texts itself.
        let editor = self
            .state
            .editors
            .get_mut(&(view.pane, path.clone()))
            .expect("the editor was just painted");
        if !self.lsp_shapes.contains_key(&path) {
            let _ = self
                .effect_tx
                .send(Effect::Lsp(selora_lsp::LspEffect::Open {
                    path: path.clone(),
                    text: editor.text(),
                }));
            self.lsp_shapes.insert(path.clone(), shape);
        } else if self.lsp_shapes.get(&path).is_none_or(|last| *last != shape) || edit_input {
            self.lsp_shapes.insert(path.clone(), shape);
            let _ = self
                .effect_tx
                .send(Effect::Lsp(selora_lsp::LspEffect::Change {
                    path: path.clone(),
                    text: editor.text(),
                }));
        }
        for position in editor.take_lsp_requests() {
            let _ = self
                .effect_tx
                .send(Effect::Lsp(selora_lsp::LspEffect::Hover {
                    path: path.clone(),
                    position,
                }));
        }
    }

    /// Empty state: no folder open (UI_SPEC.md §12).
    fn empty_no_folder(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.35);
            ui.label(
                egui::RichText::new("No folder open")
                    .color(theme::color::FG_DEFAULT)
                    .font(egui::FontId::new(
                        theme::type_scale::LG,
                        theme::font::FAMILY_SANS,
                    )),
            );
            ui.add_space(theme::space::S2);
            ui.label(
                egui::RichText::new("Open a folder to start editing.")
                    .italics()
                    .color(theme::color::FG_MUTED)
                    .font(egui::FontId::new(
                        theme::type_scale::MD,
                        theme::font::FAMILY_SANS,
                    )),
            );
            ui.add_space(theme::space::S3);
            ui.horizontal(|ui| {
                let button = egui::Button::new(
                    egui::RichText::new("Open Folder").color(theme::color::FG_DEFAULT),
                )
                .fill(theme::color::INK_MID)
                .corner_radius(theme::radius::R1 as u8);
                if ui.add(button).clicked() {
                    self.pick_folder();
                }
                ui.add_space(theme::space::S1);
                let button = egui::Button::new(
                    egui::RichText::new("Clone Repo").color(theme::color::FG_DEFAULT),
                )
                .fill(theme::color::INK_MID)
                .corner_radius(theme::radius::R1 as u8);
                if ui.add(button).clicked() {
                    self.state
                        .toast("Clone Repo is coming soon", Severity::Info);
                }
            });
        });
    }

    /// Empty state: a folder is open but this pane has no tab (UI_SPEC.md
    /// §12) — the keybind hint line, with chips from the live settings.
    fn empty_folder_no_file(&mut self, ui: &mut egui::Ui, content: egui::Rect) {
        let fuzzy_key = self.state.settings.keybinds.fuzzy_find.clone();
        let palette_key = self.state.settings.keybinds.command_palette.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.4);
                ui.horizontal(|ui| {
                    keybind_chip(ui, &fuzzy_key);
                    ui.label(
                        egui::RichText::new("to find a file")
                            .italics()
                            .color(theme::color::FG_MUTED)
                            .font(egui::FontId::new(
                                theme::type_scale::MD,
                                theme::font::FAMILY_SANS,
                            )),
                    );
                    ui.add_space(theme::space::S2);
                    keybind_chip(ui, &palette_key);
                    ui.label(
                        egui::RichText::new("for commands")
                            .italics()
                            .color(theme::color::FG_MUTED)
                            .font(egui::FontId::new(
                                theme::type_scale::MD,
                                theme::font::FAMILY_SANS,
                            )),
                    );
                });
            });
        });
    }

    // -----------------------------------------------------------------------
    // Overlays (stage 5): palettes, dialog, toasts
    // -----------------------------------------------------------------------

    fn command_palette(&mut self, ctx: &egui::Context, workbench: egui::Rect) {
        if let Some(id) = self.state.palette.show_ui(ctx, workbench) {
            self.run_command(id);
        }
    }

    fn toggle_command_palette(&mut self) {
        if self.state.fuzzy.is_open() {
            self.state.fuzzy.hide();
        }
        self.state.palette.toggle();
    }

    fn open_fuzzy(&mut self) {
        if self.state.palette.is_open() {
            self.state.palette.hide();
        }
        if self.state.fuzzy_source.is_empty() {
            self.state
                .toast("no files to find — open a folder first", Severity::Info);
            return;
        }
        self.state.fuzzy.show();
    }

    fn open_settings(&mut self) {
        if self.settings_dialog.is_none() {
            self.settings_dialog = Some(SettingsDialog::new(self.state.settings.clone()));
        }
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.settings_dialog.as_mut() else {
            return;
        };
        // The Selora section paints the live connection state (Phase 4a):
        // hand it over before every paint — it's cheap, and a transition
        // to Connected clears the dialog's masked key buffer.
        dialog.set_selora_status(self.state.selora_status.clone());
        let events = dialog.show(ctx);
        let mut close = false;
        let mut adopt: Option<selora_settings::Settings> = None;
        for event in events {
            match event {
                SettingsEvent::Close => close = true,
                SettingsEvent::Saved => {
                    self.state.toast("Settings saved", Severity::Info);
                    adopt = Some(dialog.settings().clone());
                }
                SettingsEvent::SaveFailed(message) => {
                    self.state.toast(message, Severity::Error);
                    adopt = Some(dialog.settings().clone());
                }
                // Phase 4a (ARCHITECTURE.md §7.2): validate via
                // list_models, then write the OS keyring. The worker's
                // SeloraStatus reply drives the dialog's status line and
                // the status-bar run; the Connecting form is immediate.
                SettingsEvent::Connect(key) => {
                    self.state.selora_status = selora_ai::SeloraStatus::Connecting;
                    let _ = self.effect_tx.send(Effect::SeloraConnect(key));
                }
                // Phase 4a: drop the session key + delete the keyring
                // entry; the reply resets the status to Disconnected.
                SettingsEvent::Disconnect => {
                    let _ = self.effect_tx.send(Effect::SeloraDisconnect);
                }
            }
        }
        if let Some(settings) = adopt {
            self.state.settings = settings;
        }
        if close {
            self.settings_dialog = None;
        }
    }

    /// LSP overlays (Phase 3a): the hover tooltip and the completion popup,
    /// painted in a foreground layer anchored at the caret of the pane's
    /// editor that requested them.
    fn lsp_popups(&mut self, ctx: &egui::Context) {
        // The hover card dismisses on any key press or click (a tooltip is
        // transient by nature; a new hover event replaces it).
        if let Some((path, lines)) = self.state.lsp.hover.clone() {
            let transient_input = ctx.input(|input| {
                input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key { pressed: true, .. }
                            | egui::Event::PointerButton { pressed: true, .. }
                    )
                })
            });
            if transient_input {
                self.state.lsp.hover = None;
            } else if let Some(anchor) = self.editor_caret(&path) {
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("lsp_hover"),
                ));
                selora_editor::lsp_overlay::paint_hover_tooltip(&painter, anchor, &lines);
            }
        }

        // The completion popup: re-derive the filter from the identifier
        // prefix ending at the caret (typing stays with the editor), then
        // paint at the caret. A popup whose tab closed is dropped.
        if let Some((path, mut popup)) = self.state.lsp.completion.take() {
            let mut painted = false;
            if let Some(editor) = self.active_editor_for(&path) {
                let (line, col) = (editor.cursor().line, editor.cursor().col);
                let word = self
                    .state
                    .buffers
                    .get(&path)
                    .and_then(|buffer| buffer.read().ok())
                    .and_then(|buffer| buffer.line(line).map(|text| text.to_owned()))
                    .map(|text| {
                        let start = selora_editor::lsp_overlay::completion_word_start(&text, col);
                        // Char columns, not byte offsets (the line may hold
                        // non-ASCII before the caret).
                        text.chars()
                            .skip(start)
                            .take(col.saturating_sub(start))
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                popup.set_filter(&word);
                if let Some(anchor) = editor.caret_screen_pos() {
                    let painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::new("lsp_completion"),
                    ));
                    selora_editor::lsp_overlay::paint_completion_popup(&painter, anchor, &popup);
                    painted = true;
                }
            }
            if painted {
                self.state.lsp.completion = Some((path, popup));
            }
        }
    }

    /// The active pane's editor for `path`, or the first pane showing it.
    fn active_editor_for(&self, path: &Path) -> Option<&Editor> {
        let active = self.state.editor_area.active_pane_id();
        self.state
            .editors
            .get(&(active, path.to_path_buf()))
            .or_else(|| {
                self.state
                    .editor_area
                    .pane_ids()
                    .into_iter()
                    .find_map(|pane| self.state.editors.get(&(pane, path.to_path_buf())))
            })
    }

    /// The last-painted caret position of the pane's editor for `path`
    /// (the popup anchor).
    fn editor_caret(&self, path: &Path) -> Option<egui::Pos2> {
        self.active_editor_for(path)
            .and_then(|editor| editor.caret_screen_pos())
    }

    /// Toasts (UI_SPEC.md §13): bottom-right above the status bar, `--ink-mid`
    /// fill, 1px `--ink-hi` hairline, radius-1, 12px padding, max 3.
    fn toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(next) = self.state.expire_toasts(now) {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
        if self.state.toasts.is_empty() {
            return;
        }

        let toasts = std::mem::take(&mut self.state.toasts);
        let mut dismissed = None;
        egui::Area::new(egui::Id::new("selora_toasts"))
            .order(egui::Order::Foreground)
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-TOAST_EDGE, -(TOAST_EDGE + STATUS_BAR_HEIGHT)),
            )
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.spacing_mut().item_spacing.y = theme::space::S2;
                for (index, toast) in toasts.iter().enumerate() {
                    let (glyph, color) = match toast.severity {
                        Severity::Info => ("i", theme::color::INFO),
                        Severity::Error => ("!", theme::color::ERR),
                    };
                    egui::Frame::default()
                        .fill(theme::color::INK_MID)
                        .stroke(egui::Stroke::new(1.0, theme::color::INK_HI))
                        .corner_radius(theme::radius::R1 as u8)
                        .inner_margin(egui::Margin::same(theme::space::S3 as i8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(glyph).color(color).font(
                                    egui::FontId::new(
                                        theme::type_scale::XS,
                                        theme::font::FAMILY_MONO,
                                    ),
                                ));
                                ui.label(
                                    egui::RichText::new(toast.text.as_str())
                                        .color(theme::color::FG_DEFAULT)
                                        .font(egui::FontId::new(
                                            theme::type_scale::SM,
                                            theme::font::FAMILY_SANS,
                                        )),
                                );
                                if ui
                                    .button(egui::RichText::new("×").color(theme::color::FG_FAINT))
                                    .clicked()
                                {
                                    dismissed = Some(index);
                                }
                            });
                        });
                }
            });
        self.state.toasts = toasts;
        if let Some(index) = dismissed {
            self.state.toasts.remove(index);
        }
    }

    // -----------------------------------------------------------------------
    // Commands and effects (§2 stage 7)
    // -----------------------------------------------------------------------

    /// Route a command-palette [`CommandId`] (UI_SPEC.md §5). The
    /// Phase 2-reachable subset runs; the rest toast their landing phase —
    /// no silent no-ops (PLAN.md §8).
    fn run_command(&mut self, id: CommandId) {
        match id {
            CommandId::FileOpenFile => self.pick_file(),
            CommandId::FileOpenFolder => self.pick_folder(),
            CommandId::FileSave => self.save_active(),
            CommandId::FileSaveAll => self.save_all(),
            CommandId::FileCloseEditor => {
                if let Some(closed) = self.state.close_editor() {
                    if self.state.prune_closed(&closed) {
                        self.close_lsp_doc(&closed);
                    }
                }
            }
            CommandId::FileCloseAllEditors => {
                let closed = self.state.close_all_editors();
                for path in &closed {
                    if self.state.prune_closed(path) {
                        self.close_lsp_doc(path);
                    }
                }
            }
            CommandId::FileGoToFile => self.open_fuzzy(),
            CommandId::ViewToggleFileTree => {
                self.show_tree = !self.show_tree;
            }
            CommandId::ViewSplitEditor => self.state.split_editor(),
            CommandId::ViewCommandPalette => self.toggle_command_palette(),
            CommandId::SettingsOpen => self.open_settings(),
            // Phase 3: LSP, search, bottom panel.
            CommandId::ViewTogglePanel => self.toggle_bottom_panel(),
            CommandId::ViewGoToSymbol => {
                self.state
                    .toast("go to symbol lands in Phase 3", Severity::Info);
            }
            CommandId::EditToggleComment => {
                self.state
                    .toast("toggle comment lands in Phase 3", Severity::Info);
            }
            CommandId::EditFindInFile => {
                self.state
                    .toast("find in file lands in Phase 3", Severity::Info);
            }
            CommandId::EditReplaceInFile => {
                self.state
                    .toast("replace in file lands in Phase 3", Severity::Info);
            }
            CommandId::EditGoToDefinition => {
                self.state
                    .toast("go to definition lands in Phase 3", Severity::Info);
            }
            // Phase 4: the Selora UI.
            CommandId::ViewToggleChatSidebar => {
                self.state
                    .toast("chat sidebar lands in Phase 4", Severity::Info);
            }
            CommandId::SeloraSwitchModel => {
                self.state
                    .toast("model switching lands in Phase 4", Severity::Info);
            }
            CommandId::SeloraClearApiKey => {
                self.state
                    .toast("API key management lands in Phase 4", Severity::Info);
            }
        }
    }

    /// Open `path`: reuse the loaded buffer (focus the tab) or enqueue an
    /// async read (ARCHITECTURE.md §4.1).
    fn open_path(&mut self, path: PathBuf) {
        if self.state.buffers.contains_key(&path) {
            self.state.open_file(path);
        } else {
            let _ = self.effect_tx.send(Effect::OpenFile(path));
        }
    }

    fn save_active(&mut self) {
        let Some(path) = self.state.active_path() else {
            self.state.toast("no file open to save", Severity::Info);
            return;
        };
        self.save_path(&path);
    }

    fn save_path(&mut self, path: &Path) {
        let Some(buffer) = self.state.buffers.get(path).cloned() else {
            return;
        };
        let _ = self.effect_tx.send(Effect::SaveBuffer {
            path: path.to_path_buf(),
            buffer,
        });
        // The save also tells the language server (didSave flushes any
        // pending debounced didChange first, Phase 3a).
        let _ = self
            .effect_tx
            .send(Effect::Lsp(selora_lsp::LspEffect::Save {
                path: path.to_path_buf(),
            }));
    }

    fn save_all(&mut self) {
        let mut paths: Vec<PathBuf> = Vec::new();
        for pane in self.state.editor_area.pane_ids() {
            if let Some(tabs) = self.state.editor_area.pane(pane) {
                for path in tabs.paths() {
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
        }
        if paths.is_empty() {
            self.state.toast("no open files to save", Severity::Info);
            return;
        }
        for path in paths {
            self.save_path(&path);
        }
    }

    /// Blocking `rfd` pickers run on worker threads, never the UI thread.
    fn pick_folder(&mut self) {
        wiring::spawn_pick_folder(&self.handle, &self.notify, &self.msg_tx);
    }

    fn pick_file(&mut self) {
        wiring::spawn_pick_file(&self.handle, &self.notify, &self.msg_tx);
    }
}

impl eframe::App for SeloraApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui_body(ui);
    }

    /// Graceful close: persist the session one last time (§8.2).
    fn on_exit(&mut self) {
        self.save_workspace_now();
    }
}

impl SeloraApp {
    /// One frame, in ARCHITECTURE.md §2 stage order. Split from the
    /// `eframe::App` impl so the headless smoke test can drive the exact
    /// production frame body without an `eframe::Frame`.
    fn ui_body(&mut self, ui: &mut egui::Ui) {
        // §2 stage 1: drain worker messages before any painting.
        self.drain_messages();
        // Stage 2: timeouts (the 5s workspace autosave; toast expiry happens
        // in `toasts` below, which owns their clock; the Selora ping loop
        // and the cost meter's day boundary live here too).
        self.maybe_save_workspace();
        self.drive_selora_ping();
        self.drive_ghost_text(ui.ctx());
        // Stage 3: derived state (sequential restore, dirty derivation per
        // painted editor, the LSP manager's lazy start).
        self.pump_restore();
        self.maybe_start_lsp();

        // Keybinds run only when no overlay owns the keyboard.
        self.handle_keybinds(ui.ctx());

        // Stages 4–6: layout, chrome, editor.
        title_strip(ui);
        self.status_bar(ui);
        self.bottom_panel(ui);
        self.left_region(ui);
        let mut workbench = egui::Rect::ZERO;
        self.editor_region(ui, &mut workbench);

        // Overlays.
        self.command_palette(ui.ctx(), workbench);
        self.lsp_popups(ui.ctx());
        self.settings_dialog(ui.ctx());
        self.toasts(ui.ctx());
    }
}

/// The title strip: brand mark on the left, 8px from the window edge, then
/// an 8px gap, then "Selora IDE" in `--type-xl` / `--weight-semibold`
/// (UI_SPEC.md §1.1). The OS draws the native window chrome above this.
fn title_strip(ui: &mut egui::Ui) {
    egui::Panel::top("title_strip")
        .exact_size(TITLE_BAR_HEIGHT)
        .show_separator_line(false)
        .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add_space(EDGE_PADDING);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(MARK_SIZE, MARK_SIZE), egui::Sense::hover());
                brand::paint_mark(ui.painter(), rect);
                ui.add_space(TITLE_GAP);
                let title = egui::RichText::new("Selora IDE")
                    .color(theme::color::FG_DEFAULT)
                    .font(egui::FontId::new(
                        theme::type_scale::XL,
                        theme::font::family_sans_semibold(),
                    ));
                ui.add(egui::Label::new(title));
            });
        });
}

/// Whether edit-capable keyboard input arrived this frame: printable text or
/// the destructive keys. (Arrow navigation is deliberately excluded — moving
/// the caret does not dirty a buffer.)
fn edit_typed_this_frame(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        input.events.iter().any(|event| match event {
            egui::Event::Text(_) => true,
            egui::Event::Key {
                key, pressed: true, ..
            } => matches!(
                key,
                egui::Key::Backspace | egui::Key::Delete | egui::Key::Enter | egui::Key::Tab
            ),
            _ => false,
        })
    })
}

/// The display name of the active highlighting language (status bar).
fn language_name(language: Option<Language>) -> &'static str {
    match language {
        None => "Plain Text",
        Some(Language::Rust) => "Rust",
        Some(Language::TypeScript) => "TypeScript",
        Some(Language::Tsx) => "TSX",
        Some(Language::JavaScript) => "JavaScript",
        Some(Language::Python) => "Python",
        Some(Language::Json) => "JSON",
        Some(Language::Toml) => "TOML",
        Some(Language::Markdown) => "Markdown",
        Some(Language::Html) => "HTML",
        Some(Language::Css) => "CSS",
    }
}

/// A keybind chip for the empty-state hint (UI_SPEC.md §12): `--ink-mid`
/// background, radius-1, `--type-xs` mono.
fn keybind_chip(ui: &mut egui::Ui, key: &str) {
    egui::Frame::default()
        .fill(theme::color::INK_MID)
        .corner_radius(theme::radius::R1 as u8)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(key)
                    .color(theme::color::FG_DEFAULT)
                    .font(egui::FontId::new(
                        theme::type_scale::XS,
                        theme::font::FAMILY_MONO,
                    )),
            );
        });
}

// ---------------------------------------------------------------------------
// The keybind routing table (UI_SPEC.md §15)
// ---------------------------------------------------------------------------

/// A global keybind action the shell knows how to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    FuzzyFind,
    CommandPalette,
    Save,
    ToggleChat,
    ToggleFileTree,
    SplitEditor,
    CloseEditor,
    OpenSettings,
    OpenFile,
    /// `Ctrl+J` — toggle region ④, the bottom panel (UI_SPEC.md §15).
    TogglePanel,
    /// `Ctrl+Shift+F` — search in files (ROADMAP.md Phase 3 acceptance).
    SearchInFiles,
    /// `F12` — go to definition (UI_SPEC.md §15; Phase 3a).
    GoToDefinition,
    /// `Ctrl+Space` — completions (Phase 3a).
    Completions,
    /// `Ctrl+K Ctrl+I` — hover (a two-stroke chord; Phase 3a).
    Hover,
}

/// The fixed Phase 2 bindings (parsed once). The rebindable four live in
/// `settings.keybinds` and are checked first, so user rebinds win.
fn fixed_keybinds() -> &'static [(Keybind, KeyAction)] {
    static FIXED: std::sync::OnceLock<Vec<(Keybind, KeyAction)>> = std::sync::OnceLock::new();
    FIXED.get_or_init(|| {
        let ctrl = |key: egui::Key, action: KeyAction| {
            (
                Keybind {
                    ctrl: true,
                    alt: false,
                    shift: false,
                    key,
                },
                action,
            )
        };
        let ctrl_shift = |key: egui::Key, action: KeyAction| {
            (
                Keybind {
                    ctrl: true,
                    alt: false,
                    shift: true,
                    key,
                },
                action,
            )
        };
        let bare = |key: egui::Key, action: KeyAction| {
            (
                Keybind {
                    ctrl: false,
                    alt: false,
                    shift: false,
                    key,
                },
                action,
            )
        };
        vec![
            ctrl(egui::Key::B, KeyAction::ToggleFileTree),
            ctrl(egui::Key::Backslash, KeyAction::SplitEditor),
            ctrl(egui::Key::W, KeyAction::CloseEditor),
            ctrl(egui::Key::Comma, KeyAction::OpenSettings),
            ctrl(egui::Key::O, KeyAction::OpenFile),
            ctrl(egui::Key::J, KeyAction::TogglePanel),
            ctrl_shift(egui::Key::F, KeyAction::SearchInFiles),
            bare(egui::Key::F12, KeyAction::GoToDefinition),
            ctrl(egui::Key::Space, KeyAction::Completions),
        ]
    })
}

/// Route one keyboard shortcut to an action: the user-rebindable entries
/// from `settings.keybinds` (UI_SPEC.md §15, Settings → Keybinds), then the
/// fixed Phase 2 table.
fn key_action(keybinds: &KeybindsSettings, shortcut: &egui::KeyboardShortcut) -> Option<KeyAction> {
    let rebindable = [
        (&keybinds.fuzzy_find, KeyAction::FuzzyFind),
        (&keybinds.command_palette, KeyAction::CommandPalette),
        (&keybinds.save, KeyAction::Save),
        (&keybinds.toggle_chat, KeyAction::ToggleChat),
    ];
    for (binding, action) in rebindable {
        if let Some(keybind) = Keybind::parse(binding) {
            if keybind.matches(shortcut) {
                return Some(action);
            }
        }
    }
    fixed_keybinds()
        .iter()
        .find(|(keybind, _)| keybind.matches(shortcut))
        .map(|(_, action)| *action)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use selora_fs::tree::FileTreeNode;
    use selora_settings::Settings;

    fn shortcut(ctrl: bool, shift: bool, alt: bool, key: egui::Key) -> egui::KeyboardShortcut {
        egui::KeyboardShortcut::new(
            egui::Modifiers {
                alt,
                ctrl,
                shift,
                mac_cmd: false,
                command: ctrl, // egui on Windows sets command = ctrl
            },
            key,
        )
    }

    /// A small tree fixture: /repo { src/{main.rs, ui.rs}, Cargo.toml }.
    fn fixture_tree() -> FileTreeNode {
        fn node(path: &str, is_dir: bool, children: Vec<FileTreeNode>) -> FileTreeNode {
            FileTreeNode {
                name: std::path::Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_owned()),
                path: PathBuf::from(path),
                is_dir,
                children,
                truncated: false,
            }
        }
        node(
            "/repo",
            true,
            vec![
                node(
                    "/repo/src",
                    true,
                    vec![
                        node("/repo/src/main.rs", false, vec![]),
                        node("/repo/src/ui.rs", false, vec![]),
                    ],
                ),
                node("/repo/Cargo.toml", false, vec![]),
            ],
        )
    }

    /// The whole workbench, headless: boot → open folder → open file →
    /// edit (dirty) → split → every overlay, one production frame body per
    /// step, with the bundled fonts so text layout is real. Nothing may
    /// panic and nothing may silently toast.
    #[tokio::test]
    async fn workbench_paints_boot_tree_editor_split_and_overlays_headlessly() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);

        let (msg_tx, msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        let mut app = SeloraApp::shell(
            ctx.clone(),
            Handle::current(),
            msg_tx,
            msg_rx,
            effect_tx,
            effect_rx,
        );
        // Isolated from the real %APPDATA%: no session restore, no writes.
        app.workspace_dir = None;
        app.settings_loaded = true;

        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let frame = |app: &mut SeloraApp| {
            ctx.run_ui(raw.clone(), |ui| app.ui_body(ui))
                .drop_without_applying_deltas();
        };

        // Boot with no folder: the "No folder open" empty state paints.
        frame(&mut app);
        frame(&mut app);
        assert!(app.state.workspace.is_none());
        assert!(app.state.toasts.is_empty());

        // Open Folder (worker messages): the tree and fuzzy source install.
        app.state
            .apply(AppMessage::FolderOpened(PathBuf::from("/repo")));
        app.state.apply(AppMessage::FileTreeUpdated {
            root: PathBuf::from("/repo"),
            tree: fixture_tree(),
        });
        frame(&mut app);
        assert!(app.state.file_tree.tree().is_some());
        assert_eq!(app.state.fuzzy_source.len(), 3);

        // Open a file: the tab opens, and the first paint creates the pane's
        // editor with Rust highlighting selected by extension.
        let main_rs = PathBuf::from("/repo/src/main.rs");
        app.state.apply(AppMessage::BufferLoaded {
            path: main_rs.clone(),
            text: "fn main() {}\n".to_owned(),
        });
        frame(&mut app);
        let pane = app.state.editor_area.active_pane_id();
        let editor = app
            .state
            .editors
            .get(&(pane, main_rs.clone()))
            .expect("the pane's editor was created on first paint");
        assert_eq!(editor.language(), Some(Language::Rust));
        assert!(!app.state.is_dirty(&main_rs));

        // Edit the shared buffer: the next paint's shape check dirties the tab.
        {
            let buffer = app
                .state
                .buffers
                .get(&main_rs)
                .cloned()
                .expect("shared buffer");
            buffer
                .write()
                .expect("lock")
                .insert(selora_core::Position::new(1, 0), "// edited\n");
        }
        frame(&mut app);
        assert!(app.state.is_dirty(&main_rs));

        // Ctrl+\: two panes, both with the tab, both painted.
        app.state.split_editor();
        frame(&mut app);
        assert_eq!(app.state.editor_area.pane_ids().len(), 2);
        for pane in app.state.editor_area.pane_ids() {
            assert!(app.state.editors.contains_key(&(pane, main_rs.clone())));
        }

        // Every overlay paints (and stays open — no input in a headless
        // frame dismisses anything).
        app.state.palette.show();
        frame(&mut app);
        assert!(app.state.palette.is_open());
        app.state.palette.hide();

        app.state.fuzzy.show();
        frame(&mut app);
        assert!(app.state.fuzzy.is_open());
        app.state.fuzzy.hide();

        app.settings_dialog = Some(SettingsDialog::new(Settings::default()));
        frame(&mut app);
        app.settings_dialog = None;

        // The git activity takes over region ④: the bottom panel paints the
        // status snapshot's rows, and the tree keeps its badges (Phase 3b).
        app.activity_bar.selected = Activity::Git;
        app.state
            .apply(AppMessage::GitStatusChanged(selora_git::GitStatus {
                branch: Some("main".into()),
                entries: vec![selora_git::FileStatus {
                    abs_path: main_rs.clone(),
                    path: PathBuf::from("src/main.rs"),
                    staged: selora_git::Slot::Unmodified,
                    worktree: selora_git::Slot::Modified,
                }],
            }));
        frame(&mut app);
        assert!(app.state.git_status.is_some());
        assert_eq!(
            app.state
                .file_tree
                .git_decoration(&main_rs)
                .map(|(letter, _)| letter),
            Some('M')
        );
        app.activity_bar.selected = Activity::Files;

        // The search activity takes over region ④ (Phase 3c): the panel
        // paints installed results, and a result click jumps the editor —
        // for an already-open buffer directly, for a new one after load.
        app.activity_bar.selected = Activity::Search;
        app.show_bottom_panel = true;
        app.state.search.set_root(PathBuf::from("/repo"));
        app.state.search.set_query("main");
        let run = app.state.search.submit_query().expect("run event");
        let search_id = match run {
            SearchEvent::RunSearch { id, .. } => id,
            other => panic!("unexpected search event: {other:?}"),
        };
        app.state.apply(AppMessage::SearchResults {
            id: search_id,
            results: Ok(vec![selora_fs::search::SearchHit {
                path: main_rs.clone(),
                line_number: 2,
                line_text: "// edited".into(),
                spans: vec![],
            }]),
        });
        frame(&mut app);
        assert_eq!(app.state.search.groups().len(), 1);

        // Already-open buffer: the jump lands on the next paint.
        app.open_at_line(main_rs.clone(), 2);
        frame(&mut app);
        let pane = app.state.editor_area.active_pane_id();
        let editor = app
            .state
            .editors
            .get(&(pane, main_rs.clone()))
            .expect("editor");
        assert_eq!(editor.cursor(), selora_core::Position::new(1, 0));

        // Not-yet-open file: queued, then applied when the buffer loads and
        // the pane's editor is created (Phase 3c's pending-goto seam).
        let ui_rs = PathBuf::from("/repo/src/ui.rs");
        app.open_at_line(ui_rs.clone(), 1);
        assert!(app
            .goto_after_load
            .front()
            .is_some_and(|(path, _)| path == &ui_rs));
        app.state.apply(AppMessage::BufferLoaded {
            path: ui_rs.clone(),
            text: "fn ui() {}\n".to_owned(),
        });
        frame(&mut app);
        assert!(app.goto_after_load.is_empty());
        let pane = app.state.editor_area.active_pane_id();
        let editor = app.state.editors.get(&(pane, ui_rs)).expect("editor");
        assert_eq!(editor.cursor(), selora_core::Position::new(0, 0));
        app.activity_bar.selected = Activity::Files;
        app.show_bottom_panel = false;

        // Painting alone never surfaces an error (PLAN.md §8).
        assert!(app.state.toasts.is_empty());
    }

    /// The Phase 3a document sync, headless: the paint loop sends didOpen
    /// on a tab's first paint, didChange after the buffer's shape moves,
    /// didSave on FileSave, and didClose when the last tab for the buffer
    /// closes. Settings never "arrive" here, so the LSP manager never
    /// starts and the shell keeps the manager's effect receiver — the test
    /// observes the dispatcher's forwarded stream directly (the dispatcher
    /// task gets the runtime's turns via the yields).
    #[tokio::test]
    async fn lsp_document_sync_flows_open_change_save_close_from_the_shell() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);

        let (msg_tx, msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        let mut app = SeloraApp::shell(
            ctx.clone(),
            Handle::current(),
            msg_tx,
            msg_rx,
            effect_tx,
            effect_rx,
        );
        app.workspace_dir = None;

        // A real (unique) file so FileSave's worker actually succeeds.
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let main_rs =
            std::env::temp_dir().join(format!("selora-app-shell-lsp-{}-{n}", std::process::id()));
        std::fs::write(&main_rs, "fn main() {}\n").expect("write temp file");

        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let frame = |app: &mut SeloraApp| {
            ctx.run_ui(raw.clone(), |ui| app.ui_body(ui))
                .drop_without_applying_deltas();
        };

        // Open the file: the first paint is the didOpen.
        app.state
            .apply(AppMessage::FolderOpened(std::env::temp_dir()));
        app.state.apply(AppMessage::BufferLoaded {
            path: main_rs.clone(),
            text: "fn main() {}\n".to_owned(),
        });
        frame(&mut app);

        // Edit the shared buffer: the next paint's shape check sends
        // didChange with the full new text.
        {
            let buffer = app
                .state
                .buffers
                .get(&main_rs)
                .cloned()
                .expect("shared buffer");
            buffer
                .write()
                .expect("lock")
                .insert(selora_core::Position::new(1, 0), "// edited\n");
        }
        frame(&mut app);

        // Ctrl+S and closing the tab complete the lifecycle.
        app.run_command(CommandId::FileSave);
        app.run_command(CommandId::FileCloseEditor);

        // Give the dispatcher task turns to forward the LSP effects.
        let mut effects = Vec::new();
        for _ in 0..32 {
            tokio::task::yield_now().await;
            if let Some(receiver) = app.lsp_effect_rx.as_mut() {
                while let Ok(effect) = receiver.try_recv() {
                    effects.push(effect);
                }
            }
        }
        let _ = std::fs::remove_file(&main_rs);

        assert_eq!(effects.len(), 4, "effects: {effects:?}");
        assert!(
            matches!(
                &effects[0],
                selora_lsp::LspEffect::Open { path, text }
                    if path == &main_rs && text == "fn main() {}\n"
            ),
            "effects: {effects:?}"
        );
        assert!(
            matches!(
                &effects[1],
                selora_lsp::LspEffect::Change { path, text }
                    if path == &main_rs && text == "fn main() {}\n// edited\n"
            ),
            "effects: {effects:?}"
        );
        assert!(
            matches!(
                &effects[2],
                selora_lsp::LspEffect::Save { path } if path == &main_rs
            ),
            "effects: {effects:?}"
        );
        assert!(
            matches!(
                &effects[3],
                selora_lsp::LspEffect::Close { path } if path == &main_rs
            ),
            "effects: {effects:?}"
        );
    }

    /// The UI_SPEC.md §15 routing table, over the default settings: every
    /// default binding routes to its action, and nothing else does.
    #[test]
    fn keybind_routing_table_matches_the_defaults() {
        let keybinds = KeybindsSettings::default();

        let cases = [
            (KeyAction::FuzzyFind, true, false, false, egui::Key::P),
            (KeyAction::CommandPalette, true, true, false, egui::Key::P),
            (KeyAction::Save, true, false, false, egui::Key::S),
            (KeyAction::ToggleChat, true, false, false, egui::Key::L),
            (KeyAction::ToggleFileTree, true, false, false, egui::Key::B),
            (
                KeyAction::SplitEditor,
                true,
                false,
                false,
                egui::Key::Backslash,
            ),
            (KeyAction::CloseEditor, true, false, false, egui::Key::W),
            (
                KeyAction::OpenSettings,
                true,
                false,
                false,
                egui::Key::Comma,
            ),
            (KeyAction::OpenFile, true, false, false, egui::Key::O),
            (KeyAction::TogglePanel, true, false, false, egui::Key::J),
            (KeyAction::SearchInFiles, true, true, false, egui::Key::F),
        ];
        for (expected, ctrl, shift, alt, key) in cases {
            let got = key_action(&keybinds, &shortcut(ctrl, shift, alt, key));
            assert_eq!(
                got,
                Some(expected),
                "shortcut ctrl={ctrl} shift={shift} alt={alt} {key:?} routed to {got:?}, expected {expected:?}"
            );
        }

        // Bare keys and wrong modifiers route to nothing.
        for (ctrl, shift, alt, key) in [
            (false, false, false, egui::Key::P),
            (false, false, false, egui::Key::S),
            (false, false, false, egui::Key::B),
            (true, true, false, egui::Key::S),
            (true, false, true, egui::Key::P),
            (false, true, false, egui::Key::O),
        ] {
            assert_eq!(
                key_action(&keybinds, &shortcut(ctrl, shift, alt, key)),
                None,
                "unexpected action for ctrl={ctrl} shift={shift} alt={alt} {key:?}"
            );
        }
    }

    /// User rebinds (Settings → Keybinds) win over the defaults: a remapped
    /// fuzzy finder routes to FuzzyFind on the new key, and the old default
    /// goes quiet (Ctrl+P now matches nothing rebindable — the fixed table
    /// has no Ctrl+P either).
    #[test]
    fn user_rebinds_win_over_the_default_table() {
        let keybinds = KeybindsSettings {
            fuzzy_find: "Ctrl+K".into(),
            command_palette: "Ctrl+Alt+P".into(),
            ..KeybindsSettings::default()
        };

        assert_eq!(
            key_action(&keybinds, &shortcut(true, false, false, egui::Key::K)),
            Some(KeyAction::FuzzyFind)
        );
        assert_eq!(
            key_action(&keybinds, &shortcut(true, true, false, egui::Key::P)),
            None
        );
        // Ctrl+Shift+P no longer opens the palette; the new binding does.
        assert_eq!(
            key_action(&keybinds, &shortcut(true, false, true, egui::Key::P)),
            Some(KeyAction::CommandPalette)
        );
        // Unparsable bindings are skipped, not fatal.
        let garbage = KeybindsSettings {
            save: "not a keybind".into(),
            ..KeybindsSettings::default()
        };
        assert_eq!(
            key_action(&garbage, &shortcut(true, false, false, egui::Key::S)),
            None
        );
    }

    /// The edit signal used for dirty derivation: text and destructive keys
    /// count; arrows and releases do not.
    #[test]
    fn edit_signal_classifies_input_events() {
        let ctx = egui::Context::default();
        let run = |events: Vec<egui::Event>| {
            ctx.input_mut(|input| input.events = events);
            edit_typed_this_frame(&ctx)
        };
        assert!(run(vec![egui::Event::Text("x".into())]));
        assert!(run(vec![egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]));
        assert!(!run(vec![egui::Event::Key {
            key: egui::Key::ArrowLeft,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]));
        assert!(!run(vec![egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Default::default(),
        }]));
        assert!(!run(vec![egui::Event::PointerMoved(egui::Pos2::ZERO)]));
    }

    #[test]
    fn language_names_are_stable() {
        assert_eq!(language_name(None), "Plain Text");
        assert_eq!(language_name(Some(Language::Rust)), "Rust");
        assert_eq!(language_name(Some(Language::Markdown)), "Markdown");
    }
}
