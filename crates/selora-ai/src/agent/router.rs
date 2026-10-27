//! The tool router (ARCHITECTURE.md §17.1, §17.2) — Phase 5b's file.
//!
//! Maps a [`ToolCall`] to an implementation over `selora-core`,
//! `selora-fs`, runtime fs, and `tokio::process`. Read-type tools
//! (`read_file`, `list_dir`, `fuzzy_find`, `search_files`) and
//! allowlisted `bash` (§17.5) execute immediately →
//! [`ToolOutcome::Ready`]. `write_file` / `edit_file` / `delete_file` /
//! non-allowlisted `bash` build an [`ApprovalCard`](super::ApprovalCard)
//! → [`ToolOutcome::NeedsApproval`] and the loop parks (§17.4) until the
//! user decides; [`execute_approved`] is the *only* path that then
//! executes them. A `Ready` outcome for a side-effecting tool is a
//! security bug — the split is tested.
//!
//! Manifest v2 (§18.4) adds `run_workflow`, whose card is the §18.1
//! confirmation moment: the script body, the model-supplied `world.run`
//! command set, and the cost cap. An approved call returns a confirmation
//! acknowledgment, not an execution result. The desktop starts the engine
//! separately through `workflow_run`; it parses the script and checks its
//! extracted commands against the supplied set before spawning. The engine
//! in `crates/selora-workflow` repeats that check before execution.
//!
//! Arguments are parsed here from the call's raw JSON-arguments string
//! against each tool's schema (§17.6: "parsed against the tool's schema
//! by the router, never by the wire layer"); malformed or missing
//! arguments become model-visible error results, as do unknown tools
//! (§17.1's manifest is closed).
//!
//! The guards themselves live beside the code they bound —
//! [`tools::fs::resolve_rooted`] (path rooting), [`tools::bash`] (env,
//! timeout, output cap) — each with its own test citing its §.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::chat_types::{ToolCall, ToolResult, ToolSpec};

use super::tools::{bash, fs, web};
use super::{plan, ApprovalCard};

/// What the router produced for one tool call: a finished result, or a
/// card the user must approve before execution proceeds.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutcome {
    /// The tool ran (read tools, auto-approved allowlisted bash) — carry
    /// the result straight into the conversation.
    Ready(ToolResult),
    /// The tool is side-effecting: build the card, park the loop
    /// (§17.4). Execution happens on the approval reply.
    NeedsApproval(ApprovalCard),
}

/// A validated `run_workflow` argument set (§18.1/§18.2): exactly what the
/// confirmation card shows and what [`execute_approved`] re-validates on
/// the approval reply. This router validates the payload, not JavaScript
/// syntax or command extraction. The desktop and `crates/selora-workflow`
/// independently extract script commands and reject any outside the grant.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowPlan {
    /// Workflow name, unique within the run.
    pub name: String,
    /// The complete script body (≤ [`WORKFLOW_SCRIPT_MAX_BYTES`]).
    pub script: String,
    /// The model-supplied `world.run` command literals, deduped in supplied
    /// order. This list is not trusted as an extraction from `script`.
    pub commands: Vec<String>,
    /// Workflow-level cost cap in USD.
    pub max_cost_usd: f64,
}

/// The router's slice of the `agent.*` settings (§17.5): the bash shell
/// and the auto-approve allowlist. Mirrors `selora-settings`'s
/// `AgentSettings` shell/auto_approve_commands fields (the loop-cap
/// fields are 5c's); the app maps its loaded settings into this at
/// ToolContext construction. Serde defaults match §17.5 so the `agent`
/// JSON object deserializes directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentToolSettings {
    /// §17.5: PowerShell — `pwsh` if installed, else `powershell`. Empty
    /// means auto-resolve; any other value is used verbatim.
    #[serde(default)]
    pub shell: String,
    /// §17.5: read-only commands that auto-run without a card. Prefix
    /// match, user-editable; defaults are §17.5's list.
    #[serde(default = "default_auto_approve_commands")]
    pub auto_approve_commands: Vec<String>,
    /// §17.1 `web_search`: the optional Brave Search API key (DuckDuckGo
    /// Lite is the no-key default). The orchestrator/app wires this from
    /// its own storage — the router never reads settings or the keyring.
    /// `skip_serializing` so a key handed in can never round-trip into a
    /// persisted settings file (§17.2: the key never leaves the client).
    #[serde(default, skip_serializing)]
    pub brave_api_key: Option<String>,
}

fn default_auto_approve_commands() -> Vec<String> {
    [
        "git status",
        "git diff",
        "git log",
        "git show",
        "ls",
        "dir",
        "cat",
        "type",
        "Get-Content",
        "Get-ChildItem",
        "cargo check",
        "cargo build",
        "cargo test",
        "cargo clippy",
        "cargo tree",
        "node --version",
        "rustc --version",
        "where",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

impl Default for AgentToolSettings {
    fn default() -> Self {
        Self {
            shell: String::new(),
            auto_approve_commands: default_auto_approve_commands(),
            brave_api_key: None,
        }
    }
}

/// Everything a tool execution needs: the workspace `root` (canonicalized
/// once, so §17.2 containment is cheap and consistent), the settings,
/// and the turn's [`BashAbortHandle`] (§17.3 Stop).
///
/// **Construct one per agent turn** — the abort handle is turn-scoped
/// (killing it is sticky for the rest of the turn) and 5c clones it
/// before awaiting `execute_tool` so the Stop button can kill a running
/// bash child.
#[derive(Clone)]
pub struct ToolContext {
    /// Canonicalized workspace root; every path resolves against it
    /// (§17.2).
    pub root: PathBuf,
    /// The `agent.*` settings slice (§17.5).
    pub settings: AgentToolSettings,
    /// The Stop button's kill switch for bash children (§17.3).
    pub abort: bash::BashAbortHandle,
    /// Phase 9b: the session's plan/todo state. Shared with the desktop
    /// session record, so turns survive and the chat snapshot hydrates it.
    pub plan: plan::PlanStore,
}

impl ToolContext {
    /// Build a context: `root` is canonicalized (falling back to the path
    /// as given when it does not exist — the tools will report that). The
    /// plan store starts in execution mode; `with_plan` attaches a real
    /// session.
    pub fn new(root: impl Into<PathBuf>, settings: AgentToolSettings) -> Self {
        let root = root.into();
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        Self {
            root,
            settings,
            abort: bash::BashAbortHandle::default(),
            plan: plan::new_store(),
        }
    }

    /// Attach a session's shared plan/todo store (Phase 9b).
    pub fn with_plan(mut self, plan: plan::PlanStore) -> Self {
        self.plan = plan;
        self
    }
}

/// §18.1: the `run_workflow` script body cap (64 KiB, measured in bytes).
pub const WORKFLOW_SCRIPT_MAX_BYTES: usize = 64 * 1024;

/// §18.1: the default workflow-level cost cap (`workflow.max_cost_usd`).
/// Used when the model omits `max_cost_usd`; the card always shows a
/// concrete cap.
pub const WORKFLOW_DEFAULT_MAX_COST_USD: f64 = 5.0;

/// One confirmed `world.run` command literal's cap. The command set is a
/// grant the user reviews line by line, so each line stays short and
/// single-line.
pub const WORKFLOW_COMMAND_MAX_BYTES: usize = 2048;

/// The confirmed command set's cap — bounded like §18.1's 256-step cap so
/// a card stays reviewable.
pub const WORKFLOW_MAX_COMMANDS: usize = 256;

/// The stable marker in a `run_workflow` confirmation acknowledgment.
/// It does not indicate that the engine started or completed a run.
pub const WORKFLOW_CONFIRMED: &str = "workflow confirmed";

/// The fixed tool manifest (ARCHITECTURE.md §17.1 — the list is closed;
/// adding a tool is an architecture change, not a code change).
///
/// Manifest v2 (§18.4) is additive: the ten §17.1 tools plus
/// `run_workflow`, the first exercise of §17.1's "versioned with the
/// app" clause.
///
/// Descriptions tell the model the workspace-rooted path contract and
/// the approval behavior, so it uses the tools correctly on the first
/// try.
pub fn tool_manifest() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "read_file",
            "Read a text file from the workspace (≤4MB). Use start_line/end_line \
             (1-based, inclusive) to read slices of long files.",
            json_schema(
                [
                    ("path", "string"),
                    ("start_line", "integer"),
                    ("end_line", "integer"),
                ],
                ["path"],
            ),
        ),
        ToolSpec::new(
            "list_dir",
            "List one directory's direct entries (hidden and gitignored entries \
             are skipped). Paths are workspace-relative.",
            json_schema([("path", "string")], ["path"]),
        ),
        ToolSpec::new(
            "fuzzy_find",
            "Fuzzy-match file paths across the workspace by name; returns the \
             best-ranked matches as workspace-relative paths.",
            json_schema([("query", "string")], ["query"]),
        ),
        ToolSpec::new(
            "search_files",
            "Search all workspace files for a literal substring \
             (case-insensitive); returns `path:line: text` hits.",
            json_schema([("query", "string")], ["query"]),
        ),
        ToolSpec::new(
            "write_file",
            "Create a file (or overwrite an existing one) with full content. \
             The user approves a diff before anything is written; creating a \
             file is shown as all additions. Empty content is refused for a \
             non-empty file (use `delete_file` to delete); an overwrite keeps \
             the previous bytes in a sibling `.<name>.selora-bak` file.",
            json_schema(
                [("path", "string"), ("content", "string")],
                ["path", "content"],
            ),
        ),
        ToolSpec::new(
            "edit_file",
            "Edit a file by replacing an exact substring. Copy old_str exactly \
             from the file (read_file first); include enough surrounding \
             context to make it unique, or set replace_all to replace every \
             occurrence. The user approves a diff before anything is written.",
            json_schema(
                [
                    ("path", "string"),
                    ("old_str", "string"),
                    ("new_str", "string"),
                    ("replace_all", "boolean"),
                ],
                ["path", "old_str", "new_str"],
            ),
        ),
        ToolSpec::new(
            "delete_file",
            "Delete a file (git-tracked files are recoverable through git). The \
             user approves before anything is deleted. Directories are \
             refused — delete files one by one.",
            json_schema([("path", "string")], ["path"]),
        ),
        ToolSpec::new(
            "bash",
            "Run a shell command (PowerShell). cwd defaults to the workspace \
             root and must stay inside it. timeout_secs: 1-600 (default 120) \
             — the child is killed on timeout. Read-only commands (git \
             status, cargo check, …) auto-run; everything else needs user \
             approval. Output is capped at 256KB.",
            json_schema(
                [
                    ("command", "string"),
                    ("timeout_secs", "integer"),
                    ("cwd", "string"),
                ],
                ["command"],
            ),
        ),
        ToolSpec::new(
            "web_search",
            "Search the web; returns a numbered list of results (title + URL).",
            json_schema([("query", "string")], ["query"]),
        ),
        ToolSpec::new(
            "web_fetch",
            "Fetch an http/https URL as text (≤256KB). Private and reserved \
             addresses are refused.",
            json_schema([("url", "string")], ["url"]),
        ),
        // -- §18.4 manifest v2: the one additive tool ---------------------
        ToolSpec::new(
            "run_workflow",
            "Write a JavaScript workflow that orchestrates subagents and \
             deterministic command gates, then run it once the user confirms. \
             The script executes against Selora's fixed workflow facade only: \
             agent(name?, persona?) -> { ask(type?, instructions) }, \
             phase(name), world.run(cmd, args, { timeoutMs? }), \
             files.glob/grep/read, git.changedFiles/diff/status/log, \
             report(item, artifactId?), artifact.markdown/file, and \
             log(message). There is no fetch, fs, process, network, or \
             timers. `commands` must list the exact `world.run` command \
             literals the script may spawn; a command assembled at runtime \
             fails the run. The user is shown the full script, this command \
             set, and the cost cap and must confirm before any part of the \
             workflow runs — that confirmation never pre-approves a \
             subagent's writes. Script body ≤64 KiB; max_cost_usd defaults \
             to 5.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Workflow name, unique within the run (resume keys steps by it)."
                    },
                    "script": {
                        "type": "string",
                        "description": "The complete JavaScript workflow body (≤64 KiB), written against the fixed facade."
                    },
                    "commands": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "The exact `world.run` command literals the script may spawn. Shown to the user for confirmation."
                    },
                    "max_cost_usd": {
                        "type": "number",
                        "description": "Workflow-level cost cap in USD, checked between steps (default 5)."
                    }
                },
                "required": ["name", "script", "commands"],
            }),
        ),
        // -- Phase 9b / manifest v3: plan mode + real todos ----------------
        ToolSpec::new(
            "enter_plan_mode",
            "Switch this session to plan mode before a non-trivial change. While \
             planning, every write, delete, bash command, and workflow is \
             structurally refused with a `plan mode is active` error; read, web, \
             and todo tools still work. Explore, then propose the plan with \
             `exit_plan_mode`.",
            json_schema([], []),
        ),
        ToolSpec::new(
            "exit_plan_mode",
            "Propose a plan and ask the user to approve it. `plan` is the ordered \
             step list (1-32 single-line steps); `summary` is optional context. \
             The user sees an approval card with Approve and Revise. Approving \
             flips the session to execution — writes still need their own diff \
             cards. Denying or revising keeps the session in plan mode; revise \
             the plan and call this tool again.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "plan": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Ordered plan steps (1-32 single-line strings)."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Optional one-paragraph summary of the plan (≤2000 bytes)."
                    }
                },
                "required": ["plan"],
            }),
        ),
        ToolSpec::new(
            "todo_read",
            "Read the session's stored todo list. Todos persist across turns and \
             hydrate from the session snapshot.",
            json_schema([], []),
        ),
        ToolSpec::new(
            "todo_write",
            "Replace the session's todo list with a validated list (≤64 items). \
             Each item is { title, status? } with status one of `pending`, \
             `in_progress`, `completed` (default `pending`). Returns the stored \
             list; nothing is reported unless it was stored. Use this to keep \
             live progress while executing.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "title": { "type": "string" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"]
                                }
                            },
                            "required": ["title"]
                        },
                        "description": "The complete replacement list (empty clears it)."
                    }
                },
                "required": ["todos"],
            }),
        ),
    ]
}

/// Build a JSON-Schema object from `(name, type)` property pairs and the
/// required list.
fn json_schema(
    props: impl IntoIterator<Item = (&'static str, &'static str)>,
    required: impl IntoIterator<Item = &'static str>,
) -> Value {
    let properties: Map<String, Value> = props
        .into_iter()
        .map(|(name, ty)| {
            (
                name.to_owned(),
                serde_json::json!({ "type": ty, "description": format!("`{name}` ({ty})") }),
            )
        })
        .collect();
    let required: Vec<String> = required.into_iter().map(str::to_owned).collect();
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

/// Route one tool call (§17.3's loop calls this for each `tool_calls`
/// entry, in order).
///
/// - Read tools and allowlisted bash run → [`ToolOutcome::Ready`].
/// - Side-effecting tools plan only → [`ToolOutcome::NeedsApproval`];
///   planning failures (bad path, missing old_str, oversize content) are
///   `Ready` **error results** — the model can correct them without a
///   card.
pub async fn execute_tool(ctx: &ToolContext, call: ToolCall) -> ToolOutcome {
    match route(ctx, &call).await {
        Ok(outcome) => outcome,
        Err(message) => ToolOutcome::Ready(ToolResult::error(&call.id, message)),
    }
}

async fn route(ctx: &ToolContext, call: &ToolCall) -> Result<ToolOutcome, String> {
    // Phase 9b's structural gate. It sits before dispatch, so a planning
    // session cannot reach a write/delete/bash/workflow implementation at
    // all — allowlisted commands and pre-approvals included. Reads, web and
    // todo tools are not side-effecting and still run.
    if plan::is_planning(&ctx.plan) && plan::is_side_effecting_tool(&call.name) {
        return Err(plan::plan_mode_rejection(&call.name));
    }
    let args = parse_arguments(&call.name, &call.arguments)?;

    match call.name.as_str() {
        // -- §17.1 read tools: no approval, straight to Ready -----------
        "read_file" => {
            let path = req_str(&args, &call.name, "path")?;
            let start_line = opt_u64(&args, &call.name, "start_line")?;
            let end_line = opt_u64(&args, &call.name, "end_line")?;
            Ok(ToolOutcome::Ready(fs::read_file(
                &ctx.root, &call.id, &path, start_line, end_line,
            )))
        }
        "list_dir" => {
            let path = req_str(&args, &call.name, "path")?;
            Ok(ToolOutcome::Ready(fs::list_dir(&ctx.root, &call.id, &path)))
        }
        "fuzzy_find" => {
            let query = req_str(&args, &call.name, "query")?;
            Ok(ToolOutcome::Ready(fs::fuzzy_find(
                &ctx.root, &call.id, &query,
            )))
        }
        "search_files" => {
            let query = req_str(&args, &call.name, "query")?;
            Ok(ToolOutcome::Ready(fs::search_files(
                &ctx.root, &call.id, &query,
            )))
        }

        // -- §17.1 write tools: plan → card, never Ready-with-side-effect -
        "write_file" => {
            let path = req_str(&args, &call.name, "path")?;
            let content = req_str(&args, &call.name, "content")?;
            match fs::plan_write(&ctx.root, &path, &content) {
                Ok((rel, diff)) => Ok(ToolOutcome::NeedsApproval(ApprovalCard::Diff {
                    path: rel,
                    diff,
                })),
                Err(message) => Ok(ToolOutcome::Ready(ToolResult::error(&call.id, message))),
            }
        }
        "edit_file" => {
            let path = req_str(&args, &call.name, "path")?;
            let old_str = req_str(&args, &call.name, "old_str")?;
            let new_str = req_str(&args, &call.name, "new_str")?;
            let replace_all = opt_bool(&args, &call.name, "replace_all")?.unwrap_or(false);
            match fs::plan_edit(&ctx.root, &path, &old_str, &new_str, replace_all) {
                Ok((rel, diff)) => Ok(ToolOutcome::NeedsApproval(ApprovalCard::Diff {
                    path: rel,
                    diff,
                })),
                Err(message) => Ok(ToolOutcome::Ready(ToolResult::error(&call.id, message))),
            }
        }
        "delete_file" => {
            let path = req_str(&args, &call.name, "path")?;
            // Validate now so a bad path is a correctable error, not a
            // card the user must deny: §17.2 containment, then the §17.1
            // contract (files only).
            match fs::resolve_rooted(&ctx.root, &path) {
                Ok(resolved) => {
                    let rel = fs::rel_display(&ctx.root, &resolved);
                    let meta = std::fs::symlink_metadata(&resolved);
                    match meta {
                        Ok(m) if m.is_dir() => Ok(ToolOutcome::Ready(ToolResult::error(
                            &call.id,
                            format!("`{rel}` is a directory — delete_file deletes files only"),
                        ))),
                        Ok(_) => Ok(ToolOutcome::NeedsApproval(ApprovalCard::Delete {
                            path: rel,
                        })),
                        Err(e) => Ok(ToolOutcome::Ready(ToolResult::error(
                            &call.id,
                            format!("cannot delete `{rel}`: {e}"),
                        ))),
                    }
                }
                Err(message) => Ok(ToolOutcome::Ready(ToolResult::error(&call.id, message))),
            }
        }

        // -- §17.5 bash: allowlist auto-run, else a command card ---------
        "bash" => {
            let command = req_str(&args, &call.name, "command")?;
            let timeout_secs = opt_u64(&args, &call.name, "timeout_secs")?;
            let cwd = opt_str(&args, &call.name, "cwd")?;
            if bash::is_allowlisted(&command, &ctx.settings.auto_approve_commands) {
                Ok(ToolOutcome::Ready(
                    bash::run_bash(ctx, &call.id, &command, timeout_secs, cwd.as_deref()).await,
                ))
            } else {
                Ok(ToolOutcome::NeedsApproval(ApprovalCard::Command {
                    command,
                }))
            }
        }

        // -- Phase 5e's web tools: routed like any other tool; the SSRF
        // guard lives inside web_fetch itself (re-checked per redirect
        // hop) — the router adds no second filter --------------------
        "web_search" => {
            let query = req_str(&args, &call.name, "query")?;
            let mut result = web::web_search(&query, ctx.settings.brave_api_key.as_deref()).await;
            result.id = call.id.clone();
            Ok(ToolOutcome::Ready(result))
        }
        "web_fetch" => {
            let url = req_str(&args, &call.name, "url")?;
            let mut result = web::web_fetch(&url).await;
            result.id = call.id.clone();
            Ok(ToolOutcome::Ready(result))
        }

        // -- §18.1/§18.4: the card *is* the confirmation moment. Nothing
        // runs here, and a malformed plan is a model-visible error, not a
        // card the user must decode and deny -----------------------------
        "run_workflow" => {
            let plan = parse_workflow(&args, &call.name)?;
            Ok(ToolOutcome::NeedsApproval(ApprovalCard::Workflow {
                name: plan.name,
                script: plan.script,
                commands: plan.commands,
                max_cost_usd: plan.max_cost_usd,
            }))
        }
        "manage_todo" | "todo_write" => {
            let todos = if call.name == "manage_todo" {
                // The pre-9b alias. It shares storage semantics with
                // todo_write; a JSON-string payload is accepted because the
                // old schema declared `todos` as a string.
                match args.get("todos") {
                    Some(Value::String(raw)) => {
                        let parsed: Value = serde_json::from_str(raw).map_err(|e| {
                            format!(
                                "`manage_todo` is an alias for `todo_write`: pass `todos` as an \
                                 array of {{ title, status? }} objects (the string was not JSON: {e})"
                            )
                        })?;
                        plan::parse_todo_value(&parsed)?
                    }
                    Some(value) => plan::parse_todo_value(value)?,
                    None => {
                        return Err(
                            "`manage_todo` is an alias for `todo_write`: it requires `todos`"
                                .to_owned(),
                        )
                    }
                }
            } else {
                plan::parse_todos(&args)?
            };
            // Store first; the success body is rendered from the stored
            // list, never from the model's argument text.
            let mut session = plan::lock(&ctx.plan);
            session.todos = todos;
            let rendered = plan::render_todos(&session.todos);
            let count = session.todos.len();
            drop(session);
            Ok(ToolOutcome::Ready(ToolResult::success(
                &call.id,
                format!("Stored {count} todo(s) for this session.\n{rendered}"),
            )))
        }
        "todo_read" => {
            let snapshot = plan::snapshot(&ctx.plan);
            let phase = match snapshot.phase {
                plan::PlanPhase::Planning => "planning",
                plan::PlanPhase::Executing => "executing",
            };
            Ok(ToolOutcome::Ready(ToolResult::success(
                &call.id,
                format!(
                    "Plan phase: {phase}.\n{}",
                    plan::render_todos(&snapshot.todos)
                ),
            )))
        }

        // -- Phase 9b: plan mode. Entering is local session state (no
        // side effect outside the session); leaving is a plan card ---------
        "enter_plan_mode" => {
            let mut session = plan::lock(&ctx.plan);
            if session.phase != plan::PlanPhase::Planning {
                session.phase = plan::PlanPhase::Planning;
                session.plan = None;
            }
            let todo_count = session.todos.len();
            drop(session);
            Ok(ToolOutcome::Ready(ToolResult::success(
                &call.id,
                format!(
                    "Plan mode entered: writes, deletes, bash commands, and workflows are \
                     blocked until the plan is approved. Reads and todo tools still work. \
                     {todo_count} stored todo(s) were kept. Explore first, then call \
                     exit_plan_mode with the ordered steps."
                ),
            )))
        }
        "exit_plan_mode" => {
            if !plan::is_planning(&ctx.plan) {
                return Err(
                    "`exit_plan_mode` requires an active plan: call `enter_plan_mode` first"
                        .to_owned(),
                );
            }
            let draft = plan::parse_plan(&args)?;
            Ok(ToolOutcome::NeedsApproval(ApprovalCard::Plan {
                summary: draft.summary,
                steps: draft.steps,
            }))
        }

        // -- §17.1: the manifest is closed --------------------------------
        unknown => Err(format!(
            "unknown tool `{unknown}` — the tool manifest (§17.1) is fixed"
        )),
    }
}

/// Execute an **approved** side-effecting call (§17.4): 5c hands the
/// original [`ToolCall`] back after the user's Approve /
/// ApproveAllThisTurn. This is the only path that writes, deletes, or
/// runs non-allowlisted bash — and it re-runs every guard (arguments are
/// re-parsed, paths re-resolved, edits re-validated against the file as
/// it is now), so a stale approval cannot be replayed against a changed
/// workspace.
///
/// `run_workflow` is re-validated here too but **does not execute**: it
/// returns [`workflow_confirmed_result`], an acknowledgment of the user's
/// explicit confirmation. The desktop starts the engine separately through
/// `workflow_run`, validating script syntax and extracting commands against
/// the supplied grant. This path creates no journal or subagent and reports
/// neither engine startup nor run completion.
///
/// Only `write_file`/`edit_file`/`delete_file`/`bash`/`run_workflow`
/// execute (or, for the workflow, validate-and-report) here; anything
/// else is an error result (read tools already ran, and a card was never
/// built for them).
pub async fn execute_approved(ctx: &ToolContext, call: ToolCall) -> ToolResult {
    // Phase 9b: a stale or pre-plan approval cannot bypass plan mode. The
    // gate runs before argument parsing for the same structural reason as
    // `route`.
    if plan::is_planning(&ctx.plan) && plan::is_side_effecting_tool(&call.name) {
        return ToolResult::error(&call.id, plan::plan_mode_rejection(&call.name));
    }
    let args = match parse_arguments(&call.name, &call.arguments) {
        Ok(args) => args,
        Err(message) => return ToolResult::error(&call.id, message),
    };
    match call.name.as_str() {
        "write_file" => {
            let path = match req_str(&args, &call.name, "path") {
                Ok(p) => p,
                Err(m) => return ToolResult::error(&call.id, m),
            };
            let content = match req_str(&args, &call.name, "content") {
                Ok(c) => c,
                Err(m) => return ToolResult::error(&call.id, m),
            };
            fs::apply_write(&ctx.root, &call.id, &path, &content)
        }
        "edit_file" => {
            let path = match req_str(&args, &call.name, "path") {
                Ok(p) => p,
                Err(m) => return ToolResult::error(&call.id, m),
            };
            let old_str = match req_str(&args, &call.name, "old_str") {
                Ok(s) => s,
                Err(m) => return ToolResult::error(&call.id, m),
            };
            let new_str = match req_str(&args, &call.name, "new_str") {
                Ok(s) => s,
                Err(m) => return ToolResult::error(&call.id, m),
            };
            let replace_all = opt_bool(&args, &call.name, "replace_all")
                .ok()
                .flatten()
                .unwrap_or(false);
            fs::apply_edit(&ctx.root, &call.id, &path, &old_str, &new_str, replace_all)
        }
        "delete_file" => {
            let path = match req_str(&args, &call.name, "path") {
                Ok(p) => p,
                Err(m) => return ToolResult::error(&call.id, m),
            };
            fs::apply_delete(&ctx.root, &call.id, &path)
        }
        "bash" => {
            let command = match req_str(&args, &call.name, "command") {
                Ok(c) => c,
                Err(m) => return ToolResult::error(&call.id, m),
            };
            let timeout_secs = opt_u64(&args, &call.name, "timeout_secs").ok().flatten();
            let cwd = opt_str(&args, &call.name, "cwd").ok().flatten();
            bash::run_bash(ctx, &call.id, &command, timeout_secs, cwd.as_deref()).await
        }
        // §18.1: re-validate the card payload and acknowledge confirmation.
        // The desktop starts the engine separately; never run a script here.
        "run_workflow" => match parse_workflow(&args, &call.name) {
            Ok(plan) => workflow_confirmed_result(&call.id, &plan),
            Err(message) => ToolResult::error(&call.id, message),
        },
        // Phase 9b: the only path that leaves plan mode. It runs after the
        // user's Approve on the plan card; the re-validation means a stale
        // card cannot smuggle a changed step list through.
        "exit_plan_mode" => match plan::parse_plan(&args) {
            Ok(draft) => {
                let mut session = plan::lock(&ctx.plan);
                if session.phase != plan::PlanPhase::Planning {
                    return ToolResult::error(
                        &call.id,
                        "the plan is no longer active: the session already left plan mode",
                    );
                }
                session.phase = plan::PlanPhase::Executing;
                session.plan = Some(draft.clone());
                plan::seed_todos_from_plan(&mut session, &draft);
                drop(session);
                plan_approved_result(&call.id, &draft)
            }
            Err(message) => ToolResult::error(&call.id, message),
        },
        other => ToolResult::error(
            &call.id,
            format!("`{other}` is not a side-effecting tool — nothing to approve"),
        ),
    }
}

/// Acknowledge the user's confirmation of the validated card payload.
///
/// `ok` means confirmation succeeded, not that a workflow started or finished.
/// The desktop handles startup separately through `workflow_run`, checking
/// script commands against the model-supplied grant; the engine rechecks it.
/// The message echoes the supplied command count and cap, not execution evidence.
pub fn workflow_confirmed_result(id: impl Into<String>, plan: &WorkflowPlan) -> ToolResult {
    ToolResult {
        id: id.into(),
        ok: true,
        content: format!(
            "run_workflow `{}`: {WORKFLOW_CONFIRMED} - confirmation acknowledged \
             ({} command(s), cap ${:.2}). This acknowledgment does not report workflow \
             startup or completion; the IDE handles execution separately. Do not call \
             run_workflow again for this script; the Workflows panel reports progress.",
            plan.name,
            plan.commands.len(),
            plan.max_cost_usd
        ),
    }
}

/// A denied card's tool result (§17.4: "a denied tool returns a
/// tool-result of `\"user denied this action\"`"). 5c calls this on
/// Deny and continues the loop.
pub fn denied_result(call: &ToolCall) -> ToolResult {
    ToolResult::error(&call.id, "user denied this action")
}

/// The tool result for an approved plan card (Phase 9b). The session is
/// now executing, but approval of the *plan* is not approval of any write:
/// every write/delete/bash still builds its own card.
pub fn plan_approved_result(id: impl Into<String>, plan: &plan::PlanDraft) -> ToolResult {
    let summary = plan
        .summary
        .as_deref()
        .map(|summary| format!(" ({summary})"))
        .unwrap_or_default();
    ToolResult::success(
        id,
        format!(
            "plan approved{summary}: {} step(s). The session is now in execution mode. \
             Write, delete, and bash tools are available again but each side effect still \
             needs its own approval card. Keep the todo list current as you work.",
            plan.steps.len()
        ),
    )
}

/// The tool result after the user chooses Revise on a plan card (Phase 9b).
/// The session stays in plan mode; the model must propose a new plan.
pub fn revise_plan_result(call: &ToolCall) -> ToolResult {
    ToolResult::error(
        &call.id,
        "user asked to revise the plan: the session is still in plan mode. Adjust the \
         steps and call exit_plan_mode again with the revised plan. No files were changed.",
    )
}

// -----------------------------------------------------------------------------
// Argument parsing (§17.6 — the router's job, not the wire layer's)
// -----------------------------------------------------------------------------

/// Parse the call's raw arguments JSON into an object. An empty string is
/// an empty object (a call with no arguments at all).
fn parse_arguments(tool: &str, raw: &str) -> Result<Map<String, Value>, String> {
    if raw.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| format!("invalid arguments JSON for `{tool}`: {e}"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("arguments for `{tool}` must be a JSON object"))
}

/// Required string parameter (schema-checked, model-visible errors).
fn req_str(args: &Map<String, Value>, tool: &str, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(format!("`{tool}` parameter `{key}` must be a string")),
        None => Err(format!("`{tool}` requires a string parameter `{key}`")),
    }
}

/// Optional string parameter.
fn opt_str(args: &Map<String, Value>, tool: &str, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{tool}` parameter `{key}` must be a string")),
    }
}

/// Optional boolean parameter.
fn opt_bool(args: &Map<String, Value>, tool: &str, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{tool}` parameter `{key}` must be a boolean")),
    }
}

/// Optional integer parameter.
fn opt_u64(args: &Map<String, Value>, tool: &str, key: &str) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("`{tool}` parameter `{key}` must be a non-negative integer")),
        Some(_) => Err(format!(
            "`{tool}` parameter `{key}` must be a non-negative integer"
        )),
    }
}

/// Optional floating-point parameter.
fn opt_f64(args: &Map<String, Value>, tool: &str, key: &str) -> Result<Option<f64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_f64()
            .map(Some)
            .ok_or_else(|| format!("`{tool}` parameter `{key}` must be a number")),
        Some(_) => Err(format!("`{tool}` parameter `{key}` must be a number")),
    }
}

/// Required array-of-strings parameter (§18.1's confirmed command set).
fn req_str_array(args: &Map<String, Value>, tool: &str, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(index, item)| match item {
                Value::String(s) => Ok(s.clone()),
                _ => Err(format!(
                    "`{tool}` parameter `{key}[{index}]` must be a string command literal"
                )),
            })
            .collect(),
        Some(_) => Err(format!(
            "`{tool}` parameter `{key}` must be an array of strings"
        )),
        None => Err(format!(
            "`{tool}` requires an array-of-strings parameter `{key}`"
        )),
    }
}

/// Validate a `run_workflow` argument set (§18.1's caps, §18.2's command
/// contract) into the [`WorkflowPlan`] the card shows.
///
/// Both the routing path (card) and [`execute_approved`] validate the original
/// call payload here. This does not parse JavaScript or extract commands;
/// the desktop and engine check the script against the supplied grant.
fn parse_workflow(args: &Map<String, Value>, tool: &str) -> Result<WorkflowPlan, String> {
    let name = req_str(args, tool, "name")?;
    let name = name.trim();
    if name.is_empty() {
        return Err(format!("`{tool}` requires a non-empty workflow `name`"));
    }
    if name.len() > 200 || name.chars().any(|c| c == '\n' || c == '\r') {
        return Err(format!(
            "`{tool}` workflow `name` must be a single line of at most 200 bytes"
        ));
    }
    let script = req_str(args, tool, "script")?;
    if script.trim().is_empty() {
        return Err(format!("`{tool}` requires a non-empty `script` body"));
    }
    if script.len() > WORKFLOW_SCRIPT_MAX_BYTES {
        return Err(format!(
            "`{tool}` script is {} bytes — §18.1 caps it at {WORKFLOW_SCRIPT_MAX_BYTES} bytes (64 KiB); shorten the script",
            script.len()
        ));
    }
    let raw_commands = req_str_array(args, tool, "commands")?;
    if raw_commands.len() > WORKFLOW_MAX_COMMANDS {
        return Err(format!(
            "`{tool}` confirmed command set has {} entries — the cap is {WORKFLOW_MAX_COMMANDS}",
            raw_commands.len()
        ));
    }
    let mut commands: Vec<String> = Vec::with_capacity(raw_commands.len());
    for (index, command) in raw_commands.into_iter().enumerate() {
        let command = command.trim();
        if command.is_empty() {
            return Err(format!(
                "`{tool}` parameter `commands[{index}]` is empty — every confirmed command needs a literal"
            ));
        }
        if command.len() > WORKFLOW_COMMAND_MAX_BYTES {
            return Err(format!(
                "`{tool}` parameter `commands[{index}]` is longer than {WORKFLOW_COMMAND_MAX_BYTES} bytes"
            ));
        }
        if command.chars().any(|c| c == '\n' || c == '\r' || c == '\0') {
            return Err(format!(
                "`{tool}` parameter `commands[{index}]` must be a single-line command literal"
            ));
        }
        // The supplied grant is a set: identical literals appear once on the card.
        if !commands.iter().any(|existing| existing == command) {
            commands.push(command.to_owned());
        }
    }
    let max_cost_usd =
        opt_f64(args, tool, "max_cost_usd")?.unwrap_or(WORKFLOW_DEFAULT_MAX_COST_USD);
    if !max_cost_usd.is_finite() || max_cost_usd <= 0.0 {
        return Err(format!(
            "`{tool}` parameter `max_cost_usd` must be a positive number of USD"
        ));
    }
    Ok(WorkflowPlan {
        name: name.to_owned(),
        script,
        commands,
        max_cost_usd,
    })
}

// -----------------------------------------------------------------------------
// Tests — routing and the approval split
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::tools::testutil::{temp_root, touch};
    use super::*;

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "call_1".to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    fn ctx(root: &Path) -> ToolContext {
        ToolContext::new(root, AgentToolSettings::default())
    }

    #[test]
    fn manifest_is_section_17_1s_ten_tools_plus_manifest_v2s_run_workflow() {
        let manifest = tool_manifest();
        let names: Vec<&str> = manifest.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "read_file",
                "list_dir",
                "fuzzy_find",
                "search_files",
                "write_file",
                "edit_file",
                "delete_file",
                "bash",
                "web_search",
                "web_fetch",
                // §18.4: manifest v2 is additive — the ten §17.1 tools and
                // exactly one new entry.
                "run_workflow",
                // Phase 9b: manifest v3 adds plan mode and real todos.
                "enter_plan_mode",
                "exit_plan_mode",
                "todo_read",
                "todo_write",
            ],
            "§17.1's closed set plus manifest v2/v3's additive tools"
        );
        for spec in &manifest {
            assert!(
                !spec.description.is_empty(),
                "{} needs a description",
                spec.name
            );
            assert!(
                spec.parameters.get("type").and_then(Value::as_str) == Some("object"),
                "{}: parameters must be a JSON-Schema object",
                spec.name
            );
            assert!(
                spec.parameters
                    .get("properties")
                    .is_some_and(|p| p.as_object().is_some()),
                "{}: parameters must carry properties",
                spec.name
            );
            assert!(
                spec.parameters
                    .get("required")
                    .is_some_and(|r| r.as_array().is_some()),
                "{}: parameters must list required",
                spec.name
            );
        }
    }

    #[test]
    fn manifest_v2_run_workflow_describes_the_facade_and_confirmation() {
        let spec = tool_manifest()
            .into_iter()
            .find(|spec| spec.name == "run_workflow")
            .expect("§18.4: run_workflow is manifest v2");

        let description = spec.description.to_lowercase();
        for needle in ["facade", "confirm", "script", "command", "cost"] {
            assert!(
                description.contains(needle),
                "the description must tell the model about `{needle}`: {}",
                spec.description
            );
        }
        for facade in [
            "agent(",
            "phase(",
            "world.run",
            "files.",
            "git.",
            "report(",
            "artifact.",
            "log(",
        ] {
            assert!(
                spec.description.contains(facade),
                "the fixed facade member `{facade}` must be named: {}",
                spec.description
            );
        }
        for absent in ["fetch", "fs", "process"] {
            assert!(
                description.contains(absent),
                "the description must warn that `{absent}` is absent"
            );
        }

        let properties = spec.parameters["properties"]
            .as_object()
            .expect("run_workflow parameters are an object");
        let mut keys: Vec<&str> = properties.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["commands", "max_cost_usd", "name", "script"]);
        assert_eq!(properties["commands"]["type"], "array");
        assert_eq!(properties["commands"]["items"]["type"], "string");
        assert_eq!(properties["max_cost_usd"]["type"], "number");
        let required: Vec<&str> = spec.parameters["required"]
            .as_array()
            .expect("required list")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(required, ["name", "script", "commands"]);
    }

    #[tokio::test]
    async fn run_workflow_always_builds_a_workflow_card() {
        let root = temp_root("route-workflow-card");
        let script = "phase('scan');\n\
                      const reviewer = agent('reviewer');\n\
                      await reviewer.ask({ instructions: 'review src' });\n\
                      await world.run('cargo', ['test'], { timeoutMs: 120000 });\n\
                      report({ ok: true });\n";
        let args = serde_json::json!({
            "name": "repo-audit",
            "script": script,
            "commands": ["cargo test", "git status", "cargo test"],
        });
        let outcome = execute_tool(&ctx(&root), call("run_workflow", &args.to_string())).await;
        match outcome {
            ToolOutcome::NeedsApproval(ApprovalCard::Workflow {
                name,
                script: card_script,
                commands,
                max_cost_usd,
            }) => {
                assert_eq!(name, "repo-audit");
                assert_eq!(card_script, script, "the card shows the full script");
                assert_eq!(
                    commands,
                    ["cargo test", "git status"],
                    "the model-supplied command set is deduped in supplied order"
                );
                assert_eq!(
                    max_cost_usd, WORKFLOW_DEFAULT_MAX_COST_USD,
                    "an omitted cap falls back to §18.1's $5 default"
                );
            }
            other => panic!("run_workflow must park on a workflow card, got {other:?}"),
        }
        // Nothing ran and nothing was written: the card is the confirmation.
        assert!(!root.join(".selora").exists());
        assert!(std::fs::read_dir(&root).unwrap().next().is_none());

        // An explicit cap reaches the card verbatim; a gate-free workflow
        // (empty command set) is legitimate and still cards.
        let capped = serde_json::json!({
            "name": "docs-pass",
            "script": "log('nothing to gate');\n",
            "commands": [],
            "max_cost_usd": 1.25,
        });
        match execute_tool(&ctx(&root), call("run_workflow", &capped.to_string())).await {
            ToolOutcome::NeedsApproval(ApprovalCard::Workflow {
                commands,
                max_cost_usd,
                ..
            }) => {
                assert!(commands.is_empty());
                assert_eq!(max_cost_usd, 1.25);
            }
            other => panic!("a gate-free workflow still needs confirmation, got {other:?}"),
        }
    }

    #[test]
    fn workflow_card_serializes_for_parked_session_persistence() {
        // Phase R3 stores parked cards; the wire shape is part of the
        // session contract, so pin it here (§17.4 / §18.1).
        let card = ApprovalCard::Workflow {
            name: "repo-audit".to_owned(),
            script: "log('hi');\n".to_owned(),
            commands: vec!["cargo test".to_owned()],
            max_cost_usd: 5.0,
        };
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "Workflow": {
                    "name": "repo-audit",
                    "script": "log('hi');\n",
                    "commands": ["cargo test"],
                    "max_cost_usd": 5.0,
                }
            })
        );
        let back: ApprovalCard = serde_json::from_value(json).unwrap();
        assert_eq!(back, card);
    }

    #[tokio::test]
    async fn run_workflow_malformed_or_oversized_plans_are_model_visible_errors() {
        let root = temp_root("route-workflow-errors");
        let oversized = "x".repeat(WORKFLOW_SCRIPT_MAX_BYTES + 1);
        let too_many_commands: Vec<String> = (0..=WORKFLOW_MAX_COMMANDS)
            .map(|index| format!("cmd {index}"))
            .collect();
        let long_command = "c".repeat(WORKFLOW_COMMAND_MAX_BYTES + 1);
        let cases: Vec<(&str, Value)> = vec![
            (
                "missing script",
                serde_json::json!({"name": "n", "commands": []}),
            ),
            (
                "non-string script",
                serde_json::json!({"name": "n", "script": 7, "commands": []}),
            ),
            (
                "blank script",
                serde_json::json!({"name": "n", "script": "  \n\t", "commands": []}),
            ),
            (
                "oversized script",
                serde_json::json!({"name": "n", "script": oversized, "commands": []}),
            ),
            (
                "missing name",
                serde_json::json!({"script": "log('x')", "commands": []}),
            ),
            (
                "blank name",
                serde_json::json!({"name": "   ", "script": "log('x')", "commands": []}),
            ),
            (
                "multiline name",
                serde_json::json!({"name": "a\nb", "script": "log('x')", "commands": []}),
            ),
            (
                "missing commands",
                serde_json::json!({"name": "n", "script": "log('x')"}),
            ),
            (
                "commands not an array",
                serde_json::json!({"name": "n", "script": "log('x')", "commands": "cargo test"}),
            ),
            (
                "non-string command",
                serde_json::json!({"name": "n", "script": "log('x')", "commands": ["ok", 3]}),
            ),
            (
                "blank command",
                serde_json::json!({"name": "n", "script": "log('x')", "commands": ["ok", "  "]}),
            ),
            (
                "multiline command",
                serde_json::json!({
                    "name": "n",
                    "script": "log('x')",
                    "commands": ["cargo test\nRemove-Item -Recurse ."]
                }),
            ),
            (
                "oversized command",
                serde_json::json!({"name": "n", "script": "log('x')", "commands": [long_command]}),
            ),
            (
                "too many commands",
                serde_json::json!({"name": "n", "script": "log('x')", "commands": too_many_commands}),
            ),
            (
                "zero cap",
                serde_json::json!({"name": "n", "script": "log('x')", "commands": [], "max_cost_usd": 0}),
            ),
            (
                "negative cap",
                serde_json::json!({"name": "n", "script": "log('x')", "commands": [], "max_cost_usd": -1}),
            ),
            (
                "non-numeric cap",
                serde_json::json!({"name": "n", "script": "log('x')", "commands": [], "max_cost_usd": "5"}),
            ),
        ];
        for (case, args) in cases {
            match execute_tool(&ctx(&root), call("run_workflow", &args.to_string())).await {
                ToolOutcome::Ready(result) => {
                    assert!(!result.ok, "{case} must be a model-visible error");
                    assert_eq!(result.id, "call_1");
                    assert!(
                        !result.content.is_empty(),
                        "{case} needs an explainable error"
                    );
                }
                ToolOutcome::NeedsApproval(card) => {
                    panic!("{case} must not build a card the user has to decode, got {card:?}")
                }
            }
        }

        // The 64 KiB cap is inclusive — one byte over is rejected above,
        // exactly 64 KiB still cards.
        let at_cap = serde_json::json!({
            "name": "big",
            "script": "x".repeat(WORKFLOW_SCRIPT_MAX_BYTES),
            "commands": [],
        });
        assert!(matches!(
            execute_tool(&ctx(&root), call("run_workflow", &at_cap.to_string())).await,
            ToolOutcome::NeedsApproval(ApprovalCard::Workflow { .. })
        ));
    }

    #[tokio::test]
    async fn execute_approved_revalidates_and_acknowledges_workflow_without_running_it() {
        let root = temp_root("route-workflow-approved");
        let script = "phase('go');\nlog('hello');\n";
        let args = serde_json::json!({
            "name": "audit",
            "script": script,
            "commands": ["cargo test"],
            "max_cost_usd": 2.5,
        });
        let c = call("run_workflow", &args.to_string());

        // Routing only requests confirmation — no engine, no journal.
        assert!(matches!(
            execute_tool(&ctx(&root), c.clone()).await,
            ToolOutcome::NeedsApproval(ApprovalCard::Workflow { .. })
        ));
        assert!(!root.join(".selora").exists());

        // An approved call re-validates the payload and acknowledges confirmation.
        // Success here is not evidence that a script started or completed.
        let result = execute_approved(&ctx(&root), c).await;
        assert!(
            result.ok,
            "confirmation succeeds without reporting an engine result: {}",
            result.content
        );
        assert!(
            result.content.contains(WORKFLOW_CONFIRMED),
            "{}",
            result.content
        );
        assert!(result.content.contains("audit"), "{}", result.content);
        assert!(
            result.content.contains("1 command"),
            "the acknowledgment echoes the supplied command count: {}",
            result.content
        );
        assert!(result.content.contains("$2.50"), "{}", result.content);
        assert!(
            result.content.contains("confirmation acknowledged"),
            "{}",
            result.content
        );
        assert!(
            result
                .content
                .contains("does not report workflow startup or completion"),
            "{}",
            result.content
        );
        assert_eq!(result.id, "call_1");
        assert!(
            !root.join(".selora").exists(),
            "a confirmation acknowledgment must not create a run journal"
        );
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "no workflow side effects may reach the workspace"
        );

        // Re-validation still applies on the approved path: a stale or
        // corrupted approval cannot smuggle an oversized script through.
        let oversized = serde_json::json!({
            "name": "big",
            "script": "x".repeat(WORKFLOW_SCRIPT_MAX_BYTES + 1),
            "commands": [],
        });
        let result =
            execute_approved(&ctx(&root), call("run_workflow", &oversized.to_string())).await;
        assert!(!result.ok);
        assert!(result.content.contains("caps it at"), "{}", result.content);
        assert!(!root.join(".selora").exists());
    }

    #[tokio::test]
    async fn read_tools_execute_immediately_as_ready() {
        let root = temp_root("route-read");
        touch(&root, "src/main.rs", "fn main() {}\n");
        for (name, args) in [
            ("read_file", r#"{"path":"src/main.rs"}"#),
            ("list_dir", r#"{"path":"."}"#),
            ("fuzzy_find", r#"{"query":"main"}"#),
            ("search_files", r#"{"query":"fn main"}"#),
        ] {
            let outcome = execute_tool(&ctx(&root), call(name, args)).await;
            match outcome {
                ToolOutcome::Ready(result) => {
                    assert!(result.ok, "{name}: {}", result.content);
                    assert_eq!(result.id, "call_1");
                }
                ToolOutcome::NeedsApproval(_) => {
                    panic!("{name} is read-only — it must never need approval (§17.1)")
                }
            }
        }
    }

    #[tokio::test]
    async fn write_edit_delete_build_cards_never_ready() {
        let root = temp_root("route-cards");
        touch(&root, "f.txt", "alpha\nbeta\n");
        let write = execute_tool(
            &ctx(&root),
            call(
                "write_file",
                r#"{"path":"new.rs","content":"fn main() {}"}"#,
            ),
        )
        .await;
        match write {
            ToolOutcome::NeedsApproval(ApprovalCard::Diff { path, diff }) => {
                assert_eq!(path, "new.rs");
                assert!(diff.contains("+fn main() {}"), "{diff}");
            }
            other => panic!("write_file must build a diff card, got {other:?}"),
        }
        // Nothing hit disk without approval.
        assert!(!root.join("new.rs").exists());

        let edit = execute_tool(
            &ctx(&root),
            call(
                "edit_file",
                r#"{"path":"f.txt","old_str":"beta","new_str":"BETA"}"#,
            ),
        )
        .await;
        match edit {
            ToolOutcome::NeedsApproval(ApprovalCard::Diff { path, diff }) => {
                assert_eq!(path, "f.txt");
                assert!(diff.contains("-beta") && diff.contains("+BETA"), "{diff}");
            }
            other => panic!("edit_file must build a diff card, got {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "alpha\nbeta\n",
            "no unapproved write may reach disk"
        );

        let delete = execute_tool(&ctx(&root), call("delete_file", r#"{"path":"f.txt"}"#)).await;
        match delete {
            ToolOutcome::NeedsApproval(ApprovalCard::Delete { path }) => assert_eq!(path, "f.txt"),
            other => panic!("delete_file must build a delete card, got {other:?}"),
        }
        assert!(root.join("f.txt").exists(), "no unapproved delete");
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn allowlisted_bash_auto_runs_and_others_get_a_card() {
        let root = temp_root("route-bash");
        let settings = AgentToolSettings {
            auto_approve_commands: vec!["Write-Output".to_owned()],
            ..AgentToolSettings::default()
        };
        let ctx = ToolContext::new(&root, settings);
        let auto = execute_tool(&ctx, call("bash", r#"{"command":"Write-Output agent"}"#)).await;
        match auto {
            ToolOutcome::Ready(result) => {
                assert!(result.ok, "{}", result.content);
                assert!(result.content.contains("agent"));
            }
            other => panic!("allowlisted bash must auto-run (§17.5), got {other:?}"),
        }
        let carded = execute_tool(
            &ctx,
            call("bash", r#"{"command":"Remove-Item -Recurse ."}"#),
        )
        .await;
        match carded {
            ToolOutcome::NeedsApproval(ApprovalCard::Command { command }) => {
                assert_eq!(command, "Remove-Item -Recurse .");
            }
            other => panic!("non-allowlisted bash must build a command card, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn web_tools_route_through_the_web_module() {
        let root = temp_root("route-web");
        // No network: an unparseable URL fails inside web_fetch before
        // any request. The router must deliver the failure as a Ready
        // outcome with the call's id stamped (5e's results carry an
        // empty id; the router owns the mapping).
        let fetch = execute_tool(&ctx(&root), call("web_fetch", r#"{"url":"not a url"}"#)).await;
        match fetch {
            ToolOutcome::Ready(result) => {
                assert!(!result.ok);
                assert_eq!(result.id, "call_1");
                assert!(result.content.contains("web_fetch"), "{}", result.content);
            }
            other => panic!("web tools never need approval, got {other:?}"),
        }
        // web_search itself performs a real query (DuckDuckGo or Brave) —
        // it is 5e's to test; here we only check argument validation
        // stays model-visible without a card.
        let bad = execute_tool(&ctx(&root), call("web_search", r#"{"query": 7}"#)).await;
        match bad {
            ToolOutcome::Ready(result) => assert!(!result.ok, "{}", result.content),
            other => panic!("invalid web_search arguments must error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_tool_is_a_model_visible_error() {
        let root = temp_root("route-unknown");
        let outcome = execute_tool(&ctx(&root), call("rm_rf_everything", "{}")).await;
        match outcome {
            ToolOutcome::Ready(result) => {
                assert!(!result.ok);
                assert!(
                    result.content.contains("unknown tool"),
                    "{}",
                    result.content
                );
            }
            other => panic!("unknown tools error out, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bad_and_missing_arguments_are_model_visible_errors() {
        let root = temp_root("route-args");
        for (name, args) in [
            ("read_file", "not json at all"),
            ("read_file", r#"{"path": 42}"#),
            ("read_file", r#"{}"#),
            ("write_file", r#"{"path":"a.txt"}"#), // missing content
            ("bash", r#"{"timeout_secs":"soon"}"#),
        ] {
            let outcome = execute_tool(&ctx(&root), call(name, args)).await;
            match outcome {
                ToolOutcome::Ready(result) => {
                    assert!(
                        !result.ok,
                        "{name} `{args}` should error, got {}",
                        result.content
                    );
                }
                ToolOutcome::NeedsApproval(_) => {
                    panic!("{name} `{args}` must not produce a card from invalid arguments")
                }
            }
        }
    }

    #[tokio::test]
    async fn write_planning_failures_are_errors_not_cards() {
        let root = temp_root("route-plan-err");
        // Escape paths never even build a card.
        let escape = execute_tool(
            &ctx(&root),
            call("write_file", r#"{"path":"../out.txt","content":"x"}"#),
        )
        .await;
        match escape {
            ToolOutcome::Ready(result) => {
                assert!(!result.ok);
                assert!(result.content.contains("escapes"), "{}", result.content);
            }
            other => panic!("an escaping write must be an error, got {other:?}"),
        }
        let delete_escape =
            execute_tool(&ctx(&root), call("delete_file", r#"{"path":"../../x"}"#)).await;
        match delete_escape {
            ToolOutcome::Ready(result) => assert!(!result.ok),
            other => panic!("an escaping delete must be an error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn approved_calls_execute_and_rejected_ones_do_not() {
        let root = temp_root("route-approved");
        // write
        let c = call(
            "write_file",
            r#"{"path":"made.txt","content":"approved\n"}"#,
        );
        let outcome = execute_tool(&ctx(&root), c.clone()).await;
        assert!(matches!(outcome, ToolOutcome::NeedsApproval(_)));
        let result = execute_approved(&ctx(&root), c).await;
        assert!(result.ok, "{}", result.content);
        assert_eq!(
            std::fs::read_to_string(root.join("made.txt")).unwrap(),
            "approved\n"
        );

        // edit
        touch(&root, "e.txt", "one two\n");
        let c = call(
            "edit_file",
            r#"{"path":"e.txt","old_str":"two","new_str":"TWO"}"#,
        );
        assert!(matches!(
            execute_tool(&ctx(&root), c.clone()).await,
            ToolOutcome::NeedsApproval(_)
        ));
        let result = execute_approved(&ctx(&root), c).await;
        assert!(result.ok, "{}", result.content);
        assert_eq!(
            std::fs::read_to_string(root.join("e.txt")).unwrap(),
            "one TWO\n"
        );

        // delete
        let c = call("delete_file", r#"{"path":"e.txt"}"#);
        assert!(matches!(
            execute_tool(&ctx(&root), c.clone()).await,
            ToolOutcome::NeedsApproval(_)
        ));
        let result = execute_approved(&ctx(&root), c).await;
        assert!(result.ok, "{}", result.content);
        assert!(!root.join("e.txt").exists());

        // An approved write still re-checks the guards: a stale call whose
        // path escaped would be refused even here.
        let stale = call("write_file", r#"{"path":"../escape.txt","content":"x"}"#);
        let result = execute_approved(&ctx(&root), stale).await;
        assert!(!result.ok);
        assert!(result.content.contains("escapes"));
        assert!(!root.join("../escape.txt").exists());
    }

    #[tokio::test]
    async fn execute_approved_rejects_read_only_tools() {
        let root = temp_root("route-approved-readonly");
        touch(&root, "f.txt", "x");
        let c = call("read_file", r#"{"path":"f.txt"}"#);
        let result = execute_approved(&ctx(&root), c).await;
        assert!(!result.ok);
        assert!(result.content.contains("not a side-effecting tool"));
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn approved_bash_runs() {
        let root = temp_root("route-approved-bash");
        let ctx = ToolContext::new(&root, AgentToolSettings::default());
        let c = call("bash", r#"{"command":"Write-Output ran"}"#);
        match execute_tool(&ctx, c.clone()).await {
            ToolOutcome::NeedsApproval(_) => {}
            other => panic!("expected a command card, got {other:?}"),
        }
        let result = execute_approved(&ctx, c).await;
        assert!(result.ok, "{}", result.content);
        assert!(result.content.contains("ran"));
    }

    #[test]
    fn denied_result_is_the_section_17_4_message() {
        let c = call("delete_file", r#"{"path":"x"}"#);
        let result = denied_result(&c);
        assert!(!result.ok);
        assert_eq!(result.content, "user denied this action");
        assert_eq!(result.id, "call_1");
    }

    #[test]
    fn tool_context_canonicalizes_the_root() {
        let root = temp_root("ctx");
        let ctx = ctx(&root);
        assert!(ctx.root.is_absolute());
        // A path inside the root still resolves (containment compares
        // canonical against canonical).
        assert!(fs::resolve_rooted(&ctx.root, ".").is_ok());
    }

    // -----------------------------------------------------------------
    // Phase 9b — plan mode, live todos, manifest v3
    // -----------------------------------------------------------------

    /// A context whose session shares `plan` with the test.
    fn ctx_with_plan(root: &Path, plan: &plan::PlanStore) -> ToolContext {
        ToolContext::new(root, AgentToolSettings::default()).with_plan(plan.clone())
    }

    async fn enter_plan(ctx: &ToolContext) {
        match execute_tool(ctx, call("enter_plan_mode", "{}")).await {
            ToolOutcome::Ready(result) => assert!(result.ok, "{}", result.content),
            other => panic!("enter_plan_mode must run without a card, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn plan_mode_rejects_every_side_effect_before_disk_but_reads_work() {
        let root = temp_root("plan-gate");
        touch(&root, "f.txt", "alpha\n");
        touch(&root, "src/main.rs", "fn main() {}\n");
        let store = plan::new_store();
        let ctx = ctx_with_plan(&root, &store);
        enter_plan(&ctx).await;
        assert_eq!(plan::snapshot(&store).phase, plan::PlanPhase::Planning);

        // Reads and todo tools still run while planning.
        for (name, args) in [
            ("read_file", r#"{"path":"f.txt"}"#),
            ("list_dir", r#"{"path":"."}"#),
            ("fuzzy_find", r#"{"query":"main"}"#),
            ("search_files", r#"{"query":"fn main"}"#),
        ] {
            match execute_tool(&ctx, call(name, args)).await {
                ToolOutcome::Ready(result) => assert!(result.ok, "{name}: {}", result.content),
                other => panic!("{name} must stay usable while planning, got {other:?}"),
            }
        }
        match execute_tool(&ctx, call("todo_write", r#"{"todos":[{"title":"Audit"}]}"#)).await {
            ToolOutcome::Ready(result) => assert!(result.ok, "{}", result.content),
            other => panic!("todo_write must work while planning, got {other:?}"),
        }

        // Every side-effecting tool is structurally refused — no card, no
        // disk write, no bash child. This includes allowlisted bash.
        let allowlisted = ToolContext::new(
            &root,
            AgentToolSettings {
                auto_approve_commands: vec!["Write-Output".to_owned()],
                ..AgentToolSettings::default()
            },
        )
        .with_plan(store.clone());
        for (name, args) in [
            ("write_file", r#"{"path":"new.txt","content":"x"}"#),
            (
                "edit_file",
                r#"{"path":"f.txt","old_str":"alpha","new_str":"beta"}"#,
            ),
            ("delete_file", r#"{"path":"f.txt"}"#),
            ("bash", r#"{"command":"Write-Output side-effect"}"#),
            ("bash", r#"{"command":"Remove-Item -Recurse ."}"#),
            (
                "run_workflow",
                r#"{"name":"w","script":"log('x');","commands":[]}"#,
            ),
        ] {
            let outcome = if name == "bash" {
                execute_tool(&allowlisted, call(name, args)).await
            } else {
                execute_tool(&ctx, call(name, args)).await
            };
            match outcome {
                ToolOutcome::Ready(result) => {
                    assert!(!result.ok, "{name} must be refused: {}", result.content);
                    assert!(
                        result.content.contains("approve the plan first"),
                        "{name}: {}",
                        result.content
                    );
                }
                other => panic!("{name} must not build a card during planning, got {other:?}"),
            }
        }
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "alpha\n",
            "no planning write/delete may reach disk"
        );
        assert!(!root.join("new.txt").exists());

        // The approved path re-checks the gate: a pre-plan approval cannot
        // execute a write while the session is still planning.
        let stale = call("write_file", r#"{"path":"new.txt","content":"x"}"#);
        let result = execute_approved(&ctx, stale).await;
        assert!(!result.ok);
        assert!(result.content.contains("approve the plan first"));
        assert!(!root.join("new.txt").exists());
    }

    #[tokio::test]
    async fn exit_plan_mode_cards_then_approval_flips_and_writes_card_again() {
        let root = temp_root("plan-approve");
        let store = plan::new_store();
        let ctx = ctx_with_plan(&root, &store);
        enter_plan(&ctx).await;

        let exit = call(
            "exit_plan_mode",
            r#"{"summary":"Add the file","plan":["Read the tree","Write new.txt","Verify"]}"#,
        );
        match execute_tool(&ctx, exit.clone()).await {
            ToolOutcome::NeedsApproval(ApprovalCard::Plan { summary, steps }) => {
                assert_eq!(summary.as_deref(), Some("Add the file"));
                assert_eq!(steps.len(), 3);
            }
            other => panic!("exit_plan_mode must build a plan card, got {other:?}"),
        }
        assert_eq!(
            plan::snapshot(&store).phase,
            plan::PlanPhase::Planning,
            "a card is not an approval — the session stays planning"
        );

        // Approve: the session flips, the approved plan is stored, and its
        // steps seed the (empty) todo list.
        let approved = execute_approved(&ctx, exit).await;
        assert!(approved.ok, "{}", approved.content);
        assert!(approved.content.contains("execution mode"));
        assert!(approved
            .content
            .contains("still needs its own approval card"));
        let snapshot = plan::snapshot(&store);
        assert_eq!(snapshot.phase, plan::PlanPhase::Executing);
        assert_eq!(snapshot.plan.as_ref().unwrap().steps.len(), 3);
        assert_eq!(snapshot.todos.len(), 3);

        // A write after approval still cards (no new trust primitive) and
        // then executes only through the approved path.
        let write = call("write_file", r#"{"path":"new.txt","content":"ok\n"}"#);
        match execute_tool(&ctx, write.clone()).await {
            ToolOutcome::NeedsApproval(ApprovalCard::Diff { path, .. }) => {
                assert_eq!(path, "new.txt")
            }
            other => panic!("an approved plan must not skip the diff card, got {other:?}"),
        }
        assert!(!root.join("new.txt").exists());
        let result = execute_approved(&ctx, write).await;
        assert!(result.ok, "{}", result.content);
        assert_eq!(
            std::fs::read_to_string(root.join("new.txt")).unwrap(),
            "ok\n"
        );
    }

    #[tokio::test]
    async fn deny_or_revise_keeps_planning_and_nothing_runs() {
        let root = temp_root("plan-revise");
        let store = plan::new_store();
        let ctx = ctx_with_plan(&root, &store);
        enter_plan(&ctx).await;
        let exit = call("exit_plan_mode", r#"{"plan":["One step"]}"#);

        // Deny is handled by the loop as a normal denial; the router never
        // flips the phase, so a following write stays blocked.
        assert!(matches!(
            execute_tool(&ctx, exit.clone()).await,
            ToolOutcome::NeedsApproval(ApprovalCard::Plan { .. })
        ));
        let write = call("write_file", r#"{"path":"x.txt","content":"x"}"#);
        let result = execute_tool(&ctx, write.clone()).await;
        match result {
            ToolOutcome::Ready(result) => assert!(!result.ok),
            ToolOutcome::NeedsApproval(_) => panic!("planning must never card a write"),
        }
        assert!(!root.join("x.txt").exists());

        // The revise result is model-visible and keeps the session planning.
        let revised = revise_plan_result(&exit);
        assert!(!revised.ok);
        assert!(revised.content.contains("still in plan mode"));
        assert_eq!(plan::snapshot(&store).phase, plan::PlanPhase::Planning);
        assert!(!root.join("x.txt").exists());
    }

    #[tokio::test]
    async fn todos_round_trip_through_session_state_and_alias_uses_real_storage() {
        let root = temp_root("todo-store");
        let store = plan::new_store();
        let ctx = ctx_with_plan(&root, &store);

        // No fabrication: an empty session reports an empty list.
        match execute_tool(&ctx, call("todo_read", "{}")).await {
            ToolOutcome::Ready(result) => {
                assert!(result.ok);
                assert!(result.content.contains("No todos"), "{}", result.content);
            }
            other => panic!("todo_read must be a ready read, got {other:?}"),
        }

        let written = r#"{"todos":[{"title":"Read","status":"completed"},{"title":"Write","status":"in_progress"}]}"#;
        match execute_tool(&ctx, call("todo_write", written)).await {
            ToolOutcome::Ready(result) => {
                assert!(result.ok);
                assert!(result.content.contains("Stored 2 todo(s)"));
                assert!(result.content.contains("[in_progress] Write"));
            }
            other => panic!("todo_write must be a ready write, got {other:?}"),
        }
        // The store is the source of truth (not the success text).
        assert_eq!(plan::snapshot(&store).todos.len(), 2);

        // todo_read returns the stored list after a *different* call —
        // session-scoped, not turn-scoped.
        match execute_tool(&ctx, call("todo_read", "{}")).await {
            ToolOutcome::Ready(result) => {
                assert!(result.content.contains("Read"), "{}", result.content);
                assert!(result.content.contains("Write"), "{}", result.content);
            }
            other => panic!("todo_read must return the stored list, got {other:?}"),
        }

        // The legacy alias writes the same store; a JSON string is accepted
        // because the old schema declared `todos` as a string.
        let alias = serde_json::json!({
            "todos": serde_json::json!([{ "title": "Alias step" }]).to_string()
        });
        match execute_tool(&ctx, call("manage_todo", &alias.to_string())).await {
            ToolOutcome::Ready(result) => assert!(result.ok, "{}", result.content),
            other => panic!("manage_todo is an alias, got {other:?}"),
        }
        let snapshot = plan::snapshot(&store);
        assert_eq!(snapshot.todos.len(), 1);
        assert_eq!(snapshot.todos[0].title, "Alias step");

        // An empty array clears the stored list; success is still backed
        // by storage.
        match execute_tool(&ctx, call("todo_write", r#"{"todos":[]}"#)).await {
            ToolOutcome::Ready(result) => assert!(result.ok),
            other => panic!("clearing is a ready write, got {other:?}"),
        }
        assert!(plan::snapshot(&store).todos.is_empty());
    }

    #[tokio::test]
    async fn plan_and_todo_malformed_arguments_are_model_visible_errors() {
        let root = temp_root("plan-args");
        let store = plan::new_store();
        let ctx = ctx_with_plan(&root, &store);
        let cases = [
            ("exit_plan_mode", "{}"),
            ("exit_plan_mode", r#"{"plan":[]}"#),
            ("exit_plan_mode", r#"{"plan":"not an array"}"#),
            ("exit_plan_mode", r#"{"plan":[7]}"#),
            // A valid plan outside plan mode is still refused: the tool
            // cannot fabricate plan mode.
            ("exit_plan_mode", r#"{"plan":["valid step"]}"#),
            ("todo_write", "{}"),
            ("todo_write", r#"{"todos":[{"title":""}]}"#),
            ("todo_write", r#"{"todos":[{"title":"x","status":"done"}]}"#),
            ("manage_todo", r#"{"todos":"not json"}"#),
            ("manage_todo", "{}"),
        ];
        for (name, args) in cases {
            match execute_tool(&ctx, call(name, args)).await {
                ToolOutcome::Ready(result) => {
                    assert!(!result.ok, "{name} `{args}` must error");
                    assert_eq!(result.id, "call_1");
                }
                ToolOutcome::NeedsApproval(card) => {
                    panic!("{name} `{args}` must not card, got {card:?}")
                }
            }
        }
        // Failed writes never mutate the session.
        assert!(plan::snapshot(&store).todos.is_empty());
        assert!(plan::snapshot(&store).plan.is_none());
        assert_eq!(plan::snapshot(&store).phase, plan::PlanPhase::Executing);
    }

    #[test]
    fn plan_card_serializes_for_parked_session_persistence() {
        let card = ApprovalCard::Plan {
            summary: Some("Do the thing".into()),
            steps: vec!["First".into(), "Second".into()],
        };
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "Plan": {
                    "summary": "Do the thing",
                    "steps": ["First", "Second"],
                }
            })
        );
        let back: ApprovalCard = serde_json::from_value(json).unwrap();
        assert_eq!(back, card);
    }
}
