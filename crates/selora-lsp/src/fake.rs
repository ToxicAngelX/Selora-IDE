//! An in-memory fake LSP server for headless tests (ARCHITECTURE.md §13).
//!
//! A duplex stream + a scripted task: it answers `initialize` with empty
//! capabilities, answers other requests with queued canned results (or
//! `null`), records everything the client sends, and lets the test push raw
//! notifications (diagnostics, progress) or kill the connection at will.
//! This is the stand-in for rust-analyzer in unit tests; the real-server
//! integration test lives in `tests/`.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::client::LaunchedServer;
use crate::framing::MessageReader;
use crate::jsonrpc;

/// What a test can tell the fake to do.
#[derive(Debug)]
pub enum FakeAction {
    /// Queue a canned `result` for the *next* request the client sends.
    Respond(Value),
    /// Push a raw message to the client (a server→client notification).
    Notify(Value),
    /// Drop the connection (the "server died" path).
    Die,
}

/// Control handle for one fake server instance.
#[derive(Clone)]
pub struct FakeHandle {
    tx: UnboundedSender<FakeAction>,
    /// The seen-messages receiver, shared with clones (the sender half
    /// stays with the fake task).
    seen_shared: Arc<Mutex<UnboundedReceiver<Value>>>,
}

impl FakeHandle {
    /// Queue a canned response for the next request.
    pub fn respond(&self, result: Value) {
        let _ = self.tx.send(FakeAction::Respond(result));
    }

    /// Push a server notification (already a complete JSON-RPC value).
    pub fn notify(&self, message: Value) {
        let _ = self.tx.send(FakeAction::Notify(message));
    }

    /// Drop the connection; the client should report the server dead.
    pub fn die(&self) {
        let _ = self.tx.send(FakeAction::Die);
    }

    /// The next message the client sent, if any (does not wait).
    pub fn try_next_seen(&self) -> Option<Value> {
        self.seen_shared.lock().expect("seen lock").try_recv().ok()
    }

    /// Wait (bounded) for the client to send its next message.
    pub async fn next_seen(&self) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(message) = self.try_next_seen() {
                    return message;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("timed out waiting for a client message")
    }
}

/// Launch a fake server; return the client-side transport to hand to
/// [`crate::client::LspClient::spawn_io`] plus the control handle.
pub fn launch_fake() -> (LaunchedServer, FakeHandle) {
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    let (client_reader, client_writer) = tokio::io::split(client_stream);
    let launched = LaunchedServer {
        reader: Box::new(client_reader),
        writer: Box::new(client_writer),
        child: None,
    };
    let (action_tx, action_rx) = mpsc::unbounded_channel();
    let (seen_tx, seen_rx) = mpsc::unbounded_channel();
    let handle = FakeHandle {
        tx: action_tx,
        seen_shared: Arc::new(Mutex::new(seen_rx)),
    };
    tokio::spawn(run_fake(server_stream, action_rx, seen_tx));
    (launched, handle)
}

async fn run_fake<R>(
    stream: R,
    mut actions: UnboundedReceiver<FakeAction>,
    seen: UnboundedSender<Value>,
) where
    R: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = MessageReader::new(reader);
    // Canned results, FIFO: the first queued result answers the next
    // request. Requests with nothing queued stay pending — tests answer
    // those by hand via `notify` (that's how out-of-order routing is
    // exercised).
    let mut canned: Vec<Value> = Vec::new();
    let mut alive = true;
    while alive {
        tokio::select! {
            message = reader.next() => {
                let message = match message {
                    Ok(Some(message)) => message,
                    Ok(None) => break, // client went away
                    Err(_) => break,
                };
                let _ = seen.send(message.clone());
                match jsonrpc::classify(message) {
                    // The handshake always succeeds.
                    Some(jsonrpc::Incoming::ServerRequest { id, method, .. }) if method == "initialize" => {
                        let response = jsonrpc::client_response(&id, json!({"capabilities": {}}));
                        write(&mut writer, &response).await;
                    }
                    Some(jsonrpc::Incoming::ServerRequest { id, .. }) => {
                        if let Some(result) = canned.first().cloned() {
                            canned.remove(0);
                            let response = jsonrpc::client_response(&id, result);
                            write(&mut writer, &response).await;
                        }
                    }
                    Some(jsonrpc::Incoming::Notification { method, .. }) if method == "exit" => break,
                    _ => {}
                }
            }
            action = actions.recv() => {
                match action {
                    Some(FakeAction::Respond(result)) => canned.push(result),
                    Some(FakeAction::Notify(message)) => write(&mut writer, &message).await,
                    Some(FakeAction::Die) | None => alive = false,
                }
            }
        }
    }
    // Dropping `writer`/`reader` closes the duplex: the client sees EOF.
}

async fn write<W: AsyncWrite + Unpin>(writer: &mut W, message: &Value) {
    let _ = crate::framing::write_message(writer, message).await;
}
