//! The desktop [`Host`] implementation for the workflow engine (Phase 8, 8c).
//!
//! `selora-workflow` is deliberately outside-in: the engine owns the boa realm,
//! the facade, the schema validator and the caps, and talks to the machine
//! only through the blocking [`Host`] trait. This module is the trusted half of
//! that seam for the desktop shell. Its facade is blocking and synchronous;
//! subagent turns stream on dedicated runtime threads, with approval-gated tools:
//!
//! * `files.*` walk and read the workspace root through `selora-fs`
//!   ([`build_tree`], [`search_in_files`]). Every path is validated as a
//!   workspace-relative path and canonicalized before use, so `..`, absolute
//!   paths and symlinks that escape the root are rejected. There is no write
//!   method to implement. Matching the sidebar's engines means hidden entries
//!   (`.github/…`, `.git/…`) and `.gitignore`d files are **not** searched.
//! * `git.*` are read-only over `selora-git` (`GitRepo::status`,
//!   `diff_index`, `diff_workdir`). Results are filtered to the workspace root
//!   when the workspace is a subdirectory of a larger repository.
//! * `world_run` is a PowerShell child (`pwsh` when present, else
//!   `powershell`) with `-NoProfile -NonInteractive`, rooted at the workspace,
//!   started from a **clean environment allowlist**, contained in a Windows Job
//!   Object so descendants die with it, with stdout+stderr captured under one
//!   byte budget and the whole process tree killed on timeout or on
//!   [`WorkflowHost::cancel`]. The engine has already checked the command
//!   against the confirmation set; this layer only maps the confirmed command
//!   and its args onto the child's argv.
//! * `ask` runs `selora_ai::run_agent_turn` with a rooted router, per-ask caps,
//!   and one lossless tool conversation per agent name. The workflow scheduler
//!   owns same-name FIFO and global concurrency. Writes and every bash command
//!   require a turn-local approval; script confirmation grants never cross in.
//! * `report` / `artifact.*` / `log` are stored on the host, per run, so the
//!   shell can render them next to the engine's own event stream.
//!
//! # Caps
//!
//! Defaults (all configurable on [`WorkflowHostConfig`]):
//!
//! * 256 KiB per file read, 256 KiB per `world.run` capture (stdout+stderr
//!   together), 256 KiB per list/diff/tool output,
//! * 1 000 glob/grep items,
//! * 4 concurrent asks (ARCHITECTURE.md §18.3),
//! * a 120 s default `world.run` timeout.
//!
//! `files.read`, `git.diff` and process capture **cap-reject or truncate
//! explicitly**: a read over the cap is an error (never a silently shortened
//! file), list output stops at the budget, and a diff over the cap names the
//! cap and suggests narrowing with `git.diff(base, path)`.
//!
//! # Explicit limitations of this wave
//!
//! * Agent tools operate on disk, not the shell's open-buffer store. The caller
//!   must refresh buffers after approved edits, as for other external changes.
//! * Without model metadata, receipt costs are authoritative but the model-price
//!   fallback is unknown (zero). The shell should supply `with_model`.
//! * Collections (reports, artifacts, logs) are in memory for one run; the
//!   engine's journal is the durable record. Ask history can be checkpointed and
//!   restored through the `Host` snapshot hooks during replay.
//! * **Exit codes and process trees.** `world.run` reports the PowerShell
//!   process's exit code; Windows PowerShell 5.1 collapses a failing native
//!   command to `1` (`pwsh` preserves it). The child is created suspended and
//!   assigned to a Windows Job Object *before* it executes
//!   (`workflows_process`), so a timeout or cancellation kills the whole tree,
//!   including `Start-Process`/`Start-Job` descendants, and a descendant
//!   holding the inherited capture pipes can no longer delay the reader joins.
//!   A spawn whose job creation, assignment or resume fails is terminated
//!   without ever being resumed. The residual escape is a process another
//!   broker creates on the child's behalf (for example a `ShellExecuteEx`
//!   launch started by the shell service), which is not a `CreateProcess`
//!   descendant; see that module.
//! * The legacy `with_chat` synchronous fake is cooperative only around the
//!   closure. Real turns cancel HTTP, parked approvals and running tools promptly.
//! * `git.log` and any `git.*` call with a `base` use the `git` CLI, because
//!   `selora-git`'s Lite surface has no revwalk and no base-to-worktree diff
//!   yet. Fixed argv, no shell, read-only commands; a typed error when `git`
//!   is not on PATH. `git.status`, `git.changedFiles(None)` and `git.diff`
//!   without a base stay on `selora-git`.
//!
//! # Threading
//!
//! [`Host`] is blocking on purpose (ARCHITECTURE.md §18.4). The shell calls
//! `run_workflow` from `tokio::task::spawn_blocking`; this host never runs on
//! the UI thread. The real chat call gets its own current-thread runtime on a
//! dedicated worker thread, so it is safe whether or not the caller already
//! sits inside a Tokio runtime context.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use selora_ai::{ChatMessage, Model, TurnCaps};
use selora_fs::{
    search::{search_in_files, SearchQuery},
    tree::{build_tree, FileTreeNode},
};
use selora_git::GitRepo;
use selora_workflow::{
    sandbox::{MAX_ARTIFACT_BYTES, MAX_REPORT_ITEMS, MAX_REPORT_ITEM_BYTES},
    Artifact, ArtifactContent, CommandOutput, GitCommit, GrepMatch, Host, HostError, ReportItem,
};
use serde_json::Value;

use crate::workflows_process::WorldProcess;

#[path = "workflows_agent.rs"]
mod workflows_agent;
use selora_workflow::subagent::{SubagentConfig, Subagents};
use workflows_agent::{LiveTurns, WorkflowAskExecutor};

/// Cap on one `files.read`, one list body, or one diff (256 KiB).
pub const MAX_READ_BYTES: usize = 256 * 1024;

/// Cap on one `world.run`'s captured stdout+stderr together (256 KiB).
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// Cap on items returned by `files.glob` / `files.grep` in one call.
pub const MAX_ITEMS: usize = 1_000;

/// `workflow.max_concurrency` default (ARCHITECTURE.md §18.3).
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

/// `world.run` timeout when the script does not pass `timeoutMs`.
pub const DEFAULT_WORLD_TIMEOUT_MS: u64 = 120_000;

/// `git.log` entries when the script does not pass a count.
pub const DEFAULT_GIT_LOG_COUNT: usize = 20;

/// Upper bound on a script-supplied `git.log` count.
pub const MAX_GIT_LOG_COUNT: usize = 100;

/// The environment the `world.run` child is allowed to inherit.
///
/// Nothing else crosses the boundary: in particular the gateway key
/// (`SELORA_API_KEY` / `OPENAI_API_KEY`) is never in the child's environment,
/// because the child is built with `env_clear` and then these names are copied
/// one by one. `PATH` is required to resolve the confirmed command;
/// `SystemRoot`/`TEMP`/`TMP`/`APPDATA`/`LOCALAPPDATA`/`USERPROFILE` are what
/// Windows itself needs.
pub const ALLOWED_ENV_VARS: [&str; 7] = [
    "PATH",
    "SystemRoot",
    "TEMP",
    "TMP",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
];

/// Key-bearing names the host strips from any child it starts itself.
const SECRET_ENV_VARS: [&str; 2] = ["SELORA_API_KEY", "OPENAI_API_KEY"];

/// One chat call for [`WorkflowHost`]: `(model, whole conversation including
/// the system prompt)` → assistant text.
///
/// This legacy test seam bypasses the tool loop. Production uses a streaming
/// agent turn; tool-aware tests inject a `ChatStream` instead.
pub type ChatFn = Arc<dyn Fn(&str, &[ChatMessage]) -> Result<String, HostError> + Send + Sync>;

/// Publish one subagent's pending approval after its decision slot is installed.
/// Publish without blocking; the handler may synchronously call `decide`.
pub type WorkflowApprovalHandler = Arc<dyn Fn(uuid::Uuid, selora_ai::ApprovalCard) + Send + Sync>;

/// Everything the host needs from the shell: gateway endpoint + model,
/// concurrency, and the read/capture caps.
///
/// `api_key` is never printed: the manual [`fmt::Debug`] prints `[REDACTED]`
/// (the same RISKS.md §8.3 rule `SeloraClient` follows).
#[derive(Clone)]
pub struct WorkflowHostConfig {
    /// Gateway base URL (`https://api.selora.lol` or a loopback test server).
    pub base_url: String,
    /// Gateway key. Held for the process lifetime; never logged.
    pub api_key: String,
    /// Model id every subagent ask uses.
    pub model: String,
    /// Maximum asks in flight at once (ARCHITECTURE.md §18.3).
    pub max_concurrency: usize,
    /// Cap for one `files.read` and for one list/diff body.
    pub max_read_bytes: usize,
    /// Cap for one `world.run` capture (stdout+stderr together).
    pub max_output_bytes: usize,
    /// Cap on glob/grep items.
    pub max_items: usize,
    /// Timeout when the script does not pass `timeoutMs`.
    pub default_timeout_ms: u64,
    /// Test seam; `None` means the real streaming agent-turn path.
    pub chat: Option<ChatFn>,
    model_metadata: Option<Model>,
    turn_caps: Option<TurnCaps>,
    approval_handler: Option<WorkflowApprovalHandler>,
    #[cfg(test)]
    stream_factory: Option<workflows_agent::StreamFactory>,
}

impl WorkflowHostConfig {
    /// The §18 defaults for one run.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            max_read_bytes: MAX_READ_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
            max_items: MAX_ITEMS,
            default_timeout_ms: DEFAULT_WORLD_TIMEOUT_MS,
            chat: None,
            model_metadata: None,
            turn_caps: None,
            approval_handler: None,
            #[cfg(test)]
            stream_factory: None,
        }
    }

    /// Replace the plain chat transport for legacy deterministic tests.
    #[must_use]
    #[allow(dead_code)] // used by tests and by the wiring slice
    pub fn with_chat<F>(mut self, chat: F) -> Self
    where
        F: Fn(&str, &[ChatMessage]) -> Result<String, HostError> + Send + Sync + 'static,
    {
        self.chat = Some(Arc::new(chat));
        self
    }

    /// Route tool approval cards to the run's UI. Grants stay local to one ask.
    #[must_use]
    pub fn with_approval_handler(mut self, handler: WorkflowApprovalHandler) -> Self {
        self.approval_handler = Some(handler);
        self
    }

    /// Supply authoritative model identity, context window and prices.
    #[must_use]
    pub fn with_model(mut self, model: Model) -> Self {
        self.model = model.id.clone();
        self.model_metadata = Some(model);
        self
    }

    /// Set fresh limits for each ask, never a shared mutable turn budget.
    #[must_use]
    pub fn with_turn_caps(mut self, caps: TurnCaps) -> Self {
        self.turn_caps = Some(caps);
        self
    }

    /// Override the ask concurrency cap (`0` is clamped to `1`).
    #[must_use]
    #[allow(dead_code)] // used by tests and by the wiring slice
    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    /// Override the read/list/diff cap.
    #[must_use]
    #[allow(dead_code)] // used by tests and by the wiring slice
    pub fn with_max_read_bytes(mut self, max_read_bytes: usize) -> Self {
        self.max_read_bytes = max_read_bytes.max(1);
        self
    }

    /// Override the `world.run` capture cap.
    #[must_use]
    #[allow(dead_code)] // settings-driven overrides are a wave-3 concern
    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes.max(1);
        self
    }

    /// Override the glob/grep item cap.
    #[must_use]
    #[allow(dead_code)] // used by tests and by the wiring slice
    pub fn with_max_items(mut self, max_items: usize) -> Self {
        self.max_items = max_items.max(1);
        self
    }

    /// Override the default `world.run` timeout.
    #[must_use]
    #[allow(dead_code)] // settings-driven overrides are a wave-3 concern
    pub fn with_default_timeout_ms(mut self, default_timeout_ms: u64) -> Self {
        self.default_timeout_ms = default_timeout_ms;
        self
    }
}

impl fmt::Debug for WorkflowHostConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkflowHostConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("max_concurrency", &self.max_concurrency)
            .field("max_read_bytes", &self.max_read_bytes)
            .field("max_output_bytes", &self.max_output_bytes)
            .field("max_items", &self.max_items)
            .field("default_timeout_ms", &self.default_timeout_ms)
            .field("chat", &self.chat.is_some())
            .finish()
    }
}

/// The desktop [`Host`] for one workflow run.
///
/// Cheap to share (`Arc<WorkflowHost>`): every collection is behind a mutex and
/// the cancellation flag is atomic. One host per run — the report, artifact and
/// conversation stores are intentionally run-scoped.
pub struct WorkflowHost {
    root: PathBuf,
    root_display: String,
    config: WorkflowHostConfig,
    scheduler: Subagents<WorkflowAskExecutor>,
    live_turns: Mutex<LiveTurns>,
    cancelled: AtomicBool,
    conversations: Mutex<BTreeMap<String, Vec<ChatMessage>>>,
    reports: Mutex<Vec<ReportItem>>,
    artifacts: Mutex<Vec<Artifact>>,
    logs: Mutex<Vec<String>>,
    cost_bits: AtomicU64,
    tokens: AtomicU64,
}

impl WorkflowHost {
    /// Build the run's host rooted at `workspace_root`.
    ///
    /// A root that cannot be canonicalized (missing folder) is kept verbatim;
    /// every read then fails with a typed error instead of escaping anywhere.
    #[must_use]
    pub fn new(workspace_root: PathBuf, mut config: WorkflowHostConfig) -> Arc<Self> {
        let root = std::fs::canonicalize(&workspace_root).unwrap_or(workspace_root);
        let root_display = root.display().to_string();
        if let Some(model) = &config.model_metadata {
            config.model = model.id.clone();
        }
        config.max_concurrency = config.max_concurrency.max(1);
        config.max_read_bytes = config.max_read_bytes.max(1);
        config.max_output_bytes = config.max_output_bytes.max(1);
        config.max_items = config.max_items.max(1);
        let mut caps = config.turn_caps.clone().unwrap_or_else(|| {
            TurnCaps::spec_defaults(
                config
                    .model_metadata
                    .as_ref()
                    .map_or(0, |m| m.context_window),
            )
        });
        if let Some(model) = &config.model_metadata {
            caps.context_window = model.context_window;
        }
        Arc::new_cyclic(|host| Self {
            root,
            root_display,
            scheduler: Subagents::new(
                Arc::new(WorkflowAskExecutor::new(host.clone())),
                caps,
                SubagentConfig::new(config.max_concurrency),
            ),
            live_turns: Mutex::new(LiveTurns::default()),
            config,
            cancelled: AtomicBool::new(false),
            conversations: Mutex::new(BTreeMap::new()),
            reports: Mutex::new(Vec::new()),
            artifacts: Mutex::new(Vec::new()),
            logs: Mutex::new(Vec::new()),
            cost_bits: AtomicU64::new(0.0f64.to_bits()),
            tokens: AtomicU64::new(0),
        })
    }

    /// The canonical workspace root every read is confined to.
    #[must_use]
    #[allow(dead_code)] // read by the shell once the panel is wired
    pub fn workspace_root(&self) -> &Path {
        &self.root
    }

    /// Stop the run and signal every live agent turn (HTTP, tool or approval
    /// park). Queued asks reject as the cancelled turns release scheduler slots.
    /// `world_run` also checks the flag while waiting for its child.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.lock(&self.live_turns).cancel();
    }

    /// Answer exactly one live card. Wrong, stale and duplicate IDs never send.
    pub fn decide(
        &self,
        approval_id: uuid::Uuid,
        decision: selora_ai::AgentDecision,
    ) -> Result<(), HostError> {
        self.ensure_running()?;
        self.lock(&self.live_turns).decide(approval_id, decision)
    }

    /// Receipt tokens from all asks, including failed, stopped and invalid JSON turns.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.tokens.load(Ordering::SeqCst)
    }

    /// Whether [`Self::cancel`] has been called.
    #[must_use]
    #[allow(dead_code)] // consumed by the panel/wiring slice
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Add external costs to the run's meter; agent receipts accrue automatically.
    #[allow(dead_code)] // consumed by the cost-meter wiring
    pub fn add_cost_usd(&self, cost_usd: f64) {
        if !cost_usd.is_finite() || cost_usd <= 0.0 {
            return;
        }
        let mut current = self.cost_bits.load(Ordering::SeqCst);
        loop {
            let total = f64::from_bits(current) + cost_usd;
            match self.cost_bits.compare_exchange(
                current,
                total.to_bits(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return,
                Err(seen) => current = seen,
            }
        }
    }

    /// The report items collected so far (`report()` calls, in order).
    #[must_use]
    #[allow(dead_code)] // consumed by the panel/wiring slice
    pub fn reports(&self) -> Vec<ReportItem> {
        self.lock(&self.reports).clone()
    }

    /// The artifacts collected so far, in creation order.
    #[must_use]
    #[allow(dead_code)] // consumed by the panel/wiring slice
    pub fn artifacts(&self) -> Vec<Artifact> {
        self.lock(&self.artifacts).clone()
    }

    /// The run-log lines, in order.
    #[must_use]
    #[allow(dead_code)] // consumed by the panel/wiring slice
    pub fn logs(&self) -> Vec<String> {
        self.lock(&self.logs).clone()
    }

    /// The full stored conversation, including tool messages and failed turns.
    #[must_use]
    #[allow(dead_code)] // consumed by the panel/wiring slice
    pub fn conversation(&self, agent: &str) -> Option<Vec<ChatMessage>> {
        self.lock(&self.conversations).get(agent).cloned()
    }

    fn lock<'a, T>(&self, mutex: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ensure_running(&self) -> Result<(), HostError> {
        if self.is_cancelled() {
            Err(cancelled_error())
        } else {
            Ok(())
        }
    }

    fn redact_error(&self, error: HostError) -> HostError {
        HostError::new(self.redact(&error.message))
    }

    fn redact(&self, text: &str) -> String {
        if self.config.api_key.is_empty() {
            text.to_owned()
        } else {
            text.replace(&self.config.api_key, "[REDACTED]")
        }
    }

    /* ------------------------------- files ------------------------------- */

    /// Resolve a workspace-relative path and prove it is still inside the root
    /// after canonicalization (the symlink check).
    fn resolve_existing(&self, value: &str) -> Result<PathBuf, HostError> {
        let relative = workspace_relative_path(value)?;
        let candidate = self.root.join(&relative);
        let canonical = std::fs::canonicalize(&candidate)
            .map_err(|err| HostError::new(format!("`{value}` cannot be read: {err}")))?;
        if canonical.starts_with(&self.root) {
            Ok(canonical)
        } else {
            Err(outside_error(value))
        }
    }

    /// A workspace-relative, forward-slash path, or `None` when `path` is not
    /// under the root.
    ///
    /// `Path::strip_prefix` alone is not enough on Windows: git and the
    /// canonicalizer can disagree about case (`C:\Users` vs `c:\users`) or
    /// carry a `\\?\` prefix. The folded fallback compares whole components
    /// and maps the length back onto the original text, so the returned path
    /// keeps the caller's casing.
    fn workspace_relative(&self, path: &Path) -> Option<String> {
        if let Ok(relative) = path.strip_prefix(&self.root) {
            return Some(relative.to_string_lossy().replace('\\', "/"));
        }
        let text = strip_extended_prefix(&path.as_os_str().to_string_lossy().replace('\\', "/"))
            .to_owned();
        let root =
            strip_extended_prefix(&self.root.as_os_str().to_string_lossy().replace('\\', "/"))
                .trim_end_matches('/')
                .to_owned();
        if text.eq_ignore_ascii_case(&root) {
            return Some(String::new());
        }
        let folded = text.to_ascii_lowercase();
        let root_folded = root.to_ascii_lowercase();
        let rest = folded.strip_prefix(root_folded.as_str())?;
        if !rest.starts_with('/') {
            return None;
        }
        let offset = folded.len() - rest.len();
        Some(text[offset..].trim_start_matches('/').to_owned())
    }

    /* -------------------------------- git -------------------------------- */

    fn open_repo(&self) -> Result<GitRepo, HostError> {
        GitRepo::open(&self.root).map_err(|err| {
            HostError::new(format!(
                "`{}` does not expose a git repository: {err}",
                self.root_display
            ))
        })
    }

    /// Run a fixed, read-only git command in the workspace, with the gateway
    /// key removed from the child's environment.
    fn git_command(&self, args: &[String]) -> Result<std::process::Output, HostError> {
        let mut command = Command::new("git");
        command
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in SECRET_ENV_VARS {
            command.env_remove(name);
        }
        command.args(args);
        command.output().map_err(|err| {
            HostError::new(format!(
                "this git read needs the `git` CLI, which could not be started: {err}"
            ))
        })
    }

    fn git_stdout(&self, args: &[String]) -> Result<String, HostError> {
        let output = self.git_command(args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(HostError::new(format!(
                "`git {}` failed: {}",
                args.join(" "),
                stderr.trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn reject_oversize_diff(&self, diff: String) -> Result<String, HostError> {
        if diff.len() > self.config.max_read_bytes {
            return Err(HostError::new(format!(
                "the diff is {} bytes, over the {}-byte cap — pass a path to narrow it",
                diff.len(),
                self.config.max_read_bytes
            )));
        }
        Ok(diff)
    }

    /* -------------------------------- ask -------------------------------- */

    fn system_prompt(&self, agent: &str, persona: Option<&str>) -> String {
        let mut prompt = format!(
            "You are the `{agent}` subagent inside a Selora IDE workflow run.\n\
             \n\
             Workspace root: {root}\n\
             \n\
             Role and limits:\n\
             - Use read-only tools to gather evidence. Writes, edits, deletes and ALL shell \
             commands need explicit user approval; script confirmation is not a tool grant.\n\
             - Stay inside the workspace root. Do not invoke nested workflows or manage_todo.\n\
             - Answer the instructions directly. When a JSON schema is supplied, reply with \
             ONLY one JSON value that matches it: no prose, no markdown fences.\n\
             - Never claim to have changed a file, run a command, or verified something you \
             were not given.\n",
            root = self.root_display
        );
        if let Some(persona) = persona.map(str::trim).filter(|value| !value.is_empty()) {
            prompt.push_str("\nPersona:\n");
            prompt.push_str(persona);
            prompt.push('\n');
        }
        prompt
    }

    fn ask_impl(
        &self,
        agent: &str,
        persona: Option<&str>,
        instructions: &str,
        schema: Option<&Value>,
    ) -> Result<Value, HostError> {
        self.ensure_running()?;
        let agent = agent.trim();
        if agent.is_empty() {
            return Err(HostError::new(
                "`ask` needs a non-empty agent name (the run cannot identify an answer that belongs to no subagent)",
            ));
        }
        if instructions.trim().is_empty() {
            return Err(HostError::new(
                "`ask` needs non-empty instructions — an empty mission has nothing to answer",
            ));
        }

        let mut prompt = instructions.to_owned();
        if let Some(schema) = schema {
            prompt.push_str("\n\nThe answer must be one JSON value matching this schema:\n");
            prompt.push_str(&serde_json::to_string_pretty(schema).unwrap_or_default());
        }

        let handle = match persona {
            Some(persona) => self.scheduler.agent_with_persona(agent, persona),
            None => Ok(self.scheduler.agent(agent)),
        }
        .map_err(|err| HostError::new(err.to_string()))?;
        // Only the executor may snapshot full history: dispatch acquires the
        // same-name FIFO turn and a global permit before invoking it.
        let reply = handle
            .ask(prompt)
            .map_err(|err| HostError::new(err.to_string()))?;
        self.ensure_running()?;
        let answer = parse_answer(&reply.content)
            .map_err(|err| HostError::new(format!("subagent `{agent}`: {}", err.message)))?;
        Ok(answer)
    }

    fn push_artifact(&self, artifact: Artifact) -> Result<(), HostError> {
        if artifact.id.trim().is_empty() {
            return Err(HostError::new("artifact ids must not be empty"));
        }
        let mut artifacts = self.lock(&self.artifacts);
        if artifacts.iter().any(|existing| existing.id == artifact.id) {
            return Err(HostError::new(format!(
                "artifact id `{}` was already used in this run",
                artifact.id
            )));
        }
        artifacts.push(artifact);
        Ok(())
    }

    /// Append the staged (HEAD→index) and unstaged (index→worktree) sides of
    /// one workspace file, exactly like the git panel chooses a row's diff.
    fn push_workspace_diff(
        &self,
        repo: &GitRepo,
        relative: &str,
        parts: &mut Vec<String>,
    ) -> Result<(), HostError> {
        let absolute = self.root.join(relative);
        let index = repo
            .diff_index(&absolute)
            .map_err(|err| HostError::new(format!("git diff `{relative}` failed: {err}")))?;
        if !index.is_empty() {
            parts.push(index.to_unified());
        }
        let worktree = repo
            .diff_workdir(&absolute)
            .map_err(|err| HostError::new(format!("git diff `{relative}` failed: {err}")))?;
        if !worktree.is_empty() {
            parts.push(worktree.to_unified());
        }
        Ok(())
    }
}

impl Host for WorkflowHost {
    fn snapshot_agent(&self, agent: &str) -> Result<Option<Value>, HostError> {
        self.conversation(agent)
            .map(|conversation| {
                serde_json::to_value(serde_json::json!({
                    "version": 1,
                    "agent": agent,
                    "workspace_root": self.root_display,
                    "conversation": conversation,
                }))
                .map_err(|error| {
                    self.redact_error(HostError::new(format!(
                        "cannot checkpoint agent history: {error}"
                    )))
                })
            })
            .transpose()
    }

    fn restore_agent(&self, agent: &str, snapshot: &Value) -> Result<(), HostError> {
        self.ensure_running()?;
        if snapshot.get("version").and_then(Value::as_u64) != Some(1)
            || snapshot.get("agent").and_then(Value::as_str) != Some(agent)
            || snapshot.get("workspace_root").and_then(Value::as_str) != Some(&self.root_display)
        {
            return Err(HostError::new(
                "agent checkpoint identity or workspace does not match",
            ));
        }
        let history: Vec<ChatMessage> = serde_json::from_value(snapshot["conversation"].clone())
            .map_err(|error| {
                self.redact_error(HostError::new(format!("invalid agent checkpoint: {error}")))
            })?;
        if history
            .first()
            .is_none_or(|message| message.role != selora_ai::Role::System)
            || history
                .last()
                .is_none_or(|message| message.role != selora_ai::Role::Assistant)
        {
            return Err(HostError::new(
                "agent checkpoint must contain a completed conversation",
            ));
        }
        let mut pending = std::collections::BTreeSet::new();
        for message in &history {
            if message.role != selora_ai::Role::Tool && !pending.is_empty() {
                return Err(HostError::new(
                    "agent checkpoint contains unanswered tool calls",
                ));
            }
            if let Some(calls) = &message.tool_calls {
                if message.role != selora_ai::Role::Assistant {
                    return Err(HostError::new("agent checkpoint has misplaced tool calls"));
                }
                for call in calls {
                    if call.id.is_empty() || !pending.insert(call.id.clone()) {
                        return Err(HostError::new(
                            "agent checkpoint has duplicate or empty tool ids",
                        ));
                    }
                }
            }
            if message.role == selora_ai::Role::Tool
                && !message
                    .tool_call_id
                    .as_ref()
                    .is_some_and(|id| pending.remove(id))
            {
                return Err(HostError::new(
                    "agent checkpoint has an unmatched tool result",
                ));
            }
        }
        if !pending.is_empty() {
            return Err(HostError::new(
                "agent checkpoint contains unanswered tool calls",
            ));
        }
        self.lock(&self.conversations)
            .insert(agent.to_owned(), history);
        Ok(())
    }

    fn is_cancelled(&self) -> bool {
        WorkflowHost::is_cancelled(self)
    }

    fn tokens(&self) -> u64 {
        WorkflowHost::tokens(self)
    }

    fn ask(
        &self,
        agent: &str,
        persona: Option<&str>,
        instructions: &str,
    ) -> Result<Value, HostError> {
        self.ask_impl(agent, persona, instructions, None)
            .map_err(|error| self.redact_error(error))
    }

    fn ask_with_schema(
        &self,
        agent: &str,
        persona: Option<&str>,
        instructions: &str,
        schema: Option<&Value>,
    ) -> Result<Value, HostError> {
        self.ask_impl(agent, persona, instructions, schema)
            .map_err(|error| self.redact_error(error))
    }

    fn world_run(
        &self,
        cmd: &str,
        args: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<CommandOutput, HostError> {
        self.world_run_inner(cmd, args, timeout_ms)
            .map_err(|error| self.redact_error(error))
    }

    fn files_glob(&self, pattern: &str) -> Result<Vec<String>, HostError> {
        self.files_glob_inner(pattern)
            .map_err(|error| self.redact_error(error))
    }

    fn files_grep(&self, pattern: &str, glob: Option<&str>) -> Result<Vec<GrepMatch>, HostError> {
        self.files_grep_inner(pattern, glob)
            .map_err(|error| self.redact_error(error))
    }

    fn files_read(&self, path: &str) -> Result<String, HostError> {
        self.files_read_inner(path)
            .map_err(|error| self.redact_error(error))
    }

    fn git_changed_files(&self, base: Option<&str>) -> Result<Vec<String>, HostError> {
        self.git_changed_files_inner(base)
            .map_err(|error| self.redact_error(error))
    }

    fn git_diff(&self, base: Option<&str>, path: Option<&str>) -> Result<String, HostError> {
        self.git_diff_inner(base, path)
            .map_err(|error| self.redact_error(error))
    }

    fn git_status(&self) -> Result<String, HostError> {
        self.git_status_inner()
            .map_err(|error| self.redact_error(error))
    }

    fn git_log(&self, count: Option<usize>) -> Result<Vec<GitCommit>, HostError> {
        self.git_log_inner(count)
            .map_err(|error| self.redact_error(error))
    }

    fn report(&self, item: &Value, artifact_id: Option<&str>) -> Result<(), HostError> {
        self.report_inner(item, artifact_id)
            .map_err(|error| self.redact_error(error))
    }

    fn artifact_markdown(
        &self,
        id: &str,
        title: Option<&str>,
        content: &str,
    ) -> Result<(), HostError> {
        self.artifact_markdown_inner(id, title, content)
            .map_err(|error| self.redact_error(error))
    }

    fn artifact_file(&self, id: &str, title: Option<&str>, path: &str) -> Result<(), HostError> {
        self.artifact_file_inner(id, title, path)
            .map_err(|error| self.redact_error(error))
    }

    fn log(&self, message: &str) {
        self.lock(&self.logs).push(self.redact(message));
    }

    fn cost_usd(&self) -> f64 {
        f64::from_bits(self.cost_bits.load(Ordering::SeqCst))
    }
}

// Centralize error redaction at every public Host boundary, including errors
// from free path/glob helpers and child stderr, not just network errors.
impl WorkflowHost {
    fn world_run_inner(
        &self,
        cmd: &str,
        args: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<CommandOutput, HostError> {
        self.ensure_running()?;
        if cmd.trim().is_empty() {
            return Err(HostError::new("`world.run` needs a command"));
        }

        let program = powershell_program();
        // PowerShell reparses arguments after `-Command` as code. Keep the
        // confirmed command as code, but encode runtime arguments as data.
        let script = crate::workflow_command::invocation(cmd, args).map_err(HostError::new)?;
        let arguments = [
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            script,
        ];
        // The child gets exactly the allowlist, never the ambient environment
        // (and therefore never the gateway key).
        let environment: Vec<(&str, OsString)> = ALLOWED_ENV_VARS
            .iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (*name, value)))
            .collect();
        // `WorldProcess::spawn` creates the job first and starts the child
        // suspended. It returns only after the child is assigned to the job and
        // resumed, so every descendant of this child is contained.
        let mut child = WorldProcess::spawn(program, &arguments, &self.root, &environment)
            .map_err(|err| {
                HostError::new(format!(
                    "cannot start PowerShell (`{program}`) for `world.run`: {}",
                    err.message
                ))
            })?;
        let budget = Arc::new(Mutex::new(self.config.max_output_bytes));
        let stdout_reader = child
            .take_stdout()
            .map(|pipe| spawn_capture(pipe, Arc::clone(&budget)));
        let stderr_reader = child.take_stderr().map(|pipe| spawn_capture(pipe, budget));

        let timeout = Duration::from_millis(timeout_ms.unwrap_or(self.config.default_timeout_ms));
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(err) => {
                    // The wait itself failed, so the child's state is unknown:
                    // terminate the job, then fall back to the direct kill.
                    let containment = stop_process_tree(&mut child);
                    join_capture(stdout_reader);
                    join_capture(stderr_reader);
                    return Err(with_containment(
                        format!("cannot wait for `{cmd}`: {}", err.message),
                        containment,
                    ));
                }
            }
            if self.is_cancelled() {
                let containment = stop_process_tree(&mut child);
                join_capture(stdout_reader);
                join_capture(stderr_reader);
                return Err(with_containment(cancelled_error().message, containment));
            }
            if started.elapsed() >= timeout {
                let containment = stop_process_tree(&mut child);
                join_capture(stdout_reader);
                join_capture(stderr_reader);
                return Err(with_containment(
                    format!(
                        "`{cmd}` timed out after {} ms (the process tree was killed)",
                        timeout.as_millis()
                    ),
                    containment,
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        };

        // The direct child is gone; kill anything it left behind *before* the
        // capture joins, so a grandchild holding the inherited pipes cannot
        // block the run. The job handle stays open until both readers are done.
        let containment = child.terminate_job().err();
        let stdout = join_capture(stdout_reader).unwrap_or_default();
        let stderr = join_capture(stderr_reader).unwrap_or_default();
        if let Some(error) = containment {
            // Containment is authoritative: a successful command whose leftover
            // tree could not be killed is a failure, not a success with debris.
            return Err(error);
        }
        Ok(CommandOutput {
            exit_code: status as i32,
            stdout: decode_process_text(&stdout),
            stderr: decode_process_text(&stderr),
        })
    }

    fn files_glob_inner(&self, pattern: &str) -> Result<Vec<String>, HostError> {
        self.ensure_running()?;
        let pattern = normalize_glob(pattern)?;

        let mut files = Vec::new();
        collect_files(&build_tree(&self.root), &mut files);
        let mut paths = Vec::new();
        let mut bytes = 0usize;
        for file in files {
            let Some(relative) = self.workspace_relative(&file) else {
                continue;
            };
            if !glob_matches(&pattern, &relative) {
                continue;
            }
            if paths.len() >= self.config.max_items
                || bytes.saturating_add(relative.len()) > self.config.max_read_bytes
            {
                break;
            }
            bytes += relative.len();
            paths.push(relative);
        }
        paths.sort();
        Ok(paths)
    }

    fn files_grep_inner(
        &self,
        pattern: &str,
        glob: Option<&str>,
    ) -> Result<Vec<GrepMatch>, HostError> {
        self.ensure_running()?;
        let glob = glob.map(normalize_glob).transpose()?;
        let hits = search_in_files(&self.root, &SearchQuery::new(pattern))
            .map_err(|err| HostError::new(format!("workspace search failed: {err}")))?;

        let mut matches = Vec::new();
        let mut bytes = 0usize;
        for hit in hits {
            let Some(relative) = self.workspace_relative(&hit.path) else {
                continue;
            };
            if glob
                .as_deref()
                .is_some_and(|glob| !glob_matches(glob, &relative))
            {
                continue;
            }
            if matches.len() >= self.config.max_items
                || bytes.saturating_add(hit.line_text.len()) > self.config.max_read_bytes
            {
                break;
            }
            bytes += hit.line_text.len();
            matches.push(GrepMatch {
                path: relative,
                line: hit.line_number as u64,
                text: hit.line_text,
            });
        }
        Ok(matches)
    }

    fn files_read_inner(&self, path: &str) -> Result<String, HostError> {
        self.ensure_running()?;
        let resolved = self.resolve_existing(path)?;
        let metadata = std::fs::metadata(&resolved)
            .map_err(|err| HostError::new(format!("`{path}` cannot be read: {err}")))?;
        if !metadata.is_file() {
            return Err(HostError::new(format!("`{path}` is not a file")));
        }
        if metadata.len() > self.config.max_read_bytes as u64 {
            return Err(HostError::new(format!(
                "`{path}` is {} bytes, over the {}-byte cap",
                metadata.len(),
                self.config.max_read_bytes
            )));
        }
        let bytes = std::fs::read(&resolved)
            .map_err(|err| HostError::new(format!("`{path}` cannot be read: {err}")))?;
        String::from_utf8(bytes).map_err(|_| HostError::new(format!("`{path}` is not UTF-8 text")))
    }

    fn git_changed_files_inner(&self, base: Option<&str>) -> Result<Vec<String>, HostError> {
        self.ensure_running()?;
        match base {
            None => {
                let repo = self.open_repo()?;
                let status = repo
                    .status()
                    .map_err(|err| HostError::new(format!("git status failed: {err}")))?;
                let mut paths: Vec<String> = status
                    .entries
                    .iter()
                    .filter_map(|entry| self.workspace_relative(&entry.abs_path))
                    .collect();
                paths.sort();
                paths.dedup();
                Ok(paths)
            }
            Some(base) => {
                let base = validate_rev(base)?;
                let stdout = self.git_stdout(&[
                    "diff".to_owned(),
                    "--no-ext-diff".to_owned(),
                    "--no-textconv".to_owned(),
                    "--name-only".to_owned(),
                    base,
                    "--".to_owned(),
                    ".".to_owned(),
                ])?;
                Ok(cap_lines(&stdout, &self.config))
            }
        }
    }

    fn git_diff_inner(&self, base: Option<&str>, path: Option<&str>) -> Result<String, HostError> {
        self.ensure_running()?;
        let relative = path.map(workspace_relative_path).transpose()?;

        match base {
            None => {
                let repo = self.open_repo()?;
                let status = repo
                    .status()
                    .map_err(|err| HostError::new(format!("git status failed: {err}")))?;
                let mut parts = Vec::new();
                if let Some(relative) = &relative {
                    self.push_workspace_diff(&repo, relative, &mut parts)?;
                } else {
                    for entry in &status.entries {
                        let Some(entry_path) = self.workspace_relative(&entry.abs_path) else {
                            continue;
                        };
                        self.push_workspace_diff(&repo, &entry_path, &mut parts)?;
                    }
                }
                self.reject_oversize_diff(
                    parts
                        .into_iter()
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
            Some(base) => {
                let base = validate_rev(base)?;
                let mut args = vec![
                    "diff".to_owned(),
                    "--no-ext-diff".to_owned(),
                    "--no-textconv".to_owned(),
                    base,
                ];
                match &relative {
                    Some(relative) => {
                        args.push("--".to_owned());
                        args.push(relative.clone());
                    }
                    None => {
                        // `.` keeps a subdirectory workspace from reporting the
                        // whole enclosing repository.
                        args.push("--".to_owned());
                        args.push(".".to_owned());
                    }
                }
                let diff = self.git_stdout(&args)?;
                self.reject_oversize_diff(diff)
            }
        }
    }

    fn git_status_inner(&self) -> Result<String, HostError> {
        self.ensure_running()?;
        let repo = self.open_repo()?;
        let status = repo
            .status()
            .map_err(|err| HostError::new(format!("git status failed: {err}")))?;

        let mut out = String::new();
        match status.branch.as_deref() {
            Some(branch) => out.push_str(&format!("On branch {branch}\n")),
            None => out.push_str("HEAD is detached\n"),
        }
        let entries: Vec<_> = status
            .entries
            .iter()
            .filter(|entry| self.workspace_relative(&entry.abs_path).is_some())
            .collect();
        if entries.is_empty() {
            out.push_str("nothing to commit, working tree clean\n");
            return Ok(out);
        }
        out.push_str("Changes:\n");
        for entry in entries {
            let Some(relative) = self.workspace_relative(&entry.abs_path) else {
                continue;
            };
            let (staged, worktree) = if entry.is_untracked() {
                ('?', '?')
            } else {
                (entry.staged.letter(), entry.worktree.letter())
            };
            out.push_str(&format!("{staged}{worktree} {relative}\n"));
        }
        Ok(truncate_text(out, self.config.max_read_bytes))
    }

    fn git_log_inner(&self, count: Option<usize>) -> Result<Vec<GitCommit>, HostError> {
        self.ensure_running()?;
        let count = count
            .unwrap_or(DEFAULT_GIT_LOG_COUNT)
            .clamp(1, MAX_GIT_LOG_COUNT);
        let output = self.git_command(&[
            "log".to_owned(),
            "-n".to_owned(),
            count.to_string(),
            "--format=%h%x09%s".to_owned(),
            "--".to_owned(),
        ])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // An unborn HEAD (fresh repository) has no history yet; that is an
            // empty log, not a failure.
            if stderr.contains("does not have any commits")
                || stderr.contains("unknown revision")
                || stderr.contains("bad default revision")
            {
                return Ok(Vec::new());
            }
            return Err(HostError::new(format!(
                "`git log` failed: {}",
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut commits = Vec::new();
        let mut bytes = 0usize;
        for line in stdout.lines() {
            let Some((sha, summary)) = line.split_once('\t') else {
                continue;
            };
            if bytes.saturating_add(sha.len() + summary.len()) > self.config.max_read_bytes {
                break;
            }
            bytes += sha.len() + summary.len();
            commits.push(GitCommit {
                sha: sha.to_owned(),
                summary: summary.to_owned(),
            });
        }
        Ok(commits)
    }

    fn report_inner(&self, item: &Value, artifact_id: Option<&str>) -> Result<(), HostError> {
        self.ensure_running()?;
        let size = serde_json::to_vec(item).map_or(usize::MAX, |encoded| encoded.len());
        if size > MAX_REPORT_ITEM_BYTES {
            return Err(HostError::new(format!(
                "report item is {size} bytes, over the {MAX_REPORT_ITEM_BYTES}-byte cap"
            )));
        }
        let mut reports = self.lock(&self.reports);
        if reports.len() >= MAX_REPORT_ITEMS {
            return Err(HostError::new(format!(
                "the run already holds {MAX_REPORT_ITEMS} report items"
            )));
        }
        let index = reports.len() as u64 + 1;
        reports.push(ReportItem {
            index,
            item: item.clone(),
            artifact_id: artifact_id.map(str::to_owned),
        });
        Ok(())
    }

    fn artifact_markdown_inner(
        &self,
        id: &str,
        title: Option<&str>,
        content: &str,
    ) -> Result<(), HostError> {
        self.ensure_running()?;
        if content.len() > MAX_ARTIFACT_BYTES {
            return Err(HostError::new(format!(
                "artifact `{id}` is {} bytes, over the {MAX_ARTIFACT_BYTES}-byte cap",
                content.len()
            )));
        }
        self.push_artifact(Artifact {
            id: id.to_owned(),
            title: title.map(str::to_owned),
            content: ArtifactContent::Markdown(content.to_owned()),
        })
    }

    fn artifact_file_inner(
        &self,
        id: &str,
        title: Option<&str>,
        path: &str,
    ) -> Result<(), HostError> {
        self.ensure_running()?;
        // The path is a display/render reference for the panel, not a read the
        // host performs; resolve it now so a typo or an escape fails at the
        // facade instead of in the panel.
        let resolved = self.resolve_existing(path)?;
        let relative = self
            .workspace_relative(&resolved)
            .ok_or_else(|| outside_error(path))?;
        self.push_artifact(Artifact {
            id: id.to_owned(),
            title: title.map(str::to_owned),
            content: ArtifactContent::File(relative),
        })
    }
}

/* ------------------------------ helpers ------------------------------- */

fn cancelled_error() -> HostError {
    HostError::new("workflow run was cancelled")
}

/// Stop a `world.run` tree: terminate the Job Object first (authoritative),
/// then kill and reap the direct child as a fallback.
///
/// Returns the job's failure, if any, so the caller can still report a typed
/// containment error next to the timeout/cancel/wait failure it already has.
fn stop_process_tree(child: &mut WorldProcess) -> Option<HostError> {
    let containment = child.terminate_job().err();
    child.kill_and_reap();
    containment
}

/// Append a containment failure to the primary `world.run` failure.
fn with_containment(message: impl Into<String>, containment: Option<HostError>) -> HostError {
    let message = message.into();
    match containment {
        Some(error) => HostError::new(format!("{message}; additionally: {}", error.message)),
        None => HostError::new(message),
    }
}

fn outside_error(value: &str) -> HostError {
    HostError::new(format!("`{value}` is outside the workspace root"))
}

/// Strip a Windows extended-length (`\\?\`) or device (`\\.\`) prefix that has
/// already had its backslashes normalized to forward slashes.
fn strip_extended_prefix(text: &str) -> &str {
    text.strip_prefix("//?/")
        .or_else(|| text.strip_prefix("//./"))
        .unwrap_or(text)
}

/// Validate and normalize a workspace-relative file path: forward slashes, no
/// `.`/`..` segments, no absolute form, no drive letters (the rule
/// `selora-protocol` uses for every workspace path crossing the bridge).
fn workspace_relative_path(value: &str) -> Result<String, HostError> {
    let mut relative = value.trim().replace('\\', "/");
    while let Some(rest) = relative.strip_prefix("./") {
        relative = rest.to_owned();
    }
    selora_protocol::validate_workspace_relative_path(&relative).map_err(|_| {
        HostError::new(format!(
            "`{value}` is not a workspace-relative path (absolute paths, `..`, and drive letters are rejected)"
        ))
    })?;
    Ok(relative)
}

/// Validate and normalize a `files.*` glob. Glob patterns are matched against
/// workspace-relative paths; `..` may not appear anywhere.
fn normalize_glob(value: &str) -> Result<String, HostError> {
    let mut pattern = value.trim().replace('\\', "/");
    while let Some(rest) = pattern.strip_prefix("./") {
        pattern = rest.to_owned();
    }
    if pattern.is_empty() {
        return Err(HostError::new("the glob pattern must not be empty"));
    }
    let absolute =
        pattern.starts_with('/') || pattern.as_bytes().get(1).is_some_and(|byte| *byte == b':');
    if absolute || pattern.split('/').any(|segment| segment == "..") {
        return Err(HostError::new(format!(
            "the glob pattern `{value}` must stay inside the workspace (no absolute paths, no `..`)"
        )));
    }
    Ok(pattern)
}

/// Collect every file under `node` in the tree's (sorted, gitignore-aware)
/// pre-order.
fn collect_files(node: &FileTreeNode, out: &mut Vec<PathBuf>) {
    if !node.is_dir {
        out.push(node.path.clone());
        return;
    }
    for child in &node.children {
        collect_files(child, out);
    }
}

/// Gitignore-style glob matching over a workspace-relative path.
///
/// `**` matches zero or more path segments; `*` and `?` match within one
/// segment. A pattern with no `/` matches the file name at any depth, so
/// `"*.rs"` is the useful thing a script expects and `"src/**/*.rs"` is the
/// explicit form.
fn glob_matches(pattern: &str, path: &str) -> bool {
    let path_segments: Vec<&str> = path.split('/').collect();
    if !pattern.contains('/') {
        return path_segments
            .last()
            .is_some_and(|name| segment_matches(pattern, name));
    }
    let pattern_segments: Vec<&str> = pattern.split('/').collect();
    match_segments(&pattern_segments, &path_segments)
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| match_segments(rest, &path[skip..])),
        Some((segment, rest)) => match path.split_first() {
            Some((name, tail)) => segment_matches(segment, name) && match_segments(rest, tail),
            None => false,
        },
    }
}

fn segment_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            p += 1;
            mark = n;
        } else if let Some(star_at) = star {
            p = star_at + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Reject a `base` that would be read as a git option or carry shell-ish
/// bytes. `HEAD~1`, `main`, `origin/main` and `v1.2.3` all pass.
fn validate_rev(base: &str) -> Result<String, HostError> {
    let base = base.trim();
    let valid = !base.is_empty()
        && !base.starts_with('-')
        && base.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '~' | '^' | '@')
        });
    if valid {
        Ok(base.to_owned())
    } else {
        Err(HostError::new(format!(
            "`{base}` is not a safe git revision (letters, digits, `/._~^@-`; never a leading `-`)"
        )))
    }
}

fn cap_lines(text: &str, config: &WorkflowHostConfig) -> Vec<String> {
    let mut lines = Vec::new();
    let mut bytes = 0usize;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if lines.len() >= config.max_items
            || bytes.saturating_add(line.len()) > config.max_read_bytes
        {
            break;
        }
        bytes += line.len();
        lines.push(line.to_owned());
    }
    lines
}

fn truncate_text(mut text: String, limit: usize) -> String {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// Parse a subagent answer. The prompt asks for JSON only; models still add a
/// fence or a sentence now and then, so a fenced block and the first complete
/// JSON value are accepted — never coerced, the engine still validates the
/// shape against the script's schema.
fn parse_answer(reply: &str) -> Result<Value, HostError> {
    let text = reply.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return Ok(value);
    }
    let fenced = strip_fence(text);
    if let Ok(value) = serde_json::from_str(fenced) {
        return Ok(value);
    }
    if let Some(slice) = json_slice(fenced) {
        if let Ok(value) = serde_json::from_str(slice) {
            return Ok(value);
        }
    }
    let preview: String = text.chars().take(160).collect();
    Err(HostError::new(format!(
        "the answer was not JSON (got `{}`)",
        preview.replace(['\n', '\r'], " ")
    )))
}

fn strip_fence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let rest = rest
        .strip_prefix("json")
        .or_else(|| rest.strip_prefix("JSON"))
        .unwrap_or(rest)
        .trim_start();
    rest.strip_suffix("```")
        .map(str::trim)
        .unwrap_or(rest.trim())
}

fn json_slice(text: &str) -> Option<&str> {
    let start = text.find(['{', '['])?;
    let open = text[start..].chars().next()?;
    let close = if open == '{' { '}' } else { ']' };
    let end = text.rfind(close)?;
    (end > start).then_some(&text[start..=end])
}

/// `pwsh` when it resolves on PATH, else Windows PowerShell.
fn powershell_program() -> &'static str {
    static PROGRAM: OnceLock<&'static str> = OnceLock::new();
    PROGRAM.get_or_init(|| {
        if program_on_path("pwsh") {
            "pwsh"
        } else {
            "powershell"
        }
    })
}

fn program_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions: &[&str] = if cfg!(windows) {
        &["exe", "cmd", "bat", "com"]
    } else {
        &[""]
    };
    std::env::split_paths(&path).any(|directory| {
        extensions.iter().any(|extension| {
            let candidate = if extension.is_empty() {
                directory.join(name)
            } else {
                directory.join(format!("{name}.{extension}"))
            };
            candidate.is_file()
        })
    })
}

/// Read `reader` while charging every captured byte to the shared budget, so
/// stdout+stderr together never exceed the configured cap. Reads continue
/// after the budget is exhausted (and bytes are discarded) so the child never
/// blocks on a full pipe.
fn spawn_capture<R: Read + Send + 'static>(
    mut reader: R,
    budget: Arc<Mutex<usize>>,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    let take = {
                        let mut remaining = budget
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        let take = read.min(*remaining);
                        *remaining -= take;
                        take
                    };
                    captured.extend_from_slice(&chunk[..take]);
                }
                Err(_) => break,
            }
        }
        captured
    })
}

fn join_capture(handle: Option<std::thread::JoinHandle<Vec<u8>>>) -> Option<Vec<u8>> {
    handle.and_then(|handle| handle.join().ok())
}

/// Decode a child's bytes: Windows PowerShell writes UTF-16LE when its stdout
/// is redirected, `pwsh` and native commands write UTF-8. A UTF-16 BOM or a
/// run of NUL high bytes picks the wide decode; everything else is lossy UTF-8.
fn decode_process_text(bytes: &[u8]) -> String {
    let looks_wide = bytes.len() >= 2
        && bytes.len().is_multiple_of(2)
        && (bytes.starts_with(&[0xFF, 0xFE]) || {
            let all_pairs = bytes.as_chunks::<2>().0;
            let pairs = all_pairs.len().min(128);
            let zeros = all_pairs
                .iter()
                .take(128)
                .filter(|pair| pair[1] == 0)
                .count();
            pairs > 0 && zeros * 2 > pairs
        });
    let text = if looks_wide {
        let body = if bytes.starts_with(&[0xFF, 0xFE]) {
            &bytes[2..]
        } else {
            bytes
        };
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_start_matches('\u{feff}').to_owned()
}

/* -------------------------------- tests -------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;
    use selora_ai::Role;
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::AtomicUsize;

    fn config() -> WorkflowHostConfig {
        WorkflowHostConfig::new("http://127.0.0.1:1", "test-key", "test-model")
    }

    #[test]
    fn agent_checkpoint_restores_full_tool_history_before_followup() {
        let workspace = tempfile::tempdir().unwrap();
        let original = WorkflowHost::new(workspace.path().to_owned(), config());
        let mut tool_message = ChatMessage::assistant("");
        tool_message.tool_calls = Some(vec![selora_ai::ToolCall {
            id: "read-1".into(),
            name: "read_file".into(),
            arguments: "{\"path\":\"src/lib.rs\"}".into(),
        }]);
        let history = vec![
            ChatMessage::system("test persona"),
            ChatMessage::user("inspect"),
            tool_message,
            ChatMessage::tool_result("read-1", "fn main() {}"),
            ChatMessage::assistant("{\"ok\":true}"),
        ];
        original
            .lock(&original.conversations)
            .insert("reviewer".into(), history.clone());
        let snapshot = original.snapshot_agent("reviewer").unwrap().unwrap();
        assert!(original.snapshot_agent("missing").unwrap().is_none());
        let expected = history.clone();
        let restored = WorkflowHost::new(
            workspace.path().to_owned(),
            config().with_chat(move |_model, messages| {
                assert_eq!(&messages[..expected.len()], expected.as_slice());
                assert!(messages.last().unwrap().content.contains("follow up"));
                Ok("{\"ok\":true}".into())
            }),
        );
        restored.restore_agent("reviewer", &snapshot).unwrap();
        assert_eq!(restored.conversation("reviewer").unwrap(), history);
        restored.ask("reviewer", None, "follow up").unwrap();
        assert!(restored.restore_agent("other", &snapshot).is_err());
        let other = tempfile::tempdir().unwrap();
        assert!(WorkflowHost::new(other.path().to_owned(), config())
            .restore_agent("reviewer", &snapshot)
            .is_err());
        let mut invalid = snapshot.clone();
        invalid["conversation"][3]["tool_call_id"] = json!("wrong-id");
        assert!(restored.restore_agent("reviewer", &invalid).is_err());
        let mut invalid = snapshot;
        invalid["conversation"] = json!([]);
        assert!(restored.restore_agent("reviewer", &invalid).is_err());
    }

    fn write_file(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, text).expect("write fixture");
    }

    fn init_repo(root: &Path) -> git2::Repository {
        let mut options = git2::RepositoryInitOptions::new();
        options.initial_head("main");
        let repo = git2::Repository::init_opts(root, &options).expect("git init");
        {
            let mut config = repo.config().expect("repo config");
            config.set_str("user.name", "Selora Test").expect("name");
            config
                .set_str("user.email", "selora-test@example.com")
                .expect("email");
        }
        repo
    }

    fn commit_paths(repo: &git2::Repository, paths: &[&str], message: &str) {
        let mut index = repo.index().expect("index");
        for path in paths {
            index.add_path(Path::new(path)).expect("stage fixture");
        }
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let signature = repo.signature().expect("signature");
        let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .expect("commit fixture");
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    #[test]
    fn git_base_diff_never_executes_configured_helpers() {
        if !git_available() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let repo = init_repo(workspace.path());
        write_file(workspace.path(), "file.txt", "before\n");
        write_file(
            workspace.path(),
            ".gitattributes",
            "*.txt diff=selora-test\n",
        );
        commit_paths(&repo, &["file.txt", ".gitattributes"], "initial");
        write_file(workspace.path(), "file.txt", "after\n");
        let mut settings = repo.config().unwrap();
        settings
            .set_str("diff.external", "selora-must-not-execute-diff-helper")
            .unwrap();
        settings
            .set_str(
                "diff.selora-test.textconv",
                "selora-must-not-execute-textconv-helper",
            )
            .unwrap();
        let host = WorkflowHost::new(workspace.path().to_owned(), config());
        let diff = host.git_diff(Some("HEAD"), Some("file.txt")).unwrap();
        assert!(diff.contains("-before"));
        assert!(diff.contains("+after"));
        assert_eq!(
            host.git_changed_files(Some("HEAD")).unwrap(),
            vec!["file.txt"]
        );
    }

    #[test]
    fn config_debug_redacts_the_api_key() {
        let debug = format!("{:?}", config().with_max_items(7));
        assert!(!debug.contains("test-key"), "debug leaked the key: {debug}");
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("max_items: 7"));
    }

    #[test]
    fn files_read_is_root_contained_and_cap_rejects() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        write_file(
            workspace.path(),
            "src/lib.rs",
            "pub fn answer() -> u32 { 42 }\n",
        );
        write_file(outside.path(), "secret.txt", "outside");

        let host = WorkflowHost::new(
            workspace.path().to_path_buf(),
            config().with_max_read_bytes(32),
        );

        assert_eq!(
            host.files_read("src/lib.rs").expect("reads"),
            "pub fn answer() -> u32 { 42 }\n"
        );
        assert_eq!(
            host.files_read("./src/lib.rs").expect("normalizes `./`"),
            "pub fn answer() -> u32 { 42 }\n"
        );

        for escape in [
            "../secret.txt",
            "src/../../secret.txt",
            outside.path().to_str().expect("utf-8"),
        ] {
            let error = host
                .files_read(escape)
                .expect_err("outside path is rejected");
            assert!(
                error.message.contains("workspace-relative") || error.message.contains("outside"),
                "unexpected error for {escape}: {error}"
            );
        }
        assert!(
            host.files_read("src").is_err(),
            "a directory is not readable"
        );

        write_file(workspace.path(), "big.txt", &"x".repeat(33));
        let error = host.files_read("big.txt").expect_err("over cap");
        assert!(error.message.contains("over the 32-byte cap"), "{error}");

        // Read-only: the fixture is untouched.
        assert_eq!(
            fs::read_to_string(workspace.path().join("src/lib.rs")).expect("read back"),
            "pub fn answer() -> u32 { 42 }\n"
        );
    }

    #[test]
    fn files_glob_and_grep_stay_inside_the_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        write_file(workspace.path(), "a.rs", "fn needle() {}\n");
        write_file(workspace.path(), "b.txt", "needle in text\n");
        write_file(workspace.path(), "nested/c.rs", "// needle\n");
        write_file(workspace.path(), "nested/deep/d.rs", "fn other() {}\n");

        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());

        assert_eq!(
            host.files_glob("*.rs").expect("glob"),
            vec![
                "a.rs".to_owned(),
                "nested/c.rs".to_owned(),
                "nested/deep/d.rs".to_owned()
            ]
        );
        assert_eq!(
            host.files_glob("nested/**/*.rs").expect("glob"),
            vec!["nested/c.rs".to_owned(), "nested/deep/d.rs".to_owned()]
        );
        assert!(
            host.files_glob("../**").is_err(),
            "an escaping glob is a typed error"
        );
        assert!(host.files_glob("C:\\**").is_err());

        let hits = host.files_grep("needle", None).expect("grep");
        assert_eq!(hits.len(), 3, "hits: {hits:?}");
        assert!(hits
            .iter()
            .any(|hit| hit.path == "nested/c.rs" && hit.line == 1));

        let narrowed = host
            .files_grep("needle", Some("*.txt"))
            .expect("narrowed grep");
        assert_eq!(narrowed.len(), 1);
        assert_eq!(narrowed[0].path, "b.txt");
        assert_eq!(narrowed[0].text, "needle in text");

        assert!(host.files_grep("needle", Some("../*.rs")).is_err());
    }

    #[test]
    fn git_reads_use_the_workspace_and_reject_escapes() {
        let workspace = tempfile::tempdir().expect("workspace");
        let repo = init_repo(workspace.path());
        write_file(workspace.path(), "a.txt", "one\n");
        commit_paths(&repo, &["a.txt"], "init");
        write_file(workspace.path(), "a.txt", "one\ntwo\n");
        write_file(workspace.path(), "new.txt", "fresh\n");
        drop(repo);

        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());

        let status = host.git_status().expect("status");
        assert!(status.contains("On branch main"), "{status}");
        assert!(status.contains("a.txt"), "{status}");
        assert!(status.contains("?? new.txt"), "{status}");

        let changed = host.git_changed_files(None).expect("changed files");
        assert_eq!(changed, vec!["a.txt".to_owned(), "new.txt".to_owned()]);

        let diff = host.git_diff(None, Some("a.txt")).expect("worktree diff");
        assert!(diff.contains("+two"), "{diff}");

        assert!(host.git_diff(None, Some("../a.txt")).is_err());
        assert!(host.git_diff(None, Some("C:\\Windows\\win.ini")).is_err());
        assert!(host.git_diff(Some("--no-index"), None).is_err());
        assert!(host.git_changed_files(Some("../..")).is_err());

        // Nothing was staged or committed by the read path.
        let status_after = host.git_status().expect("status again");
        assert!(status_after.contains("?? new.txt"), "{status_after}");
    }

    #[test]
    fn git_log_and_base_diff_use_the_cli_when_git_is_available() {
        if !git_available() {
            eprintln!("skipping: no `git` on PATH");
            return;
        }
        let workspace = tempfile::tempdir().expect("workspace");
        let repo = init_repo(workspace.path());
        write_file(workspace.path(), "a.txt", "one\n");
        commit_paths(&repo, &["a.txt"], "initial commit");
        write_file(workspace.path(), "b.txt", "fresh\n");
        commit_paths(&repo, &["b.txt"], "second commit");
        write_file(workspace.path(), "a.txt", "one\ntwo\n");
        drop(repo);

        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());

        let log = host.git_log(Some(5)).expect("git log");
        assert_eq!(log.len(), 2, "commits: {log:?}");
        assert_eq!(log[0].summary, "second commit");
        assert_eq!(log[1].summary, "initial commit");
        assert!(log[0].sha.len() >= 7);

        let changed = host
            .git_changed_files(Some("HEAD"))
            .expect("changed vs HEAD");
        assert_eq!(changed, vec!["a.txt".to_owned()]);

        let diff = host
            .git_diff(Some("HEAD"), Some("a.txt"))
            .expect("diff vs HEAD");
        assert!(diff.contains("+two"), "{diff}");
    }

    #[test]
    #[cfg(windows)]
    fn world_run_clears_the_key_env_and_roots_the_cwd() {
        const SECRET: &str = "selora-workflow-secret-4242";
        std::env::set_var("SELORA_API_KEY", SECRET);
        std::env::set_var("OPENAI_API_KEY", SECRET);

        let workspace = tempfile::tempdir().expect("workspace");
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());
        let probe = "if ($env:SELORA_API_KEY) { Write-Output $env:SELORA_API_KEY } \
                     elseif ($env:OPENAI_API_KEY) { Write-Output $env:OPENAI_API_KEY } \
                     else { Write-Output 'clean' }";
        let output = host
            .world_run(probe, &[], Some(30_000))
            .expect("powershell runs");
        assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
        assert!(
            output.stdout.contains("clean"),
            "child saw a key: stdout={:?} stderr={:?}",
            output.stdout,
            output.stderr
        );
        assert!(!output.stdout.contains(SECRET));
        assert!(!output.stderr.contains(SECRET));

        let pwd = host
            .world_run("Write-Output (Get-Location).Path", &[], Some(30_000))
            .expect("pwd runs");
        let printed = PathBuf::from(pwd.stdout.trim());
        assert_eq!(
            fs::canonicalize(&printed).expect("child cwd exists"),
            fs::canonicalize(workspace.path()).expect("workspace path")
        );

        std::env::remove_var("SELORA_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    #[cfg(windows)]
    fn world_run_honors_the_timeout_and_kills_the_child() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());
        let started = Instant::now();
        let error = host
            .world_run("Start-Sleep -Seconds 30", &[], Some(800))
            .expect_err("times out");
        assert!(
            error.message.contains("timed out"),
            "unexpected error: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the child was not killed promptly: {:?}",
            started.elapsed()
        );
    }

    #[test]
    #[cfg(windows)]
    fn world_run_keeps_stdout_stderr_and_the_exit_code() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());
        let output = host
            .world_run(
                "Write-Output 'hello'; [Console]::Error.WriteLine('bad'); exit 7",
                &[],
                Some(30_000),
            )
            .expect("powershell runs");
        assert_eq!(output.exit_code, 7, "stderr: {}", output.stderr);
        assert!(output.stdout.contains("hello"), "stdout: {}", output.stdout);
        assert!(output.stderr.contains("bad"), "stderr: {}", output.stderr);
    }

    #[test]
    #[cfg(windows)]
    fn world_run_stdin_is_the_null_device() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());
        let output = host
            .world_run(
                "if ($null -eq [Console]::In.ReadLine()) { 'eof' } else { 'data' }",
                &[],
                Some(30_000),
            )
            .expect("powershell runs");
        assert!(
            output.stdout.contains("eof"),
            "stdin was not the NUL device: stdout={:?} stderr={:?}",
            output.stdout,
            output.stderr
        );
    }

    #[test]
    #[cfg(windows)]
    fn world_run_caps_stdout_and_stderr_together() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = WorkflowHost::new(
            workspace.path().to_path_buf(),
            config().with_max_output_bytes(16),
        );
        let output = host
            .world_run(
                "Write-Output ('x' * 4096); [Console]::Error.WriteLine('y' * 4096)",
                &[],
                Some(30_000),
            )
            .expect("powershell runs");
        assert!(
            output.stdout.len() + output.stderr.len() <= 16,
            "the combined cap was exceeded: {} + {} bytes",
            output.stdout.len(),
            output.stderr.len()
        );
    }

    /// A PowerShell single-quoted literal for `value` (`'` is doubled).
    #[cfg(windows)]
    fn ps_literal(value: &Path) -> String {
        format!("'{}'", value.display().to_string().replace('\'', "''"))
    }

    /// A script whose **first** statement starts a grandchild (a second
    /// PowerShell sleeping for two minutes) and records its PID; only then does
    /// it run `tail`.
    ///
    /// `-NoNewWindow` keeps `Start-Process` on `CreateProcess`, so the grandchild
    /// inherits the job like any other descendant (and inherits the capture
    /// pipes, so a failed job kill would block the reader joins). Spawning it
    /// before any other work is what catches a post-spawn job assignment: the
    /// child cannot have created it unless it was resumed first. The long sleep
    /// bounds a failed containment test instead of letting the capture join
    /// hang.
    #[cfg(windows)]
    fn grandchild_script(pid_file: &Path, tail: &str) -> String {
        format!(
            "$grand = Start-Process -FilePath {} -ArgumentList '-NoProfile',\
             '-NonInteractive','-Command','Start-Sleep -Seconds 120' -NoNewWindow -PassThru; \
             Set-Content -LiteralPath {} -Value $grand.Id; \
             {tail}",
            ps_literal(Path::new(powershell_program())),
            ps_literal(pid_file)
        )
    }

    #[cfg(windows)]
    fn process_alive(pid: u32) -> bool {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .stdin(Stdio::null())
            .output()
            .expect("tasklist runs");
        String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
    }

    /// Wait (bounded) for the grandchild's sentinel file and return its PID.
    #[cfg(windows)]
    fn wait_for_grandchild_pid(pid_file: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(text) = fs::read_to_string(pid_file) {
                if let Ok(pid) = text.trim().parse() {
                    return pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "the grandchild never wrote its PID to {pid_file:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Bounded poll: is `pid` gone by now?
    #[cfg(windows)]
    fn wait_until_gone(pid: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if !process_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        !process_alive(pid)
    }

    #[test]
    #[cfg(windows)]
    fn world_run_success_kills_leftover_descendants() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());
        let pid_file = workspace.path().join("grandchild-success.pid");
        let output = host
            .world_run(&grandchild_script(&pid_file, "exit 0"), &[], Some(30_000))
            .expect("powershell runs");
        assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
        let pid = wait_for_grandchild_pid(&pid_file);
        assert!(
            wait_until_gone(pid),
            "grandchild {pid} survived a successful `world.run`"
        );
    }

    #[test]
    #[cfg(windows)]
    fn world_run_timeout_kills_detached_grandchildren() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());
        let pid_file = workspace.path().join("grandchild-timeout.pid");
        let started = Instant::now();
        let error = host
            .world_run(
                &grandchild_script(&pid_file, "Start-Sleep -Seconds 60"),
                &[],
                Some(5_000),
            )
            .expect_err("times out");
        assert!(
            error.message.contains("timed out"),
            "unexpected error: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the run did not return promptly, so a descendant held the capture pipes: {:?}",
            started.elapsed()
        );
        let pid = wait_for_grandchild_pid(&pid_file);
        assert!(
            wait_until_gone(pid),
            "grandchild {pid} survived the timeout kill"
        );
    }

    #[test]
    #[cfg(windows)]
    fn world_run_cancel_kills_detached_grandchildren() {
        let workspace = tempfile::tempdir().expect("workspace");
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());
        let pid_file = workspace.path().join("grandchild-cancel.pid");
        let script = grandchild_script(&pid_file, "Start-Sleep -Seconds 60");
        let runner = {
            let host = Arc::clone(&host);
            std::thread::spawn(move || host.world_run(&script, &[], Some(30_000)))
        };
        let pid = wait_for_grandchild_pid(&pid_file);
        host.cancel();
        let error = runner
            .join()
            .expect("world.run thread")
            .expect_err("cancelled");
        assert!(
            error.message.contains("cancelled"),
            "unexpected error: {error}"
        );
        assert!(
            wait_until_gone(pid),
            "grandchild {pid} survived the cancel kill"
        );
    }

    #[test]
    fn ask_uses_the_injected_chat_and_keeps_per_agent_conversations() {
        let workspace = tempfile::tempdir().expect("workspace");
        let calls: Arc<Mutex<Vec<Vec<ChatMessage>>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_models: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let chat = {
            let calls = Arc::clone(&calls);
            let seen_models = Arc::clone(&seen_models);
            move |model: &str, messages: &[ChatMessage]| {
                calls.lock().expect("calls").push(messages.to_vec());
                seen_models.lock().expect("models").push(model.to_owned());
                Ok("{\"ok\":true}".to_owned())
            }
        };
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config().with_chat(chat));

        assert_eq!(
            host.ask("reviewer", Some("terse"), "Review the plan")
                .expect("first ask"),
            json!({ "ok": true })
        );
        assert_eq!(
            host.ask("reviewer", None, "Now the code")
                .expect("second ask"),
            json!({ "ok": true })
        );
        assert_eq!(
            host.ask("writer", None, "Draft the summary")
                .expect("third ask"),
            json!({ "ok": true })
        );

        let calls = calls.lock().expect("calls");
        assert_eq!(calls.len(), 3);
        assert_eq!(
            seen_models.lock().expect("models").as_slice(),
            ["test-model", "test-model", "test-model"]
        );

        // First call: system + user only.
        assert_eq!(calls[0].len(), 2);
        assert_eq!(calls[0][0].role, Role::System);
        assert!(
            calls[0][0].content.contains("read-only"),
            "system prompt names the role: {}",
            calls[0][0].content
        );
        assert!(
            calls[0][0]
                .content
                .contains(&host.workspace_root().display().to_string()),
            "system prompt names the workspace"
        );
        assert!(calls[0][0].content.contains("`reviewer`"));
        assert!(calls[0][0].content.contains("terse"));
        assert_eq!(calls[0][1].content, "Review the plan");

        // Second call on the same identity sees the whole exchange.
        assert_eq!(calls[1].len(), 4);
        assert_eq!(calls[1][2].role, Role::Assistant);
        assert_eq!(calls[1][2].content, "{\"ok\":true}");
        assert_eq!(calls[1][3].content, "Now the code");

        // A different agent starts its own conversation.
        assert_eq!(calls[2].len(), 2);
        assert!(calls[2][0].content.contains("`writer`"));
        drop(calls);

        assert_eq!(
            host.conversation("reviewer").expect("stored").len(),
            5,
            "system, user, assistant, user, assistant"
        );
        assert_eq!(host.conversation("writer").expect("stored").len(), 3);
        assert!(host.conversation("nobody").is_none());
    }

    #[test]
    fn ask_includes_the_schema_and_rejects_bad_answers() {
        let workspace = tempfile::tempdir().expect("workspace");
        let prompts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let chat = {
            let prompts = Arc::clone(&prompts);
            move |_model: &str, messages: &[ChatMessage]| {
                prompts
                    .lock()
                    .expect("prompts")
                    .push(messages.last().expect("user message").content.clone());
                Ok("```json\n{\"findings\":[]}\n```".to_owned())
            }
        };
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config().with_chat(chat));
        let schema = json!({
            "type": "object",
            "required": ["findings"],
            "properties": { "findings": { "type": "array" } }
        });
        let answer = host
            .ask_with_schema("reviewer", None, "Find issues", Some(&schema))
            .expect("fenced JSON is accepted");
        assert_eq!(answer, json!({ "findings": [] }));

        let last = prompts.lock().expect("prompts")[0].clone();
        assert!(last.contains("Find issues"));
        assert!(
            last.contains("\"findings\""),
            "schema reached the model: {last}"
        );

        let chat = |_model: &str, _messages: &[ChatMessage]| Ok("I could not do that.".to_owned());
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config().with_chat(chat));
        let error = host
            .ask("reviewer", None, "Find issues")
            .expect_err("prose is not JSON");
        assert!(error.message.contains("not JSON"), "{error}");
    }

    #[test]
    fn ask_rejects_empty_missions_and_empty_agent_names() {
        let workspace = tempfile::tempdir().expect("workspace");
        let calls = Arc::new(AtomicUsize::new(0));
        let chat = {
            let calls = Arc::clone(&calls);
            move |_model: &str, _messages: &[ChatMessage]| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("{\"ok\":true}".to_owned())
            }
        };
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config().with_chat(chat));

        for instructions in ["", "   \n\t"] {
            let error = host
                .ask("reviewer", None, instructions)
                .expect_err("empty mission");
            assert!(error.message.contains("non-empty instructions"), "{error}");
        }
        let error = host
            .ask("  ", None, "Do something")
            .expect_err("empty agent");
        assert!(error.message.contains("non-empty agent name"), "{error}");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "the model was never called"
        );
    }

    #[test]
    fn cancel_is_checked_before_ask_and_world_run() {
        let workspace = tempfile::tempdir().expect("workspace");
        let calls = Arc::new(AtomicUsize::new(0));
        let chat = {
            let calls = Arc::clone(&calls);
            move |_model: &str, _messages: &[ChatMessage]| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("{\"ok\":true}".to_owned())
            }
        };
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config().with_chat(chat));
        assert!(!host.is_cancelled());
        host.cancel();
        assert!(host.is_cancelled());

        assert!(host.ask("reviewer", None, "Do it").is_err());
        assert!(host.world_run("Write-Output hi", &[], Some(1_000)).is_err());
        assert!(host.files_read("anything").is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn reports_artifacts_logs_and_cost_are_collected() {
        let workspace = tempfile::tempdir().expect("workspace");
        write_file(workspace.path(), "src/lib.rs", "fn main() {}\n");
        let host = WorkflowHost::new(workspace.path().to_path_buf(), config());

        host.report(&json!({ "step": 1 }), None)
            .expect("report one");
        host.report(&json!({ "step": 2 }), Some("summary"))
            .expect("report two");
        host.artifact_markdown("summary", Some("Summary"), "# Done")
            .expect("markdown artifact");
        host.artifact_file("source", None, "src/lib.rs")
            .expect("file artifact");
        host.log("first line");
        host.log("second line");
        host.add_cost_usd(0.25);
        host.add_cost_usd(0.5);

        let reports = host.reports();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].index, 1);
        assert_eq!(reports[1].index, 2);
        assert_eq!(reports[1].artifact_id.as_deref(), Some("summary"));

        let artifacts = host.artifacts();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].id, "summary");
        assert_eq!(
            artifacts[0].content,
            ArtifactContent::Markdown("# Done".to_owned())
        );
        assert_eq!(
            artifacts[1].content,
            ArtifactContent::File("src/lib.rs".to_owned())
        );

        assert_eq!(host.logs(), vec!["first line", "second line"]);
        assert!((host.cost_usd() - 0.75).abs() < f64::EPSILON);

        let duplicate = host
            .artifact_markdown("summary", None, "# Again")
            .expect_err("duplicate id");
        assert!(duplicate.message.contains("already used"), "{duplicate}");

        let escape = host
            .artifact_file("outside", None, "../outside.txt")
            .expect_err("escaping artifact path");
        assert!(escape.message.contains("workspace-relative"), "{escape}");
    }

    #[test]
    fn max_concurrency_serializes_asks() {
        let workspace = tempfile::tempdir().expect("workspace");
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let chat = {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            move |_model: &str, _messages: &[ChatMessage]| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(60));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok("{\"ok\":true}".to_owned())
            }
        };
        let host = WorkflowHost::new(
            workspace.path().to_path_buf(),
            config().with_chat(chat).with_max_concurrency(1),
        );

        std::thread::scope(|scope| {
            for index in 0..3 {
                let worker = Arc::clone(&host);
                scope.spawn(move || {
                    worker
                        .ask(&format!("agent-{index}"), None, "Go")
                        .expect("ask completes");
                });
            }
        });
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }
}
