//! Phase Z4b — the LSP bridge: `selora-lsp`'s manager behind Tauri.
//!
//! [`LspBridge`] owns one `selora-lsp` manager task for the open workspace
//! and translates the crate's plain [`LspEvent`]s into the protocol's
//! [`FrontendEvent`]s. The desktop shell holds it in app state, forwards
//! editor effects ([`LspBridge::open`], [`LspBridge::change`], ...) and lets
//! the emitted events ride the same channel every other event rides.
//!
//! # Shape
//! - [`LspBridge::start_with_paths`] builds the registry from the
//!   `settings.lsp` path/argument/enabled overrides (falling back to PATH
//!   lookup), spawns the manager on Tauri's tokio runtime and a drain task
//!   that maps each event as it arrives. A machine without a language server
//!   is not an error: the manager itself reports [`LspEvent::ServerMissing`],
//!   which becomes the install toast.
//! - One bridge is bound to one workspace root. Switching or closing the
//!   folder drops it, shuts its servers down and invalidates any event still
//!   in flight, so stale diagnostics/hover/definition never apply to the new
//!   root.
//! - `selora-lsp` speaks in absolute path buffers; the protocol speaks in
//!   workspace-relative forward-slash [`WorkspacePath`]s. Every mapping
//!   helper here converts and *drops* anything outside the workspace root,
//!   so a server can never push a path the frontend would reject.
//! - Responses carry the [`BufferId`] the bridge was told owns the path
//!   ([`LspBridge::bind_buffer`]); a response for an unbound path (a tab
//!   that closed mid-flight) is dropped.
//! - Requests arrive in protocol coordinates (0-based line, UTF-16 column)
//!   while `selora-lsp` takes code-point columns. The bridge keeps the latest
//!   text per open document and converts against it, so a line containing an
//!   astral character (`🦀` is two UTF-16 units) does not shift the request.
//!
//! All shared state is behind `std::sync::Mutex`es held only for a single
//! map operation; a poisoned lock is recovered rather than panicking.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use selora_core::Position;
use selora_lsp::{
    spawn_manager, LanguageId, LspCompletionItem, LspDiagnostic, LspEffect, LspEvent, LspLocation,
    LspManagerConfig, LspRegistry, LspServerPaths, LspSeverity, Notify,
};
use selora_protocol::{
    BufferId, CompletionItemDto, CompletionResultDto, DefinitionLocationDto, DefinitionResultDto,
    DiagnosticDto, DiagnosticSeverity, FrontendEvent, HoverResultDto, LspServerHealth,
    LspServerStateDto, ProtocolErrorDto, TextPosition, ToastDto, ToastSeverity, WorkspacePath,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// The shell's event sink: the same `emit` closure the other relays call.
pub type Emit = Arc<dyn Fn(FrontendEvent) + Send + Sync>;

/// Completion candidates kept per response (the popup shows a window; a long
/// list is trimmed before it crosses the protocol).
const MAX_COMPLETIONS: usize = 200;

/// Definition targets kept per response.
const MAX_DEFINITIONS: usize = 50;

/// Path to owning buffer. Written by [`LspBridge::bind_buffer`], read when a
/// response resolves.
type BufferIndex = Mutex<HashMap<PathBuf, BufferId>>;

/// The latest diagnostics per absolute path, replaced wholesale on every
/// `textDocument/publishDiagnostics`.
type DiagnosticsMap = Mutex<BTreeMap<PathBuf, Vec<LspDiagnostic>>>;

/// The latest text per open document: the request-side UTF-16 to code-point
/// conversion needs the line the position lands on. Dropped on
/// [`LspBridge::close`].
type TextIndex = Mutex<HashMap<PathBuf, String>>;

/// The lifecycle of every server this bridge has heard from, keyed by
/// language so the bootstrap and restart replies are deterministic.
type ServerStates = Mutex<BTreeMap<LanguageId, LspServerStateDto>>;

/// The state shared with the event-drain task.
struct Shared {
    buffers: Arc<BufferIndex>,
    diagnostics: Arc<DiagnosticsMap>,
    states: Arc<ServerStates>,
    root: PathBuf,
    /// False once the bridge is shut down (workspace switch/close). Every
    /// emission checks this, so an event still in flight from the previous
    /// root can never be applied to the new workspace.
    active: Arc<AtomicBool>,
}

/// Strip Windows' verbatim prefix (`\\?\C:\...` → `C:\...`,
/// `\\?\UNC\server\share` → `\\server\share`). `std::fs::canonicalize` adds
/// it, but language servers speak the plain form: every path this bridge
/// stores, compares or sends through the manager is normalized so keys match
/// the URIs a server echoes back. Other platforms are unchanged.
fn plain_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.as_os_str().to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

/// The LSP manager plus the bookkeeping the protocol needs.
///
/// Construct one per open workspace with [`LspBridge::start_with_paths`];
/// dropping it closes the effect channel, which ends the manager task. A
/// bridge is bound to its root for life: switching or closing the workspace
/// drops this value and starts a fresh one, so no diagnostics or answer from
/// the old root can reach the new shell.
pub struct LspBridge {
    effects: UnboundedSender<LspEffect>,
    buffers: Arc<BufferIndex>,
    texts: Arc<TextIndex>,
    diagnostics: Arc<DiagnosticsMap>,
    states: Arc<ServerStates>,
    registry: LspRegistry,
    enabled: bool,
    root: PathBuf,
    emit: Emit,
    active: Arc<AtomicBool>,
}

impl LspBridge {
    /// Start with the settings-fed path/args/enabled overrides.
    pub fn start_with_paths(root: PathBuf, emit: Emit, paths: LspServerPaths) -> Self {
        let registry = LspRegistry::new(paths);
        Self::start_with_config(root, emit, LspManagerConfig::new(registry))
    }

    /// The injectable seam: tests pass a manager config with an in-memory
    /// launcher. Production goes through [`LspBridge::start_with_paths`].
    pub fn start_with_config(root: PathBuf, emit: Emit, config: LspManagerConfig) -> Self {
        // Normalize once: the manager, the servers and this bridge all key on
        // the plain path (`C:\...`), not the verbatim form canonicalize adds.
        let root = plain_path(&root);
        let registry = config.registry.clone();
        let enabled = registry.enabled();
        let config = config.with_root(Some(root.clone()));

        let (effect_tx, effect_rx) = unbounded_channel::<LspEffect>();
        let (event_tx, event_rx) = unbounded_channel::<LspEvent>();

        // No egui repaint to request: the frontend is pushed to directly.
        let notify: Notify = Arc::new(|| {});
        let runtime = tauri::async_runtime::handle();
        spawn_manager(runtime.inner(), notify, effect_rx, event_tx, config);

        let buffers: Arc<BufferIndex> = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics: Arc<DiagnosticsMap> = Arc::new(Mutex::new(BTreeMap::new()));
        let texts: Arc<TextIndex> = Arc::new(Mutex::new(HashMap::new()));
        let states: Arc<ServerStates> = Arc::new(Mutex::new(BTreeMap::new()));
        let active = Arc::new(AtomicBool::new(true));
        let shared = Shared {
            buffers: Arc::clone(&buffers),
            diagnostics: Arc::clone(&diagnostics),
            states: Arc::clone(&states),
            root: root.clone(),
            active: Arc::clone(&active),
        };
        tauri::async_runtime::spawn(drain_events(event_rx, shared, emit.clone()));

        Self {
            effects: effect_tx,
            buffers,
            texts,
            diagnostics,
            states,
            registry,
            enabled,
            root,
            emit,
            active,
        }
    }

    /// Stop accepting/emitting events for this root. Dropping the bridge
    /// calls this too, but the explicit call lets the shell retract state
    /// before the manager finishes shutting down.
    pub fn shutdown(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    /// The live diagnostics store as one flat protocol list. The bootstrap
    /// uses this instead of a hard-coded empty vector, so a Problems panel
    /// that re-bootstraps sees the same findings as the live event stream.
    pub fn diagnostics(&self) -> Vec<DiagnosticDto> {
        if !self.active.load(Ordering::SeqCst) {
            return Vec::new();
        }
        flatten_diagnostics(&self.root, &lock(&self.diagnostics))
    }

    /// Test seam: seed the live store exactly as a `publishDiagnostics`
    /// event would, without needing a server process.
    #[cfg(test)]
    pub(crate) fn inject_diagnostics(&self, path: PathBuf, diagnostics: Vec<LspDiagnostic>) {
        lock(&self.diagnostics).insert(plain_path(&path), diagnostics);
    }

    /// Every server whose lifecycle this bridge has observed, sorted by
    /// language. Empty before the first `Open` starts a server.
    pub fn server_states(&self) -> Vec<LspServerStateDto> {
        if !self.active.load(Ordering::SeqCst) {
            return Vec::new();
        }
        lock(&self.states).values().cloned().collect()
    }

    /// Restart one server for this workspace. The returned state is
    /// `starting`; `ready`/`failed` follow as `LspServerStateChanged`
    /// events. Unknown languages, a disabled settings switch, and an
    /// unresolvable command are typed errors rather than silent no-ops.
    pub fn restart(&self, language: &str) -> Result<LspServerStateDto, ProtocolErrorDto> {
        let language = parse_language(language).ok_or_else(|| ProtocolErrorDto {
            code: "lsp_unknown_language".into(),
            message: format!("Unknown language server `{language}`."),
            retryable: false,
        })?;
        if !self.enabled {
            let state = server_state(
                language,
                LspServerHealth::Failed,
                Some("Language servers are disabled in settings (lsp.enabled = false).".into()),
            );
            self.publish_state(state);
            return Err(ProtocolErrorDto {
                code: "lsp_disabled".into(),
                message: "Language servers are disabled in settings (lsp.enabled = false).".into(),
                retryable: false,
            });
        }
        if self.registry.resolve(language).is_none() {
            let message = format!(
                "{} is not configured; install it or set its path in settings.",
                language.server_name()
            );
            let state = server_state(language, LspServerHealth::Failed, Some(message.clone()));
            self.publish_state(state);
            return Err(ProtocolErrorDto {
                code: "lsp_server_not_found".into(),
                message,
                retryable: true,
            });
        }
        let state = server_state(language, LspServerHealth::Starting, None);
        self.publish_state(state.clone());
        self.send(LspEffect::Restart(language));
        Ok(state)
    }

    /// Remember which buffer owns a path (responses carry buffer ids).
    pub fn bind_buffer(&self, path: PathBuf, buffer_id: BufferId) {
        lock(&self.buffers).insert(plain_path(&path), buffer_id);
    }

    /// Forget a path's buffer (its tab closed).
    pub fn unbind_buffer(&self, path: &Path) {
        lock(&self.buffers).remove(&plain_path(path));
    }

    /// A document was opened: hand the server its full text.
    pub fn open(&self, path: PathBuf, text: String) {
        let path = plain_path(&path);
        lock(&self.texts).insert(path.clone(), text.clone());
        self.send(LspEffect::Open { path, text });
    }

    /// The buffer changed: hand the server the new revision's full text (the
    /// manager debounces before sending `didChange`).
    pub fn change(&self, path: PathBuf, text: String) {
        let path = plain_path(&path);
        lock(&self.texts).insert(path.clone(), text.clone());
        self.send(LspEffect::Change { path, text });
    }

    /// The buffer was saved (`Ctrl+S`).
    pub fn save(&self, path: PathBuf) {
        self.send(LspEffect::Save {
            path: plain_path(&path),
        });
    }

    /// P2 gap 3: the buffer was reloaded from disk. The server must receive
    /// the new full text (not a delta against the discarded local document)
    /// and the old text's diagnostics must be retracted now, so the Problems
    /// panel never shows squiggles for text that is no longer open.
    pub fn reload(&self, path: PathBuf, text: String) {
        let path = plain_path(&path);
        lock(&self.texts).insert(path.clone(), text.clone());
        self.send(LspEffect::Change {
            path: path.clone(),
            text,
        });
        self.invalidate_diagnostics(&path);
    }

    /// Drop one path's diagnostics and push the pruned list. Used when a file
    /// disappeared or was reloaded behind the open document: diagnostics from
    /// the old text are stale immediately.
    pub fn invalidate_diagnostics(&self, path: &Path) {
        let pruned = {
            let mut store = lock(&self.diagnostics);
            prune_path(&mut store, &self.root, path)
        };
        if let Some(diagnostics) = pruned {
            (self.emit)(FrontendEvent::DiagnosticsChanged(diagnostics));
        }
    }

    /// The buffer's tab closed.
    pub fn close(&self, path: PathBuf) {
        let path = plain_path(&path);
        lock(&self.texts).remove(&path);
        self.send(LspEffect::Close { path: path.clone() });
        // The manager emits no event when a document closes, so drop this
        // file's last published diagnostics here and push the pruned list.
        // Without this, a closed tab's squiggles linger in Problems until
        // the next publish or an app restart.
        let pruned = {
            let mut store = lock(&self.diagnostics);
            prune_path(&mut store, &self.root, &path)
        };
        if self.active.load(Ordering::SeqCst) {
            if let Some(diagnostics) = pruned {
                (self.emit)(FrontendEvent::DiagnosticsChanged(diagnostics));
            }
        }
    }

    /// Hover at a position (the 600 ms hold / `Ctrl+K Ctrl+I`).
    pub fn hover(&self, path: PathBuf, position: TextPosition) {
        let path = plain_path(&path);
        let position = self.core_position(&path, position);
        self.send(LspEffect::Hover { path, position });
    }

    /// Go to definition (`F12`).
    pub fn definition(&self, path: PathBuf, position: TextPosition) {
        let path = plain_path(&path);
        let position = self.core_position(&path, position);
        self.send(LspEffect::Definition { path, position });
    }

    /// Completions at a position (`Ctrl+Space`).
    pub fn completion(&self, path: PathBuf, position: TextPosition) {
        let path = plain_path(&path);
        let position = self.core_position(&path, position);
        self.send(LspEffect::Completion { path, position });
    }

    /// The request position in the manager's code-point coordinates.
    fn core_position(&self, path: &Path, position: TextPosition) -> Position {
        let texts = lock(&self.texts);
        to_core_position(texts.get(path).map(String::as_str), position)
    }

    /// Forward one effect, ignoring a manager that already ended.
    fn send(&self, effect: LspEffect) {
        if !self.active.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.effects.send(effect);
    }

    /// A bridge with a caller-owned effect channel: tests assert the exact
    /// `LspEffect` a reload sends without spawning a real language server.
    #[cfg(test)]
    pub(crate) fn start_for_test(
        root: PathBuf,
        emit: Emit,
    ) -> (Self, UnboundedReceiver<LspEffect>) {
        let root = plain_path(&root);
        let (effects, received) = unbounded_channel();
        (
            Self {
                effects,
                buffers: Arc::new(Mutex::new(HashMap::new())),
                texts: Arc::new(Mutex::new(HashMap::new())),
                diagnostics: Arc::new(Mutex::new(BTreeMap::new())),
                states: Arc::new(Mutex::new(BTreeMap::new())),
                registry: LspRegistry::new(LspServerPaths::default()),
                enabled: true,
                root,
                emit,
                active: Arc::new(AtomicBool::new(true)),
            },
            received,
        )
    }

    /// The text this bridge last handed the server for `path`.
    #[cfg(test)]
    pub(crate) fn text_for_test(&self, path: &Path) -> Option<String> {
        lock(&self.texts).get(&plain_path(path)).cloned()
    }

    /// Seed a stale diagnostic so a reload's retraction is observable.
    #[cfg(test)]
    pub(crate) fn seed_error_diagnostic_for_test(&self, path: &Path, message: &str) {
        let position = selora_lsp::LspPosition {
            line: 0,
            character: 0,
        };
        lock(&self.diagnostics).insert(
            plain_path(path),
            vec![LspDiagnostic {
                range: selora_lsp::LspRange {
                    start: position,
                    end: position,
                },
                severity: LspSeverity::Error,
                source: Some("test".into()),
                message: message.into(),
            }],
        );
    }

    /// Write one server state and push it to the shell, unless the bridge
    /// has already been shut down.
    fn publish_state(&self, state: LspServerStateDto) {
        if !self.active.load(Ordering::SeqCst) {
            return;
        }
        lock(&self.states).insert(
            parse_language(&state.language).expect("state language was parsed"),
            state.clone(),
        );
        (self.emit)(FrontendEvent::LspServerStateChanged(state));
    }
}

impl Drop for LspBridge {
    fn drop(&mut self) {
        // The effect sender is dropped right after this, which ends the
        // manager; setting `active` first makes a final in-flight event a
        // no-op for the new workspace.
        self.active.store(false, Ordering::SeqCst);
    }
}

/// Drain manager events for the bridge's lifetime.
async fn drain_events(mut events: UnboundedReceiver<LspEvent>, shared: Shared, emit: Emit) {
    while let Some(event) = events.recv().await {
        map_event(event, &shared, &emit);
    }
}

/// Translate one manager event into a protocol event (or nothing).
fn map_event(event: LspEvent, shared: &Shared, emit: &Emit) {
    // A bridge for a workspace that was just switched/closed must never leak
    // diagnostics or answers into the new root.
    if !shared.active.load(Ordering::SeqCst) {
        return;
    }
    match event {
        LspEvent::Diagnostics { path, diagnostics } => {
            let all = {
                let mut store = lock(&shared.diagnostics);
                store.insert(plain_path(&path), diagnostics);
                flatten_diagnostics(&shared.root, &store)
            };
            emit(FrontendEvent::DiagnosticsChanged(all));
        }
        LspEvent::Hover { path, content, .. } => {
            if let Some(buffer_id) = bound_buffer(&shared.buffers, &path) {
                emit(FrontendEvent::HoverResult(HoverResultDto {
                    buffer_id,
                    contents: content,
                    range: None,
                }));
            }
        }
        LspEvent::Definition {
            path, locations, ..
        } => {
            if let Some(buffer_id) = bound_buffer(&shared.buffers, &path) {
                emit(FrontendEvent::DefinitionResult(definition_dto(
                    &shared.root,
                    buffer_id,
                    &locations,
                )));
            }
        }
        LspEvent::Completions { path, items, .. } => {
            if let Some(buffer_id) = bound_buffer(&shared.buffers, &path) {
                emit(FrontendEvent::CompletionResult(completion_result_dto(
                    buffer_id, &items,
                )));
            }
        }
        LspEvent::ServerMissing { server } => {
            emit(FrontendEvent::Toast(ToastDto {
                severity: ToastSeverity::Warning,
                message: format!(
                    "Language server not found for {server:?} — install it or set its path in settings."
                ),
            }));
            publish_shared_state(
                shared,
                emit,
                server_state(
                    server,
                    LspServerHealth::Failed,
                    Some(format!(
                        "{} was not found; install it or set its path in settings.",
                        server.server_name()
                    )),
                ),
            );
        }
        LspEvent::ServerStatus { server, status } => {
            let (health, error) = match status {
                selora_lsp::ServerStatus::Starting => (LspServerHealth::Starting, None),
                selora_lsp::ServerStatus::Ready => (LspServerHealth::Ready, None),
                selora_lsp::ServerStatus::Dead(reason) => (LspServerHealth::Failed, Some(reason)),
            };
            publish_shared_state(shared, emit, server_state(server, health, error));
        }
        LspEvent::Progress { .. } => {}
    }
}

/// Record and emit one lifecycle state from the drain task.
fn publish_shared_state(shared: &Shared, emit: &Emit, state: LspServerStateDto) {
    lock(&shared.states).insert(
        parse_language(&state.language).expect("state language is known"),
        state.clone(),
    );
    emit(FrontendEvent::LspServerStateChanged(state));
}

/// The protocol state for one server lifecycle.
fn server_state(
    language: LanguageId,
    health: LspServerHealth,
    error: Option<String>,
) -> LspServerStateDto {
    LspServerStateDto {
        language: language.as_str().to_owned(),
        server: language.server_name().to_owned(),
        state: health,
        error,
    }
}

/// Accept the wire language id or the display server name.
fn parse_language(value: &str) -> Option<LanguageId> {
    match value.trim().to_ascii_lowercase().as_str() {
        "rust" | "rust-analyzer" | "rust_analyzer" => Some(LanguageId::Rust),
        "typescript" | "typescript-language-server" | "typescript_language_server" | "ts" => {
            Some(LanguageId::TypeScript)
        }
        "javascript" | "js" => Some(LanguageId::JavaScript),
        _ => None,
    }
}

/// The buffer bound to `path`, if its tab is still open.
fn bound_buffer(buffers: &BufferIndex, path: &Path) -> Option<BufferId> {
    lock(buffers).get(&plain_path(path)).cloned()
}

/// The workspace-relative forward-slash form of `path`, or `None` when it is
/// the root itself or lives outside it (such paths are never forwarded).
fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let root = plain_path(root);
    let path = plain_path(path);
    let relative = path.strip_prefix(&root).ok()?;
    let mut segments: Vec<&str> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(segment) => segments.push(segment.to_str()?),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

/// A protocol-valid workspace path, or `None` when the relative form would be
/// rejected by [`WorkspacePath`].
fn workspace_path(root: &Path, path: &Path) -> Option<WorkspacePath> {
    WorkspacePath::new(relative_path(root, path)?).ok()
}

/// LSP severity to protocol severity.
fn severity_to_dto(severity: LspSeverity) -> DiagnosticSeverity {
    match severity {
        LspSeverity::Error => DiagnosticSeverity::Error,
        LspSeverity::Warning => DiagnosticSeverity::Warning,
        LspSeverity::Information => DiagnosticSeverity::Info,
        LspSeverity::Hint => DiagnosticSeverity::Hint,
    }
}

/// Drop one path's diagnostics (its tab closed) and return the remaining
/// flat list, or None when that path had nothing stored — so close only
/// pushes an event when there is actually something to retract.
fn prune_path(
    by_path: &mut BTreeMap<PathBuf, Vec<LspDiagnostic>>,
    root: &Path,
    path: &Path,
) -> Option<Vec<DiagnosticDto>> {
    by_path
        .remove(&plain_path(path))
        .map(|_| flatten_diagnostics(root, by_path))
}

/// Every tracked path's diagnostics as one flat protocol list, dropping paths
/// outside the root (sorted by path for a stable payload).
fn flatten_diagnostics(
    root: &Path,
    by_path: &BTreeMap<PathBuf, Vec<LspDiagnostic>>,
) -> Vec<DiagnosticDto> {
    let mut out = Vec::new();
    for (path, diagnostics) in by_path {
        let Some(path) = workspace_path(root, path) else {
            continue;
        };
        for diagnostic in diagnostics {
            out.push(DiagnosticDto {
                path: path.clone(),
                line: diagnostic.range.start.line,
                column: diagnostic.range.start.character,
                severity: severity_to_dto(diagnostic.severity),
                message: diagnostic.message.clone(),
            });
        }
    }
    out
}

/// One completion item, trimmed to what the popup shows. `selora-lsp`'s item
/// carries no `insertText`, so the label is the insertion (the LSP default).
fn completion_dto(item: &LspCompletionItem) -> CompletionItemDto {
    CompletionItemDto {
        label: item.label.clone(),
        detail: item.detail.clone(),
        insert_text: Some(item.label.clone()),
        kind: item.kind.map(completion_kind).map(str::to_owned),
    }
}

/// A completions response, capped at [`MAX_COMPLETIONS`].
fn completion_result_dto(buffer_id: BufferId, items: &[LspCompletionItem]) -> CompletionResultDto {
    CompletionResultDto {
        buffer_id,
        items: items
            .iter()
            .take(MAX_COMPLETIONS)
            .map(completion_dto)
            .collect(),
    }
}

/// A definitions response: out-of-root targets dropped, capped at
/// [`MAX_DEFINITIONS`].
fn definition_dto(
    root: &Path,
    buffer_id: BufferId,
    locations: &[LspLocation],
) -> DefinitionResultDto {
    DefinitionResultDto {
        buffer_id,
        locations: locations
            .iter()
            .filter_map(|location| {
                Some(DefinitionLocationDto {
                    path: workspace_path(root, &location.path)?,
                    line: location.range.start.line,
                    utf16_column: location.range.start.character,
                })
            })
            .take(MAX_DEFINITIONS)
            .collect(),
    }
}

/// The editor's wire kind names for LSP's `CompletionItemKind` numbers (the
/// popup tags entries by name, not by number; unknown numbers read "other").
fn completion_kind(kind: i32) -> &'static str {
    match kind {
        1 => "text",
        2 => "method",
        3 => "function",
        4 => "constructor",
        5 => "field",
        6 => "variable",
        7 => "class",
        8 => "interface",
        9 => "module",
        10 => "property",
        11 => "unit",
        12 => "value",
        13 => "enum",
        14 => "keyword",
        15 => "snippet",
        16 => "color",
        17 => "file",
        18 => "reference",
        19 => "folder",
        20 => "enum_member",
        21 => "constant",
        22 => "struct",
        23 => "event",
        24 => "operator",
        25 => "type_parameter",
        _ => "other",
    }
}

/// Protocol position (0-based line, UTF-16 column) to the buffer position the
/// manager takes (`selora_core::Position.col` counts code points, and
/// `selora_lsp::position` turns those back into UTF-16 units against the same
/// text). `None` means the document was never opened here, so the units pass
/// through unchanged; out-of-range lines clamp to the last line's end, like
/// the manager clamps a stale server position.
fn to_core_position(text: Option<&str>, position: TextPosition) -> Position {
    let Some(text) = text else {
        return Position::new(position.line as usize, position.utf16_column as usize);
    };
    let line = position.line as usize;
    let line_count = text.split('\n').count();
    if line >= line_count {
        let last = line_count.saturating_sub(1);
        let end = line_at(text, last).map_or(0, |text| text.chars().count());
        return Position::new(last, end);
    }
    let line_text = line_at(text, line).unwrap_or_default();
    Position::new(line, utf16_col_to_char(line_text, position.utf16_column))
}

/// One line of `text`, with the `\r` of a CRLF terminator stripped.
fn line_at(text: &str, line: usize) -> Option<&str> {
    text.split('\n')
        .nth(line)
        .map(|text| text.strip_suffix('\r').unwrap_or(text))
}

/// UTF-16 unit offset to code-point column. A unit offset that splits a
/// surrogate pair resolves to the end of that character (the nearest valid
/// boundary, matching `selora_lsp::position`); an offset past the line's end
/// clamps to its end.
fn utf16_col_to_char(line: &str, units: u32) -> usize {
    let mut seen = 0u32;
    for (column, character) in line.chars().enumerate() {
        if seen >= units {
            return column;
        }
        seen += character.len_utf16() as u32;
    }
    line.chars().count()
}

/// Lock, recovering a poisoned mutex (a panicking task must not take the
/// bridge down with it).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use selora_lsp::{LspPosition, LspRange};

    fn repo() -> PathBuf {
        PathBuf::from("repo")
    }

    fn buffer() -> BufferId {
        BufferId::new("buffer-1").expect("test buffer id")
    }

    fn diagnostic(line: u32, character: u32, severity: LspSeverity) -> LspDiagnostic {
        let position = LspPosition { line, character };
        LspDiagnostic {
            range: LspRange {
                start: position,
                end: position,
            },
            severity,
            source: Some("rust-analyzer".into()),
            message: format!("message at {line}:{character}"),
        }
    }

    fn location(path: PathBuf, line: u32, character: u32) -> LspLocation {
        let position = LspPosition { line, character };
        LspLocation {
            path,
            range: LspRange {
                start: position,
                end: position,
            },
        }
    }

    #[test]
    fn relative_path_maps_inside_the_root() {
        let root = repo();
        let path = root.join("src").join("main.rs");
        assert_eq!(relative_path(&root, &path).as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn relative_path_rejects_outside_the_root_and_the_root_itself() {
        let root = repo();
        assert_eq!(relative_path(&root, Path::new("elsewhere/lib.rs")), None);
        assert_eq!(relative_path(&root, &root), None);
        // A sibling whose name shares the root's prefix is still outside.
        let sibling = Path::new("repository").join("src").join("main.rs");
        assert_eq!(relative_path(&root, &sibling), None);
    }

    #[test]
    fn canonicalized_root_matches_plain_server_paths() {
        // `Workspace::open` canonicalizes the root, which on Windows yields a
        // verbatim `\\?\C:\...` path. Servers answer with plain
        // `file:///C:/...` URIs, so the bridge must key everything plainly.
        let dir = tempfile::tempdir().expect("tempdir");
        let canonical = std::fs::canonicalize(dir.path()).expect("canonicalize");
        #[cfg(windows)]
        assert!(
            canonical.as_os_str().to_string_lossy().starts_with(r"\\?\"),
            "canonicalize yields a verbatim path on Windows"
        );

        let bridge = LspBridge::start_with_paths(
            canonical.clone(),
            Arc::new(|_| {}),
            LspServerPaths::default(),
        );
        assert_eq!(bridge.root, plain_path(&canonical), "root normalized once");
        #[cfg(windows)]
        assert!(!bridge
            .root
            .as_os_str()
            .to_string_lossy()
            .starts_with(r"\\?\"));

        let file = bridge.root.join("src").join("main.rs");
        // The shell binds by joining the workspace root (still verbatim) onto
        // the relative path; bind_buffer must strip the prefix.
        let shell_path = canonical.join("src").join("main.rs");
        let id = buffer();
        bridge.bind_buffer(shell_path, id.clone());

        // A server speaks `file:///C:/...`; decoding must land on `file`.
        let server_path =
            selora_lsp::uri_to_path(&selora_lsp::path_to_uri(&file)).expect("decodes");
        assert_eq!(server_path, file, "server URI decodes to the plain path");

        let emitted = Arc::new(Mutex::new(Vec::new()));
        let capture = Arc::clone(&emitted);
        let emit: Emit = Arc::new(move |event| lock(&capture).push(event));
        let shared = Shared {
            buffers: Arc::clone(&bridge.buffers),
            diagnostics: Arc::clone(&bridge.diagnostics),
            states: Arc::clone(&bridge.states),
            root: bridge.root.clone(),
            active: Arc::clone(&bridge.active),
        };

        map_event(
            LspEvent::Diagnostics {
                path: server_path.clone(),
                diagnostics: vec![diagnostic(2, 4, LspSeverity::Error)],
            },
            &shared,
            &emit,
        );
        map_event(
            LspEvent::Definition {
                path: server_path.clone(),
                position: Position::new(0, 0),
                locations: vec![location(server_path, 2, 4)],
            },
            &shared,
            &emit,
        );

        let events = lock(&emitted);
        match &events[0] {
            FrontendEvent::DiagnosticsChanged(diagnostics) => {
                assert_eq!(diagnostics.len(), 1, "diagnostic is not dropped");
                assert_eq!(diagnostics[0].path.as_str(), "src/main.rs");
            }
            other => panic!("expected diagnostics, got {other:?}"),
        }
        match &events[1] {
            FrontendEvent::DefinitionResult(result) => {
                assert_eq!(result.buffer_id, id, "bound buffer id is used");
                assert_eq!(result.locations.len(), 1, "definition is not dropped");
                assert_eq!(result.locations[0].path.as_str(), "src/main.rs");
            }
            other => panic!("expected definition, got {other:?}"),
        }
    }

    #[test]
    fn severity_maps_all_four() {
        assert_eq!(
            severity_to_dto(LspSeverity::Error),
            DiagnosticSeverity::Error
        );
        assert_eq!(
            severity_to_dto(LspSeverity::Warning),
            DiagnosticSeverity::Warning
        );
        assert_eq!(
            severity_to_dto(LspSeverity::Information),
            DiagnosticSeverity::Info
        );
        assert_eq!(severity_to_dto(LspSeverity::Hint), DiagnosticSeverity::Hint);
    }

    #[test]
    fn prune_path_retracts_a_closed_documents_diagnostics() {
        let root = repo();
        let opened = root.join("src").join("a.rs");
        let other = root.join("src").join("b.rs");
        let mut by_path = BTreeMap::new();
        by_path.insert(opened.clone(), vec![diagnostic(1, 4, LspSeverity::Error)]);
        by_path.insert(other.clone(), vec![diagnostic(7, 9, LspSeverity::Hint)]);

        let pruned = prune_path(&mut by_path, &root, &opened).expect("was stored");
        assert_eq!(pruned.len(), 1, "closed file's diagnostic is gone");
        assert_eq!(pruned[0].path.as_str(), "src/b.rs");
        assert!(!by_path.contains_key(&opened), "store no longer tracks it");
        assert!(by_path.contains_key(&other), "other paths survive");

        assert!(
            prune_path(&mut by_path, &root, &opened).is_none(),
            "a second close emits nothing"
        );
    }

    #[test]
    fn reload_sends_the_full_text_and_retracts_stale_diagnostics() {
        let root = repo();
        let path = root.join("src").join("main.rs");
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let capture = Arc::clone(&emitted);
        let emit: Emit = Arc::new(move |event| lock(&capture).push(event));
        let (bridge, mut effects) = LspBridge::start_for_test(root.clone(), emit);

        bridge.open(path.clone(), "fn main() { broken() }\n".into());
        let opened = effects.try_recv().expect("open effect");
        assert!(matches!(opened, LspEffect::Open { .. }));
        bridge.seed_error_diagnostic_for_test(&path, "cannot find function `broken`");
        assert!(bridge.text_for_test(&path).is_some());

        bridge.reload(path.clone(), "fn main() {}\n".into());

        // The reload's didChange carries the complete new document, so the
        // manager never patches its old (discarded) text.
        let effect = effects.try_recv().expect("reload effect");
        match effect {
            LspEffect::Change { path: sent, text } => {
                assert_eq!(sent, plain_path(&path));
                assert_eq!(text, "fn main() {}\n");
            }
            other => panic!("expected a full-text change, got {other:?}"),
        }
        assert_eq!(
            bridge.text_for_test(&path).as_deref(),
            Some("fn main() {}\n"),
            "later position requests convert against the reloaded text"
        );

        // The old text's diagnostic is retracted in the same call, before any
        // later publish can repaint squiggles for it.
        let events = lock(&emitted);
        assert!(
            events.iter().any(|event| matches!(
                event,
                FrontendEvent::DiagnosticsChanged(diagnostics) if diagnostics.is_empty()
            )),
            "stale diagnostics are retracted: {events:#?}"
        );
    }

    #[test]
    fn invalidate_diagnostics_drops_only_the_deleted_path() {
        let root = repo();
        let deleted = root.join("src").join("a.rs");
        let kept = root.join("src").join("b.rs");
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let capture = Arc::clone(&emitted);
        let emit: Emit = Arc::new(move |event| lock(&capture).push(event));
        let (bridge, _effects) = LspBridge::start_for_test(root, emit);

        bridge.seed_error_diagnostic_for_test(&deleted, "gone");
        bridge.seed_error_diagnostic_for_test(&kept, "still here");
        bridge.invalidate_diagnostics(&deleted);

        let events = lock(&emitted);
        match events.last() {
            Some(FrontendEvent::DiagnosticsChanged(diagnostics)) => {
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(diagnostics[0].path.as_str(), "src/b.rs");
            }
            other => panic!("expected pruned diagnostics, got {other:?}"),
        }
        drop(events);
        // A path with nothing stored emits nothing: reopen cannot resurrect it.
        let before = lock(&emitted).len();
        bridge.invalidate_diagnostics(&deleted);
        assert_eq!(lock(&emitted).len(), before);
    }

    #[test]
    fn flatten_diagnostics_merges_two_paths_and_drops_outside() {
        let root = repo();
        let mut by_path = BTreeMap::new();
        by_path.insert(
            root.join("src").join("a.rs"),
            vec![
                diagnostic(1, 4, LspSeverity::Error),
                diagnostic(2, 0, LspSeverity::Warning),
            ],
        );
        by_path.insert(
            root.join("src").join("b.rs"),
            vec![diagnostic(7, 9, LspSeverity::Hint)],
        );
        by_path.insert(
            Path::new("outside").join("c.rs"),
            vec![diagnostic(0, 0, LspSeverity::Information)],
        );

        let flat = flatten_diagnostics(&root, &by_path);
        assert_eq!(flat.len(), 3, "out-of-root entry dropped");
        assert_eq!(flat[0].path.as_str(), "src/a.rs");
        assert_eq!((flat[0].line, flat[0].column), (1, 4));
        assert_eq!(flat[0].severity, DiagnosticSeverity::Error);
        assert_eq!(flat[1].severity, DiagnosticSeverity::Warning);
        assert_eq!(flat[2].path.as_str(), "src/b.rs");
        assert_eq!((flat[2].line, flat[2].column), (7, 9));
        assert_eq!(flat[2].severity, DiagnosticSeverity::Hint);
    }

    #[test]
    fn completion_caps_at_200_and_passes_detail_through() {
        let items: Vec<LspCompletionItem> = (0..250)
            .map(|index| LspCompletionItem {
                label: format!("item{index}"),
                kind: Some(3),
                detail: Some(format!("detail {index}")),
                sort_text: None,
            })
            .collect();

        let result = completion_result_dto(buffer(), &items);
        assert_eq!(result.items.len(), 200);
        assert_eq!(result.items[0].label, "item0");
        assert_eq!(result.items[0].detail.as_deref(), Some("detail 0"));
        assert_eq!(result.items[0].insert_text.as_deref(), Some("item0"));
        assert_eq!(result.items[0].kind.as_deref(), Some("function"));
        assert_eq!(result.items[199].label, "item199");

        let bare = LspCompletionItem {
            label: "keyword".into(),
            kind: None,
            detail: None,
            sort_text: None,
        };
        let result = completion_result_dto(buffer(), &[bare]);
        assert_eq!(result.items[0].detail, None);
        assert_eq!(result.items[0].kind, None);
    }

    #[test]
    fn definition_drops_out_of_root_targets() {
        let root = repo();
        let locations = vec![
            location(root.join("src").join("main.rs"), 3, 8),
            location(Path::new("vendor").join("dep.rs"), 0, 0),
            location(root.join("src").join("lib.rs"), 12, 1),
        ];

        let result = definition_dto(&root, buffer(), &locations);
        assert_eq!(result.locations.len(), 2);
        assert_eq!(result.locations[0].path.as_str(), "src/main.rs");
        assert_eq!(
            (result.locations[0].line, result.locations[0].utf16_column),
            (3, 8)
        );
        assert_eq!(result.locations[1].path.as_str(), "src/lib.rs");
    }

    #[test]
    fn definition_caps_at_50() {
        let root = repo();
        let locations: Vec<LspLocation> = (0..80)
            .map(|index| location(root.join("src").join(format!("f{index}.rs")), index, 0))
            .collect();

        let result = definition_dto(&root, buffer(), &locations);
        assert_eq!(result.locations.len(), 50);
    }

    #[test]
    fn position_converts_utf16_units_to_code_points() {
        let text = "fn main() {\n    let crab = \"🦀\";\n}\n";
        let prefix = "    let crab = \"";
        // The emoji is two UTF-16 units but one code point.
        let after_emoji = TextPosition {
            line: 1,
            utf16_column: prefix.len() as u32 + 2,
        };
        assert_eq!(
            to_core_position(Some(text), after_emoji),
            Position::new(1, prefix.chars().count() + 1)
        );
        // A unit offset inside the surrogate pair resolves to its end.
        let inside_pair = TextPosition {
            line: 1,
            utf16_column: prefix.len() as u32 + 1,
        };
        assert_eq!(
            to_core_position(Some(text), inside_pair),
            Position::new(1, prefix.chars().count() + 1)
        );
        // ASCII lines are unchanged by the conversion.
        assert_eq!(
            to_core_position(
                Some(text),
                TextPosition {
                    line: 0,
                    utf16_column: 3
                }
            ),
            Position::new(0, 3)
        );
    }

    #[test]
    fn position_falls_back_without_text_and_clamps_past_the_end() {
        assert_eq!(
            to_core_position(
                None,
                TextPosition {
                    line: 2,
                    utf16_column: 7
                }
            ),
            Position::new(2, 7)
        );
        let text = "a\r\n🦀b";
        assert_eq!(
            to_core_position(
                Some(text),
                TextPosition {
                    line: 1,
                    utf16_column: 2
                }
            ),
            Position::new(1, 1)
        );
        assert_eq!(
            to_core_position(
                Some(text),
                TextPosition {
                    line: 9,
                    utf16_column: 0
                }
            ),
            Position::new(1, 2)
        );
    }

    // -- P1 lifecycle ------------------------------------------------------

    fn capture() -> (Emit, Arc<Mutex<Vec<FrontendEvent>>>) {
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&emitted);
        let emit: Emit = Arc::new(move |event| lock(&sink).push(event));
        (emit, emitted)
    }

    fn quiet_config(paths: LspServerPaths) -> LspManagerConfig {
        // Pin the PATH lookup to an empty list so the test never depends on
        // the machine's installed servers.
        LspManagerConfig::new(LspRegistry::new(paths).with_search_dirs(vec![]))
    }

    #[test]
    fn stale_root_events_are_suppressed_after_shutdown() {
        let root = repo();
        let (emit, emitted) = capture();
        let shared = Shared {
            buffers: Arc::new(Mutex::new(HashMap::new())),
            diagnostics: Arc::new(Mutex::new(BTreeMap::new())),
            states: Arc::new(Mutex::new(BTreeMap::new())),
            root: root.clone(),
            active: Arc::new(AtomicBool::new(true)),
        };

        map_event(
            LspEvent::Diagnostics {
                path: root.join("src").join("a.rs"),
                diagnostics: vec![diagnostic(1, 0, LspSeverity::Error)],
            },
            &shared,
            &emit,
        );
        assert_eq!(lock(&emitted).len(), 1, "active bridge emits");

        shared.active.store(false, Ordering::SeqCst);
        map_event(
            LspEvent::Diagnostics {
                path: root.join("src").join("a.rs"),
                diagnostics: vec![diagnostic(2, 0, LspSeverity::Error)],
            },
            &shared,
            &emit,
        );
        map_event(
            LspEvent::Hover {
                path: root.join("src").join("a.rs"),
                position: Position::new(0, 0),
                content: Some("stale".into()),
            },
            &shared,
            &emit,
        );
        assert_eq!(
            lock(&emitted).len(),
            1,
            "a shut-down bridge drops both diagnostics and hover answers"
        );
    }

    #[test]
    fn restart_is_typed_for_unknown_disabled_and_missing_servers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (emit, _) = capture();

        let disabled = LspBridge::start_with_config(
            dir.path().to_path_buf(),
            Arc::clone(&emit),
            quiet_config(LspServerPaths {
                enabled: false,
                ..LspServerPaths::default()
            }),
        );
        let unknown = disabled.restart("cobol").expect_err("unknown language");
        assert_eq!(unknown.code, "lsp_unknown_language");
        let off = disabled.restart("rust").expect_err("disabled");
        assert_eq!(off.code, "lsp_disabled");
        let states = disabled.server_states();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].state, LspServerHealth::Failed);
        assert!(states[0]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("disabled"));

        let missing = LspBridge::start_with_config(
            dir.path().to_path_buf(),
            emit,
            quiet_config(LspServerPaths::default()),
        );
        let error = missing.restart("rust").expect_err("not on PATH");
        assert_eq!(error.code, "lsp_server_not_found");
        assert!(error.retryable);
    }

    #[test]
    fn restart_reports_starting_then_failed_when_the_launcher_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (emit, emitted) = capture();
        let mut registry = LspRegistry::new(LspServerPaths {
            rust_analyzer: Some("C:/missing/rust-analyzer.exe".into()),
            ..LspServerPaths::default()
        })
        .with_search_dirs(vec![]);
        registry.register(
            LanguageId::Rust,
            selora_lsp::ServerSpec {
                name: "rust-analyzer".into(),
                program: "C:/missing/rust-analyzer.exe".into(),
                args: vec![],
            },
        );
        let mut config = LspManagerConfig::new(registry);
        let launcher: selora_lsp::Launcher =
            Arc::new(|_spec| Err(selora_core::SeloraError::Lsp("boom".into())));
        config.launcher = launcher;
        let bridge = LspBridge::start_with_config(dir.path().to_path_buf(), emit, config);

        let state = bridge.restart("rust-analyzer").expect("restart accepted");
        assert_eq!(state.state, LspServerHealth::Starting);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let states = bridge.server_states();
            if states
                .iter()
                .any(|state| state.state == LspServerHealth::Failed)
            {
                let failed = states
                    .iter()
                    .find(|state| state.state == LspServerHealth::Failed)
                    .expect("failed state");
                assert!(failed.error.as_deref().unwrap_or("").contains("boom"));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "launcher failure did not surface: {states:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            lock(&emitted).iter().any(|event| matches!(
                event,
                FrontendEvent::LspServerStateChanged(state)
                    if state.state == LspServerHealth::Failed
            )),
            "the failed state was emitted to the shell"
        );
    }

    #[test]
    fn diagnostics_snapshot_reads_the_live_store_and_drops_out_of_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (emit, _) = capture();
        let bridge = LspBridge::start_with_config(
            dir.path().to_path_buf(),
            emit,
            quiet_config(LspServerPaths::default()),
        );
        let inside = bridge.root.join("src").join("main.rs");
        {
            let mut store = lock(&bridge.diagnostics);
            store.insert(inside.clone(), vec![diagnostic(3, 2, LspSeverity::Warning)]);
            store.insert(
                PathBuf::from("outside").join("other.rs"),
                vec![diagnostic(0, 0, LspSeverity::Error)],
            );
        }
        let snapshot = bridge.diagnostics();
        assert_eq!(snapshot.len(), 1, "out-of-root diagnostics are dropped");
        assert_eq!(snapshot[0].path.as_str(), "src/main.rs");
        assert_eq!(snapshot[0].severity, DiagnosticSeverity::Warning);

        bridge.shutdown();
        assert!(
            bridge.diagnostics().is_empty(),
            "a closed bridge reports nothing"
        );
        assert!(bridge.server_states().is_empty());
    }
}
