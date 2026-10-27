//! Deterministic mock-host evidence for the ROADMAP Phase 8 acceptance steps.
//!
//! Every test here runs the real engine (`prepare` / `run_workflow` /
//! `run_workflow_persisted`) against one in-memory [`MockHost`]. No gateway, no
//! key, no window, no shell process. The names are stable because
//! `scripts/phase8-acceptance.ps1` selects them by exact name; a missing name is
//! reported PENDING by that script, never as a pass.
//!
//! Coverage split (see the script's step table):
//!
//! * step 1 - confirmation payload: script text, literal command set, cost cap,
//!   engine limits, and the grant enforced before `Run`;
//! * step 2 - approval starts the phase graph and dispatches every named agent;
//! * step 5 - a failing `world.run` gate drives the script's fix-loop rounds;
//! * step 6 - Stop ends the run before the next effect and emits one `Stopped`;
//! * step 7 - run cost/tokens match every ask the host actually billed;
//! * step 8 - the journal lands under `.selora/workflow-runs/<id>/` and the
//!   workspace-level ignore file exists while the repository ignores `/.selora/`;
//! * step 9 - `fetch`, `fs` and `process` fail as typed ambient-global errors
//!   with no host call or side effect;
//! * budget - the confirmed cap stops the run between steps.
//!
//! Approval/deny, process-tree death, resume replay/refusal and completed
//! history need the desktop Host/shell and are selected by name from their own
//! suites in the acceptance script.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use selora_workflow::journal::{JOURNAL_DIR, JOURNAL_FILE};
use selora_workflow::{
    prepare, run_workflow, run_workflow_persisted, Artifact, ArtifactContent, CommandOutput,
    CommandSet, EventSink, GitCommit, GrepMatch, Host, HostError, Limits, RunId, RunOptions,
    RunStatus, StepKind, StepStatus, WorkflowError, WorkflowEvent, WorkflowOutcome, WorkflowSpec,
};

// ---------------------------------------------------------------------------
// Deterministic mock host
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct AskCall {
    agent: String,
    persona: Option<String>,
    instructions: String,
}

#[derive(Debug, Clone, PartialEq)]
struct RunCall {
    command: String,
    args: Vec<String>,
    timeout_ms: Option<u64>,
}

struct HostState {
    asks: Vec<AskCall>,
    runs: Vec<RunCall>,
    glob_calls: usize,
    reports: Vec<Value>,
    artifacts: Vec<Artifact>,
    logs: Vec<String>,
    cost_usd: f64,
    tokens: u64,
    cancelled: bool,
    /// Answer returned by every ask (the scripts here do not branch on it).
    answer: Value,
    cost_per_ask: f64,
    tokens_per_ask: u64,
    /// Canned `world.run` exits; the last one repeats once the queue is empty.
    run_outputs: VecDeque<CommandOutput>,
}

struct MockHost {
    state: Mutex<HostState>,
}

impl MockHost {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(HostState {
                asks: Vec::new(),
                runs: Vec::new(),
                glob_calls: 0,
                reports: Vec::new(),
                artifacts: Vec::new(),
                logs: Vec::new(),
                cost_usd: 0.0,
                tokens: 0,
                cancelled: false,
                answer: json!({ "ok": true }),
                cost_per_ask: 0.0,
                tokens_per_ask: 0,
                run_outputs: VecDeque::new(),
            }),
        })
    }

    fn with_usage(cost_per_ask: f64, tokens_per_ask: u64) -> Arc<Self> {
        let host = Self::new();
        {
            let mut state = host.state.lock().expect("host state");
            state.cost_per_ask = cost_per_ask;
            state.tokens_per_ask = tokens_per_ask;
        }
        host
    }

    fn with_run_outputs(outputs: Vec<CommandOutput>) -> Arc<Self> {
        let host = Self::new();
        host.state
            .lock()
            .expect("host state")
            .run_outputs
            .extend(outputs);
        host
    }

    fn asks(&self) -> Vec<AskCall> {
        self.state.lock().expect("host state").asks.clone()
    }

    fn runs(&self) -> Vec<RunCall> {
        self.state.lock().expect("host state").runs.clone()
    }

    fn glob_calls(&self) -> usize {
        self.state.lock().expect("host state").glob_calls
    }

    fn reports(&self) -> Vec<Value> {
        self.state.lock().expect("host state").reports.clone()
    }

    fn artifacts(&self) -> Vec<Artifact> {
        self.state.lock().expect("host state").artifacts.clone()
    }

    fn cancel(&self) {
        self.state.lock().expect("host state").cancelled = true;
    }
}

impl Host for MockHost {
    fn ask(
        &self,
        agent: &str,
        persona: Option<&str>,
        instructions: &str,
    ) -> Result<Value, HostError> {
        let mut state = self.state.lock().expect("host state");
        state.asks.push(AskCall {
            agent: agent.to_owned(),
            persona: persona.map(str::to_owned),
            instructions: instructions.to_owned(),
        });
        state.cost_usd += state.cost_per_ask;
        state.tokens = state.tokens.saturating_add(state.tokens_per_ask);
        Ok(state.answer.clone())
    }

    fn world_run(
        &self,
        cmd: &str,
        args: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<CommandOutput, HostError> {
        let mut state = self.state.lock().expect("host state");
        state.runs.push(RunCall {
            command: cmd.to_owned(),
            args: args.to_vec(),
            timeout_ms,
        });
        Ok(state.run_outputs.pop_front().unwrap_or(CommandOutput {
            exit_code: 0,
            stdout: "## main...origin/main\n".to_owned(),
            stderr: String::new(),
        }))
    }

    fn files_glob(&self, _pattern: &str) -> Result<Vec<String>, HostError> {
        self.state.lock().expect("host state").glob_calls += 1;
        Ok(vec!["src/lib.rs".to_owned(), "src/main.rs".to_owned()])
    }

    fn files_grep(&self, _pattern: &str, _glob: Option<&str>) -> Result<Vec<GrepMatch>, HostError> {
        Ok(vec![GrepMatch {
            path: "src/lib.rs".to_owned(),
            line: 12,
            text: "let value = thing.unwrap();".to_owned(),
        }])
    }

    fn files_read(&self, path: &str) -> Result<String, HostError> {
        Ok(format!("// {path}\n"))
    }

    fn git_changed_files(&self, _base: Option<&str>) -> Result<Vec<String>, HostError> {
        Ok(vec!["src/lib.rs".to_owned()])
    }

    fn git_diff(&self, _base: Option<&str>, _path: Option<&str>) -> Result<String, HostError> {
        Ok("diff --git a/src/lib.rs b/src/lib.rs\n".to_owned())
    }

    fn git_status(&self) -> Result<String, HostError> {
        Ok("## main...origin/main\n".to_owned())
    }

    fn git_log(&self, _count: Option<usize>) -> Result<Vec<GitCommit>, HostError> {
        Ok(vec![GitCommit {
            sha: "abc1234".to_owned(),
            summary: "init".to_owned(),
        }])
    }

    fn report(&self, item: &Value, _artifact_id: Option<&str>) -> Result<(), HostError> {
        self.state
            .lock()
            .expect("host state")
            .reports
            .push(item.clone());
        Ok(())
    }

    fn artifact_markdown(
        &self,
        id: &str,
        title: Option<&str>,
        content: &str,
    ) -> Result<(), HostError> {
        self.state
            .lock()
            .expect("host state")
            .artifacts
            .push(Artifact {
                id: id.to_owned(),
                title: title.map(str::to_owned),
                content: ArtifactContent::Markdown(content.to_owned()),
            });
        Ok(())
    }

    fn artifact_file(&self, id: &str, title: Option<&str>, path: &str) -> Result<(), HostError> {
        self.state
            .lock()
            .expect("host state")
            .artifacts
            .push(Artifact {
                id: id.to_owned(),
                title: title.map(str::to_owned),
                content: ArtifactContent::File(path.to_owned()),
            });
        Ok(())
    }

    fn log(&self, message: &str) {
        self.state
            .lock()
            .expect("host state")
            .logs
            .push(message.to_owned());
    }

    fn cost_usd(&self) -> f64 {
        self.state.lock().expect("host state").cost_usd
    }

    fn tokens(&self) -> u64 {
        self.state.lock().expect("host state").tokens
    }

    fn is_cancelled(&self) -> bool {
        self.state.lock().expect("host state").cancelled
    }
}

fn run_spec(
    spec: &WorkflowSpec,
    host: Arc<MockHost>,
) -> (Result<WorkflowOutcome, WorkflowError>, Vec<WorkflowEvent>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let events = Arc::clone(&events);
        Arc::new(move |event| events.lock().expect("events").push(event))
    };
    let dyn_host: Arc<dyn Host> = host;
    let result = run_workflow(spec, dyn_host, sink);
    let events = events.lock().expect("events").clone();
    (result, events)
}

fn phase_names(events: &[WorkflowEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            WorkflowEvent::Phase { name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn finished_status(events: &[WorkflowEvent]) -> Option<RunStatus> {
    match events.last() {
        Some(WorkflowEvent::Finished { report }) => Some(report.status),
        _ => None,
    }
}

fn ask_agents(report: &selora_workflow::WorkflowReport) -> Vec<String> {
    report
        .steps
        .iter()
        .filter(|step| step.kind == StepKind::Ask)
        .filter_map(|step| step.agent.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Step 1 - the confirmation payload is the whole grant, before any effect.
// ---------------------------------------------------------------------------

#[test]
fn phase8_step1_confirmation_payload_matches_the_grant() {
    let script = r#"
        // world.run("comment-only") is not a grant.
        const note = "world.run('string-only')";
        phase("scan");
        const files = files.glob("src/**/*.rs");
        world.run("git", ["status"]);
        world.run('cargo', ["test"]);
        world.run("git", ["diff"]);
        report({ files: files.length, note: note.length });
    "#;

    let confirmation = prepare("Acceptance audit", script, 2.5).expect("script prepares");
    assert_eq!(confirmation.name, "Acceptance audit");
    assert_eq!(confirmation.script, script, "the card shows the exact text");
    assert_eq!(confirmation.script_bytes(), script.len());
    assert_eq!(confirmation.command_set.as_slice(), ["git", "cargo"]);
    assert_eq!(confirmation.max_cost_usd, 2.5);
    assert_eq!(confirmation.limits, Limits::standard());
    assert_eq!(confirmation.limits.max_script_bytes, 64 * 1024);
    assert_eq!(confirmation.limits.max_steps, 256);
    assert_eq!(confirmation.limits.max_report_items, 256);

    // `spec()` carries the same four facts into the run; it cannot widen them.
    let spec = confirmation.spec();
    assert_eq!(spec.name, confirmation.name);
    assert_eq!(spec.source, confirmation.script);
    assert_eq!(spec.command_set, confirmation.command_set);
    assert_eq!(spec.max_cost_usd, confirmation.max_cost_usd);

    // A script edited after approval, or a cleared grant, is rejected before
    // `Run`, so no host call and no event can happen under an unconfirmed set.
    let mut unconfirmed = WorkflowSpec::new("Acceptance audit", script);
    unconfirmed.command_set = CommandSet::new(Vec::<String>::new());
    let host = MockHost::new();
    let (result, events) = run_spec(&unconfirmed, Arc::clone(&host));
    assert_eq!(
        result,
        Err(WorkflowError::UnconfirmedCommand {
            command: "git".to_owned()
        })
    );
    assert!(events.is_empty(), "pre-flight rejection starts no run");
    assert!(host.asks().is_empty());
    assert!(host.runs().is_empty());
    assert!(host.reports().is_empty());
}

// ---------------------------------------------------------------------------
// Step 2 - approval starts the phase graph and every named-agent dispatch.
// ---------------------------------------------------------------------------

#[test]
fn phase8_step2_approval_starts_the_phase_graph_and_agent_dispatch() {
    let script = r#"
        phase("plan");
        const first = agent("reviewer", "terse").ask("review src/lib.rs");
        const second = agent("reviewer").ask("review src/main.rs");
        const third = agent("tester").ask("confirm the findings");
        phase("collect");
        report({ answers: 3, first: first.ok, second: second.ok });
        phase("done");
    "#;
    let confirmation = prepare("Fan-out", script, 5.0).expect("prepares");
    let host = MockHost::new();

    let (result, events) = run_spec(&confirmation.spec(), Arc::clone(&host));
    let outcome = result.expect("approved run completes");

    assert_eq!(outcome.report.status, RunStatus::Completed);
    assert_eq!(outcome.report.phases, ["plan", "collect", "done"]);
    assert_eq!(phase_names(&events), ["plan", "collect", "done"]);
    assert_eq!(
        ask_agents(&outcome.report),
        ["reviewer", "reviewer", "tester"]
    );
    assert_eq!(
        outcome
            .report
            .steps
            .iter()
            .filter(|step| step.kind == StepKind::Ask)
            .map(|step| step.status)
            .collect::<Vec<_>>(),
        [StepStatus::Ok, StepStatus::Ok, StepStatus::Ok]
    );

    let asks = host.asks();
    assert_eq!(asks.len(), 3, "every script ask reached the host once");
    assert_eq!(asks[0].persona.as_deref(), Some("terse"));
    assert!(asks[0].instructions.contains("src/lib.rs"));
    assert_eq!(host.reports().len(), 1);
    assert_eq!(host.reports()[0]["answers"], json!(3));
    assert_eq!(finished_status(&events), Some(RunStatus::Completed));
}

// ---------------------------------------------------------------------------
// Step 5 - a failing gate drives the script's visible fix-loop rounds.
// ---------------------------------------------------------------------------

#[test]
fn phase8_step5_failing_gate_drives_a_visible_fix_loop_round() {
    let script = r#"
        phase("gate");
        let round = 0;
        while (world.run("cargo", ["test"]).exitCode !== 0) {
            round = round + 1;
            phase("fix-" + round);
            if (round >= 3) {
                throw new Error("gate did not recover");
            }
        }
        report({ rounds: round });
        phase("done");
    "#;
    let confirmation = prepare("Gate loop", script, 5.0).expect("prepares");
    assert_eq!(confirmation.command_set.as_slice(), ["cargo"]);
    let host = MockHost::with_run_outputs(vec![
        CommandOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "first failure".to_owned(),
        },
        CommandOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "second failure".to_owned(),
        },
        CommandOutput {
            exit_code: 0,
            stdout: "green".to_owned(),
            stderr: String::new(),
        },
    ]);

    let (result, events) = run_spec(&confirmation.spec(), Arc::clone(&host));
    let outcome = result.expect("the loop recovers on the third gate");

    assert_eq!(outcome.report.status, RunStatus::Completed);
    assert_eq!(
        outcome.report.phases,
        ["gate", "fix-1", "fix-2", "done"],
        "each fix round is a visible phase node"
    );
    assert_eq!(phase_names(&events), ["gate", "fix-1", "fix-2", "done"]);
    let gate_runs = host.runs();
    assert_eq!(gate_runs.len(), 3, "one host command per loop round");
    assert!(gate_runs.iter().all(|run| run.command == "cargo"));
    assert_eq!(host.reports()[0]["rounds"], json!(2));
    assert!(outcome
        .report
        .steps
        .iter()
        .all(|step| step.status == StepStatus::Ok));
}

// ---------------------------------------------------------------------------
// Stop - the cancellation switch ends the run before the next external effect.
// ---------------------------------------------------------------------------

#[test]
fn phase8_step6_stop_before_the_next_effect_ends_the_run_stopped() {
    let host = MockHost::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let events = Arc::clone(&events);
        let cancel = Arc::clone(&host);
        Arc::new(move |event| {
            if matches!(event, WorkflowEvent::Phase { .. }) {
                // The panel's Stop arrives while the phase marker is painted.
                cancel.cancel();
            }
            events.lock().expect("events").push(event);
        })
    };
    let spec = prepare(
        "Stop run",
        r#"phase("one"); world.run("git", ["status"]); report("never"); phase("two");"#,
        5.0,
    )
    .expect("prepares")
    .spec();

    let result = run_workflow(&spec, host.clone(), sink);
    let events = events.lock().expect("events").clone();

    assert_eq!(result, Err(WorkflowError::Stopped));
    assert!(
        host.runs().is_empty(),
        "Stop was observed before world.run reached the host"
    );
    assert!(host.reports().is_empty());
    assert_eq!(finished_status(&events), Some(RunStatus::Stopped));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, WorkflowEvent::Finished { .. }))
            .count(),
        1,
        "exactly one terminal event"
    );
    assert!(
        !phase_names(&events).iter().any(|name| name == "two"),
        "the script did not continue past Stop"
    );
}

// ---------------------------------------------------------------------------
// Step 7 - the run summary bills exactly what the host charged.
// ---------------------------------------------------------------------------

#[test]
fn phase8_step7_cost_and_tokens_match_every_ask() {
    let host = MockHost::with_usage(0.25, 100);
    let spec = prepare(
        "Costed run",
        r#"
            phase("ask");
            const a = agent("reviewer").ask("one");
            const b = agent("reviewer").ask("two");
            const c = agent("writer").ask("three");
            report({ answers: 3 });
        "#,
        5.0,
    )
    .expect("prepares")
    .spec();

    let (result, events) = run_spec(&spec, Arc::clone(&host));
    let outcome = result.expect("run completes");

    assert_eq!(host.asks().len(), 3);
    assert!((host.cost_usd() - 0.75).abs() < 1e-12);
    assert_eq!(host.tokens(), 300);
    assert_eq!(outcome.report.cost_usd, host.cost_usd());
    assert_eq!(outcome.report.tokens, host.tokens());
    assert!((outcome.report.cost_usd - 0.75).abs() < 1e-12);
    assert!(outcome.report.cost_usd <= 5.0);
    assert!(outcome.report.tokens > 0);
    assert!(matches!(
        events.last(),
        Some(WorkflowEvent::Finished { report })
            if report.cost_usd == outcome.report.cost_usd
                && report.tokens == outcome.report.tokens
    ));
}

// ---------------------------------------------------------------------------
// Budget - the confirmed cap aborts between steps; no later effect is billed.
// ---------------------------------------------------------------------------

#[test]
fn phase8_budget_cap_stops_the_run_between_steps() {
    let host = MockHost::with_usage(0.6, 50);
    let spec = prepare(
        "Budgeted run",
        r#"
            agent("a").ask("one");
            agent("a").ask("two");
            report("never");
        "#,
        1.0,
    )
    .expect("prepares")
    .spec();

    let (result, events) = run_spec(&spec, Arc::clone(&host));
    match result {
        Err(WorkflowError::BudgetExceeded { limit, spent }) => {
            assert!((limit - 1.0).abs() < 1e-12);
            assert!((spent - 1.2).abs() < 1e-9, "spent was {spent}");
        }
        other => panic!("expected BudgetExceeded, got {other:?}"),
    }
    assert_eq!(host.asks().len(), 2, "no ask after the cap fired");
    assert!(
        host.reports().is_empty(),
        "the report after the cap must never reach the host"
    );
    assert_eq!(finished_status(&events), Some(RunStatus::BudgetExceeded));
    assert!(matches!(
        events.last(),
        Some(WorkflowEvent::Finished { report })
            if report.error.as_deref().is_some_and(|error| error.contains("budget"))
    ));
}

// ---------------------------------------------------------------------------
// Step 8 - journal location and gitignore contract.
// ---------------------------------------------------------------------------

/// A throwaway workspace outside the repository, deleted on drop.
struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("Temp/opencode"))
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("selora-phase8-matrix-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create the temp workspace");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn phase8_step8_journal_lands_in_the_gitignored_workspace_path() {
    let workspace = TempWorkspace::new();
    let run_id = RunId::new();
    let spec = prepare(
        "Journal run",
        r##"phase("write"); report({ ok: true }); artifact.markdown("summary", "# Done");"##,
        5.0,
    )
    .expect("prepares")
    .spec();

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let events = Arc::clone(&events);
        Arc::new(move |event| events.lock().expect("events").push(event))
    };
    let outcome = run_workflow_persisted(
        &spec,
        MockHost::new(),
        sink,
        RunOptions {
            workspace_root: workspace.path().to_path_buf(),
            run_id: run_id.clone(),
            resume: false,
            allow_incomplete_asks: false,
        },
    )
    .expect("durable run completes");
    assert_eq!(outcome.report.status, RunStatus::Completed);

    let expected = workspace
        .path()
        .join(JOURNAL_DIR)
        .join(run_id.as_str())
        .join(JOURNAL_FILE);
    assert_eq!(
        selora_workflow::journal::journal_path(workspace.path(), &run_id),
        expected
    );
    assert!(expected.is_file(), "journal exists at {expected:?}");
    assert!(expected.starts_with(workspace.path()));
    let journal = fs::read_to_string(&expected).expect("journal is readable");
    assert!(journal.contains("engine_run_started"));
    assert!(journal.contains("engine_run_finished"));
    assert!(journal.contains("engine_artifact"));

    // The engine seeds the workspace-local ignore file, so a real git
    // repository never tracks a run journal.
    assert_eq!(
        fs::read_to_string(workspace.path().join(".selora").join(".gitignore"))
            .expect("the run seeded .selora/.gitignore"),
        "*\n"
    );

    // The repository itself ignores the directory (checked from this test so
    // the contract is not only a regex in the acceptance script).
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate is two levels below the repository root");
    let gitignore = fs::read_to_string(repo_root.join(".gitignore")).expect("repo .gitignore");
    assert!(
        gitignore
            .lines()
            .any(|line| line.trim() == "/.selora/" || line.trim() == ".selora"),
        "repository .gitignore must ignore .selora/"
    );
}

// ---------------------------------------------------------------------------
// Step 9 - the facade is the whole API: fetch/fs/process do not exist.
// ---------------------------------------------------------------------------

#[test]
fn phase8_step9_fetch_fs_process_fail_before_any_host_effect() {
    for name in ["fetch", "fs", "process"] {
        let source = format!(
            r#"
                phase("probe");
                const value = {name}("probe");
                report({{ leaked: true, value: value }});
            "#
        );
        let confirmation = prepare(format!("Deny {name}"), source, 5.0)
            .expect("the syntax is valid; only the global is absent");
        let host = MockHost::new();
        let (result, events) = run_spec(&confirmation.spec(), Arc::clone(&host));

        assert_eq!(
            result,
            Err(WorkflowError::AmbientGlobal {
                name: name.to_owned()
            }),
            "`{name}` must fail as a typed ambient-global error"
        );
        assert!(host.asks().is_empty());
        assert!(host.runs().is_empty());
        assert!(host.glob_calls() == 0);
        assert!(host.reports().is_empty());
        assert!(host.artifacts().is_empty());
        assert_eq!(finished_status(&events), Some(RunStatus::Failed));

        let finished = match events.last() {
            Some(WorkflowEvent::Finished { report }) => report,
            other => panic!("missing terminal event: {other:?}"),
        };
        assert!(
            finished
                .error
                .as_deref()
                .is_some_and(|error| error.contains(name) && error.contains("not available")),
            "the failure names `{name}`: {:?}",
            finished.error
        );
        assert!(
            !phase_names(&events).iter().any(|phase| phase == "leaked"),
            "no later step ran after the denied global"
        );
    }
}
