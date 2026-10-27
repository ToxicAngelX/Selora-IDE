//! Phase 8 acceptance fixtures (ROADMAP.md Phase 8, wave 2 wiring).
//!
//! These tests exercise the engine the way 8c is supposed to: a realistic
//! script over a fake [`Host`], asserting the exact event stream the panel's
//! adapter will consume, the caps the confirmation card shows, and the
//! `world.run` command-set gate. A second fixture documents the shell's
//! journal/resume protocol — finished asks are not paid for twice, world reads
//! replay byte-identically, and an unfinished step re-dispatches.
//!
//! They deliberately use only the public API (`prepare`, `run_workflow`,
//! `journal`), so the same calls the shell will make are the ones under test.
//! `resume` is *not* wired into the engine yet: the resume fixture journals a
//! real run by hand, then asserts the [`ResumePlan`] the shell would consult.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use selora_workflow::journal::{
    journal_path, Journal, JournalEvent, ReadRequest, RunStatus as JournalRunStatus,
    StepStatus as JournalStepStatus, JOURNAL_DIR, JOURNAL_FILE,
};
use selora_workflow::{
    prepare, run_workflow, Artifact, ArtifactContent, CommandOutput, EventSink, GitCommit,
    GrepMatch, Host, HostError, ReportItem, RunId, RunStatus, StepKind, StepStatus, WorkflowError,
    WorkflowEvent, WorkflowOutcome, WorkflowSpec, WorkflowStep,
};

// ---------------------------------------------------------------------------
// Fake host
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

#[derive(Debug, Clone, PartialEq)]
struct ReportCall {
    item: Value,
    artifact_id: Option<String>,
}

/// A fake host: canned answers, a configurable glob, every call recorded.
///
/// Cloneable so the test keeps a handle after `run_workflow` takes an
/// `Arc<dyn Host>`; the state is shared, which is also how the resume fixture
/// observes that a replay touched nothing.
#[derive(Clone)]
struct FixtureHost {
    state: Arc<Mutex<HostState>>,
}

struct HostState {
    answers: VecDeque<Value>,
    asks: Vec<AskCall>,
    runs: Vec<RunCall>,
    glob: Vec<String>,
    glob_calls: usize,
    reports: Vec<ReportCall>,
    artifacts: Vec<Artifact>,
    logs: Vec<String>,
    cost: f64,
}

impl FixtureHost {
    fn new(glob: Vec<String>, answers: Vec<Value>, cost: f64) -> Self {
        Self {
            state: Arc::new(Mutex::new(HostState {
                answers: answers.into(),
                asks: Vec::new(),
                runs: Vec::new(),
                glob,
                glob_calls: 0,
                reports: Vec::new(),
                artifacts: Vec::new(),
                logs: Vec::new(),
                cost,
            })),
        }
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

    fn reports(&self) -> Vec<ReportCall> {
        self.state.lock().expect("host state").reports.clone()
    }

    fn artifacts(&self) -> Vec<Artifact> {
        self.state.lock().expect("host state").artifacts.clone()
    }
}

impl Host for FixtureHost {
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
        Ok(state
            .answers
            .pop_front()
            .unwrap_or_else(|| json!({ "ok": true })))
    }

    fn world_run(
        &self,
        cmd: &str,
        args: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<CommandOutput, HostError> {
        self.state.lock().expect("host state").runs.push(RunCall {
            command: cmd.to_owned(),
            args: args.to_vec(),
            timeout_ms,
        });
        Ok(CommandOutput {
            exit_code: 0,
            stdout: "## main...origin/main\n".to_owned(),
            stderr: String::new(),
        })
    }

    fn files_glob(&self, _pattern: &str) -> Result<Vec<String>, HostError> {
        let mut state = self.state.lock().expect("host state");
        state.glob_calls += 1;
        Ok(state.glob.clone())
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

    fn report(&self, item: &Value, artifact_id: Option<&str>) -> Result<(), HostError> {
        self.state
            .lock()
            .expect("host state")
            .reports
            .push(ReportCall {
                item: item.clone(),
                artifact_id: artifact_id.map(str::to_owned),
            });
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
        self.state.lock().expect("host state").cost
    }
}

fn run_fixture(
    spec: &WorkflowSpec,
    host: &FixtureHost,
) -> (Result<WorkflowOutcome, WorkflowError>, Vec<WorkflowEvent>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let events = Arc::clone(&events);
        Arc::new(move |event| events.lock().expect("events").push(event))
    };
    let dyn_host: Arc<dyn Host> = Arc::new(host.clone());
    let result = run_workflow(spec, dyn_host, sink);
    let events = events.lock().expect("events").clone();
    (result, events)
}

fn kind_name(kind: StepKind) -> &'static str {
    match kind {
        StepKind::Phase => "phase",
        StepKind::Ask => "ask",
        StepKind::WorldRun => "world_run",
        StepKind::Files => "files",
        StepKind::Git => "git",
        StepKind::Report => "report",
        StepKind::Artifact => "artifact",
        StepKind::Log => "log",
    }
}

fn status_name(status: StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => "pending",
        StepStatus::Running => "running",
        StepStatus::Ok => "ok",
        StepStatus::Failed => "failed",
        StepStatus::Skipped => "skipped",
    }
}

/// A compact, order-preserving rendering of the event stream the panel adapter
/// will translate. `step` carries kind + status; phase/report/artifact carry
/// enough to locate the record.
fn event_tag(event: &WorkflowEvent) -> String {
    match event {
        WorkflowEvent::Run { .. } => "run".to_owned(),
        WorkflowEvent::Phase { name } => format!("phase:{name}"),
        WorkflowEvent::Step { step } => {
            format!("step:{}:{}", kind_name(step.kind), status_name(step.status))
        }
        WorkflowEvent::Report { item } => format!("report:{}", item.index),
        WorkflowEvent::Artifact { artifact } => format!("artifact:{}", artifact.id),
        WorkflowEvent::Log { message } => format!("log:{message}"),
        WorkflowEvent::Finished { .. } => "finished".to_owned(),
    }
}

fn report_items(events: &[WorkflowEvent]) -> Vec<ReportItem> {
    events
        .iter()
        .filter_map(|event| match event {
            WorkflowEvent::Report { item } => Some(item.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. The realistic run: scan → glob → reviewer ask → git gate → report →
//    artifact → done.
// ---------------------------------------------------------------------------

const REVIEW_SCRIPT: &str = r##"
    phase("scan");
    const rustFiles = files.glob("src/**/*.rs");
    if (rustFiles.length !== 2) {
        throw new Error("expected two Rust files, got " + rustFiles.length);
    }

    phase("review");
    const review = agent("reviewer", "terse").ask(
        {
            type: "object",
            required: ["verdict", "findings"],
            properties: {
                verdict: { type: "string", enum: ["ship", "block"] },
                findings: {
                    type: "array",
                    items: {
                        type: "object",
                        required: ["path", "line"],
                        properties: {
                            path: { type: "string" },
                            line: { type: "integer" }
                        }
                    }
                }
            }
        },
        "Review every Rust file for unwrap() calls; return the worst findings"
    );

    const status = world.run("git", ["status"]);
    if (status.exitCode !== 0) {
        throw new Error("git status failed: " + status.stderr);
    }

    report({ verdict: review.verdict, worst: review.findings.length }, "summary");
    artifact.markdown(
        "summary",
        "# Review\n\n" + review.findings.length + " unwrap() calls across " + rustFiles.length + " files.",
        { title: "Review summary" }
    );
    phase("done");
    review.verdict;
"##;

#[test]
fn realistic_review_workflow_emits_the_panel_sequence_and_enforces_the_grant() {
    let host = FixtureHost::new(
        vec!["src/lib.rs".to_owned(), "src/main.rs".to_owned()],
        vec![json!({
            "verdict": "block",
            "findings": [
                { "path": "src/lib.rs", "line": 12 },
                { "path": "src/main.rs", "line": 3 }
            ]
        })],
        0.0125,
    );

    let confirmation = prepare("Review Rust files", REVIEW_SCRIPT, 5.0).expect("script prepares");

    // The card's grant: exactly the literals the runtime may hand to the host.
    assert_eq!(confirmation.command_set.as_slice(), ["git"]);
    assert_eq!(confirmation.script_bytes(), REVIEW_SCRIPT.len());
    assert!(confirmation.script_bytes() <= confirmation.limits.max_script_bytes);

    let spec = confirmation.spec();
    let (result, events) = run_fixture(&spec, &host);
    let outcome = result.expect("run completes");
    let report = &outcome.report;

    assert_eq!(report.status, RunStatus::Completed);
    assert_eq!(report.name, "Review Rust files");
    assert_eq!(report.phases, ["scan", "review", "done"]);
    assert_eq!(outcome.value, json!("block"));
    assert_eq!(report.cost_usd, 0.0125);

    // One entry per facade call, in script order, all successful.
    let kinds: Vec<StepKind> = report.steps.iter().map(|step| step.kind).collect();
    assert_eq!(
        kinds,
        [
            StepKind::Phase,
            StepKind::Files,
            StepKind::Phase,
            StepKind::Ask,
            StepKind::WorldRun,
            StepKind::Report,
            StepKind::Artifact,
            StepKind::Phase,
        ]
    );
    assert!(
        report
            .steps
            .iter()
            .all(|step| step.status == StepStatus::Ok),
        "every step must be Ok: {:?}",
        report.steps
    );
    assert_eq!(
        report
            .steps
            .iter()
            .map(|step| step.index)
            .collect::<Vec<_>>(),
        (1..=8).collect::<Vec<_>>()
    );
    assert!(report.steps.len() <= confirmation.limits.max_steps);

    // The exact event stream the panel adapter consumes. A phase marker sits
    // between its step's Running and Ok snapshots; a report/artifact event
    // lands inside its step, before that step's Ok.
    let tags: Vec<String> = events.iter().map(event_tag).collect();
    assert_eq!(
        tags,
        [
            "run",
            "step:phase:running",
            "phase:scan",
            "step:phase:ok",
            "step:files:running",
            "step:files:ok",
            "step:phase:running",
            "phase:review",
            "step:phase:ok",
            "step:ask:running",
            "step:ask:ok",
            "step:world_run:running",
            "step:world_run:ok",
            "step:report:running",
            "report:1",
            "step:report:ok",
            "step:artifact:running",
            "artifact:summary",
            "step:artifact:ok",
            "step:phase:running",
            "phase:done",
            "step:phase:ok",
            "finished",
        ]
    );
    assert!(matches!(events.first(), Some(WorkflowEvent::Run { .. })));
    assert!(matches!(
        events.last(),
        Some(WorkflowEvent::Finished { report }) if report == &outcome.report
    ));

    // Host calls: one glob, one ask on the named agent, one confirmed gate.
    assert_eq!(host.glob_calls(), 1);
    let asks = host.asks();
    assert_eq!(asks.len(), 1);
    assert_eq!(asks[0].agent, "reviewer");
    assert_eq!(asks[0].persona.as_deref(), Some("terse"));
    assert!(asks[0].instructions.contains("unwrap()"));
    assert_eq!(
        host.runs(),
        [RunCall {
            command: "git".to_owned(),
            args: vec!["status".to_owned()],
            timeout_ms: None,
        }]
    );

    // Report caps: one item, within the per-item byte cap, tied to the artifact.
    let items = report_items(&events);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].index, 1);
    assert_eq!(items[0].artifact_id.as_deref(), Some("summary"));
    assert_eq!(items[0].item["verdict"], json!("block"));
    assert_eq!(items[0].item["worst"], json!(2));
    assert_eq!(host.reports().len(), 1);
    assert_eq!(host.reports()[0].item, items[0].item);
    assert!(items.len() <= confirmation.limits.max_report_items);
    let item_bytes = serde_json::to_vec(&items[0].item)
        .expect("item serializes")
        .len();
    assert!(item_bytes <= confirmation.limits.max_report_item_bytes);

    // Artifact caps: one markdown artifact, body within the engine's cap.
    assert_eq!(report.artifacts.len(), 1);
    let artifact = &report.artifacts[0];
    assert_eq!(artifact.id, "summary");
    assert_eq!(artifact.title.as_deref(), Some("Review summary"));
    let markdown = match &artifact.content {
        ArtifactContent::Markdown(markdown) => markdown,
        other => panic!("expected markdown, got {other:?}"),
    };
    assert!(markdown.starts_with("# Review"));
    assert!(markdown.contains("2 unwrap() calls across 2 files."));
    assert!(markdown.len() <= confirmation.limits.max_artifact_bytes);
    assert_eq!(host.artifacts().len(), 1);

    // The grant is enforced, not advisory: the same script with an empty
    // command set is rejected before `Run`, so nothing reaches the host.
    let ungranted = WorkflowSpec::new("Review Rust files", REVIEW_SCRIPT);
    assert!(ungranted.command_set.is_empty());
    let (tampered, tampered_events) = run_fixture(&ungranted, &host);
    assert_eq!(
        tampered,
        Err(WorkflowError::UnconfirmedCommand {
            command: "git".to_owned(),
        })
    );
    assert!(
        tampered_events.is_empty(),
        "a pre-flight rejection emits no events: {tampered_events:?}"
    );
    assert_eq!(
        host.runs().len(),
        1,
        "the gate must not reach the host twice"
    );
}

// ---------------------------------------------------------------------------
// 2. Journal/resume fixture: the shell's replay protocol over a real run.
// ---------------------------------------------------------------------------

/// A throwaway workspace, deleted on drop.
struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "selora-phase8-acceptance-{}-{}",
            std::process::id(),
            selora_workflow::RunId::new()
        ));
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

const RESUME_SCRIPT: &str = r#"
    phase("scan");
    const rustFiles = files.glob("src/**/*.rs");
    const review = agent("reviewer").ask(
        {
            type: "object",
            required: ["findings"],
            properties: {
                findings: {
                    type: "array",
                    items: {
                        type: "object",
                        required: ["path", "line"],
                        properties: {
                            path: { type: "string" },
                            line: { type: "integer" }
                        }
                    }
                }
            }
        },
        "Review every Rust file for unwrap() calls"
    );
    report({ reviewed: rustFiles.length, findings: review.findings.length });
    review;
"#;

#[test]
fn resume_fixture_does_not_repeat_finished_asks_or_reread_the_world() {
    let workspace = TempWorkspace::new();
    let run_id = RunId::from("phase8-resume-fixture");
    let original_files = vec!["src/lib.rs".to_owned(), "src/main.rs".to_owned()];
    let answer = json!({
        "findings": [
            { "path": "src/lib.rs", "line": 12 },
            { "path": "src/main.rs", "line": 3 }
        ]
    });

    // --- First attempt: a real engine run. --------------------------------
    let host = FixtureHost::new(original_files.clone(), vec![answer.clone()], 0.0081);
    let confirmation = prepare("Resume fixture", RESUME_SCRIPT, 5.0).expect("prepares");
    let (result, _) = run_fixture(&confirmation.spec(), &host);
    let first = result.expect("first attempt completes");

    let files_step = first
        .report
        .steps
        .iter()
        .find(|step| step.kind == StepKind::Files)
        .expect("the run read the world");
    let ask_step = first
        .report
        .steps
        .iter()
        .find(|step| step.kind == StepKind::Ask)
        .expect("the run asked the reviewer");
    let report_step = first
        .report
        .steps
        .iter()
        .find(|step| step.kind == StepKind::Report)
        .expect("the run filed a report");

    // The shell's recorder: journal the completed read and ask, then crash
    // after the report step started and before it finished.
    write_first_attempt(
        workspace.path(),
        &run_id,
        files_step,
        ask_step,
        report_step,
        &serde_json::to_string(&original_files).expect("glob list serializes"),
        &answer,
    );

    // --- Reload: the resume plan is the only source of truth. -------------
    let loaded = Journal::load(workspace.path(), run_id.clone()).expect("load the journal");
    assert!(loaded.corrupt.is_empty(), "{:?}", loaded.corrupt);
    assert_eq!(loaded.entries.len(), 10);

    let plan = loaded.resume_plan();
    assert!(plan.has_started());
    assert!(!plan.is_run_finished());
    assert_eq!(plan.phases(), ["scan".to_owned()]);
    assert_eq!(
        plan.steps_started(),
        [
            "step-2".to_owned(),
            "step-3".to_owned(),
            "step-4".to_owned()
        ]
    );
    assert_eq!(plan.unfinished_steps(), ["step-4".to_owned()]);
    assert!(plan.is_step_finished("step-2"));
    assert!(plan.is_step_finished("step-3"));
    assert_eq!(plan.tokens(), 410);
    assert!((plan.spent_usd() - 0.0081).abs() < 1e-12);

    // A finished ask replays its journaled result: the resume host is not
    // asked again (this is what "finished asks are not re-paid" means).
    let resume_host = FixtureHost::new(
        vec!["src/CHANGED.rs".to_owned()],
        vec![json!({ "findings": [] })],
        0.0081,
    );
    let journaled_ask = plan
        .finished_ask("step-3")
        .expect("the finished ask replays");
    assert_eq!(journaled_ask.agent, "reviewer");
    assert_eq!(journaled_ask.result, answer);
    assert_eq!(resume_host.asks().len(), 0);
    assert_eq!(resume_host.glob_calls(), 0);

    // A world read replays byte-identically: the filesystem may have changed
    // underneath the run (the resume host's glob says so), the script must not.
    let read = ReadRequest::files("glob", ["src/**/*.rs"]);
    let replayed = plan.world_read(&read).expect("the read replays");
    assert_eq!(
        replayed,
        serde_json::to_string(&original_files).expect("glob list serializes")
    );
    assert_ne!(
        serde_json::from_str::<Vec<String>>(replayed).expect("replay is JSON"),
        vec!["src/CHANGED.rs".to_owned()]
    );
    assert_eq!(resume_host.glob_calls(), 0);

    // --- Re-dispatch only the unfinished step, then finish the run. --------
    let files: Vec<String> = serde_json::from_str(replayed).expect("replayed files");
    let findings = journaled_ask.result["findings"]
        .as_array()
        .expect("findings array");
    resume_host
        .report(
            &json!({ "reviewed": files.len(), "findings": findings.len() }),
            None,
        )
        .expect("re-dispatched report");
    assert_eq!(resume_host.reports().len(), 1);
    assert_eq!(resume_host.reports()[0].item["findings"], json!(2));

    // The resumed writer appends after the existing records, never over them.
    let mut resumed = Journal::open(workspace.path(), run_id.clone()).expect("reopen");
    assert_eq!(resumed.next_seq(), loaded.next_seq());
    resumed
        .append(JournalEvent::StepFinished {
            step_id: "step-4".to_owned(),
            status: JournalStepStatus::Ok,
            detail: None,
        })
        .expect("append step finish");
    resumed
        .append(JournalEvent::RunFinished {
            status: JournalRunStatus::Completed,
            detail: None,
            spent_usd: plan.spent_usd(),
        })
        .expect("append run finish");
    drop(resumed);

    let reloaded = Journal::load(workspace.path(), run_id.clone()).expect("reload after resume");
    let resumed_plan = reloaded.resume_plan();
    assert!(resumed_plan.is_run_finished());
    assert!(resumed_plan.unfinished_steps().is_empty());
    assert_eq!(resumed_plan.run_status(), Some(JournalRunStatus::Completed));
    // Replayed results and spend survive the second load unchanged.
    assert_eq!(
        resumed_plan
            .finished_ask("step-3")
            .expect("still journaled")
            .result,
        answer
    );
    assert_eq!(resumed_plan.tokens(), 410);
    assert!((resumed_plan.spent_usd() - 0.0081).abs() < 1e-12);

    // --- The journal lands where acceptance step 8 says it does. ------------
    assert_eq!(
        journal_path(workspace.path(), &run_id),
        workspace
            .path()
            .join(JOURNAL_DIR)
            .join(run_id.as_str())
            .join(JOURNAL_FILE)
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join(".selora").join(".gitignore"))
            .expect("the run seeded .selora/.gitignore"),
        "*\n"
    );
}

/// Journal the first attempt exactly as the shell's recorder will: one typed
/// event per engine step, with the world read's bytes and the ask's validated
/// result. Step ids are the engine's monotonic step indices.
fn write_first_attempt(
    workspace: &Path,
    run_id: &RunId,
    files_step: &WorkflowStep,
    ask_step: &WorkflowStep,
    report_step: &WorkflowStep,
    glob_json: &str,
    answer: &Value,
) {
    let mut journal = Journal::open(workspace, run_id.clone()).expect("open the journal");
    let step_id = |step: &WorkflowStep| format!("step-{}", step.index);

    journal
        .append(JournalEvent::RunStarted {
            script_hash: Some("sha256:phase8-fixture".to_owned()),
            max_cost_usd: 5.0,
        })
        .expect("run started");
    journal
        .append(JournalEvent::PhaseStarted {
            phase: "scan".to_owned(),
        })
        .expect("phase started");

    journal
        .append(JournalEvent::StepStarted {
            step_id: step_id(files_step),
            phase: Some("scan".to_owned()),
            label: Some(files_step.detail.clone()),
        })
        .expect("read step started");
    journal
        .append(JournalEvent::WorldRead {
            step_id: step_id(files_step),
            read: ReadRequest::files("glob", ["src/**/*.rs"]),
            content: glob_json.to_owned(),
        })
        .expect("world read");
    journal
        .append(JournalEvent::StepFinished {
            step_id: step_id(files_step),
            status: JournalStepStatus::Ok,
            detail: None,
        })
        .expect("read step finished");

    journal
        .append(JournalEvent::StepStarted {
            step_id: step_id(ask_step),
            phase: Some("scan".to_owned()),
            label: Some(ask_step.detail.clone()),
        })
        .expect("ask step started");
    journal
        .append(JournalEvent::AskStarted {
            step_id: step_id(ask_step),
            agent: ask_step.agent.clone().expect("ask names its agent"),
        })
        .expect("ask started");
    journal
        .append(JournalEvent::AskFinished {
            step_id: step_id(ask_step),
            agent: ask_step.agent.clone().expect("ask names its agent"),
            result: answer.clone(),
            tokens: 410,
            cost_usd: 0.0081,
        })
        .expect("ask finished");
    journal
        .append(JournalEvent::StepFinished {
            step_id: step_id(ask_step),
            status: JournalStepStatus::Ok,
            detail: None,
        })
        .expect("ask step finished");

    // The crash: the next step started, and no finish ever reached the file.
    journal
        .append(JournalEvent::StepStarted {
            step_id: step_id(report_step),
            phase: Some("scan".to_owned()),
            label: Some("report".to_owned()),
        })
        .expect("report step started");
}
