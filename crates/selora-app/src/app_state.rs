//! The Phase 2 app-state spine (ARCHITECTURE.md §2, §3.1).
//!
//! `Effect` (main thread → runtime) and `AppMessage` (runtime → main thread)
//! are the app's message contract; [`AppState`] is the single struct that owns
//! all editor state on the main thread. Workers never touch it — they only
//! send [`AppMessage`]s, which the frame loop drains at the top of every
//! frame (ARCHITECTURE.md §2 stage 1) and reduces via [`AppState::apply`].
//!
//! Phase 2 growth over the merged Phase 2 subagent contract:
//! - buffers are shared per path (`BTreeMap<PathBuf, Arc<RwLock<Buffer>>>`,
//!   ARCHITECTURE.md §3.2) and each (pane, tab) gets its own [`Editor`]
//!   instance so every pane keeps its own cursor/scroll per tab;
//! - dirty tracking is derived, not stored in the editor: the shell reports
//!   each painted editor's buffer *shape* (char + line counts — an O(log n)
//!   rope query) plus whether edit-capable keyboard input reached a focused
//!   editor this frame ([`AppState::note_editor_frame`]). A shape change
//!   since the previous frame OR an edit signal marks the tab dirty;
//!   [`AppMessage::BufferSaved`] resets the baseline and clears the flag.
//!   Shape alone cannot see same-length replacements, which is why the
//!   keyboard signal rides along. `selora-editor` is untouched.
//! - toasts (UI_SPEC.md §13) live here so every error path funnels through
//!   one place (PLAN.md §8: no silent failures).
//!
//! Known deviation from the merged subagent contract, absorbed at shell
//! integration: the `file_tree` field is a `FileTreePanel` (the Phase 2b
//! widget that owns the `FileTreeView` plus its inline rename editor) instead
//! of a bare `FileTreeView` — owning the view twice would fork expanded/
//! selection state.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use selora_ai::{ChatReceipt, ChatRequest, Model};
use selora_core::{Buffer, SeloraError, Workspace};
use selora_editor::Editor;
use selora_fs::tree::FileTreeNode;
use selora_settings::Settings;
use selora_ui::editor::split::EditorArea;
use selora_ui::palette::command::CommandPalette;
use selora_ui::palette::fuzzy::FuzzyFileFinder;
use selora_ui::sidebar::FileTreePanel;
use uuid::Uuid;

use crate::persistence::WorkspaceState;

/// Effects the main thread sends to the tokio runtime (ARCHITECTURE.md §2.2).
///
/// `SaveBuffer` carries the shared buffer itself (not a [`BufferId`]): the
/// worker snapshots the text under a read lock inside `spawn_blocking`, so
/// the frame loop never blocks on IO and the write is always of the text the
/// user sees. The path names the write target.
#[derive(Debug)]
pub enum Effect {
    // File system
    OpenFile(PathBuf),
    OpenFolder(PathBuf),
    RefreshFileTree(PathBuf),
    SaveBuffer {
        path: PathBuf,
        buffer: Arc<RwLock<Buffer>>,
    },

    // Settings (selora-settings, Phase 2f)
    SettingsLoad,
    /// Persist [`Settings`] (ARCHITECTURE.md §2.2) — the runtime calls
    /// `Settings::save` off the frame path.
    SettingsSave(Settings),

    // Selora (the Phase 1c network half — live until the Phase 4 chat UI)
    SeloraConnect(String),
    SeloraListModels,
    SeloraChat(Uuid, ChatRequest),
    /// Hold the Selora key for this session (the OS keyring itself is
    /// Phase 4, ARCHITECTURE.md §7).
    SeloraKeyringSet(String),
}

/// Messages workers send to the main thread (ARCHITECTURE.md §2.1).
#[derive(Debug)]
pub enum AppMessage {
    // Selora
    ModelsLoaded(Result<Vec<Model>, SeloraError>),
    ChatDelta(Uuid, String),
    ChatDone(Uuid, ChatReceipt),
    ChatError(Uuid, SeloraError),

    // File system
    FolderOpened(PathBuf),
    /// A fresh file tree for `root` (ARCHITECTURE.md §2.1; built by the
    /// worker via `selora_fs::tree::build_tree`).
    FileTreeUpdated {
        root: PathBuf,
        tree: FileTreeNode,
    },
    /// A file read completed; the shell creates the shared buffer and opens
    /// the tab. Files over the 4 MB Lite cap never arrive here — they fail
    /// with [`AppMessage::BufferLoadFailed`] instead (ARCHITECTURE.md §4.1).
    BufferLoaded {
        path: PathBuf,
        text: String,
    },
    BufferLoadFailed {
        path: PathBuf,
        error: SeloraError,
    },
    /// An atomic write completed; the tab's dirty dot clears.
    BufferSaved(PathBuf),
    BufferSaveFailed {
        path: PathBuf,
        error: SeloraError,
    },
    /// A blocking rfd picker resolved on a worker thread. `None` = the user
    /// cancelled (not an error — nothing is surfaced).
    DialogFilePicked(Option<PathBuf>),
    DialogFolderPicked(Option<PathBuf>),

    // Settings (Phase 2f)
    SettingsLoaded(Settings),
    /// A save completed — the shell toasts "Settings saved" (UI_SPEC.md §6).
    SettingsSaved,
    /// A load/save failed (including the `_v`-mismatch wipe error) — toast.
    SettingsError(SeloraError),

    // App
    /// A keyring operation failed (ARCHITECTURE.md §2.1; runtime side in
    /// Phase 4).
    KeyringError(SeloraError),
}

/// Toast severity (UI_SPEC.md §13): info dismisses after 4s, error after 8s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Error,
}

impl Severity {
    fn lifetime(self) -> Duration {
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
}

/// The cheap shape signature used for dirty derivation: a buffer's char and
/// line counts (O(log n) rope queries — never a full text scan).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BufferShape {
    pub len_chars: usize,
    pub line_count: usize,
}

impl BufferShape {
    /// The shape of a buffer right now.
    pub fn of(buffer: &Buffer) -> Self {
        Self {
            len_chars: buffer.len_chars(),
            line_count: buffer.line_count(),
        }
    }
}

/// The main-thread state (ARCHITECTURE.md §3.1, Phase 2 subset). Owned by
/// the UI thread; workers never touch it — they only send [`AppMessage`]s.
pub struct AppState {
    pub workspace: Option<Workspace>,
    /// Editor-area chrome: the split tree with one tab set per pane
    /// (UI_SPEC.md §3.1, §3.3). `Default` = one empty pane.
    pub editor_area: EditorArea,
    /// The file tree sidebar (header, rows, context menu, inline editor).
    /// See the module docs for the `FileTreePanel`-vs-`FileTreeView` note.
    pub file_tree: FileTreePanel,
    /// Command palette visibility + selection (UI_SPEC.md §5). The palette
    /// emits `CommandId`s; routing lives in the shell frame loop.
    pub palette: CommandPalette,
    /// The `Ctrl+P` fuzzy file finder (UI_SPEC.md §5.1), fed from the tree.
    pub fuzzy: FuzzyFileFinder,
    /// User settings (ARCHITECTURE.md §8.1). Loaded at boot via
    /// `Effect::SettingsLoad`; the Settings dialog mutates and saves it.
    pub settings: Settings,
    /// Open buffers, shared per path (ARCHITECTURE.md §3.2): the editor
    /// panes, the save worker, and (in Phase 4) LSP all hold the same arc.
    pub buffers: BTreeMap<PathBuf, Arc<RwLock<Buffer>>>,
    /// One `Editor` per (pane, tab): every pane keeps its own cursor/scroll
    /// per tab; the shell paints the active tab's editor into that pane.
    pub editors: BTreeMap<(selora_ui::editor::split::PaneId, PathBuf), Editor>,
    /// Shape baseline at the last load/save — the "clean" reference for
    /// dirty derivation.
    saved_shapes: BTreeMap<PathBuf, BufferShape>,
    /// Shape as of the previous frame's paint (the change detector).
    last_shapes: BTreeMap<PathBuf, BufferShape>,
    /// Active toasts, oldest first; at most three (UI_SPEC.md §13).
    pub toasts: Vec<Toast>,
    /// All workspace file paths + parent-dir tags, the fuzzy finder's
    /// candidate source (refreshed whenever the tree updates).
    pub fuzzy_source: Vec<(PathBuf, String)>,
    /// Gateway model catalog (Phase 1c network half; the chat UI lands in
    /// Phase 4 — kept so `ModelsLoaded` has a home).
    pub models: Vec<Model>,
    /// The last streamed chat reply (Phase 1c spike holdover, surfaced by
    /// nothing in the Phase 2 shell; the chat sidebar lands in Phase 4).
    pub chat_stream: String,
    pub chat_receipt: Option<ChatReceipt>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            workspace: None,
            editor_area: EditorArea::new(),
            file_tree: FileTreePanel::new(),
            palette: CommandPalette::new(),
            fuzzy: FuzzyFileFinder::new(),
            settings: Settings::default(),
            buffers: BTreeMap::new(),
            editors: BTreeMap::new(),
            saved_shapes: BTreeMap::new(),
            last_shapes: BTreeMap::new(),
            toasts: Vec::new(),
            fuzzy_source: Vec::new(),
            models: Vec::new(),
            chat_stream: String::new(),
            chat_receipt: None,
        }
    }
}

impl AppState {
    /// Apply one worker message — the message-drain reduction (ARCHITECTURE
    /// §2 stage 1). Called at the top of every frame for every drained
    /// message, on the main thread.
    pub fn apply(&mut self, message: AppMessage) {
        match message {
            // -- Selora (Phase 1c network half) --------------------------------
            AppMessage::ModelsLoaded(Ok(models)) => {
                self.models = models;
            }
            AppMessage::ModelsLoaded(Err(error)) => {
                self.toast(format!("selora: {error}"), Severity::Error);
            }
            AppMessage::ChatDelta(_chat_id, delta) => {
                self.chat_stream.push_str(&delta);
            }
            AppMessage::ChatDone(_chat_id, receipt) => {
                self.chat_receipt = Some(receipt);
            }
            AppMessage::ChatError(_chat_id, error) => {
                self.toast(format!("chat: {error}"), Severity::Error);
            }

            // -- File system ---------------------------------------------------
            AppMessage::FolderOpened(root) => {
                if self
                    .workspace
                    .as_ref()
                    .is_none_or(|workspace| workspace.root != root)
                {
                    self.workspace = Some(Workspace::new(root));
                }
            }
            AppMessage::FileTreeUpdated { root, tree } => {
                if self.workspace.is_none() {
                    self.workspace = Some(Workspace::new(root));
                }
                self.file_tree.set_tree(tree);
                self.refresh_fuzzy_source();
            }
            AppMessage::BufferLoaded { path, text } => {
                let buffer = Arc::new(RwLock::new(Buffer::from_text(&text)));
                let shape = BufferShape::of(&buffer.read().unwrap_or_else(|e| e.into_inner()));
                self.buffers.insert(path.clone(), buffer);
                self.saved_shapes.insert(path.clone(), shape);
                self.last_shapes.insert(path, shape);
                // Open the tab only now — a tab never points at a buffer
                // that has not loaded (no blank editors).
                self.open_file_from_load(path);
            }
            AppMessage::BufferLoadFailed { path, error } => {
                self.toast(format!("{}: {error}", path.display()), Severity::Error);
            }
            AppMessage::BufferSaved(path) => {
                let shape = self.buffers.get(&path).map(|buffer| {
                    BufferShape::of(&buffer.read().unwrap_or_else(|e| e.into_inner()))
                });
                if let Some(shape) = shape {
                    self.saved_shapes.insert(path.clone(), shape);
                }
                self.editor_area.set_dirty(&path, false);
                self.toast(format!("saved {}", path.display()), Severity::Info);
            }
            AppMessage::BufferSaveFailed { path, error } => {
                self.toast(format!("{}: {error}", path.display()), Severity::Error);
            }
            AppMessage::DialogFilePicked(Some(path)) => {
                self.open_file_from_load(path);
            }
            AppMessage::DialogFilePicked(None) => {}
            AppMessage::DialogFolderPicked(Some(path)) => {
                self.switch_folder(path);
            }
            AppMessage::DialogFolderPicked(None) => {}

            // -- Settings ------------------------------------------------------
            AppMessage::SettingsLoaded(settings) => {
                self.settings = settings;
            }
            AppMessage::SettingsSaved => {
                self.toast("Settings saved", Severity::Info);
            }
            AppMessage::SettingsError(error) => {
                self.toast(format!("{error}"), Severity::Error);
            }
            AppMessage::KeyringError(error) => {
                self.toast(format!("{error}"), Severity::Error);
            }
        }
    }

    /// Open a file into the active pane; keep `workspace.open_files` in sync
    /// (ARCHITECTURE.md §3.3 — the same focus-order semantics on both sides).
    pub fn open_file(&mut self, path: PathBuf) {
        self.editor_area.open_in_active(path.clone());
        if let Some(workspace) = &mut self.workspace {
            workspace.open(path);
        }
    }

    /// Open a picked/loaded file: reuses the buffer when it is already open
    /// (a duplicate open focuses the tab, per the tab model's semantics).
    fn open_file_from_load(&mut self, path: PathBuf) {
        self.open_file(path);
    }

    /// `Ctrl+\` (UI_SPEC.md §15): split the active editor pane.
    pub fn split_editor(&mut self) {
        self.editor_area.request_split();
    }

    /// `Ctrl+W`: close the active tab (prunes an emptied pane). Returns the
    /// closed path so the shell can drop the buffer (and later fire LspClose).
    pub fn close_editor(&mut self) -> Option<PathBuf> {
        let closed = self.editor_area.request_close();
        if let (Some(path), Some(workspace)) = (&closed, &mut self.workspace) {
            workspace.close(path);
        }
        closed
    }

    /// Close every tab in every pane. Returns the closed paths (deduped) so
    /// the shell can drop their buffers.
    pub fn close_all_editors(&mut self) -> Vec<PathBuf> {
        let panes = self.editor_area.pane_ids();
        let mut closed: Vec<PathBuf> = Vec::new();
        for pane in panes {
            let paths = self
                .editor_area
                .pane(pane)
                .map(|tabs| tabs.paths())
                .unwrap_or_default();
            for path in paths {
                if !closed.contains(&path) {
                    closed.push(path.clone());
                }
                if let Some(tabs) = self.editor_area.pane_mut(pane) {
                    tabs.close_path(&path);
                }
                if let Some(workspace) = &mut self.workspace {
                    workspace.close(&path);
                }
            }
        }
        self.editor_area.prune();
        closed
    }

    /// Drop the buffer, dirty baselines, and per-pane editors for a path
    /// that no longer has a tab anywhere. A file still open in another pane
    /// keeps everything (the buffer is shared by path).
    pub fn prune_closed(&mut self, path: &Path) {
        let still_open = self.editor_area.pane_ids().iter().any(|pane| {
            self.editor_area
                .pane(*pane)
                .is_some_and(|tabs| tabs.position(path).is_some())
        });
        if !still_open {
            self.buffers.remove(path);
            self.saved_shapes.remove(path);
            self.last_shapes.remove(path);
            self.editors.retain(|(_, open), open_path| open_path != path);
        }
    }

    /// A new folder was picked: close the old session's tabs and buffers,
    /// reset the tree, and re-root the workspace. The tree itself arrives
    /// later as `FileTreeUpdated`.
    pub fn switch_folder(&mut self, root: PathBuf) {
        let closed = self.close_all_editors();
        for path in &closed {
            self.prune_closed(path);
        }
        self.workspace = Some(Workspace::new(root));
        self.file_tree = FileTreePanel::new();
        self.fuzzy_source.clear();
    }

    /// Dirty derivation, one frame per painted editor (see the module docs).
    ///
    /// `shape` is the buffer's shape *this* frame; `edit_input` is whether
    /// edit-capable keyboard input (text / Backspace / Delete / Enter) was
    /// delivered while this editor had focus. A shape change since the
    /// previous frame or an edit signal marks the tab dirty.
    pub fn note_editor_frame(&mut self, path: &Path, shape: BufferShape, edit_input: bool) {
        let changed = self.last_shapes.insert(path.to_path_buf(), shape);
        let shape_changed = changed.is_some_and(|previous| previous != shape);
        if shape_changed || edit_input {
            self.editor_area.set_dirty(path, true);
        }
    }

    /// Whether the tab for `path` is dirty.
    pub fn is_dirty(&self, path: &Path) -> bool {
        self.editor_area
            .pane_ids()
            .iter()
            .any(|pane| self.editor_area.pane(*pane).is_some_and(|t| t.is_dirty(path)))
    }

    /// The shape baseline the buffer had when it was last loaded/saved.
    pub fn saved_shape(&self, path: &Path) -> Option<BufferShape> {
        self.saved_shapes.get(path).copied()
    }

    /// Push a toast (UI_SPEC.md §13): info 4s, error 8s, max three visible —
    /// the oldest is dropped when a fourth arrives.
    pub fn toast(&mut self, text: impl Into<String>, severity: Severity) {
        let toast = Toast {
            text: text.into(),
            severity,
            expires_at: Instant::now() + severity.lifetime(),
        };
        self.toasts.push(toast);
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    /// Drop expired toasts. Returns the earliest remaining expiry (so the
    /// shell can schedule a repaint at that instant).
    pub fn expire_toasts(&mut self, now: Instant) -> Option<Instant> {
        self.toasts.retain(|toast| toast.expires_at > now);
        self.toasts.iter().map(|toast| toast.expires_at).min()
    }

    /// The active pane's active tab path, if any.
    pub fn active_path(&self) -> Option<PathBuf> {
        let pane = self.editor_area.active_pane_id();
        self.editor_area
            .pane(pane)
            .and_then(|tabs| tabs.active_path())
            .map(Path::to_path_buf)
    }

    /// Rebuild the fuzzy finder's candidate source from the current tree:
    /// every file path, tagged with its parent directory's name (UI_SPEC.md
    /// §5.1 right column).
    pub fn refresh_fuzzy_source(&mut self) {
        let mut source = Vec::new();
        if let Some(tree) = self.file_tree.tree() {
            collect_files(tree, &mut source);
        }
        self.fuzzy_source = source;
        self.fuzzy.set_source(self.fuzzy_source.clone());
    }

    /// The session snapshot persisted to `workspace.json`
    /// (ARCHITECTURE.md §8.2): the workspace root, the open files in tab
    /// order, and the active file.
    pub fn workspace_snapshot(&self) -> WorkspaceState {
        WorkspaceState {
            root: self.workspace.as_ref().map(|workspace| workspace.root.clone()),
            open_files: self
                .workspace
                .as_ref()
                .map(|workspace| workspace.open_files.clone())
                .unwrap_or_default(),
            active_path: self.active_path(),
        }
    }
}

/// Walk a tree, collecting every file as `(path, parent-dir-name)`.
fn collect_files(node: &FileTreeNode, out: &mut Vec<(PathBuf, String)>) {
    let tag = node.name.clone();
    for child in &node.children {
        if child.is_dir {
            collect_files(child, out);
        } else {
            out.push((child.path.clone(), tag.clone()));
        }
    }
}

/// Requeue paths for restore (used by the shell's sequential-open pump).
pub type RestoreQueue = VecDeque<PathBuf>;

#[cfg(test)]
mod tests {
    use super::*;
    use selora_ui::editor::split::PaneId;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    fn loaded(state: &mut AppState, name: &str, text: &str) -> PathBuf {
        let path = path(name);
        state.apply(AppMessage::BufferLoaded {
            path: path.clone(),
            text: text.to_owned(),
        });
        path
    }

    // --- message-drain reduction -------------------------------------------

    #[test]
    fn buffer_loaded_creates_buffer_and_opens_tab() {
        let mut state = AppState::default();
        assert!(state.buffers.is_empty());
        let p = loaded(&mut state, "a.rs", "fn main() {}");

        assert!(state.buffers.contains_key(&p));
        assert_eq!(
            state.editor_area.pane(0).and_then(|t| t.active_path()),
            Some(p.as_path())
        );
        assert_eq!(
            state.workspace.as_ref().map(|w| w.active_path.clone()),
            Some(Some(p.clone()))
        );
        assert!(!state.is_dirty(&p));
        // The clean baseline matches the loaded shape.
        assert_eq!(state.saved_shape(&p), Some(BufferShape::of(&Buffer::from_text("fn main() {}"))));
    }

    #[test]
    fn folder_and_tree_updates_install_workspace_tree_and_fuzzy_source() {
        let mut state = AppState::default();
        state.apply(AppMessage::FolderOpened(path("/repo")));

        let tree = FileTreeNode {
            name: "repo".into(),
            path: path("/repo"),
            is_dir: true,
            children: vec![
                FileTreeNode {
                    name: "src".into(),
                    path: path("/repo/src"),
                    is_dir: true,
                    children: vec![FileTreeNode {
                        name: "main.rs".into(),
                        path: path("/repo/src/main.rs"),
                        is_dir: false,
                        children: vec![],
                        truncated: false,
                    }],
                    truncated: false,
                },
                FileTreeNode {
                    name: "Cargo.toml".into(),
                    path: path("/repo/Cargo.toml"),
                    is_dir: false,
                    children: vec![],
                    truncated: false,
                },
            ],
            truncated: false,
        };
        state.apply(AppMessage::FileTreeUpdated {
            root: path("/repo"),
            tree,
        });

        assert_eq!(
            state.workspace.as_ref().map(|w| w.root.clone()),
            Some(path("/repo"))
        );
        assert!(state.file_tree.tree().is_some());
        // Every file, tagged with its parent directory's name.
        assert_eq!(state.fuzzy_source.len(), 2);
        assert!(state
            .fuzzy_source
            .contains(&(path("/repo/src/main.rs"), "src".to_owned())));
        assert!(state
            .fuzzy_source
            .contains(&(path("/repo/Cargo.toml"), "repo".to_owned())));
    }

    #[test]
    fn error_paths_produce_toasts_with_the_real_message() {
        let mut state = AppState::default();
        state.apply(AppMessage::BufferLoadFailed {
            path: path("big.bin"),
            error: SeloraError::Other("file too large".into()),
        });
        state.apply(AppMessage::BufferSaveFailed {
            path: path("a.rs"),
            error: SeloraError::Io(std::io::Error::other("disk full")),
        });
        state.apply(AppMessage::SettingsError(SeloraError::Config(
            "bad schema".into(),
        )));

        assert_eq!(state.toasts.len(), 3);
        assert!(state.toasts[0].text.contains("file too large"));
        assert!(state.toasts[1].text.contains("disk full"));
        assert!(state.toasts[2].text.contains("bad schema"));
        for toast in &state.toasts {
            assert_eq!(toast.severity, Severity::Error);
        }
    }

    #[test]
    fn buffer_saved_clears_the_dirty_flag_and_rebases_the_shape() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "a.rs", "one\ntwo");
        let shape = BufferShape {
            len_chars: 7,
            line_count: 2,
        };
        state.note_editor_frame(&p, shape, false); // clean paint
        state.note_editor_frame(&p, BufferShape::of(&Buffer::from_text("one\ntwo\nthree")), false);
        assert!(state.is_dirty(&p));

        // The buffer on disk (in this test, the shared buffer) was written;
        // the worker replies BufferSaved.
        state.apply(AppMessage::BufferSaved(p.clone()));
        assert!(!state.is_dirty(&p));
        assert_eq!(
            state.saved_shape(&p),
            Some(BufferShape::of(&Buffer::from_text("one\ntwo\nthree")))
        );
    }

    #[test]
    fn toasts_capped_at_three_with_severity_lifetimes() {
        let mut state = AppState::default();
        state.toast("a", Severity::Info);
        state.toast("b", Severity::Error);
        state.toast("c", Severity::Info);
        state.toast("d", Severity::Info);
        assert_eq!(state.toasts.len(), 3);
        // Oldest dropped, newest kept.
        assert_eq!(state.toasts.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["b", "c", "d"]);
        assert_eq!(Severity::Info.lifetime(), Duration::from_secs(4));
        assert_eq!(Severity::Error.lifetime(), Duration::from_secs(8));
    }

    #[test]
    fn expire_toasts_drops_past_and_reports_next_expiry() {
        let mut state = AppState::default();
        state.toast("info", Severity::Info);
        state.toast("error", Severity::Error);
        let now = Instant::now();
        // Nothing expired yet.
        let next = state.expire_toasts(now);
        assert!(next.is_some());
        // After the info lifetime (4s) only the error toast survives.
        let later = now + Duration::from_secs(5);
        assert_eq!(state.expire_toasts(later), None);
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].severity, Severity::Error);
    }

    // --- dirty-flag derivation ----------------------------------------------

    #[test]
    fn dirty_derivation_marks_dirty_only_when_shape_or_input_change() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "a.rs", "hello");
        let shape = BufferShape {
            len_chars: 5,
            line_count: 1,
        };

        // Frame 1: baseline paint — clean.
        state.note_editor_frame(&p, shape, false);
        assert!(!state.is_dirty(&p));

        // Frame 2: same shape, no edit input — still clean.
        state.note_editor_frame(&p, shape, false);
        assert!(!state.is_dirty(&p));

        // Frame 3: same shape but the editor had focus and text arrived
        // (a same-length replacement the shape check cannot see).
        state.note_editor_frame(&p, shape, true);
        assert!(state.is_dirty(&p));

        // Frame 4: the shape changed (an insert) — stays dirty.
        state.note_editor_frame(
            &p,
            BufferShape {
                len_chars: 8,
                line_count: 1,
            },
            false,
        );
        assert!(state.is_dirty(&p));
    }

    #[test]
    fn dirty_state_is_shared_across_synchronized_panes() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "a.rs", "x");
        state.editor_area.request_split();
        state.note_editor_frame(
            &p,
            BufferShape {
                len_chars: 2,
                line_count: 1,
            },
            false,
        );
        for pane in state.editor_area.pane_ids() {
            assert!(state
                .editor_area
                .pane(pane)
                .is_some_and(|tabs| tabs.is_dirty(&p)));
        }
    }

    // --- close / prune -------------------------------------------------------

    #[test]
    fn close_editor_returns_the_path_and_syncs_the_workspace() {
        let mut state = AppState::default();
        let a = loaded(&mut state, "a.rs", "a");
        let b = loaded(&mut state, "b.rs", "b");
        assert_eq!(state.close_editor(), Some(b));
        assert_eq!(state.active_path().as_deref(), Some(a.as_path()));
        assert_eq!(
            state.workspace.as_ref().unwrap().open_files,
            vec![a.clone()]
        );
    }

    #[test]
    fn prune_closed_drops_the_buffer_unless_another_pane_keeps_the_tab() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "a.rs", "a");
        state.editor_area.request_split(); // two panes, same tab

        // Close the tab in the active pane only: still open in the sibling.
        state.editor_area.request_close();
        state.prune_closed(&p);
        assert!(state.buffers.contains_key(&p));

        // Close it in the survivor too: everything for the path is dropped.
        state.editor_area.request_close();
        state.prune_closed(&p);
        assert!(!state.buffers.contains_key(&p));
        assert!(!state.is_dirty(&p));
        assert!(state
            .editors
            .keys()
            .all(|(_, open): &(PaneId, PathBuf)| *open != p));
    }

    #[test]
    fn switch_folder_closes_the_session_and_resets_the_tree() {
        let mut state = AppState::default();
        state.apply(AppMessage::FolderOpened(path("/old")));
        loaded(&mut state, "old/a.rs", "a");
        state.apply(AppMessage::FileTreeUpdated {
            root: path("/old"),
            tree: FileTreeNode {
                name: "old".into(),
                path: path("/old"),
                is_dir: true,
                children: vec![],
                truncated: false,
            },
        });
        assert!(!state.fuzzy_source.is_empty() || state.file_tree.tree().is_some());

        state.apply(AppMessage::DialogFolderPicked(Some(path("/new"))));
        assert_eq!(
            state.workspace.as_ref().map(|w| w.root.clone()),
            Some(path("/new"))
        );
        assert!(state.buffers.is_empty());
        assert!(state.editors.is_empty());
        assert!(state.fuzzy_source.is_empty());
        assert!(state.file_tree.tree().is_none());
    }

    // --- workspace snapshot ---------------------------------------------------

    #[test]
    fn workspace_snapshot_captures_root_tabs_and_active_path() {
        let mut state = AppState::default();
        state.apply(AppMessage::FolderOpened(path("/repo")));
        let a = loaded(&mut state, "a.rs", "a");
        let b = loaded(&mut state, "b.rs", "b");

        let snapshot = state.workspace_snapshot();
        assert_eq!(snapshot.root, Some(path("/repo")));
        assert_eq!(snapshot.open_files, vec![a, b.clone()]);
        assert_eq!(snapshot.active_path, Some(b));
    }
}
