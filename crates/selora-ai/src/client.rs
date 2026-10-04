//! `SeloraClient` — the HTTP client for the Selora gateway (ARCHITECTURE.md §6).

use std::fmt;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use selora_core::SeloraError;
use serde::Deserialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::types::{ChatEvent, ChatReceipt, ChatRequest, Model};

/// 60s to establish the connection (ARCHITECTURE.md §6.1).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// 5min read timeout: SSE bodies stream for a long time, but any single read
/// gap longer than this is a dead stream. Deliberately *not* an overall
/// request timeout, so long streams stay alive while data keeps flowing.
const READ_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The SSE sentinel that terminates a chat stream.
const DONE_SENTINEL: &str = "[DONE]";
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
                        let delta = chunk
                            .choices
                            .first()
                            .and_then(|choice| choice.delta.as_ref())
                            .and_then(|delta| delta.content.as_deref());
                        if let Some(content) = delta {
                            if !content.is_empty() {
                                completion_chars += content.chars().count();
                                send_event(&tx, ChatEvent::Delta(content.to_owned()))?;
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

#[derive(Debug, Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Option<ChunkDelta>,
}

#[derive(Debug, Deserialize)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
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
    use crate::sse::MAX_DATA_FIELD_BYTES;
    use crate::types::ChatMessage;

    use wiremock::matchers::{body_partial_json, header, method, path};
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
}
