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
use selora_editor::Editor;
use selora_settings::{Keybind, KeybindsSettings};
use selora_ui::dialogs::about::{AboutDialog, AboutEvent};
use selora_ui::dialogs::settings::{SettingsDialog, SettingsEvent};
use selora_ui::editor::split::{self, PaneView};
use selora_ui::palette::registry::CommandId;
use selora_ui::panel::git::GitAction;
use selora_ui::panel::search::SearchEvent;
use selora_ui::presence::orb::OrbState;
use selora_ui::sidebar::{activity_bar, Activity, ActivityBar, ChatPanelEvent, TreeEvent};
use selora_ui::splash::{Splash, SplashEvent};
use selora_ui::status_bar::{self, workbench_zone};
use selora_ui::{empty_state, theme, title_bar, toasts};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::app_state::{AppMessage, AppState, BufferShape, Effect};
use crate::persistence::{self, WorkspaceState};
use crate::wiring;
use selora_ui::toasts::Severity;

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
/// How long the first stroke of `Ctrl+K …` stays armed (UI_SPEC.md §15
/// chords).
const CHORD_WINDOW: Duration = Duration::from_millis(500);
/// The Selora connection ping cadence (Phase 4f, ROADMAP.md: "live ping
/// every 30s").
const SELORA_PING_INTERVAL: Duration = Duration::from_secs(30);
/// The ghost-text idle threshold (Phase 4d, ROADMAP.md: "debounce 350ms
/// idle") — seconds on the egui frame clock.
const GHOST_IDLE_SECS: f64 = 0.350;
/// Chat sidebar bounds (UI_SPEC.md §2 region ③): 360px default,
/// 240–600 resizable.
const CHAT_MIN_WIDTH: f32 = 240.0;
const CHAT_MAX_WIDTH: f32 = 600.0;

/// The first-run splash's lifecycle (Phase 6a, UI_SPEC.md §7). `Pending`
/// until the boot keyring read resolves ([`AppMessage::NoStoredKey`] shows
/// it; a `Connected`/`Error` status — a key exists — dismisses it before it
/// ever paints); `Dismissed` for the rest of the run after Skip or the
/// post-Connect fade.
enum SplashState {
    Pending,
    Showing(Splash),
    Dismissed,
}

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
    /// Whether the chat sidebar (region ③) is visible (`Ctrl+L`,
    /// UI_SPEC.md §2 / §15; Phase 4c). Restored from workspace state.
    show_chat: bool,
    /// Settings have arrived (the tree panel's initial width is taken from
    /// them, so the panel first shows *after* the load resolves).
    settings_loaded: bool,
    /// The Settings dialog, while open (UI_SPEC.md §6).
    settings_dialog: Option<SettingsDialog>,
    /// The About dialog, while open (Phase 6b, UI_SPEC.md §11).
    about_dialog: Option<AboutDialog>,
    /// Whether a "Check for updates" request is in flight (the About
    /// dialog's button goes display-only until [`AppMessage::UpdateChecked`]
    /// lands).
    update_check_in_flight: bool,
    /// Where `workspace.json` is persisted; `None` = no APPDATA (headless).
    workspace_dir: Option<PathBuf>,
    /// When `workspace.json` was last written (the 5s autosave clock).
    last_workspace_save: Instant,
    /// The last persisted snapshot (skip no-op writes).
    last_workspace_snapshot: Option<WorkspaceState>,
    /// The last persisted sessions snapshot (Phase R3; skip no-op writes).
    last_sessions_snapshot: Option<Vec<selora_ui::sidebar::chat::ChatSession>>,
    /// Files to reopen on restore, in tab order. Opens are issued one at a
    /// time — unbounded-channel ordering is only guaranteed per sender, and
    /// tab order *is* the restore contract (ARCHITECTURE.md §8.2).
    restore_queue: VecDeque<PathBuf>,
    /// The restore open currently in flight.
    restore_in_flight: Option<PathBuf>,
    /// The file-tree panel width as of the previous frame (for persistence).
    last_tree_width: Option<f32>,
    /// The first-run splash (Phase 6a).
    splash: SplashState,
    /// Phase R6b: the open read-only git diff (the path + unified text);
    /// the workbench overlay over the editor area.
    git_diff: Option<(std::path::PathBuf, String)>,
    /// Phase R7: the keyboard cheat sheet (the "Show Keyboard Shortcuts"
    /// palette command).
    shortcuts_open: bool,
    /// Phase R6c: the find bar over the active pane.
    find_bar: selora_ui::editor::find_bar::FindBar,
    /// The (query, path) last applied to the active editor — re-applied
    /// when either moves.
    find_applied: Option<(String, PathBuf)>,
    /// Phase R4: the status-bar orb's reserved-arm signals — set when the
    /// effect leaves, cleared when the reply lands.
    ghost_in_flight: bool,
    search_in_flight: bool,
    tree_in_flight: bool,
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
        app.restore_session();
        app
    }

    /// Restore the last session (§8.2): a root that still exists reopens
    /// the workspace and requeues its files in tab order. Split out of
    /// `new` so the headless tests can drive it with a sandboxed
    /// `workspace_dir`.
    fn restore_session(&mut self) {
        if let Some(dir) = self.workspace_dir.clone() {
            // Phase R3: the full session history (timeline + cards),
            // independent of the workspace root. The legacy `chat_turns`
            // cover pre-R3 files (bubbles only) when sessions.json is
            // absent or refuses to load.
            let mut sessions_restored = false;
            if let Some(sessions) = persistence::load_sessions(&dir) {
                self.state.chat_panel.restore_sessions(sessions);
                sessions_restored = true;
            }
            if let Some(saved) = persistence::load(&dir) {
                let saved = persistence::restorable(&saved);
                if let Some(root) = saved.root {
                    self.state.workspace = Some(selora_core::Workspace::new(root.clone()));
                    let _ = self.effect_tx.send(Effect::OpenFolder(root));
                    self.restore_queue = saved.open_files.into();
                }
                // Region ③ restores with the session (Phase 4c): the
                // sidebar's visibility, the agent pill (Phase 5d), and the
                // legacy turns when sessions.json did not cover it.
                self.show_chat = saved.chat_open;
                self.state.chat_panel.set_agent_mode(saved.agent_mode);
                if !sessions_restored {
                    self.state.restore_chat(saved.chat_turns);
                }
            }
        }
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
            about_dialog: None,
            update_check_in_flight: false,
            workspace_dir: persistence::state_dir(),
            last_workspace_save: Instant::now(),
            last_workspace_snapshot: None,
            last_sessions_snapshot: None,
            git_diff: None,
            shortcuts_open: false,
            find_bar: selora_ui::editor::find_bar::FindBar::default(),
            find_applied: None,
            ghost_in_flight: false,
            search_in_flight: false,
            tree_in_flight: false,
            restore_queue: VecDeque::new(),
            restore_in_flight: None,
            last_tree_width: None,
            last_selora_ping: None,
            ghost_last_request: None,
            show_chat: false,
            splash: SplashState::Pending,
        }
    }

    // -----------------------------------------------------------------------
    // Frame stages 1–3: drain, timeouts, derived state
    // -----------------------------------------------------------------------

    /// §2 stage 1: drain every worker message into the AppState.
    fn drain_messages(&mut self) {
        while let Ok(message) = self.msg_rx.try_recv() {
            let is_settings = matches!(message, AppMessage::SettingsLoaded(_));
            // Phase R4: the reserved-arm signals clear when their replies
            // land (the observation lives here, ahead of `apply`).
            let ghost_ready = matches!(message, AppMessage::GhostTextReady { .. });
            // Phase R6b: the diff view's payload.
            let git_diff = match &message {
                AppMessage::GitDiffReady { path, diff } => Some((path.clone(), diff.clone())),
                _ => None,
            };
            let search_ready = matches!(message, AppMessage::SearchResults { .. });
            let tree_ready = matches!(message, AppMessage::FileTreeUpdated { .. });
            // Phase 6a: the boot keyring read resolved empty — the splash's
            // cue to show (only ever sent by the SeloraAutoConnect worker).
            let no_stored_key = matches!(message, AppMessage::NoStoredKey);
            // Phase 6b: the About dialog's button re-arms when the check
            // resolves (the toast is apply's job).
            let update_checked = matches!(message, AppMessage::UpdateChecked(_));
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
            if ghost_ready {
                self.ghost_in_flight = false;
            }
            if let Some(diff) = git_diff {
                self.git_diff = Some(diff);
            }
            if search_ready {
                self.search_in_flight = false;
            }
            if tree_ready {
                self.tree_in_flight = false;
            }
            if is_settings {
                self.settings_loaded = true;
                // DESIGN_TOKENS.md §7.4: the chat cards' pulse/spin follow
                // `ui.reduce_motion`.
                self.state
                    .chat_panel
                    .set_reduce_motion(self.state.settings.ui.reduce_motion);
                // The panel adopts the persisted model at boot (the header
                // and both send paths read the panel, not the settings);
                // a picker choice already made is never clobbered.
                if self.state.chat_panel.model().is_none() {
                    if let Some(model) = self.state.settings.selora.active_model.clone() {
                        self.state.chat_panel.set_model(model);
                    }
                }
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
                self.tree_in_flight = true;
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
            if no_stored_key && matches!(self.splash, SplashState::Pending) {
                // Phase 6a: launch with no key in the keyring → the splash
                // (UI_SPEC.md §7). A keyring *error* never lands here —
                // without a readable store the shell stays silent rather
                // than badgering (the status bar's connect affordance
                // remains).
                let mut splash = Splash::new();
                splash.set_reduce_motion(self.state.settings.ui.reduce_motion);
                self.splash = SplashState::Showing(splash);
            }
            if update_checked {
                self.update_check_in_flight = false;
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
        let snapshot = self.state.workspace_snapshot(self.show_chat);
        // Phase R3: the session history rides the same cadence (its own
        // no-op check — the layout-only snapshot below does not move when
        // a chat does).
        let sessions = self.state.chat_panel.snapshot_sessions();
        if self.last_workspace_snapshot.as_ref() == Some(&snapshot)
            && self.last_sessions_snapshot.as_ref() == Some(&sessions)
        {
            return;
        }
        if let Err(error) = persistence::save(&dir, &snapshot) {
            self.state.toast(
                format!("workspace state not saved: {error}"),
                Severity::Error,
            );
        }
        self.last_workspace_snapshot = Some(snapshot);
        if self.last_sessions_snapshot.as_ref() != Some(&sessions) {
            if let Err(error) = persistence::save_sessions(&dir, &sessions) {
                self.state
                    .toast(format!("sessions not saved: {error}"), Severity::Error);
            }
        }
        self.last_sessions_snapshot = Some(sessions);
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
            rust_analyzer_args: self.state.settings.lsp.rust_analyzer_args.clone(),
            typescript_language_server_args: self.state.settings.lsp.typescript_server_args.clone(),
            enabled: self.state.settings.lsp.enabled,
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
        self.ghost_in_flight = true;
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

    /// §17.7's LSP half: an agent buffer sync or undo can be a same-length
    /// replacement the shape-compare in `paint_pane` cannot see — drain the
    /// agent-touched paths and push a didChange for every didOpen'd one,
    /// rebasing `lsp_shapes` so the next paint does not re-send.
    fn drain_agent_touched(&mut self) {
        for path in self.state.take_agent_touched() {
            if !self.lsp_shapes.contains_key(&path) {
                continue; // never didOpen'd — no server to tell
            }
            let Some(buffer) = self.state.buffers.get(&path).cloned() else {
                continue;
            };
            let buffer = buffer.read().unwrap_or_else(|e| e.into_inner());
            let text = buffer.text();
            self.lsp_shapes
                .insert(path.clone(), BufferShape::of(&buffer));
            drop(buffer);
            let _ = self
                .effect_tx
                .send(Effect::Lsp(selora_lsp::LspEffect::Change { path, text }));
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
            || self.about_dialog.is_some()
            || self.shortcuts_open
            || matches!(self.splash, SplashState::Showing(_))
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
            KeyAction::ToggleChat => self.toggle_chat(),
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
            KeyAction::FindInFile => self.find_bar.open_for_find(),
            KeyAction::ReplaceInFile => self.find_bar.open_for_replace(),
            KeyAction::UndoAgentEdit => {
                // §17.7's Ctrl+Z: revert the newest agent edit on the
                // active buffer (no-op when the buffer moved on or no
                // agent edit touched it — never a clobber).
                if let Some(path) = self.state.active_path() {
                    self.state.undo_agent_edit(&path);
                }
            }
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
            && !matches!(
                self.activity_bar.selected,
                Activity::Git | Activity::Search | Activity::Problems
            )
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
                let problems = self.state.problems_count();
                if let Some(activity) = self.activity_bar.show_with_problems(ui, problems) {
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
            Activity::Problems => {
                // The diagnostics are already in state — the panel just
                // shows (Phase R6).
                self.show_bottom_panel = true;
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
            Activity::Chat => self.toggle_chat(),
        }
    }

    /// Show / hide region ③ (Phase 4c). Opening focuses the composer so
    /// typing starts immediately (UI_SPEC.md §4).
    fn toggle_chat(&mut self) {
        self.show_chat = !self.show_chat;
        if self.show_chat {
            self.state.chat_panel.focus_composer();
        }
    }

    fn route_tree_event(&mut self, event: TreeEvent) {
        match event {
            TreeEvent::OpenFile(path) => self.open_path(path),
            TreeEvent::TreeChanged => {
                if let Some(root) = self.state.workspace.as_ref().map(|w| w.root.clone()) {
                    self.tree_in_flight = true;
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
        if !self.show_bottom_panel
            || !matches!(
                selected,
                Activity::Git | Activity::Search | Activity::Problems
            )
        {
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
                Activity::Problems => {
                    let diagnostics = self.state.lsp.diagnostics.clone();
                    for event in selora_ui::panel::problems::show(ui, &diagnostics) {
                        match event {
                            selora_ui::panel::problems::ProblemsEvent::OpenAtLine {
                                path,
                                line,
                            } => self.open_at_line(path, line),
                        }
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
                        self.search_in_flight = true;
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
                // Phase R6b: the read-only diff view (UI_SPEC.md §8's
                // refinement, at last) — the worker diffs the working
                // tree against the index; the overlay paints on the
                // reply. No root open: nothing to diff against.
                if let Some(root) = self.state.workspace.as_ref().map(|w| w.root.clone()) {
                    let _ = self.effect_tx.send(Effect::GitDiff { root, path });
                } else {
                    self.state.toast("open a folder to diff", Severity::Info);
                }
            }
        }
    }

    /// The 24px status bar (UI_SPEC.md §1.2): the workbench zones (left:
    /// git branch + per-server LSP status; center: cursor + language +
    /// progress — the `workbench_zone` widget, Phase R0) and the right
    /// zone (the cost meter + Selora status widgets).
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar")
            .exact_size(status_bar::HEIGHT)
            .resizable(false)
            .show_separator_line(true)
            .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
            .show(ui, |ui| {
                // Left + center zones: a state snapshot in, restart
                // events out (a dead server's run is a click target).
                let pane = self.state.editor_area.active_pane_id();
                let input =
                    workbench_zone::WorkbenchInput {
                        git: self.state.git_status.as_ref().map(|status| {
                            workbench_zone::GitSummary {
                                branch: status.branch.clone().unwrap_or_default(),
                                changed_count: status.changed_count(),
                            }
                        }),
                        workspace_name: self
                            .state
                            .workspace
                            .as_ref()
                            .and_then(|workspace| workspace.root.file_name())
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "no folder".to_owned()),
                        lsp_servers: self
                            .state
                            .lsp
                            .status
                            .iter()
                            .map(|(language, status)| (*language, status.clone()))
                            .collect(),
                        cursor: self
                            .state
                            .active_path()
                            .and_then(|path| self.state.editors.get(&(pane, path.clone())))
                            .map(|editor| {
                                let cursor = editor.cursor();
                                workbench_zone::CursorSummary {
                                    line: cursor.line + 1,
                                    col: cursor.col + 1,
                                    language: workbench_zone::language_name(editor.language()),
                                }
                            }),
                        lsp_progress: self.state.lsp.progress.clone(),
                    };
                for event in workbench_zone::show(ui, &input) {
                    match event {
                        workbench_zone::WorkbenchEvent::RestartServer(language) => {
                            let _ = self
                                .effect_tx
                                .send(Effect::Lsp(selora_lsp::LspEffect::Restart(language)));
                        }
                    }
                }

                // Right: the live Selora connection (Phase 4f) at the very
                // edge, the `Σ today` cost meter (Phase 4e) left of it.
                // The active model from settings wins over the ping's own
                // (the ping cannot see the picker's choice).
                //
                // Phase R4: the reserved-arm orb left of the run —
                // `Shaping` while the tree rebuilds, `Composing` while a
                // search runs, `Weaving` while a ghost completion is in
                // flight (the Phase 6h states' surfaces, wired).
                let rect = ui.max_rect();
                let painter = ui.painter_at(rect);
                let active_model = self.state.settings.selora.active_model.clone();
                let activity = if self.tree_in_flight {
                    Some(OrbState::Shaping)
                } else if self.search_in_flight {
                    Some(OrbState::Composing)
                } else if self.ghost_in_flight {
                    Some(OrbState::Weaving)
                } else {
                    None
                };
                let status_events = selora_ui::status_bar::selora_status::show(
                    ui,
                    &self.state.selora_status,
                    active_model.as_deref(),
                    activity,
                    self.state.settings.ui.reduce_motion,
                );
                // The activity orb animates: keep frames coming while
                // one is live (the presence widgets' discipline — an
                // idle orb breathes at 20ms repaints, bounded).
                if activity.is_some() && !self.state.settings.ui.reduce_motion {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(20));
                }
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
                let activity_zone = if activity.is_some() {
                    selora_ui::status_bar::selora_status::ACTIVITY_ZONE
                } else {
                    0.0
                };
                let meter_zone = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rect.top()),
                    egui::pos2(
                        rect.right()
                            - theme::space::S3
                            - run_width
                            - activity_zone
                            - theme::space::S4,
                        rect.bottom(),
                    ),
                );
                selora_ui::status_bar::cost_meter::CostMeter::new(&self.state.cost_meter)
                    .show(ui, meter_zone);
                for event in status_events {
                    match event {
                        selora_ui::status_bar::selora_status::StatusEvent::OpenConnect => {
                            self.open_settings_to_selora();
                        }
                    }
                }
            });
    }

    /// Region ③ (Phase 4c): the chat sidebar, a resizable right panel
    /// (UI_SPEC.md §2: 360px default, 240–600). The panel widget owns the
    /// content; its events route to effects here (sends become
    /// `Effect::SeloraChat`, keyed by the panel's ChatId).
    fn chat_region(&mut self, ui: &mut egui::Ui) {
        if !self.show_chat {
            return;
        }
        let default_width = self.state.settings.ui.chat_sidebar_width as f32;
        let mut events = Vec::new();
        egui::Panel::right("chat_sidebar")
            .resizable(true)
            .default_size(default_width)
            .min_size(CHAT_MIN_WIDTH)
            .max_size(CHAT_MAX_WIDTH)
            .show_separator_line(true)
            .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
            .show(ui, |ui| {
                let outcome = self.state.chat_panel.show(ui);
                events = outcome.events;
            });
        for event in events {
            self.route_chat_event(event);
        }
    }

    /// One chat-panel event → its effect (Phase 4c).
    fn route_chat_event(&mut self, event: ChatPanelEvent) {
        match event {
            ChatPanelEvent::Send { id, .. } => {
                // The user bubble is already in the transcript; the
                // request carries the whole (stable) conversation.
                let Some(model) = self.state.chat_panel.model().cloned() else {
                    self.state
                        .toast("pick a model first — the header's picker", Severity::Info);
                    return;
                };
                let conversation = self.state.chat_panel.conversation();
                let request = selora_ai::ChatRequest::new(model, conversation).with_stream(true);
                let chat_uuid = uuid::Uuid::new_v4();
                self.state.chat_ids.insert(chat_uuid, id);
                let _ = self.effect_tx.send(Effect::SeloraChat(chat_uuid, request));
            }
            ChatPanelEvent::AgentSend { id, .. } => {
                // The agent twin of Send (§17.3): same sources, plus the
                // resolved catalog model and the `agent.*` settings — the
                // dispatcher builds the loop from exactly this payload.
                let Some(model_id) = self.state.chat_panel.model().cloned() else {
                    self.state
                        .toast("pick a model first — the header's picker", Severity::Info);
                    return;
                };
                let model = self
                    .state
                    .models
                    .iter()
                    .find(|model| model.id == model_id)
                    .cloned()
                    .unwrap_or_else(|| selora_ai::Model {
                        // Not in the catalog (a stale settings id): no
                        // context window → no context cap, never a false
                        // abort (turn.rs's context_budget_tokens).
                        id: model_id.clone(),
                        display_name: model_id,
                        context_window: 0,
                        price_per_1k_input: 0.0,
                        price_per_1k_output: 0.0,
                        supports_streaming: true,
                        supports_tools: true,
                        family: String::new(),
                    });
                let conversation = self.state.chat_panel.conversation();
                // Phase R5: the Brave key's home is the OS keyring; the
                // settings field stays empty (legacy files may carry a
                // plaintext one — it wins only if the keyring is empty).
                let mut agent = self.state.settings.agent.clone();
                if agent
                    .brave_search_api_key
                    .as_deref()
                    .is_none_or(str::is_empty)
                {
                    if let Ok(Some(key)) = selora_settings::keyring::brave_key_get() {
                        agent.brave_search_api_key = Some(key);
                    }
                }
                let request = crate::app_state::AgentTurnRequest {
                    model,
                    conversation,
                    agent,
                };
                let chat_uuid = uuid::Uuid::new_v4();
                self.state.chat_ids.insert(chat_uuid, id);
                let _ = self.effect_tx.send(Effect::AgentSend(chat_uuid, request));
            }
            ChatPanelEvent::AgentDecision {
                id,
                approval_id,
                decision,
            } => {
                // Reverse-map the panel's ChatId to the turn's request uuid
                // (one in-flight turn at a time — unambiguous). The panel's
                // Clear-during-a-turn Stop flows through this same arm; the
                // dispatcher drops unknown/ended turns silently.
                let uuid = self
                    .state
                    .chat_ids
                    .iter()
                    .find(|(_, chat_id)| **chat_id == id)
                    .map(|(uuid, _)| *uuid);
                if let Some(uuid) = uuid {
                    let _ = self
                        .effect_tx
                        .send(Effect::AgentDecision(uuid, approval_id, decision));
                }
            }
            ChatPanelEvent::StopStream { id } => {
                // The panel already finalized the streaming bubble; this
                // cancels the request itself (Phase R1): the reverse map
                // gives the dispatcher's uuid, the cancel closes the SSE
                // connection, and the map entry goes now — the aborted
                // stream never sends a terminal message.
                let uuid = self
                    .state
                    .chat_ids
                    .iter()
                    .find(|(_, chat_id)| **chat_id == id)
                    .map(|(uuid, _)| *uuid);
                if let Some(uuid) = uuid {
                    self.state.chat_ids.remove(&uuid);
                    let _ = self.effect_tx.send(Effect::SeloraChatAbort(uuid));
                }
            }
            // A card body's "open ↗" link (Phase R4): the path is
            // workspace-relative — resolve it against the root, then the
            // shared open-and-jump (opening the buffer first when it is
            // not loaded).
            ChatPanelEvent::OpenFileAt { path, line } => {
                let resolved = if path.is_absolute() {
                    path
                } else {
                    self.state
                        .workspace
                        .as_ref()
                        .map(|workspace| workspace.root.join(&path))
                        .unwrap_or(path)
                };
                self.open_at_line(resolved, line);
            }
            // A session switch/new was refused while a turn runs
            // (Phase R3).
            ChatPanelEvent::SessionBlocked => {
                self.state
                    .toast("finish or stop the current turn first", Severity::Info);
            }
            // The panel emptied the transcript itself; an in-flight stream
            // (if any) lands into a stale id and is dropped.
            ChatPanelEvent::Clear => {}
            // The text is already on the clipboard (the panel put it
            // there); these are the toast hooks.
            // Code blocks and whole messages both flow through here
            // (Phase R2 added the message-level Copy).
            ChatPanelEvent::CopyCode(_) => self.state.toast("copied to clipboard", Severity::Info),
            ChatPanelEvent::CopyTranscript(_) => {
                self.state.toast("transcript copied", Severity::Info)
            }
            ChatPanelEvent::SelectModel(model) => {
                // The picker flipped its header optimistically; this is
                // the durable write (settings.json + the dispatcher's
                // next request both follow).
                self.state.settings.selora.active_model = Some(model);
                let _ = self
                    .effect_tx
                    .send(Effect::SettingsSave(self.state.settings.clone()));
            }
            ChatPanelEvent::RefreshModels => {
                let _ = self.effect_tx.send(Effect::SeloraListModels);
            }
        }
    }

    /// The central editor area: the split tree with its tab bars, the
    /// per-pane editors, the empty states, and the fuzzy-file overlay.
    fn editor_region(&mut self, ui: &mut egui::Ui, workbench: &mut egui::Rect) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
            .show(ui, |ui| {
                *workbench = ui.max_rect();
                if self.state.workspace.is_none() {
                    match empty_state::no_folder(ui) {
                        Some(empty_state::EmptyStateEvent::OpenFolder) => self.pick_folder(),
                        Some(empty_state::EmptyStateEvent::CloneRepo) => {
                            self.state
                                .toast("Clone Repo is coming soon", Severity::Info);
                        }
                        None => {}
                    }
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

                // The read-only git diff overlay (Phase R6b): covers the
                // editor area — a header (path · working tree, close ×)
                // and the shared DiffView in a scroll. Esc closes.
                if let Some((path, diff)) = self.git_diff.clone() {
                    let rect = ui.max_rect();
                    ui.painter()
                        .rect_filled(rect, 0.0, theme::color::INK_CANVAS);
                    // A one-line header + the scroll body.
                    let header =
                        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 32.0));
                    ui.painter().hline(
                        header.x_range(),
                        header.bottom(),
                        egui::Stroke::new(1.0, theme::color::INK_HI),
                    );
                    let title = format!(
                        "{} (working tree)",
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.to_string_lossy().into_owned())
                    );
                    ui.painter().text(
                        egui::pos2(header.left() + theme::space::S3, header.center().y),
                        egui::Align2::LEFT_CENTER,
                        title,
                        egui::FontId::new(
                            theme::type_scale::SM,
                            selora_ui::theme::font::family_sans_semibold(),
                        ),
                        theme::color::FG_DEFAULT,
                    );
                    let close = egui::Rect::from_min_size(
                        egui::pos2(header.right() - 32.0, header.top()),
                        egui::vec2(32.0, 32.0),
                    );
                    let close_response = ui.interact(
                        close,
                        egui::Id::new("selora_git_diff_close"),
                        egui::Sense::click(),
                    );
                    ui.painter().text(
                        close.center(),
                        egui::Align2::CENTER_CENTER,
                        "x",
                        egui::FontId::new(theme::type_scale::SM, theme::font::FAMILY_SANS),
                        if close_response.hovered() {
                            theme::color::FG_DEFAULT
                        } else {
                            theme::color::FG_MUTED
                        },
                    );
                    if close_response.clicked()
                        || ui.input(|input| input.key_pressed(egui::Key::Escape))
                    {
                        self.git_diff = None;
                    }
                    // The body: parse + paint through the shared widget.
                    let lines = selora_ui::diff_view::parse_unified_diff(&diff);
                    let language = selora_ui::diff_view::language_for_diff(&lines);
                    let view = selora_ui::diff_view::DiffView::new(&lines, language);
                    let body = egui::Rect::from_min_max(
                        egui::pos2(rect.left(), header.bottom()),
                        rect.max,
                    );
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(body.shrink2(egui::vec2(0.0, theme::space::S2))),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("selora_git_diff_body")
                        .auto_shrink([false, false])
                        .show(&mut child, |ui| {
                            view.paint(ui, ui.available_width(), 1.0);
                        });
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
            let fuzzy_key = self.state.settings.keybinds.fuzzy_find.clone();
            let palette_key = self.state.settings.keybinds.command_palette.clone();
            empty_state::pane_no_tabs(ui, view.content, &fuzzy_key, &palette_key);
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

        // The find bar (Phase R6c): over the ACTIVE pane's top-right. The
        // query re-applies when it (or the pane's file) moved.
        if self.find_bar.open && view.pane == self.state.editor_area.active_pane_id() {
            let applied = (self.find_bar.query.clone(), path.clone());
            if self.find_applied.as_ref() != Some(&applied) {
                if let Some(editor) = self.state.editors.get_mut(&(view.pane, path.clone())) {
                    editor.set_find(&self.find_bar.query.clone(), self.find_bar.case_sensitive);
                }
                self.find_applied = Some(applied);
            }
            let (matches, active) = self
                .state
                .editors
                .get(&(view.pane, path.clone()))
                .map(|editor| (editor.find_matches(), editor.find_active()))
                .unwrap_or((0, 0));
            let bar = &mut self.find_bar;
            let events = bar.show(ui, view.content, matches, active);
            for event in events {
                let editor = self.state.editors.get_mut(&(view.pane, path.clone()));
                use selora_ui::editor::find_bar::FindBarEvent;
                match event {
                    FindBarEvent::Query(text) => {
                        if let Some(editor) = editor {
                            editor.set_find(&text, self.find_bar.case_sensitive);
                        }
                    }
                    FindBarEvent::Next => {
                        if let Some(editor) = editor {
                            editor.find_next();
                        }
                    }
                    FindBarEvent::Prev => {
                        if let Some(editor) = editor {
                            editor.find_prev();
                        }
                    }
                    FindBarEvent::Replace(replacement) => {
                        if let Some(editor) = editor {
                            editor.find_replace_current(&replacement);
                        }
                    }
                    FindBarEvent::ReplaceAll(replacement) => {
                        if let Some(editor) = editor {
                            editor.find_replace_all(&replacement);
                        }
                    }
                    FindBarEvent::Close => {
                        if let Some(editor) = editor {
                            editor.clear_find();
                        }
                        self.find_bar.close();
                        self.find_applied = None;
                    }
                }
            }
        }
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

    /// The status bar's "○ selora disconnected · connect" affordance
    /// (UI_SPEC.md §1.2): the click must land on the Connect form, not a
    /// generic settings page — jump to (or open on) the Selora section.
    fn open_settings_to_selora(&mut self) {
        match self.settings_dialog.as_mut() {
            Some(dialog) => dialog.open_selora_section(),
            None => {
                let mut dialog = SettingsDialog::new(self.state.settings.clone());
                dialog.open_selora_section();
                self.settings_dialog = Some(dialog);
            }
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
            // The dialog wrote the file itself; this only refreshes the
            // dispatcher's Selora base URL (without it, every Selora
            // effect keeps the boot-time URL until the app restarts).
            let _ = self.effect_tx.send(Effect::SettingsSynced {
                base_url: settings.selora.base_url.clone(),
            });
            self.state
                .chat_panel
                .set_reduce_motion(settings.ui.reduce_motion);
            self.state.settings = settings;
        }
        if close {
            self.settings_dialog = None;
        }
    }

    /// The About dialog (Phase 6b, UI_SPEC.md §11). The update check rides
    /// the effect channel; the reply toasts (newer tag + "download" link,
    /// up-to-date, or the error's words) and re-arms the button.
    fn about_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.about_dialog.as_mut() else {
            return;
        };
        dialog.set_checking(self.update_check_in_flight);
        let events = dialog.show(ctx);
        let mut close = false;
        for event in events {
            match event {
                AboutEvent::Close => close = true,
                AboutEvent::CheckForUpdates => {
                    self.update_check_in_flight = true;
                    let _ = self.effect_tx.send(Effect::CheckForUpdates);
                }
            }
        }
        if close {
            self.about_dialog = None;
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

    /// Toasts (UI_SPEC.md §13): the stack painter is the `toasts` widget
    /// (Phase R0); this owns the expiry clock and the repaint scheduling.
    fn toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(next) = self.state.expire_toasts(now) {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
        toasts::show(ctx, &mut self.state.toasts);
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
            // Phase 6b: the About dialog (UI_SPEC.md §11).
            CommandId::HelpAbout => {
                if self.about_dialog.is_none() {
                    self.about_dialog = Some(AboutDialog::new(env!("CARGO_PKG_VERSION")));
                }
            }
            CommandId::HelpShowShortcuts => self.shortcuts_open = true,
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
            CommandId::EditFindInFile => self.find_bar.open_for_find(),
            CommandId::EditReplaceInFile => self.find_bar.open_for_replace(),
            CommandId::EditGoToDefinition => {
                self.state
                    .toast("go to definition lands in Phase 3", Severity::Info);
            }
            // Phase 4: the Selora UI.
            CommandId::ViewToggleChatSidebar => self.toggle_chat(),
            CommandId::SeloraSwitchModel => {
                // The palette route: open the sidebar with its picker
                // dropdown armed (Phase 4c).
                if !self.show_chat {
                    self.toggle_chat();
                }
                self.state.chat_panel.open_model_picker();
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

impl SeloraApp {
    /// The cheat sheet's rows (Phase R7): the rebindable four from the
    /// live settings, then the fixed table.
    fn shortcut_rows(&self) -> Vec<selora_ui::dialogs::shortcuts::ShortcutRow> {
        use selora_ui::dialogs::shortcuts::ShortcutRow;
        let keybinds = &self.state.settings.keybinds;
        let mut rows = vec![
            ShortcutRow {
                action: "Find a file".into(),
                binding: keybinds.fuzzy_find.clone(),
            },
            ShortcutRow {
                action: "Command palette".into(),
                binding: keybinds.command_palette.clone(),
            },
            ShortcutRow {
                action: "Save".into(),
                binding: keybinds.save.clone(),
            },
            ShortcutRow {
                action: "Toggle chat sidebar".into(),
                binding: keybinds.toggle_chat.clone(),
            },
        ];
        for (keybind, action) in fixed_keybinds() {
            rows.push(ShortcutRow {
                action: key_action_label(*action).to_owned(),
                binding: keybind.to_string(),
            });
        }
        rows
    }

    /// The cheat sheet overlay (Phase R7). Esc / the backdrop closes (the
    /// Modal owns dismissal); the open flag resets when the dialog is
    /// gone.
    fn shortcuts_overlay(&mut self, ctx: &egui::Context) {
        if !self.shortcuts_open {
            return;
        }
        let rows = self.shortcut_rows();
        if selora_ui::dialogs::shortcuts::show(ctx, &rows) {
            self.shortcuts_open = false;
        }
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
        self.drain_agent_touched();

        // Keybinds run only when no overlay owns the keyboard.
        self.handle_keybinds(ui.ctx());

        // Stages 4–6: layout, chrome, editor. The title strip is the
        // `title_bar` widget (Phase R0); its menus (Phase R6) route to
        // the palette commands.
        for event in title_bar::show(ui).event.into_iter() {
            match event {
                title_bar::TitleBarEvent::Command(id) => self.run_command(id),
                title_bar::TitleBarEvent::Exit => {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        self.status_bar(ui);
        self.bottom_panel(ui);
        self.left_region(ui);
        self.chat_region(ui);
        let mut workbench = egui::Rect::ZERO;
        self.editor_region(ui, &mut workbench);

        // Overlays.
        self.command_palette(ui.ctx(), workbench);
        self.lsp_popups(ui.ctx());
        self.settings_dialog(ui.ctx());
        self.about_dialog(ui.ctx());
        self.shortcuts_overlay(ui.ctx());
        // Phase 6a: the first-run splash covers everything below the title
        // strip; toasts still paint above it (a connect failure must be
        // visible).
        self.splash_cover(ui.ctx());
        self.toasts(ui.ctx());
    }
}

impl SeloraApp {
    /// One splash event's shell route (Phase 6a). `Connect` goes to the
    /// runtime for validation + keyring storage (ARCHITECTURE.md §7.2) with
    /// the status flipped to `Connecting` immediately (the splash paints
    /// it); `Skip`/`Dismissed` close the splash for the run. Returns whether
    /// the splash closed.
    fn route_splash_event(&mut self, event: SplashEvent) -> bool {
        match event {
            SplashEvent::Connect(key) => {
                self.state.selora_status = selora_ai::SeloraStatus::Connecting;
                let _ = self.effect_tx.send(Effect::SeloraConnect(key));
                false
            }
            SplashEvent::Skip | SplashEvent::Dismissed => true,
        }
    }

    /// The first-run splash (Phase 6a, UI_SPEC.md §7): a full-window cover
    /// over the workbench, below the title strip. Painted after every panel
    /// so its opaque fill hides them.
    fn splash_cover(&mut self, ctx: &egui::Context) {
        // A connection status proving a key exists (valid or rejected)
        // resolves the boot wait without ever painting the splash.
        if matches!(self.splash, SplashState::Pending) {
            match &self.state.selora_status {
                selora_ai::SeloraStatus::Connected { .. } | selora_ai::SeloraStatus::Error(_) => {
                    self.splash = SplashState::Dismissed;
                }
                _ => {}
            }
        }
        let SplashState::Showing(splash) = &mut self.splash else {
            return;
        };
        // The widget paints the live connection state (the Settings
        // dialog's contract): Connecting disables, Connected starts the
        // fade-out, Error prints the gateway's words.
        splash.set_status(self.state.selora_status.clone());
        let screen = ctx.content_rect();
        let rect = egui::Rect::from_min_max(
            egui::pos2(screen.min.x, screen.min.y + title_bar::HEIGHT),
            screen.max,
        );
        let events = splash.show(ctx, rect);
        let mut dismiss = false;
        for event in events {
            dismiss |= self.route_splash_event(event);
        }
        if dismiss {
            self.splash = SplashState::Dismissed;
        }
    }
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
    /// `Ctrl+Z` — revert the newest agent edit on the active buffer
    /// (Phase 5d, §17.7: agent-edit-scoped undo; the buffer has no
    /// general undo log yet — human typing is unaffected).
    UndoAgentEdit,
    /// `Ctrl+F` — the find bar (Phase R6c).
    FindInFile,
    /// `Ctrl+H` — the find bar with the replace row (Phase R6c).
    ReplaceInFile,
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
            ctrl(egui::Key::Z, KeyAction::UndoAgentEdit),
            ctrl(egui::Key::F, KeyAction::FindInFile),
            ctrl(egui::Key::H, KeyAction::ReplaceInFile),
        ]
    })
}

/// The cheat sheet's action label for a fixed-table entry (Phase R7).
fn key_action_label(action: KeyAction) -> &'static str {
    match action {
        KeyAction::ToggleFileTree => "Toggle file tree",
        KeyAction::SplitEditor => "Split editor",
        KeyAction::CloseEditor => "Close editor",
        KeyAction::OpenSettings => "Open settings",
        KeyAction::OpenFile => "Open file",
        KeyAction::TogglePanel => "Toggle panel",
        KeyAction::SearchInFiles => "Search in files",
        KeyAction::GoToDefinition => "Go to definition",
        KeyAction::Completions => "Completions",
        KeyAction::Hover => "Hover",
        KeyAction::UndoAgentEdit => "Undo agent edit",
        KeyAction::FindInFile => "Find in file",
        KeyAction::ReplaceInFile => "Replace in file",
        KeyAction::FuzzyFind
        | KeyAction::CommandPalette
        | KeyAction::Save
        | KeyAction::ToggleChat => {
            "" // rebindable — the rows carry them from settings instead
        }
    }
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
    use selora_editor::Language;
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
            // Phase 5d: agent-edit undo (§17.7).
            (KeyAction::UndoAgentEdit, true, false, false, egui::Key::Z),
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

    // --- Agent Mode (Phase 5d) ------------------------------------------------

    use std::time::Duration;

    fn agent_key_event(key: egui::Key) -> egui::RawInput {
        egui::RawInput {
            focused: true,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            ..Default::default()
        }
    }

    fn raw_screen() -> egui::RawInput {
        egui::RawInput {
            focused: true,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        }
    }

    fn agent_frame(ctx: &egui::Context, app: &mut SeloraApp, raw: egui::RawInput) {
        ctx.run_ui(raw, |ui| app.ui_body(ui))
            .drop_without_applying_deltas();
    }

    /// Pump real frames (the message drain lives in ui_body) until
    /// `until` holds, with a deadline so a broken turn fails instead of
    /// hanging.
    async fn pump_until(
        ctx: &egui::Context,
        app: &mut SeloraApp,
        until: impl Fn(&SeloraApp) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            agent_frame(ctx, app, raw_screen());
            if until(app) {
                return;
            }
            assert!(Instant::now() < deadline, "pump timed out");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    fn agent_test_model() -> selora_ai::Model {
        selora_ai::Model {
            id: "test-model".into(),
            display_name: "Test Model".into(),
            context_window: 128_000,
            price_per_1k_input: 0.0,
            price_per_1k_output: 0.0,
            supports_streaming: true,
            supports_tools: true,
            family: "test".into(),
        }
    }

    /// One streamed tool call (the wiring tests' shape).
    fn agent_tool_call_sse(call_id: &str, name: &str, arguments: &str) -> String {
        let escaped = arguments
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        format!(
            "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"{call_id}\",\"type\":\"function\",\"function\":{{\"name\":\"{name}\",\"arguments\":\"\"}}}}]}}}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"function\":{{\"arguments\":\"{escaped}\"}}}}]}}}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\n\
             data: [DONE]\n\n"
        )
    }

    fn agent_answer_sse(text: &str, cost_usd: f64) -> String {
        format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":5,\"cost_usd\":{cost_usd}}}}}\n\n\
             data: [DONE]\n\n"
        )
    }

    fn agent_app(ctx: &egui::Context) -> SeloraApp {
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
        // Isolated from the real %APPDATA%; settings never arrive, so the
        // ping loop and the LSP manager stay off.
        app.workspace_dir = None;
        app
    }

    /// The whole path, headless (the brief's acceptance): composer Enter
    /// with the pill on → Effect::AgentSend → the real loop against the
    /// mock gateway → the diff card parks → the panel-event Approve route
    /// → the router writes the real file → the open buffer syncs CLEAN →
    /// the meter steps → the summary footer lands. Then Ctrl+Z's seam:
    /// undo reverts the buffer (dirty), disk keeps the approved text.
    #[tokio::test]
    async fn agent_mode_end_to_end_approve_syncs_disk_buffer_and_meter() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);

        // A real workspace with a real file.
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("selora-app-shell-agent-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).expect("mkdir");
        let main_rs = root.join("main.rs");
        std::fs::write(&main_rs, "fn main() {}\n").expect("write");

        // The mock gateway: round 1 asks for the edit, round 2 answers.
        let server = wiremock::MockServer::start().await;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains("add a comment"))
            .respond_with(ResponseTemplate::new(200).set_body_string(agent_tool_call_sse(
                "call_edit_1",
                "edit_file",
                r#"{"path":"main.rs","old_str":"fn main() {}","new_str":"// entry point\nfn main() {}"}"#,
            )))
            .with_priority(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains("tool_call_id"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(agent_answer_sse("Done.", 0.0021)),
            )
            .with_priority(1)
            .mount(&server)
            .await;

        let mut app = agent_app(&ctx);
        app.state.models = vec![agent_test_model()];
        app.state.chat_panel.set_model("test-model");
        let _ = app
            .effect_tx
            .send(Effect::SeloraKeyringSet("sk-test".into()));
        let _ = app.effect_tx.send(Effect::SettingsSynced {
            base_url: server.uri(),
        });
        let _ = app.effect_tx.send(Effect::OpenFolder(root.clone()));
        let _ = app.effect_tx.send(Effect::OpenFile(main_rs.clone()));

        // Pump until the buffer is open (the real OpenFile worker ran).
        pump_until(&ctx, &mut app, |app| {
            app.state.buffers.contains_key(&main_rs)
        })
        .await;
        assert!(!app.state.is_dirty(&main_rs));

        // Agent mode on; submit through the composer (focus frame, then
        // Enter — the mod.rs pattern).
        app.show_chat = true;
        app.state.chat_panel.set_agent_mode(true);
        app.state.chat_panel.set_composer_text("add a comment");
        app.state.chat_panel.focus_composer();
        agent_frame(&ctx, &mut app, raw_screen());
        agent_frame(&ctx, &mut app, agent_key_event(egui::Key::Enter));
        assert!(app.state.chat_panel.is_streaming(), "the turn started");
        assert_eq!(app.state.chat_ids.len(), 1);

        // Pump until the diff card parks.
        pump_until(&ctx, &mut app, |app| {
            app.state.chat_panel.pending_approval_id().is_some()
        })
        .await;
        let approval_id = app
            .state
            .chat_panel
            .pending_approval_id()
            .expect("the diff card parked");
        assert!(
            std::fs::read_to_string(&main_rs).expect("read") == "fn main() {}\n",
            "nothing writes before the decision"
        );

        // The panel-event route (the reverse ChatId→Uuid map): Approve.
        let chat_id = app.state.chat_panel.in_flight().expect("in flight");
        app.route_chat_event(ChatPanelEvent::AgentDecision {
            id: chat_id,
            approval_id,
            decision: selora_ai::AgentDecision::Approve,
        });

        // Pump until the turn ends.
        pump_until(&ctx, &mut app, |app| !app.state.chat_panel.is_streaming()).await;

        // Disk + buffer + meter + footer, in one run.
        let approved = "// entry point\nfn main() {}\n";
        assert_eq!(
            std::fs::read_to_string(&main_rs).expect("read"),
            approved,
            "the approved write is on disk"
        );
        assert_eq!(
            app.state
                .buffers
                .get(&main_rs)
                .expect("buffer")
                .read()
                .expect("lock")
                .text(),
            approved,
            "the open buffer synced (§17.7)"
        );
        assert!(
            !app.state.is_dirty(&main_rs),
            "a synced buffer ends clean (the card was the save)"
        );
        assert!(
            (app.state.cost_meter.total_usd() - 0.0021).abs() < 1e-12,
            "the meter stepped by the scripted usage: {}",
            app.state.cost_meter.total_usd()
        );
        // user bubble + the card + the answer bubble + the summary footer.
        assert_eq!(app.state.chat_panel.turn_count(), 4);

        // Ctrl+Z's seam: the buffer reverts (dirty); disk keeps the
        // approved text.
        assert!(app.state.undo_agent_edit(&main_rs));
        assert_eq!(
            app.state
                .buffers
                .get(&main_rs)
                .expect("buffer")
                .read()
                .expect("lock")
                .text(),
            "fn main() {}\n"
        );
        assert!(app.state.is_dirty(&main_rs), "undo ends dirty against disk");
        assert_eq!(
            std::fs::read_to_string(&main_rs).expect("read"),
            approved,
            "undo never touches disk"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    /// The visible-failure path end-to-end: AgentSend with no key routes
    /// to the dispatcher, which answers with a synthetic Failed turn end —
    /// the panel unblocks and the toast carries the reason.
    #[tokio::test]
    async fn agent_send_without_a_key_fails_visibly_through_the_shell() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        app.state.models = vec![agent_test_model()];
        app.state.chat_panel.set_model("test-model");

        app.show_chat = true;
        app.state.chat_panel.set_agent_mode(true);
        app.state.chat_panel.set_composer_text("do things");
        app.state.chat_panel.focus_composer();
        agent_frame(&ctx, &mut app, raw_screen());
        agent_frame(&ctx, &mut app, agent_key_event(egui::Key::Enter));
        assert!(app.state.chat_panel.is_streaming());
        assert_eq!(app.state.chat_ids.len(), 1, "the request mapped");

        // No key anywhere: the dispatcher's synthetic AgentTurnDone lands.
        pump_until(&ctx, &mut app, |app| !app.state.chat_panel.is_streaming()).await;
        assert!(app.state.chat_ids.is_empty());
        assert!(
            app.state
                .toasts
                .iter()
                .any(|toast| toast.text.contains("no API key")),
            "toasts: {:?}",
            app.state.toasts
        );
        // The summary footer still landed (a failed turn ends the timeline).
        assert_eq!(app.state.chat_panel.turn_count(), 2); // user bubble + summary
    }

    /// Phase R1: the composer's Stop — the panel half (abort_stream)
    /// finalizes the bubble; the shell half (StopStream route) cancels
    /// the mapped request and drops the mapping, so late deltas for the
    /// aborted id never surface.
    #[tokio::test]
    async fn stop_stream_cancels_the_mapped_request() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        app.show_chat = true;

        let chat_id = selora_ui::sidebar::ChatId::next();
        app.state.chat_panel.begin_stream(chat_id);
        let uuid = uuid::Uuid::new_v4();
        app.state.chat_ids.insert(uuid, chat_id);
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(app.state.chat_panel.is_streaming());

        // The click's two halves, driven directly.
        app.state.chat_panel.abort_stream(chat_id);
        app.route_chat_event(ChatPanelEvent::StopStream { id: chat_id });
        assert!(!app.state.chat_panel.is_streaming(), "the panel unlocked");
        assert!(
            app.state.chat_ids.is_empty(),
            "the mapping went with the cancel"
        );

        // A delta already in the channel when the Stop landed is
        // stale-dropped (the panel's in-flight id is cleared). A
        // ChatError cannot arrive at all — the cancel ends the relay
        // silently (the wiring test's proof).
        app.msg_tx
            .send(AppMessage::ChatDelta(uuid, "late".into()))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        agent_frame(&ctx, &mut app, raw_screen());
        assert_eq!(app.state.chat_panel.turn_count(), 0, "nothing landed");
        assert!(app.state.toasts.is_empty(), "nothing errored");
    }

    /// §17.7's LSP half: a same-length agent edit is invisible to the
    /// shape compare — the agent-touched drain still sends didChange for
    /// the didOpen'd path.
    #[tokio::test]
    async fn agent_edit_sends_lsp_change_even_when_same_length() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);

        let main_rs =
            std::env::temp_dir().join(format!("selora-app-shell-agent-lsp-{}", std::process::id()));
        std::fs::write(&main_rs, "aaa\n").expect("write");

        // Open the file; the first paint sends didOpen.
        app.state
            .apply(AppMessage::FolderOpened(std::env::temp_dir()));
        app.state.apply(AppMessage::BufferLoaded {
            path: main_rs.clone(),
            text: "aaa\n".to_owned(),
        });
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(app.lsp_shapes.contains_key(&main_rs), "didOpen tracked");

        // A same-length agent edit through the message arms.
        let uuid = uuid::Uuid::new_v4();
        let chat_id = selora_ui::sidebar::ChatId::next();
        app.state.chat_panel.begin_stream(chat_id);
        app.state.chat_ids.insert(uuid, chat_id);
        app.state.apply(AppMessage::ToolCallStarted(
            uuid,
            selora_ai::ToolCall {
                id: "call_1".into(),
                name: "edit_file".into(),
                arguments: format!(
                    "{{\"path\":\"{}\",\"old_str\":\"aaa\",\"new_str\":\"bbb\"}}",
                    main_rs.file_name().expect("name").to_string_lossy()
                ),
            },
        ));
        app.state.apply(AppMessage::ToolResult(
            uuid,
            selora_ai::ToolResult {
                id: "call_1".into(),
                ok: true,
                content: "edited".into(),
            },
        ));
        assert_eq!(
            app.state
                .buffers
                .get(&main_rs)
                .expect("buffer")
                .read()
                .expect("lock")
                .text(),
            "bbb\n"
        );

        // Drain the LSP effects the frame sent (Open first, then the
        // agent-touched Change — the shape alone saw nothing).
        agent_frame(&ctx, &mut app, raw_screen());
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

        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                selora_lsp::LspEffect::Change { path, text }
                    if path == &main_rs && text == "bbb\n"
            )),
            "the agent-touched didChange reached the manager: {effects:?}"
        );
    }

    /// Phase R6b: the git diff view — OpenDiff sends the effect, the
    /// ready message opens the overlay, Esc closes it.
    #[tokio::test]
    async fn git_diff_overlay_lifecycle() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        app.state
            .apply(AppMessage::FolderOpened(PathBuf::from("/repo")));
        app.state
            .apply(AppMessage::GitStatusChanged(selora_git::GitStatus {
                branch: Some("main".into()),
                entries: vec![selora_git::FileStatus {
                    abs_path: PathBuf::from("/repo/src/main.rs"),
                    path: PathBuf::from("src/main.rs"),
                    staged: selora_git::Slot::Unmodified,
                    worktree: selora_git::Slot::Modified,
                }],
            }));
        assert!(app.git_diff.is_none());

        // The panel action: routes the effect (the dispatcher is real but
        // no repo exists at /repo — the synthetic reply below covers the
        // shell's observation half).
        app.route_git_action(GitAction::OpenDiff(PathBuf::from("/repo/src/main.rs")));
        app.msg_tx
            .send(AppMessage::GitDiffReady {
                path: PathBuf::from("/repo/src/main.rs"),
                diff: "--- a/src/main.rs
+++ b/src/main.rs
@@ -1 +1 @@
-old
+new"
                    .into(),
            })
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(app.git_diff.is_some(), "the overlay opened");

        // It paints (with the DiffView) and Esc closes it.
        agent_frame(&ctx, &mut app, raw_screen());
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(app.git_diff.is_some(), "painting alone keeps it open");
        agent_frame(&ctx, &mut app, agent_key_event(egui::Key::Escape));
        assert!(app.git_diff.is_none(), "Esc closed the overlay");
    }

    /// Phase R4: the status-bar orb's reserved-arm signals — an effect
    /// leaving sets the flag, the reply landing clears it.
    #[tokio::test]
    async fn reserved_arm_flags_follow_the_effects() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        assert!(!app.search_in_flight);
        // The search route needs a workspace root to build the effect.
        app.state
            .apply(AppMessage::FolderOpened(PathBuf::from("/repo")));

        // A workspace search leaves → Composing; the reply clears it.
        let search_id = selora_fs::search::SearchId::next();
        app.state.search.set_root(PathBuf::from("/repo"));
        let query = selora_fs::search::SearchQuery::new("main");
        app.route_search_event(SearchEvent::RunSearch {
            id: search_id,
            query,
        });
        assert!(app.search_in_flight, "the search set its flag");
        app.msg_tx
            .send(AppMessage::SearchResults {
                id: selora_fs::search::SearchId::next(),
                results: Ok(Vec::new()),
            })
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(!app.search_in_flight, "the reply cleared it");

        // A tree refresh leaves → Shaping; FileTreeUpdated clears it.
        app.route_tree_event(TreeEvent::TreeChanged);
        assert!(app.tree_in_flight);
        app.msg_tx
            .send(AppMessage::FileTreeUpdated {
                root: PathBuf::from("/repo"),
                tree: selora_fs::tree::FileTreeNode {
                    name: "repo".into(),
                    path: PathBuf::from("/repo"),
                    is_dir: true,
                    children: Vec::new(),
                    truncated: false,
                },
            })
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(!app.tree_in_flight);
    }

    /// Phase R3: boot restore loads `sessions.json` — the full timeline
    /// (bubbles + agent cards) becomes the live session.
    #[tokio::test]
    async fn boot_restore_loads_the_full_session_history() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);

        use selora_ai::{AgentSummary, ApprovalCard, ToolResult};
        use selora_ui::sidebar::chat::{ChatSession, SessionCard, SessionEntry};

        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("selora-app-boot-sess-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let sessions = vec![ChatSession {
            id: uuid::Uuid::new_v4(),
            title: "fix the build".into(),
            created_at: 1_700_000_000,
            entries: vec![
                SessionEntry::Bubble {
                    role: selora_ai::Role::User,
                    content: "fix the build".into(),
                    error: None,
                },
                SessionEntry::Card(Box::new(SessionCard {
                    id: "call_1".into(),
                    name: "edit_file".into(),
                    arguments: r#"{"path":"main.rs"}"#.into(),
                    approval: Some((
                        selora_ai::ApprovalId::nil(),
                        ApprovalCard::Diff {
                            path: "main.rs".into(),
                            diff: "@@ -1 +1 @@".into(),
                        },
                        1,
                    )),
                    result: Some(ToolResult {
                        id: "call_1".into(),
                        ok: true,
                        content: "edited".into(),
                    }),
                    decided: None,
                    expanded: false,
                })),
                SessionEntry::Summary(AgentSummary {
                    iterations: 1,
                    tools_used: 1,
                    approved: 1,
                    denied: 0,
                    tokens: 500,
                    cost_usd: 0.01,
                    completed: true,
                }),
            ],
        }];
        persistence::save_sessions(&dir, &sessions).expect("save sessions");

        let mut app = agent_app(&ctx);
        app.workspace_dir = Some(dir.clone());
        app.restore_session();
        assert_eq!(app.state.chat_panel.session_title(), "fix the build");
        assert_eq!(app.state.chat_panel.turn_count(), 3);
        assert!(
            !app.state.chat_panel.is_streaming(),
            "a restored session never streams"
        );
        assert_eq!(
            app.state.chat_panel.sessions().len(),
            0,
            "the last session went live"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Boot restore (§8.2): the agent pill follows the persisted session.
    #[tokio::test]
    async fn boot_restore_carries_agent_mode_from_workspace_state() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);

        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("selora-app-boot-root-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).expect("mkdir");
        let dir =
            std::env::temp_dir().join(format!("selora-app-boot-cfg-{}-{n}", std::process::id()));
        persistence::save(
            &dir,
            &WorkspaceState {
                root: Some(root.clone()),
                open_files: Vec::new(),
                active_path: None,
                chat_open: true,
                chat_turns: Vec::new(),
                agent_mode: true,
            },
        )
        .expect("save workspace.json");

        let mut app = agent_app(&ctx);
        app.workspace_dir = Some(dir.clone());
        app.restore_session();
        assert!(app.state.chat_panel.is_agent_mode());
        assert!(app.show_chat);

        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    /// `ui.reduce_motion` threads into the chat panel when settings land
    /// (the SettingsLoaded adoption site).
    #[tokio::test]
    async fn settings_loaded_threads_reduce_motion_into_the_panel() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        assert!(!app.state.chat_panel.is_reduce_motion());

        let mut settings = Settings::default();
        settings.ui.reduce_motion = true;
        app.msg_tx
            .send(AppMessage::SettingsLoaded(settings))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(app.state.chat_panel.is_reduce_motion());
    }

    /// Phase 6g fix: the chat panel adopts `settings.selora.active_model`
    /// at boot — before this, the header read "no model" and sends were
    /// blocked behind a picker round-trip even though a model was persisted.
    #[tokio::test]
    async fn settings_loaded_seeds_the_panels_model_once() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        assert!(app.state.chat_panel.model().is_none());

        let mut settings = Settings::default();
        settings.selora.active_model = Some("claude-opus-5-5".to_owned());
        app.msg_tx
            .send(AppMessage::SettingsLoaded(settings))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert_eq!(
            app.state.chat_panel.model().map(|m| m.as_str()),
            Some("claude-opus-5-5")
        );

        // A later settings reload never clobbers a picker choice.
        app.state.chat_panel.set_model("kimi-k3");
        let mut settings = Settings::default();
        settings.selora.active_model = Some("claude-opus-5-5".to_owned());
        app.msg_tx
            .send(AppMessage::SettingsLoaded(settings))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert_eq!(
            app.state.chat_panel.model().map(|m| m.as_str()),
            Some("kimi-k3")
        );
    }

    /// The final Phase-5 test (the brief's acceptance): ONE scripted turn
    /// through the whole path — AgentSend → the cards land in the panel →
    /// a Deny continues the loop with the denied tool result (the wire
    /// proves it) → an Approve writes the file AND syncs the open buffer →
    /// a Stop ends the turn → the meter sees the turn's receipt →
    /// stale/late ids drop without a panic. The mock mounts carry
    /// `.expect(1)`: each scripted round must be hit exactly once.
    #[tokio::test]
    async fn agent_turn_deny_approve_stop_end_to_end() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);

        // A real workspace with the file the agent edits.
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("selora-app-shell-deny-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).expect("mkdir");
        let main_rs = root.join("main.rs");
        std::fs::write(&main_rs, "fn main() {}\n").expect("write");

        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        // Round 1: a write the user will Deny. (Every round's body still
        // carries the first user message — priorities make the grown
        // conversations win; wiremock tries lower numbers first.)
        let guard1 = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains(
                "deny then approve",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(agent_tool_call_sse(
                    "call_write_denied",
                    "write_file",
                    r#"{"path":"denied.rs","content":"nope\n"}"#,
                )),
            )
            .with_priority(3)
            .expect(1)
            .mount_as_scoped(&server)
            .await;
        // Round 2 (its body carries the denied tool result): the edit the
        // user will Approve. The usage rides this round's finish chunk.
        let guard2 = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains(
                "user denied this action",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "{}{}{}",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_edit_1\",\"type\":\"function\",\"function\":{\"name\":\"edit_file\",\"arguments\":\"\"}}]}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\\\"main.rs\\\",\\\"old_str\\\":\\\"fn main() {}\\\",\\\"new_str\\\":\\\"// entry point\\\\nfn main() {}\\\"}\"}}]}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"cost_usd\":0.0021}}\n\ndata: [DONE]\n\n",
            )))
            .with_priority(2)
            .expect(1)
            .mount_as_scoped(&server)
            .await;
        // Round 3 (its body carries the edit's tool result): a slow bash
        // the user Approves and then Stops mid-run.
        let guard3 = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains("edited"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(agent_tool_call_sse(
                    "call_bash_1",
                    "bash",
                    r#"{"command":"Start-Sleep -Seconds 30"}"#,
                )),
            )
            .with_priority(1)
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let mut app = agent_app(&ctx);
        app.state.models = vec![agent_test_model()];
        app.state.chat_panel.set_model("test-model");
        let _ = app
            .effect_tx
            .send(Effect::SeloraKeyringSet("sk-test".into()));
        let _ = app.effect_tx.send(Effect::SettingsSynced {
            base_url: server.uri(),
        });
        let _ = app.effect_tx.send(Effect::OpenFolder(root.clone()));
        let _ = app.effect_tx.send(Effect::OpenFile(main_rs.clone()));
        pump_until(&ctx, &mut app, |app| {
            app.state.buffers.contains_key(&main_rs)
        })
        .await;

        // Agent mode on; submit through the composer.
        app.show_chat = true;
        app.state.chat_panel.set_agent_mode(true);
        app.state
            .chat_panel
            .set_composer_text("deny then approve then stop");
        app.state.chat_panel.focus_composer();
        agent_frame(&ctx, &mut app, raw_screen());
        agent_frame(&ctx, &mut app, agent_key_event(egui::Key::Enter));
        assert!(app.state.chat_panel.is_streaming());

        // -- Round 1 parks on the write card (user bubble + card). --------
        pump_until(&ctx, &mut app, |app| {
            app.state.chat_panel.pending_approval_id().is_some()
                && app.state.chat_panel.turn_count() == 2
        })
        .await;
        let chat_id = app.state.chat_panel.in_flight().expect("in flight");
        let approval_id = app
            .state
            .chat_panel
            .pending_approval_id()
            .expect("the write card parked");

        // -- Deny: the loop continues; the denied result rides round 2's
        //    request (guard2's matcher + expect(1) prove the wire). -------
        app.route_chat_event(ChatPanelEvent::AgentDecision {
            id: chat_id,
            approval_id,
            decision: selora_ai::AgentDecision::Deny,
        });
        pump_until(&ctx, &mut app, |app| {
            app.state.chat_panel.turn_count() == 3
                && app.state.chat_panel.pending_approval_id().is_some()
        })
        .await;
        assert!(
            !root.join("denied.rs").exists(),
            "a denied write never lands"
        );

        // -- Approve the edit: disk writes, the open buffer syncs clean.
        //    The meter has NOT stepped yet — the receipt lands at TurnDone.
        let approval_id = app
            .state
            .chat_panel
            .pending_approval_id()
            .expect("the edit card parked");
        app.route_chat_event(ChatPanelEvent::AgentDecision {
            id: chat_id,
            approval_id,
            decision: selora_ai::AgentDecision::Approve,
        });
        let approved = "// entry point\nfn main() {}\n";
        pump_until(&ctx, &mut app, |app| {
            // Round 3 parks on the command card — the edit's ToolResult
            // (and its §17.7 sync) precede it.
            app.state.chat_panel.turn_count() == 4
                && app.state.chat_panel.pending_approval_id().is_some()
        })
        .await;
        assert_eq!(
            std::fs::read_to_string(&main_rs).expect("read"),
            approved,
            "the approved edit is on disk"
        );
        assert_eq!(
            app.state
                .buffers
                .get(&main_rs)
                .expect("buffer")
                .read()
                .expect("lock")
                .text(),
            approved,
            "the open buffer synced"
        );
        assert!(!app.state.is_dirty(&main_rs), "the card was the save");
        assert_eq!(
            app.state.cost_meter.total_usd(),
            0.0,
            "the receipt lands at TurnDone, not per round"
        );

        // -- Approve the bash, then Stop mid-run (the status-line Stop's
        //    nil id, through the panel-event route). ---------------------
        let approval_id = app
            .state
            .chat_panel
            .pending_approval_id()
            .expect("the command card parked");
        app.route_chat_event(ChatPanelEvent::AgentDecision {
            id: chat_id,
            approval_id,
            decision: selora_ai::AgentDecision::Approve,
        });
        tokio::time::sleep(Duration::from_millis(300)).await; // let the child spawn
        app.route_chat_event(ChatPanelEvent::AgentDecision {
            id: chat_id,
            approval_id: selora_ai::ApprovalId::nil(),
            decision: selora_ai::AgentDecision::Stop,
        });
        pump_until(&ctx, &mut app, |app| !app.state.chat_panel.is_streaming()).await;

        // The turn ended StoppedByUser: summary footer, toast, meter.
        assert_eq!(
            app.state.chat_panel.turn_count(),
            5,
            "user bubble + 3 cards + summary footer"
        );
        assert!(
            app.state
                .toasts
                .iter()
                .any(|toast| toast.text.contains("stopped by user")),
            "toasts: {:?}",
            app.state.toasts
        );
        assert!(
            (app.state.cost_meter.total_usd() - 0.0021).abs() < 1e-12,
            "the meter saw the turn's receipt: {}",
            app.state.cost_meter.total_usd()
        );

        // Ctrl+Z's seam: the buffer reverts (dirty); disk keeps the
        // approved text.
        app.run_key_action(KeyAction::UndoAgentEdit);
        assert_eq!(
            app.state
                .buffers
                .get(&main_rs)
                .expect("buffer")
                .read()
                .expect("lock")
                .text(),
            "fn main() {}\n"
        );
        assert!(app.state.is_dirty(&main_rs));
        assert_eq!(std::fs::read_to_string(&main_rs).expect("read"), approved);

        // Stale/late ids drop without a panic: messages for a random uuid
        // and decisions for the ended turn's.
        let stale = uuid::Uuid::new_v4();
        app.msg_tx
            .send(AppMessage::ToolCallStarted(
                stale,
                selora_ai::ToolCall {
                    id: "call_late".into(),
                    name: "read_file".into(),
                    arguments: "{}".into(),
                },
            ))
            .expect("send");
        app.msg_tx
            .send(AppMessage::ToolResult(
                stale,
                selora_ai::ToolResult {
                    id: "call_late".into(),
                    ok: true,
                    content: "late".into(),
                },
            ))
            .expect("send");
        app.msg_tx
            .send(AppMessage::ChatDelta(stale, "late".into()))
            .expect("send");
        // A stale turn end carrying a cost and a Failed end drops WHOLE:
        // no panel entry, no meter movement, no toast.
        let toasts_before = app.state.toasts.len();
        app.msg_tx
            .send(AppMessage::AgentTurnDone(
                stale,
                selora_ai::ChatReceipt {
                    request_id: stale,
                    model: "test-model".into(),
                    prompt_tokens: 0,
                    completion_tokens: 999,
                    cost_usd: 9.99,
                    duration_ms: 1,
                    estimated: true,
                },
                selora_ai::AgentSummary {
                    iterations: 9,
                    tools_used: 9,
                    approved: 9,
                    denied: 0,
                    tokens: 999,
                    cost_usd: 9.99,
                    completed: false,
                },
                selora_ai::TurnEnd::Failed("stale turn".into()),
            ))
            .expect("send");
        let _ = app.effect_tx.send(Effect::AgentDecision(
            stale,
            selora_ai::ApprovalId::nil(),
            selora_ai::AgentDecision::Approve,
        ));
        agent_frame(&ctx, &mut app, raw_screen());
        agent_frame(&ctx, &mut app, raw_screen());
        assert_eq!(app.state.chat_panel.turn_count(), 5, "nothing appended");
        assert!(
            (app.state.cost_meter.total_usd() - 0.0021).abs() < 1e-12,
            "the meter did not move"
        );
        assert_eq!(
            app.state.toasts.len(),
            toasts_before,
            "a stale turn end never toasts: {:?}",
            app.state.toasts
        );

        // The scoped mounts verify their expect(1) here.
        drop(guard1);
        drop(guard2);
        drop(guard3);
        let _ = std::fs::remove_dir_all(root);
    }

    // --- First-run splash (Phase 6a) ------------------------------------------

    #[tokio::test]
    async fn no_stored_key_shows_the_splash_and_owns_the_keyboard() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(matches!(app.splash, SplashState::Pending));
        assert!(!app.overlay_open(&ctx));

        app.msg_tx.send(AppMessage::NoStoredKey).expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(
            matches!(app.splash, SplashState::Showing(_)),
            "the splash shows on the empty-keyring signal"
        );
        // While the splash covers the workbench the global keybinds stay
        // quiet (the overlay gate).
        assert!(app.overlay_open(&ctx));
    }

    #[tokio::test]
    async fn a_validated_boot_key_never_shows_the_splash() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        // The boot keyring read found a key and it validated: Pending
        // resolves to Dismissed without painting.
        app.msg_tx
            .send(AppMessage::SeloraStatus(
                selora_ai::SeloraStatus::Connected {
                    model: None,
                    latency_ms: 40,
                },
            ))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(matches!(app.splash, SplashState::Dismissed));
        // A late NoStoredKey (channel reordering) never reopens it.
        app.msg_tx.send(AppMessage::NoStoredKey).expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(matches!(app.splash, SplashState::Dismissed));
        assert!(!app.overlay_open(&ctx));
    }

    #[tokio::test]
    async fn splash_skip_dismisses_for_the_run() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        app.msg_tx.send(AppMessage::NoStoredKey).expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(matches!(app.splash, SplashState::Showing(_)));

        assert!(app.route_splash_event(SplashEvent::Skip));
        app.splash = SplashState::Dismissed;
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(!app.overlay_open(&ctx));
        // The splash never reappears after a skip (Pending-only rule).
        app.msg_tx.send(AppMessage::NoStoredKey).expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(matches!(app.splash, SplashState::Dismissed));
    }

    /// The Connect route, headless against the mock gateway: the status
    /// flips to Connecting immediately; the gateway's rejection arrives as
    /// `SeloraStatus::Error` and the splash stays up with the gateway's
    /// words (never writes the keyring — the key failed validation).
    #[tokio::test]
    async fn splash_connect_validates_and_surfaces_rejection() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);

        let server = wiremock::MockServer::start().await;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("{\"error\":\"bad key\"}"))
            .expect(1)
            .mount(&server)
            .await;

        let mut app = agent_app(&ctx);
        let _ = app.effect_tx.send(Effect::SettingsSynced {
            base_url: server.uri(),
        });
        app.msg_tx.send(AppMessage::NoStoredKey).expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(matches!(app.splash, SplashState::Showing(_)));

        // Connect: the status flips immediately (the splash disables its
        // button on it), the key goes to the runtime.
        assert!(!app.route_splash_event(SplashEvent::Connect("sk-gw-rejected".to_owned())));
        assert!(matches!(
            app.state.selora_status,
            selora_ai::SeloraStatus::Connecting
        ));

        // The gateway's rejection lands: the splash stays up (no fade).
        pump_until(&ctx, &mut app, |app| {
            matches!(app.state.selora_status, selora_ai::SeloraStatus::Error(_))
        })
        .await;
        agent_frame(&ctx, &mut app, raw_screen());
        match &app.splash {
            SplashState::Showing(splash) => assert!(!splash.is_fading()),
            _ => panic!("a rejected key keeps the splash up"),
        }
    }

    // --- About dialog + update check (Phase 6b) -------------------------------

    #[tokio::test]
    async fn help_about_opens_the_dialog_and_esc_closes_it() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(app.about_dialog.is_none());

        app.run_command(CommandId::HelpAbout);
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(app.about_dialog.is_some());
        assert!(app.overlay_open(&ctx));

        // Esc closes (the dialog's own handling).
        agent_frame(&ctx, &mut app, agent_key_event(egui::Key::Escape));
        assert!(app.about_dialog.is_none());
        assert!(!app.overlay_open(&ctx));
    }

    #[tokio::test]
    async fn check_for_updates_clicks_through_and_the_reply_rearms() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);

        // The releases endpoint is never really hit in tests: the About
        // click only *sends the effect*; the reply below is synthetic.
        let mut app = agent_app(&ctx);
        app.run_command(CommandId::HelpAbout);

        // Click "Check for updates" (the four-frame overlay click: two
        // settle/hover frames, press, release). The dialog is centered on
        // the 1280×800 screen.
        let modal =
            egui::Rect::from_center_size(egui::pos2(640.0, 400.0), egui::vec2(448.0, 328.0));
        let check = egui::pos2(modal.center().x - 82.0, modal.bottom() - 44.0);
        let move_frame = || {
            let mut raw = raw_screen();
            raw.events = vec![egui::Event::PointerMoved(check)];
            raw
        };
        let button_frame = |pressed: bool| {
            let mut raw = raw_screen();
            raw.events = vec![egui::Event::PointerButton {
                pos: check,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }];
            raw
        };
        agent_frame(&ctx, &mut app, move_frame());
        agent_frame(&ctx, &mut app, move_frame());
        agent_frame(&ctx, &mut app, button_frame(true));
        agent_frame(&ctx, &mut app, button_frame(false));
        assert!(
            app.update_check_in_flight,
            "the click sent the check (button disarmed)"
        );

        // A newer release toasts the tag with the download link.
        app.msg_tx
            .send(AppMessage::UpdateChecked(Ok(selora_ai::ReleaseInfo {
                tag: "v9.9.9".to_owned(),
                html_url: "https://github.com/ToxicAngelX/Selora-IDE/releases/tag/v9.9.9"
                    .to_owned(),
                newer: true,
            })))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(!app.update_check_in_flight, "the reply rearmed the button");
        let toast = app.state.toasts.last().expect("a toast landed");
        assert!(toast.text.contains("v9.9.9"));
        assert_eq!(
            toast.link.as_ref().map(|(label, _)| label.as_str()),
            Some("download")
        );

        // Up-to-date and failure forms.
        app.msg_tx
            .send(AppMessage::UpdateChecked(Ok(selora_ai::ReleaseInfo {
                tag: "v0.1.0".to_owned(),
                html_url: String::new(),
                newer: false,
            })))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(app
            .state
            .toasts
            .last()
            .expect("toast")
            .text
            .contains("up to date"));
        app.msg_tx
            .send(AppMessage::UpdateChecked(Err("offline".to_owned())))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        let toast = app.state.toasts.last().expect("toast");
        assert!(toast.text.contains("update check failed: offline"));
        assert!(matches!(toast.severity, Severity::Error));
    }

    /// Phase 6h: the SeloraStatus reduction threads gateway reachability
    /// into the chat panel's presence widgets (the avatars sleep while
    /// disconnected; Connecting is not a failure).
    #[tokio::test]
    async fn gateway_status_drives_the_presence_avatars() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let mut app = agent_app(&ctx);
        assert!(app.state.chat_panel.is_gateway_connected());

        app.msg_tx
            .send(AppMessage::SeloraStatus(
                selora_ai::SeloraStatus::Disconnected,
            ))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(!app.state.chat_panel.is_gateway_connected());

        app.msg_tx
            .send(AppMessage::SeloraStatus(
                selora_ai::SeloraStatus::Connecting,
            ))
            .expect("send");
        agent_frame(&ctx, &mut app, raw_screen());
        assert!(app.state.chat_panel.is_gateway_connected());
    }
}
