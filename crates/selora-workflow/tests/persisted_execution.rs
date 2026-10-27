//! Real sandbox execution, interruption, and durable replay through the public API.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use selora_workflow::journal::{journal_path, run_dir, Journal};
use selora_workflow::{
    load_workflow_spec, prepare, run_workflow, run_workflow_persisted, CommandOutput, EventSink,
    GitCommit, GrepMatch, Host, HostError, RunId, RunOptions, RunStatus, WorkflowError,
    WorkflowEvent, WorkflowOutcome, WorkflowSpec,
};
use serde_json::{json, Value};

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("Temp/opencode"))
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("selora-durable-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn path(&self) -> &Path {
        &self.0
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
    reads: AtomicU64,
    commands: AtomicU64,
    displays: AtomicU64,
    cancelled: AtomicBool,
    fail_ask: u64,
    panic_command: bool,
    panic_ask: bool,
    checkpoint_agents: bool,
    fail_snapshot: bool,
    fail_restore: bool,
    snapshots: AtomicU64,
    restores: AtomicU64,
    conversations: Mutex<BTreeMap<String, Vec<Value>>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AgentCheckpoint {
    agent: String,
    messages: Vec<Value>,
}

impl Host for TestHost {
    fn ask(&self, agent: &str, _: Option<&str>, instructions: &str) -> Result<Value, HostError> {
        let count = self.asks.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(!self.panic_ask, "simulated process interruption during ask");
        if count == self.fail_ask {
            return Err(HostError::new("interrupted turn"));
        }
        if self.checkpoint_agents {
            let mut conversations = self.conversations.lock().unwrap();
            let history = conversations.entry(agent.to_owned()).or_default();
            if instructions == "followup" && history.len() != 6 {
                return Err(HostError::new(
                    "followup lost prior conversation and tool context",
                ));
            }
            history.extend([
                json!({"role":"user", "content":instructions}),
                json!({"role":"assistant", "tool_calls":[{"id":instructions, "name":"read"}]}),
                json!({"role":"tool", "tool_call_id":instructions, "content":"tool result"}),
            ]);
        }
        Ok(json!({"ok": true, "instructions": instructions}))
    }
    fn snapshot_agent(&self, agent: &str) -> Result<Option<Value>, HostError> {
        if !self.checkpoint_agents {
            return Ok(None);
        }
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        if self.fail_snapshot {
            return Err(HostError::new("cannot checkpoint conversation"));
        }
        let snapshot = AgentCheckpoint {
            agent: agent.to_owned(),
            messages: self
                .conversations
                .lock()
                .unwrap()
                .get(agent)
                .cloned()
                .unwrap_or_default(),
        };
        Ok(Some(serde_json::to_value(snapshot).unwrap()))
    }
    fn restore_agent(&self, agent: &str, snapshot: &Value) -> Result<(), HostError> {
        self.restores.fetch_add(1, Ordering::SeqCst);
        if self.fail_restore {
            return Err(HostError::new("cannot restore conversation"));
        }
        let checkpoint: AgentCheckpoint = serde_json::from_value(snapshot.clone())
            .map_err(|error| HostError::new(format!("malformed checkpoint: {error}")))?;
        if checkpoint.agent != agent {
            return Err(HostError::new("checkpoint agent identity mismatch"));
        }
        self.conversations
            .lock()
            .unwrap()
            .insert(agent.to_owned(), checkpoint.messages);
        Ok(())
    }
    fn world_run(&self, _: &str, _: &[String], _: Option<u64>) -> Result<CommandOutput, HostError> {
        self.commands.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.panic_command,
            "simulated process interruption during command"
        );
        Ok(CommandOutput {
            exit_code: 7,
            stdout: "gate output\n".into(),
            stderr: "gate error".into(),
        })
    }
    fn files_read(&self, _: &str) -> Result<String, HostError> {
        Ok(format!(
            "read {}\r\n\"quoted\" 世界",
            self.reads.fetch_add(1, Ordering::SeqCst) + 1
        ))
    }
    fn files_glob(&self, _: &str) -> Result<Vec<String>, HostError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec!["src/a.rs".into()])
    }
    fn files_grep(&self, _: &str, _: Option<&str>) -> Result<Vec<GrepMatch>, HostError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    fn git_changed_files(&self, _: Option<&str>) -> Result<Vec<String>, HostError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec!["a".into()])
    }
    fn git_diff(&self, _: Option<&str>, _: Option<&str>) -> Result<String, HostError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok("diff".into())
    }
    fn git_status(&self) -> Result<String, HostError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok("status".into())
    }
    fn git_log(&self, _: Option<usize>) -> Result<Vec<GitCommit>, HostError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    fn report(&self, _: &Value, _: Option<&str>) -> Result<(), HostError> {
        self.displays.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn artifact_markdown(&self, _: &str, _: Option<&str>, _: &str) -> Result<(), HostError> {
        self.displays.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn artifact_file(&self, _: &str, _: Option<&str>, _: &str) -> Result<(), HostError> {
        self.displays.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn log(&self, _: &str) {
        self.displays.fetch_add(1, Ordering::SeqCst);
    }
    fn cost_usd(&self) -> f64 {
        self.asks.load(Ordering::SeqCst) as f64
    }
    fn tokens(&self) -> u64 {
        self.asks.load(Ordering::SeqCst) * 100
    }
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

fn execute(
    workspace: &Workspace,
    id: &RunId,
    spec: &WorkflowSpec,
    host: Arc<TestHost>,
    resume: bool,
) -> (Result<WorkflowOutcome, WorkflowError>, Vec<WorkflowEvent>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = events.clone();
    let sink: EventSink = Arc::new(move |event| recorded.lock().unwrap().push(event));
    let result = run_workflow_persisted(
        spec,
        host,
        sink,
        RunOptions {
            workspace_root: workspace.0.clone(),
            run_id: id.clone(),
            resume,
            // These tests exercise replay semantics; the fail-closed default
            // for unfinished asks is covered by `tests/resume_preflight.rs`.
            allow_incomplete_asks: resume,
            acknowledge_partial_effects: false,
        },
    );
    let events = events.lock().unwrap().clone();
    (result, events)
}

fn terminal(events: &[WorkflowEvent]) -> &selora_workflow::WorkflowReport {
    match events.last().unwrap() {
        WorkflowEvent::Finished { report } => report,
        _ => panic!("missing terminal event"),
    }
}

#[test]
fn successful_same_name_asks_restore_full_checkpoints_before_unfinished_followup() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare(
        "conversation resume",
        r#"
        const worker = agent("worker");
        worker.ask("first"); worker.ask("second"); worker.ask("followup");
    "#,
        5.0,
    )
    .unwrap()
    .spec();
    let first = Arc::new(TestHost {
        checkpoint_agents: true,
        fail_ask: 3,
        ..TestHost::default()
    });
    assert!(execute(&workspace, &id, &spec, first.clone(), false)
        .0
        .is_err());
    assert_eq!(first.snapshots.load(Ordering::SeqCst), 2);
    let loaded = Journal::load(workspace.path(), id.clone()).unwrap();
    let finishes: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.kind == "engine_step_finished")
        .collect();
    assert_eq!(
        finishes[0].payload["agent_state"]["messages"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        finishes[1].payload["agent_state"]["messages"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    assert!(finishes[2].payload.get("agent_state").is_none());

    let resumed = Arc::new(TestHost {
        checkpoint_agents: true,
        ..TestHost::default()
    });
    let (result, _) = execute(&workspace, &id, &spec, resumed.clone(), true);
    let result = result.unwrap();
    assert_eq!(result.report.cost_usd, 4.0);
    assert_eq!(result.report.tokens, 400);
    assert_eq!(resumed.asks.load(Ordering::SeqCst), 1);
    assert_eq!(
        resumed.restores.load(Ordering::SeqCst),
        2,
        "restore once per actual replay, never during identity probes"
    );
    assert_eq!(resumed.snapshots.load(Ordering::SeqCst), 1);
    let conversations = resumed.conversations.lock().unwrap();
    let history = &conversations["worker"];
    assert_eq!(history.len(), 9);
    assert_eq!(history[0]["content"], "first");
    assert_eq!(history[3]["content"], "second");
    assert_eq!(history[6]["content"], "followup");
    assert_eq!(history[4]["tool_calls"][0]["id"], "second");
    assert_eq!(history[5]["tool_call_id"], "second");
}

#[test]
fn malformed_checkpoints_or_restore_failure_stop_before_new_external_calls() {
    for corruption in ["restore_failure", "wrong_shape", "null", "failed_ask_state"] {
        let workspace = Workspace::new();
        let id = RunId::new();
        let spec = prepare(
            "restore failure",
            r#"
            const worker = agent("worker"); worker.ask("first");
            worker.ask("second"); worker.ask("followup");
            files.read("new"); world.run("git", ["status"]);
        "#,
            5.0,
        )
        .unwrap()
        .spec();
        let first = Arc::new(TestHost {
            checkpoint_agents: true,
            fail_ask: 3,
            ..TestHost::default()
        });
        assert!(execute(&workspace, &id, &spec, first, false).0.is_err());
        if corruption != "restore_failure" {
            let path = journal_path(workspace.path(), &id);
            let mut entries: Vec<Value> = fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let record = entries
                .iter_mut()
                .find(|entry| {
                    entry["kind"] == "engine_step_finished"
                        && entry["payload"]["step"]["status"]
                            == if corruption == "failed_ask_state" {
                                "failed"
                            } else {
                                "ok"
                            }
                })
                .unwrap();
            record["payload"]["agent_state"] = if corruption == "null" {
                Value::Null
            } else {
                json!("malformed")
            };
            fs::write(
                path,
                entries
                    .iter()
                    .map(|entry| format!("{entry}\n"))
                    .collect::<String>(),
            )
            .unwrap();
        }
        let resumed = Arc::new(TestHost {
            checkpoint_agents: true,
            fail_restore: corruption == "restore_failure",
            ..TestHost::default()
        });
        let (result, events) = execute(&workspace, &id, &spec, resumed.clone(), true);
        assert!(result.is_err(), "{corruption}");
        assert_eq!(resumed.asks.load(Ordering::SeqCst), 0);
        assert_eq!(resumed.reads.load(Ordering::SeqCst), 0);
        assert_eq!(resumed.commands.load(Ordering::SeqCst), 0);
        assert_eq!(resumed.snapshots.load(Ordering::SeqCst), 0);
        assert!(resumed.conversations.lock().unwrap().is_empty());
        if corruption == "failed_ask_state" {
            assert!(events.is_empty());
            assert_eq!(resumed.restores.load(Ordering::SeqCst), 0);
        } else {
            assert_eq!(terminal(&events).status, RunStatus::Failed);
            assert_eq!(resumed.restores.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn checkpoint_failure_records_paid_usage_without_marking_ask_replayable() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare(
        "snapshot failure",
        r#"agent("worker").ask("first"); files.read("new");"#,
        5.0,
    )
    .unwrap()
    .spec();
    let host = Arc::new(TestHost {
        checkpoint_agents: true,
        fail_snapshot: true,
        ..TestHost::default()
    });
    let (result, events) = execute(&workspace, &id, &spec, host.clone(), false);
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("cannot checkpoint conversation"));
    assert_eq!(terminal(&events).cost_usd, 1.0);
    assert_eq!(terminal(&events).tokens, 100);
    assert_eq!(
        terminal(&events).steps[0].status,
        selora_workflow::StepStatus::Failed
    );
    assert_eq!(host.reads.load(Ordering::SeqCst), 0);
    let resumed = Arc::new(TestHost {
        checkpoint_agents: true,
        ..TestHost::default()
    });
    let (result, _) = execute(&workspace, &id, &spec, resumed.clone(), true);
    assert_eq!(result.unwrap().report.cost_usd, 2.0);
    assert_eq!(resumed.asks.load(Ordering::SeqCst), 1);
    assert_eq!(resumed.restores.load(Ordering::SeqCst), 0);
}

#[test]
fn actual_resume_replays_paid_asks_distinct_reads_commands_and_display_events() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare("durable", r#"
        phase("plan");
        const first = files.read("same");
        const answer = agent("reviewer", "careful").ask({type:"object", properties:{ok:{type:"boolean"}}}, "review");
        const second = files.read("same");
        const gate = world.run("git", ["status"], {timeoutMs:100});
        if (gate.exitCode !== 7) throw Error("wrong gate");
        files.glob("*.rs"); files.grep("x", "*.rs");
        git.changedFiles("HEAD"); git.diff(null, "a"); git.status(); git.log(2);
        report({first, second, answer, gate});
        artifact.markdown("summary", first, {title:"Summary"});
        artifact.file("file", "a"); log("progress");
        agent("finisher").ask("finish");
        [first, second, gate.stdout];
    "#, 5.0).unwrap().spec();
    let first = Arc::new(TestHost {
        fail_ask: 2,
        ..TestHost::default()
    });
    let (failed, events) = execute(&workspace, &id, &spec, first.clone(), false);
    assert!(matches!(failed, Err(WorkflowError::Host(_))));
    assert_eq!(terminal(&events).cost_usd, 2.0);
    assert_eq!(terminal(&events).tokens, 200);
    assert_eq!(load_workflow_spec(workspace.path(), &id).unwrap(), spec);
    assert_eq!(first.reads.load(Ordering::SeqCst), 8);
    assert_eq!(first.commands.load(Ordering::SeqCst), 1);

    let resumed = Arc::new(TestHost::default());
    let (result, events) = execute(&workspace, &id, &spec, resumed.clone(), true);
    let result = result.unwrap();
    assert_eq!(result.report.run_id, id);
    assert_eq!(result.report.status, RunStatus::Completed);
    assert_eq!(result.report.cost_usd, 3.0);
    assert_eq!(result.report.tokens, 300);
    assert_eq!(result.report.phases, ["plan"]);
    assert_eq!(result.report.artifacts.len(), 2);
    assert_eq!(
        result.value,
        json!([
            "read 1\r\n\"quoted\" 世界",
            "read 2\r\n\"quoted\" 世界",
            "gate output\n"
        ])
    );
    assert_eq!(resumed.asks.load(Ordering::SeqCst), 1);
    assert_eq!(resumed.reads.load(Ordering::SeqCst), 0);
    assert_eq!(resumed.commands.load(Ordering::SeqCst), 0);
    assert_eq!(resumed.displays.load(Ordering::SeqCst), 0);
    assert!(events
        .iter()
        .any(|e| matches!(e, WorkflowEvent::Report { .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, WorkflowEvent::Log { .. })));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::Artifact { .. }))
            .count(),
        2
    );
    let (again, events) = execute(&workspace, &id, &spec, resumed.clone(), true);
    assert!(again.unwrap_err().to_string().contains("completed run"));
    assert!(events.is_empty());
    assert_eq!(resumed.asks.load(Ordering::SeqCst), 1);
}

#[test]
fn changed_script_grants_name_or_cap_cannot_resume() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare("identity", r#"agent("a").ask("first");"#, 5.0)
        .unwrap()
        .spec();
    assert!(execute(
        &workspace,
        &id,
        &spec,
        Arc::new(TestHost {
            fail_ask: 1,
            ..TestHost::default()
        }),
        false
    )
    .0
    .is_err());
    for change in 0..4 {
        let mut changed = spec.clone();
        match change {
            0 => changed.source.push_str("phase('changed');"),
            1 => changed.command_set = selora_workflow::CommandSet::new(["cargo"]),
            2 => changed.max_cost_usd = 10.0,
            _ => changed.name = "different".into(),
        }
        let host = Arc::new(TestHost::default());
        let (result, events) = execute(&workspace, &id, &changed, host.clone(), true);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("resume spec differs"));
        assert!(events.is_empty());
        assert_eq!(host.asks.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn edited_spec_file_cannot_redefine_original_confirmation() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let mut spec = prepare("original", r#"agent("a").ask("retry");"#, 5.0)
        .unwrap()
        .spec();
    assert!(execute(
        &workspace,
        &id,
        &spec,
        Arc::new(TestHost {
            fail_ask: 1,
            ..TestHost::default()
        }),
        false
    )
    .0
    .is_err());
    spec.max_cost_usd = 500.0;
    fs::write(
        run_dir(workspace.path(), &id).join("spec.json"),
        serde_json::to_vec(&spec).unwrap(),
    )
    .unwrap();
    let host = Arc::new(TestHost::default());
    let (result, events) = execute(&workspace, &id, &spec, host.clone(), true);
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("journal confirmation or engine caps changed"));
    assert!(events.is_empty());
    assert_eq!(host.asks.load(Ordering::SeqCst), 0);
}

#[test]
fn historical_failed_ask_spend_blocks_new_paid_work() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare(
        "budget",
        r#"phase("one"); agent("a").ask("paid"); agent("a").ask("retry");"#,
        1.5,
    )
    .unwrap()
    .spec();
    let (result, events) = execute(
        &workspace,
        &id,
        &spec,
        Arc::new(TestHost {
            fail_ask: 2,
            ..TestHost::default()
        }),
        false,
    );
    assert!(result.is_err());
    assert_eq!(terminal(&events).cost_usd, 2.0);
    let host = Arc::new(TestHost::default());
    let (result, events) = execute(&workspace, &id, &spec, host.clone(), true);
    assert!(matches!(
        result,
        Err(WorkflowError::BudgetExceeded { spent: 2.0, .. })
    ));
    assert_eq!(terminal(&events).status, RunStatus::BudgetExceeded);
    assert_eq!(terminal(&events).tokens, 200);
    assert_eq!(host.asks.load(Ordering::SeqCst), 0);
}

#[test]
fn interrupted_ask_redispatches_but_ambiguous_command_never_does() {
    for command in [false, true] {
        let workspace = Workspace::new();
        let id = RunId::new();
        let source = if command {
            r#"world.run("git", ["status"]);"#
        } else {
            r#"agent("a").ask("retry");"#
        };
        let spec = prepare("crash", source, 5.0).unwrap().spec();
        let host = Arc::new(TestHost {
            panic_command: command,
            panic_ask: !command,
            ..TestHost::default()
        });
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(
                &workspace, &id, &spec, host, false
            )))
            .is_err()
        );
        let host = Arc::new(TestHost::default());
        let (result, events) = execute(&workspace, &id, &spec, host.clone(), true);
        if command {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("ambiguous side effects"));
            assert!(events.is_empty());
            assert_eq!(host.commands.load(Ordering::SeqCst), 0);
        } else {
            assert!(result.is_ok());
            assert_eq!(host.asks.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn phase_only_cancellation_stops_persisted_and_original_api() {
    for persisted in [false, true] {
        let workspace = Workspace::new();
        let host = Arc::new(TestHost::default());
        let cancel = host.clone();
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();
        let sink: EventSink = Arc::new(move |event| {
            if matches!(event, WorkflowEvent::Phase { .. }) {
                cancel.cancelled.store(true, Ordering::SeqCst);
            }
            recorded.lock().unwrap().push(event);
        });
        let spec = prepare(
            "cancel",
            r#"phase("one"); phase("two"); log("never"); report("never");"#,
            5.0,
        )
        .unwrap()
        .spec();
        let result = if persisted {
            run_workflow_persisted(
                &spec,
                host.clone(),
                sink,
                RunOptions {
                    workspace_root: workspace.0.clone(),
                    run_id: RunId::new(),
                    resume: false,
                    allow_incomplete_asks: false,
                    acknowledge_partial_effects: false,
                },
            )
        } else {
            run_workflow(&spec, host.clone(), sink)
        };
        assert_eq!(result.unwrap_err(), WorkflowError::Stopped);
        let events = events.lock().unwrap();
        assert_eq!(terminal(&events).status, RunStatus::Stopped);
        assert_eq!(terminal(&events).phases, ["one"]);
        assert_eq!(host.displays.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn torn_tail_is_removed_before_append_and_interior_damage_fails_closed() {
    for interior in [false, true] {
        let workspace = Workspace::new();
        let id = RunId::new();
        let spec = prepare("torn", r#"agent("a").ask("retry");"#, 5.0)
            .unwrap()
            .spec();
        assert!(execute(
            &workspace,
            &id,
            &spec,
            Arc::new(TestHost {
                fail_ask: 1,
                ..TestHost::default()
            }),
            false
        )
        .0
        .is_err());
        let path = journal_path(workspace.path(), &id);
        let mut bytes = fs::read(&path).unwrap();
        if interior {
            bytes.splice(0..0, b"not json\n".iter().copied());
        } else {
            bytes.extend_from_slice(b"{\"seq\":999,\"kind\":\"engine_step_fin");
        }
        fs::write(&path, bytes).unwrap();
        let host = Arc::new(TestHost::default());
        let (result, _) = execute(&workspace, &id, &spec, host.clone(), true);
        if interior {
            assert!(result.is_err());
            assert_eq!(host.asks.load(Ordering::SeqCst), 0);
        } else {
            assert!(result.is_ok(), "{result:?}");
            let loaded = Journal::load(workspace.path(), id).unwrap();
            assert!(loaded.corrupt.is_empty());
            assert!(fs::read(path).unwrap().ends_with(b"\n"));
        }
    }
}

#[test]
fn invalid_paths_and_io_errors_fail_before_effects() {
    let workspace = Workspace::new();
    let spec = prepare("io", r#"world.run("git", ["status"]);"#, 5.0)
        .unwrap()
        .spec();
    for bad in ["../escape", "a/b", "CON", ""] {
        let id = RunId::from(bad);
        let host = Arc::new(TestHost::default());
        let (result, events) = execute(&workspace, &id, &spec, host.clone(), false);
        assert!(result.is_err());
        assert!(events.is_empty());
        assert_eq!(host.commands.load(Ordering::SeqCst), 0);
    }
    let id = RunId::new();
    let dir = run_dir(workspace.path(), &id);
    fs::create_dir_all(dir.join("journal.jsonl")).unwrap();
    let host = Arc::new(TestHost::default());
    assert!(execute(&workspace, &id, &spec, host.clone(), false)
        .0
        .is_err());
    assert_eq!(host.commands.load(Ordering::SeqCst), 0);
}

#[test]
fn exact_normalized_inputs_are_checked_even_when_labels_match() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare(
        "inputs",
        r#"git.diff("HEAD", "a"); agent("a").ask("retry");"#,
        5.0,
    )
    .unwrap()
    .spec();
    assert!(execute(
        &workspace,
        &id,
        &spec,
        Arc::new(TestHost {
            fail_ask: 1,
            ..TestHost::default()
        }),
        false
    )
    .0
    .is_err());
    let path = journal_path(workspace.path(), &id);
    let text = fs::read_to_string(&path).unwrap();
    let mut records: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let record = records
        .iter_mut()
        .find(|entry| entry["kind"] == "engine_step_started")
        .unwrap();
    record["payload"]["inputs"] = json!(["diff", "HEAD", "other"]);
    let text = records
        .iter()
        .map(|entry| format!("{entry}\n"))
        .collect::<String>();
    fs::write(path, text).unwrap();
    let host = Arc::new(TestHost::default());
    let (result, _) = execute(&workspace, &id, &spec, host.clone(), true);
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("inputs or identity changed"));
    assert_eq!(host.reads.load(Ordering::SeqCst), 0);
    assert_eq!(host.asks.load(Ordering::SeqCst), 0);
}

#[test]
fn report_tokens_default_when_loading_older_terminal_events() {
    let report = json!({"run_id":"old", "name":"old", "status":"completed", "elapsed_ms":0,
        "phases":[], "steps":[], "artifacts":[], "cost_usd":0.0, "error":null});
    let report: selora_workflow::WorkflowReport = serde_json::from_value(report).unwrap();
    assert_eq!(report.tokens, 0);
}

#[test]
fn completed_command_is_durable_before_js_receives_its_result() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare(
        "crash after command",
        r#"const gate = world.run("git", ["status"]); gate.stdout;"#,
        5.0,
    )
    .unwrap()
    .spec();
    let first = Arc::new(TestHost::default());
    let sink: EventSink = Arc::new(|event| {
        if matches!(event, WorkflowEvent::Step { step } if step.status == selora_workflow::StepStatus::Ok)
        {
            panic!("crash after journal sync, before JS receives command result");
        }
    });
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_workflow_persisted(
            &spec,
            first.clone(),
            sink,
            RunOptions {
                workspace_root: workspace.0.clone(),
                run_id: id.clone(),
                resume: false,
                allow_incomplete_asks: false,
                acknowledge_partial_effects: false,
            },
        )
    }))
    .is_err());
    assert_eq!(first.commands.load(Ordering::SeqCst), 1);
    let host = Arc::new(TestHost::default());
    let (result, _) = execute(&workspace, &id, &spec, host.clone(), true);
    assert_eq!(result.unwrap().value, json!("gate output\n"));
    assert_eq!(host.commands.load(Ordering::SeqCst), 0);
}

#[test]
fn complete_record_without_newline_gets_safe_append_boundary() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare("no newline", r#"agent("a").ask("retry");"#, 5.0)
        .unwrap()
        .spec();
    assert!(execute(
        &workspace,
        &id,
        &spec,
        Arc::new(TestHost {
            fail_ask: 1,
            ..TestHost::default()
        }),
        false
    )
    .0
    .is_err());
    let path = journal_path(workspace.path(), &id);
    let mut bytes = fs::read(&path).unwrap();
    assert_eq!(bytes.pop(), Some(b'\n'));
    fs::write(&path, bytes).unwrap();
    assert!(
        execute(&workspace, &id, &spec, Arc::new(TestHost::default()), true)
            .0
            .is_ok()
    );
    assert!(Journal::load(workspace.path(), id)
        .unwrap()
        .corrupt
        .is_empty());
}

#[test]
fn active_run_cannot_resume_concurrently() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare("exclusive", r#"phase("one");"#, 5.0)
        .unwrap()
        .spec();
    let root = workspace.0.clone();
    let run_id = id.clone();
    let approved = spec.clone();
    let sink: EventSink = Arc::new(move |event| {
        if matches!(event, WorkflowEvent::Run { .. }) {
            let host = Arc::new(TestHost::default());
            let result = run_workflow_persisted(
                &approved,
                host.clone(),
                Arc::new(|_| {}),
                RunOptions {
                    workspace_root: root.clone(),
                    run_id: run_id.clone(),
                    resume: true,
                    allow_incomplete_asks: false,
                    acknowledge_partial_effects: false,
                },
            );
            assert!(result.is_err());
            assert_eq!(host.asks.load(Ordering::SeqCst), 0);
        }
    });
    assert!(run_workflow_persisted(
        &spec,
        Arc::new(TestHost::default()),
        sink,
        RunOptions {
            workspace_root: workspace.0.clone(),
            run_id: id,
            resume: false,
            allow_incomplete_asks: false,
            acknowledge_partial_effects: false,
        }
    )
    .is_ok());
}

#[test]
fn failed_validation_still_records_paid_usage() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare(
        "invalid answer",
        r#"agent("a").ask({type:"number"}, "answer");"#,
        0.5,
    )
    .unwrap()
    .spec();
    let (result, events) = execute(&workspace, &id, &spec, Arc::new(TestHost::default()), false);
    assert!(matches!(result, Err(WorkflowError::SchemaMismatch { .. })));
    assert_eq!(terminal(&events).tokens, 100);
    assert_eq!(terminal(&events).cost_usd, 1.0);
    let host = Arc::new(TestHost::default());
    assert!(matches!(
        execute(&workspace, &id, &spec, host.clone(), true).0,
        Err(WorkflowError::BudgetExceeded { .. })
    ));
    assert_eq!(host.asks.load(Ordering::SeqCst), 0);
}

#[cfg(any(unix, windows))]
#[test]
fn symlinked_metadata_run_and_files_never_escape_workspace() {
    fn symlink(target: &Path, link: &Path, directory: bool) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            if directory {
                std::os::windows::fs::symlink_dir(target, link)
            } else {
                std::os::windows::fs::symlink_file(target, link)
            }
        }
        #[cfg(unix)]
        {
            let _ = directory;
            std::os::unix::fs::symlink(target, link)
        }
    }
    for component in [
        ".selora",
        ".selora/workflow-runs",
        ".selora/workflow-runs/id",
        ".selora/workflow-runs/id/spec.json",
        ".selora/workflow-runs/id/journal.jsonl",
    ] {
        let workspace = Workspace::new();
        let outside = Workspace::new();
        let link = workspace.path().join(component);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        let directory = !component.ends_with("json") && !component.ends_with("jsonl");
        let target = if directory {
            outside.0.clone()
        } else {
            let target = outside.path().join("sentinel");
            fs::write(&target, "sentinel").unwrap();
            target
        };
        if let Err(error) = symlink(&target, &link, directory) {
            #[cfg(windows)]
            if error.raw_os_error() == Some(1314) {
                eprintln!(
                    "symlink containment checks skipped: Windows symlink privilege unavailable"
                );
                return;
            }
            panic!("cannot create test symlink: {error}");
        }
        let spec = prepare("links", r#"agent("a").ask("never");"#, 5.0)
            .unwrap()
            .spec();
        let host = Arc::new(TestHost::default());
        assert!(
            execute(&workspace, &RunId::from("id"), &spec, host.clone(), false)
                .0
                .is_err()
        );
        assert!(load_workflow_spec(workspace.path(), &RunId::from("id")).is_err());
        assert_eq!(host.asks.load(Ordering::SeqCst), 0);
        if !directory {
            assert_eq!(fs::read_to_string(target).unwrap(), "sentinel");
        }
    }
}

#[test]
fn journal_records_every_step_phase_report_and_artifact_content() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare(
        "history",
        r##"
        report("before any phase");
        phase("scan");
        const first = files.read("a");
        report("inside scan", "note");
        artifact.markdown("note", "artifact body");
        phase("fix");
        files.read("b");
        report({fixed: true});
        phase("verify");
        report("after phases");
    "##,
        5.0,
    )
    .unwrap()
    .spec();
    let (result, _) = execute(&workspace, &id, &spec, Arc::new(TestHost::default()), false);
    result.unwrap();

    let loaded = Journal::load(workspace.path(), id).unwrap();
    assert!(loaded.corrupt.is_empty(), "{:?}", loaded.corrupt);

    // Every step start/finish carries the phase in effect at that moment.
    let starts: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.kind == "engine_step_started")
        .collect();
    assert_eq!(starts.len(), 10, "every facade call is a step");
    assert!(starts[0].payload.get("phase").is_none(), "no phase yet");
    assert!(
        starts[1].payload.get("phase").is_none(),
        "a phase step starts before its own marker"
    );
    assert_eq!(starts[2].payload["phase"], "scan");
    assert_eq!(starts[3].payload["phase"], "scan");
    assert_eq!(starts[4].payload["phase"], "scan");
    assert_eq!(
        starts[5].payload["phase"], "scan",
        "the fix step starts in scan"
    );
    assert_eq!(starts[6].payload["phase"], "fix");
    assert_eq!(starts[9].payload["phase"], "verify");
    let phase_step = loaded
        .entries
        .iter()
        .filter(|entry| entry.kind == "engine_step_finished")
        .find(|entry| {
            entry.payload["step"]["kind"] == "phase" && entry.payload["step"]["detail"] == "fix"
        })
        .expect("the fix phase step finished");
    assert_eq!(
        phase_step.payload["phase"], "fix",
        "a phase step finishes inside the phase it opened"
    );

    // Report items are explicit records with their live ordinal and phase.
    let reports: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.kind == "engine_report_item")
        .collect();
    assert_eq!(reports.len(), 4);
    assert_eq!(reports[0].payload["index"], 1);
    assert_eq!(reports[0].payload["item"], "before any phase");
    assert!(reports[0].payload.get("phase").is_none());
    assert_eq!(reports[1].payload["index"], 2);
    assert_eq!(reports[1].payload["phase"], "scan");
    assert_eq!(reports[1].payload["artifact_id"], "note");
    assert_eq!(reports[2].payload["phase"], "fix");
    assert_eq!(reports[2].payload["item"], json!({"fixed": true}));
    assert_eq!(reports[3].payload["phase"], "verify");

    // Artifact content and an absent title are durable even without the
    // terminal report's copy.
    let artifacts: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.kind == "engine_artifact")
        .collect();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].payload["artifact"]["id"], "note");
    assert!(artifacts[0].payload["artifact"]["title"].is_null());
    assert_eq!(
        artifacts[0].payload["artifact"]["content"]["kind"],
        "markdown"
    );
    assert_eq!(
        artifacts[0].payload["artifact"]["content"]["body"],
        "artifact body"
    );
    assert_eq!(artifacts[0].payload["phase"], "scan");
}

#[test]
fn a_failed_report_step_is_never_journaled_as_reported() {
    let workspace = Workspace::new();
    let id = RunId::new();
    let spec = prepare(
        "failed report",
        r#"
        phase("scan");
        report("kept");
        report("x".repeat(40000));
    "#,
        5.0,
    )
    .unwrap()
    .spec();
    let (result, events) = execute(&workspace, &id, &spec, Arc::new(TestHost::default()), false);
    assert!(result.is_err(), "the oversized item aborts the run");
    assert_eq!(terminal(&events).status, RunStatus::Failed);

    let loaded = Journal::load(workspace.path(), id).unwrap();
    let reports: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.kind == "engine_report_item")
        .collect();
    assert_eq!(reports.len(), 1, "only the item that reached the host");
    assert_eq!(reports[0].payload["index"], 1);
    assert_eq!(reports[0].payload["item"], "kept");
    let failed = loaded
        .entries
        .iter()
        .filter(|entry| entry.kind == "engine_step_finished")
        .find(|entry| {
            entry.payload["step"]["kind"] == "report" && entry.payload["step"]["status"] == "failed"
        })
        .expect("the oversized report step failed");
    assert_eq!(failed.payload["phase"], "scan");
}
