//! Fail-closed resume: unfinished asks need explicit acceptance.
//!
//! A re-dispatched ask is not exactly-once — the interrupted turn may already
//! have run tools with side effects. These tests pin the shell's preflight
//! (`incomplete_asks`) and the refusal that happens before any host call or
//! journal append.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use selora_workflow::journal::{journal_path, run_dir, Journal};
use selora_workflow::{
    incomplete_asks, prepare, run_workflow_persisted, CommandOutput, EventSink, GitCommit,
    GrepMatch, Host, HostError, IncompleteAsk, RunId, RunOptions, WorkflowError, WorkflowEvent,
    WorkflowOutcome, WorkflowSpec,
};
use serde_json::{json, Value};

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("Temp/opencode"))
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("selora-preflight-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn journal(&self, id: &RunId) -> PathBuf {
        journal_path(self.path(), id)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct TestHost {
    asks: AtomicU64,
    commands: AtomicU64,
    /// 1-based ask number that simulates a process interruption.
    panic_on_ask: u64,
}

impl Host for TestHost {
    fn ask(&self, agent: &str, _: Option<&str>, instructions: &str) -> Result<Value, HostError> {
        let count = self.asks.fetch_add(1, Ordering::SeqCst) + 1;
        if count == self.panic_on_ask {
            panic!("simulated process interruption during ask");
        }
        Ok(json!({"agent": agent, "instructions": instructions}))
    }

    fn world_run(&self, _: &str, _: &[String], _: Option<u64>) -> Result<CommandOutput, HostError> {
        self.commands.fetch_add(1, Ordering::SeqCst);
        Ok(CommandOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    fn files_glob(&self, _: &str) -> Result<Vec<String>, HostError> {
        Ok(vec![])
    }

    fn files_grep(&self, _: &str, _: Option<&str>) -> Result<Vec<GrepMatch>, HostError> {
        Ok(vec![])
    }

    fn files_read(&self, _: &str) -> Result<String, HostError> {
        Ok(String::new())
    }

    fn git_changed_files(&self, _: Option<&str>) -> Result<Vec<String>, HostError> {
        Ok(vec![])
    }

    fn git_diff(&self, _: Option<&str>, _: Option<&str>) -> Result<String, HostError> {
        Ok(String::new())
    }

    fn git_status(&self) -> Result<String, HostError> {
        Ok(String::new())
    }

    fn git_log(&self, _: Option<usize>) -> Result<Vec<GitCommit>, HostError> {
        Ok(vec![])
    }

    fn report(&self, _: &Value, _: Option<&str>) -> Result<(), HostError> {
        Ok(())
    }

    fn artifact_markdown(&self, _: &str, _: Option<&str>, _: &str) -> Result<(), HostError> {
        Ok(())
    }

    fn artifact_file(&self, _: &str, _: Option<&str>, _: &str) -> Result<(), HostError> {
        Ok(())
    }

    fn log(&self, _: &str) {}

    fn cost_usd(&self) -> f64 {
        self.asks.load(Ordering::SeqCst) as f64
    }

    fn tokens(&self) -> u64 {
        self.asks.load(Ordering::SeqCst) * 100
    }
}

fn execute(
    workspace: &Workspace,
    id: &RunId,
    spec: &WorkflowSpec,
    host: Arc<TestHost>,
    resume: bool,
    allow_incomplete_asks: bool,
) -> (Result<WorkflowOutcome, WorkflowError>, Vec<WorkflowEvent>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&events);
    let sink: EventSink = Arc::new(move |event| recorded.lock().unwrap().push(event));
    let result = run_workflow_persisted(
        spec,
        host,
        sink,
        RunOptions {
            workspace_root: workspace.0.clone(),
            run_id: id.clone(),
            resume,
            allow_incomplete_asks,
        },
    );
    let recorded_events = events.lock().unwrap().clone();
    (result, recorded_events)
}

/// Run a two-ask script and crash inside ask number `panic_on_ask`.
fn crash_on_ask(workspace: &Workspace, id: &RunId) -> WorkflowSpec {
    let spec = prepare(
        "preflight",
        r#"agent("reviewer").ask("first"); agent("fixer").ask("second");"#,
        5.0,
    )
    .unwrap()
    .spec();
    let host = Arc::new(TestHost {
        panic_on_ask: 2,
        ..TestHost::default()
    });
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(workspace, id, &spec, host, false, false)
    }));
    assert!(crashed.is_err(), "the simulated crash must unwind");
    spec
}

#[test]
fn incomplete_asks_lists_only_unfinished_asks_in_original_order() {
    let workspace = Workspace::new();
    let id = RunId::new();
    crash_on_ask(&workspace, &id);

    assert_eq!(
        incomplete_asks(workspace.path(), &id).unwrap(),
        vec![IncompleteAsk {
            step_id: "2".to_owned(),
            agent: "fixer".to_owned(),
            label: Some("second".to_owned()),
        }]
    );
    // A freshly created run with no journal cannot be proven either way.
    assert!(incomplete_asks(workspace.path(), &RunId::new()).is_err());
}

#[test]
fn default_resume_is_refused_before_any_effect() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = crash_on_ask(&workspace, &id);
    let before = fs::read(workspace.journal(&id)).unwrap();

    let host = Arc::new(TestHost::default());
    let (result, events) = execute(&workspace, &id, &spec, Arc::clone(&host), true, false);
    let error = result.unwrap_err();
    assert_eq!(error, WorkflowError::IncompleteAsks { count: 1 });
    assert!(
        error.to_string().contains("1 unfinished ask"),
        "the refusal names the count: {error}"
    );
    assert_eq!(host.asks.load(Ordering::SeqCst), 0);
    assert_eq!(host.commands.load(Ordering::SeqCst), 0);
    assert!(events.is_empty(), "refused before WorkflowEvent::Run");
    assert_eq!(
        fs::read(workspace.journal(&id)).unwrap(),
        before,
        "refused before any journal append"
    );
    assert!(Journal::load(workspace.path(), id)
        .unwrap()
        .corrupt
        .is_empty());
}

#[test]
fn explicit_allow_redispatches_exactly_once_with_fresh_host_calls() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = crash_on_ask(&workspace, &id);

    let host = Arc::new(TestHost::default());
    let (result, events) = execute(&workspace, &id, &spec, Arc::clone(&host), true, true);
    let outcome = result.unwrap();
    assert_eq!(outcome.report.status, selora_workflow::RunStatus::Completed);
    assert_eq!(
        host.asks.load(Ordering::SeqCst),
        1,
        "only the interrupted ask is paid for again"
    );
    assert_eq!(outcome.report.cost_usd, 2.0);
    assert_eq!(outcome.report.tokens, 200);
    assert!(matches!(
        events.last(),
        Some(WorkflowEvent::Finished { .. })
    ));

    let loaded = Journal::load(workspace.path(), id).unwrap();
    let completed_asks = loaded
        .entries
        .iter()
        .filter(|entry| {
            entry.kind == "engine_step_finished"
                && entry.payload["step"]["kind"] == "ask"
                && entry.payload["step"]["status"] == "ok"
        })
        .count();
    assert_eq!(completed_asks, 2, "one result per ask, never a third");
}

#[test]
fn completed_asks_are_not_listed_and_completed_runs_still_cannot_resume() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare(
        "finished",
        r#"agent("reviewer").ask("first"); agent("fixer").ask("second");"#,
        5.0,
    )
    .unwrap()
    .spec();
    let (result, _) = execute(
        &workspace,
        &id,
        &spec,
        Arc::new(TestHost::default()),
        false,
        false,
    );
    result.unwrap();

    assert!(incomplete_asks(workspace.path(), &id).unwrap().is_empty());

    let host = Arc::new(TestHost::default());
    let (result, events) = execute(&workspace, &id, &spec, Arc::clone(&host), true, false);
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("completed run cannot resume"));
    assert!(events.is_empty());
    assert_eq!(host.asks.load(Ordering::SeqCst), 0);
}

#[test]
fn damaged_interior_journal_is_refused_even_when_asks_were_allowed() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = crash_on_ask(&workspace, &id);

    let path = workspace.journal(&id);
    let mut bytes = b"not json\n".to_vec();
    bytes.extend_from_slice(&fs::read(&path).unwrap());
    fs::write(&path, &bytes).unwrap();

    let error = incomplete_asks(workspace.path(), &id).unwrap_err();
    assert!(
        error.to_string().contains("damaged interior record"),
        "{error}"
    );

    for allow in [false, true] {
        let host = Arc::new(TestHost::default());
        let (result, events) = execute(&workspace, &id, &spec, Arc::clone(&host), true, allow);
        let error = result.unwrap_err();
        assert!(
            error.to_string().contains("damaged interior record"),
            "allow={allow}: {error}"
        );
        assert_eq!(host.asks.load(Ordering::SeqCst), 0);
        assert!(events.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes, "journal was not touched");
    }
}

#[test]
fn malformed_journal_never_masquerades_as_no_asks() {
    let malformed = [
        // A readable record with an unusable payload.
        b"{\"seq\":2,\"kind\":\"engine_step_started\",\"payload\":{}}\n".to_vec(),
        // A line that is not a record at all, terminated so it is interior.
        b"{\"seq\":2,\"kind\":\"engine_step_st\n".to_vec(),
        // A truncation that destroys the record vocabulary.
        b"garbage\n".to_vec(),
    ];
    for replacement in malformed {
        let workspace = Workspace::new();
        let id = RunId::new();
        let spec = crash_on_ask(&workspace, &id);
        let path = workspace.journal(&id);
        let mut lines: Vec<Vec<u8>> = fs::read(&path)
            .unwrap()
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(<[u8]>::to_vec)
            .collect();
        // Overwrite the interrupted ask's start record, leaving records after
        // it so the damage is unambiguously interior.
        lines[1] = replacement;
        let mut bytes = Vec::new();
        for line in lines {
            bytes.extend_from_slice(&line);
            bytes.push(b'\n');
        }
        fs::write(&path, &bytes).unwrap();

        let result = incomplete_asks(workspace.path(), &id);
        assert!(
            result.is_err(),
            "malformed journal must not look like no asks: {result:?}"
        );

        let host = Arc::new(TestHost::default());
        let (result, events) = execute(&workspace, &id, &spec, Arc::clone(&host), true, false);
        assert!(result.is_err());
        assert_eq!(host.asks.load(Ordering::SeqCst), 0);
        assert!(events.is_empty());
    }
}

#[test]
fn changed_confirmation_identity_is_refused_by_preflight() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = crash_on_ask(&workspace, &id);
    let path = workspace.journal(&id);

    let mut entries: Vec<Value> = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let record = entries
        .iter_mut()
        .find(|entry| entry["kind"] == "engine_run_started")
        .unwrap();
    record["payload"]["spec"]["name"] = json!("different");
    fs::write(
        &path,
        entries
            .iter()
            .map(|entry| format!("{entry}\n"))
            .collect::<String>(),
    )
    .unwrap();

    let error = incomplete_asks(workspace.path(), &id).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("journal confirmation or engine caps changed"),
        "{error}"
    );

    let host = Arc::new(TestHost::default());
    let (result, events) = execute(&workspace, &id, &spec, Arc::clone(&host), true, false);
    assert!(result.is_err());
    assert_eq!(host.asks.load(Ordering::SeqCst), 0);
    assert!(events.is_empty());
}

#[test]
fn torn_tail_is_still_recovered_by_the_preflight() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let _spec = crash_on_ask(&workspace, &id);
    let path = workspace.journal(&id);
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(b"{\"seq\":99,\"kind\":\"engine_step_started\"");
    fs::write(&path, bytes).unwrap();

    assert_eq!(
        incomplete_asks(workspace.path(), &id).unwrap(),
        vec![IncompleteAsk {
            step_id: "2".to_owned(),
            agent: "fixer".to_owned(),
            label: Some("second".to_owned()),
        }]
    );
    assert!(run_dir(workspace.path(), &id).join("spec.json").is_file());
}
