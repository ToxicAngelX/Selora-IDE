//! `SeloraClient` — the HTTP client for the Selora gateway (ARCHITECTURE.md §6).

use std::fmt;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use selora_core::SeloraError;
use serde::Deserialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::chat_types::ToolCall;
use crate::types::{ChatEvent, ChatMessage, ChatReceipt, ChatRequest, Model, SeloraStatus};

/// 60s to establish the connection (ARCHITECTURE.md §6.1).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// 5min read timeout: SSE bodies stream for a long time, but any single read
/// gap longer than this is a dead stream. Deliberately *not* an overall
/// request timeout, so long streams stay alive while data keeps flowing.
const READ_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The SSE sentinel that terminates a chat stream.
const DONE_SENTINEL: &str = "[DONE]";
/// The instruction that turns a chat model into a completion engine for
/// ghost text: continuation only, never prose.
const GHOST_SYSTEM_PROMPT: &str = "You are an inline code completion engine. You are given the code before the caret. Reply with ONLY the continuation that could be typed next at the caret — no prose, no explanation, no code fences, no repetition of the given code.";
/// Backoff schedule for HTTP 429/5xx retries (1s, 2s, 4s — the §6.5 hard cap
/// of 8s bounds any future extension of this schedule).
const HTTP_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
/// Backoff for the single network-error retry (ARCHITECTURE.md §6.5).
const NET_RETRY_DELAY: Duration = Duration::from_millis(500);
/// How much of a gateway error body to surface in the error message.
const MAX_ERROR_BODY_CHARS: usize = 512;

/// Client for the Selora gateway.
///
/// The API key is only ever sent in the `Authorization` header. It never
/// appears in error messages or `Debug` output: error bodies are redacted and
/// the `Debug` impl prints `[REDACTED]`.
pub struct SeloraClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
    /// Test seam: the production retry policy with millisecond delays.
    net_retry_delay: Duration,
    http_retry_delays: [Duration; 3],
}

impl fmt::Debug for SeloraClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The API key must never reach logs or panics (RISKS.md §8.3).
        f.debug_struct("SeloraClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl SeloraClient {
    /// A client for `base_url` (e.g. `https://api.selora.lol`) authenticated
    /// with `api_key`. No settings or keyring access: the caller owns key
    /// retrieval. A trailing `/` on `base_url` is tolerated.
    pub fn new(base_url: &str, api_key: &str) -> Self {
        // `build()` fails only if the TLS backend fails to initialize, which
        // a default client would equally fail on; fall back rather than panic.
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key: api_key.to_owned(),
            http,
            net_retry_delay: NET_RETRY_DELAY,
            http_retry_delays: HTTP_RETRY_DELAYS,
        }
    }

    /// `GET {base_url}/v1/models` — the model catalog, bearer-authenticated.
    pub async fn list_models(&self) -> Result<Vec<Model>, SeloraError> {
        let url = format!("{}/v1/models", self.base_url);
        let response = self
            .send_with_retry(|| self.http.get(&url).bearer_auth(&self.api_key))
            .await?;

        let body: ModelsResponse = response
            .json()
            .await
            .map_err(|err| SeloraError::Ai(format!("invalid models response: {err}")))?;

        let mut models = body.data;
        for model in &mut models {
            if model.display_name.is_empty() {
                model.display_name = model.id.clone();
            }
        }
        Ok(models)
    }

    /// `POST {base_url}/v1/chat/completions` with `stream: true`.
    ///
    /// Consumes the SSE body: one [`ChatEvent::Delta`] per content delta,
    /// [`ChatEvent::Done`] on the `[DONE]` sentinel, and the receipt is both
    /// sent on the channel and returned. On any failure the same error is
    /// sent as [`ChatEvent::Error`] before being returned.
    ///
    /// Retries (§6.5) cover request establishment only. A failure *mid-stream*
    /// is never retried: replaying the request would duplicate the deltas the
    /// UI already displayed. The caller re-sends if the user wants to.
    pub async fn stream_chat(
        &self,
        req: ChatRequest,
        tx: mpsc::UnboundedSender<ChatEvent>,
    ) -> Result<ChatReceipt, SeloraError> {
        let request_id = Uuid::new_v4();
        let started = Instant::now();
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = req.clone().with_stream(true);

        let response = match self
            .send_with_retry(|| self.http.post(&url).bearer_auth(&self.api_key).json(&body))
            .await
        {
            Ok(response) => response,
            Err(err) => {
                let _ = send_event(&tx, ChatEvent::Error(duplicate_error(&err)));
                return Err(err);
            }
        };

        // Receipt inputs: the last `usage` the gateway sent, and the raw
        // character counts for the chars/4 fallback (RISKS.md §3).
        let prompt_chars: usize = req.messages.iter().map(|m| m.content.chars().count()).sum();
        let mut usage: Option<Usage> = None;
        let mut completion_chars: usize = 0;

        let mut buffer: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(bytes) => bytes,
                Err(err) => {
                    let error = SeloraError::Net(format!("stream read failed: {err}"));
                    let _ = send_event(&tx, ChatEvent::Error(duplicate_error(&error)));
                    return Err(error);
                }
            };
            buffer.extend_from_slice(&chunk);

            loop {
                match crate::sse::parse_sse_event(&buffer) {
                    Err(err) => {
                        let _ = send_event(&tx, ChatEvent::Error(duplicate_error(&err)));
                        return Err(err);
                    }
                    Ok(None) => break, // incomplete event: wait for more bytes
                    Ok(Some((event, consumed))) => {
                        buffer.drain(..consumed);
                        let Some(data) = event.data else {
                            continue; // fieldless/comment-only event
                        };
                        if data == DONE_SENTINEL {
                            let receipt = build_receipt(
                                &req,
                                request_id,
                                started,
                                usage,
                                prompt_chars,
                                completion_chars,
                            );
                            send_event(&tx, ChatEvent::Done(receipt.clone()))?;
                            return Ok(receipt);
                        }
                        let chunk: ChatChunk = match serde_json::from_str(&data) {
                            Ok(chunk) => chunk,
                            Err(err) => {
                                let error =
                                    SeloraError::Ai(format!("malformed stream chunk: {err}"));
                                let _ = send_event(&tx, ChatEvent::Error(duplicate_error(&error)));
                                return Err(error);
                            }
                        };
                        if let Some(chunk_usage) = chunk.usage {
                            usage = Some(chunk_usage);
                        }
                        let Some(delta) = chunk
                            .choices
                            .first()
                            .and_then(|choice| choice.delta.as_ref())
                        else {
                            continue;
                        };
                        if let Some(content) = delta.content.as_deref() {
                            if !content.is_empty() {
                                completion_chars += content.chars().count();
                                send_event(&tx, ChatEvent::Delta(content.to_owned()))?;
                            }
                        }
                        if let Some(fragments) = delta.tool_calls.as_deref() {
                            for fragment in fragments {
                                let function = fragment.function.as_ref();
                                let arguments = function
                                    .and_then(|function| function.arguments.as_deref())
                                    .unwrap_or_default();
                                let name = function.and_then(|function| function.name.clone());
                                let meaningful =
                                    fragment.id.as_deref().is_some_and(|id| !id.is_empty())
                                        || name.as_deref().is_some_and(|name| !name.is_empty())
                                        || !arguments.is_empty();
                                if !meaningful {
                                    continue; // empty trailing fragment
                                }
                                send_event(
                                    &tx,
                                    ChatEvent::ToolCallDelta {
                                        // Gateways that omit `index` (single-call
                                        // responses) land in slot 0.
                                        index: fragment.index.unwrap_or(0),
                                        id: fragment.id.clone(),
                                        name,
                                        arguments: arguments.to_owned(),
                                    },
                                )?;
                            }
                        }
                    }
                }
            }
        }

        // The body ended without the `[DONE]` sentinel: the reply was
        // truncated (RISKS.md §3). Never report a receipt we cannot trust.
        let error = SeloraError::Ai("stream closed before [DONE]".into());
        let _ = send_event(&tx, ChatEvent::Error(duplicate_error(&error)));
        Err(error)
    }

    /// `POST {base_url}/v1/chat/completions` with `stream: false` — one
    /// ghost-text completion (ARCHITECTURE.md §10.6, PLAN.md §6.4).
    ///
    /// `model` is the caller's choice (the shell passes the active model);
    /// `prompt` is the editor's up-to-caret prefix. The reply's first
    /// choice content is the suggestion, trimmed. Empty or whitespace-only
    /// content is `Ok(None)`: a suggestion of nothing is no suggestion.
    ///
    /// Non-streaming on purpose — a ghost completion is short and the
    /// editor needs the whole string in one piece to paint at the caret.
    /// Receipts for ghost completions are not returned: PLAN.md §6.4 keeps
    /// the cost meter fed by the `usage` of the *chat* stream; the shell
    /// can add ghost receipts to the total via the response length if the
    /// owner asks for it.
    pub async fn ghost_complete(
        &self,
        model: &str,
        prompt: &str,
    ) -> Result<Option<String>, SeloraError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = ChatRequest::new(
            model,
            vec![
                ChatMessage::system(GHOST_SYSTEM_PROMPT),
                ChatMessage::user(prompt),
            ],
        );

        let response = self
            .send_with_retry(|| self.http.post(&url).bearer_auth(&self.api_key).json(&body))
            .await?;
        let reply: CompletionResponse = response
            .json()
            .await
            .map_err(|err| SeloraError::Ai(format!("invalid completion response: {err}")))?;

        Ok(reply
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message)
            .map(|message| message.content.unwrap_or_default())
            .map(|content| content.trim().to_owned())
            .filter(|content| !content.is_empty()))
    }

    /// `POST {base_url}/v1/chat/completions` with `stream: false` — the
    /// whole assistant reply in one piece: the content plus, when the
    /// model invoked tools, the complete `tool_calls` of the message
    /// (ARCHITECTURE.md §17.6 non-streaming shape). `Ok(None)` when the
    /// gateway returns no choices.
    ///
    /// The agent loop itself streams (§17.3 — it needs live deltas for
    /// the chat bubble); this is the non-streaming seam for callers that
    /// want a finished message in one shot. `req.stream` is forced off
    /// regardless of what the caller left in it. Tool arguments stay raw
    /// JSON strings — the router (Phase 5b) parses them against the
    /// schema, never this layer.
    pub async fn complete_chat(
        &self,
        req: ChatRequest,
    ) -> Result<Option<ChatMessage>, SeloraError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = req.with_stream(false);

        let response = self
            .send_with_retry(|| self.http.post(&url).bearer_auth(&self.api_key).json(&body))
            .await?;
        let reply: CompletionResponse = response
            .json()
            .await
            .map_err(|err| SeloraError::Ai(format!("invalid completion response: {err}")))?;

        Ok(reply
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message)
            .map(|message| {
                let mut reply = ChatMessage::assistant(message.content.unwrap_or_default());
                reply.tool_calls = message.tool_calls;
                reply
            }))
    }

    /// A cheap reachability ping (ARCHITECTURE.md §10.6): `HEAD {base_url}`,
    /// no retries, no body. Any HTTP response — even an error status —
    /// means the gateway is reachable; only a transport failure is
    /// "disconnected". The latency rides along for the status bar.
    pub async fn status(&self) -> SeloraStatus {
        let started = Instant::now();
        match self.http.head(&self.base_url).send().await {
            Ok(_) => SeloraStatus::Connected {
                model: None,
                latency_ms: started.elapsed().as_millis() as u32,
            },
            Err(_) => SeloraStatus::Disconnected,
        }
    }

    /// Send a request under the §6.5 retry policy:
    ///
    /// * network error → one retry after `net_retry_delay`;
    /// * 429/5xx → up to three retries on the `http_retry_delays` schedule;
    /// * 401 → no retry, `SeloraError::Auth`;
    /// * any other 4xx → no retry, surface the gateway's message.
    ///
    /// `build_request` is re-invoked per attempt because `RequestBuilder`
    /// is not reusable.
    async fn send_with_retry(
        &self,
        build_request: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, SeloraError> {
        let mut network_retries = 0;
        let mut http_retries = 0;

        loop {
            match build_request().send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response);
                    }
                    if status.as_u16() == 401 {
                        let body = self.error_body(response).await;
                        return Err(SeloraError::Auth(format!(
                            "key invalid or not authorized (HTTP 401{body})"
                        )));
                    }
                    if status.as_u16() == 429 || status.is_server_error() {
                        if http_retries < self.http_retry_delays.len() {
                            let delay = self.http_retry_delays[http_retries];
                            http_retries += 1;
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        let body = self.error_body(response).await;
                        return Err(SeloraError::Ai(format!(
                            "gateway unavailable (HTTP {status}){body}"
                        )));
                    }
                    let body = self.error_body(response).await;
                    return Err(SeloraError::Ai(format!(
                        "gateway rejected request (HTTP {status}){body}"
                    )));
                }
                Err(err) => {
                    if network_retries < 1 {
                        network_retries += 1;
                        tokio::time::sleep(self.net_retry_delay).await;
                        continue;
                    }
                    return Err(SeloraError::Net(format!("request failed: {err}")));
                }
            }
        }
    }

    /// Read a (bounded, redacted) error body for inclusion in an error
    /// message. The gateway's own words are surfaced, never paraphrased
    /// (RISKS.md §8) — but the API key is scrubbed if a proxy echoes it back.
    async fn error_body(&self, response: reqwest::Response) -> String {
        let text = response.text().await.unwrap_or_default();
        let text = text.trim();
        if text.is_empty() {
            return String::new();
        }
        let truncated: String = text.chars().take(MAX_ERROR_BODY_CHARS).collect();
        format!(": {}", self.redact(&truncated))
    }

    /// Replace any occurrence of the API key in `text` with `[REDACTED]`.
    fn redact(&self, text: &str) -> String {
        if self.api_key.is_empty() {
            text.to_owned()
        } else {
            text.replace(&self.api_key, "[REDACTED]")
        }
    }
}

/// Send an event, failing loudly if the UI dropped the channel: continuing to
/// stream into a dead receiver would burn gateway tokens for nothing.
fn send_event(tx: &mpsc::UnboundedSender<ChatEvent>, event: ChatEvent) -> Result<(), SeloraError> {
    tx.send(event)
        .map_err(|_| SeloraError::Ai("chat event receiver dropped".into()))
}

/// `SeloraError` is not `Clone` (it wraps `io::Error`, which is not either),
/// and `selora-core` is not ours to change. The stream needs the same error
/// twice — once on the channel, once as the return value — so we duplicate it
/// here, preserving the variant and the message. The `Io` variant is the only
/// lossy one: it re-wraps as a plain message-bearing `io::Error`.
fn duplicate_error(err: &SeloraError) -> SeloraError {
    match err {
        SeloraError::Io(_) => SeloraError::Io(std::io::Error::other(err.to_string())),
        SeloraError::Net(message) => SeloraError::Net(message.clone()),
        SeloraError::Lsp(message) => SeloraError::Lsp(message.clone()),
        SeloraError::Git(message) => SeloraError::Git(message.clone()),
        SeloraError::Ai(message) => SeloraError::Ai(message.clone()),
        SeloraError::Auth(message) => SeloraError::Auth(message.clone()),
        SeloraError::Config(message) => SeloraError::Config(message.clone()),
        SeloraError::Other(message) => SeloraError::Other(message.clone()),
    }
}

/// Build the receipt from the gateway's `usage` (exact) or chars/4
/// (estimated) — RISKS.md §3.
fn build_receipt(
    req: &ChatRequest,
    request_id: Uuid,
    started: Instant,
    usage: Option<Usage>,
    prompt_chars: usize,
    completion_chars: usize,
) -> ChatReceipt {
    let estimated = match &usage {
        Some(usage) => usage.prompt_tokens.is_none() || usage.completion_tokens.is_none(),
        None => true,
    };
    let prompt_tokens = usage
        .as_ref()
        .and_then(|usage| usage.prompt_tokens)
        .unwrap_or((prompt_chars / 4) as u32);
    let completion_tokens = usage
        .as_ref()
        .and_then(|usage| usage.completion_tokens)
        .unwrap_or((completion_chars / 4) as u32);
    let cost_usd = usage.and_then(|usage| usage.cost_usd).unwrap_or(0.0);

    ChatReceipt {
        request_id,
        model: req.model.clone(),
        prompt_tokens,
        completion_tokens,
        cost_usd,
        duration_ms: started.elapsed().as_millis() as u32,
        estimated,
    }
}

// ---- Private wire types (not part of the public surface) ----

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<Model>,
}

/// One streamed chat chunk (OpenAI-compatible shape).
#[derive(Debug, Deserialize)]
struct ChatChunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

/// One non-streaming completion reply (ghost text, ARCHITECTURE.md §10.6).
#[derive(Debug, Deserialize)]
struct CompletionResponse {
    #[serde(default)]
    choices: Vec<CompletionChoice>,
}

#[derive(Debug, Deserialize)]
struct CompletionChoice {
    #[serde(default)]
    message: Option<CompletionMessage>,
}

#[derive(Debug, Deserialize)]
struct CompletionMessage {
    #[serde(default)]
    content: Option<String>,
    /// Complete tool calls on a non-streaming assistant message
    /// (ARCHITECTURE.md §17.6) — parsed by [`ToolCall`]'s tolerant
    /// deserializer in the OpenAI nested shape.
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Option<ChunkDelta>,
}

#[derive(Debug, Deserialize)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    /// Streaming tool-call fragments (the OpenAI `delta.tool_calls[]`
    /// array) — emitted as [`ChatEvent::ToolCallDelta`] for the caller's
    /// [`crate::ToolCallAccumulator`] to reassemble.
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallFragment>>,
}

/// One `delta.tool_calls[]` fragment: `{index?, id?, function:{name?,
/// arguments?}}`. Every field may be absent — the gateway sends the id
/// and name on the first fragment of a call and the arguments across
/// many. An absent `index` (single-call gateway quirk) resolves to 0.
#[derive(Debug, Deserialize)]
struct ToolCallFragment {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FragmentFunction>,
}

#[derive(Debug, Deserialize)]
struct FragmentFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Usage block on the final chunk. Absent → chars/4 estimate (RISKS.md §3).
#[derive(Debug, Clone, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default, alias = "cost")]
    cost_usd: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_types::{ToolCall, ToolCallAccumulator, ToolSpec};
    use crate::sse::MAX_DATA_FIELD_BYTES;
    use crate::types::{ChatMessage, Role};

    use wiremock::matchers::{body_json, body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_KEY: &str = "sk-gw-test-key-000000000000";

    fn test_client(base_url: &str) -> SeloraClient {
        let mut client = SeloraClient::new(base_url, TEST_KEY);
        // Same retry policy, millisecond delays: retries stay observable
        // without slowing the suite down.
        client.net_retry_delay = Duration::from_millis(2);
        client.http_retry_delays = [Duration::from_millis(2); 3];
        client
    }

    fn chat_request() -> ChatRequest {
        // "1234" → 4 chars → 1 estimated prompt token.
        ChatRequest::new("test-model", vec![ChatMessage::user("1234")])
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<ChatEvent>) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    fn deltas(events: &[ChatEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                ChatEvent::Delta(delta) => Some(delta.clone()),
                _ => None,
            })
            .collect()
    }

    fn models_json() -> serde_json::Value {
        serde_json::json!({
            "object": "list",
            "data": [
                {
                    "id": "claude-opus-5-5",
                    "display_name": "Claude Opus 5.5",
                    "context_window": 200000,
                    "price_per_1k_input": 0.015,
                    "price_per_1k_output": 0.075,
                    "supports_streaming": true,
                    "supports_tools": true,
                    "family": "claude"
                },
                { "id": "glm-5.3" }
            ]
        })
    }

    #[tokio::test]
    async fn list_models_returns_models_with_bearer_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", format!("Bearer {TEST_KEY}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(models_json()))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let models = client.list_models().await.expect("list_models succeeds");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-opus-5-5");
        assert_eq!(models[0].price_per_1k_output, 0.075);
        assert_eq!(models[0].family, "claude");
        // Missing display_name falls back to the id.
        assert_eq!(models[1].display_name, "glm-5.3");
    }

    #[tokio::test]
    async fn list_models_invalid_json_is_an_ai_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client.list_models().await.unwrap_err();
        assert!(matches!(&err, SeloraError::Ai(msg) if msg.contains("models response")));
    }

    #[tokio::test]
    async fn stream_chat_happy_path_streams_deltas_and_returns_receipt() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2,\"cost_usd\":0.0013}}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            // The wire request must carry stream: true.
            .and(body_partial_json(
                serde_json::json!({ "model": "test-model", "stream": true }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let receipt = client
            .stream_chat(chat_request(), tx)
            .await
            .expect("stream_chat succeeds");

        let events = drain(&mut rx);
        assert_eq!(deltas(&events), ["Hello", " world"]);
        assert!(matches!(events.last(), Some(ChatEvent::Done(_))));

        assert_eq!(receipt.model, "test-model");
        assert_eq!(receipt.prompt_tokens, 9);
        assert_eq!(receipt.completion_tokens, 2);
        assert_eq!(receipt.cost_usd, 0.0013);
        assert!(!receipt.estimated, "gateway sent exact usage");
        assert!(!receipt.request_id.is_nil());
        assert!(matches!(events.last(), Some(ChatEvent::Done(done)) if *done == receipt));
    }

    #[tokio::test]
    async fn stream_chat_without_usage_estimates_tokens_from_chars() {
        let server = MockServer::start().await;
        // 8 output chars → 2 estimated completion tokens; no usage block.
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"abcd\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"abcd\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let receipt = client
            .stream_chat(chat_request(), tx)
            .await
            .expect("stream_chat succeeds");

        assert!(receipt.estimated, "no usage → estimated");
        // Prompt "1234" → 1 token; completion "abcdabcd" → 2 tokens (chars/4).
        assert_eq!(receipt.prompt_tokens, 1);
        assert_eq!(receipt.completion_tokens, 2);
        assert_eq!(receipt.cost_usd, 0.0);
        assert_eq!(deltas(&drain(&mut rx)), ["abcd", "abcd"]);
    }

    #[tokio::test]
    async fn stream_chat_401_maps_to_auth_error_with_no_retry() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .expect(1) // 401 is never retried (ARCHITECTURE.md §6.5)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(
            matches!(&err, SeloraError::Auth(msg) if msg.contains("401")),
            "expected Auth(401), got: {err:?}"
        );
        let events = drain(&mut rx);
        assert!(matches!(events.first(), Some(ChatEvent::Error(_))));
    }

    #[tokio::test]
    async fn stream_chat_502_retries_three_times_then_fails() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
            .expect(4) // 1 attempt + 3 retries (ARCHITECTURE.md §6.5)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(
            matches!(&err, SeloraError::Ai(msg) if msg.contains("502") && msg.contains("bad gateway")),
            "expected Ai(502), got: {err:?}"
        );
        assert!(matches!(drain(&mut rx).first(), Some(ChatEvent::Error(_))));
    }

    #[tokio::test]
    async fn stream_chat_other_4xx_surfaces_gateway_message_without_retry() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string("model `nope` not found"))
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, _rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(
            matches!(&err, SeloraError::Ai(msg) if msg.contains("model `nope` not found")),
            "gateway's own words must surface, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn stream_chat_missing_done_is_a_stream_error() {
        let server = MockServer::start().await;
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();

        assert!(
            matches!(&err, SeloraError::Ai(msg) if msg.contains("closed before [DONE]")),
            "expected missing-[DONE] error, got: {err:?}"
        );
        let events = drain(&mut rx);
        assert_eq!(deltas(&events), ["Hi"]);
        assert!(matches!(events.last(), Some(ChatEvent::Error(_))));
    }

    #[tokio::test]
    async fn stream_chat_oversized_chunk_aborts_the_stream() {
        let server = MockServer::start().await;
        let mut sse = String::from("data: ");
        sse.push_str(&"x".repeat(MAX_DATA_FIELD_BYTES + 1));
        sse.push_str("\n\n");
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, _rx) = mpsc::unbounded_channel();
        let err = client.stream_chat(chat_request(), tx).await.unwrap_err();
        assert!(
            matches!(&err, SeloraError::Ai(msg) if msg.contains("oversized")),
            "expected oversized-chunk error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn ghost_complete_returns_the_trimmed_first_choice() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            // Ghost completions are non-streaming on the wire.
            .and(body_partial_json(
                serde_json::json!({ "model": "glm-5.3", "stream": false }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [
                    { "message": { "role": "assistant", "content": "  foo(bar);\n" } },
                    { "message": { "role": "assistant", "content": "ignored" } }
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let suggestion = client
            .ghost_complete("glm-5.3", "fn main() { /* caret */ ")
            .await
            .expect("ghost_complete succeeds");
        assert_eq!(suggestion.as_deref(), Some("foo(bar);"));
    }

    #[tokio::test]
    async fn ghost_complete_blank_reply_is_none_not_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [ { "message": { "content": "   " } } ]
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        assert_eq!(
            client.ghost_complete("m", "prefix").await.expect("ok"),
            None
        );
    }

    #[tokio::test]
    async fn ghost_complete_no_choices_is_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": []
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        assert_eq!(
            client.ghost_complete("m", "prefix").await.expect("ok"),
            None
        );
    }

    #[tokio::test]
    async fn ghost_complete_401_is_an_auth_error() {
        let server = MockServer::start().await;
        let _mock = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client.ghost_complete("m", "prefix").await.unwrap_err();
        assert!(matches!(&err, SeloraError::Auth(msg) if msg.contains("401")));
    }

    #[tokio::test]
    async fn status_reports_connected_on_any_http_response() {
        // Even a 404 on HEAD means the gateway answered: reachable.
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        match client.status().await {
            SeloraStatus::Connected { model, latency_ms } => {
                assert_eq!(model, None);
                assert!(latency_ms < 60_000, "latency should be small: {latency_ms}");
            }
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn status_reports_disconnected_on_transport_failure() {
        // Bind a port and drop the listener: connection refused.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);

        let client = test_client(&format!("http://127.0.0.1:{port}"));
        assert_eq!(client.status().await, SeloraStatus::Disconnected);
    }

    #[tokio::test]
    async fn list_models_502_then_success_recovers() {
        let server = MockServer::start().await;
        // Mounted first → matched after the flaky one exhausts itself.
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(models_json()))
            .mount(&server)
            .await;
        // Mounted last → tried first (wiremock matches newest first), exactly once.
        // The guard must stay bound: dropping it early would unmount the mock.
        let _flaky = Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(502))
            .up_to_n_times(1)
            .mount_as_scoped(&server)
            .await;

        let client = test_client(&server.uri());
        let models = client.list_models().await.expect("recovers after one 502");
        assert_eq!(models.len(), 2);
    }

    #[tokio::test]
    async fn network_error_is_retried_once_then_surfaced() {
        // Bind a port and drop the listener: nothing is listening there, so
        // the connection is refused — a network error, not an HTTP status.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);

        let client = test_client(&format!("http://127.0.0.1:{port}"));
        let err = client.list_models().await.unwrap_err();
        assert!(
            matches!(&err, SeloraError::Net(msg) if msg.contains("request failed")),
            "expected Net error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn gateway_body_echoing_the_key_is_redacted_in_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_string(format!("key {TEST_KEY} is not allowed")),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client.list_models().await.unwrap_err();
        let rendered = err.to_string();
        assert!(!rendered.contains(TEST_KEY), "key leaked: {rendered}");
        assert!(
            rendered.contains("[REDACTED]"),
            "expected redaction: {rendered}"
        );
    }

    #[test]
    fn debug_output_never_contains_the_api_key() {
        let client = SeloraClient::new("http://example.test/", TEST_KEY);
        let rendered = format!("{client:?}");
        assert!(!rendered.contains(TEST_KEY), "key leaked: {rendered}");
        assert!(rendered.contains("[REDACTED]"));
        assert!(rendered.contains("http://example.test")); // trailing / trimmed
    }

    // ---- Phase 5a: tool-call request plumbing (ARCHITECTURE.md §17.6) ----

    /// The two-tool manifest the scripted agent conversation arms.
    fn agent_manifest() -> Vec<ToolSpec> {
        vec![
            ToolSpec::new(
                "read_file",
                "Read a workspace file",
                serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }),
            ),
            ToolSpec::new(
                "write_file",
                "Write a workspace file",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }),
            ),
        ]
    }

    /// One streamed SSE chunk carrying a content delta.
    fn content_chunk(text: &str) -> serde_json::Value {
        serde_json::json!({ "choices": [ { "delta": { "content": text } } ] })
    }

    /// One streamed SSE chunk carrying a single `delta.tool_calls[]`
    /// fragment. Absent fields are sent as JSON `null` — the parser also
    /// handles fully omitted keys.
    fn fragment_chunk(
        index: Option<usize>,
        id: Option<&str>,
        name: Option<&str>,
        arguments: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "choices": [ {
                "delta": { "tool_calls": [ {
                    "index": index,
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                } ] }
            } ]
        })
    }

    /// The terminal chunk: the gateway's `finish_reason`.
    fn finish_chunk(reason: &str) -> serde_json::Value {
        serde_json::json!({ "choices": [ { "delta": {}, "finish_reason": reason } ] })
    }

    /// Wrap streamed chunks into an SSE body terminated by `[DONE]`.
    fn sse_body(chunks: &[serde_json::Value]) -> String {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str("data: ");
            body.push_str(&chunk.to_string());
            body.push_str("\n\n");
        }
        body.push_str("data: [DONE]\n\n");
        body
    }

    /// Stream one armed agent turn and return `(text, tool_calls)` —
    /// exactly what the Phase 5c loop does per iteration: relay events,
    /// accumulate tool-call fragments, drain at `Done`.
    async fn run_agent_turn(
        client: &SeloraClient,
        conversation: Vec<ChatMessage>,
    ) -> (String, Vec<ToolCall>) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let receipt = client
            .stream_chat(
                ChatRequest::new("test-model", conversation).with_tools(agent_manifest()),
                tx,
            )
            .await
            .expect("turn streams successfully");

        let events = drain(&mut rx);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ChatEvent::Error(_))),
            "no error events expected: {events:?}"
        );
        assert!(
            matches!(events.last(), Some(ChatEvent::Done(done)) if *done == receipt),
            "the stream must end with the returned receipt"
        );

        let mut accumulator = ToolCallAccumulator::new();
        for event in &events {
            accumulator.feed(event);
        }
        (deltas(&events).concat(), accumulator.finish())
    }

    /// The scripted 3-turn agent conversation (deliverable): turn 1 asks
    /// for a read, turn 2 for a write (arguments split mid-JSON across
    /// chunks), turn 3 answers with plain text. Every request body is
    /// exact-matched against the hand-written OpenAI wire shape — tools
    /// armed, assistant `tool_calls` replayed, tool results appended.
    #[tokio::test]
    async fn agent_three_turn_conversation_read_write_then_answer() {
        let server = MockServer::start().await;

        let user_prompt = "Read src/main.rs and add an entry-point comment.";
        let read_args = r#"{"path":"src/main.rs"}"#;
        let write_args = r#"{"path":"src/lib.rs","content":"// entry point\n"}"#;
        let read_result = "fn main() { println!(\"hello, selora\"); }";
        let write_result = "wrote 25 bytes to src/lib.rs";

        let read_call = ToolCall {
            id: "call_read_1".into(),
            name: "read_file".into(),
            arguments: read_args.into(),
        };
        let write_call = ToolCall {
            id: "call_write_1".into(),
            name: "write_file".into(),
            arguments: write_args.into(),
        };

        // The expected wire bodies, hand-written (not produced by the
        // serializer under test): the OpenAI `tools` array shape, the
        // assistant message's nested `tool_calls`, the tool-result
        // messages.
        let expected_tools = serde_json::json!([
            {
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a workspace file",
                    "parameters": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"]
                    }
                }
            },
            {
                "type": "function",
                "function": {
                    "name": "write_file",
                    "description": "Write a workspace file",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "content": { "type": "string" }
                        },
                        "required": ["path", "content"]
                    }
                }
            }
        ]);
        let assistant_read = serde_json::json!({
            "role": "assistant",
            "content": "Let me read that.",
            "tool_calls": [
                { "id": "call_read_1", "type": "function",
                  "function": { "name": "read_file", "arguments": read_args } }
            ]
        });
        let assistant_write = serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                { "id": "call_write_1", "type": "function",
                  "function": { "name": "write_file", "arguments": write_args } }
            ]
        });
        let turn_bodies = [
            serde_json::json!({
                "model": "test-model",
                "messages": [ { "role": "user", "content": user_prompt } ],
                "temperature": 0.7,
                "stream": true,
                "tools": expected_tools
            }),
            serde_json::json!({
                "model": "test-model",
                "messages": [
                    { "role": "user", "content": user_prompt },
                    assistant_read.clone(),
                    { "role": "tool", "tool_call_id": "call_read_1", "content": read_result }
                ],
                "temperature": 0.7,
                "stream": true,
                "tools": expected_tools.clone()
            }),
            serde_json::json!({
                "model": "test-model",
                "messages": [
                    { "role": "user", "content": user_prompt },
                    assistant_read,
                    { "role": "tool", "tool_call_id": "call_read_1", "content": read_result },
                    assistant_write,
                    { "role": "tool", "tool_call_id": "call_write_1", "content": write_result }
                ],
                "temperature": 0.7,
                "stream": true,
                "tools": expected_tools
            }),
        ];

        // Turn 1: text commentary, then a read_file call whose arguments
        // stream in three pieces (id+name first, then two argument
        // fragments), finish_reason "tool_calls".
        let turn1_sse = sse_body(&[
            content_chunk("Let me read that."),
            fragment_chunk(Some(0), Some("call_read_1"), Some("read_file"), ""),
            fragment_chunk(Some(0), None, None, "{\"path\""),
            fragment_chunk(Some(0), None, None, ":\"src/main.rs\"}"),
            finish_chunk("tool_calls"),
        ]);
        // Turn 2: a write_file call with arguments split mid-JSON, one
        // fragment missing its `index` (single-call gateway quirk), one
        // empty trailing fragment.
        let turn2_sse = sse_body(&[
            fragment_chunk(Some(0), Some("call_write_1"), Some("write_file"), ""),
            fragment_chunk(None, None, None, "{\"path\":\"src/lib.rs\",\"co"),
            fragment_chunk(Some(0), None, None, "ntent\":\"// entry point\\n\"}"),
            fragment_chunk(Some(0), None, None, ""),
            finish_chunk("tool_calls"),
        ]);
        // Turn 3: the final answer, finish_reason "stop", with usage.
        let turn3_sse = sse_body(&[
            content_chunk("Done — src/lib.rs now opens "),
            content_chunk("with `// entry point`."),
            serde_json::json!({
                "choices": [ { "delta": {}, "finish_reason": "stop" } ],
                "usage": { "prompt_tokens": 42, "completion_tokens": 9, "cost_usd": 0.0021 }
            }),
        ]);

        // Scoped guards live to the end of the test (each verifies its
        // expect(1) on drop — one exact-body hit per turn).
        let mut guards = Vec::new();
        for (body, sse) in std::iter::zip(turn_bodies, [turn1_sse, turn2_sse, turn3_sse]) {
            guards.push(
                Mock::given(method("POST"))
                    .and(path("/v1/chat/completions"))
                    .and(body_json(body))
                    .respond_with(ResponseTemplate::new(200).set_body_string(sse))
                    .expect(1)
                    .mount_as_scoped(&server)
                    .await,
            );
        }

        let client = test_client(&server.uri());
        let mut conversation = vec![ChatMessage::user(user_prompt)];

        // Turn 1 — the model asks for a read (finish_reason: tool_calls).
        let (text, calls) = run_agent_turn(&client, conversation.clone()).await;
        assert_eq!(text, "Let me read that.");
        assert_eq!(calls, vec![read_call]);
        let mut assistant = ChatMessage::assistant(text);
        assistant.tool_calls = Some(calls);
        conversation.push(assistant);
        conversation.push(ChatMessage::tool_result("call_read_1", read_result));

        // Turn 2 — the model asks for a write (arguments reassembled).
        let (text, calls) = run_agent_turn(&client, conversation.clone()).await;
        assert_eq!(text, "");
        assert_eq!(calls, vec![write_call]);
        let mut assistant = ChatMessage::assistant(text);
        assistant.tool_calls = Some(calls);
        conversation.push(assistant);
        conversation.push(ChatMessage::tool_result("call_write_1", write_result));

        // Turn 3 — the final answer (finish_reason: stop, no tool calls).
        let (text, calls) = run_agent_turn(&client, conversation).await;
        assert!(
            calls.is_empty(),
            "a stop turn must drain no tool calls: {calls:?}"
        );
        assert_eq!(text, "Done — src/lib.rs now opens with `// entry point`.");
    }

    /// Plain chat never arms tools: an exact body match with no `tools`
    /// key (any `tools` field on the wire would miss the mock and fail).
    #[tokio::test]
    async fn plain_chat_request_carries_no_tools_on_the_wire() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_json(serde_json::json!({
                "model": "test-model",
                "messages": [ { "role": "user", "content": "1234" } ],
                "temperature": 0.7,
                "stream": true
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse_body(&[content_chunk("hi"), finish_chunk("stop")])),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let receipt = client
            .stream_chat(chat_request(), tx)
            .await
            .expect("plain chat streams");
        assert_eq!(deltas(&drain(&mut rx)), ["hi"]);
        assert!(!receipt.request_id.is_nil());
    }

    /// The loop's tool_calls-vs-stop seam: `finish_reason: "tool_calls"`
    /// streams drain into calls (and at most commentary text);
    /// `finish_reason: "stop"` streams drain nothing and the text is the
    /// final answer.
    #[tokio::test]
    async fn finish_reason_distinguishes_tool_calls_from_stop() {
        let tool_calls_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body(&[
                fragment_chunk(
                    Some(0),
                    Some("call_1"),
                    Some("read_file"),
                    r#"{"path":"a.rs"}"#,
                ),
                finish_chunk("tool_calls"),
            ])))
            .expect(1)
            .mount(&tool_calls_server)
            .await;
        let client = test_client(&tool_calls_server.uri());
        let (text, calls) = run_agent_turn(&client, vec![ChatMessage::user("read a.rs")]).await;
        assert_eq!(text, "");
        assert_eq!(
            calls,
            vec![ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            }]
        );

        let stop_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body(&[
                content_chunk("The answer is 42."),
                finish_chunk("stop"),
            ])))
            .expect(1)
            .mount(&stop_server)
            .await;
        let client = test_client(&stop_server.uri());
        let (text, calls) = run_agent_turn(&client, vec![ChatMessage::user("the answer?")]).await;
        assert!(calls.is_empty(), "stop drains no calls: {calls:?}");
        assert_eq!(text, "The answer is 42.");
    }

    /// Gateway quirk: single-call responses that omit `index` on every
    /// fragment still reassemble into one call.
    #[tokio::test]
    async fn stream_fragments_without_index_reassemble_into_the_first_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body(&[
                fragment_chunk(None, Some("call_9"), Some("list_dir"), ""),
                fragment_chunk(None, None, None, "{\"path\""),
                fragment_chunk(None, None, None, ":\".\"}"),
                finish_chunk("tool_calls"),
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let (_, calls) = run_agent_turn(&client, vec![ChatMessage::user("list")]).await;
        assert_eq!(
            calls,
            vec![ToolCall {
                id: "call_9".into(),
                name: "list_dir".into(),
                arguments: "{\"path\":\".\"}".into(),
            }]
        );
    }

    /// Non-streaming replies carry complete `tool_calls` on the assistant
    /// message (ARCHITECTURE.md §17.6) — including the gateway quirk of
    /// arguments already parsed into a JSON object.
    #[tokio::test]
    async fn complete_chat_parses_tool_calls_from_the_assistant_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(
                serde_json::json!({ "model": "test-model", "stream": false }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [ {
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "read_file",
                                    "arguments": "{\"path\":\"src/main.rs\"}"
                                }
                            },
                            {
                                "id": "call_2",
                                "type": "function",
                                "function": { "name": "list_dir", "arguments": { "path": "." } }
                            }
                        ]
                    }
                } ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let message = client
            .complete_chat(chat_request())
            .await
            .expect("complete_chat succeeds")
            .expect("a message");
        assert_eq!(message.role, Role::Assistant);
        assert_eq!(message.content, "");
        let calls = message.tool_calls.expect("two tool calls");
        assert_eq!(
            calls[0],
            ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            }
        );
        assert_eq!(calls[1].name, "list_dir");
        assert_eq!(calls[1].arguments, "{\"path\":\".\"}");
    }

    /// Non-streaming plain replies come back as ordinary assistant
    /// messages; no choices is `Ok(None)`, not an error.
    #[tokio::test]
    async fn complete_chat_returns_plain_replies_and_none_without_choices() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [ { "message": { "role": "assistant", "content": "hello there" } } ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let message = client
            .complete_chat(chat_request())
            .await
            .expect("complete_chat succeeds")
            .expect("a message");
        assert_eq!(message, ChatMessage::assistant("hello there"));

        let empty = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "choices": [] })),
            )
            .expect(1)
            .mount(&empty)
            .await;
        let client = test_client(&empty.uri());
        assert!(
            matches!(client.complete_chat(chat_request()).await, Ok(None)),
            "no choices is Ok(None)"
        );
    }
}
