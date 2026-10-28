//! Workflow-run overhead harness (scripts/perf.ps1).
//!
//! Measures the engine alone: confirmation parsing (`prepare`), a no-op run
//! (boa realm construction + teardown), and the marginal cost of facade steps
//! (`world.run`). The shell's journal writes, IPC emits, and agent turns are
//! outside this crate and therefore outside these numbers.
//!
//! Every test is `#[ignore]`d; the harness runs them in release mode with
//! `--ignored --nocapture` and records the `PERF_JSON` line.

use std::sync::Arc;
use std::time::Instant;

use selora_workflow::{
    prepare, run_workflow, CommandOutput, EventSink, GitCommit, GrepMatch, Host, HostError,
    WorkflowEvent, WorkflowSpec,
};

/// A host that does no real work, so the measurement isolates engine overhead.
struct NullHost;

impl Host for NullHost {
    fn ask(
        &self,
        _agent: &str,
        _persona: Option<&str>,
        _instructions: &str,
    ) -> Result<serde_json::Value, HostError> {
        Ok(serde_json::json!({ "ok": true }))
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

    fn report(
        &self,
        _item: &serde_json::Value,
        _artifact_id: Option<&str>,
    ) -> Result<(), HostError> {
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
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[rank]
}

fn summary(samples: &mut [u128]) -> serde_json::Value {
    samples.sort_unstable();
    serde_json::json!({
        "n": samples.len(),
        "p50_us": percentile(samples, 0.50),
        "p95_us": percentile(samples, 0.95),
        "max_us": samples.last().copied().unwrap_or(0),
    })
}

fn spec(source: &str) -> WorkflowSpec {
    prepare("perf", source, 5.0)
        .expect("script prepares")
        .spec()
}

fn run_once(spec: &WorkflowSpec) -> u128 {
    let sink: EventSink = Arc::new(|_event: WorkflowEvent| {});
    let timer = Instant::now();
    let outcome = run_workflow(spec, Arc::new(NullHost), sink).expect("run completes");
    let elapsed = timer.elapsed().as_micros();
    assert!(outcome.report.status.is_terminal_ok());
    elapsed
}

#[test]
#[ignore = "performance harness: run scripts/perf.ps1"]
fn perf_workflow_run_overhead() {
    const PREPARE_RUNS: usize = 30;
    const EMPTY_RUNS: usize = 20;
    const STEP_RUNS: usize = 10;
    const PHASE_RUNS: usize = 10;
    const STEPS: usize = 50;

    let mut prepare_samples = Vec::with_capacity(PREPARE_RUNS);
    for _ in 0..PREPARE_RUNS {
        let timer = Instant::now();
        let confirmation = prepare("perf", "phase(\"start\");", 5.0).expect("prepares");
        prepare_samples.push(timer.elapsed().as_micros());
        std::hint::black_box(confirmation);
    }

    let empty = spec("phase(\"start\");");
    let mut empty_samples = Vec::with_capacity(EMPTY_RUNS);
    for _ in 0..EMPTY_RUNS {
        empty_samples.push(run_once(&empty));
    }

    let mut step_source = String::from("phase(\"start\");\n");
    for _ in 0..STEPS {
        step_source.push_str("world.run(\"git\", [\"status\"]);\n");
    }
    let step_spec = spec(&step_source);
    let mut step_samples = Vec::with_capacity(STEP_RUNS);
    for _ in 0..STEP_RUNS {
        step_samples.push(run_once(&step_spec));
    }

    let mut phase_source = String::from("phase(\"start\");\n");
    for index in 0..STEPS {
        phase_source.push_str(&format!("phase(\"p{index}\");\n"));
    }
    let phase_spec = spec(&phase_source);
    let mut phase_samples = Vec::with_capacity(PHASE_RUNS);
    for _ in 0..PHASE_RUNS {
        phase_samples.push(run_once(&phase_spec));
    }

    let empty_p50 = summary(&mut empty_samples.clone())
        .get("p50_us")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u128;
    let step_p50 = summary(&mut step_samples.clone())
        .get("p50_us")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u128;
    let per_step_us = step_p50.saturating_sub(empty_p50) / STEPS as u128;

    println!(
        "PERF_JSON {}",
        serde_json::json!({
            "metric": "workflow_run_overhead",
            "prepare": summary(&mut prepare_samples),
            "empty_run": summary(&mut empty_samples),
            "world_run_steps": summary(&mut step_samples),
            "phase_steps": summary(&mut phase_samples),
            "steps_per_run": STEPS,
            "marginal_world_run_step_us": per_step_us,
        })
    );
}
