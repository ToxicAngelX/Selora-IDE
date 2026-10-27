//! The JS-to-Rust bridge: the §18.2 facade over [`Host`].
//!
//! The script sees exactly eight globals — `agent`, `phase`, `world`, `files`,
//! `git`, `report`, `artifact`, `log` — installed here. Everything the facade
//! does goes through the blocking [`Host`] trait; the engine itself never
//! touches the filesystem, the network, or a process.
//!
//! Three invariants live in this file:
//!
//! * **`world.run` is gated.** The command must be in the confirmed
//!   [`crate::CommandSet`]; a command that was not a literal at confirmation
//!   time fails loudly ([`WorkflowError::UnconfirmedCommand`]).
//! * **`ask` results are validated, never coerced.** A schema mismatch
//!   ([`WorkflowError::SchemaMismatch`]) names the offending field.
//! * **Every facade call is a step.** Steps are counted against the 256-step
//!   cap, journaled, and emitted as [`WorkflowEvent::Step`].
//!
//! Facade errors abort the run even if the script catches the JS exception: a
//! confirmed workflow cannot paper over a broken contract with `try`/`catch`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{
    js_string, Context, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue,
    NativeFunction,
};

use crate::budget::Budget;
use crate::persisted::{persistence_error, ArtifactRecord, Durable, ReportItemRecord};
use crate::sandbox::Limits;
use crate::schema::Schema;
use crate::subagent::{AgentRef, AgentRegistry};
use crate::{
    Artifact, ArtifactContent, CommandSet, EventSink, Host, ReportItem, RunId, StepKind,
    StepStatus, WorkflowError, WorkflowEvent, WorkflowReport, WorkflowSpec, WorkflowStep,
};

/// The eight globals a script may use, plus the identity slots an
/// `agent(...)` object carries.
const FACADE_GLOBALS: [&str; 8] = [
    "agent", "phase", "world", "files", "git", "report", "artifact", "log",
];

/// Non-writable, non-enumerable, non-configurable: the facade cannot be
/// reassigned or deleted out from under the script.
const FACADE_ATTR: Attribute = Attribute::READONLY
    .union(Attribute::NON_ENUMERABLE)
    .union(Attribute::PERMANENT);

/// Method properties on facade objects may be read and shadowed locally, but
/// not enumerated.
const METHOD_ATTR: Attribute = Attribute::WRITABLE
    .union(Attribute::NON_ENUMERABLE)
    .union(Attribute::CONFIGURABLE);

/// Hidden identity slots on the object returned by `agent()`.
const IDENTITY_ATTR: Attribute = Attribute::READONLY
    .union(Attribute::NON_ENUMERABLE)
    .union(Attribute::PERMANENT);

const AGENT_NAME_KEY: &str = "__wfAgentName";
const AGENT_PERSONA_KEY: &str = "__wfAgentPersona";

type NativeFn = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

/// Opaque handle stored as context host data so the zero-capture native
/// functions can find the run they belong to.
pub(crate) struct FacadeHandle(Rc<Facade>);

/// Run-scoped facade state. Shared by every native function of one run.
pub(crate) struct Facade {
    inner: RefCell<State>,
}

struct State {
    run_id: RunId,
    name: String,
    host: Arc<dyn Host>,
    sink: EventSink,
    limits: Limits,
    command_set: CommandSet,
    budget: Budget,
    agents: AgentRegistry,
    steps: u64,
    steps_log: Vec<WorkflowStep>,
    phases: Vec<String>,
    /// The `phase()` marker in effect; stamped onto every journaled record so
    /// history can group steps, reports and artifacts without guessing.
    current_phase: Option<String>,
    /// The step whose body is running; report/artifact records join it back
    /// to its step identity.
    current_step: Option<u64>,
    artifacts: Vec<Artifact>,
    report_items: u64,
    spent_usd: f64,
    tokens: u64,
    durable: Option<Durable>,
    replaying: bool,
    fatal: Option<WorkflowError>,
}

impl Facade {
    pub(crate) fn new(
        run_id: RunId,
        spec: &WorkflowSpec,
        limits: Limits,
        host: Arc<dyn Host>,
        sink: EventSink,
    ) -> Self {
        Self {
            inner: RefCell::new(State {
                run_id,
                name: spec.name.clone(),
                host,
                sink,
                limits,
                command_set: spec.command_set.clone(),
                budget: Budget::new(spec.max_cost_usd),
                agents: AgentRegistry::default(),
                steps: 0,
                steps_log: Vec::new(),
                phases: Vec::new(),
                current_phase: None,
                current_step: None,
                artifacts: Vec::new(),
                report_items: 0,
                spent_usd: 0.0,
                tokens: 0,
                durable: None,
                replaying: false,
                fatal: None,
            }),
        }
    }

    pub(crate) fn with_durable(self, durable: Durable) -> Self {
        {
            let mut state = self.inner.borrow_mut();
            state.spent_usd = durable.spent_usd;
            state.tokens = durable.tokens;
            state.durable = Some(durable);
        }
        self
    }

    pub(crate) fn persist_report(&self, report: &WorkflowReport) -> Result<(), WorkflowError> {
        let mut state = self.inner.borrow_mut();
        if let Some(durable) = &mut state.durable {
            durable.append(
                "engine_run_finished",
                serde_json::to_value(report).map_err(persistence_error)?,
            )?;
        }
        Ok(())
    }

    pub(crate) fn verify_replayed(&self) -> Result<(), WorkflowError> {
        let state = self.inner.borrow();
        if let Some(durable) = &state.durable {
            durable.verify_replayed(state.steps)?;
        }
        Ok(())
    }

    fn host(&self) -> Arc<dyn Host> {
        self.inner.borrow().host.clone()
    }

    fn emit(&self, event: WorkflowEvent) {
        let sink = self.inner.borrow().sink.clone();
        sink(event);
    }

    fn is_confirmed(&self, command: &str) -> bool {
        self.inner.borrow().command_set.contains(command)
    }

    fn intern_agent(&self, name: &str, persona: Option<&str>) -> AgentRef {
        self.inner.borrow_mut().agents.intern(name, persona)
    }

    fn set_spent(&self, spent_usd: f64) {
        let mut state = self.inner.borrow_mut();
        if state.durable.is_none() {
            state.spent_usd = spent_usd;
            state.tokens = state.host.tokens();
        }
    }

    /// Record the first fatal error of the run.
    pub(crate) fn fail(&self, error: WorkflowError) {
        let mut state = self.inner.borrow_mut();
        if state.fatal.is_none() {
            state.fatal = Some(error);
        }
    }

    /// The run's fatal error, if any.
    pub(crate) fn fatal(&self) -> Option<WorkflowError> {
        self.inner.borrow().fatal.clone()
    }

    pub(crate) fn cancellation(&self) -> Option<WorkflowError> {
        self.host().is_cancelled().then_some(WorkflowError::Stopped)
    }

    /// A budget violation that appeared after the last step boundary.
    pub(crate) fn budget_violation(&self) -> Option<WorkflowError> {
        let state = self.inner.borrow();
        if state.budget.allows(state.spent_usd) {
            None
        } else {
            Some(WorkflowError::BudgetExceeded {
                limit: state.budget.limit(),
                spent: state.spent_usd,
            })
        }
    }

    /// Run `body` as one counted, journaled step.
    pub(crate) fn step<T: serde::Serialize + serde::de::DeserializeOwned>(
        &self,
        kind: StepKind,
        agent: Option<&str>,
        detail: impl Into<String>,
        inputs: serde_json::Value,
        body: impl FnOnce() -> Result<T, WorkflowError>,
    ) -> Result<T, WorkflowError> {
        if self.host().is_cancelled() {
            return Err(WorkflowError::Stopped);
        }
        let index = self.begin_step(kind, agent, &detail.into(), &inputs)?;
        let replay = {
            let state = self.inner.borrow();
            state
                .durable
                .as_ref()
                .map(|durable| {
                    durable.replay(state.steps_log.last().expect("step started"), &inputs)
                })
                .transpose()?
                .flatten()
        };
        let mut agent_state = None;
        let result = (|| {
            if replay.is_some() && kind == StepKind::Ask {
                let checkpoint = self
                    .inner
                    .borrow()
                    .durable
                    .as_ref()
                    .and_then(|durable| durable.agent_state(index))
                    .cloned();
                if let Some(checkpoint) = checkpoint {
                    self.host().restore_agent(
                        agent.ok_or_else(|| {
                            persistence_error("ask checkpoint has no agent identity")
                        })?,
                        &checkpoint,
                    )?;
                }
            }
            let value = if let Some(value) = &replay {
                if matches!(
                    kind,
                    StepKind::Phase | StepKind::Report | StepKind::Artifact | StepKind::Log
                ) {
                    self.inner.borrow_mut().replaying = true;
                    let result = body();
                    self.inner.borrow_mut().replaying = false;
                    result
                } else {
                    serde_json::from_value(value.clone()).map_err(persistence_error)
                }
            } else {
                body()
            }?;
            if replay.is_none() && kind == StepKind::Ask && self.inner.borrow().durable.is_some() {
                agent_state =
                    self.host().snapshot_agent(agent.ok_or_else(|| {
                        persistence_error("ask checkpoint has no agent identity")
                    })?)?;
            }
            Ok(value)
        })();
        let result = result.map_err(|error| self.cancellation().unwrap_or(error));
        match result {
            Ok(value) => {
                self.finish_step(
                    index,
                    StepStatus::Ok,
                    None,
                    Some(serde_json::to_value(&value).map_err(persistence_error)?),
                    replay.is_some(),
                    agent_state,
                )?;
                Ok(value)
            }
            Err(error) => {
                self.finish_step(
                    index,
                    StepStatus::Failed,
                    Some(error.to_string()),
                    None,
                    replay.is_some(),
                    None,
                )?;
                Err(error)
            }
        }
    }

    fn begin_step(
        &self,
        kind: StepKind,
        agent: Option<&str>,
        detail: &str,
        inputs: &serde_json::Value,
    ) -> Result<u64, WorkflowError> {
        let (index, event) = {
            let mut state = self.inner.borrow_mut();
            if let Some(error) = &state.fatal {
                return Err(error.clone());
            }
            if state.steps as usize >= state.limits.max_steps {
                let error = WorkflowError::StepLimitExceeded {
                    limit: state.limits.max_steps,
                };
                state.fatal = Some(error.clone());
                return Err(error);
            }
            let step = WorkflowStep {
                index: state.steps + 1,
                kind,
                agent: agent.map(str::to_owned),
                status: StepStatus::Running,
                detail: detail.to_owned(),
                error: None,
            };
            let replay = state
                .durable
                .as_ref()
                .map(|durable| durable.replay(&step, inputs))
                .transpose()?
                .flatten()
                .is_some();
            if !replay && !state.budget.allows(state.spent_usd) {
                let error = WorkflowError::BudgetExceeded {
                    limit: state.budget.limit(),
                    spent: state.spent_usd,
                };
                state.fatal = Some(error.clone());
                return Err(error);
            }
            state.steps += 1;
            let phase = state.current_phase.clone();
            if !replay {
                if let Some(durable) = &mut state.durable {
                    durable.start(&step, inputs.clone(), phase)?;
                }
            }
            state.current_step = Some(step.index);
            state.steps_log.push(step.clone());
            (step.index, WorkflowEvent::Step { step })
        };
        self.emit(event);
        Ok(index)
    }

    fn finish_step(
        &self,
        index: u64,
        status: StepStatus,
        error: Option<String>,
        value: Option<serde_json::Value>,
        replay: bool,
        agent_state: Option<serde_json::Value>,
    ) -> Result<(), WorkflowError> {
        let event = {
            let mut state = self.inner.borrow_mut();
            let Some(step) = state.steps_log.iter_mut().find(|step| step.index == index) else {
                return Ok(());
            };
            step.status = status;
            step.error = error;
            let snapshot = step.clone();
            let phase = state.current_phase.clone();
            state.current_step = None;
            if !replay {
                let host = state.host.clone();
                if let Some(durable) = &mut state.durable {
                    durable.finish(&snapshot, value, agent_state, phase, host.as_ref())?;
                    let spent = durable.spent_usd;
                    let tokens = durable.tokens;
                    state.spent_usd = spent;
                    state.tokens = tokens;
                }
            }
            WorkflowEvent::Step { step: snapshot }
        };
        self.emit(event);
        Ok(())
    }

    fn record_phase(&self, name: &str) -> Result<(), WorkflowError> {
        {
            let mut state = self.inner.borrow_mut();
            if !state.replaying {
                if let Some(durable) = &mut state.durable {
                    durable.append("phase_started", serde_json::json!({"phase": name}))?;
                }
            }
            if !state.phases.iter().any(|phase| phase == name) {
                state.phases.push(name.to_owned());
            }
            state.current_phase = Some(name.to_owned());
        }
        self.emit(WorkflowEvent::Phase {
            name: name.to_owned(),
        });
        Ok(())
    }

    fn record_report(
        &self,
        item: serde_json::Value,
        artifact_id: Option<String>,
    ) -> Result<ReportItem, WorkflowError> {
        let bytes = serde_json::to_vec(&item).map_or(usize::MAX, |encoded| encoded.len());
        let recorded = {
            let mut state = self.inner.borrow_mut();
            if state.report_items as usize >= state.limits.max_report_items {
                return Err(WorkflowError::LimitExceeded {
                    what: "report item count".to_owned(),
                    limit: state.limits.max_report_items,
                });
            }
            if bytes > state.limits.max_report_item_bytes {
                return Err(WorkflowError::LimitExceeded {
                    what: format!("report item size ({bytes} bytes)"),
                    limit: state.limits.max_report_item_bytes,
                });
            }
            state.report_items += 1;
            ReportItem {
                index: state.report_items,
                item,
                artifact_id,
            }
        };
        let replaying = self.inner.borrow().replaying;
        if !replaying {
            self.host()
                .report(&recorded.item, recorded.artifact_id.as_deref())?;
        }
        if !replaying {
            let mut state = self.inner.borrow_mut();
            if let Some(step_id) = state.current_step {
                let record = ReportItemRecord {
                    step_id,
                    phase: state.current_phase.clone(),
                    index: recorded.index,
                    item: recorded.item.clone(),
                    artifact_id: recorded.artifact_id.clone(),
                };
                if let Some(durable) = &mut state.durable {
                    durable.report_item(&record)?;
                }
            }
        }
        self.emit(WorkflowEvent::Report {
            item: recorded.clone(),
        });
        Ok(recorded)
    }

    fn record_artifact(&self, artifact: Artifact) -> Result<(), WorkflowError> {
        let size = match &artifact.content {
            ArtifactContent::Markdown(content) => content.len(),
            ArtifactContent::File(path) => path.len(),
        };
        {
            let state = self.inner.borrow();
            if state
                .artifacts
                .iter()
                .any(|existing| existing.id == artifact.id)
            {
                return Err(WorkflowError::Script {
                    message: format!("artifact id `{}` was already used", artifact.id),
                });
            }
            if size > state.limits.max_artifact_bytes {
                return Err(WorkflowError::LimitExceeded {
                    what: format!("artifact size ({size} bytes)"),
                    limit: state.limits.max_artifact_bytes,
                });
            }
        }
        let host = self.host();
        let replaying = self.inner.borrow().replaying;
        if !replaying {
            match &artifact.content {
                ArtifactContent::Markdown(content) => {
                    host.artifact_markdown(&artifact.id, artifact.title.as_deref(), content)?;
                }
                ArtifactContent::File(path) => {
                    host.artifact_file(&artifact.id, artifact.title.as_deref(), path)?;
                }
            }
        }
        if !replaying {
            let mut state = self.inner.borrow_mut();
            if let Some(step_id) = state.current_step {
                let record = ArtifactRecord {
                    step_id,
                    phase: state.current_phase.clone(),
                    artifact: artifact.clone(),
                };
                if let Some(durable) = &mut state.durable {
                    durable.artifact(&record)?;
                }
            }
        }
        self.inner.borrow_mut().artifacts.push(artifact.clone());
        self.emit(WorkflowEvent::Artifact { artifact });
        Ok(())
    }

    /// The terminal report snapshot.
    pub(crate) fn report(
        &self,
        status: crate::RunStatus,
        error: Option<String>,
        elapsed_ms: u64,
    ) -> WorkflowReport {
        let state = self.inner.borrow();
        WorkflowReport {
            run_id: state.run_id.clone(),
            name: state.name.clone(),
            status,
            elapsed_ms,
            phases: state.phases.clone(),
            steps: state.steps_log.clone(),
            artifacts: state.artifacts.clone(),
            cost_usd: state.spent_usd,
            tokens: state.tokens,
            error,
        }
    }
}

/// Install the facade into `ctx` as the realm's only extra globals.
///
/// # Errors
/// A JS failure while defining a global (should not happen on a fresh realm).
pub(crate) fn install(ctx: &mut Context, facade: Rc<Facade>) -> JsResult<()> {
    ctx.insert_data(FacadeHandle(facade));

    let agent_fn = native(ctx, js_agent, "agent", 2);
    let phase_fn = native(ctx, js_phase, "phase", 1);
    let report_fn = native(ctx, js_report, "report", 2);
    let log_fn = native(ctx, js_log, "log", 1);

    let world_run = native(ctx, js_world_run, "run", 3);
    let files_glob = native(ctx, js_files_glob, "glob", 1);
    let files_grep = native(ctx, js_files_grep, "grep", 2);
    let files_read = native(ctx, js_files_read, "read", 1);
    let git_changed_files = native(ctx, js_git_changed_files, "changedFiles", 1);
    let git_diff = native(ctx, js_git_diff, "diff", 2);
    let git_status = native(ctx, js_git_status, "status", 0);
    let git_log = native(ctx, js_git_log, "log", 1);
    let artifact_markdown = native(ctx, js_artifact_markdown, "markdown", 3);
    let artifact_file = native(ctx, js_artifact_file, "file", 3);

    let world = namespace(ctx, &[("run", world_run)]);
    let files = namespace(
        ctx,
        &[
            ("glob", files_glob),
            ("grep", files_grep),
            ("read", files_read),
        ],
    );
    let git = namespace(
        ctx,
        &[
            ("changedFiles", git_changed_files),
            ("diff", git_diff),
            ("status", git_status),
            ("log", git_log),
        ],
    );
    let artifact = namespace(
        ctx,
        &[("markdown", artifact_markdown), ("file", artifact_file)],
    );

    let global = ctx.global_object();
    for (name, value) in [
        ("agent", agent_fn),
        ("phase", phase_fn),
        ("world", JsValue::from(world)),
        ("files", JsValue::from(files)),
        ("git", JsValue::from(git)),
        ("report", report_fn),
        ("artifact", JsValue::from(artifact)),
        ("log", log_fn),
    ] {
        define_global(&global, name, value, ctx)?;
    }
    debug_assert_eq!(FACADE_GLOBALS.len(), 8);
    Ok(())
}

fn define_global(global: &JsObject, name: &str, value: JsValue, ctx: &mut Context) -> JsResult<()> {
    global
        .define_property_or_throw(
            JsString::from(name),
            PropertyDescriptor::builder()
                .value(value)
                .writable(FACADE_ATTR.writable())
                .enumerable(FACADE_ATTR.enumerable())
                .configurable(FACADE_ATTR.configurable()),
            ctx,
        )
        .map(|_| ())
}

fn native(ctx: &mut Context, function: NativeFn, name: &str, length: usize) -> JsValue {
    FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(function))
        .name(JsString::from(name))
        .length(length)
        .build()
        .into()
}

fn namespace(ctx: &mut Context, entries: &[(&str, JsValue)]) -> JsObject {
    let mut initializer = ObjectInitializer::new(ctx);
    for (name, value) in entries {
        initializer.property(JsString::from(*name), value.clone(), METHOD_ATTR);
    }
    initializer.build()
}

fn facade_of(ctx: &Context) -> JsResult<Rc<Facade>> {
    ctx.get_data::<FacadeHandle>()
        .map(|handle| Rc::clone(&handle.0))
        .ok_or_else(|| js_error("workflow facade is not installed in this realm".to_owned()))
}

/// Run a facade body, turning a [`WorkflowError`] into a JS exception and
/// recording it as the run's fatal error.
fn run_js<T>(facade: &Facade, body: impl FnOnce() -> Result<T, WorkflowError>) -> JsResult<T> {
    let result = if facade.host().is_cancelled() {
        Err(WorkflowError::Stopped)
    } else {
        body()
    };
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let message = error.to_string();
            facade.fail(error);
            Err(js_error(message))
        }
    }
}

fn js_error(message: String) -> JsError {
    JsError::from(JsNativeError::typ().with_message(message))
}

fn script_error(error: JsError) -> WorkflowError {
    WorkflowError::Script {
        message: error.to_string(),
    }
}

fn require_string(args: &[JsValue], index: usize, what: &str) -> Result<String, WorkflowError> {
    match args.get(index) {
        Some(value) if !value.is_undefined() => value
            .as_string()
            .map(|text| text.to_std_string_escaped())
            .ok_or_else(|| WorkflowError::Script {
                message: format!("{what} must be a string"),
            }),
        _ => Err(WorkflowError::Script {
            message: format!("{what} is required"),
        }),
    }
}

fn optional_string(
    args: &[JsValue],
    index: usize,
    what: &str,
) -> Result<Option<String>, WorkflowError> {
    match args.get(index) {
        Some(value) if !value.is_undefined() && !value.is_null() => value
            .as_string()
            .map(|text| Some(text.to_std_string_escaped()))
            .ok_or_else(|| WorkflowError::Script {
                message: format!("{what} must be a string"),
            }),
        _ => Ok(None),
    }
}

fn optional_number(
    args: &[JsValue],
    index: usize,
    what: &str,
) -> Result<Option<f64>, WorkflowError> {
    match args.get(index) {
        Some(value) if !value.is_undefined() && !value.is_null() => value
            .as_number()
            .map(Some)
            .ok_or_else(|| WorkflowError::Script {
                message: format!("{what} must be a number"),
            }),
        _ => Ok(None),
    }
}

fn string_array(
    value: &JsValue,
    what: &str,
    ctx: &mut Context,
) -> Result<Vec<String>, WorkflowError> {
    let Some(object) = value.as_object().filter(JsObject::is_array) else {
        return Err(WorkflowError::Script {
            message: format!("{what} must be an array of strings"),
        });
    };
    let length = array_length(&object, ctx)?;
    let mut out = Vec::with_capacity(length);
    for index in 0..length {
        let element = object
            .get(JsString::from(index.to_string()), ctx)
            .map_err(script_error)?;
        let text = element.as_string().ok_or_else(|| WorkflowError::Script {
            message: format!("{what}[{index}] must be a string"),
        })?;
        out.push(text.to_std_string_escaped());
    }
    Ok(out)
}

fn array_length(object: &JsObject, ctx: &mut Context) -> Result<usize, WorkflowError> {
    let value = object
        .get(js_string!("length"), ctx)
        .map_err(script_error)?;
    let length = value.as_number().ok_or_else(|| WorkflowError::Script {
        message: "array has no numeric length".to_owned(),
    })?;
    Ok(if length.is_finite() && length > 0.0 {
        length as usize
    } else {
        0
    })
}

fn to_js<T: serde::Serialize>(value: &T, ctx: &mut Context) -> Result<JsValue, WorkflowError> {
    let json = serde_json::to_value(value).map_err(|error| WorkflowError::Script {
        message: format!("cannot serialize a host result: {error}"),
    })?;
    JsValue::from_json(&json, ctx).map_err(script_error)
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out.replace(['\n', '\r'], " ")
}

/* ------------------------------- globals -------------------------------- */

fn js_agent(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let name = optional_string(args, 0, "agent name")?
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "default".to_owned());
        if name.chars().count() > 128 {
            return Err(WorkflowError::Script {
                message: "agent name must be at most 128 characters".to_owned(),
            });
        }
        let persona = optional_string(args, 1, "agent persona")?;
        let agent = facade.intern_agent(&name, persona.as_deref());

        let ask = native(ctx, js_ask, "ask", 2);
        let persona_value = match &agent.persona {
            Some(persona) => JsValue::from(JsString::from(persona.as_str())),
            None => JsValue::undefined(),
        };
        let mut initializer = ObjectInitializer::new(ctx);
        initializer
            .property(js_string!("ask"), ask, METHOD_ATTR)
            .property(
                JsString::from(AGENT_NAME_KEY),
                JsValue::from(JsString::from(agent.name.as_str())),
                IDENTITY_ATTR,
            )
            .property(
                JsString::from(AGENT_PERSONA_KEY),
                persona_value,
                IDENTITY_ATTR,
            );
        Ok(JsValue::from(initializer.build()))
    })
}

fn js_ask(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let receiver = this.as_object().ok_or_else(|| WorkflowError::Script {
            message: "`ask` must be called as `agent(...).ask(...)`".to_owned(),
        })?;
        let name =
            identity_of(&receiver, AGENT_NAME_KEY, ctx)?.ok_or_else(|| WorkflowError::Script {
                message: "`ask` must be called as `agent(...).ask(...)`".to_owned(),
            })?;
        let persona = identity_of(&receiver, AGENT_PERSONA_KEY, ctx)?;

        let (schema_json, instructions) = parse_ask_args(args, ctx)?;
        let schema = match &schema_json {
            Some(value) => Some(
                Schema::parse(value).map_err(|message| WorkflowError::InvalidSchema { message })?,
            ),
            None => None,
        };

        let label = truncate(&instructions, 80);
        let answer = facade.step(StepKind::Ask, Some(&name), label,
            serde_json::json!({"agent": name, "persona": persona, "schema": schema_json, "instructions": instructions}), || {
            let host = facade.host();
            let answer = host.ask_with_schema(
                &name,
                persona.as_deref(),
                &instructions,
                schema_json.as_ref(),
            );
            facade.set_spent(host.cost_usd());
            let answer = answer.map_err(|error| if host.is_cancelled() { WorkflowError::Stopped } else { error.into() })?;
            if let Some(schema) = &schema {
                schema
                    .validate(&answer)
                    .map_err(|error| WorkflowError::SchemaMismatch {
                        field: error.field,
                        expected: error.expected,
                        actual: error.actual,
                    })?;
            }
            Ok(answer)
        })?;
        if let Some(schema) = &schema {
            schema
                .validate(&answer)
                .map_err(|error| WorkflowError::SchemaMismatch {
                    field: error.field,
                    expected: error.expected,
                    actual: error.actual,
                })?;
        }
        JsValue::from_json(&answer, ctx).map_err(script_error)
    })
}

fn parse_ask_args(
    args: &[JsValue],
    ctx: &mut Context,
) -> Result<(Option<serde_json::Value>, String), WorkflowError> {
    match args {
        [] => Err(WorkflowError::Script {
            message: "`ask` needs instructions".to_owned(),
        }),
        [instructions] => {
            let _ = instructions;
            Ok((None, require_string(args, 0, "ask instructions")?))
        }
        [schema, instructions, ..] => {
            let schema_json = if schema.is_undefined() || schema.is_null() {
                None
            } else {
                Some(schema.to_json(ctx).map_err(script_error)?.ok_or_else(|| {
                    WorkflowError::InvalidSchema {
                        message: "the `type` argument must be an object literal".to_owned(),
                    }
                })?)
            };
            let _ = instructions;
            Ok((schema_json, require_string(args, 1, "ask instructions")?))
        }
    }
}

fn identity_of(
    object: &JsObject,
    key: &str,
    ctx: &mut Context,
) -> Result<Option<String>, WorkflowError> {
    let value = object.get(JsString::from(key), ctx).map_err(script_error)?;
    Ok(value.as_string().map(|text| text.to_std_string_escaped()))
}

fn js_phase(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let name = require_string(args, 0, "phase name")?;
        facade.step(
            StepKind::Phase,
            None,
            name.clone(),
            serde_json::json!([name]),
            || {
                facade.record_phase(&name)?;
                Ok(())
            },
        )?;
        Ok(JsValue::undefined())
    })
}

fn js_world_run(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let command = require_string(args, 0, "world.run command")?;
        let command_args = match args.get(1) {
            Some(value) if !value.is_undefined() && !value.is_null() => {
                string_array(value, "world.run args", ctx)?
            }
            _ => Vec::new(),
        };
        let timeout_ms = world_run_timeout(args.get(2), ctx)?;
        if !facade.is_confirmed(&command) {
            return Err(WorkflowError::UnconfirmedCommand { command });
        }
        let label = if command_args.is_empty() {
            command.clone()
        } else {
            format!("{command} {}", command_args.join(" "))
        };
        let output = facade.step(
            StepKind::WorldRun,
            None,
            label,
            serde_json::json!([command, command_args, timeout_ms]),
            || {
                let output = facade
                    .host()
                    .world_run(&command, &command_args, timeout_ms)?;
                Ok(output)
            },
        )?;
        to_js(&output, ctx)
    })
}

fn world_run_timeout(
    value: Option<&JsValue>,
    ctx: &mut Context,
) -> Result<Option<u64>, WorkflowError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let Some(options) = value.as_object() else {
        return Err(WorkflowError::Script {
            message: "world.run options must be an object like { timeoutMs: 30000 }".to_owned(),
        });
    };
    let timeout = options
        .get(js_string!("timeoutMs"), ctx)
        .map_err(script_error)?;
    if timeout.is_undefined() || timeout.is_null() {
        return Ok(None);
    }
    let millis = timeout.as_number().ok_or_else(|| WorkflowError::Script {
        message: "timeoutMs must be a number".to_owned(),
    })?;
    if !millis.is_finite() || millis < 0.0 {
        return Err(WorkflowError::Script {
            message: "timeoutMs must be a finite, non-negative number".to_owned(),
        });
    }
    Ok(Some(millis as u64))
}

fn js_files_glob(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let pattern = require_string(args, 0, "files.glob pattern")?;
        let paths = facade.step(
            StepKind::Files,
            None,
            format!("glob {pattern}"),
            serde_json::json!(["glob", pattern]),
            || {
                let paths = facade.host().files_glob(&pattern)?;
                Ok(paths)
            },
        )?;
        to_js(&paths, ctx)
    })
}

fn js_files_grep(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let pattern = require_string(args, 0, "files.grep pattern")?;
        let glob = optional_string(args, 1, "files.grep glob")?;
        let label = match &glob {
            Some(glob) => format!("grep {pattern} in {glob}"),
            None => format!("grep {pattern}"),
        };
        let hits = facade.step(
            StepKind::Files,
            None,
            label,
            serde_json::json!(["grep", pattern, glob]),
            || {
                let hits = facade.host().files_grep(&pattern, glob.as_deref())?;
                Ok(hits)
            },
        )?;
        to_js(&hits, ctx)
    })
}

fn js_files_read(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let path = require_string(args, 0, "files.read path")?;
        let content = facade.step(
            StepKind::Files,
            None,
            format!("read {path}"),
            serde_json::json!(["read", path]),
            || {
                let content = facade.host().files_read(&path)?;
                Ok(content)
            },
        )?;
        to_js(&content, ctx)
    })
}

fn js_git_changed_files(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let base = optional_string(args, 0, "git.changedFiles base")?;
        let paths = facade.step(
            StepKind::Git,
            None,
            "changedFiles",
            serde_json::json!(["changedFiles", base]),
            || {
                let paths = facade.host().git_changed_files(base.as_deref())?;
                Ok(paths)
            },
        )?;
        to_js(&paths, ctx)
    })
}

fn js_git_diff(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let base = optional_string(args, 0, "git.diff base")?;
        let path = optional_string(args, 1, "git.diff path")?;
        let diff = facade.step(
            StepKind::Git,
            None,
            "diff",
            serde_json::json!(["diff", base, path]),
            || {
                let diff = facade.host().git_diff(base.as_deref(), path.as_deref())?;
                Ok(diff)
            },
        )?;
        to_js(&diff, ctx)
    })
}

fn js_git_status(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let _ = args;
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let status = facade.step(
            StepKind::Git,
            None,
            "status",
            serde_json::json!(["status"]),
            || {
                let status = facade.host().git_status()?;
                Ok(status)
            },
        )?;
        to_js(&status, ctx)
    })
}

fn js_git_log(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let count = optional_number(args, 0, "git.log count")?;
        if let Some(count) = count {
            if !count.is_finite() || count < 0.0 {
                return Err(WorkflowError::Script {
                    message: "git.log count must be a finite, non-negative number".to_owned(),
                });
            }
        }
        let count = count.map(|count| count as usize);
        let commits = facade.step(
            StepKind::Git,
            None,
            "log",
            serde_json::json!(["log", count]),
            || {
                let commits = facade.host().git_log(count)?;
                Ok(commits)
            },
        )?;
        to_js(&commits, ctx)
    })
}

fn js_report(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let Some(value) = args.first() else {
            return Err(WorkflowError::Script {
                message: "report needs an item".to_owned(),
            });
        };
        let item =
            value
                .to_json(ctx)
                .map_err(script_error)?
                .ok_or_else(|| WorkflowError::Script {
                    message: "report item must be a JSON value (not undefined)".to_owned(),
                })?;
        let artifact_id = optional_string(args, 1, "report artifactId")?;
        facade.step(
            StepKind::Report,
            None,
            "report",
            serde_json::json!([item, artifact_id]),
            || {
                facade.record_report(item, artifact_id)?;
                Ok(())
            },
        )?;
        Ok(JsValue::undefined())
    })
}

fn js_artifact_markdown(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let id = require_string(args, 0, "artifact.markdown id")?;
        let content = require_string(args, 1, "artifact.markdown content")?;
        let title = artifact_title(args.get(2), ctx)?;
        facade.step(
            StepKind::Artifact,
            None,
            format!("markdown {id}"),
            serde_json::json!(["markdown", id, title, content]),
            || {
                facade.record_artifact(Artifact {
                    id,
                    title,
                    content: ArtifactContent::Markdown(content),
                })
            },
        )?;
        Ok(JsValue::undefined())
    })
}

fn js_artifact_file(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let id = require_string(args, 0, "artifact.file id")?;
        let path = require_string(args, 1, "artifact.file path")?;
        let title = artifact_title(args.get(2), ctx)?;
        facade.step(
            StepKind::Artifact,
            None,
            format!("file {id}"),
            serde_json::json!(["file", id, title, path]),
            || {
                facade.record_artifact(Artifact {
                    id,
                    title,
                    content: ArtifactContent::File(path),
                })
            },
        )?;
        Ok(JsValue::undefined())
    })
}

fn artifact_title(
    value: Option<&JsValue>,
    ctx: &mut Context,
) -> Result<Option<String>, WorkflowError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let Some(options) = value.as_object() else {
        return Err(WorkflowError::Script {
            message: "artifact options must be an object like { title: \"…\" }".to_owned(),
        });
    };
    let title = options
        .get(js_string!("title"), ctx)
        .map_err(script_error)?;
    match title.as_string() {
        Some(text) => Ok(Some(text.to_std_string_escaped())),
        None => Ok(None),
    }
}

fn js_log(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let facade = facade_of(ctx)?;
    run_js(&facade, || {
        let message = require_string(args, 0, "log message")?;
        facade.step(
            StepKind::Log,
            None,
            truncate(&message, 120),
            serde_json::json!([message]),
            || {
                if !facade.inner.borrow().replaying {
                    facade.host().log(&message);
                }
                Ok(())
            },
        )?;
        facade.emit(WorkflowEvent::Log {
            message: message.clone(),
        });
        Ok(JsValue::undefined())
    })
}
