//! Selora gateway data model (ARCHITECTURE.md §11).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use selora_core::SeloraError;

use crate::chat_types::{ToolCall, ToolSpec};

/// Identifier of a model on the gateway (e.g. `"claude-opus-5-5"`).
pub type ModelId = String;

fn deserialize_lenient_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        Some(serde_json::Value::Number(n)) => Ok(n.as_u64().unwrap_or(128_000) as u32),
        Some(serde_json::Value::String(s)) => Ok(s.parse::<u32>().unwrap_or(128_000)),
        _ => Ok(128_000),
    }
}

fn deserialize_lenient_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        Some(serde_json::Value::Number(n)) => Ok(n.as_f64().unwrap_or(0.0)),
        Some(serde_json::Value::String(s)) => Ok(s.parse::<f64>().unwrap_or(0.0)),
        _ => Ok(0.0),
    }
}

/// A model offered by the gateway (`GET /v1/models`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    #[serde(default, alias = "model")]
    pub id: ModelId,
    /// Human-facing name. Falls back to `id` when the gateway omits it.
    #[serde(default, alias = "name", alias = "displayName")]
    pub display_name: String,
    /// Maximum context length in tokens.
    #[serde(
        default,
        alias = "context_length",
        alias = "contextWindow",
        deserialize_with = "deserialize_lenient_u32"
    )]
    pub context_window: u32,
    /// USD per 1k input tokens.
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub price_per_1k_input: f64,
    /// USD per 1k output tokens.
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
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
///
/// `Tool` is the tool-result role (ARCHITECTURE.md §17.6): a message with
/// this role carries `tool_call_id` and the tool's output content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// One message in a chat conversation.
///
/// The agent loop grows this additively (ARCHITECTURE.md §17.6): an
/// assistant message may carry `tool_calls` (the model's tool invocations),
/// and a tool-result message carries `tool_call_id`. Both are `None` for
/// every pre-agent message, so old transcripts deserialize unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Assistant-side tool invocations (`None` on plain messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The call this tool-result message answers (tool role only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// A tool-result message (ARCHITECTURE.md §17.3: results are appended
    /// as `{ role: "tool", tool_call_id, content }` and fed back into the
    /// next request).
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
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
    /// The tool manifest (ARCHITECTURE.md §17.3: every agent request arms
    /// the full fixed manifest; plain chat sends `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
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
            tools: None,
            reasoning_effort: None,
        }
    }

    /// Builder-style override of temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Builder-style override of reasoning effort.
    pub fn with_reasoning_effort(mut self, reasoning_effort: Option<String>) -> Self {
        self.reasoning_effort = reasoning_effort;
        self
    }

    /// Builder-style override of the `stream` flag.
    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    /// Builder-style override of the tool manifest (agent mode).
    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = Some(tools);
        self
    }
}

/// An event emitted on the chat channel while a completion streams.
#[derive(Debug)]
pub enum ChatEvent {
    /// One content delta from the model.
    Delta(String),
    /// A streaming fragment of a tool call (ARCHITECTURE.md §17.6: the
    /// same accumulate-then-parse pattern as content deltas — the
    /// arguments arrive as partial JSON across many deltas). `index`
    /// identifies which call of the response is growing.
    ToolCallDelta {
        /// The call's position in the response's `tool_calls` array.
        index: usize,
        /// The call id, present only on the first fragment of a call.
        id: Option<String>,
        /// The tool name, present only on the first fragment of a call.
        name: Option<String>,
        /// The partial `arguments` JSON fragment.
        arguments: String,
    },
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
    fn agent_fields_round_trip_and_stay_absent_on_plain_messages() {
        // A plain pre-agent request carries no tool fields on the wire.
        let plain = ChatRequest::new("m", vec![ChatMessage::user("hi")]);
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("tools").is_none());
        let first_message = &json["messages"][0];
        assert!(first_message.get("tool_calls").is_none());
        assert!(first_message.get("tool_call_id").is_none());
        // And deserializes back (old transcripts keep working).
        let back: ChatRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, plain);

        // An agent request: manifest armed, an assistant tool_call
        // message, and a tool-result message.
        let manifest = vec![ToolSpec::new(
            "read_file",
            "Read a workspace file",
            serde_json::json!({"type": "object"}),
        )];
        let call = ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"src/main.rs"}"#.into(),
        };
        let mut message = ChatMessage::assistant("I'll read it.");
        message.tool_calls = Some(vec![call]);
        let request = ChatRequest::new(
            "m",
            vec![message, ChatMessage::tool_result("call_1", "fn main() {}")],
        )
        .with_tools(manifest);
        let json = serde_json::to_value(&request).unwrap();
        assert!(json["tools"].is_array());
        assert!(json["messages"][0]["tool_calls"].is_array());
        assert_eq!(json["messages"][1]["role"], "tool");
        assert_eq!(json["messages"][1]["tool_call_id"], "call_1");

        let back: ChatRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, request);
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
