//! The LSP client actor (ARCHITECTURE.md §5.1): one tokio task per server,
//! owning the child process, framing stdin/stdout, and routing responses by
//! request id.
//!
//! The actor is a `tokio::select!` loop over its inbox (commands from any
//! number of [`LspClient`] handles) and the server's stdout. Requests get
//! an id from a counter and a slot in a pending table; the matching
//! response resolves the caller's oneshot. Responses with an unknown id are
//! dropped silently (ARCHITECTURE.md §5.5 — stale requests). When the
//! server's stdout closes, the actor emits `ServerStatus::Dead`, fails
//! every pending request, and exits; restarting is the manager's job (§5.6).
//!
//! The handshake (`initialize` → `initialized`) runs before any command is
//! forwarded: commands sit in the unbounded inbox until the server reports
//! ready, so callers can use the client immediately after spawn. Until then
//! a deadline bounds the handshake — a server that never answers `initialize`
//! is declared dead instead of hanging the actor.
//!
//! Tests drive [`LspClient::spawn_io`] over an in-memory duplex (the fake
//! server in [`crate::fake`]) and real fast-exiting children for the crash
//! path — no tower-lsp, per PLAN.md §4.5.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use lsp_types::notification::Notification as _;
use lsp_types::request::{Completion, GotoDefinition, HoverRequest, Request as _};
use lsp_types::{
    CompletionItem, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    GotoDefinitionResponse, Hover, HoverParams, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams,
    VersionedTextDocumentIdentifier, WorkDoneProgressParams,
};
use selora_core::SeloraError;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::process::Child;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;

use crate::framing::{write_message, MessageReader};
use crate::jsonrpc::{self, Incoming, RpcId};
use crate::registry::ServerSpec;

/// How long the actor waits for the `initialize` response before declaring
/// the server dead.
pub const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(60);

/// Reserved request id of the `initialize` handshake.
const INITIALIZE_ID: u64 = 0;

/// Lifecycle of one server, as reported to the manager / status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    /// Spawned, awaiting the `initialize` response.
    Starting,
    /// Handshake complete; notifications and requests may flow.
    Ready,
    /// The process exited or the stream broke. The reason is the status
    /// bar / toast text.
    Dead(String),
}

/// Events one client actor emits (consumed by the manager, or by tests).
#[derive(Debug, Clone)]
pub enum ClientEvent {
    Status(ServerStatus),
    /// `textDocument/publishDiagnostics`, verbatim from the server.
    Diagnostics {
        uri: String,
        diagnostics: Vec<lsp_types::Diagnostic>,
    },
    /// A `$/progress` report shaped into status-bar text
    /// (`"rust-analyzer: Indexing 42%"`).
    Progress {
        message: String,
    },
}

/// The streams and process handle of one launched server. Tests construct
/// this by hand over an in-memory duplex; [`LspClient::spawn`] fills it from
/// a real child process.
pub struct LaunchedServer {
    pub reader: Box<dyn AsyncRead + Unpin + Send>,
    pub writer: Box<dyn AsyncWrite + Unpin + Send>,
    /// The child, when there is one; dropped (killed) when the actor exits.
    pub child: Option<Child>,
}

/// Commands the actor understands. The handle's methods are thin wrappers.
enum Command {
    Notify {
        method: String,
        params: Value,
    },
    Request {
        method: String,
        params: Value,
        reply: oneshot::Sender<Result<Value, SeloraError>>,
    },
    /// Graceful `shutdown` + `exit`, then make sure the child is gone.
    Shutdown,
}

/// A clonable handle to one server's actor (ARCHITECTURE.md §10.4: typed
/// methods, fire-and-forget notifications, requests that resolve later).
///
/// Every method fails with [`SeloraError::Lsp`] once the actor is gone — a
/// dead server is never a silent no-op (PLAN.md §8).
#[derive(Clone)]
pub struct LspClient {
    name: String,
    tx: mpsc::UnboundedSender<Command>,
}

impl LspClient {
    /// Spawn a real server process (the PLAN.md §5.3 table) and run its
    /// actor.
    ///
    /// The server's stderr is routed to `Stdio::null()`: piping it without a
    /// reader would let a chatty server block on a full pipe (the app-level
    /// log-file tap is the orchestrator's wiring, §7 of the persistence
    /// model).
    pub fn spawn(
        spec: &ServerSpec,
        root: Option<&Path>,
        events: UnboundedSender<ClientEvent>,
    ) -> Result<Self, SeloraError> {
        let launched = spawn_process(spec)?;
        Ok(Self::spawn_io(spec.name.clone(), launched, root, events))
    }

    /// Run the actor over already-connected streams (tests and in-memory
    /// servers). Same protocol, no process to kill.
    pub fn spawn_io(
        name: String,
        launched: LaunchedServer,
        root: Option<&Path>,
        events: UnboundedSender<ClientEvent>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let root_uri = root.map(crate::uri::path_to_uri);
        let actor = Actor {
            name: name.clone(),
            launched,
            events,
            pending: HashMap::new(),
            next_id: INITIALIZE_ID + 1,
            rx,
            root_uri,
        };
        tokio::spawn(actor.run());
        Self { name, tx }
    }

    /// The server's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Fire-and-forget notification (no response will arrive).
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), SeloraError> {
        self.tx
            .send(Command::Notify {
                method: method.to_owned(),
                params,
            })
            .map_err(|_| dead(&self.name))
    }

    /// A request: returns when the server answers, or with an error when the
    /// server dies (or answered with a JSON-RPC error).
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, SeloraError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(Command::Request {
                method: method.to_owned(),
                params,
                reply: reply_tx,
            })
            .map_err(|_| dead(&self.name))?;
        reply_rx.await.map_err(|_| dead(&self.name))?
    }

    /// Graceful shutdown (`shutdown` request, `exit` notification, kill).
    /// Fire-and-forget: the actor exits on its own.
    pub fn shutdown(&self) {
        let _ = self.tx.send(Command::Shutdown);
    }

    // ------------------------------------------------------------------
    // Typed surface (ARCHITECTURE.md §5.4, §10.4)
    // ------------------------------------------------------------------

    /// `textDocument/didOpen`.
    pub async fn did_open(
        &self,
        uri: &str,
        language_id: &str,
        text: &str,
        version: i32,
    ) -> Result<(), SeloraError> {
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: parse_uri(uri)?,
                language_id: language_id.to_owned(),
                version,
                text: text.to_owned(),
            },
        };
        self.notify("textDocument/didOpen", to_params(params)?)
            .await
    }

    /// `textDocument/didChange` — full-document sync (Lite sends the whole
    /// text; the server applies it as one range-less change).
    pub async fn did_change(&self, uri: &str, text: &str, version: i32) -> Result<(), SeloraError> {
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: parse_uri(uri)?,
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.to_owned(),
            }],
        };
        self.notify("textDocument/didChange", to_params(params)?)
            .await
    }

    /// `textDocument/didSave` (with the saved text — servers that don't ask
    /// for it ignore the field).
    pub async fn did_save(&self, uri: &str, text: &str) -> Result<(), SeloraError> {
        let params = DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: parse_uri(uri)?,
            },
            text: Some(text.to_owned()),
        };
        self.notify("textDocument/didSave", to_params(params)?)
            .await
    }

    /// `textDocument/didClose`.
    pub async fn did_close(&self, uri: &str) -> Result<(), SeloraError> {
        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: parse_uri(uri)?,
            },
        };
        self.notify("textDocument/didClose", to_params(params)?)
            .await
    }

    /// `textDocument/hover` — `None` when the server has nothing at that
    /// position (a legal null result).
    pub async fn hover(
        &self,
        uri: &str,
        position: lsp_types::Position,
    ) -> Result<Option<Hover>, SeloraError> {
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: parse_uri(uri)?,
                },
                position,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        let value = self
            .request(HoverRequest::METHOD, to_params(params)?)
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|error| SeloraError::Lsp(format!("malformed hover response: {error}")))
    }

    /// `textDocument/definition`, normalized to locations (links included).
    pub async fn definition(
        &self,
        uri: &str,
        position: lsp_types::Position,
    ) -> Result<Vec<lsp_types::Location>, SeloraError> {
        let params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: parse_uri(uri)?,
            },
            position,
        };
        let value = self
            .request(GotoDefinition::METHOD, to_params(params)?)
            .await?;
        if value.is_null() {
            return Ok(Vec::new());
        }
        let response: GotoDefinitionResponse = serde_json::from_value(value)
            .map_err(|error| SeloraError::Lsp(format!("malformed definition response: {error}")))?;
        Ok(match response {
            GotoDefinitionResponse::Scalar(location) => vec![location],
            GotoDefinitionResponse::Array(locations) => locations,
            GotoDefinitionResponse::Link(links) => links
                .into_iter()
                .map(|link| lsp_types::Location::new(link.target_uri, link.target_range))
                .collect(),
        })
    }

    /// `textDocument/completion` — both response shapes (bare array and
    /// `CompletionList`) normalized to items.
    pub async fn completion(
        &self,
        uri: &str,
        position: lsp_types::Position,
    ) -> Result<Vec<CompletionItem>, SeloraError> {
        let params = CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: parse_uri(uri)?,
                },
                position,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: Default::default(),
            context: None,
        };
        let value = self.request(Completion::METHOD, to_params(params)?).await?;
        if value.is_null() {
            return Ok(Vec::new());
        }
        let response: CompletionResponse = serde_json::from_value(value)
            .map_err(|error| SeloraError::Lsp(format!("malformed completion response: {error}")))?;
        Ok(match response {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        })
    }
}

fn dead(name: &str) -> SeloraError {
    SeloraError::Lsp(format!("{name} server is not running"))
}

/// Launch a real server process: piped stdin/stdout, stderr to null (a
/// chatty server would block on an unread pipe), no console window on
/// Windows. This is the default launcher for [`crate::manager`].
pub fn spawn_process(spec: &ServerSpec) -> Result<LaunchedServer, SeloraError> {
    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: never flash a console for a background server.
        // (tokio's own `creation_flags`, no std trait import needed.)
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(SeloraError::Io)?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| SeloraError::Lsp("server stdin was not piped".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SeloraError::Lsp("server stdout was not piped".into()))?;
    Ok(LaunchedServer {
        reader: Box::new(stdout),
        writer: Box::new(stdin),
        child: Some(child),
    })
}

fn parse_uri(uri: &str) -> Result<lsp_types::Uri, SeloraError> {
    uri.parse()
        .map_err(|error| SeloraError::Lsp(format!("invalid document uri {uri:?}: {error}")))
}

fn to_params<T: serde::Serialize>(params: T) -> Result<Value, SeloraError> {
    serde_json::to_value(params).map_err(|error| SeloraError::Lsp(format!("params: {error}")))
}

/// The actor task. One per server.
struct Actor {
    name: String,
    launched: LaunchedServer,
    events: UnboundedSender<ClientEvent>,
    /// Request id → the caller waiting for its response.
    pending: HashMap<u64, oneshot::Sender<Result<Value, SeloraError>>>,
    next_id: u64,
    rx: mpsc::UnboundedReceiver<Command>,
    root_uri: Option<String>,
}

impl Actor {
    async fn run(mut self) {
        // Take the streams out of `self` so the borrows in the loop below
        // don't fight with `&mut self` in the message handlers.
        let reader = std::mem::replace(&mut self.launched.reader, Box::new(NoopReader));
        let mut writer = std::mem::replace(&mut self.launched.writer, Box::new(NoopWriter));
        let mut reader = MessageReader::new(reader);
        let mut initialized = false;

        // Handshake first: commands wait in the inbox until Ready.
        let _ = self
            .events
            .send(ClientEvent::Status(ServerStatus::Starting));
        if let Err(error) = self.send_initialize(&mut writer).await {
            self.die(format!("initialize write failed: {error}")).await;
            return;
        }
        let mut handshake_deadline = Box::pin(tokio::time::sleep_until(
            tokio::time::Instant::now() + INITIALIZE_TIMEOUT,
        ));
        // Commands that arrived before the handshake completed. LSP requires
        // client→server traffic to start after `initialized`; the unbounded
        // inbox would otherwise let a `didOpen` race the handshake
        // (`select!` polls whichever branch is ready first).
        let mut deferred: Vec<Command> = Vec::new();

        loop {
            tokio::select! {
                biased;
                message = reader.next() => {
                    match message {
                        Err(error) => {
                            self.die(format!("read failed: {error}")).await;
                            break;
                        }
                        Ok(None) => {
                            // §5.6: stdout closed = the server process died.
                            self.die(format!("{} exited (stdout closed)", self.name)).await;
                            break;
                        }
                        Ok(Some(value)) => {
                            if !self.handle_message(&mut writer, value, &mut initialized).await {
                                break;
                            }
                            // The handshake completing releases the deferred
                            // commands, in arrival order.
                            if initialized && !deferred.is_empty() {
                                for command in deferred.drain(..) {
                                    if !self.apply_command(&mut writer, command).await {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
                command = self.rx.recv() => {
                    let command = match command {
                        None | Some(Command::Shutdown) => {
                            // The app dropped the client (window closed) or
                            // asked for shutdown: exit the server politely.
                            self.graceful_exit(&mut writer).await;
                            break;
                        }
                        Some(command) => command,
                    };
                    if initialized {
                        if !self.apply_command(&mut writer, command).await {
                            break;
                        }
                    } else {
                        deferred.push(command);
                    }
                }
                // A server that never answers initialize is dead, not stuck.
                _ = &mut handshake_deadline, if !initialized => {
                    self.die("initialize timed out".into()).await;
                    break;
                }
            }
        }
        // Nobody is left to answer these: fail loudly rather than hang.
        for (_, reply) in self.pending.drain() {
            let _ = reply.send(Err(dead(&self.name)));
        }
        for command in deferred {
            // A deferred request would otherwise hang on a dropped reply
            // channel; notifications need no reply.
            if let Command::Request { reply, .. } = command {
                let _ = reply.send(Err(dead(&self.name)));
            }
        }
    }

    /// Execute one inbox command. Returns `false` when the actor should stop
    /// (a fatal write).
    async fn apply_command(
        &mut self,
        writer: &mut Box<dyn AsyncWrite + Unpin + Send>,
        command: Command,
    ) -> bool {
        match command {
            Command::Notify { method, params } => {
                if let Err(error) =
                    write_message(writer, &jsonrpc::client_notification(&method, &params)).await
                {
                    self.die(format!("write failed: {error}")).await;
                    return false;
                }
            }
            Command::Request {
                method,
                params,
                reply,
            } => {
                let id = self.next_id;
                self.next_id += 1;
                self.pending.insert(id, reply);
                if let Err(error) =
                    write_message(writer, &jsonrpc::client_request(id, &method, &params)).await
                {
                    if let Some(reply) = self.pending.remove(&id) {
                        let _ = reply.send(Err(SeloraError::Lsp(format!("write failed: {error}"))));
                    }
                    self.die(format!("write failed: {error}")).await;
                    return false;
                }
            }
            // Handled by the caller (the loop breaks before we get here).
            Command::Shutdown => {}
        }
        true
    }

    /// Returns `false` when the actor should stop (a fatal write during
    /// message handling).
    async fn handle_message(
        &mut self,
        writer: &mut Box<dyn AsyncWrite + Unpin + Send>,
        value: Value,
        initialized: &mut bool,
    ) -> bool {
        match jsonrpc::classify(value) {
            Some(Incoming::Response { id, outcome }) => {
                if id == RpcId::Number(INITIALIZE_ID as i64) && !*initialized {
                    match outcome {
                        Ok(_) => {
                            *initialized = true;
                            let notified = write_message(
                                writer,
                                &jsonrpc::client_notification(
                                    lsp_types::notification::Initialized::METHOD,
                                    &json!({}),
                                ),
                            )
                            .await;
                            let _ = self.events.send(ClientEvent::Status(ServerStatus::Ready));
                            if let Err(error) = notified {
                                self.die(format!("write failed: {error}")).await;
                                return false;
                            }
                        }
                        Err(error) => {
                            self.die(format!("initialize rejected: {}", error.describe()))
                                .await;
                            return false;
                        }
                    }
                    return true;
                }
                // Route by request id; unknown ids are stale (§5.5), dropped
                // silently.
                let id = match id {
                    RpcId::Number(number) if number >= 0 => number as u64,
                    _ => return true, // a string id we never issued
                };
                if let Some(reply) = self.pending.remove(&id) {
                    let outcome = outcome.map_err(|error| {
                        SeloraError::Lsp(format!("server error: {}", error.describe()))
                    });
                    let _ = reply.send(outcome);
                }
            }
            Some(Incoming::ServerRequest { id, method, params }) => {
                // The Lite client answers every server request with a benign
                // result: workDoneProgress/create, registerCapability
                // (we ignore registrations), configuration (one null
                // section per requested item).
                let result = match method.as_str() {
                    "workspace/configuration" => Value::Array(
                        params
                            .get("items")
                            .and_then(Value::as_array)
                            .map(|items| vec![Value::Null; items.len()])
                            .unwrap_or_default(),
                    ),
                    _ => Value::Null,
                };
                if let Err(error) =
                    write_message(writer, &jsonrpc::client_response(&id, result)).await
                {
                    self.die(format!("write failed: {error}")).await;
                    return false;
                }
            }
            Some(Incoming::Notification { method, params }) => {
                self.handle_notification(method, params);
            }
            None => {} // not JSON-RPC shaped; tolerate and continue
        }
        true
    }

    fn handle_notification(&mut self, method: String, params: Value) {
        match method.as_str() {
            "textDocument/publishDiagnostics" => {
                if let Ok(params) =
                    serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params)
                {
                    let _ = self.events.send(ClientEvent::Diagnostics {
                        uri: params.uri.as_str().to_owned(),
                        diagnostics: params.diagnostics,
                    });
                }
            }
            "$/progress" => {
                if let Ok(params) = serde_json::from_value::<lsp_types::ProgressParams>(params) {
                    if let Some(message) = format_progress(&self.name, &params.value) {
                        let _ = self.events.send(ClientEvent::Progress { message });
                    }
                }
            }
            // window/logMessage, telemetry, $/cancelRequest, …: tolerated.
            _ => {}
        }
    }

    async fn send_initialize(
        &mut self,
        writer: &mut Box<dyn AsyncWrite + Unpin + Send>,
    ) -> Result<(), std::io::Error> {
        // Built as raw JSON (not `InitializeParams`) so we send exactly the
        // capability set Lite implements and nothing else.
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "selora-ide", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": self.root_uri,
            "capabilities": {
                "textDocument": {
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "completion": {
                        "completionItem": {
                            "snippetSupport": false,
                            "documentationFormat": ["markdown", "plaintext"]
                        }
                    }
                },
                "window": { "workDoneProgress": true }
            }
        });
        let message = jsonrpc::client_request(INITIALIZE_ID, "initialize", &params);
        write_message(writer, &message).await
    }

    async fn graceful_exit(&mut self, writer: &mut Box<dyn AsyncWrite + Unpin + Send>) {
        // Best-effort polite shutdown: request, notification, then make sure
        // the process is gone (the §5.6 restart path also lands here).
        let _ = write_message(
            writer,
            &jsonrpc::client_request(self.next_id, "shutdown", &Value::Null),
        )
        .await;
        let _ = write_message(
            writer,
            &jsonrpc::client_notification(lsp_types::notification::Exit::METHOD, &Value::Null),
        )
        .await;
        self.kill_child().await;
    }

    async fn die(&mut self, reason: String) {
        let _ = self
            .events
            .send(ClientEvent::Status(ServerStatus::Dead(reason)));
        self.kill_child().await;
    }

    async fn kill_child(&mut self) {
        if let Some(mut child) = self.launched.child.take() {
            // Give the server a moment to exit on its own after `exit`.
            if tokio::time::timeout(Duration::from_millis(500), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        }
    }
}

/// A stand-in stream pair used only while the actor owns the real ones (the
/// actor never reads/writes these — it exits before touching them again).
struct NoopWriter;

impl AsyncWrite for NoopWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::task::Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

struct NoopReader;

impl AsyncRead for NoopReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// Shape a `$/progress` report into status-bar text:
/// `"rust-analyzer: Indexing 42%"` (deliverable 7's exact format).
fn format_progress(server: &str, value: &lsp_types::ProgressParamsValue) -> Option<String> {
    let lsp_types::ProgressParamsValue::WorkDone(progress) = value;
    let (title, message, percentage) = match progress {
        lsp_types::WorkDoneProgress::Begin(begin) => (
            Some(begin.title.clone()),
            begin.message.clone(),
            begin.percentage,
        ),
        lsp_types::WorkDoneProgress::Report(report) => {
            (None, report.message.clone(), report.percentage)
        }
        lsp_types::WorkDoneProgress::End(end) => (None, end.message.clone(), None),
    };
    let base = title.or(message)?;
    Some(match percentage {
        Some(percentage) => format!("{server}: {base} {percentage}%"),
        None => format!("{server}: {base}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{launch_fake, FakeHandle};
    use serde_json::json;
    use tokio::sync::mpsc;

    /// A client over the in-memory fake, plus the fake's control handle and
    /// the actor's event stream.
    async fn spawn_with_fake() -> (LspClient, FakeHandle, mpsc::UnboundedReceiver<ClientEvent>) {
        let (launched, fake) = launch_fake();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let client = LspClient::spawn_io("fake-server".into(), launched, None, events_tx);
        (client, fake, events_rx)
    }

    /// Wait for the next client event (bounded — a broken test fails, it
    /// doesn't hang).
    async fn next_event(events: &mut mpsc::UnboundedReceiver<ClientEvent>) -> ClientEvent {
        tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timed out waiting for a client event")
            .expect("event channel open")
    }

    async fn await_ready(events: &mut mpsc::UnboundedReceiver<ClientEvent>) {
        match next_event(events).await {
            ClientEvent::Status(ServerStatus::Starting) => {}
            other => panic!("expected Starting, got {other:?}"),
        }
        match next_event(events).await {
            ClientEvent::Status(ServerStatus::Ready) => {}
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    fn rpc_response(id: serde_json::Value, result: serde_json::Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    }

    #[tokio::test]
    async fn initialize_handshake_completes_before_other_messages() {
        let (client, fake, mut events) = spawn_with_fake().await;
        await_ready(&mut events).await;

        // The very first thing the fake saw is the initialize request, with
        // our client info.
        let initialize = fake.next_seen().await;
        assert_eq!(initialize["method"], "initialize");
        assert!(initialize["params"]["capabilities"].is_object());
        assert_eq!(initialize["params"]["clientInfo"]["name"], "selora-ide");

        // The actor sent `initialized` right after the response.
        let initialized = fake.next_seen().await;
        assert_eq!(initialized["method"], "initialized");

        // A didOpen sent immediately after spawn still works — it was queued
        // behind the handshake.
        client
            .did_open("file:///a.rs", "rust", "fn main() {}", 0)
            .await
            .expect("did_open");
        let did_open = fake.next_seen().await;
        assert_eq!(did_open["method"], "textDocument/didOpen");
        assert_eq!(did_open["params"]["textDocument"]["uri"], "file:///a.rs");
        assert_eq!(did_open["params"]["textDocument"]["languageId"], "rust");
        assert_eq!(did_open["params"]["textDocument"]["text"], "fn main() {}");
    }

    /// The core routing contract (ARCHITECTURE.md §5.1): responses are
    /// matched by request id, not arrival order — here the fake answers the
    /// *second* request first.
    #[tokio::test]
    async fn responses_route_by_request_id_even_out_of_order() {
        let (client, fake, mut events) = spawn_with_fake().await;
        await_ready(&mut events).await;
        let _ = fake.next_seen().await; // initialize
        let _ = fake.next_seen().await; // initialized

        let client2 = client.clone();
        let first = tokio::spawn(async move { client2.request("first", json!({})).await });
        let client3 = client.clone();
        let second = tokio::spawn(async move { client3.request("second", json!({})).await });

        let id_first = fake.next_seen().await["id"].clone();
        let id_second = fake.next_seen().await["id"].clone();
        assert_ne!(id_first, id_second);

        // Answer in reverse order.
        fake.notify(rpc_response(id_second, json!("result-of-second")));
        fake.notify(rpc_response(id_first, json!("result-of-first")));

        assert_eq!(
            second.await.expect("join").expect("ok"),
            json!("result-of-second")
        );
        assert_eq!(
            first.await.expect("join").expect("ok"),
            json!("result-of-first")
        );
    }

    /// §5.5: a response for an id we never issued (or already consumed) is
    /// dropped silently, and the pending request still resolves when the
    /// right id arrives.
    #[tokio::test]
    async fn stale_responses_are_dropped_silently() {
        let (client, fake, mut events) = spawn_with_fake().await;
        await_ready(&mut events).await;
        let _ = fake.next_seen().await;
        let _ = fake.next_seen().await;

        let handle = tokio::spawn({
            let client = client.clone();
            async move { client.request("hover", json!({})).await }
        });
        let id = fake.next_seen().await["id"].clone();

        // A bogus id first, then the real one.
        fake.notify(rpc_response(json!(9999), json!("nobody-waiting-for-this")));
        fake.notify(rpc_response(id, json!("the-answer")));

        let result = handle.await.expect("join").expect("ok");
        assert_eq!(result, json!("the-answer"));
    }

    #[tokio::test]
    async fn server_notifications_reach_the_event_channel() {
        let (_client, fake, mut events) = spawn_with_fake().await;
        await_ready(&mut events).await;

        // publishDiagnostics → a Diagnostics event with the uri + payload.
        fake.notify(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///repo/src/a.rs",
                "diagnostics": [{
                    "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 5 } },
                    "severity": 1,
                    "message": "expected `(`"
                }]
            }
        }));
        match next_event(&mut events).await {
            ClientEvent::Diagnostics { uri, diagnostics } => {
                assert_eq!(uri, "file:///repo/src/a.rs");
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(diagnostics[0].message, "expected `(`");
                assert_eq!(diagnostics[0].range.start.character, 3);
            }
            other => panic!("expected diagnostics, got {other:?}"),
        }

        // $/progress → a formatted status-bar line.
        fake.notify(json!({
            "jsonrpc": "2.0",
            "method": "$/progress",
            "params": {
                "token": "progress-1",
                "value": { "kind": "begin", "title": "Indexing", "percentage": 42 }
            }
        }));
        match next_event(&mut events).await {
            ClientEvent::Progress { message } => {
                assert_eq!(message, "fake-server: Indexing 42%");
            }
            other => panic!("expected progress, got {other:?}"),
        }
    }

    /// Server→client requests (workDoneProgress/create and friends) get a
    /// benign response the server can proceed on.
    #[tokio::test]
    async fn server_requests_are_answered() {
        let (_client, fake, mut events) = spawn_with_fake().await;
        await_ready(&mut events).await;
        let _ = fake.next_seen().await; // initialize
        let _ = fake.next_seen().await; // initialized

        fake.notify(json!({
            "jsonrpc": "2.0",
            "id": 77,
            "method": "window/workDoneProgress/create",
            "params": { "token": "progress-1" }
        }));
        let response = fake.next_seen().await;
        assert_eq!(response["id"], json!(77));
        assert_eq!(response["result"], Value::Null);

        // workspace/configuration is answered with one null section per
        // requested item, not a bare null.
        fake.notify(json!({
            "jsonrpc": "2.0",
            "id": 78,
            "method": "workspace/configuration",
            "params": { "items": [{}, {}] }
        }));
        let response = fake.next_seen().await;
        assert_eq!(response["result"], json!([null, null]));
    }

    /// §5.6 crash path over the in-memory transport: the connection dies,
    /// the actor reports Dead, and a request in flight resolves with an
    /// error instead of hanging forever.
    #[tokio::test]
    async fn dead_connection_surfaces_status_and_fails_pending() {
        let (client, fake, mut events) = spawn_with_fake().await;
        await_ready(&mut events).await;

        let handle = tokio::spawn({
            let client = client.clone();
            async move { client.request("hover", json!({})).await }
        });
        let _ = fake.next_seen().await; // the request itself

        fake.die();
        match next_event(&mut events).await {
            ClientEvent::Status(ServerStatus::Dead(reason)) => {
                assert!(reason.contains("fake-server"), "reason: {reason}");
            }
            other => panic!("expected Dead, got {other:?}"),
        }
        let result = handle.await.expect("join");
        assert!(matches!(result, Err(SeloraError::Lsp(_))), "got {result:?}");

        // After death, new calls fail instead of silently no-op'ing.
        let error = client.notify("textDocument/didOpen", json!({})).await;
        assert!(error.is_err(), "calls on a dead client must fail");
    }

    /// The real-process crash path: a child that exits immediately closes
    /// its stdout, which the actor reports as Dead.
    #[tokio::test]
    async fn real_child_exit_surfaces_dead() {
        // `exit 0` exits immediately on both platforms' built-in shells.
        let spec = if cfg!(windows) {
            ServerSpec {
                name: "crasher".into(),
                program: "cmd".into(),
                args: vec!["/C".into(), "exit".into(), "0".into()],
            }
        } else {
            ServerSpec {
                name: "crasher".into(),
                program: "sh".into(),
                args: vec!["-c".into(), "exit 0".into()],
            }
        };
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let client = LspClient::spawn(&spec, None, events_tx).expect("spawn");
        assert_eq!(client.name(), "crasher");

        match next_event(&mut events).await {
            ClientEvent::Status(ServerStatus::Starting) => {}
            other => panic!("expected Starting, got {other:?}"),
        }
        match next_event(&mut events).await {
            ClientEvent::Status(ServerStatus::Dead(reason)) => {
                assert!(reason.contains("crasher"), "reason: {reason}");
            }
            other => panic!("expected Dead, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn graceful_shutdown_sends_the_lsp_sequence() {
        let (client, fake, mut events) = spawn_with_fake().await;
        await_ready(&mut events).await;
        let _ = fake.next_seen().await;
        let _ = fake.next_seen().await;

        client.shutdown();
        // The fake sees the `shutdown` request, then the `exit` notification,
        // then breaks its loop — closing the stream.
        let shutdown = fake.next_seen().await;
        assert_eq!(shutdown["method"], "shutdown");
        let exit = fake.next_seen().await;
        assert_eq!(exit["method"], "exit");
        // The actor exits without a Dead event (the app is closing too);
        // the observable contract is that the handle is dead afterwards.
        let error = client.notify("textDocument/didOpen", json!({})).await;
        assert!(error.is_err(), "calls after shutdown must fail");
    }

    #[test]
    fn progress_messages_match_the_status_bar_format() {
        use lsp_types::{WorkDoneProgress, WorkDoneProgressBegin, WorkDoneProgressReport};

        let begin = lsp_types::ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
            WorkDoneProgressBegin {
                title: "Indexing".into(),
                message: None,
                percentage: Some(42),
                ..Default::default()
            },
        ));
        assert_eq!(
            format_progress("rust-analyzer", &begin).as_deref(),
            Some("rust-analyzer: Indexing 42%")
        );

        let report = lsp_types::ProgressParamsValue::WorkDone(WorkDoneProgress::Report(
            WorkDoneProgressReport {
                message: Some("3/25 files".into()),
                percentage: Some(12),
                ..Default::default()
            },
        ));
        assert_eq!(
            format_progress("rust-analyzer", &report).as_deref(),
            Some("rust-analyzer: 3/25 files 12%")
        );

        // No title and no message: nothing to show (None, not a panic).
        let empty =
            lsp_types::ProgressParamsValue::WorkDone(WorkDoneProgress::End(Default::default()));
        assert_eq!(format_progress("rust-analyzer", &empty), None);
    }
}
