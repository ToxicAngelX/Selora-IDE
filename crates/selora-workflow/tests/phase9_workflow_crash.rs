//! Phase 9 subagent crash harness: a real child process is killed while an
//! ask is in flight, then the parent restarts the confirmed workflow and must
//! get exactly one re-dispatch and one journaled completion.
//!
//! The child runs the same test binary (`--exact child_process_hangs_mid_ask`)
//! with `SELORA_PHASE9_CHILD=ask`. It writes `ask-started` only from inside
//! `Host::ask`, which the engine calls after journaling the step start. The
//! parent kills it there, so the crash window is after the journaled intent
//! and before the journaled result — the window resume exists for.
//!
//! PENDING (no public API on `integration` yet): the Phase 9c background
//! subagent runtime. Its parked-child/parent-cancel/late-result windows are
//! tracked in `docs/PHASE9_SOAK.md`; this harness covers the Phase 8 engine
//! that the Phase 9 runtime reuses.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use selora_workflow::journal::journal_path;
use selora_workflow::{
    incomplete_asks, prepare, run_workflow_persisted, CommandOutput, EventSink, GitCommit,
    GrepMatch, Host, HostError, RunId, RunOptions, WorkflowEvent, WorkflowOutcome, WorkflowSpec,
};
use serde_json::{json, Value};

const SPEC_SOURCE: &str = r#"agent("worker").ask("do the work");"#;
const WAIT: Duration = Duration::from_secs(15);

struct TestHost {
    asks: AtomicU64,
    marker: Option<PathBuf>,
}

impl TestHost {
    fn new(marker: Option<PathBuf>) -> Self {
        Self {
            asks: AtomicU64::new(0),
            marker,
        }
    }
}

impl Host for TestHost {
    fn ask(
        &self,
        _agent: &str,
        _persona: Option<&str>,
        _instructions: &str,
    ) -> Result<Value, HostError> {
        self.asks.fetch_add(1, Ordering::SeqCst);
        if let Some(marker) = &self.marker {
            std::fs::write(marker, b"in-ask").map_err(|error| HostError::new(error.to_string()))?;
            // The child must still be inside the ask when the parent kills it.
            loop {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        Ok(json!({"ok": true}))
    }

    fn world_run(
        &self,
        _cmd: &str,
        _args: &[String],
        _timeout_ms: Option<u64>,
    ) -> Result<CommandOutput, HostError> {
        Ok(CommandOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    fn files_glob(&self, _pattern: &str) -> Result<Vec<String>, HostError> {
        Ok(Vec::new())
    }

    fn files_grep(&self, _pattern: &str, _glob: Option<&str>) -> Result<Vec<GrepMatch>, HostError> {
        Ok(Vec::new())
    }

    fn files_read(&self, _path: &str) -> Result<String, HostError> {
        Ok(String::new())
    }

    fn git_changed_files(&self, _base: Option<&str>) -> Result<Vec<String>, HostError> {
        Ok(Vec::new())
    }

    fn git_diff(&self, _base: Option<&str>, _path: Option<&str>) -> Result<String, HostError> {
        Ok(String::new())
    }

    fn git_status(&self) -> Result<String, HostError> {
        Ok(String::new())
    }

    fn git_log(&self, _count: Option<usize>) -> Result<Vec<GitCommit>, HostError> {
        Ok(Vec::new())
    }

    fn report(&self, _item: &Value, _artifact_id: Option<&str>) -> Result<(), HostError> {
        Ok(())
    }

    fn artifact_markdown(
        &self,
        _id: &str,
        _title: Option<&str>,
        _content: &str,
    ) -> Result<(), HostError> {
        Ok(())
    }

    fn artifact_file(&self, _id: &str, _title: Option<&str>, _path: &str) -> Result<(), HostError> {
        Ok(())
    }

    fn log(&self, _message: &str) {}

    fn cost_usd(&self) -> f64 {
        self.asks.load(Ordering::SeqCst) as f64 * 0.25
    }

    fn tokens(&self) -> u64 {
        self.asks.load(Ordering::SeqCst) * 7
    }
}

fn workspace(label: &str) -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Temp/opencode");
    let root = base.join(format!(
        "selora-phase9-workflow-{label}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("workspace");
    root
}

fn options(root: &Path, run: &RunId, resume: bool) -> RunOptions {
    RunOptions {
        workspace_root: root.to_path_buf(),
        run_id: run.clone(),
        resume,
        // The crash left an ask in flight; resuming it is the point.
        allow_incomplete_asks: resume,
    }
}

fn execute(
    spec: &WorkflowSpec,
    root: &Path,
    run: &RunId,
    host: Arc<TestHost>,
    resume: bool,
) -> Result<WorkflowOutcome, selora_workflow::WorkflowError> {
    let sink: EventSink = Arc::new(|_event: WorkflowEvent| {});
    run_workflow_persisted(spec, host, sink, options(root, run, resume))
}

/// Child half. No env = no-op, so the normal suite is unaffected.
#[test]
fn child_process_hangs_mid_ask() {
    if std::env::var("SELORA_PHASE9_CHILD").as_deref() != Ok("ask") {
        return;
    }
    let root = PathBuf::from(std::env::var("SELORA_PHASE9_ROOT").expect("root env"));
    let run = RunId::from(std::env::var("SELORA_PHASE9_RUN").expect("run env"));
    let spec = prepare("phase9 crash", SPEC_SOURCE, 5.0)
        .expect("spec")
        .spec();
    let host = Arc::new(TestHost::new(Some(root.join("ask-started"))));
    let _ = execute(&spec, &root, &run, host, false);
    // The parent kills this process while `ask` sleeps; reaching here means
    // the harness broke, not the product.
    std::process::exit(9);
}

#[test]
fn process_kill_mid_ask_then_restart_dispatches_once_and_accounts_once() {
    let root = workspace("ask-crash");
    let run = RunId::new();
    let spec = prepare("phase9 crash", SPEC_SOURCE, 5.0)
        .expect("spec")
        .spec();

    let exe = std::env::current_exe().expect("test binary");
    let mut child = std::process::Command::new(exe)
        .args(["--exact", "child_process_hangs_mid_ask", "--nocapture"])
        .env("SELORA_PHASE9_CHILD", "ask")
        .env("SELORA_PHASE9_ROOT", &root)
        .env("SELORA_PHASE9_RUN", run.as_str())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn child");

    let marker = root.join("ask-started");
    let deadline = Instant::now() + WAIT;
    while !marker.is_file() {
        assert!(
            Instant::now() < deadline,
            "child never reached Host::ask; no journal produced"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // The crash window: the engine journaled the step start and is inside the
    // ask. No step finish exists yet.
    let before = selora_workflow::journal::Journal::load(&root, run.clone()).expect("load");
    assert!(
        before
            .entries
            .iter()
            .any(|entry| entry.kind == "engine_step_started"),
        "the intent must be durable before the ask"
    );
    assert!(
        !before
            .entries
            .iter()
            .any(|entry| entry.kind == "engine_step_finished"),
        "the result must not be journaled before the ask returns"
    );
    let pending = incomplete_asks(&root, &run.clone()).expect("incomplete scan");
    assert_eq!(pending.len(), 1, "exactly one interrupted ask");

    child.kill().expect("kill child");
    let _ = child.wait();

    // Restart: one re-dispatch, one completion, no duplicate paid work.
    let resumed_host = Arc::new(TestHost::new(None));
    let resumed =
        execute(&spec, &root, &run, Arc::clone(&resumed_host), true).expect("resume succeeds");
    assert_eq!(resumed.report.status, selora_workflow::RunStatus::Completed);
    assert_eq!(
        resumed_host.asks.load(Ordering::SeqCst),
        1,
        "the interrupted ask is re-dispatched exactly once"
    );
    assert_eq!(resumed.report.tokens, 7, "token accounting is not doubled");
    assert_eq!(resumed.report.cost_usd, 0.25);

    let after = selora_workflow::journal::Journal::load(&root, run.clone()).expect("load");
    assert!(
        after.corrupt.is_empty(),
        "restart left a damaged journal: {:?}",
        after.corrupt
    );
    let finished = after
        .entries
        .iter()
        .filter(|entry| entry.kind == "engine_step_finished")
        .count();
    assert_eq!(finished, 1, "one journaled completion");
    // The resume journals a second start for the same logical step (the first
    // attempt's start is the durable intent), then exactly one finish. The
    // retry is visible in the journal, but the paid work is not duplicated.
    let starts: Vec<&selora_workflow::journal::JournalEntry> = after
        .entries
        .iter()
        .filter(|entry| entry.kind == "engine_step_started")
        .collect();
    assert_eq!(starts.len(), 2, "the crashed intent plus the retry");
    assert_eq!(
        starts[0].payload["step"]["index"], starts[1].payload["step"]["index"],
        "the retry keeps the same step identity"
    );

    // A second restart of a completed run is refused before any host call.
    let extra = Arc::new(TestHost::new(None));
    let again = execute(&spec, &root, &run, Arc::clone(&extra), true);
    assert!(again.is_err(), "completed runs cannot resume");
    assert_eq!(extra.asks.load(Ordering::SeqCst), 0);
    assert!(journal_path(&root, &run).is_file());

    let _ = std::fs::remove_dir_all(root);
}
