//! The bash tool (ARCHITECTURE.md §17.5) — Phase 5b's file.
//!
//! PowerShell (`pwsh` if installed, else `powershell`; the settings'
//! `agent.shell` overrides — see [`resolve_shell`]), cwd root-contained,
//! prefix-allowlist auto-run (else the router builds a command card and
//! the loop parks, §17.4), timeout with child kill, stripped environment,
//! stdout+stderr combined and capped.
//!
//! Guards in this file (each has its own test, citing its §):
//! - **Shell & cwd** (§17.5): [`run_bash`] — cwd defaults to the
//!   workspace root and resolves through the §17.2 path guard.
//! - **Timeout** (§17.5): [`DEFAULT_TIMEOUT_SECS`] / [`MAX_TIMEOUT_SECS`];
//!   on timeout the child is killed and the tool result says so.
//! - **Environment** (§17.5): [`build_child_env`] — a fixed *allowlist*
//!   copied from the app process, so the Selora key and any app secret
//!   structurally cannot reach the child.
//! - **Output** (§17.2/§17.5): 256KB combined, truncated with a notice.
//! - **Stop** (§17.3): [`BashAbortHandle`] — 5c's Stop button kills the
//!   running child (the handle is a kill *signal*; `run_bash` selects on
//!   it and kills the child it owns).

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Notify;

use crate::chat_types::ToolResult;

use super::super::router::ToolContext;
use super::fs::{cap_output, de_verbatim, resolve_rooted};
use super::MAX_OUTPUT_BYTES;

/// §17.5: default per-command timeout.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// §17.5: max per-command timeout (a larger `timeout_secs` is clamped).
pub const MAX_TIMEOUT_SECS: u64 = 600;

/// Windows `CREATE_NO_WINDOW` — never flash a console for a tool run.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// §17.5: the only environment variables the child may see. This is an
/// allowlist, not a denylist: anything not here (the Selora key, app
/// secrets, tokens) is *structurally* absent from the child's environment.
/// The set is what PowerShell, git, and cargo minimally need.
const ENV_ALLOWLIST: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "PATH",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "NUMBER_OF_PROCESSORS",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    "LANG",
    "TERM",
];

// -----------------------------------------------------------------------------
// The abort handle (§17.3 Stop button)
// -----------------------------------------------------------------------------

/// The 5c loop's kill switch for a running bash child (§17.3: "a Stop
/// button on the streaming bubble aborts immediately, killing any running
/// bash child").
///
/// The handle is a *signal*, not a raw `Child` reference: `run_bash` owns
/// the child and `select!`s on this handle; `kill()` makes the signal
/// sticky for the rest of the turn (a stopped turn must not run further
/// commands). Construct a fresh [`ToolContext`] per agent turn so each
/// turn gets a fresh handle.
#[derive(Clone, Default)]
pub struct BashAbortHandle {
    inner: Arc<AbortInner>,
}

#[derive(Default)]
struct AbortInner {
    killed: AtomicBool,
    notify: Notify,
}

impl BashAbortHandle {
    /// Signal the running bash child (if any) to die, and mark the turn's
    /// handle as killed. Called by 5c when the user presses Stop.
    pub fn kill(&self) {
        self.inner.killed.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    /// Whether [`kill`](Self::kill) has been called on this handle.
    pub fn is_killed(&self) -> bool {
        self.inner.killed.load(Ordering::SeqCst)
    }

    /// Resolves when the handle is killed. The `Notified` future is
    /// created *before* checking the flag (the tokio-documented pattern)
    /// so a kill racing the check is never missed.
    async fn wait(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.is_killed() {
                return;
            }
            notified.await;
            if self.is_killed() {
                return;
            }
        }
    }
}

// -----------------------------------------------------------------------------
// §17.5 pieces: allowlist, shell, environment, timeout
// -----------------------------------------------------------------------------

/// §17.5 allowlist check: read-only commands auto-run without a card.
/// Prefix match on the trimmed command, case-insensitive (PowerShell
/// verbs are), with a word boundary — `cargo testx` must NOT match
/// `cargo test`.
pub(crate) fn is_allowlisted(command: &str, allowlist: &[String]) -> bool {
    let command = command.trim();
    allowlist
        .iter()
        .any(|prefix| prefix_matches(command, prefix.trim()))
}

/// Case-insensitive prefix match with a whitespace/end boundary.
fn prefix_matches(command: &str, prefix: &str) -> bool {
    if prefix.is_empty() || command.len() < prefix.len() || !command.is_char_boundary(prefix.len())
    {
        return false;
    }
    if !command[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return false;
    }
    let rest = &command[prefix.len()..];
    rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')
}

/// §17.5 shell resolution: `pwsh` if installed, else `powershell`. An
/// explicit non-empty `agent.shell` setting wins (a bare
/// "powershell"/"pwsh" still resolves per §17.5, so the default settings
/// value behaves like the spec).
pub(crate) fn resolve_shell(preferred: &str) -> String {
    let preferred = preferred.trim();
    let auto = preferred.is_empty()
        || preferred.eq_ignore_ascii_case("powershell")
        || preferred.eq_ignore_ascii_case("pwsh");
    if auto {
        if find_on_path("pwsh").is_some() {
            "pwsh".to_owned()
        } else {
            "powershell".to_owned()
        }
    } else {
        preferred.to_owned()
    }
}

/// §17.5 environment: build the child's env from the fixed
/// [`ENV_ALLOWLIST`] only — the Selora key and app secrets are never
/// copied. (§17.2: "the API key never leaves the client.")
pub(crate) fn build_child_env() -> Vec<(String, String)> {
    ENV_ALLOWLIST
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| ((*key).to_owned(), value))
        })
        .collect()
}

/// §17.5 timeout: default 120s, clamped into 1..=600.
pub(crate) fn clamp_timeout(requested: Option<u64>) -> u64 {
    requested
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS)
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for candidate in [dir.join(name), dir.join(format!("{name}.exe"))] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

// -----------------------------------------------------------------------------
// The tool
// -----------------------------------------------------------------------------

/// Run one bash command (§17.5). Called by the router either directly
/// (allowlisted auto-run) or through `execute_approved` (the user
/// approved the command card) — **only** those two paths reach here.
///
/// `cwd` (the tool's argument) defaults to the workspace root and must
/// resolve inside it (§17.5 "cwd root-contained", via the §17.2 guard).
pub(crate) async fn run_bash(
    ctx: &ToolContext,
    id: &str,
    command: &str,
    timeout_secs: Option<u64>,
    cwd: Option<&str>,
) -> ToolResult {
    if command.trim().is_empty() {
        return ToolResult::error(id, "command must not be empty");
    }

    // §17.5: cwd is the workspace root, or the tool's cwd argument —
    // still root-contained (the §17.2 path guard rejects escapes).
    let cwd = match cwd {
        Some(dir) => match resolve_rooted(&ctx.root, dir) {
            Ok(p) if p.is_dir() => p,
            Ok(_) => return ToolResult::error(id, format!("cwd `{dir}` is not a directory")),
            Err(e) => return ToolResult::error(id, e),
        },
        None => ctx.root.clone(),
    };

    let shell = resolve_shell(&ctx.settings.shell);
    let mut cmd = Command::new(&shell);
    cmd.arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(command)
        // §17.5: a clean, stripped environment.
        .env_clear()
        .envs(build_child_env())
        .current_dir(de_verbatim(&cwd))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return ToolResult::error(id, format!("failed to start `{shell}`: {e}")),
    };

    // Read both pipes in tasks (each capped, so a chatty child can never
    // grow memory unboundedly); wait for exit while selecting on the
    // Stop signal and the timeout.
    let out_task = spawn_read(child.stdout.take().expect("piped stdout"));
    let err_task = spawn_read(child.stderr.take().expect("piped stderr"));
    let timeout = Duration::from_secs(clamp_timeout(timeout_secs));

    // Pin the wait future so the &mut borrow of `child` can be dropped
    // (explicitly, below) before `kill()` needs it again.
    let mut wait = Box::pin(child.wait());
    enum RunEnd {
        Status(std::io::Result<std::process::ExitStatus>),
        Stopped,
        Timeout,
    }
    let end = tokio::select! {
        status = &mut wait => RunEnd::Status(status),
        _ = ctx.abort.wait() => RunEnd::Stopped,
        _ = tokio::time::sleep(timeout) => RunEnd::Timeout,
    };
    drop(wait); // release &mut child before kill()

    match end {
        RunEnd::Stopped => {
            // start_kill is the *synchronous* TerminateProcess — tokio's
            // `kill()` is async (kill + wait) and must be awaited, which
            // the Stop path must not do unboundedly.
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
            let _ = drain(out_task).await;
            let _ = drain(err_task).await;
            ToolResult::error(id, "command stopped by the user")
        }
        RunEnd::Timeout => {
            // §17.5: on timeout the child is killed and the result says so.
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
            let _ = drain(out_task).await;
            let _ = drain(err_task).await;
            ToolResult::error(
                id,
                format!(
                    "command timed out after {}s and was killed — pass a larger timeout_secs (max {MAX_TIMEOUT_SECS}) for long-running commands",
                    timeout.as_secs()
                ),
            )
        }
        RunEnd::Status(status) => {
            // Bounded drain: a child that spawned a *grandchild* holding
            // the pipe write-end must not hang the tool past the parent's
            // exit — after 2s we report what we have.
            let out = drain(out_task).await.unwrap_or_default();
            let err = drain(err_task).await.unwrap_or_default();
            let status = match status {
                Ok(s) => s,
                Err(e) => return ToolResult::error(id, format!("command failed: {e}")),
            };
            // §17.5/§17.2: stdout+stderr combined, ≤256KB, truncated with
            // a notice (cap_output appends it). Streams are concatenated,
            // not interleaved.
            let mut text = String::from_utf8_lossy(&out).into_owned();
            if !err.is_empty() {
                text.push_str("\n[stderr]\n");
                text.push_str(&String::from_utf8_lossy(&err));
            }
            let (text, _) = cap_output(&text, MAX_OUTPUT_BYTES);
            let content = format!("(exit code {})\n{}", status.code().unwrap_or(-1), text);
            ToolResult {
                id: id.to_owned(),
                ok: true,
                content,
            }
        }
    }
}

/// Read one pipe to EOF, keeping at most `MAX_OUTPUT_BYTES + 1` bytes —
/// one byte past the cap, so the §17.2 truncation notice is emitted for
/// output that merely *equals* the cap in combined form. Memory stays
/// bounded regardless of how chatty the child is.
async fn read_capped(pipe: &mut (impl tokio::io::AsyncRead + Unpin), buf: &mut Vec<u8>) {
    let keep = MAX_OUTPUT_BYTES + 1;
    let mut scratch = [0u8; 8 * 1024];
    loop {
        match pipe.read(&mut scratch).await {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() < keep {
                    let room = keep - buf.len();
                    buf.extend_from_slice(&scratch[..n.min(room)]);
                }
                // Past the keep-limit: keep draining so the child never
                // blocks on a full pipe, but keep nothing.
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn spawn_read(
    mut pipe: impl tokio::io::AsyncRead + Unpin + Send + 'static,
) -> tokio::task::JoinHandle<Vec<u8>> {
    tokio::spawn(async move {
        let mut buf = Vec::new();
        read_capped(&mut pipe, &mut buf).await;
        buf
    })
}

/// After a kill, give the pipe tasks a moment to observe EOF and finish
/// (they normally complete immediately — the OS closes the handles).
async fn drain(task: tokio::task::JoinHandle<Vec<u8>>) -> Result<Vec<u8>, tokio::task::JoinError> {
    match tokio::time::timeout(Duration::from_secs(2), task).await {
        Ok(result) => result,
        Err(_) => Ok(Vec::new()),
    }
}

// -----------------------------------------------------------------------------
// Tests — every guard gets its own test, citing its §
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::testutil::{temp_root, touch};
    use super::*;
    use crate::agent::router::{AgentToolSettings, ToolContext};

    fn ctx(root: &Path, allowlist: &[&str]) -> ToolContext {
        ToolContext::new(
            root,
            AgentToolSettings {
                shell: "powershell".to_owned(),
                auto_approve_commands: allowlist.iter().map(|s| s.to_string()).collect(),
                brave_api_key: None,
            },
        )
    }

    // -- §17.5 allowlist ---------------------------------------------------------

    #[test]
    fn s17_5_allowlist_prefix_matches_with_boundary() {
        let allow = [
            "cargo test".to_owned(),
            "git status".to_owned(),
            "Get-Content".to_owned(),
        ];
        assert!(is_allowlisted("cargo test", &allow));
        assert!(is_allowlisted("cargo test -p selora-core", &allow));
        assert!(is_allowlisted("  cargo test  ", &allow));
        // Case-insensitive (PowerShell verbs).
        assert!(is_allowlisted("get-content Cargo.toml", &allow));
        // Word boundary: `cargo testx` must not match `cargo test`.
        assert!(!is_allowlisted("cargo testx", &allow));
        assert!(!is_allowlisted("cargo test-fixture", &allow));
        // Not in the list at all.
        assert!(!is_allowlisted("cargo publish", &allow));
        assert!(!is_allowlisted("Remove-Item -Recurse .", &allow));
        assert!(!is_allowlisted("", &allow));
        // A command that merely *contains* an allowlisted prefix later on.
        assert!(!is_allowlisted("echo cargo test", &allow));
    }

    #[test]
    fn s17_5_timeout_defaults_and_clamps() {
        assert_eq!(clamp_timeout(None), DEFAULT_TIMEOUT_SECS);
        assert_eq!(clamp_timeout(Some(0)), 1);
        assert_eq!(clamp_timeout(Some(30)), 30);
        assert_eq!(clamp_timeout(Some(100_000)), MAX_TIMEOUT_SECS);
        assert_eq!(DEFAULT_TIMEOUT_SECS, 120);
        assert_eq!(MAX_TIMEOUT_SECS, 600);
    }

    // -- §17.5 environment --------------------------------------------------------

    #[test]
    fn s17_5_env_allowlist_contains_no_secrets() {
        for key in ENV_ALLOWLIST {
            assert!(
                !key.contains("SELORA")
                    && !key.contains("KEY")
                    && !key.contains("TOKEN")
                    && !key.contains("SECRET"),
                "`{key}` must not be in the child env allowlist"
            );
        }
    }

    #[test]
    fn s17_5_env_stripping_excludes_selora_and_app_secrets() {
        // The Selora key and app secrets live in this process's env…
        std::env::set_var("SELORA_API_KEY", "sk-live-do-not-leak");
        std::env::set_var("SELORA_TEST_SECRET", "app-secret");
        let child_env = build_child_env();
        for (key, _) in &child_env {
            assert!(
                !key.starts_with("SELORA"),
                "the child env must never carry `{key}` (§17.5/§17.2: the API key never leaves the client)"
            );
        }
        assert!(
            !child_env.iter().any(|(k, _)| k == "SELORA_API_KEY"),
            "the Selora key leaked into the bash child env"
        );
        if cfg!(windows) {
            assert!(
                child_env.iter().any(|(k, _)| k == "SystemRoot"),
                "PowerShell needs SystemRoot — the allowlist is not a full env_clear"
            );
        }
        std::env::remove_var("SELORA_API_KEY");
        std::env::remove_var("SELORA_TEST_SECRET");
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn s17_5_env_stripping_holds_in_the_real_child() {
        // End-to-end: the child itself reports what it can see.
        std::env::set_var("SELORA_TEST_SECRET", "app-secret");
        let root = temp_root("bash-env");
        let ctx = ctx(&root, &[]);
        let result = run_bash(
            &ctx,
            "call_1",
            "(Get-ChildItem env: | Where-Object { $_.Name -like 'SELORA*' }).Count",
            Some(30),
            None,
        )
        .await;
        std::env::remove_var("SELORA_TEST_SECRET");
        assert!(result.ok, "{}", result.content);
        assert!(
            result.content.contains("(exit code 0)"),
            "{}",
            result.content
        );
        // PowerShell prints the count; on a stripped env it is 0. The
        // count line is the last output line before nothing else.
        let body = result.content.replace("(exit code 0)\n", "");
        assert!(
            body.trim() == "0",
            "the child must see zero SELORA_* variables, saw: {body:?}"
        );
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn bash_runs_and_reports_exit_code() {
        let root = temp_root("bash-run");
        let ctx = ctx(&root, &[]);
        let result = run_bash(
            &ctx,
            "call_1",
            "Write-Output 'hello from selora'",
            Some(30),
            None,
        )
        .await;
        assert!(result.ok, "{}", result.content);
        assert!(result.content.contains("hello from selora"));
        assert!(result.content.contains("(exit code 0)"));
        // A failing command is a successful tool run with a non-zero exit.
        let failed = run_bash(&ctx, "call_2", "exit 3", Some(30), None).await;
        assert!(failed.ok);
        assert!(failed.content.contains("(exit code 3)"));
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn s17_5_cwd_is_root_contained_and_used() {
        let root = temp_root("bash-cwd");
        touch(&root, "sub/file.txt", "x");
        let ctx = ctx(&root, &[]);
        // The tool's cwd argument is used…
        let result = run_bash(
            &ctx,
            "call_1",
            "Get-Location | Select-Object -ExpandProperty Path",
            Some(30),
            Some("sub"),
        )
        .await;
        assert!(result.ok, "{}", result.content);
        let plain = de_verbatim(&root).to_string_lossy().to_string();
        let sub = format!("{}\\sub", plain);
        assert!(
            result.content.contains(&sub),
            "expected cwd {sub}, got: {}",
            result.content
        );
        // …and must stay inside the root (§17.2 guard on the path).
        let escape = run_bash(&ctx, "call_2", "Get-Location", Some(30), Some("..")).await;
        assert!(!escape.ok);
        assert!(escape.content.contains("escapes"));
        let missing = run_bash(&ctx, "call_3", "Get-Location", Some(30), Some("nope")).await;
        assert!(!missing.ok);
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn s17_5_timeout_kills_the_child() {
        let root = temp_root("bash-timeout");
        let ctx = ctx(&root, &[]);
        let started = std::time::Instant::now();
        let result = run_bash(&ctx, "call_1", "Start-Sleep -Seconds 60", Some(1), None).await;
        assert!(!result.ok, "{}", result.content);
        assert!(
            result.content.contains("timed out after 1s"),
            "{}",
            result.content
        );
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn s17_2_bash_output_capped_at_256kb() {
        let root = temp_root("bash-cap");
        let ctx = ctx(&root, &[]);
        // ~300KB of output → truncated to the 256KB cap with a notice.
        let result = run_bash(
            &ctx,
            "call_1",
            "Write-Output ('x' * 300000)",
            Some(60),
            None,
        )
        .await;
        assert!(result.ok);
        assert!(
            result.content.contains("[output truncated at 256KB]"),
            "{}",
            result.content.chars().rev().take(120).collect::<String>()
        );
        assert!(result.content.len() < MAX_OUTPUT_BYTES + 512);
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn s17_3_stop_button_kills_the_running_child() {
        let root = temp_root("bash-stop");
        let ctx = ctx(&root, &[]);
        let handle = ctx.abort.clone();
        let task = tokio::spawn(async move {
            run_bash(&ctx, "call_1", "Start-Sleep -Seconds 60", None, None).await
        });
        // Let the child spawn, then press Stop.
        tokio::time::sleep(Duration::from_millis(800)).await;
        handle.kill();
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(15), task)
            .await
            .expect("the stopped command must finish quickly")
            .expect("task join");
        assert!(!result.ok, "{}", result.content);
        assert!(
            result.content.contains("stopped by the user"),
            "{}",
            result.content
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        // The signal is sticky for the turn.
        assert!(handle.is_killed());
    }
}
