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

use selora_ai::{ChatEvent, SeloraClient};
use selora_core::{Buffer, SeloraError};
use selora_fs::tree::build_tree;
use selora_git::{open_repo, GitError};
use selora_settings::Settings;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

use crate::app_state::{AppMessage, Effect};

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
pub fn spawn_connect(
    handle: &Handle,
    notify: &Notify,
    tx: UnboundedSender<AppMessage>,
    base_url: String,
    api_key: String,
) {
    let notify = notify.clone();
    handle.spawn(async move {
        let client = SeloraClient::new(&base_url, &api_key);
        let result = client.list_models().await;
        let _ = tx.send(AppMessage::ModelsLoaded(result));
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
pub fn spawn_chat(
    handle: &Handle,
    notify: &Notify,
    tx: UnboundedSender<AppMessage>,
    base_url: String,
    api_key: String,
    chat_id: Uuid,
    request: selora_ai::ChatRequest,
) {
    let notify = notify.clone();
    let stream_handle = handle.clone();
    handle.spawn(async move {
        let client = SeloraClient::new(&base_url, &api_key);
        let (events_tx, mut events_rx) = mpsc::unbounded_channel::<ChatEvent>();

        let stream =
            stream_handle.spawn(async move { client.stream_chat(request, events_tx).await });

        let mut terminal_seen = false;
        while let Some(event) = events_rx.recv().await {
            terminal_seen |= matches!(event, ChatEvent::Done(_) | ChatEvent::Error(_));
            let message = match event {
                ChatEvent::Delta(text) => AppMessage::ChatDelta(chat_id, text),
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
        // event was already relayed; anything else is an error to surface.
        match stream.await {
            Ok(Ok(_receipt)) => {}
            Ok(Err(error)) => {
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
// The effect dispatcher (ARCHITECTURE.md §2.2: one consumer task)
// ---------------------------------------------------------------------------

/// Spawn the single `Effect` consumer task. It routes each effect to a
/// worker, tracks the in-memory Selora key, and follows the gateway base URL
/// from settings as they load/save. The task ends when the UI drops the
/// effect sender (window closed).
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
        while let Some(effect) = effect_rx.recv().await {
            match effect {
                Effect::OpenFile(path) => {
                    spawn_open_file(&dispatch_handle, &notify, &msg_tx, path);
                }
                Effect::OpenFolder(root) => {
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
                Effect::SeloraConnect(key) => {
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
                    );
                }
                Effect::SeloraListModels => match api_key.clone() {
                    Some(key) => {
                        spawn_connect(
                            &dispatch_handle,
                            &notify,
                            msg_tx.clone(),
                            base_url.clone(),
                            key,
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
                        spawn_chat(
                            &dispatch_handle,
                            &notify,
                            msg_tx.clone(),
                            base_url.clone(),
                            key,
                            chat_id,
                            request,
                        );
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
                Effect::SeloraKeyringSet(key) => {
                    // The OS keyring is Phase 4 (ARCHITECTURE.md §7); for now
                    // the key lives only in this dispatcher's memory.
                    api_key = Some(key);
                }
                Effect::GitStatus(root) => {
                    spawn_git_status(&dispatch_handle, &notify, &msg_tx, root);
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

    // --- the Phase 1c network half, against a mock gateway -------------------

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
        spawn_connect(&handle, &noop_notify(), tx, server.uri(), TEST_KEY.into());

        let message = rx.recv().await.expect("connect result message");
        match message {
            AppMessage::ModelsLoaded(Ok(models)) => assert_eq!(models.len(), 2),
            other => panic!("expected ModelsLoaded(Ok), got {other:?}"),
        }
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
}
