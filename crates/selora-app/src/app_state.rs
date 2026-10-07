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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use selora_ai::{ChatReceipt, ChatRequest, Model, Role, SeloraStatus};
use selora_core::{Buffer, Position, SeloraError, Workspace};
use selora_editor::Editor;
use selora_fs::search::{SearchError, SearchHit, SearchId, SearchQuery};
use selora_fs::tree::FileTreeNode;
use selora_git::GitStatus;
use selora_settings::Settings;
use selora_ui::editor::split::EditorArea;
use selora_ui::palette::command::CommandPalette;
use selora_ui::palette::fuzzy::FuzzyFileFinder;
use selora_ui::panel::git::GitPanel;
use selora_ui::panel::search::SearchPanel;
use selora_ui::sidebar::{ChatId, ChatPanel, ChatTurn, FileTreePanel};
use selora_ui::status_bar::cost_meter::CostMeterState;
use uuid::Uuid;

use crate::persistence::WorkspaceState;

/// Everything the dispatcher needs to start one agent turn (§17.3):
/// the resolved model, the panel's conversation, and the `agent.*`
/// settings slice (boxed — the large-variant clippy reason documented at
/// `AgentSettings`'s `Settings` field). The shell builds it exactly like
/// the plain-chat `ChatRequest` (the payload travels with the effect —
/// the `SeloraChat` precedent; the panel stays the conversation's single
/// source of truth).
///
/// Adjustment note: the Phase-5 skeleton declared `AgentSend(Uuid,
/// String)`; carrying the full request mirrors `Effect::SeloraChat` and
/// keeps the dispatcher from duplicating panel state.
#[derive(Debug, Clone)]
pub struct AgentTurnRequest {
    /// The resolved catalog model (an unknown id carries
    /// `context_window: 0` — no context cap, never a false abort).
    pub model: Model,
    /// The panel's stable conversation (cards and the in-flight bubble
    /// excluded by `ChatPanel::conversation`).
    pub conversation: Vec<selora_ai::ChatMessage>,
    /// The `agent.*` settings slice (caps, shell, allowlist, Brave key).
    pub agent: Box<selora_settings::AgentSettings>,
}

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
    /// Adopt new settings that are *already on disk* (the Settings dialog
    /// writes the file itself): refresh the dispatcher's Selora base URL
    /// without another write and without a reply. Without this, every
    /// Selora effect keeps the boot-time base URL until the app restarts.
    SettingsSynced {
        base_url: String,
    },

    // Selora (the Phase 1c network half — live until the Phase 4 chat UI
    // constructs them; the dispatcher and the wiremock tests exercise them).
    #[allow(dead_code)]
    SeloraConnect(String),
    #[allow(dead_code)]
    SeloraListModels,
    #[allow(dead_code)]
    SeloraChat(Uuid, ChatRequest),
    /// Ping the gateway for the status-bar connection indicator (Phase
    /// 4f): the worker calls `SeloraClient::status` and replies with
    /// [`AppMessage::SeloraStatus`]. Without a connected key it replies
    /// `Disconnected` without touching the network — `Connected` always
    /// means key present + gateway answered.
    SeloraPing,
    /// Boot-time restore (Phase 4a): the worker reads the OS keyring; a
    /// stored key is validated against `base_url` and becomes the
    /// session's key. Sent by the shell once settings arrive (the base
    /// URL is theirs).
    SeloraAutoConnect {
        base_url: String,
    },
    /// Disconnect (Phase 4a): drop the session key and delete the
    /// keyring entry; the reply resets the status to `Disconnected`.
    SeloraDisconnect,
    /// One ghost-text completion request (Phase 4d, PLAN.md §6.4): the
    /// up-to-caret `prefix` of `path`, to complete with `model`.
    /// `request_caret` rides along so the reply can be stale-dropped by
    /// the editor. The worker replies
    /// [`AppMessage::GhostTextReady`].
    SeloraGhostText {
        path: PathBuf,
        prefix: String,
        model: String,
        request_caret: Position,
    },
    /// Hold the Selora key for this session without validation or
    /// keyring persistence — the wiremock tests' seam (production keys
    /// enter through `SeloraConnect` / `SeloraAutoConnect`).
    #[allow(dead_code)]
    SeloraKeyringSet(String),

    /// "Check for updates" (Phase 6b, UI_SPEC.md §11): the worker GETs the
    /// repo's latest GitHub Release and replies [`AppMessage::UpdateChecked`].
    CheckForUpdates,

    // Agent Mode (ARCHITECTURE.md §17.6 — additive; 5b/5c/5d consume,
    // the orchestrator owns this shared file). The request `Uuid` keys
    // the reply messages, the same `chat_ids` pattern as `SeloraChat`.
    /// Start an agent turn (§17.3): the loop streams the conversation
    /// with the fixed tool manifest armed, executing tools through the
    /// router rooted at the workspace.
    AgentSend(Uuid, AgentTurnRequest),
    /// The user's answer to an approval card (§17.4).
    AgentDecision(Uuid, selora_ai::ApprovalId, selora_ai::AgentDecision),

    // Git (Phase 3b)
    /// Snapshot the status of the repo containing `root` (git2 discover;
    /// resolves to [`AppMessage::GitNoRepo`] when there is no repo).
    GitStatus(PathBuf),
    /// Stage one file (absolute path — a `GitStatus` entry's `abs_path`).
    GitStage(PathBuf),
    /// Unstage one file (absolute path).
    GitUnstage(PathBuf),
    /// Commit the index of the repo containing `root`.
    GitCommit(PathBuf, String),

    // Search (Phase 3c)
    /// Run a workspace search off the frame path; the reply is
    /// [`AppMessage::SearchResults`] with the same id (stale replies are
    /// dropped by the panel).
    Search {
        id: SearchId,
        root: PathBuf,
        query: SearchQuery,
    },

    // LSP (Phase 3a)
    /// Forward a language-server effect to the LSP manager task (opened by
    /// the shell once settings + workspace root are known).
    Lsp(selora_lsp::LspEffect),
}

/// Messages workers send to the main thread (ARCHITECTURE.md §2.1).
#[derive(Debug)]
pub enum AppMessage {
    // Selora
    ModelsLoaded(Result<Vec<Model>, SeloraError>),
    ChatDelta(Uuid, String),
    ChatDone(Uuid, ChatReceipt),
    ChatError(Uuid, SeloraError),
    /// The latest gateway ping result (Phase 4f): the status-bar right
    /// zone's connection state. Sent by the ping worker the shell's
    /// 30s loop drives; `Connected` implies both gateway reachable and
    /// key present (the worker does not ping without a key).
    SeloraStatus(SeloraStatus),
    /// A ghost-text completion reply (Phase 4d): the suggestion for
    /// `path`, requested at `request_caret` — the editor stale-drops a
    /// reply whose caret no longer matches (buffer keyed by path, not
    /// the arch doc's BufferId, to match this codebase).
    GhostTextReady {
        path: PathBuf,
        request_caret: Position,
        suggestion: Option<String>,
    },

    // Agent Mode (ARCHITECTURE.md §17.6 — additive; the request `Uuid`
    // keys the turn, the same `chat_ids` pattern as the chat messages.
    // `ToolResult` carries its own `tool_call_id`, so the variant does
    // not repeat it).
    /// The loop started executing one tool call (a read-type card opens
    /// immediately; the collapsed card paints).
    ToolCallStarted(Uuid, selora_ai::ToolCall),
    /// One tool call finished (approved-and-executed, auto-run, or
    /// denied → `ok: false`).
    ToolResult(Uuid, selora_ai::ToolResult),
    /// The loop parked on an approval card (§17.4): the UI renders the
    /// diff/command/delete card; the user's answer returns as
    /// [`Effect::AgentDecision`].
    ApprovalNeeded(Uuid, selora_ai::ApprovalId, selora_ai::ApprovalCard),
    /// The turn ended: the receipt feeds the cost meter (§17.3), the
    /// summary paints the footer (UI_SPEC.md §4.2), and a non-`Completed`
    /// end toasts its reason (`TurnEnd::toast`).
    ///
    /// Adjustment note: the Phase-5 skeleton carried three fields; the
    /// [`selora_ai::TurnEnd`] is additive-in-spirit — §17.3 requires the
    /// abort reason in a toast and neither the receipt nor the summary
    /// carries it.
    AgentTurnDone(
        Uuid,
        ChatReceipt,
        selora_ai::AgentSummary,
        selora_ai::TurnEnd,
    ),

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
    #[allow(dead_code)]
    KeyringError(SeloraError),
    /// The boot keyring read found no stored key (Phase 6a): the shell's
    /// cue to show the first-run splash (UI_SPEC.md §7). Sent by the
    /// `SeloraAutoConnect` worker — and only by it — so it is never
    /// confused with the ping loop's keyless `Disconnected` replies.
    NoStoredKey,
    /// The update check resolved (Phase 6b): the toast names the newer tag
    /// (with the release page as its link), reports "up to date", or
    /// surfaces the error's words.
    UpdateChecked(Result<selora_ai::ReleaseInfo, String>),

    // Git (Phase 3b)
    /// A fresh status snapshot (also sent after every stage/unstage/commit
    /// so the panel and the tree badges refresh).
    GitStatusChanged(GitStatus),
    /// The open folder is not inside a git repository (not an error).
    GitNoRepo,
    /// A commit landed; the payload is the short oid, for the toast.
    GitCommitted(String),
    /// A git effect failed — toast (PLAN.md §8).
    GitError(SeloraError),

    // Search (Phase 3c)
    /// The results of search `id` — `Ok` installs them in the panel
    /// (stale ids dropped), `Err` toasts and clears the running flag.
    SearchResults {
        id: SearchId,
        results: Result<Vec<SearchHit>, SearchError>,
    },

    // LSP (Phase 3a)
    /// A language-server event (status, diagnostics, hover, definition,
    /// completions, progress) — reduced by [`AppState::apply_lsp`].
    Lsp(selora_lsp::LspEvent),
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
    /// An optional action link, painted as a trailing hyperlink (Phase 6b:
    /// the update-available toast's "download"). eframe opens it in the
    /// system browser.
    pub link: Option<(String, String)>,
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
    /// Gateway model catalog (Phase 4): the model picker's list source
    /// (also the price table the cost meter resolves receipts against).
    pub models: Vec<Model>,
    /// The chat sidebar's view state (Phase 4c): transcript, composer,
    /// streaming bubble. Driven by [`AppMessage::ChatDelta`] /
    /// [`ChatDone`] / [`ChatError`] / [`ModelsLoaded`]; sends go out as
    /// [`Effect::SeloraChat`] from the shell.
    pub chat_panel: ChatPanel,
    /// The live chat request map: the `Uuid` of each dispatched
    /// `Effect::SeloraChat` → the panel's [`ChatId`] it was sent for.
    /// Entries leave at the stream's terminal message; unknown ids are
    /// stale replies and dropped.
    pub chat_ids: BTreeMap<Uuid, ChatId>,
    /// The in-flight agent turn's `write_file`/`edit_file` calls, stashed
    /// at [`AppMessage::ToolCallStarted`] as `tool_call_id → (name,
    /// arguments)` so the [`AppMessage::ToolResult`] arm can sync the open
    /// buffer (§17.7 — the card is the save moment). One turn at a time;
    /// cleared at turn end and on folder switch.
    agent_write_calls: BTreeMap<String, (String, String)>,
    /// §17.7 + the 5d brief's deliverable 13: per-path `(pre, post)`
    /// snapshot stacks backing `Ctrl+Z` for agent edits. Agent-edit-scoped
    /// undo, NOT a general undo (the buffer has no undo log yet — human
    /// typing stays non-undoable; when §3.4's UndoLog lands this migrates
    /// into it). A restore applies ONLY while the buffer still equals the
    /// snapshot's post-edit text — a human edit since invalidates the
    /// stack, never a clobber.
    agent_edits: BTreeMap<PathBuf, Vec<(String, String)>>,
    /// Paths whose buffer text changed outside the editor's own input
    /// (an agent sync or undo): the shell drains this each frame and
    /// sends `LspEffect::Change` for didOpen'd paths — the shape-compare
    /// at the dirty/LSP seam cannot see same-length replacements.
    agent_touched: BTreeSet<PathBuf>,
    /// Gateway connection state for the status-bar right zone (Phase 4f):
    /// driven by [`AppMessage::SeloraStatus`] from the 30s ping loop;
    /// `Connected` implies a key is present and the gateway answered.
    pub selora_status: SeloraStatus,
    /// The `Σ today` running total + per-model breakdown (Phase 4e),
    /// stepped from [`AppMessage::ChatDone`]; painted by the shell's
    /// status bar right zone.
    pub cost_meter: CostMeterState,
    /// Latest git status snapshot; `None` until the first
    /// [`AppMessage::GitStatusChanged`] (Phase 3b).
    pub git_status: Option<GitStatus>,
    /// Whether the workspace is inside a git repository at all.
    pub in_git_repo: bool,
    /// The git panel's view state (commit draft, selection, sections;
    /// Phase 3b). Data comes from `git_status`; actions go out as
    /// `Effect`s via the shell.
    pub git_panel: GitPanel,
    /// The search panel's view state (query, toggles, collapsed groups;
    /// Phase 3c). Results arrive via [`AppMessage::SearchResults`];
    /// searches go out as [`Effect::Search`] via the shell.
    pub search: SearchPanel,
    /// LSP view state (Phase 3a): server status, the progress line, the
    /// hover card, and the completion popup — the status-bar and overlay
    /// data the shell paints.
    pub lsp: LspViewState,
    /// The F12 definition to jump to once its file is open (a definition
    /// into a not-yet-loaded file is opened first, then jumped).
    pub pending_definition: Option<selora_lsp::LspLocation>,
}

/// Phase 3a LSP view state (status bar, hover card, completion popup).
#[derive(Debug, Default)]
pub struct LspViewState {
    /// Per-language server status (status bar).
    pub status: BTreeMap<selora_lsp::LanguageId, selora_lsp::ServerStatus>,
    /// The latest work-done progress line, status bar center
    /// ("rust-analyzer: Indexing 42%").
    pub progress: Option<String>,
    /// Shaped hover tooltip content, for the path it was requested on.
    pub hover: Option<(PathBuf, Vec<String>)>,
    /// The open completion popup, for the path it was requested on.
    pub completion: Option<(PathBuf, selora_editor::lsp_overlay::CompletionPopup)>,
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
            chat_panel: ChatPanel::new(),
            chat_ids: BTreeMap::new(),
            agent_write_calls: BTreeMap::new(),
            agent_edits: BTreeMap::new(),
            agent_touched: BTreeSet::new(),
            selora_status: SeloraStatus::Disconnected,
            cost_meter: CostMeterState::new(),
            git_status: None,
            in_git_repo: false,
            git_panel: GitPanel::new(),
            search: SearchPanel::new(),
            lsp: LspViewState::default(),
            pending_definition: None,
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
                self.chat_panel.set_models(models.clone());
                self.models = models;
            }
            AppMessage::ModelsLoaded(Err(error)) => {
                self.toast(format!("selora: {error}"), Severity::Error);
            }
            AppMessage::ChatDelta(chat_uuid, delta) => {
                // An unknown uuid is a stale reply (the map entry left at
                // the stream's terminal message) — dropped, not appended.
                if let Some(chat_id) = self.chat_ids.get(&chat_uuid) {
                    let chat_id = *chat_id;
                    self.chat_panel.push_assistant_delta(chat_id, &delta);
                }
            }
            AppMessage::ChatDone(chat_uuid, receipt) => {
                if let Some(chat_id) = self.chat_ids.remove(&chat_uuid) {
                    self.chat_panel.complete_stream(chat_id);
                }
                // Feed the cost meter (Phase 4e): the receipt's own cost
                // when the gateway reported one, else the price table of
                // the model resolved from the catalog (never a guess).
                let model = self.models.iter().find(|model| model.id == receipt.model);
                self.cost_meter.add_receipt(
                    &receipt,
                    model,
                    selora_ui::status_bar::cost_meter::day_number(SystemTime::now()),
                );
            }
            AppMessage::ChatError(chat_uuid, error) => {
                if let Some(chat_id) = self.chat_ids.remove(&chat_uuid) {
                    self.chat_panel.fail_stream(chat_id, &error.to_string());
                }
                self.toast(format!("chat: {error}"), Severity::Error);
            }
            AppMessage::SeloraStatus(status) => {
                // Phase 6h: the presence avatars sleep while the gateway
                // is unreachable (Connecting counts as reachable — a ping
                // is in flight, not a failure).
                self.chat_panel.set_gateway_connected(matches!(
                    status,
                    selora_ai::SeloraStatus::Connected { .. } | selora_ai::SeloraStatus::Connecting
                ));
                self.selora_status = status;
            }
            AppMessage::GhostTextReady {
                path,
                request_caret,
                suggestion,
            } => {
                // Offer the suggestion to every pane's editor for the
                // path: the editor whose caret still matches the request
                // keeps it, the rest stale-drop it (the ghost is
                // paint-only until Tab, ARCHITECTURE.md §9.2).
                for ((_pane, tab_path), editor) in self.editors.iter_mut() {
                    if tab_path == &path {
                        editor.set_ghost(suggestion.clone(), request_caret);
                    }
                }
            }

            // -- Agent Mode (Phase 5) -------------------------------------------
            // All four arms gate on `chat_ids`: a message for an unknown or
            // ended turn is stale and dropped, never a panic (the
            // `ChatDelta` precedent above).
            AppMessage::ToolCallStarted(uuid, call) => {
                if let Some(chat_id) = self.chat_ids.get(&uuid) {
                    let chat_id = *chat_id;
                    // Stash write/edit arguments for the §17.7 buffer sync
                    // when the (approved) result lands.
                    if matches!(call.name.as_str(), "write_file" | "edit_file") {
                        self.agent_write_calls
                            .insert(call.id.clone(), (call.name.clone(), call.arguments.clone()));
                    }
                    self.chat_panel.tool_call_started(chat_id, call);
                }
            }
            AppMessage::ToolResult(uuid, result) => {
                if let Some(chat_id) = self.chat_ids.get(&uuid) {
                    let chat_id = *chat_id;
                    // §17.7: an approved write/edit to an OPEN buffer also
                    // updates the in-memory text (clean buffers only —
                    // never a clobber; see sync_agent_write). A denied or
                    // failed call (`ok: false`) never touches buffers.
                    if let Some((name, arguments)) = self.agent_write_calls.remove(&result.id) {
                        if result.ok {
                            self.sync_agent_write(&name, &arguments);
                        }
                    }
                    self.chat_panel.tool_result(chat_id, result);
                }
            }
            AppMessage::ApprovalNeeded(uuid, approval_id, card) => {
                if let Some(chat_id) = self.chat_ids.get(&uuid) {
                    let chat_id = *chat_id;
                    self.chat_panel.approval_needed(chat_id, approval_id, card);
                }
            }
            AppMessage::AgentTurnDone(uuid, receipt, summary, end) => {
                // A stale/late/duplicate turn end drops WHOLE: no panel
                // reduction, no meter movement, no toast (the arm-block
                // contract above; the brief's "late/stale request ids are
                // dropped"). The relay emits at most one TurnDone per turn
                // keyed by a live chat_ids entry, so a miss means the turn
                // is not ours to account.
                let Some(chat_id) = self.chat_ids.remove(&uuid) else {
                    return;
                };
                self.chat_panel.agent_turn_done(chat_id, summary);
                // The turn's receipt feeds the cost meter exactly like
                // ChatDone (the relay synthesized it from the loop's own
                // totals; `estimated` — an aggregate, not a
                // gateway-verbatim receipt).
                let model = self.models.iter().find(|model| model.id == receipt.model);
                self.cost_meter.add_receipt(
                    &receipt,
                    model,
                    selora_ui::status_bar::cost_meter::day_number(SystemTime::now()),
                );
                // §17.3: an abnormal end toasts the reason (caps, Stop,
                // failure).
                if !end.is_completed() {
                    let severity = match &end {
                        selora_ai::TurnEnd::Failed(_) => Severity::Error,
                        _ => Severity::Info,
                    };
                    self.toast(end.toast(), severity);
                }
                // The turn's write-call stash dies with it (one in-flight
                // turn at a time).
                self.agent_write_calls.clear();
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
                    self.workspace = Some(Workspace::new(root.clone()));
                }
                self.file_tree.set_tree(tree);
                // The search panel displays results relative to the root.
                self.search.set_root(root);
                self.refresh_fuzzy_source();
            }
            AppMessage::BufferLoaded { path, text } => {
                let buffer = Arc::new(RwLock::new(Buffer::from_text(&text)));
                let shape = BufferShape::of(&buffer.read().unwrap_or_else(|e| e.into_inner()));
                self.buffers.insert(path.clone(), buffer);
                self.saved_shapes.insert(path.clone(), shape);
                self.last_shapes.insert(path.clone(), shape);
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
            // Shell-only signal (Phase 6a first-run splash): no document
            // state follows from it.
            AppMessage::NoStoredKey => {}
            // Phase 6b (UI_SPEC.md §11): the update check's result is a
            // toast — the newer tag with its release page as "download",
            // "up to date", or the error's words.
            AppMessage::UpdateChecked(result) => match result {
                Ok(info) if info.newer => {
                    self.toast_with_link(
                        format!("{} is available", info.tag),
                        Severity::Info,
                        ("download".to_owned(), info.html_url),
                    );
                }
                Ok(_) => self.toast("Selora IDE is up to date", Severity::Info),
                Err(error) => self.toast(format!("update check failed: {error}"), Severity::Error),
            },

            // -- Git (Phase 3b) -------------------------------------------------
            AppMessage::GitStatusChanged(status) => {
                self.in_git_repo = true;
                self.file_tree.set_git_status(Some(&status));
                self.git_status = Some(status);
            }
            AppMessage::GitNoRepo => {
                self.in_git_repo = false;
                self.git_status = None;
                self.file_tree.set_git_status(None);
            }
            AppMessage::GitCommitted(short_id) => {
                self.toast(format!("committed {short_id}"), Severity::Info);
            }
            AppMessage::GitError(error) => {
                self.toast(format!("git: {error}"), Severity::Error);
            }

            // -- Search (Phase 3c) ----------------------------------------------
            AppMessage::SearchResults { id, results } => match results {
                Ok(hits) => self.search.set_results(id, hits),
                Err(error) => {
                    self.search.search_failed(id);
                    self.toast(format!("search: {error}"), Severity::Error);
                }
            },

            // -- LSP (Phase 3a) ---------------------------------------------------
            AppMessage::Lsp(event) => self.apply_lsp(event),
        }
    }

    /// Reduce one [`LspEvent`] (Phase 3a). Diagnostics are applied to every
    /// pane's editor for the path (buffers are shared per path); hover and
    /// completions become shell-paintable view state; a definition into an
    /// unopened file is stashed for the shell to open-then-jump.
    fn apply_lsp(&mut self, event: selora_lsp::LspEvent) {
        use selora_editor::lsp_overlay as view;
        use selora_lsp::LspEvent;

        match event {
            LspEvent::ServerStatus { server, status } => {
                if let selora_lsp::ServerStatus::Dead(reason) = &status {
                    self.toast(
                        format!("{}: {reason}", server.server_name()),
                        Severity::Error,
                    );
                }
                self.lsp.status.insert(server, status);
            }
            LspEvent::ServerMissing { server } => {
                self.toast(
                    format!(
                        "{} not found; install it or set the path in settings",
                        server.server_name()
                    ),
                    Severity::Info,
                );
            }
            LspEvent::Progress { server: _, message } => {
                self.lsp.progress = Some(message);
            }
            LspEvent::Diagnostics { path, diagnostics } => {
                let mapped: Vec<view::LspDiagnostic> = diagnostics
                    .into_iter()
                    .map(|diagnostic| view::LspDiagnostic {
                        range: view::LspRange {
                            start: view::LspPosition {
                                line: diagnostic.range.start.line,
                                character: diagnostic.range.start.character,
                            },
                            end: view::LspPosition {
                                line: diagnostic.range.end.line,
                                character: diagnostic.range.end.character,
                            },
                        },
                        severity: match diagnostic.severity {
                            selora_lsp::LspSeverity::Error => view::LspSeverity::Error,
                            selora_lsp::LspSeverity::Warning => view::LspSeverity::Warning,
                            selora_lsp::LspSeverity::Information => view::LspSeverity::Information,
                            selora_lsp::LspSeverity::Hint => view::LspSeverity::Hint,
                        },
                        source: diagnostic.source,
                        message: diagnostic.message,
                    })
                    .collect();
                // Every pane showing this buffer shares it by path.
                for ((_pane, tab_path), editor) in self.editors.iter_mut() {
                    if tab_path == &path {
                        editor.set_diagnostics(mapped.clone());
                    }
                }
            }
            LspEvent::Hover {
                path,
                position: _,
                content,
            } => {
                self.lsp.hover =
                    content.map(|text| (path, selora_editor::lsp_overlay::shape_hover(&text)));
            }
            LspEvent::Definition { locations, .. } => {
                // The first location wins (Lite has no result picker); the
                // shell resolves the jump (open-then-jump if needed).
                self.pending_definition = locations.into_iter().next();
            }
            LspEvent::Completions { path, items, .. } => {
                let entries = items
                    .into_iter()
                    .map(|item| view::CompletionEntry {
                        label: item.label,
                        kind: item.kind,
                        detail: item.detail,
                    })
                    .collect();
                self.lsp.completion = Some((path, view::CompletionPopup::new(entries)));
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

    /// Jump every pane's editor that shows `path` to a **1-based** line (a
    /// search-result click, Phase 3c; later also the problems panel and
    /// go-to-definition). Returns whether any editor jumped — `false`
    /// means the buffer is not open yet and the caller should open it.
    pub fn goto_line(&mut self, path: &Path, line_1based: usize) -> bool {
        let panes = self.editor_area.pane_ids();
        let mut jumped = false;
        for pane in panes {
            if let Some(editor) = self.editors.get_mut(&(pane, path.to_path_buf())) {
                editor.goto_line(line_1based);
                jumped = true;
            }
        }
        jumped
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
    pub fn prune_closed(&mut self, path: &Path) -> bool {
        let still_open = self.editor_area.pane_ids().iter().any(|pane| {
            self.editor_area
                .pane(*pane)
                .is_some_and(|tabs| tabs.position(path).is_some())
        });
        if !still_open {
            self.buffers.remove(path);
            self.saved_shapes.remove(path);
            self.last_shapes.remove(path);
            self.editors.retain(|open_tab, _| open_tab.1 != path);
            return true;
        }
        false
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
        self.git_status = None;
        self.in_git_repo = false;
        self.search = SearchPanel::new();
        self.lsp.hover = None;
        self.lsp.completion = None;
        self.lsp.progress = None;
        self.pending_definition = None;
        // The conversation belongs to the old workspace (Phase 4c).
        self.chat_panel.clear();
        self.chat_ids.clear();
        // …and so do the agent turn's stashes (Phase 5d).
        self.agent_write_calls.clear();
        self.agent_edits.clear();
        self.agent_touched.clear();
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

    /// Whether the tab for `path` is dirty. The §17.7 sync guards on
    /// this (a dirty buffer is never touched by an agent write) — and the
    /// shell's dirty *derivation* is [`Self::note_editor_frame`].
    pub fn is_dirty(&self, path: &Path) -> bool {
        self.editor_area.pane_ids().iter().any(|pane| {
            self.editor_area
                .pane(*pane)
                .is_some_and(|t| t.is_dirty(path))
        })
    }

    /// The shape baseline the buffer had when it was last loaded/saved
    /// (tests assert the reset-on-save behavior).
    #[allow(dead_code)]
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
            link: None,
        };
        self.toasts.push(toast);
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    /// A toast carrying an action link (UI_SPEC.md §13 + §11: the
    /// update-available toast's "download"). `(label, url)`.
    pub fn toast_with_link(
        &mut self,
        text: impl Into<String>,
        severity: Severity,
        link: (String, String),
    ) {
        self.toast(text, severity);
        if let Some(toast) = self.toasts.last_mut() {
            toast.link = Some(link);
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
    /// The `workspace.json` snapshot (§8.2): root, tabs in order, the
    /// active file, and the chat sidebar's state (Phase 4c) — the shell
    /// passes its visibility flag.
    pub fn workspace_snapshot(&self, chat_open: bool) -> WorkspaceState {
        WorkspaceState {
            root: self
                .workspace
                .as_ref()
                .map(|workspace| workspace.root.clone()),
            open_files: self
                .workspace
                .as_ref()
                .map(|workspace| workspace.open_files.clone())
                .unwrap_or_default(),
            active_path: self.active_path(),
            chat_open,
            chat_turns: self
                .chat_panel
                .snapshot_turns()
                .into_iter()
                .map(|turn| crate::persistence::ChatTurnState {
                    role: match turn.role {
                        Role::User => "user".to_owned(),
                        Role::Assistant => "assistant".to_owned(),
                        Role::System => "system".to_owned(),
                        // Tool-result turns are loop artifacts, not chat
                        // bubbles — `restore_chat` drops them below.
                        Role::Tool => "tool".to_owned(),
                    },
                    content: turn.content,
                })
                .collect(),
            // The header's Agent Mode pill (Phase 5d) persists like
            // chat_open.
            agent_mode: self.chat_panel.is_agent_mode(),
        }
    }

    /// Rebuild the chat transcript from persisted turns (boot restore,
    /// Phase 4c). Unknown role tags are dropped, not fatal.
    pub fn restore_chat(&mut self, turns: Vec<crate::persistence::ChatTurnState>) {
        let restored: Vec<ChatTurn> = turns
            .into_iter()
            .filter_map(|turn| match turn.role.as_str() {
                "user" => Some(ChatTurn::user(turn.content)),
                "assistant" => Some(ChatTurn::assistant(turn.content)),
                _ => None,
            })
            .collect();
        self.chat_panel.restore_turns(restored);
    }

    // -- §17.7: agent writes meet open buffers --------------------------------

    /// An approved `write_file`/`edit_file` to a file that is OPEN in a
    /// buffer also updates the in-memory text (the diff card was the disk
    /// write; a synced buffer ends matching disk — clean). A DIRTY buffer
    /// (unsaved user edits) is never touched: the disk write already
    /// happened, so the honest outcome is a toast, never a clobber.
    fn sync_agent_write(&mut self, name: &str, arguments: &str) {
        // The router already validated these arguments; a parse failure
        // here is defensive-only (never a panic).
        let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) else {
            return;
        };
        let Some(rel) = args.get("path").and_then(serde_json::Value::as_str) else {
            return;
        };
        let Some(path) = self.resolve_open_buffer(rel) else {
            return; // not open — the disk write stands alone
        };
        if self.is_dirty(&path) {
            self.toast(
                format!(
                    "agent wrote {rel} to disk; the open buffer has unsaved changes — reload to see it"
                ),
                Severity::Info,
            );
            return;
        }
        let Some(buffer) = self.buffers.get(&path).cloned() else {
            return;
        };
        let pre = buffer.read().unwrap_or_else(|e| e.into_inner()).text();
        // The buffer is clean, so it equals the disk text the router
        // computed its diff against — the transform is exact.
        let post = match name {
            "write_file" => args
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            "edit_file" => {
                let old_str = args.get("old_str").and_then(serde_json::Value::as_str);
                let new_str = args.get("new_str").and_then(serde_json::Value::as_str);
                let replace_all = args
                    .get("replace_all")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                match (old_str, new_str) {
                    (Some(old_str), Some(new_str)) => {
                        if pre.contains(old_str) {
                            Some(if replace_all {
                                pre.replace(old_str, new_str)
                            } else {
                                pre.replacen(old_str, new_str, 1)
                            })
                        } else {
                            // Belt-and-braces: a clean buffer should hold
                            // old_str verbatim. Toast rather than guess.
                            self.toast(
                                format!(
                                    "agent edit to {rel} did not apply to the open buffer — reload to see it"
                                ),
                                Severity::Info,
                            );
                            return;
                        }
                    }
                    _ => return,
                }
            }
            _ => return,
        };
        let Some(post) = post else { return };
        if post == pre {
            return; // a no-op write: nothing to sync, nothing to undo
        }

        // Mutate the shared buffer in place (the Arc stays; every pane's
        // editor keeps its clamped cursor), then resync highlighting.
        replace_buffer_text(&buffer, &post);
        for ((_pane, tab_path), editor) in self.editors.iter_mut() {
            if tab_path == &path {
                editor.resync_syntax();
            }
        }
        // The undo snapshot (Ctrl+Z restores `pre` while the buffer still
        // equals `post`).
        self.agent_edits
            .entry(path.clone())
            .or_default()
            .push((pre, post));
        // The buffer now matches disk exactly: rebase the dirty baselines
        // so the tab ends CLEAN (approving the card was the save).
        let shape = BufferShape::of(&buffer.read().unwrap_or_else(|e| e.into_inner()));
        self.saved_shapes.insert(path.clone(), shape);
        self.last_shapes.insert(path.clone(), shape);
        self.editor_area.set_dirty(&path, false);
        self.mark_agent_touched(&path);
    }

    /// The open buffer for a workspace-relative tool path: the root join
    /// first, then a normalized `strip_prefix` scan over the open buffers
    /// (buffer keys come from the shell's open paths; the router resolves
    /// against the canonical root — the two can differ by prefix
    /// normalization on Windows).
    fn resolve_open_buffer(&self, rel: &str) -> Option<PathBuf> {
        let workspace = self.workspace.as_ref()?;
        let joined = workspace.root.join(rel);
        if self.buffers.contains_key(&joined) {
            return Some(joined);
        }
        let rel = rel.replace('\\', "/");
        let root = normalize_path(&workspace.root);
        self.buffers
            .keys()
            .find(|path| {
                normalize_path(path)
                    .strip_prefix(&root)
                    .map(str::to_owned)
                    .is_some_and(|rest| rest.trim_start_matches('/') == rel)
            })
            .cloned()
    }

    /// `Ctrl+Z` on an agent-edited buffer (the 5d brief's deliverable 13):
    /// restore the pre-edit text of the newest snapshot whose post-edit
    /// text is still on screen — a human edit since invalidates the path's
    /// stack (a no-op, never a clobber). Disk is never touched (the card
    /// was the write); the buffer ends dirty against disk. Returns whether
    /// a snapshot applied.
    pub fn undo_agent_edit(&mut self, path: &Path) -> bool {
        let Some(buffer) = self.buffers.get(path).cloned() else {
            return false;
        };
        let current = buffer.read().unwrap_or_else(|e| e.into_inner()).text();
        let Some(stack) = self.agent_edits.get_mut(path) else {
            return false;
        };
        let Some(index) = stack.iter().rposition(|(_, post)| *post == current) else {
            // The buffer moved on: the whole stack is invalidated.
            stack.clear();
            return false;
        };
        let (pre, _) = stack.remove(index);
        // Entries above the matched one referred to texts that no longer
        // exist — they die with it.
        stack.truncate(index);
        if stack.is_empty() {
            self.agent_edits.remove(path);
        }
        replace_buffer_text(&buffer, &pre);
        for ((_pane, tab_path), editor) in self.editors.iter_mut() {
            if tab_path == path {
                editor.resync_syntax();
            }
        }
        // A same-length undo is shape-invisible — mark dirty explicitly
        // (the shape-compare caveat in the module docs).
        self.editor_area.set_dirty(path, true);
        self.mark_agent_touched(path);
        true
    }

    /// Mark a buffer as changed by an agent sync/undo (the shape-compare
    /// at the dirty/LSP seam cannot see same-length replacements).
    fn mark_agent_touched(&mut self, path: &Path) {
        self.agent_touched.insert(path.to_path_buf());
    }

    /// Drain the agent-touched paths (the shell sends `LspEffect::Change`
    /// for each didOpen'd one, each frame after the message drain).
    pub fn take_agent_touched(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.agent_touched)
            .into_iter()
            .collect()
    }
}

/// Replace a shared buffer's whole text in place (§17.7): the Arc stays,
/// so every pane's editor keeps its (clamped) cursor.
fn replace_buffer_text(buffer: &Arc<RwLock<Buffer>>, text: &str) {
    let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
    // `remove` clamps both ends (buffer.rs:147-155): `usize::MAX` resolves
    // to the buffer's end — including a trailing newline's final position.
    buffer.remove(Position::new(0, 0), Position::new(usize::MAX, usize::MAX));
    buffer.insert(Position::new(0, 0), text);
}

/// A path with Windows separators and the `\\?\` verbatim prefix
/// normalized away (the resolve fallback's comparison form).
fn normalize_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .strip_prefix("//?/")
        .map(str::to_owned)
        .unwrap_or(normalized)
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

#[cfg(test)]
mod tests {
    use super::*;
    use selora_ui::editor::split::PaneId;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    fn loaded(state: &mut AppState, name: &str, text: &str) -> PathBuf {
        let loaded_path = path(name);
        state.apply(AppMessage::FolderOpened(path("/repo")));
        state.apply(AppMessage::BufferLoaded {
            path: loaded_path.clone(),
            text: text.to_owned(),
        });
        loaded_path
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
            state.workspace.as_ref().and_then(|w| w.active_path.clone()),
            Some(p.clone())
        );
        assert!(!state.is_dirty(&p));
        // The clean baseline matches the loaded shape.
        assert_eq!(
            state.saved_shape(&p),
            Some(BufferShape::of(&Buffer::from_text("fn main() {}")))
        );
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
        state.note_editor_frame(&p, BufferShape::of(&Buffer::from_text("one\ntwo")), false);

        // The user edits the shared buffer (the worker saves *its* text).
        {
            let buffer = state.buffers.get(&p).cloned().expect("buffer");
            buffer
                .write()
                .expect("lock")
                .insert(selora_core::Position::new(1, 3), "\nthree");
        }
        state.note_editor_frame(
            &p,
            BufferShape::of(&Buffer::from_text("one\ntwo\nthree")),
            false,
        );
        assert!(state.is_dirty(&p));

        // The write completed; the worker replies BufferSaved.
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
        assert_eq!(
            state
                .toasts
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>(),
            ["b", "c", "d"]
        );
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
        // After the info lifetime (4s) only the error toast survives, and
        // the reported expiry is its own (8s), still in the future.
        let later = now + Duration::from_secs(5);
        let next_error = state.expire_toasts(later);
        assert!(next_error.is_some_and(|expiry| expiry > later));
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].severity, Severity::Error);
        // And past its lifetime too, everything is gone.
        let much_later = now + Duration::from_secs(9);
        assert_eq!(state.expire_toasts(much_later), None);
        assert!(state.toasts.is_empty());
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
            state.workspace.as_ref().expect("workspace").open_files,
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

    // --- git (Phase 3b wiring) ------------------------------------------------

    /// A status snapshot installs the panel data and the tree badges; the
    /// no-repo marker clears both.
    #[test]
    fn git_messages_drive_panel_data_and_tree_badges() {
        let mut state = AppState::default();
        state.apply(AppMessage::FolderOpened(path("/repo")));

        let status = selora_git::GitStatus {
            branch: Some("main".into()),
            entries: vec![selora_git::FileStatus {
                abs_path: path("/repo/src/a.rs"),
                path: path("src/a.rs"),
                staged: selora_git::Slot::Unmodified,
                worktree: selora_git::Slot::Modified,
            }],
        };
        state.apply(AppMessage::GitStatusChanged(status));
        assert!(state.in_git_repo);
        assert_eq!(
            state.git_status.as_ref().and_then(|s| s.branch.clone()),
            Some("main".into())
        );
        // The tree can map the changed file to its badge.
        assert_eq!(
            state.file_tree.git_decoration(&path("/repo/src/a.rs")),
            Some(('M', selora_ui::theme::color::WARN))
        );

        state.apply(AppMessage::GitNoRepo);
        assert!(!state.in_git_repo);
        assert!(state.git_status.is_none());
        assert_eq!(
            state.file_tree.git_decoration(&path("/repo/src/a.rs")),
            None
        );
    }

    #[test]
    fn git_committed_and_git_error_toast() {
        let mut state = AppState::default();
        state.apply(AppMessage::GitCommitted("abc1234".into()));
        state.apply(AppMessage::GitError(SeloraError::Git("bad object".into())));
        assert_eq!(state.toasts.len(), 2);
        assert_eq!(state.toasts[0].text, "committed abc1234");
        assert_eq!(state.toasts[0].severity, Severity::Info);
        assert_eq!(state.toasts[1].severity, Severity::Error);
        assert!(state.toasts[1].text.contains("git"));
    }

    /// A folder switch resets the git surface along with the tree.
    #[test]
    fn switch_folder_resets_git_state() {
        let mut state = AppState::default();
        state.apply(AppMessage::FolderOpened(path("/old")));
        state.apply(AppMessage::GitStatusChanged(selora_git::GitStatus {
            branch: Some("main".into()),
            entries: vec![],
        }));
        assert!(state.in_git_repo);

        state.apply(AppMessage::DialogFolderPicked(Some(path("/new"))));
        assert!(!state.in_git_repo);
        assert!(state.git_status.is_none());
    }

    // --- search (Phase 3c wiring) ----------------------------------------------

    /// Search results install for the current id, stale replies are
    /// dropped, and a failure toasts (PLAN.md §8).
    #[test]
    fn search_results_install_fresh_and_drop_stale() {
        let mut state = AppState::default();
        state.search.set_query("rope");
        let run = state.search.submit_query().expect("run");
        let id = match run {
            selora_ui::panel::search::SearchEvent::RunSearch { id, .. } => id,
            other => panic!("{other:?}"),
        };
        assert!(state.search.is_running());

        state.apply(AppMessage::SearchResults {
            id,
            results: Ok(vec![selora_fs::search::SearchHit {
                path: path("/repo/a.rs"),
                line_number: 7,
                line_text: "let r = Rope::new();".into(),
                spans: vec![],
            }]),
        });
        assert!(!state.search.is_running());
        assert_eq!(state.search.hit_count(), 1);
        assert!(state.toasts.is_empty());

        // A stale reply (a second Enter raced the first worker): ignored.
        let stale = selora_fs::search::SearchId::next();
        state.apply(AppMessage::SearchResults {
            id: stale,
            results: Ok(vec![]),
        });
        assert_eq!(state.search.hit_count(), 1);

        // A failure for the current id: toast + not running.
        let failed_id = match state.search.submit_query().expect("second run") {
            selora_ui::panel::search::SearchEvent::RunSearch { id, .. } => id,
            other => panic!("{other:?}"),
        };
        state.apply(AppMessage::SearchResults {
            id: failed_id,
            results: Err(selora_fs::search::SearchError::Rg("boom".into())),
        });
        assert!(!state.search.is_running());
        assert_eq!(state.toasts.len(), 1);
        assert!(state.toasts[0].text.contains("ripgrep failed"));
    }

    // --- LSP (Phase 3a wiring) ------------------------------------------------

    /// Diagnostics land in the editor overlay, hover/completions become
    /// shell-paintable view state, and the first definition location is
    /// stashed for the open-then-jump (ARCHITECTURE.md §5).
    #[test]
    fn lsp_events_land_in_view_state_and_the_editor_overlay() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "a.rs", "fn main() {}");
        let pane = state.editor_area.active_pane_id();
        // The shell creates the per-pane editor at paint time; stand one up
        // the same way (shell.rs paint_editor).
        let buffer = state.buffers.get(&p).cloned().expect("buffer");
        state
            .editors
            .insert((pane, p.clone()), selora_editor::Editor::new(buffer));

        state.apply(AppMessage::Lsp(selora_lsp::LspEvent::ServerStatus {
            server: selora_lsp::LanguageId::Rust,
            status: selora_lsp::ServerStatus::Ready,
        }));
        state.apply(AppMessage::Lsp(selora_lsp::LspEvent::Diagnostics {
            path: p.clone(),
            diagnostics: vec![selora_lsp::LspDiagnostic {
                range: selora_lsp::LspRange {
                    start: selora_lsp::LspPosition {
                        line: 0,
                        character: 3,
                    },
                    end: selora_lsp::LspPosition {
                        line: 0,
                        character: 7,
                    },
                },
                severity: selora_lsp::LspSeverity::Error,
                source: Some("rust-analyzer".into()),
                message: "unresolved symbol".into(),
            }],
        }));
        state.apply(AppMessage::Lsp(selora_lsp::LspEvent::Hover {
            path: p.clone(),
            position: selora_core::Position::new(0, 3),
            content: Some("fn main()\nthe entry point".into()),
        }));
        state.apply(AppMessage::Lsp(selora_lsp::LspEvent::Definition {
            path: p.clone(),
            position: selora_core::Position::new(0, 3),
            locations: vec![selora_lsp::LspLocation {
                path: path("lib.rs"),
                range: selora_lsp::LspRange::default(),
            }],
        }));
        state.apply(AppMessage::Lsp(selora_lsp::LspEvent::Completions {
            path: p.clone(),
            position: selora_core::Position::new(0, 4),
            items: vec![selora_lsp::LspCompletionItem {
                label: "main".into(),
                kind: Some(3),
                detail: None,
                sort_text: None,
            }],
        }));

        // Status bar data.
        assert_eq!(
            state.lsp.status.get(&selora_lsp::LanguageId::Rust),
            Some(&selora_lsp::ServerStatus::Ready)
        );
        // The editor overlay carries the diagnostic.
        let editor = state.editors.get(&(pane, p.clone())).expect("editor");
        assert_eq!(editor.diagnostics().len(), 1);
        assert_eq!(editor.diagnostics()[0].message, "unresolved symbol");
        // The hover card is shaped and bound to the requested path.
        let (hover_path, lines) = state.lsp.hover.as_ref().expect("hover");
        assert_eq!(hover_path, &p);
        assert!(lines.join("\n").contains("fn main()"));
        // The definition jump is pending for the shell.
        assert_eq!(
            state
                .pending_definition
                .as_ref()
                .map(|location| location.path.clone()),
            Some(path("lib.rs"))
        );
        // The completion popup is open with the entry (unfiltered).
        let (completion_path, popup) = state.lsp.completion.as_ref().expect("completion");
        assert_eq!(completion_path, &p);
        assert_eq!(popup.filtered().len(), 1);
        // Happy-path events toast nothing.
        assert!(state.toasts.is_empty());
    }

    /// A dead server is an error toast; a missing one is an info toast.
    #[test]
    fn lsp_dead_and_missing_servers_toast() {
        let mut state = AppState::default();
        state.apply(AppMessage::Lsp(selora_lsp::LspEvent::ServerStatus {
            server: selora_lsp::LanguageId::Rust,
            status: selora_lsp::ServerStatus::Dead("exit code 1".into()),
        }));
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].severity, Severity::Error);
        assert!(state.toasts[0].text.contains("rust-analyzer"));
        assert!(state.toasts[0].text.contains("exit code 1"));

        state.apply(AppMessage::Lsp(selora_lsp::LspEvent::ServerMissing {
            server: selora_lsp::LanguageId::TypeScript,
        }));
        assert_eq!(state.toasts.len(), 2);
        assert_eq!(state.toasts[1].severity, Severity::Info);
        assert!(state.toasts[1].text.contains("typescript-language-server"));
    }

    /// A folder switch clears the LSP popups and the pending jump (the
    /// servers themselves are told by the shell's close-before-re-root).
    #[test]
    fn switch_folder_clears_lsp_popups_and_pending_jump() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "a.rs", "fn main() {}");
        state.apply(AppMessage::Lsp(selora_lsp::LspEvent::Hover {
            path: p,
            position: selora_core::Position::new(0, 3),
            content: Some("fn main()".into()),
        }));
        state.pending_definition = Some(selora_lsp::LspLocation {
            path: path("lib.rs"),
            range: selora_lsp::LspRange::default(),
        });
        assert!(state.lsp.hover.is_some());

        state.apply(AppMessage::DialogFolderPicked(Some(path("/new"))));
        assert!(state.lsp.hover.is_none());
        assert!(state.lsp.completion.is_none());
        assert!(state.lsp.progress.is_none());
        assert!(state.pending_definition.is_none());
    }

    // --- ghost text (Phase 4d wiring) ----------------------------------------

    /// A ghost reply lands only on the editor whose caret still matches
    /// the request; stale replies and `None` leave no ghost (the ghost is
    /// paint-only until Tab, ARCHITECTURE.md §9.2).
    #[test]
    fn ghost_reply_lands_on_the_matching_editor_and_stale_drops() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "a.rs", "fn main() {}");
        let pane = state.editor_area.active_pane_id();
        let buffer = state.buffers.get(&p).cloned().expect("buffer");
        state
            .editors
            .insert((pane, p.clone()), selora_editor::Editor::new(buffer));

        // A reply whose caret matches nothing on screen is stale.
        state.apply(AppMessage::GhostTextReady {
            path: p.clone(),
            request_caret: Position::new(9, 9),
            suggestion: Some("stale()".into()),
        });
        assert_eq!(
            state
                .editors
                .get(&(pane, p.clone()))
                .expect("editor")
                .ghost(),
            None
        );

        // The matching reply (the fresh editor's caret is 0:0) becomes
        // the ghost.
        state.apply(AppMessage::GhostTextReady {
            path: p.clone(),
            request_caret: Position::new(0, 0),
            suggestion: Some("foo();".into()),
        });
        assert_eq!(
            state
                .editors
                .get(&(pane, p.clone()))
                .expect("editor")
                .ghost(),
            Some("foo();")
        );

        // A later `None` (the gateway had nothing) clears it.
        state.apply(AppMessage::GhostTextReady {
            path: p.clone(),
            request_caret: Position::new(0, 0),
            suggestion: None,
        });
        assert_eq!(
            state
                .editors
                .get(&(pane, p.clone()))
                .expect("editor")
                .ghost(),
            None
        );
    }

    // --- chat (Phase 4c wiring) ------------------------------------------------

    /// Chat messages stream into the panel keyed by the request map; the
    /// map entry leaves at the terminal message, so a late reply for it
    /// is dropped; the receipt feeds the cost meter.
    #[test]
    fn chat_messages_stream_into_the_panel_and_drop_stale_ids() {
        let mut state = AppState::default();
        state.apply(AppMessage::FolderOpened(path("/repo")));

        // The shell's Send half: the panel opened the stream (submit sets
        // it in flight) and the shell recorded the request id.
        let chat_uuid = Uuid::new_v4();
        let chat_id = selora_ui::sidebar::ChatId::next();
        state.chat_panel.begin_stream(chat_id);
        state.chat_ids.insert(chat_uuid, chat_id);
        state.chat_panel.push_user("write a haiku");

        state.apply(AppMessage::ChatDelta(chat_uuid, "line one\n".into()));
        state.apply(AppMessage::ChatDelta(chat_uuid, "line two".into()));
        // A reply for an unknown uuid (the entry already left, or a race):
        // dropped, not appended.
        state.apply(AppMessage::ChatDelta(Uuid::new_v4(), "garbage".into()));
        assert!(state.chat_panel.is_streaming());

        state.apply(AppMessage::ChatDone(
            chat_uuid,
            ChatReceipt {
                request_id: Uuid::new_v4(),
                model: "test-model".into(),
                prompt_tokens: 9,
                completion_tokens: 2,
                cost_usd: 0.001,
                duration_ms: 42,
                estimated: false,
            },
        ));
        // The terminal message removed the map entry — a late delta for
        // the same uuid is now stale and dropped.
        state.apply(AppMessage::ChatDelta(chat_uuid, "late".into()));

        assert!(state.chat_ids.is_empty());
        assert!(!state.chat_panel.is_streaming());
        let transcript = state.chat_panel.transcript();
        assert!(transcript.contains("write a haiku"), "{transcript}");
        assert!(transcript.contains("line one\nline two"), "{transcript}");
        assert!(!transcript.contains("garbage"));
        assert!(!transcript.contains("late"));
        // The gateway-reported cost landed in the meter verbatim.
        assert!((state.cost_meter.total_usd() - 0.001).abs() < f64::EPSILON);
    }

    // --- workspace snapshot ---------------------------------------------------

    #[test]
    fn workspace_snapshot_captures_root_tabs_and_active_path() {
        let mut state = AppState::default();
        state.apply(AppMessage::FolderOpened(path("/repo")));
        let a = loaded(&mut state, "a.rs", "a");
        let b = loaded(&mut state, "b.rs", "b");

        let snapshot = state.workspace_snapshot(true);
        assert_eq!(snapshot.root, Some(path("/repo")));
        assert_eq!(snapshot.open_files, vec![a, b.clone()]);
        assert_eq!(snapshot.active_path, Some(b));
        assert!(snapshot.chat_open);
        assert!(snapshot.chat_turns.is_empty());
        assert!(!snapshot.agent_mode, "the pill defaults off (Phase 5d)");

        state.chat_panel.set_agent_mode(true);
        assert!(state.workspace_snapshot(true).agent_mode);
    }

    // --- Agent Mode (Phase 5d, ARCHITECTURE.md §17.6/§17.7) -------------------

    /// Open an agent turn: the panel in flight + the uuid→ChatId mapping,
    /// exactly what the shell's AgentSend route does.
    fn agent_turn_open(state: &mut AppState) -> (Uuid, selora_ui::sidebar::ChatId) {
        let uuid = Uuid::new_v4();
        let chat_id = selora_ui::sidebar::ChatId::next();
        state.chat_panel.begin_stream(chat_id);
        state.chat_ids.insert(uuid, chat_id);
        (uuid, chat_id)
    }

    fn tool_call(id: &str, name: &str, arguments: &str) -> selora_ai::ToolCall {
        selora_ai::ToolCall {
            id: id.to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    fn tool_result(id: &str, ok: bool, content: &str) -> selora_ai::ToolResult {
        selora_ai::ToolResult {
            id: id.to_owned(),
            ok,
            content: content.to_owned(),
        }
    }

    fn agent_receipt(cost: f64) -> ChatReceipt {
        ChatReceipt {
            request_id: Uuid::new_v4(),
            model: "test-model".into(),
            prompt_tokens: 0,
            completion_tokens: 100,
            cost_usd: cost,
            duration_ms: 5,
            estimated: true,
        }
    }

    fn agent_summary() -> selora_ai::AgentSummary {
        selora_ai::AgentSummary {
            iterations: 2,
            tools_used: 1,
            approved: 1,
            denied: 0,
            tokens: 100,
            cost_usd: 0.25,
            completed: true,
        }
    }

    fn buffer_text(state: &AppState, p: &Path) -> String {
        state
            .buffers
            .get(p)
            .expect("buffer")
            .read()
            .expect("lock")
            .text()
    }

    #[test]
    fn agent_messages_reduce_into_the_panel_and_drop_stale_ids() {
        let mut state = AppState::default();
        state.apply(AppMessage::FolderOpened(path("/repo")));
        let (uuid, _chat_id) = agent_turn_open(&mut state);

        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call("call_1", "read_file", r#"{"path":"a.rs"}"#),
        ));
        state.apply(AppMessage::ApprovalNeeded(
            uuid,
            selora_ai::ApprovalId::from_u128(1),
            selora_ai::ApprovalCard::Diff {
                path: "a.rs".into(),
                diff: "diff".into(),
            },
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_1", true, "contents"),
        ));

        // Unknown uuids are stale: dropped, never a panic.
        let stale = Uuid::new_v4();
        state.apply(AppMessage::ToolCallStarted(
            stale,
            tool_call("call_9", "read_file", "{}"),
        ));
        state.apply(AppMessage::ToolResult(
            stale,
            tool_result("call_9", true, "x"),
        ));
        state.apply(AppMessage::ApprovalNeeded(
            stale,
            selora_ai::ApprovalId::nil(),
            selora_ai::ApprovalCard::Delete { path: "x".into() },
        ));
        // A stale turn end drops WHOLE: no panel reduction, no meter
        // movement, no toast — even carrying a cost and a Failed end.
        state.apply(AppMessage::AgentTurnDone(
            stale,
            agent_receipt(9.99),
            agent_summary(),
            selora_ai::TurnEnd::Failed("stale turn".into()),
        ));
        // The stale AgentTurnDone did not end the live turn.
        assert!(state.chat_panel.is_streaming());
        // user-free timeline: one card so far.
        assert_eq!(state.chat_panel.turn_count(), 1);
        assert_eq!(
            state.cost_meter.total_usd(),
            0.0,
            "a stale turn end never moves the meter"
        );
        assert!(
            state.toasts.is_empty(),
            "a stale turn end never toasts: {:?}",
            state.toasts
        );

        state.apply(AppMessage::AgentTurnDone(
            uuid,
            agent_receipt(0.25),
            agent_summary(),
            selora_ai::TurnEnd::Completed,
        ));
        assert!(!state.chat_panel.is_streaming());
        assert!(state.chat_ids.is_empty());
        // The receipt fed the cost meter exactly like ChatDone.
        assert!((state.cost_meter.total_usd() - 0.25).abs() < f64::EPSILON);
        // The summary footer landed.
        assert_eq!(state.chat_panel.turn_count(), 2);
        // A Completed end toasts nothing.
        assert!(state.toasts.is_empty());
    }

    #[test]
    fn agent_turn_done_toasts_abnormal_ends() {
        let mut state = AppState::default();
        let (uuid, _) = agent_turn_open(&mut state);
        state.apply(AppMessage::AgentTurnDone(
            uuid,
            agent_receipt(0.25),
            agent_summary(),
            selora_ai::TurnEnd::IterationCap { limit: 25 },
        ));
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].severity, Severity::Info);
        assert!(state.toasts[0].text.contains("iteration cap (25)"));

        let (uuid, _) = agent_turn_open(&mut state);
        state.apply(AppMessage::AgentTurnDone(
            uuid,
            agent_receipt(0.0),
            agent_summary(),
            selora_ai::TurnEnd::Failed("gateway unreachable".into()),
        ));
        assert_eq!(state.toasts.len(), 2);
        assert_eq!(state.toasts[1].severity, Severity::Error);
        assert!(state.toasts[1].text.contains("gateway unreachable"));

        let (uuid, _) = agent_turn_open(&mut state);
        state.apply(AppMessage::AgentTurnDone(
            uuid,
            agent_receipt(0.0),
            agent_summary(),
            selora_ai::TurnEnd::StoppedByUser,
        ));
        assert_eq!(state.toasts.len(), 3);
        assert_eq!(state.toasts[2].severity, Severity::Info);
    }

    #[test]
    fn an_approved_write_to_a_clean_open_buffer_syncs_it_and_ends_clean() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "/repo/a.rs", "fn main() {}\n");
        let (uuid, _) = agent_turn_open(&mut state);

        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call(
                "call_1",
                "write_file",
                r#"{"path":"a.rs","content":"fn main() { /* hi */ }\n"}"#,
            ),
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_1", true, "wrote"),
        ));

        // The buffer shows the approved text and ends CLEAN (the card was
        // the save); the baselines rebase.
        assert_eq!(buffer_text(&state, &p), "fn main() { /* hi */ }\n");
        assert!(!state.is_dirty(&p));
        assert_eq!(
            state.saved_shape(&p),
            Some(BufferShape::of(&Buffer::from_text(
                "fn main() { /* hi */ }\n"
            )))
        );
        assert!(state.toasts.is_empty());
        // The LSP drain saw the path, once.
        assert_eq!(state.take_agent_touched(), vec![p.clone()]);
        assert!(state.take_agent_touched().is_empty());

        // Ctrl+Z: the buffer reverts to the pre-edit text, dirty against
        // disk (undo never touches disk).
        assert!(state.undo_agent_edit(&p));
        assert_eq!(buffer_text(&state, &p), "fn main() {}\n");
        assert!(state.is_dirty(&p));
        // The stack is drained: a second undo is a no-op.
        assert!(!state.undo_agent_edit(&p));
    }

    #[test]
    fn an_approved_edit_applies_the_exact_transform() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "/repo/a.rs", "one\ntwo\none\n");
        let (uuid, _) = agent_turn_open(&mut state);

        // replacen(old, new, 1): the first occurrence only.
        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call(
                "call_1",
                "edit_file",
                r#"{"path":"a.rs","old_str":"one","new_str":"1"}"#,
            ),
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_1", true, "edited"),
        ));
        assert_eq!(buffer_text(&state, &p), "1\ntwo\none\n");

        // replace_all: every remaining occurrence.
        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call(
                "call_2",
                "edit_file",
                r#"{"path":"a.rs","old_str":"one","new_str":"ONE","replace_all":true}"#,
            ),
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_2", true, "edited"),
        ));
        assert_eq!(buffer_text(&state, &p), "1\ntwo\nONE\n");

        // Two snapshots: undo pops the newest first.
        assert!(state.undo_agent_edit(&p));
        assert_eq!(buffer_text(&state, &p), "1\ntwo\none\n");
        assert!(state.undo_agent_edit(&p));
        assert_eq!(buffer_text(&state, &p), "one\ntwo\none\n");
    }

    #[test]
    fn a_dirty_buffer_is_never_touched_by_an_agent_write() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "/repo/a.rs", "fn main() {}\n");
        // The user edits: dirty.
        {
            let buffer = state.buffers.get(&p).cloned().expect("buffer");
            buffer
                .write()
                .expect("lock")
                .insert(Position::new(1, 0), "// mine\n");
        }
        let dirty_shape = BufferShape::of(&Buffer::from_text("fn main() {}\n// mine\n"));
        state.note_editor_frame(&p, dirty_shape, true);
        assert!(state.is_dirty(&p));

        let (uuid, _) = agent_turn_open(&mut state);
        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call(
                "call_1",
                "write_file",
                r#"{"path":"a.rs","content":"fn main() { /* agent */ }\n"}"#,
            ),
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_1", true, "wrote"),
        ));

        // The buffer keeps the user's text; the toast names the rel path.
        assert_eq!(buffer_text(&state, &p), "fn main() {}\n// mine\n");
        assert!(state.is_dirty(&p));
        assert_eq!(state.toasts.len(), 1);
        assert!(
            state.toasts[0].text.contains("a.rs"),
            "{:?}",
            state.toasts[0]
        );
        assert!(state.toasts[0].text.contains("unsaved changes"));
        // Nothing synced: no undo snapshot, no LSP touch.
        assert!(!state.undo_agent_edit(&p));
        assert!(state.take_agent_touched().is_empty());

        // edit_file is equally guarded.
        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call(
                "call_2",
                "edit_file",
                r#"{"path":"a.rs","old_str":"fn","new_str":"fn!"}"#,
            ),
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_2", true, "edited"),
        ));
        assert_eq!(buffer_text(&state, &p), "fn main() {}\n// mine\n");
        assert_eq!(state.toasts.len(), 2);
    }

    #[test]
    fn unmatched_or_failed_results_never_touch_buffers() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "/repo/a.rs", "fn main() {}\n");
        let (uuid, _) = agent_turn_open(&mut state);

        // An ok result for an unstashed tool_call_id (a read): nothing.
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_404", true, "x"),
        ));
        // A denied write (ok: false): nothing.
        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call(
                "call_1",
                "write_file",
                r#"{"path":"a.rs","content":"clobber\n"}"#,
            ),
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_1", false, "user denied this action"),
        ));

        assert_eq!(buffer_text(&state, &p), "fn main() {}\n");
        assert!(!state.is_dirty(&p));
        assert!(state.toasts.is_empty());
        assert!(state.take_agent_touched().is_empty());
    }

    #[test]
    fn undo_of_a_same_length_agent_edit_marks_dirty_explicitly() {
        let mut state = AppState::default();
        // A same-length agent edit pair: shape(post) == shape(pre) — the
        // case the module docs warn the shape-compare cannot see.
        let p = loaded(&mut state, "/repo/a.rs", "aaa\n");
        let (uuid, _) = agent_turn_open(&mut state);
        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call(
                "call_1",
                "edit_file",
                r#"{"path":"a.rs","old_str":"aaa","new_str":"bbb"}"#,
            ),
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_1", true, "edited"),
        ));
        assert_eq!(buffer_text(&state, &p), "bbb\n");
        assert!(!state.is_dirty(&p));

        // Undo of the same-length edit marks dirty EXPLICITLY (shape is
        // blind to it) and re-touches the LSP path.
        let _ = state.take_agent_touched();
        assert!(state.undo_agent_edit(&p));
        assert_eq!(buffer_text(&state, &p), "aaa\n");
        assert!(state.is_dirty(&p));
        assert_eq!(state.take_agent_touched(), vec![p.clone()]);
    }

    #[test]
    fn undo_agent_edit_invalidates_after_a_human_edit_and_never_clobbers() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "/repo/a.rs", "aaa\n");
        let (uuid, _) = agent_turn_open(&mut state);
        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call(
                "call_1",
                "edit_file",
                r#"{"path":"a.rs","old_str":"aaa","new_str":"ccc"}"#,
            ),
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_1", true, "edited"),
        ));
        assert_eq!(buffer_text(&state, &p), "ccc\n");

        // A HUMAN edit on top: the snapshot's post no longer matches —
        // the stack invalidates, never clobbers.
        {
            let buffer = state.buffers.get(&p).cloned().expect("buffer");
            buffer
                .write()
                .expect("lock")
                .insert(Position::new(0, 3), "!");
        }
        assert!(!state.undo_agent_edit(&p), "the buffer moved on");
        assert_eq!(buffer_text(&state, &p), "ccc!\n", "never a clobber");
        assert!(!state.undo_agent_edit(&p), "the stack is drained");
    }

    #[test]
    fn switch_folder_clears_the_agent_stashes() {
        let mut state = AppState::default();
        let p = loaded(&mut state, "/repo/a.rs", "x\n");
        let (uuid, _) = agent_turn_open(&mut state);
        state.apply(AppMessage::ToolCallStarted(
            uuid,
            tool_call("call_1", "write_file", r#"{"path":"a.rs","content":"y\n"}"#),
        ));
        state.apply(AppMessage::ToolResult(
            uuid,
            tool_result("call_1", true, "wrote"),
        ));
        assert_eq!(buffer_text(&state, &p), "y\n");
        assert!(!state.agent_edits.is_empty());

        state.apply(AppMessage::DialogFolderPicked(Some(path("/new"))));
        assert!(state.agent_edits.is_empty());
        assert!(state.agent_write_calls.is_empty());
        assert!(state.agent_touched.is_empty());
    }
}
