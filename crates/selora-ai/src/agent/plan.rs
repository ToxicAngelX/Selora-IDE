//! Session-scoped plan mode and live todos (ROADMAP Phase 9b).
//!
//! Two things live here, both bounded and validated in one place so the
//! router, the loop and the desktop hydration path cannot disagree:
//!
//! - **Plan mode** ([`PlanSession`]): a session starts in
//!   [`PlanPhase::Executing`] (normal agent behavior). `enter_plan_mode`
//!   flips it to [`PlanPhase::Planning`]; while planning, the router
//!   structurally refuses every side-effecting tool (writes, deletes,
//!   `bash`, `run_workflow`) before it can reach disk or spawn. A plan
//!   leaves plan mode only through `exit_plan_mode`, whose card the user
//!   approves. Approving that card flips the session back to execution;
//!   denying or revising keeps it planning. This adds **no** trust
//!   primitive: after approval every write still cards exactly as before.
//! - **Todos** ([`TodoItem`]): a bounded, validated list stored in the
//!   session. `todo_write` replaces it, `todo_read` returns it, and
//!   `manage_todo` remains as an alias. Nothing reports success unless the
//!   list was actually stored (the router stores first, then renders).
//!
//! The store is an `Arc<Mutex<..>>` so one session's desktop record and the
//! router's [`ToolContext`](super::ToolContext) share the same state: a
//! tool call mutates it, the chat snapshot hydrates it after a reload, and
//! the loop's additive events carry it live.

use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

/// The session's mode. `Executing` is the pre-9b behavior and the serde
/// default, so every existing session hydrates as execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanPhase {
    Planning,
    #[default]
    Executing,
}

/// One live todo item. Ids are assigned by the store (`todo-1`, `todo-2`,
/// …) so the model cannot fabricate stable ids and the UI can key rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub status: TodoStatus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

/// A proposed plan. `summary` is optional; `steps` is never empty once
/// validated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub steps: Vec<String>,
}

/// The session's plan/todo record. `plan` is the *approved* plan (or the
/// one awaiting approval once the card exists).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSession {
    #[serde(default)]
    pub phase: PlanPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanDraft>,
    #[serde(default)]
    pub todos: Vec<TodoItem>,
}

/// A copy of the store for events/snapshots.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSnapshot {
    pub phase: PlanPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanDraft>,
    #[serde(default)]
    pub todos: Vec<TodoItem>,
}

/// The shared session store.
pub type PlanStore = Arc<Mutex<PlanSession>>;

/// A fresh session store in execution mode with no todos.
pub fn new_store() -> PlanStore {
    Arc::new(Mutex::new(PlanSession::default()))
}

/// Lock a store, recovering from a poisoned mutex: a panic in a tool must
/// not wedge the rest of the session's plan state (the data is still
/// structurally valid — every mutation goes through these validated paths).
pub fn lock(store: &PlanStore) -> MutexGuard<'_, PlanSession> {
    store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Read a copy of the store.
pub fn snapshot(store: &PlanStore) -> PlanSnapshot {
    let session = lock(store);
    PlanSnapshot {
        phase: session.phase,
        plan: session.plan.clone(),
        todos: session.todos.clone(),
    }
}

/// `true` while the session is planning (the structural gate).
pub fn is_planning(store: &PlanStore) -> bool {
    lock(store).phase == PlanPhase::Planning
}

/// The tools a planning session may not run. `bash` is included even when
/// a command would be allowlisted: a shell is a side effect we cannot
/// prove read-only, and plan mode's promise is structural, not textual.
pub fn is_side_effecting_tool(name: &str) -> bool {
    matches!(
        name,
        "write_file" | "edit_file" | "delete_file" | "bash" | "run_workflow"
    )
}

/// The model-visible refusal while planning. It names the one action that
/// unblocks the session (approving the plan) so the model can adapt.
pub const PLAN_MODE_REJECTION: &str = "plan mode is active: approve the plan first \
     (call exit_plan_mode and wait for the user's approval); reads and todo tools still work";

/// The exact refusal for a specific tool, as a model-visible error body.
pub fn plan_mode_rejection(tool: &str) -> String {
    format!("`{tool}` blocked — {PLAN_MODE_REJECTION}")
}

// ---------------------------------------------------------------------------
// Bounds (one place, mirrored by the manifest descriptions)
// ---------------------------------------------------------------------------

/// A session's todo list cap. Large enough for a real plan, small enough
/// that every write stays reviewable.
pub const MAX_TODOS: usize = 64;
/// One todo title's byte cap.
pub const MAX_TODO_TITLE_BYTES: usize = 500;
/// The approved plan's step cap.
pub const MAX_PLAN_STEPS: usize = 32;
/// One plan step's byte cap.
pub const MAX_PLAN_STEP_BYTES: usize = 500;
/// The optional plan summary's byte cap.
pub const MAX_PLAN_SUMMARY_BYTES: usize = 2000;

// ---------------------------------------------------------------------------
// Validation (the router's argument boundary)
// ---------------------------------------------------------------------------

/// Parse and validate a `todo_write`/`manage_todo` list. An empty array is
/// a legitimate clear; anything malformed is a model-visible error.
pub fn parse_todos(
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<TodoItem>, String> {
    let raw = args
        .get("todos")
        .ok_or_else(|| "todo_write requires an array parameter `todos`".to_owned())?;
    parse_todo_value(raw)
}

/// Parse an already-extracted todo value (the alias may hand a JSON string).
pub fn parse_todo_value(raw: &serde_json::Value) -> Result<Vec<TodoItem>, String> {
    let items = raw
        .as_array()
        .ok_or_else(|| "`todos` must be an array of { title, status? } objects".to_owned())?;
    if items.len() > MAX_TODOS {
        return Err(format!(
            "`todos` has {} entries — the cap is {MAX_TODOS}",
            items.len()
        ));
    }
    let mut todos = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| format!("`todos[{index}]` must be an object with a `title`"))?;
        let title = object
            .get("title")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("`todos[{index}].title` must be a string"))?
            .trim();
        if title.is_empty() {
            return Err(format!("`todos[{index}].title` must not be empty"));
        }
        if title.len() > MAX_TODO_TITLE_BYTES {
            return Err(format!(
                "`todos[{index}].title` is {} bytes — the cap is {MAX_TODO_TITLE_BYTES}",
                title.len()
            ));
        }
        if title.chars().any(|c| c == '\n' || c == '\r' || c == '\0') {
            return Err(format!(
                "`todos[{index}].title` must be a single line (no newlines or NUL)"
            ));
        }
        let status = match object.get("status") {
            None | Some(serde_json::Value::Null) => TodoStatus::Pending,
            Some(value) => parse_status(value).ok_or_else(|| {
                format!(
                    "`todos[{index}].status` must be one of `pending`, `in_progress`, `completed`"
                )
            })?,
        };
        todos.push(TodoItem {
            id: format!("todo-{}", index + 1),
            title: title.to_owned(),
            status,
        });
    }
    Ok(todos)
}

fn parse_status(value: &serde_json::Value) -> Option<TodoStatus> {
    match value.as_str()? {
        "pending" => Some(TodoStatus::Pending),
        "in_progress" => Some(TodoStatus::InProgress),
        "completed" => Some(TodoStatus::Completed),
        _ => None,
    }
}

/// Parse and validate an `exit_plan_mode` argument set. The plan is the
/// required `plan` string array; `summary` is optional context.
pub fn parse_plan(args: &serde_json::Map<String, serde_json::Value>) -> Result<PlanDraft, String> {
    let steps = args
        .get("plan")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            "exit_plan_mode requires an array parameter `plan` of step strings".to_owned()
        })?;
    if steps.is_empty() {
        return Err(
            "`plan` must contain at least one step — an empty plan is not a plan".to_owned(),
        );
    }
    if steps.len() > MAX_PLAN_STEPS {
        return Err(format!(
            "`plan` has {} steps — the cap is {MAX_PLAN_STEPS}",
            steps.len()
        ));
    }
    let mut validated = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let text = step
            .as_str()
            .ok_or_else(|| format!("`plan[{index}]` must be a string step"))?
            .trim();
        if text.is_empty() {
            return Err(format!("`plan[{index}]` must not be empty"));
        }
        if text.len() > MAX_PLAN_STEP_BYTES {
            return Err(format!(
                "`plan[{index}]` is {} bytes — the cap is {MAX_PLAN_STEP_BYTES}",
                text.len()
            ));
        }
        if text.chars().any(|c| c == '\n' || c == '\r' || c == '\0') {
            return Err(format!("`plan[{index}]` must be a single line"));
        }
        validated.push(text.to_owned());
    }
    let summary = match args.get("summary") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(summary)) => {
            let summary = summary.trim();
            if summary.is_empty() {
                None
            } else if summary.len() > MAX_PLAN_SUMMARY_BYTES {
                return Err(format!(
                    "`summary` is {} bytes — the cap is {MAX_PLAN_SUMMARY_BYTES}",
                    summary.len()
                ));
            } else {
                Some(summary.to_owned())
            }
        }
        Some(_) => return Err("`summary` must be a string".to_owned()),
    };
    Ok(PlanDraft {
        summary,
        steps: validated,
    })
}

// ---------------------------------------------------------------------------
// Rendering (the tool result the model sees)
// ---------------------------------------------------------------------------

/// The stored list as a stable, model-visible rendering.
pub fn render_todos(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return "No todos are stored for this session.".to_owned();
    }
    let mut out = String::from("Stored todos (session-scoped):\n");
    for (index, todo) in todos.iter().enumerate() {
        let status = match todo.status {
            TodoStatus::Pending => "pending",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Completed => "completed",
        };
        out.push_str(&format!(
            "{}. [{}] {} (id: {})\n",
            index + 1,
            status,
            todo.title,
            todo.id
        ));
    }
    out
}

/// Seed the approved plan's steps as todos only when the session has none.
/// An approved plan is a user-reviewed step list, never a fabricated one.
pub fn seed_todos_from_plan(session: &mut PlanSession, plan: &PlanDraft) {
    if !session.todos.is_empty() {
        return;
    }
    session.todos = plan
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| TodoItem {
            id: format!("todo-{}", index + 1),
            title: step.clone(),
            status: TodoStatus::Pending,
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        value.as_object().expect("object").clone()
    }

    #[test]
    fn todos_validate_and_assign_stable_ids() {
        let todos = parse_todos(&args(json!({
            "todos": [
                { "title": "Read the router", "status": "completed" },
                { "title": "  Write the plan  " },
                { "title": "Apply", "status": "in_progress" },
            ]
        })))
        .expect("valid list");
        assert_eq!(todos.len(), 3);
        assert_eq!(todos[0].id, "todo-1");
        assert_eq!(todos[1].title, "Write the plan");
        assert_eq!(todos[1].status, TodoStatus::Pending);
        assert_eq!(todos[2].status, TodoStatus::InProgress);
        assert!(render_todos(&todos).contains("[completed] Read the router"));
    }

    #[test]
    fn todos_reject_malformed_or_unbounded_input() {
        let cases = [
            json!({}),
            json!({ "todos": "not an array" }),
            json!({ "todos": ["plain string"] }),
            json!({ "todos": [{ "title": 7 }] }),
            json!({ "todos": [{ "title": "  " }] }),
            json!({ "todos": [{ "title": "a\nb" }] }),
            json!({ "todos": [{ "title": "x".repeat(MAX_TODO_TITLE_BYTES + 1) }] }),
            json!({ "todos": [{ "title": "ok", "status": "done" }] }),
            json!({ "todos": (0..=MAX_TODOS).map(|i| json!({"title": format!("t{i}")})).collect::<Vec<_>>() }),
        ];
        for case in cases {
            assert!(
                parse_todos(&args(case.clone())).is_err(),
                "must reject {case}"
            );
        }
    }

    #[test]
    fn empty_todo_list_is_a_valid_clear() {
        assert!(parse_todos(&args(json!({ "todos": [] })))
            .expect("clear is valid")
            .is_empty());
    }

    #[test]
    fn plan_validates_steps_and_summary() {
        let plan = parse_plan(&args(json!({
            "plan": [" Read ", "Edit", "Test"],
            "summary": "  Add the tool  ",
        })))
        .expect("valid plan");
        assert_eq!(plan.steps, ["Read", "Edit", "Test"]);
        assert_eq!(plan.summary.as_deref(), Some("Add the tool"));

        for case in [
            json!({}),
            json!({ "plan": [] }),
            json!({ "plan": "one step" }),
            json!({ "plan": [7] }),
            json!({ "plan": ["  "] }),
            json!({ "plan": ["multi\nline"] }),
            json!({ "plan": ["x".repeat(MAX_PLAN_STEP_BYTES + 1)] }),
            json!({ "plan": (0..=MAX_PLAN_STEPS).map(|i| json!(format!("s{i}"))).collect::<Vec<_>>() }),
            json!({ "plan": ["ok"], "summary": 7 }),
            json!({ "plan": ["ok"], "summary": "x".repeat(MAX_PLAN_SUMMARY_BYTES + 1) }),
        ] {
            assert!(
                parse_plan(&args(case.clone())).is_err(),
                "must reject {case}"
            );
        }
    }

    #[test]
    fn plan_approval_seeds_empty_todos_only() {
        let plan = PlanDraft {
            summary: None,
            steps: vec!["one".into(), "two".into()],
        };
        let mut session = PlanSession::default();
        seed_todos_from_plan(&mut session, &plan);
        assert_eq!(session.todos.len(), 2);
        assert_eq!(session.todos[1].status, TodoStatus::Pending);

        let mut existing = PlanSession {
            todos: vec![TodoItem {
                id: "todo-1".into(),
                title: "keep me".into(),
                status: TodoStatus::Completed,
            }],
            ..PlanSession::default()
        };
        seed_todos_from_plan(&mut existing, &plan);
        assert_eq!(
            existing.todos.len(),
            1,
            "existing todos are never clobbered"
        );
    }

    #[test]
    fn side_effecting_tools_are_the_structural_gate() {
        for tool in [
            "write_file",
            "edit_file",
            "delete_file",
            "bash",
            "run_workflow",
        ] {
            assert!(is_side_effecting_tool(tool), "{tool} must be gated");
        }
        for tool in [
            "read_file",
            "list_dir",
            "fuzzy_find",
            "search_files",
            "web_search",
            "web_fetch",
            "enter_plan_mode",
            "exit_plan_mode",
            "todo_read",
            "todo_write",
            "manage_todo",
        ] {
            assert!(!is_side_effecting_tool(tool), "{tool} must stay usable");
        }
    }
}
