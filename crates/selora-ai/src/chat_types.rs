//! Tool-call plumbing types (ARCHITECTURE.md §17.6) — Phase 5a's file.
//!
//! The types every Phase 5 subagent consumes: the manifest entries
//! ([`ToolSpec`]), the model's invocations ([`ToolCall`]), and the
//! runtime's results ([`ToolResult`]). The wire behavior — serializing
//! `ToolSpec` into the OpenAI `tools` request shape, accumulating
//! streaming [`crate::ChatEvent::ToolCallDelta`] fragments into complete
//! calls — is 5a's implementation; this skeleton fixes the shared
//! vocabulary so 5b (router), 5c (loop), 5d (cards), and 5e (web) can
//! build against it in parallel.
//!
//! The manifest itself is NOT defined here: it is fixed by
//! ARCHITECTURE.md §17.1 and lives in the agent module
//! ([`crate::agent::tool_manifest`]).

use serde::{Deserialize, Serialize};

/// A tool call's id (the gateway's string; echoed back in the result
/// message's `tool_call_id`).
pub type ToolCallId = String;

/// One entry in the tool manifest sent with every agent request
/// (ARCHITECTURE.md §17.1). `parameters` is a JSON-Schema object — kept
/// as raw `serde_json::Value` so the manifest stays declarative.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// A JSON-Schema object describing the arguments object.
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    /// A manifest entry with a literal-JSON schema.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// The model's invocation of one tool (a `tool_calls` entry of an
/// assistant message, or one parsed product of the delta accumulator).
///
/// `arguments` is the raw JSON object string — parsed against the tool's
/// schema by the router (Phase 5b), never by the wire layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    /// The raw arguments JSON string (may stream in fragments).
    pub arguments: String,
}

/// The runtime's result for one tool call — what the loop appends to the
/// conversation as a tool-result message and what the chat renders in
/// the tool card's expanded body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: ToolCallId,
    /// `false` when the tool errored (guard rejection, timeout, denied) —
    /// the content then carries the error words the model can adapt to.
    pub ok: bool,
    /// The tool's output content (already truncated to the §17.2 caps by
    /// the router).
    pub content: String,
}

impl ToolResult {
    /// An error result (guard rejection, failure, denial).
    pub fn error(id: impl Into<ToolCallId>, content: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ok: false,
            content: content.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_spec_and_call_round_trip() {
        let spec = ToolSpec::new(
            "read_file",
            "Read a workspace file",
            serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        );
        let json = serde_json::to_value(&spec).unwrap();
        let back: ToolSpec = serde_json::from_value(json).unwrap();
        assert_eq!(back, spec);

        let call = ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"src/main.rs"}"#.into(),
        };
        let json = serde_json::to_value(&call).unwrap();
        let back: ToolCall = serde_json::from_value(json).unwrap();
        assert_eq!(back, call);
    }
}
