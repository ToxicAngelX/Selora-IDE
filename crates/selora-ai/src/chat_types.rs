//! Tool-call plumbing types (ARCHITECTURE.md §17.6) — Phase 5a's file.
//!
//! The types every Phase 5 subagent consumes: the manifest entries
//! ([`ToolSpec`]), the model's invocations ([`ToolCall`]), the runtime's
//! results ([`ToolResult`]), and the [`ToolCallAccumulator`] that folds
//! streaming fragments into complete calls.
//!
//! Wire behavior (all of it lives here, all of it OpenAI-compatible):
//! * `ChatRequest.tools` serializes as
//!   `[{"type":"function","function":{"name","description","parameters"}}]`
//!   — and stays off the wire entirely when unarmed (the
//!   `skip_serializing_if` rule in `types.rs`).
//! * an assistant message's `tool_calls` serialize as
//!   `[{"id","type":"function","function":{"name","arguments"}}]`, so a
//!   grown agent conversation replays on the gateway verbatim.
//! * streaming fragments (`delta.tool_calls[]`) are parsed by the client
//!   into [`crate::ChatEvent::ToolCallDelta`] events and reassembled by
//!   [`ToolCallAccumulator`] — the same accumulate-then-parse pattern as
//!   content deltas. The event channel plus the accumulator are the whole
//!   contract with the Phase 5c loop: `ChatEvent` and `ChatReceipt` are
//!   shared with `selora-app`/`selora-ui` and stay exactly as the prep
//!   defined them, so the loop reads the tool_calls-vs-stop distinction
//!   off the drain (non-empty ⇔ `finish_reason == "tool_calls"`).
//!
//! The manifest itself is NOT defined here: it is fixed by
//! ARCHITECTURE.md §17.1 and lives in the agent module
//! ([`crate::agent::tool_manifest`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::types::ChatEvent;

/// A tool call's id (the gateway's string; echoed back in the result
/// message's `tool_call_id`).
pub type ToolCallId = String;

/// One entry in the tool manifest sent with every agent request
/// (ARCHITECTURE.md §17.1). `parameters` is a JSON-Schema object — kept
/// as raw `serde_json::Value` so the manifest stays declarative.
///
/// Wire shape: the OpenAI-compatible `tools` array entry,
/// `{"type":"function","function":{"name":…,"description":…,"parameters":…}}`.
/// The `type` discriminator is ignored on the way in — we only ever send
/// `"function"` tools.
#[derive(Debug, Clone, PartialEq)]
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

impl Serialize for ToolSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct WireFunctionRef<'a> {
            name: &'a str,
            description: &'a str,
            parameters: &'a serde_json::Value,
        }
        #[derive(Serialize)]
        struct WireToolRef<'a> {
            r#type: &'static str,
            function: WireFunctionRef<'a>,
        }
        WireToolRef {
            r#type: "function",
            function: WireFunctionRef {
                name: &self.name,
                description: &self.description,
                parameters: &self.parameters,
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ToolSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireFunction {
            name: String,
            #[serde(default)]
            description: String,
            parameters: serde_json::Value,
        }
        #[derive(Deserialize)]
        struct WireTool {
            function: WireFunction,
        }
        let wire = WireTool::deserialize(deserializer)?;
        Ok(Self {
            name: wire.function.name,
            description: wire.function.description,
            parameters: wire.function.parameters,
        })
    }
}

/// The model's invocation of one tool (a `tool_calls` entry of an
/// assistant message, or one parsed product of the delta accumulator).
///
/// `arguments` is the raw JSON object string — parsed against the tool's
/// schema by the router (Phase 5b), never by the wire layer.
///
/// Wire shape: the OpenAI-compatible `tool_calls` entry,
/// `{"id":…,"type":"function","function":{"name":…,"arguments":…}}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    /// The raw arguments JSON string (may stream in fragments).
    pub arguments: String,
}

impl Serialize for ToolCall {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct WireFunctionRef<'a> {
            name: &'a str,
            arguments: &'a str,
        }
        #[derive(Serialize)]
        struct WireCallRef<'a> {
            id: &'a str,
            r#type: &'static str,
            function: WireFunctionRef<'a>,
        }
        WireCallRef {
            id: &self.id,
            r#type: "function",
            function: WireFunctionRef {
                name: &self.name,
                arguments: &self.arguments,
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ToolCall {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireFunction {
            #[serde(default)]
            name: Option<String>,
            #[serde(default, deserialize_with = "tolerant_arguments")]
            arguments: String,
        }
        #[derive(Deserialize)]
        struct WireCall {
            id: String,
            #[serde(default)]
            function: Option<WireFunction>,
        }
        let wire = WireCall::deserialize(deserializer)?;
        let function = wire.function;
        Ok(Self {
            id: wire.id,
            name: function
                .as_ref()
                .and_then(|function| function.name.clone())
                .unwrap_or_default(),
            arguments: function
                .map(|function| function.arguments)
                .unwrap_or_default(),
        })
    }
}

/// The `arguments` field is a JSON *string* on the OpenAI wire, but some
/// gateways emit the already-parsed object (or `null`) on non-streaming
/// replies. Both fold back into the raw string form — the router always
/// sees a string.
fn tolerant_arguments<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(text) => text,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
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

/// Folds streaming [`ChatEvent::ToolCallDelta`] fragments into complete
/// [`ToolCall`]s — the tool-call twin of the content-delta accumulate
/// loop (ARCHITECTURE.md §17.6).
///
/// One accumulator per streamed request: `feed` (or `push`) each event as
/// it arrives, then [`finish`](Self::finish) once
/// [`ChatEvent::Done`](crate::ChatEvent::Done) lands. A non-empty drain
/// is the stream's `finish_reason == "tool_calls"` equivalent — the
/// Phase 5c loop runs the drained calls in order and the text deltas are
/// (at most) running commentary; an empty drain means
/// `finish_reason == "stop"` and the text deltas are the final answer.
///
/// Gateway quirks tolerated (asserted in the tests below):
/// * `arguments` split at arbitrary byte boundaries — fragments
///   concatenate in arrival order, per call index;
/// * a missing `index` on single-call responses — the client maps those
///   fragments to index 0 before the event is emitted;
/// * id and name arriving only on the first fragment — first non-empty
///   value wins, later repeats do not corrupt or duplicate;
/// * empty trailing fragments — no-ops;
/// * a call that never got an id — a deterministic `call_{index}` is
///   synthesized so the tool-result message still round-trips.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCallAccumulator {
    /// Partial calls keyed by wire `index`; a `BTreeMap` so [`finish`](Self::finish)
    /// drains in index order (the gateway's call order).
    slots: BTreeMap<usize, PartialToolCall>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    /// An empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one streamed event: `ToolCallDelta` fragments accumulate,
    /// every other variant is ignored — the loop can feed the whole
    /// channel blindly.
    pub fn feed(&mut self, event: &ChatEvent) {
        if let ChatEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments,
        } = event
        {
            self.push(*index, id.clone(), name.clone(), arguments);
        }
    }

    /// Push one raw fragment: the fields of a `delta.tool_calls[]` entry,
    /// with the index already resolved (`None` → 0 by the client).
    pub fn push(
        &mut self,
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: &str,
    ) {
        let id = id.filter(|id| !id.is_empty());
        let name = name.filter(|name| !name.is_empty());
        if id.is_none() && name.is_none() && arguments.is_empty() {
            return; // empty trailing fragment
        }
        let slot = self.slots.entry(index).or_default();
        if slot.id.is_empty() {
            slot.id = id.unwrap_or_default();
        }
        if slot.name.is_empty() {
            slot.name = name.unwrap_or_default();
        }
        slot.arguments.push_str(arguments);
    }

    /// Drain the accumulated calls in wire-index order (the gateway's
    /// call order). Call this once the stream finishes.
    ///
    /// A slot with no id, no name, and no arguments never existed as far
    /// as the model was concerned and is dropped; an id-less call that
    /// did carry payload gets a synthesized `call_{index}` id; a call
    /// with payload but no name is kept (name `""`) so the router fails
    /// it visibly instead of the wire layer silently dropping it.
    pub fn finish(self) -> Vec<ToolCall> {
        self.slots
            .into_iter()
            .filter(|(_, slot)| {
                !(slot.id.is_empty() && slot.name.is_empty() && slot.arguments.is_empty())
            })
            .map(|(index, slot)| ToolCall {
                id: if slot.id.is_empty() {
                    format!("call_{index}")
                } else {
                    slot.id
                },
                name: slot.name,
                arguments: slot.arguments,
            })
            .collect()
    }

    /// Whether any meaningful fragment has arrived.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, ChatRequest};

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

    #[test]
    fn tool_spec_serializes_to_the_openai_function_shape() {
        let spec = ToolSpec::new(
            "read_file",
            "Read a workspace file",
            serde_json::json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
        );
        assert_eq!(
            serde_json::to_value(&spec).unwrap(),
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a workspace file",
                    "parameters": { "type": "object", "properties": { "path": { "type": "string" } } }
                }
            })
        );
        // The `type` discriminator is ignored on the way in.
        let back: ToolSpec = serde_json::from_value(serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a workspace file",
                "parameters": { "type": "object" }
            }
        }))
        .unwrap();
        assert_eq!(
            back,
            ToolSpec::new(
                "read_file",
                "Read a workspace file",
                serde_json::json!({ "type": "object" })
            )
        );
    }

    #[test]
    fn tool_call_serializes_to_the_openai_function_shape() {
        let call = ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"src/main.rs"}"#.into(),
        };
        assert_eq!(
            serde_json::to_value(&call).unwrap(),
            serde_json::json!({
                "id": "call_1",
                "type": "function",
                "function": { "name": "read_file", "arguments": "{\"path\":\"src/main.rs\"}" }
            })
        );

        // Gateway shape parses back: the `type` field is ignored and an
        // arguments-as-object quirk (some gateways, non-streaming replies)
        // folds into the string form.
        let parsed: ToolCall = serde_json::from_value(serde_json::json!({
            "id": "call_1",
            "type": "function",
            "function": { "name": "read_file", "arguments": { "path": "src/main.rs" } }
        }))
        .unwrap();
        assert_eq!(parsed, call);

        // Null arguments (tool-call-only reply, content-less) is an empty
        // string, not an error.
        let parsed: ToolCall = serde_json::from_value(serde_json::json!({
            "id": "call_2",
            "type": "function",
            "function": { "name": "list_dir", "arguments": null }
        }))
        .unwrap();
        assert_eq!(parsed.arguments, "");
    }

    #[test]
    fn assistant_message_with_tool_calls_round_trips() {
        // Deliverable: a grown agent conversation replays on the gateway.
        let mut message = ChatMessage::assistant("Reading both now.");
        message.tool_calls = Some(vec![
            ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            },
            ToolCall {
                id: "call_2".into(),
                name: "list_dir".into(),
                arguments: r#"{"path":"."}"#.into(),
            },
        ]);
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"], "Reading both now.");
        assert_eq!(json["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(json["tool_calls"][1]["id"], "call_2");
        let back: ChatMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, message);
    }

    #[test]
    fn chat_request_tools_are_on_the_wire_only_when_armed() {
        let plain = ChatRequest::new("m", vec![ChatMessage::user("hi")]);
        let json = serde_json::to_value(&plain).unwrap();
        assert!(
            json.get("tools").is_none(),
            "plain chat must not arm tools: {json}"
        );

        let armed = plain.with_tools(vec![ToolSpec::new(
            "read_file",
            "Read a workspace file",
            serde_json::json!({ "type": "object" }),
        )]);
        let json = serde_json::to_value(&armed).unwrap();
        assert_eq!(
            json["tools"],
            serde_json::json!([
                {
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "description": "Read a workspace file",
                        "parameters": { "type": "object" }
                    }
                }
            ])
        );
    }

    // ---- ToolCallAccumulator ----

    #[test]
    fn accumulator_reassembles_arguments_split_mid_json() {
        let mut accumulator = ToolCallAccumulator::new();
        accumulator.push(
            0,
            Some("call_1".into()),
            Some("write_file".into()),
            r#"{"path""#,
        );
        accumulator.push(0, None, None, r#":"src/li"#);
        accumulator.push(0, None, None, r#"b.rs","content":"// header\n"}"#);
        assert!(!accumulator.is_empty());
        assert_eq!(
            accumulator.finish(),
            vec![ToolCall {
                id: "call_1".into(),
                name: "write_file".into(),
                arguments: r#"{"path":"src/lib.rs","content":"// header\n"}"#.into(),
            }]
        );
    }

    #[test]
    fn accumulator_tracks_parallel_calls_by_index_in_order() {
        let mut accumulator = ToolCallAccumulator::new();
        accumulator.push(
            1,
            Some("call_b".into()),
            Some("list_dir".into()),
            r#"{"path""#,
        );
        accumulator.push(
            0,
            Some("call_a".into()),
            Some("read_file".into()),
            r#"{"path":"a.rs"}"#,
        );
        accumulator.push(1, None, None, r#":"."}"#);
        assert_eq!(
            accumulator.finish(),
            vec![
                ToolCall {
                    id: "call_a".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path":"a.rs"}"#.into(),
                },
                ToolCall {
                    id: "call_b".into(),
                    name: "list_dir".into(),
                    arguments: r#"{"path":"."}"#.into(),
                },
            ]
        );
    }

    #[test]
    fn accumulator_ignores_empty_trailing_fragments() {
        // Only empty fragments: nothing accumulates.
        let mut empty = ToolCallAccumulator::new();
        assert!(empty.is_empty());
        empty.push(0, None, None, "");
        empty.push(3, None, None, "");
        assert!(empty.is_empty());
        assert!(empty.finish().is_empty());

        // And after a real call they change nothing.
        let mut accumulator = ToolCallAccumulator::new();
        accumulator.push(0, Some("call_1".into()), Some("read_file".into()), "");
        accumulator.push(0, None, None, "");
        assert_eq!(
            accumulator.finish(),
            vec![ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: String::new(),
            }]
        );
    }

    #[test]
    fn accumulator_first_id_and_name_win() {
        let mut accumulator = ToolCallAccumulator::new();
        accumulator.push(0, Some("call_1".into()), Some("read_file".into()), r#"{"p"#);
        // Repeated (or divergent) later ids and names must not corrupt.
        accumulator.push(
            0,
            Some("call_wrong".into()),
            Some("read_file".into()),
            r#"ath":"a"}"#,
        );
        assert_eq!(
            accumulator.finish(),
            vec![ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"a"}"#.into(),
            }]
        );
    }

    #[test]
    fn accumulator_synthesizes_ids_for_id_less_calls() {
        let mut accumulator = ToolCallAccumulator::new();
        // Gateways that omit ids entirely: a deterministic id is
        // synthesized so the tool-result message still round-trips.
        accumulator.push(0, None, Some("read_file".into()), r#"{"path":"a.rs"}"#);
        accumulator.push(1, None, Some("list_dir".into()), r#"{"path":"."}"#);
        assert_eq!(
            accumulator.finish(),
            vec![
                ToolCall {
                    id: "call_0".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path":"a.rs"}"#.into(),
                },
                ToolCall {
                    id: "call_1".into(),
                    name: "list_dir".into(),
                    arguments: r#"{"path":"."}"#.into(),
                },
            ]
        );
    }

    #[test]
    fn accumulator_feeds_from_chat_events_and_ignores_the_rest() {
        let mut accumulator = ToolCallAccumulator::new();
        accumulator.feed(&ChatEvent::Delta("text".into()));
        accumulator.feed(&ChatEvent::Error(selora_core::SeloraError::Ai("x".into())));
        assert!(accumulator.is_empty());

        accumulator.feed(&ChatEvent::ToolCallDelta {
            index: 0,
            id: Some("call_1".into()),
            name: Some("read_file".into()),
            arguments: r#"{"path":"a.rs"}"#.into(),
        });
        accumulator.feed(&ChatEvent::ToolCallDelta {
            index: 0,
            id: None,
            name: None,
            arguments: String::new(),
        });
        accumulator.feed(&ChatEvent::Delta("more text".into()));
        assert_eq!(
            accumulator.finish(),
            vec![ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            }]
        );
    }
}
