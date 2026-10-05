//! The LSP manager (the app-facing half of the client, ARCHITECTURE.md §2
//! message flow).
//!
//! One task owns every language session. The app's runtime dispatches
//! [`LspEffect`]s into it (the same role `wiring.rs`'s dispatcher plays for
//! the file/Selora effects — the orchestrator maps its own `Effect`
//! variants onto these; the verbatim proposal is in the crate docs); the
//! manager replies with [`LspEvent`]s on a single unbounded channel plus a
//! repaint-notify callback, exactly like the other workers.
//!
//! What the manager adds on top of raw [`LspClient`]s:
//! - **lifecycle**: spawns a server the first time a file of its language
//!   opens (resolving the command via [`LspRegistry`]), reports status for
//!   the status bar, restarts on request after a crash (§5.6) and replays
//!   every open document into the fresh server;
//! - **debounce**: `didChange` is held for [`DEBOUNCE`] after the last edit
//!   (deliverable 4: full text per commit, 250 ms) so a fast typist doesn't
//!   flood the server;
//! - **translation**: LSP wire types (UTF-16 positions, `Uri`s, `Hover`
//!   contents, …) become the plain app-side data in [`LspEvent`] — the app
//!   never needs a `lsp-types` dependency.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use lsp_types::{CompletionItem, DiagnosticSeverity, HoverContents, MarkedString};
use selora_core::{Position, SeloraError};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{sleep_until, Instant, Sleep};

use crate::client::{spawn_process, ClientEvent, LaunchedServer, LspClient, ServerStatus};
use crate::position;
use crate::registry::{LanguageId, LspRegistry, ServerSpec};
use crate::uri::{path_to_uri, uri_to_path};

/// `didChange` debounce (deliverable 4): a commit's full text is sent this
/// long after the last edit.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// How far in the future the idle debounce timer sleeps (a year — never
/// fires; re-armed whenever a change lands).
const SLEEP_NEVER: Duration = Duration::from_secs(365 * 24 * 3600);

/// The repaint seam, identical to `wiring.rs`'s `Notify`.
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// How a server is launched: the default spawns the real process; tests
/// inject an in-memory fake (`crate::fake`).
pub type Launcher = Arc<dyn Fn(&ServerSpec) -> Result<LaunchedServer, SeloraError> + Send + Sync>;

/// Requests the main thread sends *to* LSP (the app's `Effect` variants map
/// 1:1 onto these — see the crate docs for the verbatim proposal).
#[derive(Debug)]
pub enum LspEffect {
    /// A document was opened (first paint of a tab). Carries the full text.
    Open { path: PathBuf, text: String },
    /// The buffer changed (the full text of the new revision).
    Change { path: PathBuf, text: String },
    /// The buffer was saved (Ctrl+S).
    Save { path: PathBuf },
    /// The buffer's tab was closed.
    Close { path: PathBuf },
    /// Hover at a position (`Ctrl+K Ctrl+I` / the 600 ms hover hold).
    Hover { path: PathBuf, position: Position },
    /// Go to definition (F12).
    Definition { path: PathBuf, position: Position },
    /// Completions at a position (Ctrl+Space).
    Completion { path: PathBuf, position: Position },
    /// Restart a (dead) server — the status bar button (§5.6).
    Restart(LanguageId),
}

/// Messages LSP sends *to* the main thread (the app's `AppMessage` variants
/// map 1:1 onto these). Plain data only: no `lsp-types` types cross this
/// boundary.
#[derive(Debug, Clone)]
pub enum LspEvent {
    /// Server lifecycle for the status bar (and the death toast).
    ServerStatus {
        server: LanguageId,
        status: ServerStatus,
    },
    /// No command could be resolved for a language — the one-time toast
    /// "install it or set the path in settings" (ARCHITECTURE.md §5.3).
    ServerMissing { server: LanguageId },
    /// Fresh diagnostics for a document (the squiggle overlay's input).
    Diagnostics {
        path: PathBuf,
        diagnostics: Vec<LspDiagnostic>,
    },
    /// Hover resolved (content as a plain markdown text block; `None` = the
    /// server had nothing). The requested position rides along so the app
    /// can drop stale results (§5.5).
    Hover {
        path: PathBuf,
        position: Position,
        content: Option<String>,
    },
    /// Definition resolved (possibly into another file).
    Definition {
        path: PathBuf,
        position: Position,
        locations: Vec<LspLocation>,
    },
    /// Completions resolved for a position.
    Completions {
        path: PathBuf,
        position: Position,
        items: Vec<LspCompletionItem>,
    },
    /// A work-done progress line for the status bar center
    /// ("rust-analyzer: Indexing 42%").
    Progress { server: LanguageId, message: String },
}

/// An LSP (line, UTF-16 character) position — the wire coordinate. The
/// editor-side overlay converts it to code-point columns against the line
/// text it already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// One diagnostic, in app-side coordinates (UI_SPEC.md §3.6).
#[derive(Debug, Clone, PartialEq)]
pub struct LspDiagnostic {
    pub range: LspRange,
    pub severity: LspSeverity,
    pub source: Option<String>,
    pub message: String,
}

/// A definition target: a file plus a UTF-16 range in it.
#[derive(Debug, Clone, PartialEq)]
pub struct LspLocation {
    pub path: PathBuf,
    pub range: LspRange,
}

/// One completion entry (the popup's data).
#[derive(Debug, Clone, PartialEq)]
pub struct LspCompletionItem {
    pub label: String,
    /// Raw LSP `CompletionItemKind` number (the editor maps it to a tag).
    pub kind: Option<i32>,
    pub detail: Option<String>,
    /// The string the server wants sorting by, when it differs from the
    /// label.
    pub sort_text: Option<String>,
}

/// Manager tunables. Defaults match the spec; tests shrink the debounce and
/// inject the in-memory launcher.
#[derive(Clone)]
pub struct LspManagerConfig {
    pub registry: LspRegistry,
    /// The workspace root handed to servers at `initialize`.
    pub root: Option<PathBuf>,
    /// `didChange` hold-off (default [`DEBOUNCE`]).
    pub debounce: Duration,
    /// Server launcher (default: real processes).
    pub launcher: Launcher,
}

impl LspManagerConfig {
    pub fn new(registry: LspRegistry) -> Self {
        Self {
            registry,
            root: None,
            debounce: DEBOUNCE,
            launcher: Arc::new(spawn_process),
        }
    }

    /// The workspace root handed to servers at `initialize`.
    pub fn with_root(mut self, root: Option<PathBuf>) -> Self {
        self.root = root;
        self
    }
}

/// Spawn the manager task. It ends when the app drops the effect sender.
pub fn spawn_manager(
    handle: &Handle,
    notify: Notify,
    effect_rx: UnboundedReceiver<LspEffect>,
    event_tx: UnboundedSender<LspEvent>,
    config: LspManagerConfig,
) {
    handle.spawn(async move {
        Manager::new(notify, event_tx, config).run(effect_rx).await;
    });
}

/// One open document (for change debouncing and post-restart replay).
struct Doc {
    language: LanguageId,
    version: i32,
    /// The text as of the last `didOpen`/`didChange` sent.
    sent_text: String,
    /// The latest text (edits land here first).
    text: String,
}

struct Manager {
    notify: Notify,
    event_tx: UnboundedSender<LspEvent>,
    config: LspManagerConfig,
    docs: HashMap<PathBuf, Doc>,
    sessions: HashMap<LanguageId, LspClient>,
    /// Languages whose "server missing" toast was already sent (one-time).
    missing_notified: HashSet<LanguageId>,
    /// The single bus all client actors report on (tagged by language).
    client_rx: UnboundedReceiver<(LanguageId, ClientEvent)>,
    client_tx: UnboundedSender<(LanguageId, ClientEvent)>,
    /// Paths with a pending debounced `didChange`.
    pending_changes: HashSet<PathBuf>,
    /// The debounce timer; reset to `SLEEP_NEVER` while idle.
    debounce_sleep: Pin<Box<Sleep>>,
}

impl Manager {
    fn new(notify: Notify, event_tx: UnboundedSender<LspEvent>, config: LspManagerConfig) -> Self {
        let (client_tx, client_rx) = unbounded_channel();
        Self {
            notify,
            event_tx,
            config,
            docs: HashMap::new(),
            sessions: HashMap::new(),
            missing_notified: HashSet::new(),
            client_rx,
            client_tx,
            pending_changes: HashSet::new(),
            debounce_sleep: Box::pin(sleep_until(Instant::now() + SLEEP_NEVER)),
        }
    }

    async fn run(mut self, mut effect_rx: UnboundedReceiver<LspEffect>) {
        loop {
            tokio::select! {
                effect = effect_rx.recv() => {
                    match effect {
                        None => break, // app gone: sessions end with us
                        Some(effect) => self.handle_effect(effect).await,
                    }
                }
                event = self.client_rx.recv() => {
                    if let Some((language, event)) = event {
                        self.handle_client_event(language, event);
                    }
                }
                _ = &mut self.debounce_sleep => {
                    self.flush_changes().await;
                }
            }
        }
        // Graceful exit: every server gets shutdown + exit (best effort).
        for client in self.sessions.values() {
            client.shutdown();
        }
    }

    async fn handle_effect(&mut self, effect: LspEffect) {
        match effect {
            LspEffect::Open { path, text } => self.open_doc(path, text).await,
            LspEffect::Change { path, text } => self.change_doc(path, text),
            LspEffect::Save { path } => self.save_doc(&path).await,
            LspEffect::Close { path } => self.close_doc(&path).await,
            LspEffect::Hover { path, position } => self.hover(path, position).await,
            LspEffect::Definition { path, position } => self.definition(path, position).await,
            LspEffect::Completion { path, position } => self.completion(path, position).await,
            LspEffect::Restart(language) => self.restart(language).await,
        }
    }

    async fn open_doc(&mut self, path: PathBuf, text: String) {
        let Some(language) = LanguageId::from_path(&path) else {
            return; // no server for this language in Lite: nothing to track
        };
        self.ensure_session(language).await;
        let version = 0;
        if let Some(client) = self.sessions.get(&language) {
            let _ = client
                .did_open(&path_to_uri(&path), language.as_str(), &text, version)
                .await;
        }
        self.docs.insert(
            path,
            Doc {
                language,
                version,
                sent_text: text.clone(),
                text,
            },
        );
    }

    fn change_doc(&mut self, path: PathBuf, text: String) {
        let Some(doc) = self.docs.get_mut(&path) else {
            return; // never opened (or no language): nothing to sync
        };
        if doc.text != text {
            doc.text = text;
            self.pending_changes.insert(path);
            // Sliding window: every keystroke re-arms, so only the last
            // revision inside the debounce window is sent.
            self.debounce_sleep
                .as_mut()
                .reset(Instant::now() + self.config.debounce);
        }
    }

    async fn flush_changes(&mut self) {
        self.debounce_sleep
            .as_mut()
            .reset(Instant::now() + SLEEP_NEVER);
        for path in std::mem::take(&mut self.pending_changes) {
            let Some(doc) = self.docs.get_mut(&path) else {
                continue;
            };
            if doc.text == doc.sent_text {
                continue; // a save already flushed this revision
            }
            doc.version += 1;
            doc.sent_text = doc.text.clone();
            let Doc {
                language,
                version,
                sent_text,
                ..
            } = doc.clone_map();
            if let Some(client) = self.sessions.get(&language) {
                let _ = client
                    .did_change(&path_to_uri(&path), &sent_text, version)
                    .await;
            }
        }
    }

    async fn save_doc(&mut self, path: &Path) {
        // A save flushes any pending change first: the server's post-save
        // analysis must see the revision that hit the disk.
        if self.pending_changes.contains(path) {
            self.flush_changes().await;
        }
        let (language, text) = match self.docs.get(path) {
            Some(doc) => (doc.language, doc.sent_text.clone()),
            None => return,
        };
        if let Some(client) = self.sessions.get(&language) {
            let _ = client.did_save(&path_to_uri(path), &text).await;
        }
    }

    async fn close_doc(&mut self, path: &Path) {
        self.pending_changes.remove(path);
        let language = self.docs.remove(path).map(|doc| doc.language);
        if let Some(language) = language {
            if let Some(client) = self.sessions.get(&language) {
                let _ = client.did_close(&path_to_uri(path)).await;
            }
        }
    }

    async fn hover(&mut self, path: PathBuf, position: Position) {
        let Some(result) = self
            .request_position(&path, position, "textDocument/hover")
            .await
        else {
            return;
        };
        let content = result.and_then(|value| {
            serde_json::from_value::<lsp_types::Hover>(value)
                .ok()
                .and_then(shape_hover)
        });
        self.send(LspEvent::Hover {
            path,
            position,
            content,
        });
    }

    async fn definition(&mut self, path: PathBuf, position: Position) {
        let Some(result) = self
            .request_position(&path, position, "textDocument/definition")
            .await
        else {
            return;
        };
        let locations = result
            .and_then(|value| {
                serde_json::from_value::<lsp_types::GotoDefinitionResponse>(value).ok()
            })
            .map(definition_locations)
            .unwrap_or_default();
        self.send(LspEvent::Definition {
            path,
            position,
            locations,
        });
    }

    async fn completion(&mut self, path: PathBuf, position: Position) {
        let Some(result) = self
            .request_position(&path, position, "textDocument/completion")
            .await
        else {
            return;
        };
        let items = result
            .and_then(|value| serde_json::from_value::<lsp_types::CompletionResponse>(value).ok())
            .map(completion_items)
            .unwrap_or_default();
        self.send(LspEvent::Completions {
            path,
            position,
            items,
        });
    }

    /// The shared body of hover/definition/completion: find the session and
    /// document, convert the position, run the request. `None` = no
    /// session/document for the path (nothing to ask). A failed request
    /// yields `Some(None)`: stale/dead-server results are dropped silently
    /// (§5.5) — the status events carry the real errors.
    async fn request_position(
        &mut self,
        path: &Path,
        position: Position,
        method: &str,
    ) -> Option<Option<serde_json::Value>> {
        let language = self.docs.get(path)?.language;
        let client = self.sessions.get(&language)?.clone();
        let text = self.docs.get(path)?.text.clone();
        let lsp_position = position::to_lsp(&text, position);
        let params = serde_json::json!({
            "textDocument": { "uri": path_to_uri(path) },
            "position": { "line": lsp_position.line, "character": lsp_position.character },
        });
        Some(client.request(method, params).await.ok())
    }

    async fn restart(&mut self, language: LanguageId) {
        if let Some(client) = self.sessions.remove(&language) {
            client.shutdown();
        }
        self.ensure_session(language).await;
        if self.sessions.contains_key(&language) {
            self.replay_open_docs(language).await;
        }
    }

    /// Re-`didOpen` every document of a language into a fresh server
    /// (post-restart, §5.6). Versions restart at 0 like a first open.
    async fn replay_open_docs(&mut self, language: LanguageId) {
        let docs: Vec<(PathBuf, String)> = self
            .docs
            .iter()
            .filter(|(_, doc)| doc.language == language)
            .map(|(path, doc)| (path.clone(), doc.text.clone()))
            .collect();
        for (path, text) in docs {
            if let Some(doc) = self.docs.get_mut(&path) {
                doc.version = 0;
                doc.sent_text = text.clone();
            }
            if let Some(client) = self.sessions.get(&language) {
                let _ = client
                    .did_open(&path_to_uri(&path), language.as_str(), &text, 0)
                    .await;
            }
        }
    }

    /// Make sure a session exists for `language` (spawn on first need).
    /// Absent command → one-time [`LspEvent::ServerMissing`]; failed spawn
    /// → [`LspEvent::ServerStatus`] with the error.
    async fn ensure_session(&mut self, language: LanguageId) {
        if self.sessions.contains_key(&language) {
            return;
        }
        let Some(spec) = self.config.registry.resolve(language) else {
            if self.missing_notified.insert(language) {
                self.send(LspEvent::ServerMissing { server: language });
            }
            return;
        };
        let launched = match (self.config.launcher)(&spec) {
            Ok(launched) => launched,
            Err(error) => {
                self.send(LspEvent::ServerStatus {
                    server: language,
                    status: ServerStatus::Dead(format!("spawn failed: {error}")),
                });
                return;
            }
        };
        // The actor reports on the shared bus, tagged with its language.
        let (relay_tx, mut relay_rx) = unbounded_channel::<ClientEvent>();
        let bus_tx = self.client_tx.clone();
        tokio::spawn(async move {
            while let Some(event) = relay_rx.recv().await {
                let _ = bus_tx.send((language, event));
            }
        });
        let root = self.config.root.as_deref();
        let client = LspClient::spawn_io(spec.name, launched, root, relay_tx);
        self.sessions.insert(language, client);
    }

    fn handle_client_event(&mut self, language: LanguageId, event: ClientEvent) {
        match event {
            ClientEvent::Status(status) => {
                if let ServerStatus::Dead(_) = status {
                    // §5.6: the session is gone until the user restarts it.
                    self.sessions.remove(&language);
                }
                self.send(LspEvent::ServerStatus {
                    server: language,
                    status,
                });
            }
            ClientEvent::Diagnostics { uri, diagnostics } => {
                let Some(path) = uri_to_path(&uri) else {
                    return;
                };
                self.send(LspEvent::Diagnostics {
                    path,
                    diagnostics: diagnostics
                        .into_iter()
                        .map(|diagnostic| LspDiagnostic {
                            range: to_lsp_range(diagnostic.range),
                            severity: to_lsp_severity(diagnostic.severity),
                            source: diagnostic.source,
                            message: diagnostic.message,
                        })
                        .collect(),
                });
            }
            ClientEvent::Progress { message } => {
                self.send(LspEvent::Progress {
                    server: language,
                    message,
                });
            }
        }
    }

    fn send(&self, event: LspEvent) {
        if self.event_tx.send(event).is_ok() {
            (self.notify)();
        }
    }
}

impl Doc {
    /// The fields `flush_changes` needs, cloned out of the borrow.
    fn clone_map(&self) -> Doc {
        Doc {
            language: self.language,
            version: self.version,
            sent_text: self.sent_text.clone(),
            text: self.text.clone(),
        }
    }
}

fn to_lsp_range(range: lsp_types::Range) -> LspRange {
    LspRange {
        start: LspPosition {
            line: range.start.line,
            character: range.start.character,
        },
        end: LspPosition {
            line: range.end.line,
            character: range.end.character,
        },
    }
}

fn to_lsp_severity(severity: Option<DiagnosticSeverity>) -> LspSeverity {
    // An omitted severity is up to the client; Information is the honest
    // reading (not an error, not a warning).
    match severity {
        Some(severity) if severity == DiagnosticSeverity::ERROR => LspSeverity::Error,
        Some(severity) if severity == DiagnosticSeverity::WARNING => LspSeverity::Warning,
        Some(severity) if severity == DiagnosticSeverity::HINT => LspSeverity::Hint,
        _ => LspSeverity::Information,
    }
}

fn definition_locations(response: lsp_types::GotoDefinitionResponse) -> Vec<LspLocation> {
    let uri_ranges: Vec<(String, lsp_types::Range)> = match response {
        lsp_types::GotoDefinitionResponse::Scalar(location) => {
            vec![(location.uri.as_str().to_owned(), location.range)]
        }
        lsp_types::GotoDefinitionResponse::Array(locations) => locations
            .into_iter()
            .map(|location| (location.uri.as_str().to_owned(), location.range))
            .collect(),
        lsp_types::GotoDefinitionResponse::Link(links) => links
            .into_iter()
            .map(|link| (link.target_uri.as_str().to_owned(), link.target_range))
            .collect(),
    };
    uri_ranges
        .into_iter()
        .filter_map(|(uri, range)| {
            Some(LspLocation {
                path: uri_to_path(&uri)?,
                range: to_lsp_range(range),
            })
        })
        .collect()
}

fn completion_items(response: lsp_types::CompletionResponse) -> Vec<LspCompletionItem> {
    let items = match response {
        lsp_types::CompletionResponse::Array(items) => items,
        lsp_types::CompletionResponse::List(list) => list.items,
    };
    items
        .into_iter()
        .map(|item: CompletionItem| LspCompletionItem {
            label: item.label,
            // The kind is a newtype with a private field; re-read the wire
            // number (it serializes transparently) for the editor's tag
            // mapping.
            kind: item
                .kind
                .and_then(|kind| serde_json::to_value(kind).ok())
                .and_then(|value| value.as_i64())
                .map(|number| number as i32),
            detail: item.detail,
            sort_text: item.sort_text,
        })
        .collect()
}

/// Hover contents → one plain markdown block for the tooltip (the editor's
/// `shape_hover` then trims it for display).
fn shape_hover(hover: lsp_types::Hover) -> Option<String> {
    let parts: Vec<String> = match hover.contents {
        HoverContents::Scalar(MarkedString::String(text)) => vec![text],
        HoverContents::Scalar(MarkedString::LanguageString(code)) => {
            vec![format!("```{}\n{}\n```", code.language, code.value)]
        }
        HoverContents::Array(parts) => parts
            .into_iter()
            .map(|part| match part {
                MarkedString::String(text) => text,
                MarkedString::LanguageString(code) => {
                    format!("```{}\n{}\n```", code.language, code.value)
                }
            })
            .collect(),
        HoverContents::Markup(markup) => vec![markup.value],
    };
    let text = parts.join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{launch_fake, FakeHandle};
    use crate::manager::LspEffect::{
        Change, Close, Completion, Definition, Hover, Open, Restart, Save,
    };
    use crate::LspServerPaths;
    use serde_json::json;
    use std::sync::Mutex;

    /// A launcher that hands every session an in-memory fake and collects
    /// the control handles so the test can inspect / kill them.
    fn fake_launcher() -> (Launcher, Arc<Mutex<Vec<FakeHandle>>>) {
        let handles = Arc::new(Mutex::new(Vec::new()));
        let collected = handles.clone();
        let launcher: Launcher = Arc::new(move |_spec| {
            let (launched, handle) = launch_fake();
            collected.lock().expect("handles lock").push(handle);
            Ok(launched)
        });
        (launcher, handles)
    }

    /// A manager over fakes, with a short debounce for timing tests.
    /// Returns the effect sender, the event stream, and the fake handles.
    async fn manager_harness(
        debounce: Duration,
        with_server: bool,
    ) -> (
        UnboundedSender<LspEffect>,
        UnboundedReceiver<LspEvent>,
        Arc<Mutex<Vec<FakeHandle>>>,
    ) {
        let mut registry = LspRegistry::new(LspServerPaths::default()).with_search_dirs(vec![]);
        if with_server {
            registry.register(
                LanguageId::Rust,
                ServerSpec {
                    name: "fake-server".into(),
                    program: "unused".into(),
                    args: vec![],
                },
            );
        }
        let (launcher, handles) = fake_launcher();
        let config = LspManagerConfig {
            registry,
            root: None,
            debounce,
            launcher,
        };
        let (effect_tx, effect_rx) = unbounded_channel();
        let (event_tx, event_rx) = unbounded_channel();
        spawn_manager(
            &Handle::current(),
            Arc::new(|| {}),
            effect_rx,
            event_tx,
            config,
        );
        (effect_tx, event_rx, handles)
    }

    async fn next_event(events: &mut UnboundedReceiver<LspEvent>) -> LspEvent {
        tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timed out waiting for an Lsp event")
            .expect("event channel open")
    }

    /// Drain until an event matches (ignoring interleaved status/progress).
    async fn event_until(
        events: &mut UnboundedReceiver<LspEvent>,
        matches: impl Fn(&LspEvent) -> bool,
    ) -> LspEvent {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.expect("event channel open");
                if matches(&event) {
                    return event;
                }
            }
        })
        .await
        .expect("timed out waiting for a matching event")
    }

    /// An absolute sample path (uri round-trips need a real absolute path —
    /// a relative path makes a file:// uri that can't convert back).
    fn sample_path() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from("C:\\repo\\src\\a.rs")
        } else {
            PathBuf::from("/repo/src/a.rs")
        }
    }

    #[tokio::test]
    async fn open_spawns_the_session_and_sends_did_open() {
        let (effect_tx, mut events, handles) =
            manager_harness(Duration::from_millis(50), true).await;
        let path = sample_path();
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: "fn main() {}".into(),
        });

        // Status flows through: Starting then Ready.
        match next_event(&mut events).await {
            LspEvent::ServerStatus {
                server: LanguageId::Rust,
                status: ServerStatus::Starting,
            } => {}
            other => panic!("expected Starting, got {other:?}"),
        }
        match next_event(&mut events).await {
            LspEvent::ServerStatus {
                server: LanguageId::Rust,
                status: ServerStatus::Ready,
            } => {}
            other => panic!("expected Ready, got {other:?}"),
        }

        let fake = handles.lock().expect("handles lock")[0].clone();
        // initialize → initialized → didOpen, in that order.
        let initialize = fake.next_seen().await;
        assert_eq!(initialize["method"], "initialize");
        let _ = fake.next_seen().await; // initialized
        let did_open = fake.next_seen().await;
        assert_eq!(did_open["method"], "textDocument/didOpen");
        assert_eq!(did_open["params"]["textDocument"]["languageId"], "rust");
        assert_eq!(did_open["params"]["textDocument"]["text"], "fn main() {}");
        assert_eq!(did_open["params"]["textDocument"]["version"], 0);
        // The uri is a file:// uri for the path.
        assert!(did_open["params"]["textDocument"]["uri"]
            .as_str()
            .unwrap()
            .ends_with("a.rs"));
    }

    #[tokio::test]
    async fn did_change_is_debounced_and_coalesced() {
        // 80 ms debounce: fast enough for the test, slow enough to prove
        // the hold-off.
        let (effect_tx, mut events, handles) =
            manager_harness(Duration::from_millis(80), true).await;
        let path = sample_path();
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: "fn a()".into(),
        });
        let _ = event_until(&mut events, |event| {
            matches!(
                event,
                LspEvent::ServerStatus {
                    status: ServerStatus::Ready,
                    ..
                }
            )
        })
        .await;
        let fake = handles.lock().expect("handles lock")[0].clone();
        let _ = fake.next_seen().await; // initialize
        let _ = fake.next_seen().await; // initialized
        let _ = fake.next_seen().await; // didOpen

        // Three rapid changes: only the last revision may be sent, and only
        // after the debounce window.
        for text in ["fn a() {", "fn a() {}", "fn a() {} // done"] {
            let _ = effect_tx.send(Change {
                path: path.clone(),
                text: text.into(),
            });
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            fake.try_next_seen().is_none(),
            "didChange must not be sent inside the debounce window"
        );

        let did_change = fake.next_seen().await;
        assert_eq!(did_change["method"], "textDocument/didChange");
        assert_eq!(
            did_change["params"]["contentChanges"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            did_change["params"]["contentChanges"][0]["text"],
            "fn a() {} // done"
        );
        // Full-document sync: no range on the change, and the version bumped.
        assert!(did_change["params"]["contentChanges"][0]["range"].is_null());
        assert_eq!(did_change["params"]["textDocument"]["version"], 1);
    }

    #[tokio::test]
    async fn save_flushes_pending_changes_then_sends_did_save() {
        let (effect_tx, mut events, handles) = manager_harness(Duration::from_secs(5), true).await;
        let path = sample_path();
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: "fn a()".into(),
        });
        let _ = event_until(&mut events, |event| {
            matches!(
                event,
                LspEvent::ServerStatus {
                    status: ServerStatus::Ready,
                    ..
                }
            )
        })
        .await;
        let fake = handles.lock().expect("handles lock")[0].clone();
        // initialize → initialized → didOpen precede everything else.
        let _ = fake.next_seen().await;
        let _ = fake.next_seen().await;
        let _ = fake.next_seen().await;

        // An unsent change followed immediately by a save: the save must
        // flush it first (the server analyzes the saved revision).
        let _ = effect_tx.send(Change {
            path: path.clone(),
            text: "fn a() {}".into(),
        });
        let _ = effect_tx.send(Save { path: path.clone() });

        let mut methods = Vec::new();
        for _ in 0..2 {
            let message = fake.next_seen().await;
            methods.push(message["method"].as_str().expect("method").to_owned());
            if message["method"] == "textDocument/didChange" {
                assert_eq!(message["params"]["contentChanges"][0]["text"], "fn a() {}");
                assert_eq!(message["params"]["textDocument"]["version"], 1);
            }
            if message["method"] == "textDocument/didSave" {
                assert_eq!(message["params"]["text"], "fn a() {}");
            }
        }
        assert_eq!(
            methods,
            vec![
                "textDocument/didChange".to_owned(),
                "textDocument/didSave".to_owned()
            ]
        );
    }

    #[tokio::test]
    async fn hover_round_trips_with_utf16_positions() {
        let (effect_tx, mut events, handles) = manager_harness(Duration::from_secs(5), true).await;
        let path = sample_path();
        // 🦀 is 1 code point but 2 UTF-16 units: the position just after it
        // (code-point col 9) is UTF-16 character 10.
        let text = "fn a(x: 🦀) {}";
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: text.into(),
        });
        let _ = event_until(&mut events, |event| {
            matches!(
                event,
                LspEvent::ServerStatus {
                    status: ServerStatus::Ready,
                    ..
                }
            )
        })
        .await;
        let fake = handles.lock().expect("handles lock")[0].clone();
        let _ = fake.next_seen().await; // initialize
        let _ = fake.next_seen().await; // initialized
        let _ = fake.next_seen().await; // didOpen

        // The fake answers the hover request with a canned result.
        fake.respond(json!({
            "contents": { "kind": "markdown", "value": "```rust\nfn a(x: Crab)\n```" }
        }));
        let _ = effect_tx.send(Hover {
            path: path.clone(),
            position: Position::new(0, 9),
        });

        // The request the fake saw carries the UTF-16 position.
        let request = fake.next_seen().await;
        assert_eq!(request["method"], "textDocument/hover");
        assert_eq!(request["params"]["position"]["line"], 0);
        assert_eq!(request["params"]["position"]["character"], 10);

        match event_until(&mut events, |event| matches!(event, LspEvent::Hover { .. })).await {
            LspEvent::Hover {
                path: hover_path,
                position,
                content,
            } => {
                assert_eq!(hover_path, path);
                assert_eq!(position, Position::new(0, 9)); // echoed back
                assert_eq!(content.as_deref(), Some("```rust\nfn a(x: Crab)\n```"));
            }
            other => panic!("expected Hover, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn definition_and_completion_events_carry_plain_data() {
        let (effect_tx, mut events, handles) = manager_harness(Duration::from_secs(5), true).await;
        let path = sample_path();
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: "let x = foo();".into(),
        });
        let _ = event_until(&mut events, |event| {
            matches!(
                event,
                LspEvent::ServerStatus {
                    status: ServerStatus::Ready,
                    ..
                }
            )
        })
        .await;
        let fake = handles.lock().expect("handles lock")[0].clone();
        let _ = fake.next_seen().await;
        let _ = fake.next_seen().await;
        let _ = fake.next_seen().await;

        // Definition into another file (LocationLink shape).
        let foo_path = if cfg!(windows) {
            PathBuf::from("C:\\repo\\src\\foo.rs")
        } else {
            PathBuf::from("/repo/src/foo.rs")
        };
        fake.respond(json!([{
            "originSelectionRange": { "start": { "line": 0, "character": 8 }, "end": { "line": 0, "character": 11 } },
            "targetUri": path_to_uri(&foo_path),
            "targetRange": { "start": { "line": 4, "character": 0 }, "end": { "line": 6, "character": 1 } },
            "targetSelectionRange": { "start": { "line": 4, "character": 3 }, "end": { "line": 4, "character": 6 } }
        }]));
        let _ = effect_tx.send(Definition {
            path: path.clone(),
            position: Position::new(0, 9),
        });
        let _ = fake.next_seen().await; // the request
        match event_until(&mut events, |event| {
            matches!(event, LspEvent::Definition { .. })
        })
        .await
        {
            LspEvent::Definition { locations, .. } => {
                assert_eq!(locations.len(), 1);
                assert!(locations[0].path.ends_with("foo.rs"));
                assert_eq!(locations[0].range.start.line, 4);
                assert_eq!(locations[0].range.end.line, 6);
            }
            other => panic!("expected Definition, got {other:?}"),
        }

        // Completion (CompletionList shape).
        fake.respond(json!({
            "isIncomplete": false,
            "items": [
                { "label": "new()", "kind": 3, "detail": "fn() -> Vec<T>" },
                { "label": "with_capacity(capacity: usize)", "kind": 3 }
            ]
        }));
        let _ = effect_tx.send(Completion {
            path: path.clone(),
            position: Position::new(0, 14),
        });
        let _ = fake.next_seen().await; // the request
        match event_until(&mut events, |event| {
            matches!(event, LspEvent::Completions { .. })
        })
        .await
        {
            LspEvent::Completions { items, .. } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].label, "new()");
                assert_eq!(items[0].kind, Some(3));
                assert_eq!(items[0].detail.as_deref(), Some("fn() -> Vec<T>"));
                assert_eq!(items[1].detail, None);
            }
            other => panic!("expected Completions, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn diagnostics_are_translated_to_paths_and_plain_types() {
        let (effect_tx, mut events, handles) = manager_harness(Duration::from_secs(5), true).await;
        let path = sample_path();
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: "fn x(".into(),
        });
        let _ = event_until(&mut events, |event| {
            matches!(
                event,
                LspEvent::ServerStatus {
                    status: ServerStatus::Ready,
                    ..
                }
            )
        })
        .await;
        let fake = handles.lock().expect("handles lock")[0].clone();

        // The server publishes diagnostics keyed by uri; the event must be
        // keyed by path.
        let uri = path_to_uri(&path);
        fake.notify(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "diagnostics": [
                    {
                        "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } },
                        "severity": 1,
                        "source": "rustc",
                        "message": "unclosed delimiter"
                    },
                    {
                        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 2 } },
                        "message": "no severity given"
                    }
                ]
            }
        }));
        match event_until(&mut events, |event| {
            matches!(event, LspEvent::Diagnostics { .. })
        })
        .await
        {
            LspEvent::Diagnostics {
                path: diag_path,
                diagnostics,
            } => {
                assert_eq!(diag_path, path);
                assert_eq!(diagnostics.len(), 2);
                assert_eq!(diagnostics[0].severity, LspSeverity::Error);
                assert_eq!(diagnostics[0].source.as_deref(), Some("rustc"));
                assert_eq!(diagnostics[0].message, "unclosed delimiter");
                assert_eq!(diagnostics[0].range.start.line, 0);
                assert_eq!(diagnostics[0].range.start.character, 4);
                // Omitted severity reads as Information.
                assert_eq!(diagnostics[1].severity, LspSeverity::Information);
            }
            other => panic!("expected Diagnostics, got {other:?}"),
        }
    }

    /// §5.6 + deliverable: crash → dead event → restart → replay of open
    /// documents into the fresh server.
    #[tokio::test]
    async fn crash_recovery_restarts_and_replays_open_documents() {
        let (effect_tx, mut events, handles) = manager_harness(Duration::from_secs(5), true).await;
        let path = sample_path();
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: "fn main() {}".into(),
        });
        let _ = event_until(&mut events, |event| {
            matches!(
                event,
                LspEvent::ServerStatus {
                    status: ServerStatus::Ready,
                    ..
                }
            )
        })
        .await;

        // The server dies.
        handles.lock().expect("handles lock")[0].die();
        match event_until(&mut events, |event| {
            matches!(
                event,
                LspEvent::ServerStatus {
                    status: ServerStatus::Dead(_),
                    ..
                }
            )
        })
        .await
        {
            LspEvent::ServerStatus {
                server: LanguageId::Rust,
                status: ServerStatus::Dead(reason),
            } => {
                assert!(reason.contains("fake-server"), "reason: {reason}");
            }
            other => panic!("expected Dead, got {other:?}"),
        }

        // The user restarts from the status bar.
        let _ = effect_tx.send(Restart(LanguageId::Rust));
        let _ = event_until(&mut events, |event| {
            matches!(
                event,
                LspEvent::ServerStatus {
                    status: ServerStatus::Ready,
                    ..
                }
            )
        })
        .await;
        assert_eq!(
            handles.lock().expect("handles lock").len(),
            2,
            "a second server was launched"
        );

        // The fresh server gets the still-open document replayed.
        let fresh = handles.lock().expect("handles lock")[1].clone();
        let _ = fresh.next_seen().await; // initialize
        let _ = fresh.next_seen().await; // initialized
        let replay = fresh.next_seen().await;
        assert_eq!(replay["method"], "textDocument/didOpen");
        assert_eq!(replay["params"]["textDocument"]["text"], "fn main() {}");
        assert_eq!(replay["params"]["textDocument"]["version"], 0);
    }

    #[tokio::test]
    async fn missing_server_is_reported_once() {
        let (effect_tx, mut events, _handles) =
            manager_harness(Duration::from_millis(50), false).await;
        let path = sample_path();
        // First open with no resolvable command: the one-time toast event.
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: String::new(),
        });
        match next_event(&mut events).await {
            LspEvent::ServerMissing {
                server: LanguageId::Rust,
            } => {}
            other => panic!("expected ServerMissing, got {other:?}"),
        }
        // A second open does not repeat it (one-time, ARCHITECTURE.md §5.3).
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: String::new(),
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), events.recv())
                .await
                .is_err(),
            "no second ServerMissing event"
        );
    }

    #[tokio::test]
    async fn close_sends_did_close_and_stops_tracking() {
        let (effect_tx, mut events, handles) = manager_harness(Duration::from_secs(5), true).await;
        let path = sample_path();
        let _ = effect_tx.send(Open {
            path: path.clone(),
            text: "fn a() {}".into(),
        });
        let _ = event_until(&mut events, |event| {
            matches!(
                event,
                LspEvent::ServerStatus {
                    status: ServerStatus::Ready,
                    ..
                }
            )
        })
        .await;
        let fake = handles.lock().expect("handles lock")[0].clone();
        let _ = fake.next_seen().await; // initialize
        let _ = fake.next_seen().await; // initialized
        let _ = fake.next_seen().await; // didOpen

        let _ = effect_tx.send(Close { path: path.clone() });
        let did_close = fake.next_seen().await;
        assert_eq!(did_close["method"], "textDocument/didClose");

        // After close, changes for that path are ignored (no session churn,
        // no crash — the manager just has nothing to sync).
        let _ = effect_tx.send(Change {
            path: path.clone(),
            text: "fn b()".into(),
        });
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(fake.try_next_seen().is_none());
    }

    #[tokio::test]
    async fn requests_for_paths_without_a_session_are_silent() {
        let (effect_tx, mut events, _handles) =
            manager_harness(Duration::from_millis(50), false).await;
        // A hover for a path that was never opened (or has no server): no
        // event, no crash — nothing to show.
        let _ = effect_tx.send(Hover {
            path: sample_path(),
            position: Position::new(0, 0),
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), events.recv())
                .await
                .is_err(),
            "no event for an unopened path"
        );
    }

    #[test]
    fn severity_mapping() {
        use lsp_types::DiagnosticSeverity;
        assert_eq!(
            to_lsp_severity(Some(DiagnosticSeverity::ERROR)),
            LspSeverity::Error
        );
        assert_eq!(
            to_lsp_severity(Some(DiagnosticSeverity::WARNING)),
            LspSeverity::Warning
        );
        assert_eq!(
            to_lsp_severity(Some(DiagnosticSeverity::INFORMATION)),
            LspSeverity::Information
        );
        assert_eq!(
            to_lsp_severity(Some(DiagnosticSeverity::HINT)),
            LspSeverity::Hint
        );
        assert_eq!(to_lsp_severity(None), LspSeverity::Information);
    }

    #[test]
    fn hover_contents_shapes_to_markdown() {
        let hover = lsp_types::Hover {
            contents: HoverContents::Scalar(MarkedString::LanguageString(
                lsp_types::LanguageString {
                    language: "rust".into(),
                    value: "fn foo()".into(),
                },
            )),
            range: None,
        };
        assert_eq!(
            shape_hover(hover).as_deref(),
            Some("```rust\nfn foo()\n```")
        );

        let empty = lsp_types::Hover {
            contents: HoverContents::Scalar(MarkedString::String("  ".into())),
            range: None,
        };
        assert_eq!(shape_hover(empty), None);
    }
}
