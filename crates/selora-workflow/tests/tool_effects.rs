//! P2 gap 2: the per-run tool-effect ledger makes a resume fail closed on
//! durable evidence that a side effect may already exist.
//!
//! These tests pin the crash window (a `started` record without `finished`),
//! the preflight classification, the typed refusals before any host call, the
//! separate acknowledgement, and redaction.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use selora_workflow::journal::{journal_path, run_dir, Journal};
use selora_workflow::{
    partial_effects, prepare, redacted_tool_arguments, run_workflow_persisted, tool_effects_path,
    CommandOutput, EventSink, GitCommit, GrepMatch, Host, HostError, RunId, RunOptions,
    ToolEffectLedger, WorkflowError, WorkflowEvent, WorkflowOutcome, WorkflowSpec,
};
use serde_json::{json, Value};

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("Temp/opencode"))
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("selora-effects-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn journal(&self, id: &RunId) -> PathBuf {
        journal_path(self.path(), id)
    }

    fn effects(&self, id: &RunId) -> PathBuf {
        tool_effects_path(self.path(), id)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct EffectHost {
    asks: AtomicU64,
    /// 1-based ask number that simulates a crash after recording `started`.
    panic_after_start: u64,
    /// 1-based ask number that simulates a crash before any tool effect.
    panic_before_effect: u64,
    /// Ask numbers whose tool effects complete (recorded started+finished).
    complete_effects: Vec<u64>,
    /// Ask number that writes a secret-bearing `write_file` tool call.
    secret_ask: u64,
    ledger: Option<Arc<Mutex<ToolEffectLedger>>>,
    secret: AtomicBool,
}

impl EffectHost {
    fn with_effects(
        ledger: ToolEffectLedger,
        complete_effects: Vec<u64>,
        panic_after_start: u64,
        secret_ask: u64,
        secret: bool,
    ) -> Self {
        let secret_flag = AtomicBool::new(secret);
        Self {
            ledger: Some(Arc::new(Mutex::new(ledger))),
            complete_effects,
            panic_after_start,
            secret_ask,
            secret: secret_flag,
            ..Self::default()
        }
    }
}

impl Host for EffectHost {
    fn ask(&self, agent: &str, _: Option<&str>, instructions: &str) -> Result<Value, HostError> {
        let count = self.asks.fetch_add(1, Ordering::SeqCst) + 1;
        if count == self.panic_before_effect {
            panic!("simulated process interruption before any tool effect");
        }
        if let Some(ledger) = &self.ledger {
            let ledger = ledger.lock().unwrap_or_else(|poison| poison.into_inner());
            let step = Some(selora_workflow::ToolEffectStep {
                step_id: count.to_string(),
                agent: Some(agent.to_owned()),
            });
            let secret = self.secret.load(Ordering::SeqCst) && count == self.secret_ask;
            let args = if secret {
                redacted_tool_arguments(
                    "write_file",
                    &json!({"path": "notes.txt", "content": "sk-live-SECRET-value"}),
                )
            } else {
                json!({"path": "notes.txt", "content_bytes": 12})
            };
            let tool = if secret { "write_file" } else { "edit_file" };
            let effect = ledger
                .record_started(step.as_ref(), &format!("call-{count}"), tool, args)
                .unwrap();
            if count == self.panic_after_start {
                panic!("simulated process interruption after the effect started");
            }
            if self.complete_effects.contains(&count) {
                ledger
                    .record_finished(&effect, &format!("call-{count}"), tool, true)
                    .unwrap();
            }
        } else if count == self.panic_after_start {
            panic!("simulated process interruption during the ask");
        }
        Ok(json!({"agent": agent, "instructions": instructions}))
    }

    fn world_run(&self, _: &str, _: &[String], _: Option<u64>) -> Result<CommandOutput, HostError> {
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
    host: Arc<EffectHost>,
    resume: bool,
    allow_incomplete_asks: bool,
    acknowledge_partial_effects: bool,
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
            acknowledge_partial_effects,
        },
    );
    let recorded_events = events.lock().unwrap().clone();
    (result, recorded_events)
}

fn two_ask_spec() -> WorkflowSpec {
    prepare(
        "effects",
        r#"agent("writer").ask("first"); agent("fixer").ask("second");"#,
        5.0,
    )
    .unwrap()
    .spec()
}

/// Start a run, complete ask 1's effects, then crash inside ask 2 after its
/// `started` record was appended.
fn crash_after_started(workspace: &Workspace, id: &RunId, secret: bool) -> WorkflowSpec {
    let spec = two_ask_spec();
    let ledger = ToolEffectLedger::open_for_run(workspace.path(), id).unwrap();
    let host = Arc::new(EffectHost::with_effects(ledger, vec![1], 2, 2, secret));
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(workspace, id, &spec, host, false, false, false)
    }));
    assert!(crashed.is_err(), "the simulated crash must unwind");
    spec
}

#[test]
fn crash_leaves_a_started_record_without_finished() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let _spec = crash_after_started(&workspace, &id, false);

    let records = selora_workflow::load_tool_effects(workspace.path(), &id).unwrap();
    assert_eq!(records.len(), 3, "ask1 start+finish, ask2 start");
    assert_eq!(records[0].seq, 1);
    assert_eq!(records[1].seq, 2);
    assert_eq!(records[2].seq, 3);
    assert_eq!(records[2].phase, selora_workflow::ToolEffectPhase::Started);
    assert!(records[2].ok.is_none());
    assert!(
        !records
            .iter()
            .any(|record| record.effect_id == records[2].effect_id
                && record.phase == selora_workflow::ToolEffectPhase::Finished),
        "the crash must leave its started record unmatched"
    );

    let partial = partial_effects(workspace.path(), &id).unwrap();
    assert!(!partial.is_empty());
    assert_eq!(partial.unfinished_effects, 1);
    assert_eq!(partial.unfinished_ask_effects, 0);
    assert_eq!(partial.unattributed_effects, 0);
}

#[test]
fn partial_effects_refuse_before_any_host_call_or_journal_append() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = crash_after_started(&workspace, &id, false);
    let before = fs::read(workspace.journal(&id)).unwrap();
    let effects_before = fs::read(workspace.effects(&id)).unwrap();

    let host = Arc::new(EffectHost::default());
    let (result, events) = execute(
        &workspace,
        &id,
        &spec,
        Arc::clone(&host),
        true,
        false,
        false,
    );
    match result.unwrap_err() {
        WorkflowError::PartialEffects { effects } => assert_eq!(effects, 1),
        other => panic!("expected PartialEffects, got {other:?}"),
    }
    assert_eq!(host.asks.load(Ordering::SeqCst), 0);
    assert!(events.is_empty(), "refused before WorkflowEvent::Run");
    assert_eq!(
        fs::read(workspace.journal(&id)).unwrap(),
        before,
        "refused before any journal append"
    );
    assert_eq!(
        fs::read(workspace.effects(&id)).unwrap(),
        effects_before,
        "the refusal never rewrites the ledger"
    );
    assert!(Journal::load(workspace.path(), id)
        .unwrap()
        .corrupt
        .is_empty());
}

#[test]
fn explicit_acknowledgement_starts_exactly_once() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = crash_after_started(&workspace, &id, false);

    let host = Arc::new(EffectHost::default());
    // Acknowledgement alone also covers the unfinished-ask re-dispatch: it
    // names the same effects.
    let (result, _) = execute(&workspace, &id, &spec, Arc::clone(&host), true, false, true);
    let outcome = result.unwrap();
    assert_eq!(outcome.report.status, selora_workflow::RunStatus::Completed);
    assert_eq!(
        host.asks.load(Ordering::SeqCst),
        1,
        "only the interrupted ask is dispatched again"
    );

    let completed_asks = fs::read_to_string(workspace.journal(&id))
        .unwrap()
        .lines()
        .filter(|line| line.contains("engine_step_finished"))
        .filter(|line| line.contains(r#""kind":"ask""#) && line.contains(r#""status":"ok""#))
        .count();
    assert_eq!(completed_asks, 2, "one result per ask, never a third");
}

#[test]
fn read_only_unfinished_ask_still_resumes_normally() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = two_ask_spec();
    // No ledger records at all: both asks are read-only.
    let host = Arc::new(EffectHost {
        panic_after_start: 2,
        ..EffectHost::default()
    });
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(&workspace, &id, &spec, host, false, false, false)
    }));
    assert!(crashed.is_err());

    let partial = partial_effects(workspace.path(), &id).unwrap();
    assert!(partial.is_empty(), "read-only asks write no ledger records");

    let host = Arc::new(EffectHost::default());
    let (result, _) = execute(&workspace, &id, &spec, Arc::clone(&host), true, true, false);
    result.unwrap();
    assert_eq!(host.asks.load(Ordering::SeqCst), 1);
}

#[test]
fn completed_effects_do_not_block_resume() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = two_ask_spec();
    let ledger = ToolEffectLedger::open_for_run(workspace.path(), &id).unwrap();
    // ask1 completes its effect; ask2 crashes before recording anything (the
    // crash happened while the model was thinking, not in a tool).
    let host = Arc::new(EffectHost {
        ledger: Some(Arc::new(Mutex::new(ledger))),
        complete_effects: vec![1],
        panic_before_effect: 2,
        ..EffectHost::default()
    });
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(&workspace, &id, &spec, host, false, false, false)
    }));
    assert!(crashed.is_err());

    let partial = partial_effects(workspace.path(), &id).unwrap();
    assert!(
        partial.is_empty(),
        "a completed ask's paired effects are not partial: {partial:?}"
    );
    let host = Arc::new(EffectHost::default());
    let (result, _) = execute(&workspace, &id, &spec, Arc::clone(&host), true, true, false);
    result.unwrap();
    assert_eq!(host.asks.load(Ordering::SeqCst), 1);
}

#[test]
fn corrupt_ledger_is_a_typed_refusal_even_with_acknowledgement() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = crash_after_started(&workspace, &id, false);
    let path = workspace.effects(&id);
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(b"not json\n");
    fs::write(&path, &bytes).unwrap();

    let error = partial_effects(workspace.path(), &id).unwrap_err();
    assert!(
        matches!(error, WorkflowError::ToolEffectLedger { .. }),
        "corruption is typed, never an empty ledger: {error:?}"
    );

    for acknowledge in [false, true] {
        let host = Arc::new(EffectHost::default());
        let (result, events) = execute(
            &workspace,
            &id,
            &spec,
            Arc::clone(&host),
            true,
            false,
            acknowledge,
        );
        assert!(
            matches!(result.unwrap_err(), WorkflowError::ToolEffectLedger { .. }),
            "acknowledge={acknowledge} cannot override unverifiable evidence"
        );
        assert_eq!(host.asks.load(Ordering::SeqCst), 0);
        assert!(events.is_empty());
    }
}

#[test]
fn ledger_never_records_a_secret_or_full_file_body() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let _spec = crash_after_started(&workspace, &id, true);

    let ledger = fs::read_to_string(workspace.effects(&id)).unwrap();
    assert!(
        !ledger.contains("sk-live-SECRET-value"),
        "the secret leaked into the ledger: {ledger}"
    );
    assert!(
        ledger.contains("content_bytes"),
        "the redacted summary keeps only sizes: {ledger}"
    );
    assert!(run_dir(workspace.path(), &id).join("spec.json").is_file());
}
