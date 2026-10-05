//! JSON-RPC 2.0 message classification (the payload inside the framing).
//!
//! One place decides what an incoming `Value` *is* — a response to one of
//! our requests, a request the server wants answered, or a notification —
//! and one place builds the three shapes we send. Keeping this as pure
//! functions over [`serde_json::Value`] (rather than a serde enum) is
//! deliberate: LSP servers in the wild attach extra fields, and the
//! protocol's request/response dichotomy is two `Option`s in the wire
//! format, not a discriminated union.

use serde_json::{json, Value};

/// A JSON-RPC request id, as servers send them back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RpcId {
    Number(i64),
    String(String),
}

impl RpcId {
    /// The id as the wire `Value` (its own type on the wire).
    pub fn to_value(&self) -> Value {
        match self {
            Self::Number(n) => json!(n),
            Self::String(s) => json!(s),
        }
    }
}

/// A JSON-RPC error object (a response's `error` member).
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    /// The message the app surfaces (PLAN.md §8: no swallowed errors).
    pub fn describe(&self) -> String {
        format!("code {}: {}", self.code, self.message)
    }
}

/// What a server sent us, sorted into the three protocol shapes.
#[derive(Debug, Clone)]
pub enum Incoming {
    /// A response to a request we sent. LSP guarantees exactly one of
    /// `result` / `error`; a response with `id` and neither is treated as
    /// `Ok(null)` (the permissive read, matching other clients).
    Response {
        id: RpcId,
        outcome: Result<Value, RpcError>,
    },
    /// A request the server wants answered (`id` + `method`).
    ServerRequest {
        id: RpcId,
        method: String,
        params: Value,
    },
    /// A notification (`method`, no `id`).
    Notification { method: String, params: Value },
}

/// Classify one incoming message. `None` = the value has neither an `id` nor
/// a `method` (not JSON-RPC); callers drop it rather than fail the server.
pub fn classify(value: Value) -> Option<Incoming> {
    let object = value.as_object()?;
    let id = object.get("id").and_then(parse_id);
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match (id, method) {
        (Some(id), Some(method)) => Some(Incoming::ServerRequest {
            id,
            method,
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(id), None) => {
            let outcome = if let Some(error) = object.get("error").and_then(Value::as_object) {
                Err(RpcError {
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
            } else {
                Ok(object.get("result").cloned().unwrap_or(Value::Null))
            };
            Some(Incoming::Response { id, outcome })
        }
        (None, Some(method)) => Some(Incoming::Notification {
            method,
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, None) => None,
    }
}

fn parse_id(value: &Value) -> Option<RpcId> {
    match value {
        Value::Number(n) => n.as_i64().map(RpcId::Number),
        Value::String(s) => Some(RpcId::String(s.clone())),
        _ => None,
    }
}

/// A request we send (client → server). Ids are u64 counters owned by the
/// actor; `0` is reserved for the `initialize` handshake.
pub fn client_request(id: u64, method: &str, params: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

/// A notification we send (no id, no response will come).
pub fn client_notification(method: &str, params: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

/// A response to a server-initiated request.
pub fn client_response(id: &RpcId, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.to_value(),
        "result": result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_response_with_result_and_error() {
        let ok = classify(json!({"jsonrpc": "2.0", "id": 4, "result": {"x": 1}}));
        match ok {
            Some(Incoming::Response { id, outcome }) => {
                assert_eq!(id, RpcId::Number(4));
                assert_eq!(outcome.expect("ok"), json!({"x": 1}));
            }
            other => panic!("expected a response, got {other:?}"),
        }

        let error = classify(json!({
            "jsonrpc": "2.0", "id": "string-id",
            "error": {"code": -32601, "message": "method not found"}
        }));
        match error {
            Some(Incoming::Response { id, outcome }) => {
                assert_eq!(id, RpcId::String("string-id".into()));
                let error = outcome.expect_err("error");
                assert_eq!(error.code, -32601);
                assert_eq!(error.describe(), "code -32601: method not found");
            }
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn classifies_server_request_and_notification() {
        match classify(
            json!({"jsonrpc": "2.0", "id": 1, "method": "window/workDoneProgress/create", "params": {"token": "t"}}),
        ) {
            Some(Incoming::ServerRequest { id, method, .. }) => {
                assert_eq!(id, RpcId::Number(1));
                assert_eq!(method, "window/workDoneProgress/create");
            }
            other => panic!("expected a server request, got {other:?}"),
        }
        match classify(json!({"jsonrpc": "2.0", "method": "exit"})) {
            Some(Incoming::Notification { method, params }) => {
                assert_eq!(method, "exit");
                assert_eq!(params, Value::Null); // absent params read as null
            }
            other => panic!("expected a notification, got {other:?}"),
        }
    }

    #[test]
    fn non_rpc_values_are_none() {
        assert!(classify(json!("hello")).is_none());
        assert!(classify(json!({"foo": 1})).is_none());
        // A response with neither result nor error reads as Ok(null), not None.
        match classify(json!({"jsonrpc": "2.0", "id": 9})).expect("classified") {
            Incoming::Response { outcome, .. } => {
                assert_eq!(outcome.expect("null result"), Value::Null)
            }
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn outgoing_shapes_round_trip_through_classify() {
        let request = client_request(3, "textDocument/hover", &json!({"doc": 1}));
        match classify(request).expect("classified") {
            Incoming::ServerRequest { id, method, params } => {
                // The server side of the same wire shape.
                assert_eq!(id, RpcId::Number(3));
                assert_eq!(method, "textDocument/hover");
                assert_eq!(params, json!({"doc": 1}));
            }
            other => panic!("got {other:?}"),
        }

        let notification = client_notification("initialized", &json!({}));
        match classify(notification).expect("classified") {
            Incoming::Notification { method, .. } => assert_eq!(method, "initialized"),
            other => panic!("got {other:?}"),
        }

        let response = client_response(&RpcId::Number(5), Value::Null);
        match classify(response).expect("classified") {
            Incoming::Response { id, outcome } => {
                assert_eq!(id, RpcId::Number(5));
                assert_eq!(outcome.expect("ok"), Value::Null);
            }
            other => panic!("got {other:?}"),
        }
    }
}
