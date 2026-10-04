//! Selora gateway data model (ARCHITECTURE.md §11).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use selora_core::SeloraError;

/// Identifier of a model on the gateway (e.g. `"claude-opus-5-5"`).
pub type ModelId = String;

/// A model offered by the gateway (`GET /v1/models`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: ModelId,
    /// Human-facing name. Falls back to `id` when the gateway omits it.
    #[serde(default, alias = "name", alias = "displayName")]
    pub display_name: String,
    /// Maximum context length in tokens.
    #[serde(default, alias = "context_length", alias = "contextWindow")]
    pub context_window: u32,
    /// USD per 1k input tokens.
    #[serde(default)]
    pub price_per_1k_input: f64,
    /// USD per 1k output tokens.
    #[serde(default)]
    pub price_per_1k_output: f64,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_tools: bool,
    /// Model family: "claude" / "gpt" / "kimi" / "glm" / ...
    #[serde(default)]
    pub family: String,
}

fn default_true() -> bool {
    true
}

/// A chat role (wire names are lowercase, OpenAI-compatible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// One message in a chat conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// A chat completion request (`POST /v1/chat/completions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: ModelId,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub stream: bool,
}

impl ChatRequest {
    /// New request with a default temperature of 0.7 and streaming off.
    /// `stream_chat` always sets `stream: true` on the wire regardless.
    pub fn new(model: impl Into<ModelId>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            temperature: 0.7,
            max_tokens: None,
            stream: false,
        }
    }

    /// Builder-style override of the `stream` flag.
    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }
}

/// An event emitted on the chat channel while a completion streams.
#[derive(Debug)]
pub enum ChatEvent {
    /// One content delta from the model.
    Delta(String),
    /// The stream completed; carries the receipt for the cost meter.
    Done(ChatReceipt),
    /// The stream failed; the same error is returned by `stream_chat`.
    Error(SeloraError),
}

/// The receipt for one completed chat request — the source of truth for the
/// cost meter (RISKS.md §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatReceipt {
    pub request_id: Uuid,
    pub model: ModelId,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub duration_ms: u32,
    /// `true` when the gateway did not return usage and the token counts are
    /// the chars/4 estimate (RISKS.md §3).
    pub estimated: bool,
}

/// Gateway connection state for the status bar.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SeloraStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected {
        model: Option<ModelId>,
        latency_ms: u32,
    },
    Error(String),
}

/// USD cost of one completion, from a model's price table
/// (ARCHITECTURE.md §6.6). The cost meter sums these over a session.
pub fn compute_cost_usd(model: &Model, prompt_tokens: u32, completion_tokens: u32) -> f64 {
    (f64::from(prompt_tokens) * model.price_per_1k_input
        + f64::from(completion_tokens) * model.price_per_1k_output)
        / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_serializes_stream_flag_and_omits_max_tokens() {
        let req = ChatRequest::new("m", vec![ChatMessage::user("hi")]).with_stream(true);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "m");
        assert_eq!(json["stream"], true);
        assert_eq!(json["messages"][0]["role"], "user");
        assert!(json.get("max_tokens").is_none());

        let req = req.with_stream(false);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["stream"], false);
    }

    #[test]
    fn model_deserializes_minimal_and_aliased_fields() {
        let json = serde_json::json!({ "id": "glm-5.3" });
        let model: Model = serde_json::from_value(json).unwrap();
        assert_eq!(model.id, "glm-5.3");
        assert!(model.supports_streaming, "streaming defaults to true");
        assert_eq!(model.price_per_1k_input, 0.0);

        let json = serde_json::json!({
            "id": "m2", "name": "Model Two", "context_length": 128000
        });
        let model: Model = serde_json::from_value(json).unwrap();
        assert_eq!(model.display_name, "Model Two");
        assert_eq!(model.context_window, 128000);
    }

    #[test]
    fn compute_cost_usd_uses_per_1k_prices() {
        let model = Model {
            id: "m".into(),
            display_name: "m".into(),
            context_window: 0,
            price_per_1k_input: 0.5,
            price_per_1k_output: 1.5,
            supports_streaming: true,
            supports_tools: false,
            family: String::new(),
        };
        assert!((compute_cost_usd(&model, 1000, 1000) - 2.0).abs() < 1e-12);
        assert!((compute_cost_usd(&model, 250, 0) - 0.125).abs() < 1e-12);
    }
}
