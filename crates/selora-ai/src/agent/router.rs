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
use super::ApprovalCard;

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
}

impl ToolContext {
    /// Build a context: `root` is canonicalized (falling back to the path
    /// as given when it does not exist — the tools will report that).
    pub fn new(root: impl Into<PathBuf>, settings: AgentToolSettings) -> Self {
        let root = root.into();
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        Self {
            root,
            settings,
            abort: bash::BashAbortHandle::default(),
        }
    }
}

/// The fixed tool manifest (ARCHITECTURE.md §17.1 — the list is closed;
/// adding a tool is an architecture change, not a code change).
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
             file is shown as all additions.",
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
/// Only `write_file`/`edit_file`/`delete_file`/`bash` execute here;
/// anything else is an error result (read tools already ran, and a card
/// was never built for them).
pub async fn execute_approved(ctx: &ToolContext, call: ToolCall) -> ToolResult {
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
        other => ToolResult::error(
            &call.id,
            format!("`{other}` is not a side-effecting tool — nothing to approve"),
        ),
    }
}

/// A denied card's tool result (§17.4: "a denied tool returns a
/// tool-result of `\"user denied this action\"`"). 5c calls this on
/// Deny and continues the loop.
pub fn denied_result(call: &ToolCall) -> ToolResult {
    ToolResult::error(&call.id, "user denied this action")
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
    fn manifest_has_exactly_the_ten_section_17_1_tools() {
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
            ],
            "§17.1 is a closed set"
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
}
