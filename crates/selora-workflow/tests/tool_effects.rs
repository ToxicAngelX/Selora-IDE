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
    CommandOutput, EffectBegin, EventSink, GitCommit, GrepMatch, Host, HostError, RunId,
    RunOptions, ToolEffectLedger, ToolEffectPhase, WorkflowError, WorkflowEvent, WorkflowOutcome,
    WorkflowSpec,
};
use serde_json::{json, Value};

/// Run one `begin_effect` that must produce a fresh attempt.
fn execute_effect(
    ledger: &ToolEffectLedger,
    step: Option<&selora_workflow::ToolEffectStep>,
    call_id: &str,
    tool: &str,
    args: Value,
) -> (String, u32) {
    match ledger
        .begin_effect(step, call_id, tool, args)
        .expect("a fresh effect")
    {
        EffectBegin::Execute { effect_id, attempt } => (effect_id, attempt),
        other => panic!("expected Execute, got {other:?}"),
    }
}

struct Workspace(PathBuf);

#[test]
fn an_early_opened_ledger_rereads_sequence_before_its_first_write() {
    let root = Workspace::new();
    let id = RunId::from("reread-sequence");
    let first = ToolEffectLedger::open_for_run(root.path(), &id).unwrap();
    let stale = ToolEffectLedger::open_for_run(root.path(), &id).unwrap();
    let (effect, attempt) = execute_effect(&first, None, "one", "write_file", json!({}));
    first
        .record_finished(&effect, attempt, "one", "write_file", true)
        .unwrap();
    execute_effect(&stale, None, "two", "bash", json!({}));
    let records = selora_workflow::load_tool_effects(root.path(), &id).unwrap();
    assert_eq!(
        records.iter().map(|record| record.seq).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

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
            let (effect, attempt) = match ledger
                .begin_effect(step.as_ref(), &format!("call-{count}"), tool, args)
                .unwrap()
            {
                EffectBegin::Execute { effect_id, attempt } => (effect_id, attempt),
                other => panic!("a fresh ask must execute, got {other:?}"),
            };
            if count == self.panic_after_start {
                panic!("simulated process interruption after the effect started");
            }
            if self.complete_effects.contains(&count) {
                ledger
                    .record_finished(&effect, attempt, &format!("call-{count}"), tool, true)
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
    host: Arc<dyn Host>,
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
fn deleting_a_ledger_cannot_turn_partial_effects_into_safe_resume() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = crash_after_started(&workspace, &id, false);
    fs::remove_file(workspace.effects(&id)).unwrap();
    assert!(matches!(
        partial_effects(workspace.path(), &id),
        Err(WorkflowError::ToolEffectLedger { .. })
    ));
    let host = Arc::new(EffectHost::default());
    let before = fs::read(workspace.journal(&id)).unwrap();
    let (result, events) = execute(&workspace, &id, &spec, host.clone(), true, true, true);
    assert!(matches!(
        result,
        Err(WorkflowError::ToolEffectLedger { .. })
    ));
    assert_eq!(host.asks.load(Ordering::SeqCst), 0);
    assert!(events.is_empty());
    assert_eq!(before, fs::read(workspace.journal(&id)).unwrap());
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
        Arc::clone(&host) as Arc<dyn Host>,
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
    let (result, _) = execute(
        &workspace,
        &id,
        &spec,
        Arc::clone(&host) as Arc<dyn Host>,
        true,
        false,
        true,
    );
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
    let (result, _) = execute(
        &workspace,
        &id,
        &spec,
        Arc::clone(&host) as Arc<dyn Host>,
        true,
        true,
        false,
    );
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
    let (result, _) = execute(
        &workspace,
        &id,
        &spec,
        Arc::clone(&host) as Arc<dyn Host>,
        true,
        true,
        false,
    );
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
            Arc::clone(&host) as Arc<dyn Host>,
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

// ---------------------------------------------------------------------------
// Stable identity, replay, retries and acknowledgement evidence
// ---------------------------------------------------------------------------

#[test]
fn stable_effect_id_depends_only_on_durable_identity() {
    let step = selora_workflow::ToolEffectStep {
        step_id: "2".into(),
        agent: Some("fixer".into()),
    };
    let base = selora_workflow::stable_effect_id("run-1", Some(&step), "call-1", "write_file");
    assert_eq!(
        base,
        selora_workflow::stable_effect_id("run-1", Some(&step), "call-1", "write_file"),
        "the same durable identity must hash to the same effect id"
    );
    for other in [
        selora_workflow::stable_effect_id("run-2", Some(&step), "call-1", "write_file"),
        selora_workflow::stable_effect_id("run-1", None, "call-1", "write_file"),
        selora_workflow::stable_effect_id("run-1", Some(&step), "call-2", "write_file"),
        selora_workflow::stable_effect_id("run-1", Some(&step), "call-1", "edit_file"),
    ] {
        assert_ne!(base, other, "every durable identity field must matter");
    }
    assert!(base.starts_with("fx-"));
}

#[test]
fn a_completed_effect_is_replayed_not_executed_again() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let ledger = ToolEffectLedger::open_for_run(workspace.path(), &id).unwrap();
    let step = selora_workflow::ToolEffectStep {
        step_id: "1".into(),
        agent: Some("writer".into()),
    };
    let args = json!({"path": "notes.txt", "content_bytes": 4});
    let (effect, attempt) =
        execute_effect(&ledger, Some(&step), "call-1", "write_file", args.clone());
    ledger
        .record_finished(&effect, attempt, "call-1", "write_file", true)
        .unwrap();

    let replay = ledger
        .begin_effect(Some(&step), "call-1", "write_file", args.clone())
        .unwrap();
    assert_eq!(
        replay,
        EffectBegin::AlreadyCompleted {
            effect_id: effect.clone(),
            attempt,
        }
    );
    // A second replay must not append a second `replayed` record.
    ledger
        .begin_effect(Some(&step), "call-1", "write_file", args)
        .unwrap();

    let records = selora_workflow::load_tool_effects(workspace.path(), &id).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.phase == ToolEffectPhase::Started)
            .count(),
        1,
        "a completed effect never starts a second attempt"
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.phase == ToolEffectPhase::Replayed)
            .count(),
        1,
        "replay evidence is durable and idempotent"
    );
}

#[test]
fn an_unfinished_effect_refuses_a_retry_until_acknowledged() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let ledger = ToolEffectLedger::open_for_run(workspace.path(), &id).unwrap();
    let step = selora_workflow::ToolEffectStep {
        step_id: "2".into(),
        agent: Some("fixer".into()),
    };
    let args = json!({"path": "notes.txt", "content_bytes": 4});
    let (effect, attempt) =
        execute_effect(&ledger, Some(&step), "call-2", "write_file", args.clone());

    let refused = ledger
        .begin_effect(Some(&step), "call-2", "write_file", args.clone())
        .unwrap_err();
    assert!(
        matches!(refused, WorkflowError::ToolEffectLedger { .. }),
        "an unmatched attempt is a typed refusal: {refused:?}"
    );

    let mut ids = std::collections::BTreeSet::new();
    ids.insert(effect.clone());
    assert_eq!(
        ledger
            .acknowledge_effects(&ids, "owner", "accepted the risk")
            .unwrap(),
        1
    );
    assert_eq!(
        ledger
            .acknowledge_effects(&ids, "owner", "accepted the risk")
            .unwrap(),
        0,
        "acknowledgement evidence is idempotent"
    );
    let records = selora_workflow::load_tool_effects(workspace.path(), &id).unwrap();
    let ack = records
        .iter()
        .find(|record| record.phase == ToolEffectPhase::Acknowledged)
        .expect("durable acknowledgement evidence");
    assert_eq!(ack.effect_id, effect);
    assert_eq!(ack.attempt, attempt);
    assert_eq!(ack.step.as_ref(), Some(&step), "the ask is named");
    assert_eq!(ack.acknowledged_by.as_deref(), Some("owner"));
    assert_eq!(
        ack.acknowledged_reason.as_deref(),
        Some("accepted the risk")
    );

    let retry = ledger
        .begin_effect(Some(&step), "call-2", "write_file", args)
        .unwrap();
    assert_eq!(
        retry,
        EffectBegin::Execute {
            effect_id: effect,
            attempt: attempt + 1,
        }
    );
}

#[test]
fn a_failed_effect_refuses_a_retry_until_acknowledged() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let ledger = ToolEffectLedger::open_for_run(workspace.path(), &id).unwrap();
    let args = json!({"path": "notes.txt", "content_bytes": 4});
    let (effect, attempt) = execute_effect(&ledger, None, "call-3", "write_file", args.clone());
    ledger
        .record_finished(&effect, attempt, "call-3", "write_file", false)
        .unwrap();

    assert!(ledger
        .begin_effect(None, "call-3", "write_file", args.clone())
        .is_err());
    let mut ids = std::collections::BTreeSet::new();
    ids.insert(effect);
    ledger
        .acknowledge_effects(&ids, "owner", "accepted a partial write")
        .unwrap();
    let retry = ledger
        .begin_effect(None, "call-3", "write_file", args)
        .unwrap();
    assert_eq!(retry.attempt(), 2);
}

#[test]
fn one_call_id_cannot_be_reinterpreted_with_different_arguments() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let ledger = ToolEffectLedger::open_for_run(workspace.path(), &id).unwrap();
    execute_effect(
        &ledger,
        None,
        "call-4",
        "write_file",
        json!({"path": "a.txt"}),
    );
    let error = ledger
        .begin_effect(None, "call-4", "write_file", json!({"path": "b.txt"}))
        .unwrap_err();
    assert!(
        error.to_string().contains("different call arguments"),
        "a re-used id with different arguments fails closed: {error}"
    );
}

#[test]
fn concurrent_append_handles_keep_the_sequence_contiguous() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let ledger = Arc::new(ToolEffectLedger::open_for_run(workspace.path(), &id).unwrap());
    let mut threads = Vec::new();
    for writer in 0..4 {
        let ledger = Arc::clone(&ledger);
        threads.push(std::thread::spawn(move || {
            for call in 0..5 {
                let call_id = format!("call-{writer}-{call}");
                let (effect, attempt) =
                    execute_effect(&ledger, None, &call_id, "bash", json!({"command_bytes": 3}));
                ledger
                    .record_finished(&effect, attempt, &call_id, "bash", true)
                    .unwrap();
            }
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    let records = selora_workflow::load_tool_effects(workspace.path(), &id).unwrap();
    assert_eq!(records.len(), 40);
    assert_eq!(
        records.iter().map(|record| record.seq).collect::<Vec<_>>(),
        (1..=40).collect::<Vec<_>>(),
        "the per-run lock serializes read/append/sync"
    );
}

/// A host that records each ask'"'"'s tool effect with a stable call id and
/// counts real executions. The second ask crashes after its effect is durable
/// but before the journal records the ask result, which is the crash window
/// replay must cover.
struct ReplayHost {
    asks: AtomicU64,
    executions: AtomicU64,
    crash_once: AtomicBool,
    step: Mutex<Option<selora_workflow::ToolEffectStep>>,
    ledger: Mutex<ToolEffectLedger>,
}

impl ReplayHost {
    fn new(ledger: ToolEffectLedger) -> Self {
        Self {
            asks: AtomicU64::new(0),
            executions: AtomicU64::new(0),
            crash_once: AtomicBool::new(true),
            step: Mutex::new(None),
            ledger: Mutex::new(ledger),
        }
    }
}

impl Host for ReplayHost {
    fn note_step_started(&self, step: &selora_workflow::WorkflowStep) {
        *self
            .step
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(selora_workflow::ToolEffectStep {
            step_id: step.index.to_string(),
            agent: step.agent.clone(),
        });
    }

    fn ask(&self, agent: &str, _: Option<&str>, instructions: &str) -> Result<Value, HostError> {
        self.asks.fetch_add(1, Ordering::SeqCst);
        let step = self
            .step
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
            .expect("the engine reports the owning step before an ask");
        let call_id = format!("call-{}", step.step_id);
        {
            let ledger = self
                .ledger
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let args = json!({"path": format!("f{}.txt", step.step_id), "content_bytes": 4});
            match ledger
                .begin_effect(Some(&step), &call_id, "write_file", args)
                .unwrap()
            {
                EffectBegin::Execute { effect_id, attempt } => {
                    self.executions.fetch_add(1, Ordering::SeqCst);
                    ledger
                        .record_finished(&effect_id, attempt, &call_id, "write_file", true)
                        .unwrap();
                }
                EffectBegin::AlreadyCompleted { .. } => {}
            }
        }
        if step.step_id == "2" && self.crash_once.swap(false, Ordering::SeqCst) {
            panic!("simulated crash after a completed effect, before the journal result");
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

#[test]
fn a_crashed_ask_replays_its_completed_effect_instead_of_running_it_twice() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = two_ask_spec();
    let ledger = ToolEffectLedger::open_for_run(workspace.path(), &id).unwrap();
    let host = Arc::new(ReplayHost::new(ledger));

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(
            &workspace,
            &id,
            &spec,
            Arc::clone(&host) as Arc<dyn Host>,
            false,
            false,
            false,
        )
    }));
    assert!(crashed.is_err(), "the second ask must crash");
    assert_eq!(
        host.executions.load(Ordering::SeqCst),
        2,
        "each ask ran its tool exactly once before the crash"
    );

    let partial = partial_effects(workspace.path(), &id).unwrap();
    assert_eq!(partial.unfinished_ask_effects, 1);
    assert!(!partial.is_empty());

    // Ordinary resume refuses: the ask that would be dispatched again owns a
    // durable effect.
    let (refused, _) = execute(
        &workspace,
        &id,
        &spec,
        Arc::new(EffectHost::default()),
        true,
        true,
        false,
    );
    assert!(matches!(
        refused,
        Err(WorkflowError::PartialEffects { effects: 1 })
    ));

    assert_eq!(
        selora_workflow::acknowledge_partial_effects(
            workspace.path(),
            &id,
            "test-owner",
            "accept the effect risk",
        )
        .unwrap(),
        1
    );
    let partial = partial_effects(workspace.path(), &id).unwrap();
    assert!(partial.is_empty(), "durable acceptance clears the risk");
    assert_eq!(partial.acknowledged_effects, 1);

    let (result, _) = execute(
        &workspace,
        &id,
        &spec,
        Arc::clone(&host) as Arc<dyn Host>,
        true,
        true,
        false,
    );
    result.unwrap();
    assert_eq!(
        host.executions.load(Ordering::SeqCst),
        2,
        "the completed effect was replayed, never executed a second time"
    );

    let records = selora_workflow::load_tool_effects(workspace.path(), &id).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.phase == ToolEffectPhase::Started)
            .count(),
        2,
        "one started record per real execution"
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.phase == ToolEffectPhase::Replayed)
            .count(),
        1,
        "the replay is durable evidence"
    );
}

#[test]
fn an_acknowledging_resume_writes_per_ask_evidence_before_starting() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = crash_after_started(&workspace, &id, false);

    let host = Arc::new(EffectHost::default());
    let (result, _) = execute(&workspace, &id, &spec, host, true, false, true);
    result.unwrap();

    let records = selora_workflow::load_tool_effects(workspace.path(), &id).unwrap();
    let ack = records
        .iter()
        .find(|record| record.phase == ToolEffectPhase::Acknowledged)
        .expect("the acknowledgement is durable evidence");
    assert_eq!(
        ack.step.as_ref().map(|step| step.step_id.as_str()),
        Some("2")
    );
    assert_eq!(
        ack.step.as_ref().and_then(|step| step.agent.as_deref()),
        Some("fixer")
    );
    assert_eq!(ack.acknowledged_by.as_deref(), Some("workflow resume"));
    assert!(ack
        .acknowledged_reason
        .as_deref()
        .is_some_and(|reason| !reason.is_empty()));
}

#[test]
fn a_failed_effect_counts_as_partial_evidence() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = two_ask_spec();
    let ledger = ToolEffectLedger::open_for_run(workspace.path(), &id).unwrap();
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

    // A second tool call of the completed ask reported an error: its effect
    // may be partial, so it is durable risk even though the ask succeeded.
    let ledger = ToolEffectLedger::open_for_run(workspace.path(), &id).unwrap();
    let step = selora_workflow::ToolEffectStep {
        step_id: "1".into(),
        agent: Some("writer".into()),
    };
    let (failed, failed_attempt) = execute_effect(
        &ledger,
        Some(&step),
        "call-9",
        "write_file",
        json!({"path": "other.txt", "content_bytes": 3}),
    );
    ledger
        .record_finished(&failed, failed_attempt, "call-9", "write_file", false)
        .unwrap();

    let partial = partial_effects(workspace.path(), &id).unwrap();
    assert_eq!(partial.failed_effects, 1);
    assert!(partial.risk_count() >= 1);
}

#[test]
fn inconsistent_attempt_evidence_is_typed_corruption() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let path = workspace.effects(&id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let record = |seq: u64, phase: &str, attempt: u32, ok: Option<bool>| {
        let mut value = json!({
            "seq": seq,
            "run_id": id.as_str(),
            "effect_id": "fx-test",
            "call_id": "call-1",
            "tool": "write_file",
            "phase": phase,
            "attempt": attempt,
        });
        if let Some(ok) = ok {
            value["ok"] = json!(ok);
        }
        value.to_string()
    };
    let cases = [
        vec![record(1, "started", 1, None), record(2, "started", 1, None)],
        vec![record(1, "finished", 1, Some(true))],
        vec![
            record(1, "started", 1, None),
            record(2, "replayed", 1, None),
        ],
        vec![
            record(1, "started", 1, None),
            record(2, "finished", 1, Some(true)),
            record(3, "started", 2, None),
        ],
    ];
    for lines in cases {
        fs::write(&path, lines.join("\n") + "\n").unwrap();
        let error = selora_workflow::load_tool_effects(workspace.path(), &id).unwrap_err();
        assert!(
            matches!(error, WorkflowError::ToolEffectLedger { .. }),
            "inconsistent evidence is typed corruption: {error:?}"
        );
    }
}
