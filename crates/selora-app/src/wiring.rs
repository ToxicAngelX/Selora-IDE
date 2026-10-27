//! Runtime wiring — the tokio side of the app, UI-free (ARCHITECTURE.md §1, §2).
//!
//! Everything here is deliberately free of egui so the message-drain logic
//! can be tested headlessly (and end-to-end against a wiremock mock gateway)
//! without opening a window. The contracts implemented:
//!
//! * ARCHITECTURE.md §1.4 — tasks are spawned onto the multi-threaded
//!   runtime built *before* the eframe loop; they never touch UI state.
//! * §2 — the Message-drain pattern: workers send [`AppMessage`]s on an
//!   unbounded channel; the main thread drains it at the top of each frame
//!   and reduces into [`AppState`][crate::app_state::AppState].
//! * §4 — the file effects: `OpenFile` reads off the frame path (with the
//!   4 MB Lite cap), `SaveBuffer` writes atomically (temp + rename),
//!   `OpenFolder`/`RefreshFileTree` build the tree via `selora_fs::tree`.
//! * §6 — the SeloraClient calls the Phase 1c spike needs
//!   (`list_models`, `stream_chat`). The chat UI is Phase 4; this half
//!   stays live and tested until then.
//!
//! Every `spawn_*` function takes a `notify` callback that is invoked after
//! each message send; the UI passes `ctx.request_repaint()` there — without
//! it egui sleeps between frames and worker results would never appear.
//!
//! [`spawn_dispatcher`] is the single consumer task that owns the
//! `mpsc::Receiver<Effect>` (ARCHITECTURE.md §2.2) and routes each effect to
//! one of the `spawn_*` workers. It also owns the in-memory Selora key (the
//! OS keyring lands in Phase 4) and the current gateway base URL (updated
//! whenever settings are loaded/saved).

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use selora_ai::{ChatEvent, ChatReceipt, SeloraClient};
use selora_core::{Buffer, SeloraError};
use selora_fs::tree::build_tree;
use selora_git::{open_repo, GitError};
use selora_settings::Settings;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;
use uuid::Uuid;

use crate::app_state::{AgentTurnRequest, AppMessage, Effect};

/// Default gateway base URL (ARCHITECTURE.md §6.1). Overridable in settings;
/// tests point it at a wiremock server.
pub const DEFAULT_BASE_URL: &str = "https://api.selora.lol";

/// The repaint seam: called after every worker → main-thread message send.
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// The Lite Edition file-size cap (ARCHITECTURE.md §4.1): files larger than
/// this are rejected with a toast instead of being loaded.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Effect workers: file system
// ---------------------------------------------------------------------------

/// Read a file's text for a buffer, rejecting files over [`MAX_FILE_BYTES`]
/// (ARCHITECTURE.md §4.1). Runs inside `spawn_blocking`.
pub fn read_file_text(path: &Path) -> Result<String, SeloraError> {
    let len = std::fs::metadata(path).map_err(SeloraError::Io)?.len();
    if len > MAX_FILE_BYTES {
        return Err(SeloraError::Other(format!(
            "file is {} MB — over the {} MB Lite limit and will not open",
            len / (1024 * 1024),
            MAX_FILE_BYTES / (1024 * 1024)
        )));
    }
    std::fs::read_to_string(path).map_err(SeloraError::Io)
}

/// Atomic text write (ARCHITECTURE.md §4.2): `path.tmp` next to the target,
/// then rename over it — same directory, so the rename never crosses a
/// volume. No `.tmp` file is left behind on failure.
pub fn atomic_write_text(path: &Path, text: &str) -> Result<(), SeloraError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    std::fs::write(&tmp, text)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&tmp);
            Err(SeloraError::Io(error))
        }
    }
}

/// Effect (§2.2): read a file off the frame path. Replies
/// [`AppMessage::BufferLoaded`] (the shell creates the shared buffer and
/// opens the tab) or [`AppMessage::BufferLoadFailed`].
pub fn spawn_open_file(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    path: PathBuf,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let message = match read_file_text(&path) {
            Ok(text) => AppMessage::BufferLoaded { path, text },
            Err(error) => AppMessage::BufferLoadFailed { path, error },
        };
        let _ = tx.send(message);
        notify();
    });
}

/// Effect (§2.2): build the file tree for a folder off the frame path and
/// reply [`AppMessage::FileTreeUpdated`] — preceded by
/// [`AppMessage::FolderOpened`] so the workspace root and the status bar
/// update before the (possibly slow) walk finishes.
pub fn spawn_open_folder(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    root: PathBuf,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let _ = tx.send(AppMessage::FolderOpened(root.clone()));
        notify();
        let tree = build_tree(&root);
        let _ = tx.send(AppMessage::FileTreeUpdated { root, tree });
        notify();
    });
}

/// Effect (§2.2): rebuild the tree for an already-open workspace (the tree
/// is stale after a New File / Rename / Delete in the sidebar,
/// ARCHITECTURE.md §4.5: Lite does not watch the filesystem).
pub fn spawn_refresh_tree(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    root: PathBuf,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let tree = build_tree(&root);
        let _ = tx.send(AppMessage::FileTreeUpdated { root, tree });
        notify();
    });
}

/// Effect (§2.2): atomically write a shared buffer's current text to its
/// path (ARCHITECTURE.md §4.2). The text is snapshotted under a read lock
/// *inside* the blocking task, so the UI thread never waits on IO.
pub fn spawn_save_buffer(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    path: PathBuf,
    buffer: Arc<RwLock<Buffer>>,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let text = buffer
            .read()
            .map_err(|_| SeloraError::Other("buffer lock poisoned".into()))
            .map(|buffer| buffer.text());
        let message = match text {
            Ok(text) => match atomic_write_text(&path, &text) {
                Ok(()) => AppMessage::BufferSaved(path),
                Err(error) => AppMessage::BufferSaveFailed { path, error },
            },
            Err(error) => AppMessage::BufferSaveFailed { path, error },
        };
        let _ = tx.send(message);
        notify();
    });
}

// ---------------------------------------------------------------------------
// Effect workers: settings
// ---------------------------------------------------------------------------

/// Effect (§2.2): load settings off the frame path. `load_or_default` keeps
/// the toast-able error (schema-version wipes, bad JSON) alongside the
/// usable defaults, so both messages are sent (ARCHITECTURE.md §8.1).
pub fn spawn_settings_load(handle: &Handle, notify: &Notify, tx: &UnboundedSender<AppMessage>) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let (settings, error) = Settings::load_or_default();
        let _ = tx.send(AppMessage::SettingsLoaded(settings));
        notify();
        if let Some(error) = error {
            let _ = tx.send(AppMessage::SettingsError(error));
            notify();
        }
    });
}

/// Effect (§2.2): persist settings off the frame path.
pub fn spawn_settings_save(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    settings: Settings,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let message = match settings.save() {
            Ok(()) => AppMessage::SettingsSaved,
            Err(error) => AppMessage::SettingsError(error),
        };
        let _ = tx.send(message);
        notify();
    });
}

// ---------------------------------------------------------------------------
// Effect workers: git (Phase 3b)
// ---------------------------------------------------------------------------

/// `GitError` → `SeloraError` at the wiring seam (selora-git deliberately
/// depends on no workspace crate, PLAN.md §5).
fn git_error(error: GitError) -> SeloraError {
    SeloraError::Git(error.to_string())
}

/// Effect (§2.2): snapshot the status of the repo containing `root`.
/// Not-a-repo is [`AppMessage::GitNoRepo`], not an error (the panel shows
/// its "no repository" state).
pub fn spawn_git_status(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    root: PathBuf,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let message = match open_repo(&root) {
            Ok(repo) => match repo.status() {
                Ok(status) => AppMessage::GitStatusChanged(status),
                Err(error) => AppMessage::GitError(git_error(error)),
            },
            // Discover failing means "no repository here" — the common,
            // non-error case for any non-git folder.
            Err(_) => AppMessage::GitNoRepo,
        };
        let _ = tx.send(message);
        notify();
    });
}

/// Effect (§2.2): stage one file, then re-snapshot so the panel and the
/// tree badges refresh in one round trip.
/// Effect (Phase R6b): the working-tree diff of one file, as unified
/// text for the git panel's read-only diff view.
pub fn spawn_git_diff(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    root: PathBuf,
    path: PathBuf,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let message = match open_repo(&root) {
            Ok(repo) => match repo.diff_workdir(&path) {
                Ok(diff) => AppMessage::GitDiffReady {
                    path,
                    diff: diff.to_unified(),
                },
                Err(error) => AppMessage::GitError(git_error(error)),
            },
            Err(_) => AppMessage::GitNoRepo,
        };
        let _ = tx.send(message);
        notify();
    });
}

pub fn spawn_git_stage(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    path: PathBuf,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let outcome = open_repo(&path)
            .map_err(git_error)
            .and_then(|repo| repo.stage(&path).map_err(git_error))
            .map(|()| None);
        send_git_outcome(&tx, &notify, outcome, &path);
    });
}

/// Effect (§2.2): unstage one file, then re-snapshot.
pub fn spawn_git_unstage(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    path: PathBuf,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let outcome = open_repo(&path)
            .map_err(git_error)
            .and_then(|repo| repo.unstage(&path).map_err(git_error))
            .map(|()| None);
        send_git_outcome(&tx, &notify, outcome, &path);
    });
}

/// Effect (§2.2): commit the index of the repo containing `root`, toast
/// the short oid, then re-snapshot.
pub fn spawn_git_commit(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    root: PathBuf,
    message: String,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let outcome = open_repo(&root).map_err(git_error).and_then(|repo| {
            repo.commit(&message)
                .map(|oid| Some(oid.to_string()[..7].to_owned()))
                .map_err(git_error)
        });
        send_git_outcome(&tx, &notify, outcome, &root);
    });
}

/// The shared tail of every mutating git worker: surface the outcome
/// (short-oid toast on commit, silent success on stage/unstage, an error
/// toast otherwise), then always send a fresh status snapshot so the panel
/// and the tree badges never show stale rows.
fn send_git_outcome(
    tx: &UnboundedSender<AppMessage>,
    notify: &Notify,
    outcome: Result<Option<String>, SeloraError>,
    repo_path: &Path,
) {
    match outcome {
        Ok(Some(short_id)) => {
            let _ = tx.send(AppMessage::GitCommitted(short_id));
        }
        Ok(None) => {}
        Err(error) => {
            let _ = tx.send(AppMessage::GitError(error));
        }
    }
    // The refreshed snapshot (or the not-a-repo marker).
    let message = match open_repo(repo_path) {
        Ok(repo) => match repo.status() {
            Ok(status) => AppMessage::GitStatusChanged(status),
            Err(error) => AppMessage::GitError(git_error(error)),
        },
        Err(_) => AppMessage::GitNoRepo,
    };
    let _ = tx.send(message);
    notify();
}

// ---------------------------------------------------------------------------
// Effect workers: search (Phase 3c)
// ---------------------------------------------------------------------------

/// Effect (§2.2): run a workspace search off the frame path and reply
/// [`AppMessage::SearchResults`] with the same id (the panel drops stale
/// replies).
pub fn spawn_search(
    handle: &Handle,
    notify: &Notify,
    tx: &UnboundedSender<AppMessage>,
    id: selora_fs::search::SearchId,
    root: PathBuf,
    query: selora_fs::search::SearchQuery,
) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let results = selora_fs::search::search_in_files(&root, &query);
        let _ = tx.send(AppMessage::SearchResults { id, results });
        notify();
    });
}

// ---------------------------------------------------------------------------
// Effect workers: LSP (Phase 3a)
// ---------------------------------------------------------------------------

/// Spawn the LSP manager task plus the `LspEvent` → [`AppMessage::Lsp`]
/// relay: the app drains one message channel (ARCHITECTURE.md §2.1), so the
/// manager's events are forwarded onto it with a repaint after each send.
/// The task pair ends when the shell drops the effect sender or the app
/// drops the message receiver.
#[allow(clippy::too_many_arguments)]
pub fn spawn_lsp_manager(
    handle: &Handle,
    notify: Notify,
    effect_rx: UnboundedReceiver<selora_lsp::LspEffect>,
    msg_tx: UnboundedSender<AppMessage>,
    config: selora_lsp::LspManagerConfig,
) {
    let (event_tx, event_rx) = mpsc::unbounded_channel::<selora_lsp::LspEvent>();
    let relay_notify = notify.clone();
    handle.spawn(async move {
        let mut event_rx = event_rx;
        while let Some(event) = event_rx.recv().await {
            // Receiver dropped (window closed): stop relaying.
            if msg_tx.send(AppMessage::Lsp(event)).is_err() {
                return;
            }
            relay_notify();
        }
    });
    selora_lsp::spawn_manager(handle, notify, effect_rx, event_tx, config);
}

/// Effect worker: one gateway connection probe (Phase 4f). The probe is
/// `list_models` — not a bare HEAD — so `Connected` means the gateway
/// answered *and* the key is valid; the round-trip is the latency. The
/// catalog is not re-sent (the 30s probe must not toast when the gateway
/// is down — status only).
fn spawn_selora_ping(
    handle: &Handle,
    notify: &Notify,
    msg_tx: UnboundedSender<AppMessage>,
    base_url: String,
    api_key: String,
) {
    let notify = notify.clone();
    handle.spawn(async move {
        let client = SeloraClient::new(&base_url, &api_key);
        let started = std::time::Instant::now();
        let status = match client.list_models().await {
            Ok(_) => selora_ai::SeloraStatus::Connected {
                model: None,
                latency_ms: started.elapsed().as_millis() as u32,
            },
            Err(error) => selora_ai::SeloraStatus::Error(error.to_string()),
        };
        let _ = msg_tx.send(AppMessage::SeloraStatus(status));
        notify();
    });
}

// ---------------------------------------------------------------------------
// Effect workers: blocking file/folder pickers (rfd)
// ---------------------------------------------------------------------------

/// Effect-side helper: run a blocking `rfd` folder picker on a worker thread
/// (never the UI thread — the dialog pumps a native message loop) and reply
/// with [`AppMessage::DialogFolderPicked`]. `None` = cancelled (not an
/// error; nothing is surfaced).
pub fn spawn_pick_folder(handle: &Handle, notify: &Notify, tx: &UnboundedSender<AppMessage>) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let picked = rfd::FileDialog::new().pick_folder();
        let _ = tx.send(AppMessage::DialogFolderPicked(picked));
        notify();
    });
}

/// Effect-side helper: the file-picker twin of [`spawn_pick_folder`].
pub fn spawn_pick_file(handle: &Handle, notify: &Notify, tx: &UnboundedSender<AppMessage>) {
    let tx = tx.clone();
    let notify = notify.clone();
    handle.spawn_blocking(move || {
        let picked = rfd::FileDialog::new().pick_file();
        let _ = tx.send(AppMessage::DialogFilePicked(picked));
        notify();
    });
}

// ---------------------------------------------------------------------------
// Effect workers: Selora gateway (Phase 1c network half, live until Phase 4)
// ---------------------------------------------------------------------------

/// Effect (§2.2): connect to the gateway and list its models.
/// Effect (§2.2): validate a key against the gateway and, on success,
/// make it the session's key.
///
/// Validation is `list_models` (a key that lists models is a good key,
/// Phase 4a); the round-trip doubles as the latency measurement. On
/// success the reply is `ModelsLoaded(Ok)` + `SeloraStatus(Connected)`;
/// on failure `ModelsLoaded(Err)` (the toast path) +
/// `SeloraStatus(Error(gateway words))`. When `persist_key` is set, a
/// validated key is also written to the OS keyring (validate first, store
/// second — a bad key never lands in the credential store); tests pass
/// `false` so they never touch the real Windows Credential Manager.
#[allow(clippy::too_many_arguments)]
pub fn spawn_connect(
    handle: &Handle,
    notify: &Notify,
    tx: UnboundedSender<AppMessage>,
    base_url: String,
    api_key: String,
    persist_key: bool,
) {
    let notify = notify.clone();
    handle.spawn(async move {
        let client = SeloraClient::new(&base_url, &api_key);
        let started = std::time::Instant::now();
        let result = client.list_models().await;
        let latency_ms = started.elapsed().as_millis() as u32;
        // A single Credential Manager call, ms-scale — fine inline on the
        // async task (PLAN.md §5; never a file).
        if persist_key && result.is_ok() {
            if let Err(error) = selora_settings::keyring::api_key_set(&api_key) {
                let _ = tx.send(AppMessage::KeyringError(error));
            }
        }
        let status = match &result {
            Ok(_) => selora_ai::SeloraStatus::Connected {
                model: None,
                latency_ms,
            },
            Err(error) => selora_ai::SeloraStatus::Error(error.to_string()),
        };
        let _ = tx.send(AppMessage::ModelsLoaded(result));
        let _ = tx.send(AppMessage::SeloraStatus(status));
        notify();
    });
}

/// Effect (Phase 4a): delete the stored key (Disconnect). The keyring
/// delete is idempotent; the reply resets the status to `Disconnected`.
fn spawn_disconnect(handle: &Handle, notify: &Notify, tx: UnboundedSender<AppMessage>) {
    let notify = notify.clone();
    handle.spawn(async move {
        if let Err(error) = selora_settings::keyring::api_key_delete() {
            let _ = tx.send(AppMessage::KeyringError(error));
        }
        let _ = tx.send(AppMessage::SeloraStatus(
            selora_ai::SeloraStatus::Disconnected,
        ));
        notify();
    });
}

/// Effect (Phase 6b): the "Check for updates" GET (UI_SPEC.md §11). The
/// one non-gateway HTTP call in the app; the comparison against the running
/// build's version happens in selora-ai (`check_latest_release`).
fn spawn_update_check(handle: &Handle, notify: &Notify, tx: UnboundedSender<AppMessage>) {
    let notify = notify.clone();
    handle.spawn(async move {
        let result = selora_ai::check_latest_release(env!("CARGO_PKG_VERSION"))
            .await
            .map_err(|error| error.to_string());
        let _ = tx.send(AppMessage::UpdateChecked(result));
        notify();
    });
}

/// Effect (Phase 4d): one ghost-text completion. The reply carries the
/// request's caret back so the editor can stale-drop it; no key → a
/// `None` suggestion (the ghost simply does not appear).
#[allow(clippy::too_many_arguments)]
fn spawn_ghost(
    handle: &Handle,
    notify: &Notify,
    tx: UnboundedSender<AppMessage>,
    base_url: String,
    api_key: String,
    path: std::path::PathBuf,
    prefix: String,
    model: String,
    request_caret: selora_core::Position,
) {
    let notify = notify.clone();
    handle.spawn(async move {
        let client = SeloraClient::new(&base_url, &api_key);
        let suggestion = client.ghost_complete(&model, &prefix).await.unwrap_or(None);
        let _ = tx.send(AppMessage::GhostTextReady {
            path,
            request_caret,
            suggestion,
        });
        notify();
    });
}

/// Effect (§2.2): run a chat completion request and stream the reply.
///
/// `selora-ai`'s `stream_chat` emits [`ChatEvent`]s on a channel *and*
/// returns a result, so this spawns two futures: the stream itself, and a
/// relay loop that reduces the events to [`AppMessage`]s (waking the UI per
/// message). The relay stops at the first terminal event (`Done`/`Error`);
/// the stream's return value is then joined only to surface an error that
/// never reached the channel — no silent failures (PLAN.md §8).
#[allow(clippy::too_many_arguments)]
pub fn spawn_chat(
    handle: &Handle,
    notify: &Notify,
    tx: UnboundedSender<AppMessage>,
    base_url: String,
    api_key: String,
    chat_id: Uuid,
    request: selora_ai::ChatRequest,
    mut cancel: watch::Receiver<bool>,
) {
    let notify = notify.clone();
    let stream_handle = handle.clone();
    handle.spawn(async move {
        let client = SeloraClient::new(&base_url, &api_key);
        let (events_tx, mut events_rx) = mpsc::unbounded_channel::<ChatEvent>();

        // Phase R1: the cancel watch — a user Stop flips it, the request
        // future is dropped (the SSE connection closes), and the task
        // resolves `None` so the relay stays silent about a cancel.
        let stream = stream_handle.spawn(async move {
            tokio::select! {
                _ = cancel.changed() => None,
                result = client.stream_chat(request, events_tx) => Some(result),
            }
        });

        let mut terminal_seen = false;
        while let Some(event) = events_rx.recv().await {
            terminal_seen |= matches!(event, ChatEvent::Done(_) | ChatEvent::Error(_));
            let message = match event {
                ChatEvent::Delta(text) => AppMessage::ChatDelta(chat_id, text),
                // Plain chat never arms tools (`ChatRequest::tools` is
                // `None`), so a tool-call delta cannot arrive here — the
                // agent loop (Phase 5c) owns its own relay.
                ChatEvent::ToolCallDelta { .. } => continue,
                ChatEvent::Done(receipt) => AppMessage::ChatDone(chat_id, receipt),
                ChatEvent::Error(error) => AppMessage::ChatError(chat_id, error),
            };
            // Receiver dropped (window closed): stop relaying. The stream task
            // notices on its next event send and exits instead of burning
            // gateway tokens into a dead channel.
            if tx.send(message).is_err() {
                return;
            }
            notify();
            if terminal_seen {
                break;
            }
        }

        // Always join the stream task. When it resolved normally its terminal
        // event was already relayed; a cancel resolved `None` (silent by
        // design); anything else is an error to surface.
        match stream.await {
            Ok(None) => {} // cancelled (Phase R1)
            Ok(Some(Ok(_receipt))) => {}
            Ok(Some(Err(error))) => {
                if !terminal_seen {
                    let _ = tx.send(AppMessage::ChatError(chat_id, error));
                    notify();
                }
            }
            Err(join_error) => {
                if !terminal_seen {
                    let error = SeloraError::Other(format!("chat worker died: {join_error}"));
                    let _ = tx.send(AppMessage::ChatError(chat_id, error));
                    notify();
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Agent Mode (ARCHITECTURE.md §17 — Phase 5d's wiring half)
// ---------------------------------------------------------------------------

/// The receipt an agent turn reports at `TurnDone` (the brief's seam
/// decision — `AgentTurnReport` carries no receipt): the loop's own token
/// and cost totals (it already applied its per-round `compute_cost_usd`
/// fallback, so `cost_usd` is the honest total), the relay's wall clock,
/// `estimated: true` (an aggregate, not a gateway-verbatim receipt). The
/// prompt/completion split is unrecoverable from the loop's summed
/// `tokens` (selora-ai is frozen) and cosmetic for the meter, which sums
/// cost, not tokens — `completion_tokens` carries the clamped total.
fn synthesize_receipt(
    turn_id: Uuid,
    model: &str,
    summary: &selora_ai::AgentSummary,
    elapsed: std::time::Duration,
) -> ChatReceipt {
    ChatReceipt {
        request_id: turn_id,
        model: model.to_owned(),
        prompt_tokens: 0,
        completion_tokens: summary.tokens.min(u64::from(u32::MAX)) as u32,
        cost_usd: summary.cost_usd,
        duration_ms: elapsed.as_millis().min(u128::from(u32::MAX)) as u32,
        estimated: true,
    }
}

/// The Brave key mapping (§17.1): an empty/whitespace setting is no key
/// (DuckDuckGo Lite); a real key maps into the router's
/// `skip_serializing` field and is never logged (§17.2).
fn map_brave_key(key: Option<&str>) -> Option<String> {
    key.map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
}

/// A zeroed turn summary (the visible-failure path before a turn exists).
fn zeroed_summary() -> selora_ai::AgentSummary {
    selora_ai::AgentSummary {
        iterations: 0,
        tools_used: 0,
        approved: 0,
        denied: 0,
        tokens: 0,
        cost_usd: 0.0,
        completed: false,
    }
}

/// Effect (§17.6): run one agent turn — 5c's loop over 5a's gateway
/// stream and 5b's router rooted at the workspace — relaying its events
/// as the four agent [`AppMessage`]s, and return the turn's decision
/// channel for the dispatcher to key by request id.
///
/// The relay mirrors [`spawn_chat`]: every event maps to one message with
/// `notify()` after each send; `AgentEvent::Delta` reuses
/// `AppMessage::ChatDelta` (the `chat_ids` map keys both); `State` is not
/// forwarded (the panel derives its status line from the card states —
/// `Thinking` is in-flight-with-no-running-card, when the bubble's wave
/// already shows). `TurnDone` synthesizes the receipt (see
/// [`synthesize_receipt`]). A send failure means the window is gone: stop
/// relaying, then still join the loop task so a panicked turn becomes one
/// synthetic `Failed` end when `TurnDone` never arrived (the
/// worker-died pattern, `spawn_chat`).
#[allow(clippy::too_many_arguments)]
pub fn spawn_agent(
    handle: &Handle,
    notify: &Notify,
    tx: UnboundedSender<AppMessage>,
    base_url: String,
    api_key: String,
    workspace_root: PathBuf,
    turn_id: Uuid,
    request: AgentTurnRequest,
) -> UnboundedSender<selora_ai::TurnDecision> {
    let notify = notify.clone();

    // Build the turn: the client behind 5c's ChatStream adapter; the
    // router behind 5b's ToolRunner, rooted at the workspace (§17.2).
    let client = SeloraClient::new(&base_url, &api_key);
    let chat = selora_ai::ClientChatStream::new(client);
    let tool_settings = selora_ai::AgentToolSettings {
        shell: request.agent.shell.clone(),
        auto_approve_commands: request.agent.auto_approve_commands.clone(),
        // The Brave key maps into the router's `skip_serializing` field —
        // it is never persisted back out and never logged (§17.2).
        brave_api_key: map_brave_key(request.agent.brave_search_api_key.as_deref()),
    };
    let tools = selora_ai::RouterToolRunner::new(selora_ai::ToolContext::new(
        workspace_root,
        tool_settings,
    ));
    // §17.3 caps from settings.agent; the context window + the 0.20
    // reserve come from the model via spec_defaults (turn.rs:78-96).
    let caps = selora_ai::TurnCaps {
        max_tool_iterations: request.agent.max_iterations,
        max_turn_secs: request.agent.max_turn_secs,
        max_cost_usd: request.agent.max_cost_usd,
        ..selora_ai::TurnCaps::spec_defaults(request.model.context_window)
    };
    let input =
        selora_ai::TurnInput::new(request.model.clone(), request.conversation).with_caps(caps);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<selora_ai::AgentEvent>();
    let (decision_tx, join) = selora_ai::spawn_agent_turn(handle, input, chat, tools, events_tx);

    let model_id = request.model.id.clone();
    let started = Instant::now();
    handle.spawn(async move {
        let mut done_seen = false;
        while let Some(event) = events_rx.recv().await {
            let message = match event {
                selora_ai::AgentEvent::Delta(text) => AppMessage::ChatDelta(turn_id, text),
                selora_ai::AgentEvent::ToolCallStarted(call) => {
                    AppMessage::ToolCallStarted(turn_id, call)
                }
                selora_ai::AgentEvent::ToolResult(result) => {
                    AppMessage::ToolResult(turn_id, result)
                }
                selora_ai::AgentEvent::ApprovalNeeded { approval_id, card } => {
                    AppMessage::ApprovalNeeded(turn_id, approval_id, card)
                }
                selora_ai::AgentEvent::State(_) => continue,
                selora_ai::AgentEvent::Compacted(_) => continue,
                selora_ai::AgentEvent::CompactionFailed(_) => continue,
                selora_ai::AgentEvent::TurnDone(report) => {
                    done_seen = true;
                    AppMessage::AgentTurnDone(
                        turn_id,
                        synthesize_receipt(turn_id, &model_id, &report.summary, started.elapsed()),
                        report.summary,
                        report.end,
                    )
                }
            };
            if tx.send(message).is_err() {
                break; // window closed — stop relaying, still join below
            }
            notify();
            if done_seen {
                break;
            }
        }
        // Always join the loop task: a panicked turn without a TurnDone
        // becomes one synthetic Failed end (never a hang, PLAN.md §8).
        if let Err(join_error) = join.await {
            if !done_seen {
                let _ = tx.send(AppMessage::AgentTurnDone(
                    turn_id,
                    synthesize_receipt(turn_id, &model_id, &zeroed_summary(), started.elapsed()),
                    zeroed_summary(),
                    selora_ai::TurnEnd::Failed(format!("agent worker died: {join_error}")),
                ));
                notify();
            }
        }
    });
    decision_tx
}

/// A turn that can never start (no API key / no workspace root) still
/// gets its terminal message — the panel is waiting on this uuid and a
/// hang is never acceptable (PLAN.md §8).
pub fn send_agent_turn_failed(
    tx: &UnboundedSender<AppMessage>,
    notify: &Notify,
    turn_id: Uuid,
    model: &str,
    reason: &str,
) {
    let _ = tx.send(AppMessage::AgentTurnDone(
        turn_id,
        synthesize_receipt(turn_id, model, &zeroed_summary(), std::time::Duration::ZERO),
        zeroed_summary(),
        selora_ai::TurnEnd::Failed(reason.to_owned()),
    ));
    notify();
}

// ---------------------------------------------------------------------------
// The effect dispatcher (ARCHITECTURE.md §2.2: one consumer task)
// ---------------------------------------------------------------------------

/// Spawn the single `Effect` consumer task. It routes each effect to a
/// worker, tracks the in-memory Selora key, and follows the gateway base URL
/// from settings as they load/save. The task ends when the UI drops the
/// effect sender (window closed).
///
/// Agent Mode (§17.6): the dispatcher also tracks the workspace root (the
/// router's `ToolContext` needs it — `Effect::OpenFolder` is the only
/// re-root effect) and the live turns' decision channels, keyed by request
/// id (`agent_turns`; an ended turn's closed channel is pruned on the next
/// `AgentSend`, bounding the map).
pub fn spawn_dispatcher(
    handle: &Handle,
    notify: Notify,
    effect_rx: UnboundedReceiver<Effect>,
    msg_tx: UnboundedSender<AppMessage>,
    base_url: String,
    lsp_effect_tx: UnboundedSender<selora_lsp::LspEffect>,
) {
    let dispatch_handle = handle.clone();
    handle.spawn(async move {
        let mut effect_rx = effect_rx;
        let mut api_key: Option<String> = None;
        let mut base_url = base_url;
        let mut workspace_root: Option<PathBuf> = None;
        let mut agent_turns: std::collections::HashMap<
            Uuid,
            UnboundedSender<selora_ai::TurnDecision>,
        > = std::collections::HashMap::new();
        // Phase R1: in-flight plain chats' cancel watches (the composer's
        // Stop → `Effect::SeloraChatAbort`). A finished chat's stream task
        // drops its receiver; `receiver_count()` prunes those.
        let mut chat_cancels: std::collections::HashMap<Uuid, watch::Sender<bool>> =
            std::collections::HashMap::new();
        while let Some(effect) = effect_rx.recv().await {
            match effect {
                Effect::OpenFile(path) => {
                    spawn_open_file(&dispatch_handle, &notify, &msg_tx, path);
                }
                Effect::OpenFolder(root) => {
                    workspace_root = Some(root.clone());
                    spawn_open_folder(&dispatch_handle, &notify, &msg_tx, root);
                }
                Effect::RefreshFileTree(root) => {
                    spawn_refresh_tree(&dispatch_handle, &notify, &msg_tx, root);
                }
                Effect::SaveBuffer { path, buffer } => {
                    spawn_save_buffer(&dispatch_handle, &notify, &msg_tx, path, buffer);
                }
                Effect::SettingsLoad => {
                    spawn_settings_load(&dispatch_handle, &notify, &msg_tx);
                }
                Effect::SettingsSave(settings) => {
                    base_url = settings.selora.base_url.clone();
                    spawn_settings_save(&dispatch_handle, &notify, &msg_tx, settings);
                }
                Effect::SettingsSynced { base_url: url } => {
                    // The dialog already wrote the file — this only
                    // refreshes the dispatcher's view. An empty URL means
                    // the field was cleared: fall back to the default (the
                    // `SeloraConnect` arm's rule, so ping/chat don't build
                    // requests against an empty origin).
                    base_url = if url.is_empty() {
                        DEFAULT_BASE_URL.to_owned()
                    } else {
                        url
                    };
                }
                Effect::SeloraConnect(key) => {
                    // The key becomes the session key immediately (the
                    // `SeloraAutoConnect` idiom): the ping loop, chat, and
                    // ghost text must all see it — otherwise the next
                    // `SeloraPing` (30s) answers "no key" and flips a just-
                    // connected status line back to `Disconnected`.
                    api_key = Some(key.clone());
                    base_url = if base_url.is_empty() {
                        DEFAULT_BASE_URL.to_owned()
                    } else {
                        base_url
                    };
                    spawn_connect(
                        &dispatch_handle,
                        &notify,
                        msg_tx.clone(),
                        base_url.clone(),
                        key,
                        true,
                    );
                }
                Effect::SeloraAutoConnect { base_url: url } => {
                    // Boot restore (Phase 4a): the key's home is the OS
                    // keyring; a stored key becomes the session key and is
                    // re-validated (the reply drives the status bar). The
                    // read is a single Credential Manager call — inline.
                    base_url = url;
                    match selora_settings::keyring::api_key_get() {
                        Ok(Some(key)) => {
                            api_key = Some(key.clone());
                            spawn_connect(
                                &dispatch_handle,
                                &notify,
                                msg_tx.clone(),
                                base_url.clone(),
                                key,
                                false,
                            );
                        }
                        // No stored key is a fact the shell acts on (Phase
                        // 6a: the first-run splash) — say so explicitly; a
                        // silent arm is indistinguishable from "not yet
                        // resolved".
                        Ok(None) => {
                            let _ = msg_tx.send(AppMessage::NoStoredKey);
                            notify();
                        }
                        Err(error) => {
                            let _ = msg_tx.send(AppMessage::KeyringError(error));
                            notify();
                        }
                    }
                }
                Effect::SeloraDisconnect => {
                    api_key = None;
                    spawn_disconnect(&dispatch_handle, &notify, msg_tx.clone());
                }
                Effect::CheckForUpdates => {
                    spawn_update_check(&dispatch_handle, &notify, msg_tx.clone());
                }
                Effect::SeloraListModels => match api_key.clone() {
                    Some(key) => {
                        spawn_connect(
                            &dispatch_handle,
                            &notify,
                            msg_tx.clone(),
                            base_url.clone(),
                            key,
                            false,
                        );
                    }
                    None => {
                        // No silent no-op (PLAN.md §8): an unconnected list
                        // request fails visibly.
                        let _ = msg_tx.send(AppMessage::ModelsLoaded(Err(SeloraError::Auth(
                            "not connected — connect a Selora key first".into(),
                        ))));
                        notify();
                    }
                },
                Effect::SeloraChat(chat_id, request) => match api_key.clone() {
                    Some(key) => {
                        // Prune finished chats' dead senders first.
                        chat_cancels.retain(|_, cancel| cancel.receiver_count() > 0);
                        let (cancel_tx, cancel_rx) = watch::channel(false);
                        spawn_chat(
                            &dispatch_handle,
                            &notify,
                            msg_tx.clone(),
                            base_url.clone(),
                            key,
                            chat_id,
                            request,
                            cancel_rx,
                        );
                        chat_cancels.insert(chat_id, cancel_tx);
                    }
                    None => {
                        let _ = msg_tx.send(AppMessage::ChatError(
                            chat_id,
                            SeloraError::Auth(
                                "no API key connected — keyring storage arrives in Phase 4".into(),
                            ),
                        ));
                        notify();
                    }
                },
                Effect::SeloraChatAbort(chat_id) => {
                    // A true cancel (Phase R1): the watch flip drops the
                    // SSE request future (the connection closes) and the
                    // relay ends silently — a user Stop is never an
                    // error. Unknown/ended ids are no-ops.
                    if let Some(cancel) = chat_cancels.remove(&chat_id) {
                        let _ = cancel.send(true);
                    }
                }
                Effect::SeloraPing => match api_key.clone() {
                    Some(key) => {
                        spawn_selora_ping(
                            &dispatch_handle,
                            &notify,
                            msg_tx.clone(),
                            base_url.clone(),
                            key,
                        );
                    }
                    // No key → disconnected, no network call: `Connected`
                    // must always mean key present + gateway answered.
                    None => {
                        let _ = msg_tx.send(AppMessage::SeloraStatus(
                            selora_ai::SeloraStatus::Disconnected,
                        ));
                        notify();
                    }
                },
                Effect::SeloraGhostText {
                    path,
                    prefix,
                    model,
                    request_caret,
                } => match api_key.clone() {
                    Some(key) => {
                        spawn_ghost(
                            &dispatch_handle,
                            &notify,
                            msg_tx.clone(),
                            base_url.clone(),
                            key,
                            path,
                            prefix,
                            model,
                            request_caret,
                        );
                    }
                    // No key: a None suggestion — the ghost simply never
                    // appears (not an error; PLAN.md §6.4 keeps ghost text
                    // best-effort).
                    None => {
                        let _ = msg_tx.send(AppMessage::GhostTextReady {
                            path,
                            request_caret,
                            suggestion: None,
                        });
                        notify();
                    }
                },
                Effect::SeloraKeyringSet(key) => {
                    // The OS keyring is Phase 4 (ARCHITECTURE.md §7); for now
                    // the key lives only in this dispatcher's memory.
                    api_key = Some(key);
                }
                // Agent Mode (Phase 5, ARCHITECTURE.md §17.6): the turn
                // runs on the runtime; the relay reduces its events to the
                // four agent AppMessages; the decision channel keys by
                // request id.
                Effect::AgentSend(turn_id, request) => {
                    match (api_key.clone(), workspace_root.clone()) {
                        (Some(key), Some(root)) => {
                            // Prune ended turns' closed channels (the loop
                            // drops its receiver at turn end).
                            agent_turns.retain(|_, decisions| !decisions.is_closed());
                            let decisions = spawn_agent(
                                &dispatch_handle,
                                &notify,
                                msg_tx.clone(),
                                base_url.clone(),
                                key,
                                root,
                                turn_id,
                                request,
                            );
                            agent_turns.insert(turn_id, decisions);
                        }
                        (key, root) => {
                            // A visible failure, never a hang (PLAN.md §8):
                            // the panel is parked on this uuid's stream.
                            let reason = match (&key, &root) {
                                (None, _) => "no API key connected — connect a Selora key first",
                                (Some(_), None) => {
                                    "no workspace folder open — agent tools need a root"
                                }
                                (Some(_), Some(_)) => unreachable!("the Ok arm matched above"),
                            };
                            send_agent_turn_failed(
                                &msg_tx,
                                &notify,
                                turn_id,
                                &request.model.id,
                                reason,
                            );
                        }
                    }
                }
                Effect::AgentDecision(turn_id, approval_id, decision) => {
                    // An ended turn's channel is closed (the loop drops its
                    // receiver); an unknown uuid is stale. Both are silent
                    // no-ops, never a panic.
                    if let Some(decisions) = agent_turns.get(&turn_id) {
                        if decisions.send((approval_id, decision)).is_err() {
                            agent_turns.remove(&turn_id);
                        }
                    }
                }
                Effect::GitStatus(root) => {
                    spawn_git_status(&dispatch_handle, &notify, &msg_tx, root);
                }
                Effect::GitDiff { root, path } => {
                    spawn_git_diff(&dispatch_handle, &notify, &msg_tx, root, path);
                }
                Effect::GitStage(path) => {
                    spawn_git_stage(&dispatch_handle, &notify, &msg_tx, path);
                }
                Effect::GitUnstage(path) => {
                    spawn_git_unstage(&dispatch_handle, &notify, &msg_tx, path);
                }
                Effect::GitCommit(root, message) => {
                    spawn_git_commit(&dispatch_handle, &notify, &msg_tx, root, message);
                }
                Effect::Search { id, root, query } => {
                    spawn_search(&dispatch_handle, &notify, &msg_tx, id, root, query);
                }
                Effect::Lsp(effect) => {
                    // Forward to the LSP manager task (spawned by the shell
                    // once settings + workspace root are known; effects sent
                    // before that queue in the channel harmlessly).
                    let _ = lsp_effect_tx.send(effect);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::runtime::Handle;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Headless stand-in for the UI's `ctx.request_repaint()` notify seam.
    fn noop_notify() -> Notify {
        Arc::new(|| {})
    }

    /// The LSP effect channel the dispatcher forwards onto (tests never
    /// drive the manager, so the receiver is dropped immediately).
    fn lsp_effect_tx() -> UnboundedSender<selora_lsp::LspEffect> {
        mpsc::unbounded_channel::<selora_lsp::LspEffect>().0
    }

    const TEST_KEY: &str = "sk-gw-test-key";

    /// A happy-path SSE body: two deltas, a usage-bearing final chunk, DONE.
    const STREAM_SSE: &str = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2,\"cost_usd\":0.0013}}\n\n",
        "data: [DONE]\n\n",
    );

    const MODELS_JSON: &str = r#"{"object":"list","data":[
        {"id":"test-model","display_name":"Test Model"},
        {"id":"other-model"}
    ]}"#;

    fn unique_temp_path(label: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "selora-app-wiring-{label}-{}-{n}",
            std::process::id()
        ))
    }

    // --- file effects --------------------------------------------------------

    #[test]
    fn read_file_text_reads_small_files_and_rejects_the_oversize() {
        let small = unique_temp_path("small.txt");
        std::fs::write(&small, b"hello").expect("write");
        assert_eq!(read_file_text(&small).expect("read small file"), "hello");

        // Over the cap: a file extended to 4 MB + 1 (set_len, no bytes
        // actually written).
        let big = unique_temp_path("big.bin");
        let file = std::fs::File::create(&big).expect("create");
        file.set_len(MAX_FILE_BYTES + 1).expect("set_len");
        drop(file);

        let error = read_file_text(&big).expect_err("oversize must be rejected");
        assert!(error.to_string().contains("Lite limit"), "got: {error:?}");

        let _ = std::fs::remove_file(small);
        let _ = std::fs::remove_file(big);
    }

    #[test]
    fn read_file_text_missing_file_is_a_clean_io_error() {
        let missing = unique_temp_path("missing.txt");
        assert!(matches!(read_file_text(&missing), Err(SeloraError::Io(_))));
    }

    #[test]
    fn atomic_write_text_writes_and_leaves_no_temp_file() {
        let target = unique_temp_path("nested").join("out.rs");
        atomic_write_text(&target, "one\ntwo").expect("write");
        assert_eq!(
            std::fs::read_to_string(&target).expect("read back"),
            "one\ntwo"
        );
        assert!(!target
            .parent()
            .map(|dir| dir.join("out.rs.tmp").exists())
            .unwrap_or(false));

        // A second write replaces the first atomically.
        atomic_write_text(&target, "three").expect("replace");
        assert_eq!(
            std::fs::read_to_string(&target).expect("read back"),
            "three"
        );

        let _ = std::fs::remove_dir_all(target.parent().expect("parent"));
    }

    // --- the dispatcher ------------------------------------------------------

    /// Drain messages until one matching `stop` arrives (with a timeout so a
    /// broken test fails instead of hanging).
    async fn recv_until(
        rx: &mut UnboundedReceiver<AppMessage>,
        stop: impl Fn(&AppMessage) -> bool,
    ) -> AppMessage {
        let deadline = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let message = rx.recv().await.expect("channel open");
                if stop(&message) {
                    return message;
                }
            }
        });
        deadline.await.expect("timed out waiting for a message")
    }

    #[tokio::test]
    async fn dispatcher_opens_and_saves_a_file() {
        let handle = Handle::current();
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        spawn_dispatcher(
            &handle,
            noop_notify(),
            effect_rx,
            msg_tx.clone(),
            DEFAULT_BASE_URL.into(),
            lsp_effect_tx(),
        );

        let file = unique_temp_path("dispatch.rs");
        std::fs::write(&file, b"fn main() {}").expect("write");
        let _ = effect_tx.send(Effect::OpenFile(file.clone()));
        let loaded = recv_until(&mut msg_rx, |m| {
            matches!(
                m,
                AppMessage::BufferLoaded { .. } | AppMessage::BufferLoadFailed { .. }
            )
        })
        .await;
        match loaded {
            AppMessage::BufferLoaded { path, text } => {
                assert_eq!(path, file);
                assert_eq!(text, "fn main() {}");
            }
            other => panic!("expected BufferLoaded, got {other:?}"),
        }

        // Save a modified buffer through the same dispatcher.
        let buffer = Arc::new(RwLock::new(Buffer::from_text("fn main() { /* edited */ }")));
        let _ = effect_tx.send(Effect::SaveBuffer {
            path: file.clone(),
            buffer,
        });
        let saved = recv_until(&mut msg_rx, |m| {
            matches!(
                m,
                AppMessage::BufferSaved(_) | AppMessage::BufferSaveFailed { .. }
            )
        })
        .await;
        match saved {
            AppMessage::BufferSaved(path) => assert_eq!(path, file),
            other => panic!("expected BufferSaved, got {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(&file).expect("read back"),
            "fn main() { /* edited */ }"
        );

        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn dispatcher_rejects_an_oversize_open() {
        let handle = Handle::current();
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        spawn_dispatcher(
            &handle,
            noop_notify(),
            effect_rx,
            msg_tx,
            DEFAULT_BASE_URL.into(),
            lsp_effect_tx(),
        );

        let big = unique_temp_path("dispatch-big.bin");
        let file = std::fs::File::create(&big).expect("create");
        file.set_len(MAX_FILE_BYTES + 1).expect("set_len");
        drop(file);

        let _ = effect_tx.send(Effect::OpenFile(big.clone()));
        let message = recv_until(&mut msg_rx, |m| {
            matches!(m, AppMessage::BufferLoadFailed { .. })
        })
        .await;
        match message {
            AppMessage::BufferLoadFailed { path, error } => {
                assert_eq!(path, big);
                assert!(error.to_string().contains("Lite limit"));
            }
            other => panic!("expected BufferLoadFailed, got {other:?}"),
        }

        let _ = std::fs::remove_file(big);
    }

    #[tokio::test]
    async fn dispatcher_builds_the_tree_for_a_folder() {
        let handle = Handle::current();
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        spawn_dispatcher(
            &handle,
            noop_notify(),
            effect_rx,
            msg_tx,
            DEFAULT_BASE_URL.into(),
            lsp_effect_tx(),
        );

        let root = unique_temp_path("tree-root");
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("src").join("main.rs"), b"x").expect("write");

        let _ = effect_tx.send(Effect::OpenFolder(root.clone()));
        let message = recv_until(&mut msg_rx, |m| {
            matches!(m, AppMessage::FileTreeUpdated { .. })
        })
        .await;
        match message {
            AppMessage::FileTreeUpdated {
                root: message_root,
                tree,
            } => {
                assert_eq!(message_root, root);
                assert!(tree.is_dir);
                assert!(tree.child("src").is_some_and(|src| src.is_dir));
            }
            other => panic!("expected FileTreeUpdated, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn dispatcher_settings_round_trip_in_a_sandboxed_dir() {
        // Route Settings::save/load away from the real %APPDATA% for the
        // duration of this test. No other test in this binary touches the
        // override, so the window is race-free.
        let dir = unique_temp_path("settings-sandbox");
        std::env::set_var(selora_settings::SETTINGS_DIR_ENV, &dir);

        let handle = Handle::current();
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        spawn_dispatcher(
            &handle,
            noop_notify(),
            effect_rx,
            msg_tx,
            DEFAULT_BASE_URL.into(),
            lsp_effect_tx(),
        );

        let mut settings = Settings::default();
        settings.editor.font_size = 19;
        let _ = effect_tx.send(Effect::SettingsSave(settings));
        let message = recv_until(&mut msg_rx, |m| {
            matches!(m, AppMessage::SettingsSaved | AppMessage::SettingsError(_))
        })
        .await;
        assert!(
            matches!(message, AppMessage::SettingsSaved),
            "got {message:?}"
        );
        assert!(dir.join("settings.json").is_file());

        let _ = effect_tx.send(Effect::SettingsLoad);
        let message = recv_until(&mut msg_rx, |m| matches!(m, AppMessage::SettingsLoaded(_))).await;
        match message {
            AppMessage::SettingsLoaded(loaded) => assert_eq!(loaded.editor.font_size, 19),
            other => panic!("expected SettingsLoaded, got {other:?}"),
        }

        std::env::remove_var(selora_settings::SETTINGS_DIR_ENV);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn dispatcher_reports_git_no_repo_for_a_plain_folder() {
        let handle = Handle::current();
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        spawn_dispatcher(
            &handle,
            noop_notify(),
            effect_rx,
            msg_tx,
            DEFAULT_BASE_URL.into(),
            lsp_effect_tx(),
        );

        // A plain temp folder: not an error, just GitNoRepo.
        let root = unique_temp_path("not-a-repo");
        std::fs::create_dir_all(&root).expect("mkdir");
        let _ = effect_tx.send(Effect::GitStatus(root.clone()));
        let message = recv_until(&mut msg_rx, |m| matches!(m, AppMessage::GitNoRepo)).await;
        assert!(matches!(message, AppMessage::GitNoRepo));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn dispatcher_unconnected_chat_fails_visibly() {
        let handle = Handle::current();
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        spawn_dispatcher(
            &handle,
            noop_notify(),
            effect_rx,
            msg_tx,
            DEFAULT_BASE_URL.into(),
            lsp_effect_tx(),
        );

        let chat_id = Uuid::new_v4();
        let request = selora_ai::ChatRequest::new("test-model", vec![]);
        let _ = effect_tx.send(Effect::SeloraChat(chat_id, request));

        let message = recv_until(&mut msg_rx, |m| matches!(m, AppMessage::ChatError(..))).await;
        match message {
            AppMessage::ChatError(id, error) => {
                assert_eq!(id, chat_id);
                assert!(error.to_string().contains("no API key"));
            }
            other => panic!("expected ChatError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn manual_connect_makes_the_session_key_visible_to_the_ping_loop() {
        // The mock rejects every request: validation fails (so the real OS
        // keyring is never written — a failed connect persists nothing),
        // but the ping that follows must still *reach* the gateway with
        // the just-connected key. Before the fix the dispatcher never
        // adopted the key, so the ping answered `Disconnected` without a
        // network call and the status line flipped back 30s after a
        // successful connect.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401))
            .expect(2)
            .mount(&server)
            .await;

        let handle = Handle::current();
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        spawn_dispatcher(
            &handle,
            noop_notify(),
            effect_rx,
            msg_tx,
            server.uri(),
            lsp_effect_tx(),
        );

        let _ = effect_tx.send(Effect::SeloraConnect("sk-gw-rejected".into()));
        let message = recv_until(&mut msg_rx, |m| matches!(m, AppMessage::SeloraStatus(_))).await;
        match &message {
            AppMessage::SeloraStatus(selora_ai::SeloraStatus::Error(_)) => {}
            other => panic!("validation must fail against the 401 mock, got {other:?}"),
        }

        let _ = effect_tx.send(Effect::SeloraPing);
        let message = recv_until(&mut msg_rx, |m| matches!(m, AppMessage::SeloraStatus(_))).await;
        match &message {
            // The ping reached the gateway carrying the session key (the
            // second 401) — not the keyless `Disconnected` reply.
            AppMessage::SeloraStatus(selora_ai::SeloraStatus::Error(_)) => {}
            other => panic!("ping must use the just-connected key, got {other:?}"),
        }

        // Both requests (connect + ping) hit the mock.
        server.verify().await;
    }

    // --- the Phase 1c network half, against a mock gateway -------------------

    #[tokio::test]
    async fn ghost_completion_against_a_mock_gateway_round_trips_the_caret() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"choices":[{"message":{"role":"assistant","content":"foo(bar);"}}]}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let handle = Handle::current();
        let (tx, mut rx) = mpsc::unbounded_channel::<AppMessage>();
        let path = unique_temp_path("ghost.rs");
        let caret = selora_core::Position::new(3, 7);
        spawn_ghost(
            &handle,
            &noop_notify(),
            tx,
            server.uri(),
            TEST_KEY.into(),
            path.clone(),
            "fn main() { ".into(),
            "test-model".into(),
            caret,
        );

        match rx.recv().await.expect("ghost reply") {
            AppMessage::GhostTextReady {
                path: reply_path,
                request_caret,
                suggestion,
            } => {
                assert_eq!(reply_path, path);
                assert_eq!(request_caret, caret);
                assert_eq!(suggestion.as_deref(), Some("foo(bar);"));
            }
            other => panic!("expected GhostTextReady, got {other:?}"),
        }
    }

    /// The no-key ghost path (dispatcher logic): a best-effort `None`
    /// suggestion, never an error — the ghost just never appears
    /// (PLAN.md §6.4).
    #[tokio::test]
    async fn dispatcher_ghost_text_without_a_key_replies_none() {
        let handle = Handle::current();
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        spawn_dispatcher(
            &handle,
            noop_notify(),
            effect_rx,
            msg_tx,
            DEFAULT_BASE_URL.into(),
            lsp_effect_tx(),
        );

        let path = unique_temp_path("ghost-nokey.rs");
        let _ = effect_tx.send(Effect::SeloraGhostText {
            path: path.clone(),
            prefix: "fn main()".into(),
            model: "test-model".into(),
            request_caret: selora_core::Position::new(0, 9),
        });
        let message = recv_until(&mut msg_rx, |m| {
            matches!(m, AppMessage::GhostTextReady { .. })
        })
        .await;
        match message {
            AppMessage::GhostTextReady {
                path: reply_path,
                suggestion,
                ..
            } => {
                assert_eq!(reply_path, path);
                assert_eq!(suggestion, None);
            }
            other => panic!("expected GhostTextReady, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn connect_against_a_mock_gateway_lists_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(MODELS_JSON))
            .expect(1)
            .mount(&server)
            .await;

        let handle = Handle::current();
        let (tx, mut rx) = mpsc::unbounded_channel::<AppMessage>();
        spawn_connect(
            &handle,
            &noop_notify(),
            tx,
            server.uri(),
            TEST_KEY.into(),
            false,
        );

        let message = rx.recv().await.expect("connect result message");
        match message {
            AppMessage::ModelsLoaded(Ok(models)) => assert_eq!(models.len(), 2),
            other => panic!("expected ModelsLoaded(Ok), got {other:?}"),
        }
    }

    /// A cancel pair whose sender stays alive for the test's duration
    /// (a dropped sender would read as a cancel in the stream task's
    /// select — the dispatcher holds the sender in production).
    fn watch_keepalive() -> (watch::Sender<bool>, watch::Receiver<bool>) {
        static KEEP: std::sync::Mutex<Vec<watch::Sender<bool>>> = std::sync::Mutex::new(Vec::new());
        let pair = watch::channel(false);
        KEEP.lock().expect("lock").push(pair.0.clone());
        pair
    }

    #[tokio::test]
    async fn chat_streams_deltas_and_receipt_against_a_mock_gateway() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(STREAM_SSE))
            .expect(1)
            .mount(&server)
            .await;

        let handle = Handle::current();
        let (tx, mut rx) = mpsc::unbounded_channel::<AppMessage>();
        let chat_id = Uuid::new_v4();
        let request = selora_ai::ChatRequest::new("test-model", vec![]).with_stream(true);
        spawn_chat(
            &handle,
            &noop_notify(),
            tx,
            server.uri(),
            TEST_KEY.into(),
            chat_id,
            request,
            watch_keepalive().1,
        );

        // Drain exactly like the frame loop: reduce messages in arrival
        // order until the stream's terminal message.
        let mut deltas = String::new();
        let mut receipt = None;
        loop {
            let message = rx.recv().await.expect("chat message");
            let terminal = matches!(
                message,
                AppMessage::ChatDone(..) | AppMessage::ChatError(..)
            );
            match message {
                AppMessage::ChatDelta(id, delta) => {
                    assert_eq!(id, chat_id);
                    deltas.push_str(&delta);
                }
                AppMessage::ChatDone(id, r) => {
                    assert_eq!(id, chat_id);
                    receipt = Some(r);
                }
                AppMessage::ChatError(_, error) => panic!("unexpected chat error: {error}"),
                other => panic!("unexpected message: {other:?}"),
            }
            if terminal {
                break;
            }
        }

        assert_eq!(deltas, "Hello world");
        let receipt = receipt.expect("receipt after Done");
        assert_eq!(receipt.prompt_tokens, 9);
        assert_eq!(receipt.completion_tokens, 2);
        assert_eq!(receipt.cost_usd, 0.0013);
    }

    /// Phase R1: the cancel watch is a true abort — the in-flight request
    /// future is dropped (the connection closes) and the relay ends
    /// WITHOUT surfacing a terminal message (a user stop is never an
    /// error). Observable as the message channel closing: the relay owns
    /// its only sender.
    #[tokio::test]
    async fn chat_cancel_ends_the_stream_silently() {
        let server = MockServer::start().await;
        // The response stalls server-side: without the cancel the relay
        // would sit on the request for the whole delay.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(STREAM_SSE)
                    .set_delay(Duration::from_secs(10)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let handle = Handle::current();
        let (tx, mut rx) = mpsc::unbounded_channel::<AppMessage>();
        let chat_id = Uuid::new_v4();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        spawn_chat(
            &handle,
            &noop_notify(),
            tx,
            server.uri(),
            TEST_KEY.into(),
            chat_id,
            selora_ai::ChatRequest::new("test-model", vec![]).with_stream(true),
            cancel_rx,
        );

        // Synchronize with the server instead of assuming the nested stream
        // task was scheduled within a fixed sleep under parallel test load.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if server
                    .received_requests()
                    .await
                    .is_some_and(|requests| !requests.is_empty())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the request reaches the delayed mock before cancellation");
        assert!(
            rx.try_recv().is_err(),
            "nothing arrived while the response stalls"
        );

        // The user stops: the channel closes (relay ended) and no
        // terminal message ever surfaced.
        cancel_tx.send(true).expect("cancel");
        let closed = matches!(
            tokio::time::timeout(Duration::from_secs(2), rx.recv()).await,
            Ok(None)
        );
        assert!(closed, "the relay ended silently (no terminal message)");
    }

    #[tokio::test]
    async fn chat_missing_done_surfaces_the_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n"),
            )
            .mount(&server)
            .await;

        let handle = Handle::current();
        let (tx, mut rx) = mpsc::unbounded_channel::<AppMessage>();
        let chat_id = Uuid::new_v4();
        spawn_chat(
            &handle,
            &noop_notify(),
            tx,
            server.uri(),
            TEST_KEY.into(),
            chat_id,
            selora_ai::ChatRequest::new("test-model", vec![]).with_stream(true),
            watch_keepalive().1,
        );

        let mut deltas = String::new();
        loop {
            let message = rx.recv().await.expect("chat message");
            match message {
                AppMessage::ChatDelta(_, delta) => deltas.push_str(&delta),
                AppMessage::ChatError(_, error) => {
                    assert_eq!(deltas, "Hi");
                    assert!(
                        error.to_string().contains("closed before [DONE]"),
                        "got: {error}"
                    );
                    break;
                }
                other => panic!("expected ChatDelta/ChatError, got {other:?}"),
            }
        }
    }

    // --- Agent Mode (Phase 5d, ARCHITECTURE.md §17.6) -------------------------

    /// A catalog-shaped model for the turn requests.
    fn agent_model() -> selora_ai::Model {
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

    fn agent_request(agent: selora_settings::AgentSettings) -> AgentTurnRequest {
        AgentTurnRequest {
            model: agent_model(),
            conversation: vec![selora_ai::ChatMessage::user("edit it")],
            agent: Box::new(agent),
        }
    }

    /// One streamed tool call: id+name on the first chunk, the arguments
    /// in one fragment, then `finish_reason: tool_calls` (the
    /// fragment-split shapes are selora-ai's own tests' business).
    fn tool_call_sse(call_id: &str, name: &str, arguments: &str) -> String {
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

    /// A plain answer with a usage-bearing final chunk.
    fn answer_sse(text: &str, cost_usd: f64) -> String {
        format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":5,\"cost_usd\":{cost_usd}}}}}\n\n\
             data: [DONE]\n\n"
        )
    }

    /// The dispatcher against a mock gateway, with a key and a workspace
    /// root installed.
    async fn agent_dispatcher(
        server: &MockServer,
        root: &Path,
    ) -> (UnboundedSender<Effect>, UnboundedReceiver<AppMessage>) {
        let handle = Handle::current();
        let (msg_tx, msg_rx) = mpsc::unbounded_channel::<AppMessage>();
        let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
        spawn_dispatcher(
            &handle,
            noop_notify(),
            effect_rx,
            msg_tx,
            server.uri(),
            lsp_effect_tx(),
        );
        let _ = effect_tx.send(Effect::SeloraKeyringSet(TEST_KEY.into()));
        let _ = effect_tx.send(Effect::OpenFolder(root.to_path_buf()));
        (effect_tx, msg_rx)
    }

    /// The full scripted turn (the brief's acceptance): read (auto-runs on
    /// the real temp fs) → write (parked) → Approve → the router writes
    /// the real file → the final answer's receipt lands with the loop's
    /// cost and `TurnEnd::Completed`, and the grown conversation went back
    /// out on the wire (the round-3 mount's body matcher).
    #[tokio::test]
    async fn agent_turn_runs_read_write_approve_and_writes_the_real_file() {
        let root = unique_temp_path("agent-happy");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("main.rs"), "fn main() {}\n").expect("write main.rs");

        let server = MockServer::start().await;
        // Every round's body still carries the first user message, so a
        // plain mount order would let round 1 shadow the rest (wiremock
        // tries same-priority mocks in insertion order). Priorities make
        // the grown-conversation rounds win their own requests: lower
        // number = tried first.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains("edit it"))
            .respond_with(ResponseTemplate::new(200).set_body_string(tool_call_sse(
                "call_read_1",
                "read_file",
                r#"{"path":"main.rs"}"#,
            )))
            .with_priority(3)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains("tool_call_id"))
            .respond_with(ResponseTemplate::new(200).set_body_string(tool_call_sse(
                "call_write_1",
                "write_file",
                r#"{"path":"new.rs","content":"// from the agent\n"}"#,
            )))
            .with_priority(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            // Round 3's body carries the write's tool result — the grown
            // conversation is on the wire.
            .and(wiremock::matchers::body_string_contains("wrote"))
            .respond_with(ResponseTemplate::new(200).set_body_string(answer_sse("Done.", 0.0021)))
            .with_priority(1)
            .mount(&server)
            .await;

        let (effect_tx, mut msg_rx) = agent_dispatcher(&server, &root).await;
        let turn_id = Uuid::new_v4();
        let _ = effect_tx.send(Effect::AgentSend(
            turn_id,
            agent_request(selora_settings::AgentSettings::default()),
        ));

        // Round 1: the read card opens and completes without approval.
        recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::ToolCallStarted(id, _) if *id == turn_id),
        )
        .await;
        recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::ToolResult(id, r) if *id == turn_id && r.ok),
        )
        .await;

        // Round 2: the write parks on the diff card.
        let approval = recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::ApprovalNeeded(id, _, _) if *id == turn_id),
        )
        .await;
        let approval_id = match approval {
            AppMessage::ApprovalNeeded(_, approval_id, selora_ai::ApprovalCard::Diff { .. }) => {
                approval_id
            }
            other => panic!("expected ApprovalNeeded(Diff), got {other:?}"),
        };
        assert!(
            !root.join("new.rs").exists(),
            "nothing writes before Approve"
        );

        let _ = effect_tx.send(Effect::AgentDecision(
            turn_id,
            approval_id,
            selora_ai::AgentDecision::Approve,
        ));

        // Round 3: the answer and the synthesized receipt.
        let done = recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::AgentTurnDone(id, ..) if *id == turn_id),
        )
        .await;
        match done {
            AppMessage::AgentTurnDone(id, receipt, summary, end) => {
                assert_eq!(id, turn_id);
                assert_eq!(end, selora_ai::TurnEnd::Completed);
                assert!(summary.completed);
                assert_eq!(summary.approved, 1);
                // The receipt carries the loop's own totals (round 1 was
                // usage-free → the price fallback; round 2 scripted 0.0021).
                assert!(
                    (receipt.cost_usd - 0.0021).abs() < 1e-12,
                    "receipt: {receipt:?}"
                );
                assert!(receipt.estimated);
                assert_eq!(receipt.model, "test-model");
            }
            other => panic!("expected AgentTurnDone, got {other:?}"),
        }

        // The approved write went to disk through the real router.
        assert_eq!(
            std::fs::read_to_string(root.join("new.rs")).expect("new.rs written"),
            "// from the agent\n"
        );

        // A decision for the ended turn is a silent no-op (never a panic).
        let _ = effect_tx.send(Effect::AgentDecision(
            turn_id,
            approval_id,
            selora_ai::AgentDecision::Deny,
        ));
        let _ = effect_tx.send(Effect::AgentDecision(
            Uuid::new_v4(),
            selora_ai::ApprovalId::nil(),
            selora_ai::AgentDecision::Stop,
        ));

        let _ = std::fs::remove_dir_all(root);
    }

    /// §17.4: Deny continues the loop with the denied tool result — the
    /// next request's body carries it; nothing reaches disk.
    #[tokio::test]
    async fn agent_turn_deny_continues_the_loop_and_writes_nothing() {
        let root = unique_temp_path("agent-deny");
        std::fs::create_dir_all(&root).expect("mkdir");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains("edit it"))
            .respond_with(ResponseTemplate::new(200).set_body_string(tool_call_sse(
                "call_write_1",
                "write_file",
                r#"{"path":"new.rs","content":"nope\n"}"#,
            )))
            .with_priority(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            // The denied tool result is in the next request's body.
            .and(wiremock::matchers::body_string_contains(
                "user denied this action",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(answer_sse("Understood.", 0.0)),
            )
            .with_priority(1)
            .mount(&server)
            .await;

        let (effect_tx, mut msg_rx) = agent_dispatcher(&server, &root).await;
        let turn_id = Uuid::new_v4();
        let _ = effect_tx.send(Effect::AgentSend(
            turn_id,
            agent_request(selora_settings::AgentSettings::default()),
        ));

        let approval = recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::ApprovalNeeded(id, _, _) if *id == turn_id),
        )
        .await;
        let approval_id = match approval {
            AppMessage::ApprovalNeeded(_, approval_id, _) => approval_id,
            other => panic!("{other:?}"),
        };
        let _ = effect_tx.send(Effect::AgentDecision(
            turn_id,
            approval_id,
            selora_ai::AgentDecision::Deny,
        ));

        // The denied call's ToolResult is ok:false…
        recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::ToolResult(id, r) if *id == turn_id && !r.ok),
        )
        .await;
        // …and the turn ends Completed after the model's adaptation.
        let done = recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::AgentTurnDone(id, ..) if *id == turn_id),
        )
        .await;
        match done {
            AppMessage::AgentTurnDone(_, _, summary, end) => {
                assert_eq!(end, selora_ai::TurnEnd::Completed);
                assert_eq!(summary.denied, 1);
                assert_eq!(summary.approved, 0);
            }
            other => panic!("{other:?}"),
        }
        assert!(!root.join("new.rs").exists(), "a denied write never lands");

        let _ = std::fs::remove_dir_all(root);
    }

    /// §17.3: Stop kills a running bash child; the turn ends
    /// `StoppedByUser` promptly. (A real powershell child — the router's
    /// own tests do the same.)
    #[tokio::test]
    async fn agent_turn_stop_mid_bash_ends_the_turn() {
        let root = unique_temp_path("agent-stop");
        std::fs::create_dir_all(&root).expect("mkdir");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(tool_call_sse(
                "call_bash_1",
                "bash",
                r#"{"command":"Start-Sleep -Seconds 30"}"#,
            )))
            .mount(&server)
            .await;

        let (effect_tx, mut msg_rx) = agent_dispatcher(&server, &root).await;
        let turn_id = Uuid::new_v4();
        let _ = effect_tx.send(Effect::AgentSend(
            turn_id,
            agent_request(selora_settings::AgentSettings::default()),
        ));

        // The command card parks (Start-Sleep is not allowlisted).
        let approval = recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::ApprovalNeeded(id, _, _) if *id == turn_id),
        )
        .await;
        let approval_id = match approval {
            AppMessage::ApprovalNeeded(_, approval_id, selora_ai::ApprovalCard::Command { .. }) => {
                approval_id
            }
            other => panic!("expected ApprovalNeeded(Command), got {other:?}"),
        };
        let _ = effect_tx.send(Effect::AgentDecision(
            turn_id,
            approval_id,
            selora_ai::AgentDecision::Approve,
        ));

        // The command is running; Stop (naming no card, the status-line
        // button's nil id) aborts it.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let stop_started = std::time::Instant::now();
        let _ = effect_tx.send(Effect::AgentDecision(
            turn_id,
            selora_ai::ApprovalId::nil(),
            selora_ai::AgentDecision::Stop,
        ));
        let done = recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::AgentTurnDone(id, ..) if *id == turn_id),
        )
        .await;
        assert!(
            stop_started.elapsed() < Duration::from_secs(10),
            "Stop must end the turn promptly"
        );
        match done {
            AppMessage::AgentTurnDone(_, _, _, end) => {
                assert_eq!(end, selora_ai::TurnEnd::StoppedByUser);
            }
            other => panic!("{other:?}"),
        }

        let _ = std::fs::remove_dir_all(root);
    }

    /// No key / no workspace root: the turn never starts but its terminal
    /// message still arrives (the panel never hangs, PLAN.md §8).
    #[tokio::test]
    async fn agent_send_without_key_or_root_fails_visibly() {
        let server = MockServer::start().await;
        let root = unique_temp_path("agent-no-root");

        // Case 1: a key but no OpenFolder ever arrived (no root).
        let (effect_tx, mut msg_rx) = {
            let handle = Handle::current();
            let (msg_tx, msg_rx) = mpsc::unbounded_channel::<AppMessage>();
            let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
            spawn_dispatcher(
                &handle,
                noop_notify(),
                effect_rx,
                msg_tx,
                server.uri(),
                lsp_effect_tx(),
            );
            let _ = effect_tx.send(Effect::SeloraKeyringSet(TEST_KEY.into()));
            (effect_tx, msg_rx)
        };
        let turn_id = Uuid::new_v4();
        let _ = effect_tx.send(Effect::AgentSend(
            turn_id,
            agent_request(selora_settings::AgentSettings::default()),
        ));
        let done = recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::AgentTurnDone(id, ..) if *id == turn_id),
        )
        .await;
        match done {
            AppMessage::AgentTurnDone(_, _, summary, selora_ai::TurnEnd::Failed(reason)) => {
                assert!(reason.contains("no workspace folder"), "got: {reason}");
                assert!(!summary.completed);
                assert_eq!(summary.tools_used, 0);
            }
            other => panic!("expected a Failed AgentTurnDone, got {other:?}"),
        }

        // Case 2: a root but no key.
        std::fs::create_dir_all(&root).expect("mkdir");
        let (effect_tx, mut msg_rx) = {
            let handle = Handle::current();
            let (msg_tx, msg_rx) = mpsc::unbounded_channel::<AppMessage>();
            let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();
            spawn_dispatcher(
                &handle,
                noop_notify(),
                effect_rx,
                msg_tx,
                server.uri(),
                lsp_effect_tx(),
            );
            let _ = effect_tx.send(Effect::OpenFolder(root.clone()));
            (effect_tx, msg_rx)
        };
        let turn_id = Uuid::new_v4();
        let _ = effect_tx.send(Effect::AgentSend(
            turn_id,
            agent_request(selora_settings::AgentSettings::default()),
        ));
        let done = recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::AgentTurnDone(id, ..) if *id == turn_id),
        )
        .await;
        match done {
            AppMessage::AgentTurnDone(_, _, _, selora_ai::TurnEnd::Failed(reason)) => {
                assert!(reason.contains("no API key"), "got: {reason}");
            }
            other => panic!("expected a Failed AgentTurnDone, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(root);
    }

    /// The caps mapping, proven end-to-end: `agent.max_iterations = 1`
    /// aborts the turn at the second tool call with `IterationCap { 1 }`.
    #[tokio::test]
    async fn agent_turn_maps_settings_caps_into_the_loop() {
        let root = unique_temp_path("agent-caps");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("a.rs"), "fn a() {}\n").expect("write");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains("edit it"))
            .respond_with(ResponseTemplate::new(200).set_body_string(tool_call_sse(
                "call_read_1",
                "read_file",
                r#"{"path":"a.rs"}"#,
            )))
            .with_priority(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_string_contains("tool_call_id"))
            .respond_with(ResponseTemplate::new(200).set_body_string(tool_call_sse(
                "call_read_2",
                "read_file",
                r#"{"path":"a.rs"}"#,
            )))
            .with_priority(1)
            .mount(&server)
            .await;

        let (effect_tx, mut msg_rx) = agent_dispatcher(&server, &root).await;
        let turn_id = Uuid::new_v4();
        let settings = selora_settings::AgentSettings {
            max_iterations: 1,
            max_turn_secs: 12,
            max_cost_usd: 0.25,
            ..Default::default()
        };
        let _ = effect_tx.send(Effect::AgentSend(turn_id, agent_request(settings)));

        let done = recv_until(
            &mut msg_rx,
            |m| matches!(m, AppMessage::AgentTurnDone(id, ..) if *id == turn_id),
        )
        .await;
        match done {
            AppMessage::AgentTurnDone(_, _, _, end) => {
                assert_eq!(end, selora_ai::TurnEnd::IterationCap { limit: 1 });
            }
            other => panic!("{other:?}"),
        }

        let _ = std::fs::remove_dir_all(root);
    }

    /// The Brave key mapping (§17.1/§17.2): empty/whitespace is no key; a
    /// real key passes through trimmed.
    #[test]
    fn brave_key_mapping_trims_and_drops_empty() {
        assert_eq!(map_brave_key(None), None);
        assert_eq!(map_brave_key(Some("")), None);
        assert_eq!(map_brave_key(Some("   ")), None);
        assert_eq!(
            map_brave_key(Some("  brave-key  ")).as_deref(),
            Some("brave-key")
        );
    }
}
