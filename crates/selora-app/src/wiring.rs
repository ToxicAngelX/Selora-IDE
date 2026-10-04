//! Phase 1c wiring — the tokio side of the spike, UI-free.
//!
//! Everything here is deliberately free of egui so the message-drain logic
//! can be tested headlessly (and end-to-end against a wiremock mock gateway)
//! without opening a window. The contracts implemented:
//!
//! * ARCHITECTURE.md §1.4 — tasks are spawned onto the multi-threaded
//!   runtime built *before* the eframe loop; they never touch UI state.
//! * §2 — the Message-drain pattern: workers send [`AppMessage`]s on an
//!   unbounded channel; the main thread drains it at the top of each frame
//!   and reduces into [`SpikeState`].
//! * §6 — the two SeloraClient calls the spike needs (`list_models`,
//!   `stream_chat`).
//!
//! The two `spawn_*` functions are the Effect side (§2.2, spike subset). Each
//! takes a `notify` callback that is invoked after every message send; the UI
//! passes `ctx.request_repaint()` there — without it egui sleeps between
//! frames and streaming deltas would never appear on screen.

use selora_ai::{ChatEvent, ChatMessage, ChatReceipt, ChatRequest, Model, SeloraClient};
use selora_core::SeloraError;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, UnboundedSender};

/// Default gateway base URL (ARCHITECTURE.md §6.1). The spike panel allows
/// overriding it only so tests (and mock servers) can point elsewhere.
pub const DEFAULT_BASE_URL: &str = "https://api.selora.lol";

/// Messages workers send to the main thread (ARCHITECTURE.md §2.1, spike
/// subset: one connection, one in-flight chat stream).
#[derive(Debug)]
pub enum AppMessage {
    /// `list_models` resolved. `Ok` carries the catalog; the model count is
    /// the "connected (N models)" readout.
    ModelsLoaded(Result<Vec<Model>, SeloraError>),
    /// One streamed content delta.
    ChatDelta(String),
    /// The stream completed; carries the receipt for the cost readout.
    ChatDone(ChatReceipt),
    /// The stream failed.
    ChatError(SeloraError),
}

/// Gateway connection state for the spike panel.
#[derive(Debug, Default)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    /// Connected; carries the model count.
    Connected(usize),
    /// Failed; carries the gateway's error message verbatim (no paraphrase).
    Failed(String),
}

impl ConnectionState {
    /// The panel readout: "connected (N models)" / "error: <message>".
    pub fn label(&self) -> String {
        match self {
            Self::Disconnected => "disconnected".to_owned(),
            Self::Connecting => "connecting…".to_owned(),
            Self::Connected(count) => format!("connected ({count} models)"),
            Self::Failed(message) => format!("error: {message}"),
        }
    }
}

/// The main-thread state the message drain reduces into (ARCHITECTURE.md
/// §3.1, spike subset). Owned by the UI thread; workers never touch it.
#[derive(Debug, Default)]
pub struct SpikeState {
    pub connection: ConnectionState,
    pub models: Vec<Model>,
    /// Everything streamed so far, appended delta by delta.
    pub stream: String,
    /// Whether a chat stream is in flight.
    pub streaming: bool,
    /// The receipt of the last completed stream ("tokens: X / cost: $Y").
    pub receipt: Option<ChatReceipt>,
    /// The error of the last failed stream, if any.
    pub chat_error: Option<String>,
}

impl SpikeState {
    /// Apply one worker message — the entire message-drain reduction. Called
    /// at the top of every frame for every drained message.
    pub fn apply(&mut self, message: AppMessage) {
        match message {
            AppMessage::ModelsLoaded(Ok(models)) => {
                self.connection = ConnectionState::Connected(models.len());
                self.models = models;
            }
            AppMessage::ModelsLoaded(Err(error)) => {
                self.connection = ConnectionState::Failed(error.to_string());
            }
            AppMessage::ChatDelta(delta) => {
                self.stream.push_str(&delta);
            }
            AppMessage::ChatDone(receipt) => {
                self.streaming = false;
                self.receipt = Some(receipt);
            }
            AppMessage::ChatError(error) => {
                self.streaming = false;
                self.chat_error = Some(error.to_string());
            }
        }
    }

    /// A connect was dispatched; the readout shows "connecting…" until the
    /// `ModelsLoaded` message arrives.
    pub fn begin_connect(&mut self) {
        self.connection = ConnectionState::Connecting;
    }

    /// A chat was dispatched; clear the previous stream so the panel shows
    /// this one from its first delta.
    pub fn begin_chat(&mut self) {
        self.stream.clear();
        self.receipt = None;
        self.chat_error = None;
        self.streaming = true;
    }
}

/// Effect (§2.2): connect to the gateway and list its models.
///
/// The task builds its own [`SeloraClient`] from the given key; the key is
/// used for this request only and is never stored anywhere (no persistence
/// in the spike).
pub fn spawn_connect(
    handle: &Handle,
    notify: impl Fn() + Send + Sync + 'static,
    tx: UnboundedSender<AppMessage>,
    base_url: String,
    api_key: String,
) {
    handle.spawn(async move {
        let client = SeloraClient::new(&base_url, &api_key);
        let result = client.list_models().await;
        let _ = tx.send(AppMessage::ModelsLoaded(result));
        notify();
    });
}

/// Effect (§2.2): send `prompt` as the user message and stream the reply.
///
/// `selora-ai`'s `stream_chat` emits [`ChatEvent`]s on a channel *and*
/// returns a result, so this spawns two futures: the stream itself, and a
/// relay loop that reduces the events to [`AppMessage`]s (waking the UI per
/// message). The relay stops at the first terminal event (`Done`/`Error`);
/// the stream's return value is then joined only to surface an error that
/// never reached the channel — no silent failures (PLAN.md §8).
pub fn spawn_chat(
    handle: &Handle,
    notify: impl Fn() + Send + Sync + 'static,
    tx: UnboundedSender<AppMessage>,
    base_url: String,
    api_key: String,
    model: String,
    prompt: String,
) {
    let stream_handle = handle.clone();
    handle.spawn(async move {
        let client = SeloraClient::new(&base_url, &api_key);
        let request = ChatRequest::new(model, vec![ChatMessage::user(prompt)]).with_stream(true);
        let (events_tx, mut events_rx) = mpsc::unbounded_channel::<ChatEvent>();

        let stream =
            stream_handle.spawn(async move { client.stream_chat(request, events_tx).await });

        let mut terminal_seen = false;
        while let Some(event) = events_rx.recv().await {
            terminal_seen |= matches!(event, ChatEvent::Done(_) | ChatEvent::Error(_));
            let message = match event {
                ChatEvent::Delta(text) => AppMessage::ChatDelta(text),
                ChatEvent::Done(receipt) => AppMessage::ChatDone(receipt),
                ChatEvent::Error(error) => AppMessage::ChatError(error),
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
                    let _ = tx.send(AppMessage::ChatError(error));
                    notify();
                }
            }
            Err(join_error) => {
                if !terminal_seen {
                    let error = SeloraError::Other(format!("chat worker died: {join_error}"));
                    let _ = tx.send(AppMessage::ChatError(error));
                    notify();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Handle;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Headless stand-in for the UI's `ctx.request_repaint()` notify seam.
    fn noop() {}

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

    fn test_model() -> Model {
        Model {
            id: "test-model".into(),
            display_name: "Test Model".into(),
            context_window: 8192,
            price_per_1k_input: 0.001,
            price_per_1k_output: 0.002,
            supports_streaming: true,
            supports_tools: false,
            family: "test".into(),
        }
    }

    fn test_receipt() -> ChatReceipt {
        ChatReceipt {
            request_id: uuid::Uuid::new_v4(),
            model: "test-model".into(),
            prompt_tokens: 9,
            completion_tokens: 2,
            cost_usd: 0.0013,
            duration_ms: 42,
            estimated: false,
        }
    }

    #[test]
    fn drain_reduces_a_full_chat_lifecycle() {
        let mut state = SpikeState::default();
        assert_eq!(state.connection.label(), "disconnected");

        state.apply(AppMessage::ModelsLoaded(Ok(vec![test_model()])));
        assert_eq!(state.connection.label(), "connected (1 models)");
        assert_eq!(state.models.len(), 1);

        state.begin_chat();
        assert!(state.streaming);
        state.apply(AppMessage::ChatDelta("Hello".into()));
        state.apply(AppMessage::ChatDelta(" world".into()));
        assert_eq!(state.stream, "Hello world");

        state.apply(AppMessage::ChatDone(test_receipt()));
        assert!(!state.streaming);
        assert_eq!(state.receipt.as_ref().map(|r| r.prompt_tokens), Some(9));

        // A new chat clears the previous stream and receipt.
        state.begin_chat();
        assert!(state.streaming);
        assert_eq!(state.stream, "");
        assert!(state.receipt.is_none());
        assert!(state.chat_error.is_none());
    }

    #[test]
    fn connect_failure_is_surfaced_verbatim() {
        let mut state = SpikeState::default();
        state.apply(AppMessage::ModelsLoaded(Err(SeloraError::Net(
            "request failed: boom".into(),
        ))));
        assert_eq!(
            state.connection.label(),
            "error: network error: request failed: boom"
        );
    }

    #[test]
    fn chat_error_clears_streaming_and_sets_the_error() {
        let mut state = SpikeState::default();
        state.begin_chat();
        state.apply(AppMessage::ChatDelta("partial".into()));
        state.apply(AppMessage::ChatError(SeloraError::Auth(
            "key invalid".into(),
        )));
        assert!(!state.streaming);
        assert_eq!(state.stream, "partial");
        assert_eq!(state.chat_error.as_deref(), Some("auth error: key invalid"));
    }

    /// The full spike flow against a mock gateway, driven through the same
    /// spawn functions the UI buttons use, drained the same way the frame
    /// loop drains: connect → models → send → deltas → receipt.
    #[tokio::test]
    async fn end_to_end_connect_and_stream_against_a_mock_gateway() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(MODELS_JSON))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(STREAM_SSE))
            .expect(1)
            .mount(&server)
            .await;

        let handle = Handle::current();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = SpikeState::default();

        spawn_connect(&handle, noop, tx.clone(), server.uri(), TEST_KEY.into());
        let message = rx.recv().await.expect("connect result message");
        state.apply(message);
        assert_eq!(state.connection.label(), "connected (2 models)");
        let model = state.models[0].id.clone();

        state.begin_chat();
        assert!(state.streaming);
        spawn_chat(
            &handle,
            noop,
            tx.clone(),
            server.uri(),
            TEST_KEY.into(),
            model,
            "say hi".into(),
        );

        // Drain exactly like the frame loop: reduce messages in arrival
        // order until the stream's terminal message.
        loop {
            let message = rx.recv().await.expect("chat message");
            let terminal = matches!(message, AppMessage::ChatDone(_) | AppMessage::ChatError(_));
            state.apply(message);
            if terminal {
                break;
            }
        }

        assert_eq!(state.stream, "Hello world");
        assert!(!state.streaming);
        assert!(state.chat_error.is_none());
        let receipt = state.receipt.expect("receipt after Done");
        assert_eq!(receipt.prompt_tokens, 9);
        assert_eq!(receipt.completion_tokens, 2);
        assert_eq!(receipt.cost_usd, 0.0013);
        assert!(!receipt.estimated);
    }

    #[tokio::test]
    async fn invalid_key_surfaces_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .expect(1) // 401 is never retried
            .mount(&server)
            .await;

        let handle = Handle::current();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = SpikeState::default();

        spawn_connect(&handle, noop, tx, server.uri(), TEST_KEY.into());
        let message = rx.recv().await.expect("connect result message");
        state.apply(message);

        assert!(
            matches!(&state.connection, ConnectionState::Failed(message) if message.contains("401")),
            "expected a 401-bearing failure, got: {:?}",
            state.connection
        );
    }

    #[tokio::test]
    async fn stream_missing_done_reports_chat_error_after_deltas() {
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
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = SpikeState::default();

        state.begin_chat();
        spawn_chat(
            &handle,
            noop,
            tx,
            server.uri(),
            TEST_KEY.into(),
            "test-model".into(),
            "say hi".into(),
        );

        loop {
            let message = rx.recv().await.expect("chat message");
            let terminal = matches!(message, AppMessage::ChatDone(_) | AppMessage::ChatError(_));
            state.apply(message);
            if terminal {
                break;
            }
        }

        assert_eq!(state.stream, "Hi");
        assert!(!state.streaming);
        assert!(
            state
                .chat_error
                .as_deref()
                .is_some_and(|message| message.contains("closed before [DONE]")),
            "expected missing-[DONE] error, got: {:?}",
            state.chat_error
        );
    }
}
