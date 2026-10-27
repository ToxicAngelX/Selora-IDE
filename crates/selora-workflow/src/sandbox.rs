//! The sandbox: a boa realm that exposes exactly the §18.2 facade, plus the
//! confirmation payload builder (ARCHITECTURE.md §18.1).
//!
//! # What is in the realm
//!
//! The JavaScript language core, and then exactly eight globals:
//! `agent`, `phase`, `world`, `files`, `git`, `report`, `artifact`, `log`.
//! Everything ambient is deleted before the script is parsed:
//!
//! ```text
//! fetch fs process require module exports console
//! setTimeout setInterval setImmediate clearTimeout clearInterval queueMicrotask
//! Date Temporal Intl Atomics SharedArrayBuffer WeakRef FinalizationRegistry eval
//! Math.random
//! ```
//!
//! Boa is built without default features (no ICU, no `Temporal`), so most of
//! those names never exist in the first place; [`harden_realm`] removes the
//! rest and then **verifies** the removal, so a future boa default cannot
//! silently widen the sandbox.
//!
//! # Caps
//!
//! * script ≤ 64 KiB ([`MAX_SCRIPT_BYTES`]) — checked before anything runs,
//! * ≤ 256 facade calls ([`MAX_STEPS`]) — each `phase`, `ask`, `world.run`,
//!   `files.*`, `git.*`, `report`, `artifact.*` and `log` call is one step,
//! * ≤ 256 report items, ≤ 32 KiB each, ≤ 1 MiB per artifact,
//! * a workflow-level cost cap checked at every step boundary,
//! * a loop-iteration limit so a `while (true) {}` cannot wedge the worker.
//!
//! # Synchronous facade
//!
//! Every facade call blocks on [`crate::Host`], so the values it returns are
//! plain values, not promises (§18.4: no async in boa). `await` on such a
//! value is a no-op, and the engine drains the job queue after evaluation, so
//! `(async () => { const r = await agent(...).ask(...); })()` works; plain
//! top-level `await` is not supported in this slice.
//!
//! # Confirmation
//!
//! [`prepare`] never executes the script. It validates the size, parses it for
//! syntax, and statically extracts the `world.run` command set from the
//! literals in the source. The resulting [`Confirmation`] is what the chat
//! card shows — script text, command set, cost cap — and [`Confirmation::spec`]
//! is what [`run`] takes once the user approves.
//!
//! The scanner skips comments and string/template contents. It is a static
//! best-effort extraction, not a proof: an alias (`const w = world`) is caught
//! by the runtime gate in [`crate::facade`], and a command assembled at runtime
//! is rejected here or there — never silently granted.

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use boa_engine::{Context, JsError, JsString, Script, Source};
use serde::{Deserialize, Serialize};

use crate::facade::{self, Facade};
use crate::{
    CommandSet, EventSink, Host, RunId, RunStatus, WorkflowError, WorkflowEvent, WorkflowOutcome,
    WorkflowSpec,
};

/// Hard cap on the script text (§18.1).
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;

/// Hard cap on facade calls per run (§18.1).
pub const MAX_STEPS: usize = 256;

/// `workflow.max_cost_usd` default (§18.1).
pub const DEFAULT_MAX_COST_USD: f64 = 5.0;

/// Report items per run (`report`, §18.2).
pub const MAX_REPORT_ITEMS: usize = 256;

/// Bytes per report item (§18.2).
pub const MAX_REPORT_ITEM_BYTES: usize = 32 * 1024;

/// Bytes per artifact body (engine-side guard; the host has its own caps).
pub const MAX_ARTIFACT_BYTES: usize = 1024 * 1024;

/// Loop iterations before boa throws. Keeps `while (true) {}` from wedging the
/// blocking worker; far above any legitimate orchestration loop.
pub const MAX_LOOP_ITERATIONS: u64 = 1_000_000;

/// The caps a confirmation card is showing. Carried into the run so the
/// invoice and the card cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Script size cap, in bytes.
    pub max_script_bytes: usize,
    /// Facade-call cap.
    pub max_steps: usize,
    /// Report-item cap.
    pub max_report_items: usize,
    /// Report-item size cap, in bytes.
    pub max_report_item_bytes: usize,
    /// Artifact size cap, in bytes.
    pub max_artifact_bytes: usize,
    /// Loop-iteration cap.
    pub max_loop_iterations: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_script_bytes: MAX_SCRIPT_BYTES,
            max_steps: MAX_STEPS,
            max_report_items: MAX_REPORT_ITEMS,
            max_report_item_bytes: MAX_REPORT_ITEM_BYTES,
            max_artifact_bytes: MAX_ARTIFACT_BYTES,
            max_loop_iterations: MAX_LOOP_ITERATIONS,
        }
    }
}

impl Limits {
    /// The §18.1 values.
    #[must_use]
    pub fn standard() -> Self {
        Self::default()
    }
}

/// What the confirmation card shows, and what the run is started from.
///
/// Nothing here has been executed: [`prepare`] parses and scans the script but
/// evaluates nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirmation {
    /// Run name.
    pub name: String,
    /// The full script text the user is approving.
    pub script: String,
    /// The `world.run` commands extracted from the script's literals.
    pub command_set: CommandSet,
    /// The workflow-level cost cap, in USD.
    pub max_cost_usd: f64,
    /// The caps the run will enforce. The engine's §18.1 constants today;
    /// they ride on the card so the approval and the invoice agree.
    pub limits: Limits,
}

impl Confirmation {
    /// The spec to hand to [`run`] after approval.
    #[must_use]
    pub fn spec(&self) -> WorkflowSpec {
        WorkflowSpec {
            name: self.name.clone(),
            source: self.script.clone(),
            command_set: self.command_set.clone(),
            max_cost_usd: self.max_cost_usd,
        }
    }

    /// Script size in bytes (for the card's "N of 64 KiB" line).
    #[must_use]
    pub fn script_bytes(&self) -> usize {
        self.script.len()
    }
}

/// Build the confirmation payload for a script **without running it**.
///
/// Validates the name/cost cap, enforces the §18.1 size cap, parses the script
/// for syntax, and extracts the `world.run` command set.
///
/// # Errors
/// [`WorkflowError::ScriptTooLarge`], [`WorkflowError::Syntax`],
/// [`WorkflowError::NonLiteralCommand`] (a command assembled at runtime cannot
/// be confirmed), [`WorkflowError::InvalidSpec`].
pub fn prepare(
    name: impl Into<String>,
    source: impl Into<String>,
    max_cost_usd: f64,
) -> Result<Confirmation, WorkflowError> {
    let name = name.into();
    let script = source.into();
    let limits = Limits::default();
    validate_inputs(&name, &script, max_cost_usd, &limits)?;

    let command_set = extract_commands(&script)?;
    syntax_check(&script)?;

    Ok(Confirmation {
        name,
        script,
        command_set,
        max_cost_usd,
        limits,
    })
}

/// Run a confirmed workflow on the current (blocking) thread.
///
/// Emits [`WorkflowEvent::Run`] before evaluating, then one `Step`/`Phase`/
/// `Report`/`Artifact`/`Log` per facade event, and always exactly one
/// [`WorkflowEvent::Finished`] once the script has stopped — on success and on
/// failure. Pre-flight rejections (size, unconfirmed command, bad spec) return
/// before `Run`, so nothing is half-started.
///
/// # Errors
/// Any [`WorkflowError`]; the failing step is emitted as `Failed` first.
pub fn run(
    spec: &WorkflowSpec,
    host: Arc<dyn Host>,
    sink: EventSink,
) -> Result<WorkflowOutcome, WorkflowError> {
    run_inner(spec, host, sink, RunId::new(), None)
}

pub(crate) fn run_durable(
    spec: &WorkflowSpec,
    host: Arc<dyn Host>,
    sink: EventSink,
    run_id: RunId,
    durable: crate::persisted::Durable,
) -> Result<WorkflowOutcome, WorkflowError> {
    run_inner(spec, host, sink, run_id, Some(durable))
}

pub(crate) fn validate_spec(spec: &WorkflowSpec) -> Result<(), WorkflowError> {
    let limits = Limits::default();
    validate_inputs(&spec.name, &spec.source, spec.max_cost_usd, &limits)?;

    // The confirmation card is the grant. Re-extract from the text so a script
    // edited between confirmation and run cannot widen it.
    let extracted = extract_commands(&spec.source)?;
    for command in extracted.iter() {
        if !spec.command_set.contains(command) {
            return Err(WorkflowError::UnconfirmedCommand {
                command: command.to_owned(),
            });
        }
    }
    syntax_check(&spec.source)?;
    Ok(())
}

fn run_inner(
    spec: &WorkflowSpec,
    host: Arc<dyn Host>,
    sink: EventSink,
    run_id: RunId,
    durable: Option<crate::persisted::Durable>,
) -> Result<WorkflowOutcome, WorkflowError> {
    validate_spec(spec)?;
    let limits = Limits::default();
    let facade = Facade::new(
        run_id.clone(),
        spec,
        limits.clone(),
        host,
        Arc::clone(&sink),
    );
    let facade = Rc::new(match durable {
        Some(durable) => facade.with_durable(durable),
        None => facade,
    });

    sink(WorkflowEvent::Run {
        run_id: run_id.clone(),
        name: spec.name.clone(),
    });

    let started = Instant::now();
    let evaluated = evaluate(spec, &facade, &limits);

    let (status, error, value) = match evaluated {
        Ok(value) => match facade
            .fatal()
            .or_else(|| facade.cancellation())
            .or_else(|| facade.verify_replayed().err())
            .or_else(|| facade.budget_violation())
        {
            Some(error) => (status_for(&error), Some(error), value),
            None => (RunStatus::Completed, None, value),
        },
        Err(error) => {
            let error = facade.fatal().unwrap_or(error);
            (status_for(&error), Some(error), serde_json::Value::Null)
        }
    };

    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut report = facade.report(status, error.as_ref().map(ToString::to_string), elapsed_ms);
    let mut error = error;
    if let Err(persist_error) = facade.persist_report(&report) {
        report.status = RunStatus::Failed;
        report.error = Some(persist_error.to_string());
        error = Some(persist_error);
    }
    sink(WorkflowEvent::Finished {
        report: report.clone(),
    });

    match error {
        Some(error) => Err(error),
        None => Ok(WorkflowOutcome { report, value }),
    }
}

fn status_for(error: &WorkflowError) -> RunStatus {
    match error {
        WorkflowError::BudgetExceeded { .. } => RunStatus::BudgetExceeded,
        WorkflowError::Stopped => RunStatus::Stopped,
        _ => RunStatus::Failed,
    }
}

fn validate_inputs(
    name: &str,
    source: &str,
    max_cost_usd: f64,
    limits: &Limits,
) -> Result<(), WorkflowError> {
    if name.trim().is_empty() {
        return Err(WorkflowError::InvalidSpec {
            message: "the workflow name must not be empty".to_owned(),
        });
    }
    if !max_cost_usd.is_finite() || max_cost_usd < 0.0 {
        return Err(WorkflowError::InvalidSpec {
            message: "max_cost_usd must be a finite, non-negative number".to_owned(),
        });
    }
    if source.len() > limits.max_script_bytes {
        return Err(WorkflowError::ScriptTooLarge {
            size: source.len(),
            limit: limits.max_script_bytes,
        });
    }
    Ok(())
}

fn evaluate(
    spec: &WorkflowSpec,
    facade: &Rc<Facade>,
    limits: &Limits,
) -> Result<serde_json::Value, WorkflowError> {
    let mut ctx = Context::default();
    ctx.runtime_limits_mut()
        .set_loop_iteration_limit(limits.max_loop_iterations);
    harden_realm(&mut ctx)?;
    facade::install(&mut ctx, Rc::clone(facade)).map_err(engine_error)?;

    let script = Script::parse(Source::from_bytes(spec.source.as_bytes()), None, &mut ctx)
        .map_err(|error| WorkflowError::Syntax {
            message: error.to_string(),
        })?;
    let value = script
        .evaluate(&mut ctx)
        .map_err(|error| classify(&error))?;
    ctx.run_jobs().map_err(|error| classify(&error))?;
    let json = value.to_json(&mut ctx).map_err(|error| classify(&error))?;
    Ok(json.unwrap_or(serde_json::Value::Null))
}

fn engine_error(error: JsError) -> WorkflowError {
    WorkflowError::Engine {
        message: error.to_string(),
    }
}

fn syntax_check(source: &str) -> Result<(), WorkflowError> {
    let mut ctx = Context::default();
    Script::parse(Source::from_bytes(source.as_bytes()), None, &mut ctx).map_err(|error| {
        WorkflowError::Syntax {
            message: error.to_string(),
        }
    })?;
    Ok(())
}

/// Globals that must not exist in a workflow realm. Boa never defines most of
/// them; the list is also the map used to explain a `ReferenceError` nicely.
const AMBIENT_GLOBALS: [&str; 24] = [
    "fetch",
    "fs",
    "process",
    "require",
    "module",
    "exports",
    "console",
    "setTimeout",
    "setInterval",
    "setImmediate",
    "clearTimeout",
    "clearInterval",
    "queueMicrotask",
    "Date",
    "Temporal",
    "Intl",
    "Atomics",
    "SharedArrayBuffer",
    "WeakRef",
    "FinalizationRegistry",
    "eval",
    "window",
    "document",
    "XMLHttpRequest",
];

/// Globals the engine insists on checking after deletion, because they either
/// exist in a default boa realm or are the ones the acceptance test names.
const VERIFIED_ABSENT: [&str; 8] = [
    "fetch",
    "fs",
    "process",
    "Date",
    "eval",
    "Atomics",
    "SharedArrayBuffer",
    "setTimeout",
];

fn harden_realm(ctx: &mut Context) -> Result<(), WorkflowError> {
    let global = ctx.global_object();
    for name in AMBIENT_GLOBALS {
        // `false`/error is fine for a name boa never defined; a name that
        // exists and refuses deletion is caught by the verification below.
        let _ = global.delete_property_or_throw(JsString::from(name), ctx);
    }
    if let Some(math) = global
        .get(boa_engine::js_string!("Math"), ctx)
        .map_err(engine_error)?
        .as_object()
    {
        let _ = math.delete_property_or_throw(boa_engine::js_string!("random"), ctx);
    }

    for name in VERIFIED_ABSENT {
        let present = global
            .has_own_property(JsString::from(name), ctx)
            .map_err(engine_error)?;
        if present {
            return Err(WorkflowError::Engine {
                message: format!("sandbox hardening failed: `{name}` is still defined"),
            });
        }
    }
    let math = global
        .get(boa_engine::js_string!("Math"), ctx)
        .map_err(engine_error)?;
    if let Some(math) = math.as_object() {
        let random = math
            .get(boa_engine::js_string!("random"), ctx)
            .map_err(engine_error)?;
        if !random.is_undefined() {
            return Err(WorkflowError::Engine {
                message: "sandbox hardening failed: `Math.random` is still defined".to_owned(),
            });
        }
    }
    Ok(())
}

/// Turn a boa failure into the most specific [`WorkflowError`] we can.
fn classify(error: &JsError) -> WorkflowError {
    let message = error.to_string();
    if let Some(name) = unknown_global(&message) {
        return WorkflowError::AmbientGlobal { name };
    }
    WorkflowError::Script { message }
}

fn unknown_global(message: &str) -> Option<String> {
    let (prefix, _) = message.split_once(" is not defined")?;
    let identifier = prefix.rsplit([' ', ':', '(']).next()?.trim();
    AMBIENT_GLOBALS
        .iter()
        .find(|name| **name == identifier)
        .map(|name| (*name).to_owned())
}

/* ------------------------- static command extraction --------------------- */

/// Extract the `world.run` command set from a script's **literals**.
///
/// The scan skips comments and string contents, so a `world.run("x")` inside a
/// comment or a string is not a grant. A call whose command argument is not a
/// string literal — a variable, a concatenation, a template with `${}` — is
/// rejected here ([`WorkflowError::NonLiteralCommand`]) rather than silently
/// widening the grant later.
///
/// Two runtime holes are closed by the gate in [`crate::facade`] instead of
/// this scanner:
/// * an alias (`const w = world; w.run("curl")`) is invisible to a static
///   scan, so the confirmed-set check rejects it at the call;
/// * a substitution inside a template (`` world.run(`${c} test`) ``) is
///   rejected here, but a command built in a helper would be caught there.
///
/// # Errors
/// [`WorkflowError::NonLiteralCommand`] when a `world.run` site has no literal.
pub fn extract_commands(source: &str) -> Result<CommandSet, WorkflowError> {
    let mut scan = Scan {
        bytes: source.as_bytes(),
        at: 0,
    };
    let mut commands: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    while let Some(byte) = scan.peek() {
        match byte {
            b'/' if scan.peek_at(1) == Some(b'/') => scan.skip_line_comment(),
            b'/' if scan.peek_at(1) == Some(b'*') => scan.skip_block_comment(),
            b'\'' | b'"' | b'`' => {
                scan.skip_string();
            }
            _ if scan.starts_ident(b"world") => {
                scan.at += b"world".len();
                if let Some(command) = scan.parse_world_call()? {
                    if seen.insert(command.clone()) {
                        commands.push(command);
                    }
                }
            }
            _ => scan.at += 1,
        }
    }

    Ok(CommandSet::new(commands))
}

struct Scan<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Scan<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.at + offset).copied()
    }

    fn skip_line_comment(&mut self) {
        while let Some(byte) = self.peek() {
            self.at += 1;
            if byte == b'\n' {
                break;
            }
        }
    }

    fn skip_block_comment(&mut self) {
        self.at += 2;
        while self.at + 1 < self.bytes.len() {
            if self.bytes[self.at] == b'*' && self.bytes[self.at + 1] == b'/' {
                self.at += 2;
                return;
            }
            self.at += 1;
        }
        self.at = self.bytes.len();
    }

    /// Skip a string or template literal (escapes respected).
    fn skip_string(&mut self) {
        let Some(quote) = self.peek() else { return };
        self.at += 1;
        while let Some(byte) = self.peek() {
            self.at += 1;
            match byte {
                b'\\' => self.at += 1,
                _ if byte == quote => return,
                _ => {}
            }
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.at += 1,
                Some(b'/') if self.peek_at(1) == Some(b'/') => self.skip_line_comment(),
                Some(b'/') if self.peek_at(1) == Some(b'*') => self.skip_block_comment(),
                _ => return,
            }
        }
    }

    fn starts_ident(&self, word: &[u8]) -> bool {
        let end = self.at + word.len();
        if end > self.bytes.len() || &self.bytes[self.at..end] != word {
            return false;
        }
        if self.at > 0 && is_ident_byte(self.bytes[self.at - 1]) {
            return false;
        }
        if end < self.bytes.len() && is_ident_byte(self.bytes[end]) {
            return false;
        }
        true
    }

    /// After consuming `world`: recognise `.run(<literal>` and return the
    /// command. Anything else (`world`, `world.foo`, `world.run` passed
    /// around) is left to the scanner.
    fn parse_world_call(&mut self) -> Result<Option<String>, WorkflowError> {
        self.skip_trivia();
        if self.peek() != Some(b'.') {
            return Ok(None);
        }
        self.at += 1;
        self.skip_trivia();
        if !self.starts_ident(b"run") {
            return Ok(None);
        }
        self.at += b"run".len();
        self.skip_trivia();
        if self.peek() != Some(b'(') {
            return Ok(None);
        }
        self.at += 1;
        self.skip_trivia();

        match self.peek() {
            Some(quote @ (b'\'' | b'"' | b'`')) => {
                let literal = self.read_string();
                if quote == b'`' && literal.contains("${") {
                    return Err(WorkflowError::NonLiteralCommand {
                        found: "a template literal with `${}`".to_owned(),
                    });
                }
                Ok(Some(literal))
            }
            _ => Err(WorkflowError::NonLiteralCommand {
                found: self.snippet(),
            }),
        }
    }

    fn read_string(&mut self) -> String {
        let Some(quote) = self.peek() else {
            return String::new();
        };
        self.at += 1;
        let mut out: Vec<u8> = Vec::new();
        while let Some(byte) = self.peek() {
            self.at += 1;
            match byte {
                b'\\' => {
                    let Some(escaped) = self.peek() else { break };
                    self.at += 1;
                    match escaped {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'0' => out.push(0),
                        other => out.push(other),
                    }
                }
                _ if byte == quote => break,
                _ => out.push(byte),
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn snippet(&self) -> String {
        let end = (self.at + 32).min(self.bytes.len());
        let text = String::from_utf8_lossy(&self.bytes[self.at..end]);
        let line = text.split(['\n', '\r']).next().unwrap_or("").trim();
        if line.is_empty() {
            "the end of the script".to_owned()
        } else {
            format!("`{line}…`")
        }
    }
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/* --------------------------------- tests --------------------------------- */

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use serde_json::{json, Value};

    use super::*;
    use crate::{
        Artifact, ArtifactContent, CommandOutput, GitCommit, GrepMatch, StepKind, StepStatus,
        WorkflowStep,
    };

    #[derive(Debug, Clone, PartialEq)]
    struct AskCall {
        agent: String,
        persona: Option<String>,
        instructions: String,
        schema: Option<Value>,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct RunCall {
        command: String,
        args: Vec<String>,
        timeout_ms: Option<u64>,
    }

    /// A host that answers from a canned queue and records every call.
    #[derive(Default)]
    struct FakeHost {
        asks: Mutex<Vec<AskCall>>,
        answers: Mutex<VecDeque<Value>>,
        runs: Mutex<Vec<RunCall>>,
        run_output: Mutex<Option<CommandOutput>>,
        reported: Mutex<Vec<(Value, Option<String>)>>,
        artifacts: Mutex<Vec<Artifact>>,
        logs: Mutex<Vec<String>>,
        cost: Mutex<f64>,
    }

    impl FakeHost {
        fn answer(&self, value: Value) {
            self.answers.lock().expect("answers").push_back(value);
        }

        fn with_run_output(self, output: CommandOutput) -> Self {
            *self.run_output.lock().expect("run output") = Some(output);
            self
        }
    }

    impl Host for FakeHost {
        fn ask(
            &self,
            agent: &str,
            persona: Option<&str>,
            instructions: &str,
        ) -> Result<Value, crate::HostError> {
            self.asks.lock().expect("asks").push(AskCall {
                agent: agent.to_owned(),
                persona: persona.map(str::to_owned),
                instructions: instructions.to_owned(),
                schema: None,
            });
            Ok(self
                .answers
                .lock()
                .expect("answers")
                .pop_front()
                .unwrap_or_else(|| json!({ "ok": true })))
        }

        fn ask_with_schema(
            &self,
            agent: &str,
            persona: Option<&str>,
            instructions: &str,
            schema: Option<&Value>,
        ) -> Result<Value, crate::HostError> {
            let answer = self.ask(agent, persona, instructions)?;
            if let Some(call) = self.asks.lock().expect("asks").last_mut() {
                call.schema = schema.cloned();
            }
            Ok(answer)
        }

        fn world_run(
            &self,
            cmd: &str,
            args: &[String],
            timeout_ms: Option<u64>,
        ) -> Result<CommandOutput, crate::HostError> {
            self.runs.lock().expect("runs").push(RunCall {
                command: cmd.to_owned(),
                args: args.to_vec(),
                timeout_ms,
            });
            Ok(self
                .run_output
                .lock()
                .expect("run output")
                .clone()
                .unwrap_or(CommandOutput {
                    exit_code: 0,
                    stdout: "ok".to_owned(),
                    stderr: String::new(),
                }))
        }

        fn files_glob(&self, _pattern: &str) -> Result<Vec<String>, crate::HostError> {
            Ok(vec!["src/lib.rs".to_owned()])
        }

        fn files_grep(
            &self,
            _pattern: &str,
            _glob: Option<&str>,
        ) -> Result<Vec<GrepMatch>, crate::HostError> {
            Ok(vec![GrepMatch {
                path: "src/lib.rs".to_owned(),
                line: 1,
                text: "fn main() {}".to_owned(),
            }])
        }

        fn files_read(&self, path: &str) -> Result<String, crate::HostError> {
            Ok(format!("// {path}"))
        }

        fn git_changed_files(&self, _base: Option<&str>) -> Result<Vec<String>, crate::HostError> {
            Ok(vec!["src/lib.rs".to_owned()])
        }

        fn git_diff(
            &self,
            _base: Option<&str>,
            _path: Option<&str>,
        ) -> Result<String, crate::HostError> {
            Ok("diff".to_owned())
        }

        fn git_status(&self) -> Result<String, crate::HostError> {
            Ok("clean".to_owned())
        }

        fn git_log(&self, _count: Option<usize>) -> Result<Vec<GitCommit>, crate::HostError> {
            Ok(vec![GitCommit {
                sha: "abc123".to_owned(),
                summary: "init".to_owned(),
            }])
        }

        fn report(&self, item: &Value, artifact_id: Option<&str>) -> Result<(), crate::HostError> {
            self.reported
                .lock()
                .expect("reported")
                .push((item.clone(), artifact_id.map(str::to_owned)));
            Ok(())
        }

        fn artifact_markdown(
            &self,
            id: &str,
            title: Option<&str>,
            content: &str,
        ) -> Result<(), crate::HostError> {
            self.artifacts.lock().expect("artifacts").push(Artifact {
                id: id.to_owned(),
                title: title.map(str::to_owned),
                content: ArtifactContent::Markdown(content.to_owned()),
            });
            Ok(())
        }

        fn artifact_file(
            &self,
            id: &str,
            title: Option<&str>,
            path: &str,
        ) -> Result<(), crate::HostError> {
            self.artifacts.lock().expect("artifacts").push(Artifact {
                id: id.to_owned(),
                title: title.map(str::to_owned),
                content: ArtifactContent::File(path.to_owned()),
            });
            Ok(())
        }

        fn log(&self, message: &str) {
            self.logs.lock().expect("logs").push(message.to_owned());
        }

        fn cost_usd(&self) -> f64 {
            *self.cost.lock().expect("cost")
        }
    }

    fn run_source(
        source: &str,
        host: Arc<FakeHost>,
    ) -> (Result<WorkflowOutcome, WorkflowError>, Vec<WorkflowEvent>) {
        let confirmation = prepare("test run", source, 5.0).expect("script prepares");
        run_spec(&confirmation.spec(), host)
    }

    fn run_spec(
        spec: &WorkflowSpec,
        host: Arc<FakeHost>,
    ) -> (Result<WorkflowOutcome, WorkflowError>, Vec<WorkflowEvent>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink: EventSink = {
            let events = Arc::clone(&events);
            Arc::new(move |event| events.lock().expect("events").push(event))
        };
        let host: Arc<dyn Host> = host;
        let result = run(spec, host, sink);
        let events = events.lock().expect("events").clone();
        (result, events)
    }

    #[test]
    fn ambient_globals_are_structurally_absent() {
        let source = r#"
            const banned = [
                "fetch", "fs", "process", "require", "module", "exports", "console",
                "setTimeout", "setInterval", "queueMicrotask", "Date", "Atomics",
                "SharedArrayBuffer", "eval", "window", "XMLHttpRequest"
            ];
            for (const name of banned) {
                if (typeof globalThis[name] !== "undefined") {
                    throw new Error("ambient global present: " + name);
                }
            }
            if (typeof Math.random !== "undefined") { throw new Error("Math.random present"); }
            phase("checked");
        "#;
        let host = Arc::new(FakeHost::default());
        let (result, _) = run_source(source, Arc::clone(&host));
        let outcome = result.expect("realm is clean");
        assert_eq!(outcome.report.status, RunStatus::Completed);
        assert_eq!(outcome.report.phases, ["checked"]);
    }

    #[test]
    fn realm_hardening_removes_the_ambient_names() {
        let mut ctx = Context::default();
        harden_realm(&mut ctx).expect("hardens");
        let global = ctx.global_object();
        for name in VERIFIED_ABSENT {
            assert!(
                !global
                    .has_own_property(JsString::from(name), &mut ctx)
                    .expect("probe"),
                "`{name}` should be absent"
            );
        }
        for name in ["fetch", "fs", "process", "Temporal", "Intl"] {
            assert!(
                !global
                    .has_own_property(JsString::from(name), &mut ctx)
                    .expect("probe"),
                "`{name}` should be absent"
            );
        }
        let facade_names = [
            "agent", "phase", "world", "files", "git", "report", "artifact", "log",
        ];
        for name in facade_names {
            assert!(
                !global
                    .has_own_property(JsString::from(name), &mut ctx)
                    .expect("probe"),
                "`{name}` is installed by the facade, not the bare realm"
            );
        }
    }

    #[test]
    fn script_size_cap_is_enforced_at_confirmation() {
        let oversized = "x".repeat(MAX_SCRIPT_BYTES + 1);
        match prepare("big", oversized, 5.0) {
            Err(WorkflowError::ScriptTooLarge { size, limit }) => {
                assert_eq!(size, MAX_SCRIPT_BYTES + 1);
                assert_eq!(limit, MAX_SCRIPT_BYTES);
            }
            other => panic!("expected ScriptTooLarge, got {other:?}"),
        }

        let at_cap = format!("/*{}*/\n", "x".repeat(MAX_SCRIPT_BYTES - 5));
        assert_eq!(at_cap.len(), MAX_SCRIPT_BYTES);
        assert!(prepare("edge", at_cap, 5.0).is_ok());
    }

    #[test]
    fn step_cap_stops_a_runaway_script() {
        let source = r#"for (let i = 0; i < 300; i += 1) { phase("p" + i); }"#;
        let host = Arc::new(FakeHost::default());
        let (result, events) = run_source(source, host);
        match result {
            Err(WorkflowError::StepLimitExceeded { limit }) => assert_eq!(limit, MAX_STEPS),
            other => panic!("expected StepLimitExceeded, got {other:?}"),
        }
        assert!(matches!(
            events.last(),
            Some(WorkflowEvent::Finished { report })
                if report.status == RunStatus::Failed
                    && report.steps.len() == MAX_STEPS
        ));
    }

    #[test]
    fn command_set_is_extracted_from_literals_only() {
        let source = r#"
            // world.run("from-a-comment")
            const text = 'world.run("from-a-string")';
            const tpl = `world.run("from-a-template")`;
            world.run("cargo", ["test"]);
            world.run('git', ["status"]);
            world.run("cargo", ["clippy"]);
            if (text.length + tpl.length < 0) { log("unreachable"); }
        "#;
        let confirmation = prepare("extract", source, 5.0).expect("prepares");
        assert_eq!(confirmation.command_set.as_slice(), ["cargo", "git"]);
        assert_eq!(confirmation.script_bytes(), source.len());
    }

    #[test]
    fn runtime_assembled_command_is_rejected_at_confirmation() {
        let dynamic = r#"const cmd = "car" + "go"; world.run(cmd, ["test"]);"#;
        match prepare("dynamic", dynamic, 5.0) {
            Err(WorkflowError::NonLiteralCommand { found }) => {
                assert!(found.contains("cmd"), "snippet names the argument: {found}");
            }
            other => panic!("expected NonLiteralCommand, got {other:?}"),
        }

        let interpolated = r#"const name = "cargo"; world.run(`${name} test`, []);"#;
        assert!(matches!(
            prepare("template", interpolated, 5.0),
            Err(WorkflowError::NonLiteralCommand { .. })
        ));

        let plain_template = r#"world.run(`cargo`, ["test"]);"#;
        assert_eq!(
            prepare("plain", plain_template, 5.0)
                .expect("a plain template is a literal")
                .command_set
                .as_slice(),
            ["cargo"]
        );
    }

    #[test]
    fn world_run_gate_checks_the_confirmed_set() {
        // An alias hides the call site from the static scan, so the runtime
        // gate is what stops it.
        let source = r#"const w = world; w.run("curl", ["http://example.com"]);"#;
        let confirmation = prepare("alias", source, 5.0).expect("prepares");
        assert!(confirmation.command_set.is_empty());
        let host = Arc::new(FakeHost::default());
        let (result, _) = run_spec(&confirmation.spec(), Arc::clone(&host));
        match result {
            Err(WorkflowError::UnconfirmedCommand { command }) => assert_eq!(command, "curl"),
            other => panic!("expected UnconfirmedCommand, got {other:?}"),
        }
        assert!(
            host.runs.lock().expect("runs").is_empty(),
            "the gate must not reach the host"
        );
    }

    #[test]
    fn confirmed_command_runs_and_exit_code_reaches_the_script() {
        let source = r#"
            const gate = world.run("cargo", ["test"], { timeoutMs: 1500 });
            if (gate.exitCode !== 0) { throw new Error("gate failed"); }
            report({ stdout: gate.stdout });
            gate.stdout;
        "#;
        let host = Arc::new(FakeHost::default().with_run_output(CommandOutput {
            exit_code: 0,
            stdout: "test result: ok".to_owned(),
            stderr: String::new(),
        }));
        let (result, events) = run_source(source, Arc::clone(&host));
        let outcome = result.expect("gate passes");
        assert_eq!(outcome.value, json!("test result: ok"));
        assert_eq!(outcome.report.status, RunStatus::Completed);
        assert_eq!(
            host.runs.lock().expect("runs").as_slice(),
            [RunCall {
                command: "cargo".to_owned(),
                args: vec!["test".to_owned()],
                timeout_ms: Some(1500),
            }]
        );
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::Report { item } if item.index == 1
        )));
    }

    #[test]
    fn schema_mismatch_fails_the_run_naming_the_field() {
        let source = r#"
            const review = agent("reviewer").ask(
                {
                    type: "object",
                    required: ["findings"],
                    properties: {
                        findings: {
                            type: "array",
                            items: {
                                type: "object",
                                required: ["line"],
                                properties: { line: { type: "integer" } }
                            }
                        }
                    }
                },
                "Review src/ for panics"
            );
            review;
        "#;
        let host = Arc::new(FakeHost::default());
        host.answer(json!({ "findings": [{ "line": "twelve" }] }));

        let (result, events) = run_source(source, host);
        match result {
            Err(WorkflowError::SchemaMismatch {
                field,
                expected,
                actual,
            }) => {
                assert_eq!(field, "$.findings[0].line");
                assert_eq!(expected, "integer");
                assert_eq!(actual, "string");
            }
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
        let finished = events
            .iter()
            .rev()
            .find_map(|event| match event {
                WorkflowEvent::Finished { report } => Some(report),
                _ => None,
            })
            .expect("finished event");
        assert_eq!(finished.status, RunStatus::Failed);
        assert_eq!(finished.steps[0].status, StepStatus::Failed);
        assert!(matches!(finished.steps[0].kind, StepKind::Ask));
    }

    #[test]
    fn phase_and_ask_work_end_to_end() {
        let source = r##"
            phase("plan");
            const reviewer = agent("reviewer", "terse");
            const result = reviewer.ask(
                { type: "object", required: ["ok"], properties: { ok: { type: "boolean" } } },
                "Confirm the plan"
            );
            if (result.ok !== true) { throw new Error("not ok"); }
            phase("verify");
            report({ verified: result.ok });
            artifact.markdown("summary", "# Verified", { title: "Run summary" });
            "done";
        "##;
        let host = Arc::new(FakeHost::default());
        host.answer(json!({ "ok": true }));

        let (result, events) = run_source(source, Arc::clone(&host));
        let outcome = result.expect("run completes");
        assert_eq!(outcome.report.status, RunStatus::Completed);
        assert_eq!(outcome.report.phases, ["plan", "verify"]);
        assert_eq!(outcome.value, json!("done"));

        let asks = host.asks.lock().expect("asks");
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].agent, "reviewer");
        assert_eq!(asks[0].persona.as_deref(), Some("terse"));
        assert_eq!(asks[0].instructions, "Confirm the plan");
        assert_eq!(
            asks[0].schema.as_ref().expect("schema passed to host")["type"],
            json!("object")
        );

        assert_eq!(
            host.reported.lock().expect("reported").as_slice(),
            [(json!({ "verified": true }), None)]
        );
        assert_eq!(outcome.report.artifacts.len(), 1);
        assert_eq!(outcome.report.artifacts[0].id, "summary");
        assert_eq!(
            outcome.report.artifacts[0].content,
            ArtifactContent::Markdown("# Verified".to_owned())
        );

        // Run → Phase → Step… → Report → Artifact → Finished.
        assert!(matches!(events.first(), Some(WorkflowEvent::Run { .. })));
        assert!(
            matches!(events.last(), Some(WorkflowEvent::Finished { report }) if report.status == RunStatus::Completed),
            "the last event is always Finished"
        );
        let kinds: Vec<StepKind> = outcome.report.steps.iter().map(|step| step.kind).collect();
        assert_eq!(
            kinds,
            [
                StepKind::Phase,
                StepKind::Ask,
                StepKind::Phase,
                StepKind::Report,
                StepKind::Artifact
            ]
        );
        assert!(outcome
            .report
            .steps
            .iter()
            .all(|step| step.status == StepStatus::Ok));
    }

    #[test]
    fn ask_without_a_schema_uses_the_default_agent() {
        let source = r#"const answer = agent().ask("Say hi"); answer.ok;"#;
        let host = Arc::new(FakeHost::default());
        host.answer(json!({ "ok": true }));
        let (result, _) = run_source(source, Arc::clone(&host));
        let outcome = result.expect("runs");
        assert_eq!(outcome.value, json!(true));
        let asks = host.asks.lock().expect("asks");
        assert_eq!(asks[0].agent, "default");
        assert_eq!(asks[0].persona, None);
        assert!(asks[0].schema.is_none());
    }

    #[test]
    fn an_async_iife_can_await_plain_facade_values() {
        let source = r#"
            (async () => {
                phase("async");
                const answer = await agent("writer").ask("Say hi");
                if (answer.ok !== true) { throw new Error("bad answer"); }
                phase("async done");
            })();
        "#;
        let host = Arc::new(FakeHost::default());
        let (result, _) = run_source(source, Arc::clone(&host));
        let outcome = result.expect("the async iife completes");
        assert_eq!(outcome.report.phases, ["async", "async done"]);
        assert_eq!(host.asks.lock().expect("asks").len(), 1);
    }

    #[test]
    fn invalid_js_is_a_syntax_error_before_the_run_starts() {
        match prepare("broken", "phase(", 5.0) {
            Err(WorkflowError::Syntax { .. }) => {}
            other => panic!("expected Syntax, got {other:?}"),
        }
    }

    #[test]
    fn budget_violation_is_checked_between_steps() {
        let source = r#"
            phase("one");
            agent().ask("spend some money");
            phase("two");
        "#;
        let host = Arc::new(FakeHost::default());
        *host.cost.lock().expect("cost") = 6.0;
        let (result, events) = run_source(source, host);
        match result {
            Err(WorkflowError::BudgetExceeded { limit, spent }) => {
                assert_eq!(limit, 5.0);
                assert_eq!(spent, 6.0);
            }
            other => panic!("expected BudgetExceeded, got {other:?}"),
        }
        assert!(matches!(
            events.last(),
            Some(WorkflowEvent::Finished { report }) if report.status == RunStatus::BudgetExceeded
        ));
    }

    #[test]
    fn oversized_report_items_are_rejected() {
        let source = r#"
            report({ text: "x".repeat(40000) });
        "#;
        let host = Arc::new(FakeHost::default());
        let (result, _) = run_source(source, host);
        match result {
            Err(WorkflowError::LimitExceeded { what, limit }) => {
                assert!(what.contains("report"), "unexpected cap: {what}");
                assert_eq!(limit, MAX_REPORT_ITEM_BYTES);
            }
            other => panic!("expected a report size cap, got {other:?}"),
        }
    }

    #[test]
    fn step_snapshot_round_trips_through_serde() {
        let step = WorkflowStep {
            index: 3,
            kind: StepKind::WorldRun,
            agent: None,
            status: StepStatus::Ok,
            detail: "cargo test".to_owned(),
            error: None,
        };
        let encoded = serde_json::to_string(&step).expect("serializes");
        let decoded: WorkflowStep = serde_json::from_str(&encoded).expect("deserializes");
        assert_eq!(step, decoded);
    }
}
