//! Phase 9 session slice — the authoritative todo vocabulary.
//!
//! `manage_todo` used to echo its arguments back as success while storing
//! nothing. This module gives the tool a validated shape and a store seam:
//! the desktop session engine supplies a [`TodoSink`] bound to one session,
//! the router writes through it, and the tool result reports what was
//! actually stored. A context without a sink answers a typed error instead
//! of pretending success (workflow subagents and tests have no session todo
//! list).
//!
//! Bounds are deliberately small: a plan the user cannot scan is not a plan.
//! `id` is required and unique within the session so a status update names
//! exactly one task; unknown fields are ignored for forward compatibility.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// At most this many todos per session. A larger list is rejected (not
/// truncated) so the model gets a correctable error instead of a silently
/// shortened plan.
pub const MAX_TODOS: usize = 50;

/// At most this many characters of task text per todo.
pub const MAX_TODO_TEXT_CHARS: usize = 500;

/// At most this many characters of id per todo.
pub const MAX_TODO_ID_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}

/// One validated todo. The desktop maps this to the protocol's `TodoDto`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub text: String,
    pub status: TodoStatus,
}

/// The session-scoped store behind `manage_todo`. `store` replaces the whole
/// list atomically (the tool's contract is a full plan write); `load` returns
/// the authoritative list so the tool result can be truthful.
pub trait TodoSink: Send + Sync {
    fn load(&self) -> Result<Vec<TodoItem>, String>;
    fn store(&self, todos: Vec<TodoItem>) -> Result<(), String>;
}

/// Parse the `todos` argument. Accepted forms:
///
/// - a JSON array of `{ "id", "text", "status" }` objects;
/// - a string containing that array (some models stringify JSON arguments).
///
/// Anything else is an error naming the expected shape. Validation is strict:
/// a missing/blank id or text, an unknown status, a duplicate id, or a bound
/// violation rejects the whole write, so a partially-applied plan cannot
/// exist.
pub fn parse_todos(raw: &Value) -> Result<Vec<TodoItem>, String> {
    let value = match raw {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .map_err(|_| "`todos` must be a JSON array or a string containing one".to_string())?,
        other => other.clone(),
    };
    let Value::Array(items) = value else {
        return Err("`todos` must be a JSON array of {id, text, status} objects".into());
    };
    if items.len() > MAX_TODOS {
        return Err(format!(
            "`todos` has {} items; the bound is {MAX_TODOS}",
            items.len()
        ));
    }
    let mut todos = Vec::with_capacity(items.len());
    let mut seen = std::collections::BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let Value::Object(object) = item else {
            return Err(format!("`todos[{index}]` must be an object"));
        };
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("`todos[{index}].id` must be a non-empty string"))?;
        if id.chars().count() > MAX_TODO_ID_CHARS {
            return Err(format!(
                "`todos[{index}].id` is longer than {MAX_TODO_ID_CHARS} characters"
            ));
        }
        if !seen.insert(id.to_owned()) {
            return Err(format!("`todos[{index}].id` duplicates `{id}`"));
        }
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| format!("`todos[{index}].text` must be a non-empty string"))?;
        if text.chars().count() > MAX_TODO_TEXT_CHARS {
            return Err(format!(
                "`todos[{index}].text` is longer than {MAX_TODO_TEXT_CHARS} characters"
            ));
        }
        let raw_status = object
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("`todos[{index}].status` must be a string"))?;
        let status = TodoStatus::parse(raw_status).ok_or_else(|| {
            format!(
                "`todos[{index}].status` is `{raw_status}`; expected pending, in_progress, or completed"
            )
        })?;
        todos.push(TodoItem {
            id: id.to_owned(),
            text: text.to_owned(),
            status,
        });
    }
    Ok(todos)
}

/// The truthful tool result: the list as stored, not the model's raw input.
pub fn stored_result(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return "Stored 0 todos: the session plan is now empty.".into();
    }
    let mut out = format!("Stored {} todo(s):", todos.len());
    for todo in todos {
        out.push_str(&format!(
            "\n- [{}] {} (id: {})",
            todo.status.as_str(),
            todo.text,
            todo.id
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_the_canonical_array_and_reports_store_order() {
        let raw = json!([
            { "id": "t1", "text": "inspect router", "status": "completed" },
            { "id": "t2", "text": "write tests", "status": "in_progress" },
        ]);
        let todos = parse_todos(&raw).expect("valid");
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].status, TodoStatus::Completed);
        assert_eq!(todos[1].status, TodoStatus::InProgress);
        let result = stored_result(&todos);
        assert!(result.starts_with("Stored 2 todo(s):"));
        assert!(result.contains("[completed] inspect router (id: t1)"));
        assert!(result.contains("[in_progress] write tests (id: t2)"));
    }

    #[test]
    fn accepts_a_stringified_json_array() {
        let raw = json!(r#"[{"id":"a","text":"step","status":"pending"}]"#);
        let todos = parse_todos(&raw).expect("stringified array");
        assert_eq!(todos[0].id, "a");
    }

    #[test]
    fn rejects_malformed_input() {
        for raw in [
            json!("not json"),
            json!({ "id": "t", "text": "x", "status": "pending" }),
            json!([{ "text": "no id", "status": "pending" }]),
            json!([{ "id": "t", "status": "pending" }]),
            json!([{ "id": "t", "text": "x" }]),
            json!([{ "id": "t", "text": "x", "status": "doing" }]),
            json!([{ "id": "  ", "text": "x", "status": "pending" }]),
            json!([{ "id": "t", "text": "  ", "status": "pending" }]),
            json!([{ "id": 7, "text": "x", "status": "pending" }]),
            json!([null]),
        ] {
            assert!(parse_todos(&raw).is_err(), "accepted {raw}");
        }
    }

    #[test]
    fn rejects_duplicate_ids_and_over_cap_lists() {
        let duplicate = json!([
            { "id": "same", "text": "one", "status": "pending" },
            { "id": "same", "text": "two", "status": "pending" },
        ]);
        assert!(parse_todos(&duplicate).unwrap_err().contains("duplicates"));

        let over: Vec<Value> = (0..=MAX_TODOS)
            .map(|index| json!({ "id": format!("t{index}"), "text": "x", "status": "pending" }))
            .collect();
        let error = parse_todos(&Value::Array(over)).unwrap_err();
        assert!(error.contains(&MAX_TODOS.to_string()), "{error}");
    }

    #[test]
    fn rejects_over_long_text() {
        let raw = json!([{
            "id": "t",
            "text": "x".repeat(MAX_TODO_TEXT_CHARS + 1),
            "status": "pending"
        }]);
        assert!(parse_todos(&raw).unwrap_err().contains("longer than"));
    }

    #[test]
    fn empty_list_is_a_valid_clear() {
        assert!(parse_todos(&json!([])).expect("clear").is_empty());
        assert!(stored_result(&[]).contains("Stored 0 todos"));
    }
}
