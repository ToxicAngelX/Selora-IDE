use selora_protocol::{
    clamp_region_size, ApprovalId, BootstrapState, CoreInstanceId, DecisionDto, EventEnvelope,
    FileTreeDto, FrontendCommand, FrontendEvent, LayoutConstraintsDto, LayoutDto, ProtocolErrorDto,
    RegionId, RequestEnvelope, RequestId, SessionId, SettingsPatchDto, SettingsViewDto, SurfaceId,
    ToastDto, ToastSeverity, WorkflowApprovalNeededDto, WorkflowArtifactDto,
    WorkflowArtifactEventDto, WorkflowFinishedDto, WorkflowPhaseDto, WorkflowPhaseEventDto,
    WorkflowReportDto, WorkflowReportEventDto, WorkflowSpecDto, WorkflowStartedDto, WorkflowStatus,
    WorkflowStepDto, WorkflowStepEventDto, WorkspaceId, PROTOCOL_VERSION,
};
use selora_settings::LayoutSettings;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::Emitter;
use url::Url;
use uuid::Uuid;

use crate::lsp_bridge::LspBridge;
use crate::sessions::{ApiKeySource, EngineConfig, SessionEngine};
use crate::workspace::{Workspace, WorkspaceError};

/// The `reveal_failed` wire error for a file-manager launch that did not start.
fn reveal_failed(cause: std::io::Error) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "reveal_failed".into(),
        message: format!("Could not open the file manager: {cause}"),
        retryable: true,
    }
}

/// Select `path` in the platform file manager. Windows uses explorer.exe
/// `/select,`; macOS uses Finder's `open -R`; other Unix opens the parent
/// folder.
fn reveal_in_file_explorer(path: &std::path::Path) -> Result<(), ProtocolErrorDto> {
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("explorer.exe");
        if path.is_dir() {
            command.arg(path);
        } else {
            command.arg(format!("/select,{}", path.display()));
        }
        command.spawn().map(|_| ()).map_err(reveal_failed)?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(reveal_failed)?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let target = path.parent().filter(|parent| !parent.as_os_str().is_empty());
        std::process::Command::new("xdg-open")
            .arg(target.unwrap_or(path))
            .spawn()
            .map(|_| ())
            .map_err(reveal_failed)?;
    }
    Ok(())
}

/// The stable id for a workspace root: `ws_` + FNV-1a of the canonical path,
/// so a restart re-attaches to the same identity (and the frontend's envelope
/// context keeps matching).
fn workspace_id_for(root: &std::path::Path) -> WorkspaceId {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    WorkspaceId::new(format!("ws_{hash:016x}")).expect("hex id is valid")
}

/// The history a durable journal can prove: its terminal run report plus the
/// report items its step inputs reconstruct.
///
/// Every field stays empty when the terminal payload is absent or malformed:
/// the bootstrap must never fabricate progress the journal did not record.
#[derive(Default)]
struct WorkflowRunHistory {
    phases: Vec<String>,
    steps: Vec<selora_protocol::WorkflowStepDto>,
    reports: Vec<selora_protocol::WorkflowReportDto>,
    artifacts: Vec<selora_protocol::WorkflowArtifactDto>,
    tokens: u64,
    cost_usd: f64,
    error: Option<String>,
}

/// Rebuild a completed (or interrupted) run's history from its journal.
///
/// The terminal `engine_run_finished` record carries the engine's own
/// [`selora_workflow::WorkflowReport`] (phases, steps, artifacts, tokens, cost,
/// error). Report items are not in that report, so they come from the
/// `engine_step_started` inputs of successful `report` steps; a step whose
/// inputs are missing is skipped rather than invented.
fn workflow_run_history(journal: &selora_workflow::journal::LoadedJournal) -> WorkflowRunHistory {
    let Some(report) = journal
        .entries
        .iter()
        .rev()
        .find(|entry| entry.kind == "engine_run_finished")
        .and_then(|entry| {
            serde_json::from_value::<selora_workflow::WorkflowReport>(entry.payload.clone()).ok()
        })
    else {
        return WorkflowRunHistory::default();
    };

    let mut start_inputs = std::collections::BTreeMap::new();
    for entry in &journal.entries {
        if entry.kind != "engine_step_started" {
            continue;
        }
        let index = entry
            .payload
            .get("step")
            .and_then(|step| step.get("index"))
            .and_then(serde_json::Value::as_u64);
        if let (Some(index), Some(inputs)) = (index, entry.payload.get("inputs")) {
            start_inputs.insert(index, inputs.clone());
        }
    }

    let mut reports = Vec::new();
    let mut report_ordinal = 0u64;
    for step in &report.steps {
        if step.kind != selora_workflow::StepKind::Report
            || step.status != selora_workflow::StepStatus::Ok
        {
            continue;
        }
        // The live stream ids report items by their 1-based report ordinal;
        // advance it even when this item cannot be reconstructed so later ids
        // still match what the run emitted.
        report_ordinal += 1;
        let Some(items) = start_inputs
            .get(&step.index)
            .and_then(|inputs| inputs.as_array())
        else {
            continue;
        };
        let Some(item) = items.first() else {
            continue;
        };
        reports.push(selora_protocol::WorkflowReportDto {
            id: format!("report-{report_ordinal}"),
            item: workflow_report_item_text(item),
            artifact_id: items
                .get(1)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        });
    }

    let artifacts = report
        .artifacts
        .into_iter()
        .map(|artifact| {
            let (kind, body) = match artifact.content {
                selora_workflow::ArtifactContent::Markdown(body) => ("markdown".to_owned(), body),
                selora_workflow::ArtifactContent::File(path) => ("file".to_owned(), path),
            };
            selora_protocol::WorkflowArtifactDto {
                id: artifact.id,
                title: artifact.title.unwrap_or_default(),
                kind,
                body,
            }
        })
        .collect();

    WorkflowRunHistory {
        phases: report.phases,
        steps: report
            .steps
            .into_iter()
            .map(|step| workflow_step_dto(step, String::new()))
            .collect(),
        reports,
        artifacts,
        tokens: report.tokens,
        cost_usd: report.cost_usd,
        error: report.error,
    }
}

/// Read a journal's terminal status, preferring the raw
/// `engine_run_finished` record over the typed resume plan.
///
/// The raw record is authoritative when the engine's report payload cannot be
/// fully parsed (for example, only `{"status":"budget_exceeded"}` survived):
/// the shell must still know a cap fired before it offers or accepts a resume.
fn workflow_terminal_status(
    journal: &selora_workflow::journal::LoadedJournal,
) -> Option<selora_workflow::journal::RunStatus> {
    journal
        .entries
        .iter()
        .rev()
        .find_map(|entry| {
            if entry.kind != "engine_run_finished" {
                return None;
            }
            match entry
                .payload
                .get("status")
                .and_then(serde_json::Value::as_str)
            {
                Some(status) if status.eq_ignore_ascii_case("completed") => {
                    Some(selora_workflow::journal::RunStatus::Completed)
                }
                Some(status) if status.eq_ignore_ascii_case("failed") => {
                    Some(selora_workflow::journal::RunStatus::Failed)
                }
                Some(status) if status.eq_ignore_ascii_case("stopped") => {
                    Some(selora_workflow::journal::RunStatus::Stopped)
                }
                Some(status)
                    if status.eq_ignore_ascii_case("budget_exceeded")
                        || status.eq_ignore_ascii_case("BudgetExceeded") =>
                {
                    Some(selora_workflow::journal::RunStatus::BudgetExceeded)
                }
                _ => None,
            }
        })
        .or_else(|| journal.resume_plan().run_status())
}

/// Discover durable runs for the bootstrap. Corrupt or foreign directories are
/// omitted: the panel must never offer to resume something the engine cannot
/// prove it owns.
fn persisted_workflow_runs(
    root: &std::path::Path,
) -> Vec<selora_protocol::WorkflowPersistedRunDto> {
    let dir = root.join(selora_workflow::journal::JOURNAL_DIR);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let id = selora_workflow::RunId::from(name);
        let Ok(spec) = selora_workflow::load_workflow_spec(root, &id) else {
            continue;
        };
        let Ok(journal) = selora_workflow::journal::Journal::load(root, id.clone()) else {
            continue;
        };
        let status = workflow_terminal_status(&journal);
        let ambiguous_command = workflow_has_ambiguous_command(&journal);
        let budget_tripped = status == Some(selora_workflow::journal::RunStatus::BudgetExceeded);
        let completed = status == Some(selora_workflow::journal::RunStatus::Completed);
        let history = workflow_run_history(&journal);
        let (status, detail) = match status {
            Some(selora_workflow::journal::RunStatus::Completed) => {
                (WorkflowStatus::Completed, None)
            }
            Some(selora_workflow::journal::RunStatus::Failed) => (
                WorkflowStatus::Failed,
                history
                    .error
                    .clone()
                    .or_else(|| Some("Interrupted after a failed step.".into())),
            ),
            Some(selora_workflow::journal::RunStatus::Stopped) => (
                WorkflowStatus::Stopped,
                history
                    .error
                    .clone()
                    .or_else(|| Some("Stopped before completion.".into())),
            ),
            Some(selora_workflow::journal::RunStatus::BudgetExceeded) => (
                WorkflowStatus::Failed,
                history.error.clone().or_else(|| {
                    Some(
                        "Stopped at its cost cap; continuing needs a new confirmed run and budget."
                            .into(),
                    )
                }),
            ),
            None => (
                WorkflowStatus::Pending,
                Some("Interrupted before completion.".into()),
            ),
        };
        runs.push(selora_protocol::WorkflowPersistedRunDto {
            run_id: id.to_string(),
            name: spec.name,
            status,
            // A completed run is history, never a replay target. A budget trip
            // needs a new consent budget; an interrupted command may already
            // have side effects. Never offer any of them as a silent replay.
            resumable: !completed && !ambiguous_command && !budget_tripped && !journal.is_damaged(),
            detail: if ambiguous_command {
                Some("An interrupted command has ambiguous side effects and cannot resume.".into())
            } else if journal.is_damaged() {
                Some("The run journal is damaged and cannot resume safely.".into())
            } else {
                detail
            },
            phases: history.phases,
            steps: history.steps,
            reports: history.reports,
            artifacts: history.artifacts,
            tokens: history.tokens,
            cost_usd: history.cost_usd,
        });
    }
    runs.sort_by(|a, b| b.run_id.cmp(&a.run_id));
    runs
}

/// Mirror the engine's fail-closed rule for an interrupted `world.run`: its
/// effects may already exist, so it must not be advertised as replayable.
fn workflow_has_ambiguous_command(journal: &selora_workflow::journal::LoadedJournal) -> bool {
    let mut started = std::collections::BTreeMap::new();
    let mut finished = std::collections::BTreeMap::new();
    for entry in &journal.entries {
        match entry.kind.as_str() {
            "engine_step_started" => {
                let index = entry
                    .payload
                    .get("step")
                    .and_then(|step| step.get("index"))
                    .and_then(serde_json::Value::as_u64);
                let kind = entry
                    .payload
                    .get("step")
                    .and_then(|step| step.get("kind"))
                    .and_then(serde_json::Value::as_str);
                if let Some(index) = index {
                    started.insert(index, kind == Some("world_run"));
                }
            }
            "engine_step_finished" => {
                let index = entry
                    .payload
                    .get("step")
                    .and_then(|step| step.get("index"))
                    .and_then(serde_json::Value::as_u64);
                let status = entry
                    .payload
                    .get("step")
                    .and_then(|step| step.get("status"))
                    .and_then(serde_json::Value::as_str);
                if let Some(index) = index {
                    finished.insert(index, status == Some("ok"));
                }
            }
            _ => {}
        }
    }
    started
        .iter()
        .any(|(index, is_command)| *is_command && finished.get(index) != Some(&true))
}

/// The wire view of the settings the web shell consumes. Kept in one place so
/// the startup bootstrap and a persisted `SettingsUpdate` cannot drift.
fn settings_view(settings: &selora_settings::Settings) -> SettingsViewDto {
    SettingsViewDto {
        reduced_motion: settings.ui.reduce_motion,
        theme: settings.ui.theme.clone(),
        // §5.1: the rebindable pair, populated from settings.keybinds so a
        // user's egui-side rebind carries into the web shell.
        keybind_command_palette: settings.keybinds.command_palette.clone(),
        keybind_toggle_chat: settings.keybinds.toggle_chat.clone(),
    }
}
/// §3.4 (docs/ui/PHASE_Z2_ARCHITECTURE.md): keep the most recent 1024
/// events; a replay request older than the oldest retained event simply
/// yields a gap at the gate and the frontend re-bootstraps — the failure
/// mode is already designed for.
const EVENT_RETENTION_CAP: usize = 1024;

/// The only hosts `OpenExternal` hands to the OS browser: the project's own
/// pages (the repository and the Releases API) plus the provider consoles
/// the Settings view links to. A WebView anchor cannot navigate, so every
/// external link crosses this allowlist; `file:`, `javascript:`, `data:`,
/// UNC paths and unknown hosts are rejected with a typed error.
const EXTERNAL_URL_HOST_ALLOWLIST: &[&str] = &[
    "api.github.com",
    "console.anthropic.com",
    "github.com",
    "ollama.com",
    "open.bigmodel.cn",
    "openrouter.ai",
    "platform.deepseek.com",
    "platform.openai.com",
];

/// Why [`validate_external_url`] refused a link. Dispatch maps this to the
/// `external_url_rejected` protocol error; the tests assert the variants, so
/// proving a rejection never spawns a process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExternalUrlRejection {
    Empty,
    Backslash,
    Malformed,
    Scheme,
    MissingHost,
    Credentials,
    Port,
    HostNotAllowed,
}

impl ExternalUrlRejection {
    fn message(self) -> &'static str {
        match self {
            Self::Empty => "The link is empty.",
            Self::Backslash => "The link must not contain backslashes.",
            Self::Malformed => "The link is not an absolute URL.",
            Self::Scheme => "Only http and https links can be opened.",
            Self::MissingHost => "The link does not name a host.",
            Self::Credentials => "The link must not embed credentials.",
            Self::Port => "The link must not name a port.",
            Self::HostNotAllowed => "The link's host is not on the allowlist.",
        }
    }

    fn into_protocol_error(self) -> ProtocolErrorDto {
        ProtocolErrorDto {
            code: "external_url_rejected".into(),
            message: format!("Blocked external link: {}", self.message()),
            retryable: false,
        }
    }
}

/// The pure allowlist check behind `OpenExternal` (it never spawns): an
/// absolute `http`/`https` URL whose host is exactly one of
/// [`EXTERNAL_URL_HOST_ALLOWLIST`], with no credentials, no explicit port and
/// no backslashes (Windows reads them as path separators). Returns the
/// normalized URL to hand to the OS browser.
fn validate_external_url(raw: &str) -> Result<String, ExternalUrlRejection> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ExternalUrlRejection::Empty);
    }
    if trimmed.contains('\\') {
        return Err(ExternalUrlRejection::Backslash);
    }
    let parsed = Url::parse(trimmed).map_err(|_| ExternalUrlRejection::Malformed)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ExternalUrlRejection::Scheme);
    }
    let host = parsed.host_str().ok_or(ExternalUrlRejection::MissingHost)?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ExternalUrlRejection::Credentials);
    }
    if parsed.port().is_some() {
        return Err(ExternalUrlRejection::Port);
    }
    if !EXTERNAL_URL_HOST_ALLOWLIST
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(host))
    {
        return Err(ExternalUrlRejection::HostNotAllowed);
    }
    Ok(parsed.to_string())
}

/// The one seam all layout reads and writes route through (§4.5), so the
/// per-workspace map drops in without touching dispatch. Layout lives in
/// the `layout` section of `settings.json` — not the egui app's
/// `workspace.json` model (§4.1's decision, with its reasons recorded
/// there).
pub(crate) struct LayoutStore {
    /// The `settings.json` this store persists into. Parameterized so tests
    /// and sandboxed shells point at a tempdir — never the real user profile.
    settings_path: PathBuf,
}

impl LayoutStore {
    pub(crate) fn new(settings_path: PathBuf) -> Self {
        Self { settings_path }
    }

    /// Read the settings file (missing file = first-run defaults). Load
    /// errors propagate: `SaveLayout` maps them to `layout_save_failed`
    /// (§2.3 item 7) and the in-memory layout is unaffected.
    pub(crate) fn load_settings(
        &self,
    ) -> Result<selora_settings::Settings, selora_settings::SeloraError> {
        selora_settings::Settings::load_from(&self.settings_path)
    }

    /// §4.5's `load_layout(&Settings)`: the persisted `layout` section as a
    /// sanitized `LayoutDto` (§4.3 — a persisted value can never wedge the
    /// shell).
    pub(crate) fn load_layout(settings: &selora_settings::Settings) -> LayoutDto {
        LayoutDto::from(settings.layout.clone()).sanitized()
    }

    /// §4.5's `persist_layout(&mut Settings, &LayoutDto)`: writes the layout
    /// into the settings' `layout` section and saves the whole file through
    /// the existing atomic temp+rename path (`Settings::save_to`), so every
    /// other section survives.
    pub(crate) fn persist_layout(
        &self,
        settings: &mut selora_settings::Settings,
        layout: &LayoutDto,
    ) -> Result<(), selora_settings::SeloraError> {
        settings.layout = LayoutSettings::from(layout.clone());
        settings.save_to(&self.settings_path)
    }

    pub(crate) fn save_settings(
        &self,
        settings: &selora_settings::Settings,
    ) -> Result<(), selora_settings::SeloraError> {
        settings.save_to(&self.settings_path)
    }
}

/// The monotonic event log behind `selora://event` (§2.4/§3.4): one global
/// sequence, the 1024-event replay ring, and the emit path. Extracted in
/// Z3 so the session relays ([`crate::sessions`]) record through the same
/// seam the layout commands do — every event, layout or agent, rides one
/// monotonic sequence.
pub(crate) struct EventLog {
    core_instance_id: CoreInstanceId,
    sequence: Mutex<u64>,
    events: Mutex<Vec<EventEnvelope<FrontendEvent>>>,
}

impl EventLog {
    pub(crate) fn new(core_instance_id: CoreInstanceId) -> Self {
        Self {
            core_instance_id,
            sequence: Mutex::new(0),
            events: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn core_instance_id(&self) -> CoreInstanceId {
        self.core_instance_id.clone()
    }

    /// The bootstrap's `snapshot_sequence`: every event recorded so far.
    pub(crate) fn snapshot_sequence(&self) -> u64 {
        *self.sequence.lock().expect("sequence lock")
    }

    /// Record and emit (the envelope's session id stays `None`).
    pub fn record<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        payload: FrontendEvent,
        request_id: Option<selora_protocol::RequestId>,
    ) {
        let event = self.push(payload, request_id, None);
        let _ = app.emit("selora://event", &event);
    }

    /// Record and emit with the session on the envelope — the Z3 session
    /// events (`ChatDelta` and the agent stream) name their session so the
    /// frontend can route by it without opening every payload.
    pub fn record_session<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        payload: FrontendEvent,
        request_id: Option<selora_protocol::RequestId>,
        session_id: SessionId,
    ) {
        let event = self.push(payload, request_id, Some(session_id));
        let _ = app.emit("selora://event", &event);
    }

    /// Record without emitting — the §4.4 boot toast is recorded during
    /// `DesktopState` construction, before any `AppHandle` exists; the
    /// replay flow (§3.4) delivers it.
    pub(crate) fn push(
        &self,
        payload: FrontendEvent,
        request_id: Option<selora_protocol::RequestId>,
        session_id: Option<SessionId>,
    ) -> EventEnvelope<FrontendEvent> {
        let mut sequence = self.sequence.lock().expect("sequence lock");
        *sequence = sequence.saturating_add(1);
        let mut events = self.events.lock().expect("event lock");
        let event = EventEnvelope {
            protocol_version: PROTOCOL_VERSION,
            workspace_id: None,
            core_instance_id: self.core_instance_id.clone(),
            sequence: *sequence,
            request_id,
            session_id,
            payload,
        };
        events.push(event.clone());
        if events.len() > EVENT_RETENTION_CAP {
            // Keep the most recent 1024; the sequences keep counting.
            let overflow = events.len() - EVENT_RETENTION_CAP;
            events.drain(0..overflow);
        }
        event
    }

    /// §3.4's replay backing store for the `get_events_after` command.
    pub fn events_after(&self, sequence: u64) -> Vec<EventEnvelope<FrontendEvent>> {
        self.events
            .lock()
            .expect("event lock")
            .iter()
            .filter(|event| event.sequence > sequence)
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Phase 8 wave 2 — workflow dispatch (ROADMAP 8c, ARCHITECTURE §18).
//
// `state.rs` owns the command/event mapping only: `selora-workflow` runs the
// script, `workflows_host.rs` implements the blocking `Host`, and the run map
// here gives `WorkflowStop`/`WorkflowDecision` their address. The engine is a
// blocking call and always rides `spawn_blocking` (ARCHITECTURE §18.4).
// ---------------------------------------------------------------------------

/// One run's stop switch: `workflows_host::WorkflowHost::cancel` captured as a
/// closure, so the run map never needs the concrete host type.
pub(crate) type WorkflowCancel = Arc<dyn Fn() + Send + Sync>;

pub(crate) type WorkflowDecide = Arc<
    dyn Fn(Uuid, selora_ai::AgentDecision) -> Result<(), selora_workflow::HostError> + Send + Sync,
>;

/// What one run needs from `workflows_host.rs`: the engine host and its stop
/// switch, both built for the workspace root the run was started in.
pub(crate) struct WorkflowHostBundle {
    pub(crate) host: Arc<dyn selora_workflow::Host>,
    pub(crate) cancel: WorkflowCancel,
    pub(crate) decide: Option<WorkflowDecide>,
}

/// The shell-owned half of the host config: the gateway endpoint, key and
/// model are read from the session engine at run start. The host's cap
/// defaults live in `workflows_host.rs`.
pub(crate) struct WorkflowHostSettings {
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) model_metadata: Option<selora_ai::Model>,
    pub(crate) caps: selora_ai::TurnCaps,
    pub(crate) on_approval: crate::workflows_host::WorkflowApprovalHandler,
}

/// Builds one host per run. Production is [`production_workflow_host`]; tests
/// install a fake through [`DesktopState::install_workflow_host_factory`].
pub(crate) type WorkflowHostFactory =
    Arc<dyn Fn(PathBuf, WorkflowHostSettings) -> WorkflowHostBundle + Send + Sync>;

/// The construction contract this slice expects from `workflows_host.rs`:
/// `WorkflowHost::new(root, WorkflowHostConfig) -> Arc<WorkflowHost>`, plus
/// `impl selora_workflow::Host for WorkflowHost` and `WorkflowHost::cancel`.
/// `WorkflowHostConfig::new(base_url, api_key, model)` is the shell's entry
/// point; every cap stays at the host's §18 defaults.
fn production_workflow_host(root: PathBuf, settings: WorkflowHostSettings) -> WorkflowHostBundle {
    let mut config = crate::workflows_host::WorkflowHostConfig::new(
        settings.base_url,
        settings.api_key,
        settings.model,
    )
    .with_turn_caps(settings.caps)
    .with_approval_handler(settings.on_approval);
    if let Some(model) = settings.model_metadata {
        config = config.with_model(model);
    }
    let host = crate::workflows_host::WorkflowHost::new(root, config);
    let cancel_host = Arc::clone(&host);
    let decision_host = Arc::clone(&host);
    WorkflowHostBundle {
        host,
        cancel: Arc::new(move || cancel_host.cancel()),
        decide: Some(Arc::new(move |id, decision| {
            decision_host.decide(id, decision)
        })),
    }
}

/// One live run, keyed by the `request_id` the `WorkflowRun` command carried
/// (the id `WorkflowStop`/`WorkflowDecision` name). The host is kept alive for
/// the run's lifetime and its `cancel` closure is the stop switch.
struct ActiveRun {
    /// Kept for the run's lifetime; the engine's own `Arc` clone does the work.
    /// The `WorkflowDecision` slice will read the approval slot through it.
    #[allow(dead_code)]
    host: Arc<dyn selora_workflow::Host>,
    cancel: WorkflowCancel,
    decide: Option<WorkflowDecide>,
    run_id: String,
    workspace_id: WorkspaceId,
    /// Distinguishes this run from a later run that reuses the request id.
    token: Uuid,
}

/// The bridge from the engine's blocking progress events to `selora://event`.
///
/// It owns the per-run protocol context (request id, cost cap, latest phase)
/// and the exactly-once guard for `WorkflowFinished`. The sink is an `Fn`, so
/// the mutable pieces are behind locks/atomics; each call is short and takes
/// no `await`.
struct WorkflowStream<R: tauri::Runtime> {
    log: Arc<EventLog>,
    app: tauri::AppHandle<R>,
    request_id: RequestId,
    workspace_id: WorkspaceId,
    /// The workspace the run was started in. The terminal refresh uses it to
    /// prove the user is still looking at the same workspace.
    workspace_root: PathBuf,
    /// The shell's one open-workspace slot. The terminal refresh locks it,
    /// reloads clean buffers, then releases it before emitting any event.
    workspace: Arc<Mutex<Option<Workspace>>>,
    /// The language-server bridge slot. A clean reload sends the new full
    /// text and retracts stale diagnostics before the next request.
    lsp: Arc<Mutex<Option<LspBridge>>>,
    max_cost_usd: f64,
    name: String,
    started: AtomicBool,
    /// The most recent `phase()` marker: the engine's steps carry no phase.
    phase: Mutex<String>,
    /// Engine-minted run id, filled by the first (`Run`) event.
    run_id: Arc<Mutex<Option<String>>>,
    finished: AtomicBool,
}

impl<R: tauri::Runtime> WorkflowStream<R> {
    fn record(&self, payload: FrontendEvent) {
        self.log
            .record(&self.app, payload, Some(self.request_id.clone()));
    }

    /// The engine emits `Run` before evaluating, so every later event has an
    /// id. The request id is only a defensive fallback for a misbehaving sink.
    fn current_run_id(&self) -> String {
        self.run_id
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
            .unwrap_or_else(|| self.request_id.as_str().to_owned())
    }

    /// Post-run disk refresh (ROADMAP P8 gap 3): a workflow edits files
    /// directly, so open editor tabs must re-read them.
    ///
    /// The refresh only runs when the open workspace is still the one this run
    /// started in — a workspace switch during the run must never update the
    /// new workspace's buffers, raise its conflicts, or sync its language
    /// server. The workspace lock is taken for the reload only: the engine
    /// call already finished, and every event is emitted after the guard is
    /// dropped.
    fn refresh_workspace_buffers(&self) {
        let outcome = {
            let guard = self
                .workspace
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(workspace) = guard.as_ref() else {
                return;
            };
            if workspace.root() != self.workspace_root.as_path()
                || workspace_id_for(workspace.root()) != self.workspace_id
            {
                return;
            }
            workspace.refresh_clean_buffers()
        };
        self.publish_refresh(outcome);
    }

    /// Emit one `BufferSnapshot` per changed clean buffer (after syncing its
    /// language server), one `BufferConflict` per dirty buffer whose disk text
    /// changed, one `BufferConflictCleared` for a marker that no longer
    /// applies, and one warning toast naming files the refresh could not read.
    /// Dirty local text is never replaced here.
    fn publish_refresh(&self, outcome: crate::workspace::BufferRefreshOutcome) {
        for snapshot in outcome.updated {
            self.sync_lsp_reload(&snapshot.path, &snapshot.text);
            self.record(FrontendEvent::BufferSnapshot(snapshot));
        }
        for cleared in outcome.cleared {
            self.record(FrontendEvent::BufferConflictCleared(cleared));
        }
        for skipped in &outcome.skipped {
            let absolute = self.workspace_root.join(skipped.path.as_str());
            self.with_lsp(|bridge| bridge.invalidate_diagnostics(&absolute));
        }
        for conflict in outcome.conflicts {
            self.record(FrontendEvent::BufferConflict(conflict));
        }
        if outcome.skipped.is_empty() {
            return;
        }
        let notes = outcome
            .skipped
            .iter()
            .map(|skipped| format!("'{}' ({})", skipped.path.as_str(), skipped.message))
            .collect::<Vec<_>>();
        self.record(FrontendEvent::Toast(ToastDto {
            severity: ToastSeverity::Warning,
            message: format!(
                "Workflow files changed on disk; the editor {}.",
                notes.join("; ")
            ),
        }));
    }

    /// Send the reloaded text to the language server, then retract this path's
    /// diagnostics from the old text. A bridge that never started has no open
    /// document and no diagnostics, so there is nothing to sync.
    fn sync_lsp_reload(&self, path: &selora_protocol::WorkspacePath, text: &str) {
        let absolute = self.workspace_root.join(path.as_str());
        self.with_lsp(|bridge| bridge.reload(absolute, text.to_owned()));
    }

    /// Run `action` against the run workspace's bridge, starting it if needed.
    /// Only called after the run's workspace identity matched the open one.
    fn with_lsp(&self, action: impl FnOnce(&LspBridge)) {
        let log: Arc<EventLog> = Arc::clone(&self.log);
        let app = self.app.clone();
        let emit: crate::lsp_bridge::Emit = Arc::new(move |event| {
            log.record(&app, event, None);
        });
        let mut guard = self.lsp.lock().unwrap_or_else(|poison| poison.into_inner());
        if guard.is_none() {
            *guard = Some(LspBridge::start(self.workspace_root.clone(), emit));
        }
        if let Some(bridge) = guard.as_ref() {
            action(bridge);
        }
    }

    fn record_started(&self, name: String) {
        if !self.started.swap(true, Ordering::SeqCst) {
            self.record(FrontendEvent::WorkflowStarted(WorkflowStartedDto {
                run_id: self.current_run_id(),
                request_id: self.request_id.clone(),
                workspace_id: Some(self.workspace_id.clone()),
                name,
                max_cost_usd: self.max_cost_usd,
            }));
        }
    }

    fn on_approval(&self, id: Uuid, card: selora_ai::ApprovalCard) {
        if self.finished.load(Ordering::SeqCst) {
            return;
        }
        let (summary, path_or_command, diff_or_input) = match card {
            selora_ai::ApprovalCard::Diff { path, diff } => {
                (format!("Write {path}"), path, Some(diff))
            }
            selora_ai::ApprovalCard::Delete { path } => (format!("Delete {path}"), path, None),
            selora_ai::ApprovalCard::Command { command } => {
                (format!("Run {command}"), command, None)
            }
            // Nested workflow tools are not armed by WorkflowHost. Do not
            // turn an unexpected nested grant into a generic write card.
            selora_ai::ApprovalCard::Workflow { .. } => return,
        };
        self.record(FrontendEvent::WorkflowApprovalNeeded(
            WorkflowApprovalNeededDto {
                run_id: self.current_run_id(),
                request_id: self.request_id.clone(),
                approval_id: ApprovalId::new(id.to_string()).expect("UUID is a valid approval id"),
                summary,
                path_or_command,
                diff_or_input,
                can_approve_all_this_turn: false,
            },
        ));
    }

    fn on_event(&self, event: selora_workflow::WorkflowEvent) {
        use selora_workflow::WorkflowEvent;
        match event {
            WorkflowEvent::Run { run_id, name } => {
                let run_id = run_id.to_string();
                *self
                    .run_id
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = Some(run_id.clone());
                self.record_started(name);
            }
            WorkflowEvent::Phase { name } => {
                *self
                    .phase
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = name.clone();
                self.record(FrontendEvent::WorkflowPhase(WorkflowPhaseEventDto {
                    run_id: self.current_run_id(),
                    phase: WorkflowPhaseDto {
                        name,
                        status: WorkflowStatus::Running,
                    },
                }));
            }
            WorkflowEvent::Step { step } => {
                let phase = self
                    .phase
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .clone();
                self.record(FrontendEvent::WorkflowStep(WorkflowStepEventDto {
                    run_id: self.current_run_id(),
                    step: workflow_step_dto(step, phase),
                }));
            }
            WorkflowEvent::Report { item } => {
                self.record(FrontendEvent::WorkflowReport(WorkflowReportEventDto {
                    run_id: self.current_run_id(),
                    report: WorkflowReportDto {
                        id: format!("report-{}", item.index),
                        item: workflow_report_item_text(&item.item),
                        artifact_id: item.artifact_id,
                    },
                }));
            }
            WorkflowEvent::Artifact { artifact } => {
                let (kind, body) = match artifact.content {
                    selora_workflow::ArtifactContent::Markdown(body) => {
                        ("markdown".to_owned(), body)
                    }
                    selora_workflow::ArtifactContent::File(path) => ("file".to_owned(), path),
                };
                self.record(FrontendEvent::WorkflowArtifact(WorkflowArtifactEventDto {
                    run_id: self.current_run_id(),
                    artifact: WorkflowArtifactDto {
                        id: artifact.id,
                        title: artifact.title.unwrap_or_default(),
                        kind,
                        body,
                    },
                }));
            }
            // The protocol union has no `workflow_log` arm; the run log the
            // panel shows is the step/report stream. Engine log lines are
            // intentionally dropped rather than smuggled into a toast.
            WorkflowEvent::Log { .. } => {}
            WorkflowEvent::Finished { report } => {
                if self.finished.swap(true, Ordering::SeqCst) {
                    return;
                }
                let run_id = report.run_id.to_string();
                *self
                    .run_id
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = Some(run_id.clone());
                self.record(FrontendEvent::WorkflowFinished(WorkflowFinishedDto {
                    run_id,
                    request_id: self.request_id.clone(),
                    status: workflow_run_status(report.status),
                    // The engine reports cost, not tokens; the run summary
                    // carries what the host actually meters.
                    tokens: report.tokens,
                    cost_usd: report.cost_usd,
                    max_cost_usd: self.max_cost_usd,
                    error: report.error,
                }));
                // Terminal event recorded: files the run wrote are now safe to
                // reload into clean tabs.
                self.refresh_workspace_buffers();
            }
        }
    }

    /// The defensive terminal event for a run whose engine returned before it
    /// could emit `Finished` (a pre-`Run` rejection, a worker panic, or an
    /// engine contract break). The normal path is guarded by
    /// [`Self::finished`], so this is a no-op after the engine's own terminal
    /// event.
    fn finish_fallback(
        &self,
        status: WorkflowStatus,
        cost_usd: f64,
        tokens: u64,
        error: Option<String>,
    ) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        // Once dispatch accepted a worker, even a persistence preflight
        // failure needs a visible run row, not an orphaned terminal event.
        self.record_started(self.name.clone());
        self.record(FrontendEvent::WorkflowFinished(WorkflowFinishedDto {
            run_id: self.current_run_id(),
            request_id: self.request_id.clone(),
            status,
            tokens,
            cost_usd,
            max_cost_usd: self.max_cost_usd,
            error,
        }));
        // A worker that never reached the engine's `Finished` still may have
        // written files before the panic/pre-Run rejection.
        self.refresh_workspace_buffers();
    }
}

fn workflow_run_status(status: selora_workflow::RunStatus) -> WorkflowStatus {
    use selora_workflow::RunStatus;
    match status {
        RunStatus::Pending => WorkflowStatus::Pending,
        RunStatus::Running => WorkflowStatus::Running,
        RunStatus::AwaitingApproval => WorkflowStatus::AwaitingApproval,
        RunStatus::Completed => WorkflowStatus::Completed,
        RunStatus::Failed => WorkflowStatus::Failed,
        RunStatus::Stopped => WorkflowStatus::Stopped,
        // The protocol has no separate budget state; a cap trip is a failure
        // whose `error` names the budget.
        RunStatus::BudgetExceeded => WorkflowStatus::Failed,
    }
}

fn workflow_step_status(status: selora_workflow::StepStatus) -> WorkflowStatus {
    use selora_workflow::StepStatus;
    match status {
        StepStatus::Pending => WorkflowStatus::Pending,
        StepStatus::Running => WorkflowStatus::Running,
        StepStatus::Ok => WorkflowStatus::Completed,
        StepStatus::Failed => WorkflowStatus::Failed,
        // No `skipped` arm exists on the wire; `stopped` keeps a journal-skipped
        // step from reading as a success.
        StepStatus::Skipped => WorkflowStatus::Stopped,
    }
}

fn workflow_step_label(kind: selora_workflow::StepKind) -> &'static str {
    use selora_workflow::StepKind;
    match kind {
        StepKind::Phase => "phase",
        StepKind::Ask => "agent",
        StepKind::WorldRun => "command",
        StepKind::Files => "files",
        StepKind::Git => "git",
        StepKind::Report => "report",
        StepKind::Artifact => "artifact",
        StepKind::Log => "log",
    }
}

fn workflow_step_dto(step: selora_workflow::WorkflowStep, phase: String) -> WorkflowStepDto {
    // The engine's `detail` is the human label (`cargo test`, `read
    // src/main.rs`, an ask's instructions truncated to 80 chars); the wire
    // `label` is that action. The wire `detail` is the diagnostic slot, so a
    // failed step puts its error there.
    let label = if step.detail.is_empty() {
        workflow_step_label(step.kind).to_owned()
    } else {
        step.detail
    };
    WorkflowStepDto {
        id: format!("step-{}", step.index),
        phase,
        label,
        status: workflow_step_status(step.status),
        agent: step.agent,
        detail: step.error,
    }
}

fn workflow_report_item_text(item: &serde_json::Value) -> String {
    match item {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn workflow_run_not_found() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "workflow_run_not_found".into(),
        message: "No workflow run is active for that request.".into(),
        retryable: false,
    }
}

fn workflow_spec_error(error: selora_workflow::WorkflowError) -> ProtocolErrorDto {
    use selora_workflow::WorkflowError;
    let code = match error {
        WorkflowError::ScriptTooLarge { .. } => "workflow_script_too_large",
        WorkflowError::Syntax { .. } => "workflow_script_invalid",
        WorkflowError::NonLiteralCommand { .. } => "workflow_command_not_literal",
        _ => "invalid_workflow_spec",
    };
    ProtocolErrorDto {
        code: code.into(),
        message: error.to_string(),
        retryable: false,
    }
}

fn workflow_persistence_error(error: selora_workflow::WorkflowError) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "workflow_resume_rejected".into(),
        message: error.to_string(),
        retryable: false,
    }
}

/// A run whose cost cap already fired cannot resume: the confirmed grant is
/// spent, and resume never raises it silently. Continuing needs a new run with
/// a newly confirmed budget.
fn workflow_budget_exhausted_error(max_cost_usd: f64, spent_usd: f64) -> ProtocolErrorDto {
    let spend = if spent_usd > 0.0 {
        format!(" (${spent_usd:.2} spent)")
    } else {
        String::new()
    };
    ProtocolErrorDto {
        code: "workflow_budget_exhausted".into(),
        message: format!(
            "This run reached its ${max_cost_usd:.2} cost cap{spend}. A resume cannot raise the confirmed budget: start a new run and confirm a new budget to continue."
        ),
        retryable: false,
    }
}

/// The resume refusal that asks for a second confirmation. It names the count,
/// the first few step/agent labels, and the effect risk in one string (the
/// protocol error has no structured detail slot).
fn workflow_incomplete_asks_message(asks: &[selora_workflow::IncompleteAsk]) -> String {
    const SHOWN: usize = 3;
    let names = asks
        .iter()
        .take(SHOWN)
        .map(|ask| match &ask.label {
            Some(label) => format!("step {} ({}: {label})", ask.step_id, ask.agent),
            None => format!("step {} ({})", ask.step_id, ask.agent),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let more = asks.len().saturating_sub(SHOWN);
    let suffix = if more > 0 {
        format!(", and {more} more")
    } else {
        String::new()
    };
    format!(
        "Resume would dispatch {} unfinished ask step(s) again: {names}{suffix}. Tools in those steps may already have applied effects. Confirm to resume anyway.",
        asks.len()
    )
}

/// §18.1's confirmation contract, restated at the command boundary: the size
/// cap and the syntax scan run through [`selora_workflow::prepare`], and every
/// command the script actually runs must be in the set the user confirmed.
/// The engine re-checks both when the run starts (it refuses to trust the
/// spec); doing it here means a bad request gets a typed command error instead
/// of a phantom run.
fn build_workflow_spec(
    spec: WorkflowSpecDto,
) -> Result<selora_workflow::WorkflowSpec, ProtocolErrorDto> {
    let WorkflowSpecDto {
        name,
        script,
        commands,
        max_cost_usd,
    } = spec;
    let prepared =
        selora_workflow::prepare(name, script, max_cost_usd).map_err(workflow_spec_error)?;
    let approved = selora_workflow::CommandSet::new(commands);
    for command in prepared.command_set.iter() {
        if !approved.contains(command) {
            return Err(ProtocolErrorDto {
                code: "workflow_command_not_confirmed".into(),
                message: format!(
                    "The workflow script runs `{command}`, which is not in the confirmed command set."
                ),
                retryable: false,
            });
        }
    }
    Ok(selora_workflow::WorkflowSpec {
        name: prepared.name,
        source: prepared.script,
        command_set: approved,
        max_cost_usd: prepared.max_cost_usd,
    })
}

#[derive(Clone)]
pub struct DesktopState {
    log: Arc<EventLog>,
    /// The last persisted view. Behind a lock because `dispatch` takes `&self`
    /// while an accepted `SettingsUpdate` must change what the next bootstrap
    /// (and its event) reports.
    settings: Arc<Mutex<SettingsViewDto>>,
    /// §2.4: the authoritative in-memory layout, seeded at startup from
    /// settings (§4) and mutated only by accepted `ToggleSurface` /
    /// `ResizeRegion` commands. The frontend applies changes only from the
    /// `WorkspaceLayoutChanged` event.
    layout: Arc<Mutex<LayoutDto>>,
    /// The persistence seam (§4.5).
    layout_store: Arc<LayoutStore>,
    /// The Z3 session engine (chat + agent turns) behind the agent dock.
    sessions: Arc<SessionEngine>,
    /// Phase Z4: the open workspace (file tree, buffers, Git). None until
    /// the folder picker succeeds; every workspace command answers the typed
    /// no_workspace error while it is unset.
    workspace: Arc<Mutex<Option<Workspace>>>,
    /// Phase Z4b: the language-server bridge for the open workspace, created
    /// lazily on the first editor command (it needs an AppHandle to emit).
    lsp: Arc<Mutex<Option<LspBridge>>>,
    /// Phase 8 wave 2: the live workflow runs, keyed by their `WorkflowRun`
    /// request id (the id `WorkflowStop`/`WorkflowDecision` name). One entry
    /// per accepted run; the worker removes its own entry on completion.
    workflow_runs: Arc<Mutex<HashMap<RequestId, ActiveRun>>>,
    /// The host constructor. A `Mutex` so the hermetic tests can swap in a
    /// fake host; production never replaces it.
    workflow_hosts: Arc<Mutex<WorkflowHostFactory>>,
}

/// How [`DesktopState::launch_workflow`] should open the durable run.
enum WorkflowLaunch {
    /// A newly confirmed run.
    New,
    /// Replay an existing run; the flag is the user's explicit consent to
    /// re-dispatch unfinished asks.
    Resume { allow_incomplete_asks: bool },
}

impl DesktopState {
    /// The production constructor: `%APPDATA%\selora-ide\settings.json` (or
    /// the `SELORA_SETTINGS_DIR` override).
    pub fn new() -> Self {
        let state = Self::from_settings_path(selora_settings::Settings::default_path());
        // Z3's stand-in for the Z4 folder picker: point the agent's tools at
        // a real tree via `SELORA_WORKSPACE_ROOT` (an existing directory).
        // Without it `SendAgent` answers the typed `no_workspace` error.
        // Z4: attach a real workspace at boot — `SELORA_WORKSPACE_ROOT` when
        // set, otherwise the process working directory. The explorer, editor
        // and the agent's tools all read through this one Workspace.
        let boot_root = std::env::var("SELORA_WORKSPACE_ROOT")
            .ok()
            .map(|root| PathBuf::from(root.trim()))
            .filter(|root| !root.as_os_str().is_empty() && root.is_dir())
            .or_else(|| std::env::current_dir().ok());
        if let Some(root) = boot_root {
            let _ = state.attach_workspace(root);
        }
        state
    }

    /// Hermetic constructor: an explicit settings path. The §4.4 boot toast
    /// (a settings load/wipe error) is recorded *during construction* — no
    /// `AppHandle` exists yet, so it is recorded, not emitted; the replay
    /// flow (§3.4) delivers it, and its sequence is ≤ the bootstrap's
    /// `snapshot_sequence`.
    pub fn from_settings_path(settings_path: PathBuf) -> Self {
        let layout_store = Arc::new(LayoutStore::new(settings_path.clone()));
        // Same semantics as `Settings::load_or_default`: fall back to the
        // defaults and keep the error for the caller to toast.
        let (settings, load_error) = match selora_settings::Settings::load_from(&settings_path) {
            Ok(settings) => (settings, None),
            Err(error) => (selora_settings::Settings::default(), Some(error)),
        };
        let state = Self::from_parts(settings, layout_store, ApiKeySource::Keyring);
        if let Some(error) = load_error {
            state.log.push(
                FrontendEvent::Toast(ToastDto {
                    severity: ToastSeverity::Error,
                    message: format!("Settings could not be loaded: {error}"),
                }),
                None,
                None,
            );
        }
        state
    }

    /// The no-IO constructor for the state tests: settings provided directly,
    /// store at the default path. No test that uses it ever persists, so the
    /// default path is never written; the round-trip tests use
    /// [`DesktopState::from_settings_path`] with a tempdir instead. The
    /// session engine gets a fixed test key (and the gateway follows
    /// `settings.selora.base_url`, so a loopback mock gateway is just a
    /// settings field) — no test ever touches the real Credential Manager.
    #[cfg(test)]
    pub fn with_settings(settings: selora_settings::Settings) -> Self {
        Self::from_parts(
            settings,
            Arc::new(LayoutStore::new(selora_settings::Settings::default_path())),
            ApiKeySource::Fixed(Some("sk-gw-desktop-test-key".into())),
        )
    }

    /// The absent-key twin (the first-run `not_connected` path).
    #[cfg(test)]
    pub fn with_settings_no_key(settings: selora_settings::Settings) -> Self {
        Self::from_parts(
            settings,
            Arc::new(LayoutStore::new(selora_settings::Settings::default_path())),
            ApiKeySource::Fixed(None),
        )
    }

    fn from_parts(
        settings: selora_settings::Settings,
        layout_store: Arc<LayoutStore>,
        key_source: ApiKeySource,
    ) -> Self {
        let core_instance_id =
            CoreInstanceId::new(Uuid::new_v4().to_string()).expect("uuid is valid");
        Self {
            log: Arc::new(EventLog::new(core_instance_id)),
            settings: Arc::new(Mutex::new(settings_view(&settings))),
            // §4: the bootstrap reflects the persisted layout; §4.3's
            // sanitization runs on load.
            layout: Arc::new(Mutex::new(LayoutStore::load_layout(&settings))),
            layout_store,
            sessions: Arc::new(SessionEngine::new(EngineConfig {
                base_url: settings.selora.base_url.clone(),
                active_model: settings.selora.active_model.clone(),
                agent: (*settings.agent).clone(),
                key_source,
            })),
            workspace: Arc::new(Mutex::new(None)),
            lsp: Arc::new(Mutex::new(None)),
            workflow_runs: Arc::new(Mutex::new(HashMap::new())),
            workflow_hosts: Arc::new(Mutex::new(
                Arc::new(production_workflow_host) as WorkflowHostFactory
            )),
        }
    }

    /// Hermetic tests swap the real `workflows_host::WorkflowHost` for a fake
    /// whose commands complete without a gateway. Kept `cfg(test)` so no
    /// production path can silently bypass the real host.
    #[cfg(test)]
    fn install_workflow_host_factory(&self, factory: WorkflowHostFactory) {
        *self
            .workflow_hosts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = factory;
    }

    /// Attach a workspace root: the service for the explorer/editor/Git and
    /// the agent engine's tool root move together, so "open a folder" means
    /// one thing for the whole app.
    fn attach_workspace(&self, root: PathBuf) -> Result<(), WorkspaceError> {
        let workspace = Workspace::open(root)?;
        self.sessions()
            .set_workspace_root(Some(workspace.root().to_path_buf()));
        *self
            .workspace
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(workspace);
        Ok(())
    }

    /// Map a workspace-service failure onto the wire error shape.
    fn workspace_error(error: WorkspaceError) -> ProtocolErrorDto {
        ProtocolErrorDto {
            code: error.code.into(),
            message: error.message,
            retryable: error.code == "stale_buffer_version" || error.code == "io_error",
        }
    }

    /// Run `action` against the open workspace, or answer the typed
    /// `no_workspace` error the empty states already handle.
    fn with_workspace<T>(
        &self,
        action: impl FnOnce(&Workspace) -> Result<T, WorkspaceError>,
    ) -> Result<T, ProtocolErrorDto> {
        let guard = self
            .workspace
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match guard.as_ref() {
            Some(workspace) => action(workspace).map_err(Self::workspace_error),
            None => Err(ProtocolErrorDto {
                code: "no_workspace".into(),
                message: "Open a folder first.".into(),
                retryable: false,
            }),
        }
    }

    /// The open workspace's canonical root, if any.
    fn workspace_root_path(&self) -> Option<PathBuf> {
        let guard = self
            .workspace
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        guard
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf())
    }

    /// Address an LSP position request at the buffer's absolute path. The
    /// document must already be open in the manager (the tab's OpenFile did
    /// it); an unknown buffer is a silent no-op.
    fn request_lsp<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        buffer_id: &selora_protocol::BufferId,
        action: impl FnOnce(&LspBridge, PathBuf),
    ) {
        let Ok(snapshot) = self.with_workspace(|workspace| workspace.buffer(buffer_id)) else {
            return;
        };
        if let Some(root) = self.workspace_root_path() {
            let absolute = root.join(snapshot.path.as_str());
            self.with_lsp(app, |bridge| action(bridge, absolute));
        }
    }

    /// Run `action` against the language-server bridge, starting it on first
    /// use (it needs an `AppHandle` to emit through). A missing server is the
    /// manager's story to tell (one toast), never a command failure.
    fn with_lsp<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        action: impl FnOnce(&LspBridge),
    ) {
        let Some(root) = self.workspace_root_path() else {
            return;
        };
        let mut guard = self.lsp.lock().unwrap_or_else(|poison| poison.into_inner());
        if guard.is_none() {
            let log = Arc::clone(&self.log);
            let app_handle = app.clone();
            let emit: crate::lsp_bridge::Emit = Arc::new(move |event| {
                log.record(&app_handle, event, None);
            });
            *guard = Some(LspBridge::start(root, emit));
        }
        if let Some(bridge) = guard.as_ref() {
            action(bridge);
        }
    }

    /// The id the frontend's envelope context should carry right now.
    fn current_workspace_id(&self) -> Option<WorkspaceId> {
        let guard = self
            .workspace
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        guard
            .as_ref()
            .map(|workspace| workspace_id_for(workspace.root()))
    }

    /// Core epoch accessor retained for reconnect/event adapter wiring.
    #[allow(dead_code)]
    pub fn core_instance_id(&self) -> CoreInstanceId {
        self.log.core_instance_id()
    }

    /// The Z3 seams the boot probe and tests reach in through.
    pub(crate) fn event_log(&self) -> Arc<EventLog> {
        Arc::clone(&self.log)
    }
    pub(crate) fn sessions(&self) -> Arc<SessionEngine> {
        Arc::clone(&self.sessions)
    }

    pub fn update_selora_settings(
        &self,
        base_url: Option<String>,
        active_model: Option<String>,
    ) -> Result<(), selora_settings::SeloraError> {
        let mut settings = self.layout_store.load_settings().unwrap_or_default();
        if let Some(url) = base_url {
            settings.selora.base_url = url;
        }
        if let Some(model) = active_model {
            settings.selora.active_model = Some(model);
        }
        self.layout_store.save_settings(&settings)
    }

    /// Apply a validated settings patch through the one persistence seam:
    /// load the full file (so every other section survives), write the new
    /// values atomically, refresh the in-memory view, then publish a
    /// `BootstrapState` event so every consumer re-reads.
    ///
    /// Invalid input is rejected before either disk or memory is touched.
    pub fn apply_settings_update<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        patch: SettingsPatchDto,
        request_id: Option<selora_protocol::RequestId>,
    ) -> Result<BootstrapState, ProtocolErrorDto> {
        if patch.reduced_motion.is_none() && patch.theme.is_none() {
            return Err(ProtocolErrorDto {
                code: "invalid_settings_patch".into(),
                message: "Settings update must change at least one field.".into(),
                retryable: false,
            });
        }

        let theme = match patch.theme.as_deref() {
            Some(raw) => {
                let theme = raw.trim();
                match theme {
                    "selora-dark" | "dark" | "space-black" | "light" => Some(theme.to_owned()),
                    _ => {
                        return Err(ProtocolErrorDto {
                            code: "invalid_settings_theme".into(),
                            message: "Theme must be one of: selora-dark, space-black, light."
                                .into(),
                            retryable: false,
                        })
                    }
                }
            }
            None => None,
        };

        let mut settings = self
            .layout_store
            .load_settings()
            .map_err(|error| ProtocolErrorDto {
                code: "settings_load_failed".into(),
                message: format!("Failed to load settings: {error}"),
                retryable: true,
            })?;
        if let Some(reduced_motion) = patch.reduced_motion {
            settings.ui.reduce_motion = reduced_motion;
        }
        if let Some(theme) = theme {
            settings.ui.theme = theme;
        }
        self.layout_store
            .save_settings(&settings)
            .map_err(|error| ProtocolErrorDto {
                code: "settings_save_failed".into(),
                message: format!("Failed to save settings: {error}"),
                retryable: true,
            })?;

        *self
            .settings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = settings_view(&settings);

        let snapshot = self.bootstrap();
        self.record_event(
            app,
            FrontendEvent::BootstrapState(Box::new(snapshot.clone())),
            request_id,
        );
        Ok(snapshot)
    }

    pub fn bootstrap(&self) -> BootstrapState {
        let workspace = self
            .workspace
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (workspace_dto, buffers, active_buffer_id, file_tree, git) = match workspace.as_ref() {
            Some(open) => {
                let buffers = open.buffers();
                let active = buffers.first().map(|buffer| buffer.buffer_id.clone());
                let tree = open.tree().unwrap_or(FileTreeDto { root: None });
                let git = open.git_status().unwrap_or(selora_protocol::GitStatusDto {
                    branch: None,
                    changed_paths: Vec::new(),
                    staged_paths: Vec::new(),
                    // A failed status read has no per-slot source either; the
                    // additive P2 field stays empty (readers fall back to the
                    // legacy paths, which are empty too).
                    files: Vec::new(),
                });
                (
                    selora_protocol::WorkspaceDto {
                        id: workspace_id_for(open.root()),
                        core_instance_id: self.log.core_instance_id(),
                        root: Some(open.root().to_string_lossy().to_string()),
                        display_name: open.display_name(),
                    },
                    buffers,
                    active,
                    tree,
                    git,
                )
            }
            None => (
                selora_protocol::WorkspaceDto {
                    id: WorkspaceId::new("no-workspace").expect("valid id"),
                    core_instance_id: self.log.core_instance_id(),
                    root: None,
                    display_name: "No workspace open".into(),
                },
                Vec::new(),
                None,
                FileTreeDto { root: None },
                selora_protocol::GitStatusDto {
                    branch: None,
                    changed_paths: Vec::new(),
                    staged_paths: Vec::new(),
                    files: Vec::new(),
                },
            ),
        };
        drop(workspace);
        let workflows = self
            .workspace_root_path()
            .map(|root| persisted_workflow_runs(&root))
            .unwrap_or_default();
        BootstrapState {
            workflows,
            protocol_version: PROTOCOL_VERSION,
            core_instance_id: self.log.core_instance_id(),
            snapshot_sequence: self.log.snapshot_sequence(),
            workspace: workspace_dto,
            buffers,
            active_buffer_id,
            file_tree,
            models: self.sessions.models_dto(),
            active_model_id: self.sessions.active_model_id(),
            connection: self.sessions.connection(),
            chat: self.sessions.chat_state(),
            settings: self
                .settings
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone(),
            // §1.1: the Z1 stub values (280/220, agent dock open) were
            // diagnostic placeholders — the layout now comes from the
            // persisted settings (§4).
            layout: self.layout.lock().expect("layout lock").clone(),
            // §5.1: the bounds ride the bootstrap so no bound is ever a JS
            // literal.
            layout_constraints: LayoutConstraintsDto::from_constants(),
            git,
            diagnostics: Vec::new(),
            feature_flags: selora_protocol::FeatureFlagsDto {
                agent: true,
                git: true,
                search: true,
                editor: true,
                later_slices: vec!["lsp-bridge".into(), "workspace-trust".into()],
            },
        }
    }
    /// The Z1-era envelope validation (§2.3 steps 1–3), unchanged, then the
    /// Z2 layout arms and the Z3 session arms. The response remains a full
    /// `BootstrapState` — used for error inspection, not state (§2.4).
    pub fn dispatch<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        envelope: RequestEnvelope<FrontendCommand>,
    ) -> Result<BootstrapState, ProtocolErrorDto> {
        if envelope.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolErrorDto {
                code: "protocol_version_mismatch".into(),
                message: "Unsupported protocol version.".into(),
                retryable: false,
            });
        }
        if envelope.core_instance_id.as_ref() != Some(&self.log.core_instance_id()) {
            return Err(ProtocolErrorDto {
                code: "stale_core_instance".into(),
                message: "Request a fresh bootstrap snapshot.".into(),
                retryable: true,
            });
        }
        // The Z3 session commands carry their session in the payload; the
        // envelope's session id is allowed for them (and validated against
        // the payload inside the engine). Every other command keeps the
        // Z1/Z2 rule: no session on the envelope.
        let session_scoped = matches!(
            envelope.payload,
            FrontendCommand::SendChat { .. }
                | FrontendCommand::StopChat { .. }
                | FrontendCommand::SendAgent { .. }
                | FrontendCommand::AgentDecision { .. }
        );
        // §2.3: a command may name the workspace it was composed against —
        // "no-workspace" before one is open, or the live id after. An
        // unknown id means the frontend is stale and must re-bootstrap.
        let current_workspace = self.current_workspace_id();
        let sent_workspace = envelope
            .workspace_id
            .as_ref()
            .map(selora_protocol::WorkspaceId::as_str);
        let workspace_ok = match (current_workspace.as_ref(), sent_workspace) {
            // No claim, or the pre-open placeholder: the command names the
            // workspace by being sent to this core instance.
            (_, None) | (_, Some("no-workspace")) => true,
            (Some(current), Some(sent)) => current.as_str() == sent,
            (None, Some(_)) => false,
        };
        if !workspace_ok || (envelope.session_id.is_some() && !session_scoped) {
            return Err(ProtocolErrorDto {
                code: "stale_workspace_or_session".into(),
                message: "Request a fresh bootstrap snapshot.".into(),
                retryable: true,
            });
        }
        let request_id = envelope.request_id.clone();
        match envelope.payload {
            FrontendCommand::GetBootstrapState => Ok(self.bootstrap()),
            // The session commands carry their own request identity (the
            // `request_id` field on the variant — stable across the
            // request's lifetime, the id `StopChat`/`AgentDecision` name
            // and every stream event echoes). The envelope's request id
            // stays transport-level: it correlates the invoke's rejection.
            FrontendCommand::SendChat {
                request_id: command_request,
                request: payload,
            } => {
                self.sessions.send_chat(
                    self.event_log(),
                    app.clone(),
                    envelope.session_id,
                    command_request,
                    payload,
                )?;
                Ok(self.bootstrap())
            }
            FrontendCommand::StopChat {
                request_id: command_request,
            } => {
                self.sessions
                    .stop_chat(&command_request, envelope.session_id)?;
                Ok(self.bootstrap())
            }
            FrontendCommand::SendAgent {
                request_id: command_request,
                request: payload,
            } => {
                self.sessions.send_agent(
                    self.event_log(),
                    app.clone(),
                    envelope.session_id,
                    command_request,
                    payload,
                )?;
                Ok(self.bootstrap())
            }
            FrontendCommand::AgentDecision {
                request_id: command_request,
                approval_id,
                decision,
            } => {
                self.sessions.agent_decision(
                    &command_request,
                    &approval_id,
                    decision,
                    envelope.session_id,
                )?;
                Ok(self.bootstrap())
            }
            FrontendCommand::ToggleSurface { surface } => {
                // §2.2: the region a surface acts on is determined in Rust,
                // never by the frontend. Every accepted toggle changes state,
                // so an event always follows.
                let layout = self.apply_toggle(surface);
                self.record_event(
                    app,
                    FrontendEvent::WorkspaceLayoutChanged(layout),
                    Some(request_id),
                );
                Ok(self.bootstrap())
            }
            FrontendCommand::ResizeRegion { region, size_px } => {
                // §2.3 item 5: finite out-of-range values are clamped, never
                // rejected. The error branch is a Rust-internal invariant
                // guard — unreachable from the wire.
                let clamped = clamp_region_size(region, size_px).map_err(|_| ProtocolErrorDto {
                    code: "invalid_region_size".into(),
                    message: "Region size must be a finite number.".into(),
                    retryable: false,
                })?;
                if let Some(layout) = self.apply_resize(region, clamped) {
                    self.record_event(
                        app,
                        FrontendEvent::WorkspaceLayoutChanged(layout),
                        Some(request_id),
                    );
                }
                // An idempotent drag end: success, no event (§2.3 item 5).
                Ok(self.bootstrap())
            }
            FrontendCommand::SaveLayout => {
                // §2.3 item 7: persistence failure → `layout_save_failed`
                // (retryable); the in-memory layout is unaffected.
                self.persist_layout().map_err(|error| ProtocolErrorDto {
                    code: "layout_save_failed".into(),
                    message: format!("Failed to save the layout: {error}"),
                    retryable: true,
                })?;
                Ok(self.bootstrap())
            }
            FrontendCommand::OpenExternal { url } => {
                // The About dialog's repository link and the update toast's
                // download link: a WebView anchor cannot navigate, so the
                // backend validates the host and hands the URL to the OS
                // default browser (`main.rs` registers the opener plugin).
                let target = validate_external_url(&url)
                    .map_err(ExternalUrlRejection::into_protocol_error)?;
                tauri_plugin_opener::open_url(target.as_str(), None::<&str>).map_err(|error| {
                    ProtocolErrorDto {
                        code: "external_open_failed".into(),
                        message: format!("Could not open the link: {error}"),
                        retryable: true,
                    }
                })?;
                Ok(self.bootstrap())
            }
            FrontendCommand::OpenFolder => {
                // The keyboard/palette entry: prefer the boot override, and
                // point the UI at the native picker otherwise.
                let root = std::env::var("SELORA_WORKSPACE_ROOT")
                    .ok()
                    .map(PathBuf::from)
                    .filter(|path| path.is_dir())
                    .ok_or_else(|| ProtocolErrorDto {
                        code: "use_folder_picker".into(),
                        message: "Choose a folder with the native picker.".into(),
                        retryable: false,
                    })?;
                self.attach_workspace(root).map_err(Self::workspace_error)?;
                let snapshot = self.bootstrap();
                self.record_event(
                    app,
                    FrontendEvent::BootstrapState(Box::new(snapshot.clone())),
                    Some(request_id),
                );
                Ok(snapshot)
            }
            FrontendCommand::OpenFolderAt { root } => {
                self.attach_workspace(PathBuf::from(root))
                    .map_err(Self::workspace_error)?;
                // The workspace id changed: emit the full snapshot so the
                // frontend's envelope context follows without a reload.
                let snapshot = self.bootstrap();
                self.record_event(
                    app,
                    FrontendEvent::BootstrapState(Box::new(snapshot.clone())),
                    Some(request_id),
                );
                Ok(snapshot)
            }
            FrontendCommand::RefreshTree => {
                let tree = self.with_workspace(|workspace| workspace.tree())?;
                self.record_event(app, FrontendEvent::FileTreeChanged(tree), Some(request_id));
                Ok(self.bootstrap())
            }
            FrontendCommand::RevealInExplorer { path } => {
                let root = self.workspace_root_path().ok_or_else(|| ProtocolErrorDto {
                    code: "no_workspace".into(),
                    message: "Open a folder first.".into(),
                    retryable: false,
                })?;
                reveal_in_file_explorer(&root.join(path.as_str()))?;
                Ok(self.bootstrap())
            }
            FrontendCommand::OpenFile { path } => {
                let buffer = self.with_workspace(|workspace| workspace.open_file(path.as_str()))?;
                self.record_event(
                    app,
                    FrontendEvent::BufferSnapshot(buffer.clone()),
                    Some(request_id),
                );
                if let Some(root) = self.workspace_root_path() {
                    let absolute = root.join(buffer.path.as_str());
                    self.with_lsp(app, |bridge| {
                        bridge.bind_buffer(absolute.clone(), buffer.buffer_id.clone());
                        bridge.open(absolute, buffer.text.clone());
                    });
                }
                Ok(self.bootstrap())
            }
            FrontendCommand::CloseBuffer { buffer_id } => {
                let buffer = self.with_workspace(|workspace| workspace.buffer(&buffer_id))?;
                let root = self.workspace_root_path();
                self.with_workspace(|workspace| workspace.close_buffer(&buffer_id))?;
                if let Some(root) = root {
                    let absolute = root.join(buffer.path.as_str());
                    self.with_lsp(app, |bridge| {
                        bridge.close(absolute.clone());
                        bridge.unbind_buffer(&absolute);
                    });
                }
                // Closing is the one buffer mutation with no per-buffer event:
                // there is no BufferClosed in the union, and the frontend
                // ignores a command's return value. Publish the resulting
                // bootstrap so the tab strip actually drops the buffer.
                let snapshot = self.bootstrap();
                self.record_event(
                    app,
                    FrontendEvent::BootstrapState(Box::new(snapshot.clone())),
                    Some(request_id),
                );
                Ok(snapshot)
            }
            FrontendCommand::SaveBuffer { buffer_id } => {
                let buffer = self.with_workspace(|workspace| workspace.save_buffer(&buffer_id))?;
                self.record_event(
                    app,
                    FrontendEvent::BufferSnapshot(buffer.clone()),
                    Some(request_id.clone()),
                );
                if let Some(root) = self.workspace_root_path() {
                    let absolute = root.join(buffer.path.as_str());
                    self.with_lsp(app, |bridge| bridge.save(absolute));
                }
                // A save writes the local text, so any external-change marker
                // is resolved. Clear the banner explicitly.
                if let Some(cleared) =
                    self.with_workspace(|workspace| Ok(workspace.take_conflict(&buffer_id)))?
                {
                    self.record_event(
                        app,
                        FrontendEvent::BufferConflictCleared(cleared),
                        Some(request_id),
                    );
                }
                Ok(self.bootstrap())
            }
            FrontendCommand::ReloadBufferFromDisk { buffer_id } => {
                let buffer =
                    self.with_workspace(|workspace| workspace.reload_buffer_from_disk(&buffer_id))?;
                if let Some(root) = self.workspace_root_path() {
                    let absolute = root.join(buffer.path.as_str());
                    self.with_lsp(app, |bridge| bridge.reload(absolute, buffer.text.clone()));
                }
                self.record_event(
                    app,
                    FrontendEvent::BufferSnapshot(buffer.clone()),
                    Some(request_id.clone()),
                );
                self.record_event(
                    app,
                    FrontendEvent::BufferConflictCleared(
                        selora_protocol::BufferConflictClearedDto {
                            buffer_id: buffer.buffer_id,
                            path: buffer.path,
                        },
                    ),
                    Some(request_id),
                );
                Ok(self.bootstrap())
            }
            FrontendCommand::KeepBufferEdits { buffer_id } => {
                let buffer =
                    self.with_workspace(|workspace| workspace.keep_buffer_edits(&buffer_id))?;
                self.record_event(
                    app,
                    FrontendEvent::BufferConflictCleared(
                        selora_protocol::BufferConflictClearedDto {
                            buffer_id: buffer.buffer_id,
                            path: buffer.path,
                        },
                    ),
                    Some(request_id),
                );
                Ok(self.bootstrap())
            }
            FrontendCommand::ApplyEditorChange {
                buffer_id,
                version,
                changes,
            } => {
                let changed = self.with_workspace(|workspace| {
                    workspace.apply_change(&buffer_id, version, &changes)
                })?;
                self.record_event(app, FrontendEvent::BufferChanged(changed), Some(request_id));
                if let Ok(snapshot) = self.with_workspace(|workspace| workspace.buffer(&buffer_id))
                {
                    if let Some(root) = self.workspace_root_path() {
                        let absolute = root.join(snapshot.path.as_str());
                        self.with_lsp(app, |bridge| bridge.change(absolute, snapshot.text.clone()));
                    }
                }
                Ok(self.bootstrap())
            }
            FrontendCommand::SetSelection {
                buffer_id,
                selection,
            } => {
                self.with_workspace(|workspace| workspace.set_selection(&buffer_id, selection))?;
                Ok(self.bootstrap())
            }
            FrontendCommand::HoverAt {
                buffer_id,
                position,
            } => {
                self.request_lsp(app, &buffer_id, |bridge, absolute| {
                    bridge.hover(absolute, position)
                });
                Ok(self.bootstrap())
            }
            FrontendCommand::DefinitionAt {
                buffer_id,
                position,
            } => {
                self.request_lsp(app, &buffer_id, |bridge, absolute| {
                    bridge.definition(absolute, position)
                });
                Ok(self.bootstrap())
            }
            FrontendCommand::CompletionAt {
                buffer_id,
                position,
            } => {
                self.request_lsp(app, &buffer_id, |bridge, absolute| {
                    bridge.completion(absolute, position)
                });
                Ok(self.bootstrap())
            }
            FrontendCommand::Search { query } => {
                let results = self.with_workspace(|workspace| workspace.search(&query))?;
                self.record_event(app, FrontendEvent::SearchResults(results), Some(request_id));
                Ok(self.bootstrap())
            }
            FrontendCommand::GitStage { path } => {
                let status = self.with_workspace(|workspace| workspace.git_stage(path.as_str()))?;
                self.record_event(
                    app,
                    FrontendEvent::GitStatusChanged(status),
                    Some(request_id),
                );
                Ok(self.bootstrap())
            }
            FrontendCommand::GitUnstage { path } => {
                let status =
                    self.with_workspace(|workspace| workspace.git_unstage(path.as_str()))?;
                self.record_event(
                    app,
                    FrontendEvent::GitStatusChanged(status),
                    Some(request_id),
                );
                Ok(self.bootstrap())
            }
            FrontendCommand::GitCommit { message } => {
                let status = self.with_workspace(|workspace| workspace.git_commit(&message))?;
                self.record_event(
                    app,
                    FrontendEvent::GitStatusChanged(status),
                    Some(request_id),
                );
                Ok(self.bootstrap())
            }
            FrontendCommand::OpenGitDiff { path } => {
                let diff = self.with_workspace(|workspace| workspace.git_diff(path.as_str()))?;
                self.record_event(app, FrontendEvent::GitDiffReady(diff), Some(request_id));
                Ok(self.bootstrap())
            }
            FrontendCommand::SettingsUpdate { patch } => {
                self.apply_settings_update(app, patch, Some(request_id))
            }
            // Phase 8 wave 2: dispatch only. The workflow commands carry
            // their own request id (the lifetime id `WorkflowStop` names and
            // every stream event echoes), like the session commands.
            FrontendCommand::WorkflowRun {
                request_id: command_request,
                spec,
            } => {
                self.start_workflow(app, command_request, spec)?;
                Ok(self.bootstrap())
            }
            FrontendCommand::WorkflowStop {
                request_id: command_request,
            } => {
                self.stop_workflow(&command_request)?;
                Ok(self.bootstrap())
            }
            FrontendCommand::WorkflowResume {
                request_id,
                run_id,
                allow_incomplete_asks,
            } => {
                let root = self.workspace_root_path().ok_or_else(|| ProtocolErrorDto {
                    code: "no_workspace".into(),
                    message: "Open the workflow's workspace before resuming it.".into(),
                    retryable: false,
                })?;
                let run_id = selora_workflow::RunId::from(run_id);
                let spec = selora_workflow::load_workflow_spec(&root, &run_id)
                    .map_err(workflow_persistence_error)?;
                let journal = selora_workflow::journal::Journal::load(&root, run_id.clone())
                    .map_err(|error| ProtocolErrorDto {
                        code: "workflow_resume_rejected".into(),
                        message: error.to_string(),
                        retryable: false,
                    })?;
                // Synchronous preflight: a cap that already fired is a terminal
                // state. Refuse before any host, worker, or event exists, and
                // never let a resume raise the confirmed budget silently.
                let terminal = workflow_terminal_status(&journal);
                if terminal == Some(selora_workflow::journal::RunStatus::BudgetExceeded) {
                    return Err(workflow_budget_exhausted_error(
                        spec.max_cost_usd,
                        journal.resume_plan().spent_usd(),
                    ));
                }
                let completed = terminal == Some(selora_workflow::journal::RunStatus::Completed);
                if completed {
                    return Err(ProtocolErrorDto {
                        code: "workflow_already_completed".into(),
                        message: "A completed workflow cannot be resumed or charged again.".into(),
                        retryable: false,
                    });
                }
                // Fail closed before a worker or event exists: a resume that
                // would dispatch an unfinished ask again may re-run tools whose
                // earlier effects already landed. The user's explicit
                // confirmation is the flag; never infer it.
                let pending = selora_workflow::incomplete_asks(&root, &run_id)
                    .map_err(workflow_persistence_error)?;
                if !pending.is_empty() && !allow_incomplete_asks {
                    return Err(ProtocolErrorDto {
                        code: "workflow_incomplete_asks".into(),
                        message: workflow_incomplete_asks_message(&pending),
                        retryable: false,
                    });
                }
                self.launch_workflow(
                    app,
                    request_id,
                    root,
                    spec,
                    run_id,
                    WorkflowLaunch::Resume {
                        allow_incomplete_asks,
                    },
                )?;
                Ok(self.bootstrap())
            }
            FrontendCommand::WorkflowDecision {
                request_id: command_request,
                approval_id,
                decision,
            } => {
                self.workflow_decision(&command_request, &approval_id, decision)?;
                Ok(self.bootstrap())
            }
            // Everything outside Z4 keeps the typed unsupported_phase_slice
            // error (the contract's "Explicit non-goals" list; adapter.rs).
            other => Err(crate::adapter::unsupported(&other)),
        }
    }

    // -----------------------------------------------------------------
    // Phase 8 wave 2 — WorkflowRun / WorkflowStop / WorkflowDecision
    // -----------------------------------------------------------------

    /// Validate a `WorkflowRun` spec, reserve its request id, and start the
    /// blocking engine on a worker thread.
    ///
    /// Policy: **concurrent runs are allowed** — every protocol event is
    /// `run_id`-scoped, so the panel can follow more than one graph — but a
    /// new run may not reuse a *live* request id. `WorkflowStop` and
    /// `WorkflowDecision` address a run by that id, so it has to stay unique
    /// while the run is active. (The engine's per-run cost cap already bounds
    /// each run; there is no global workflow budget in this slice.)
    fn start_workflow<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        request_id: RequestId,
        spec: WorkflowSpecDto,
    ) -> Result<(), ProtocolErrorDto> {
        let spec = build_workflow_spec(spec)?;
        let root = self.workspace_root_path().ok_or_else(|| ProtocolErrorDto {
            code: "no_workspace".into(),
            message: "A workflow needs a workspace root — open a folder first.".into(),
            retryable: false,
        })?;
        self.launch_workflow(
            app,
            request_id,
            root,
            spec,
            selora_workflow::RunId::new(),
            WorkflowLaunch::New,
        )
    }

    fn launch_workflow<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        request_id: RequestId,
        root: PathBuf,
        spec: selora_workflow::WorkflowSpec,
        durable_run_id: selora_workflow::RunId,
        launch: WorkflowLaunch,
    ) -> Result<(), ProtocolErrorDto> {
        let (resume, allow_incomplete_asks) = match launch {
            WorkflowLaunch::New => (false, false),
            WorkflowLaunch::Resume {
                allow_incomplete_asks,
            } => (true, allow_incomplete_asks),
        };
        let workspace_id = workspace_id_for(&root);
        let run_id = Arc::new(Mutex::new(Some(durable_run_id.to_string())));
        let stream = Arc::new(WorkflowStream {
            log: self.event_log(),
            app: app.clone(),
            request_id: request_id.clone(),
            workspace_id: workspace_id.clone(),
            workspace_root: root.clone(),
            workspace: Arc::clone(&self.workspace),
            lsp: Arc::clone(&self.lsp),
            max_cost_usd: spec.max_cost_usd,
            name: spec.name.clone(),
            started: AtomicBool::new(false),
            phase: Mutex::new(String::new()),
            run_id,
            finished: AtomicBool::new(false),
        });
        // Clone the factory out under the lock; the lock is never held while
        // the host is built or the worker is spawned.
        let factory = {
            let guard = self
                .workflow_hosts
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            Arc::clone(&*guard)
        };
        // A workflow can be pure files/git/commands, so a missing gateway key
        // does not block the run: the host's `ask` reports it at the failing
        // step and the run finishes `failed` with that error.
        let approval_stream = Arc::clone(&stream);
        let model_metadata = self.sessions.resolve_model(None).ok();
        let caps = self.sessions.workflow_turn_caps(
            model_metadata
                .as_ref()
                .map_or(0, |model| model.context_window),
        );
        let bundle = factory(
            root.clone(),
            WorkflowHostSettings {
                base_url: self.sessions.base_url(),
                api_key: self.sessions.fetch_api_key().unwrap_or_default(),
                model: self.sessions.active_model_id().unwrap_or_default(),
                model_metadata,
                caps,
                on_approval: Arc::new(move |id, card| approval_stream.on_approval(id, card)),
            },
        );
        let token = Uuid::new_v4();
        {
            let mut runs = self
                .workflow_runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if runs.contains_key(&request_id) {
                return Err(ProtocolErrorDto {
                    code: "workflow_run_already_active".into(),
                    message: "That request already owns a live workflow run.".into(),
                    retryable: false,
                });
            }
            if runs.values().any(|run| {
                run.run_id == durable_run_id.as_str() && run.workspace_id == workspace_id
            }) {
                return Err(ProtocolErrorDto {
                    code: "workflow_run_already_active".into(),
                    message: "That persisted workflow already has a live worker.".into(),
                    retryable: false,
                });
            }
            runs.insert(
                request_id.clone(),
                ActiveRun {
                    host: Arc::clone(&bundle.host),
                    cancel: bundle.cancel,
                    decide: bundle.decide,
                    run_id: durable_run_id.to_string(),
                    workspace_id,
                    token,
                },
            );
        }

        let sink_stream = Arc::clone(&stream);
        let sink: selora_workflow::EventSink = Arc::new(move |event| sink_stream.on_event(event));

        let runs = Arc::clone(&self.workflow_runs);
        tauri::async_runtime::spawn_blocking(move || {
            let host = bundle.host;
            let usage_host = Arc::clone(&host);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                selora_workflow::run_workflow_persisted(
                    &spec,
                    host,
                    sink,
                    selora_workflow::RunOptions {
                        workspace_root: root,
                        run_id: durable_run_id,
                        resume,
                        // Only a user-confirmed resume passes `true`; a new
                        // run and an unconfirmed resume keep the fail-closed
                        // default for unfinished asks.
                        allow_incomplete_asks,
                    },
                )
            }));
            match outcome {
                Ok(Ok(outcome)) => stream.finish_fallback(
                    workflow_run_status(outcome.report.status),
                    outcome.report.cost_usd,
                    outcome.report.tokens,
                    outcome.report.error,
                ),
                Ok(Err(error)) => {
                    // The engine emits `Finished` before returning a post-Run
                    // error, so the guard makes this a no-op on that path. It
                    // only fires for a pre-Run rejection (which the validation
                    // above should make unreachable).
                    let status = if matches!(error, selora_workflow::WorkflowError::Stopped) {
                        WorkflowStatus::Stopped
                    } else {
                        WorkflowStatus::Failed
                    };
                    stream.finish_fallback(
                        status,
                        usage_host.cost_usd(),
                        usage_host.tokens(),
                        Some(error.to_string()),
                    );
                }
                Err(_) => stream.finish_fallback(
                    WorkflowStatus::Failed,
                    usage_host.cost_usd(),
                    usage_host.tokens(),
                    Some("the workflow worker panicked".into()),
                ),
            }
            // Token-checked removal: a later run may already have reused the
            // request id once this one finished, and must not be evicted.
            let mut active = runs.lock().unwrap_or_else(|poison| poison.into_inner());
            if active
                .get(&request_id)
                .is_some_and(|run| run.token == token)
            {
                active.remove(&request_id);
            }
        });
        Ok(())
    }

    /// `WorkflowStop`: flip the active host's cancel switch. The engine then
    /// ends the script and emits its one `WorkflowFinished(stopped)`; the
    /// worker removes the entry. A finished/unknown run is a typed error, not
    /// silent success — the protocol's stop names a live run.
    fn stop_workflow(&self, request_id: &RequestId) -> Result<(), ProtocolErrorDto> {
        let cancel = {
            let runs = self
                .workflow_runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            runs.get(request_id).map(|run| Arc::clone(&run.cancel))
        };
        let Some(cancel) = cancel else {
            return Err(workflow_run_not_found());
        };
        cancel();
        Ok(())
    }

    /// `WorkflowDecision`: workflow approvals are the subagent cards of the
    /// next slice. Wave 2 has no approval slot on the engine host, so the
    /// decision is typed-unsupported and the run is left intact (the panel can
    /// still Stop it).
    fn workflow_decision(
        &self,
        request_id: &RequestId,
        approval_id: &ApprovalId,
        decision: DecisionDto,
    ) -> Result<(), ProtocolErrorDto> {
        if decision == DecisionDto::Stop {
            return self.stop_workflow(request_id);
        }
        let (decide, workspace_id) = {
            let runs = self
                .workflow_runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let run = runs.get(request_id).ok_or_else(workflow_run_not_found)?;
            (run.decide.clone(), run.workspace_id.clone())
        };
        if self.current_workspace_id().as_ref() != Some(&workspace_id) {
            return Err(ProtocolErrorDto {
                code: "workflow_workspace_changed".into(),
                message: "Return to the workflow's workspace before deciding its approval.".into(),
                retryable: false,
            });
        }
        let decide = decide.ok_or_else(|| ProtocolErrorDto {
            code: "workflow_approval_unavailable".into(),
            message: "This workflow host has no approval channel.".into(),
            retryable: false,
        })?;
        let id = Uuid::parse_str(approval_id.as_str()).map_err(|_| ProtocolErrorDto {
            code: "workflow_approval_not_found".into(),
            message: "No pending workflow approval matches that id.".into(),
            retryable: false,
        })?;
        let decision = match decision {
            DecisionDto::Approve => selora_ai::AgentDecision::Approve,
            DecisionDto::Deny => selora_ai::AgentDecision::Deny,
            DecisionDto::ApproveAllThisTurn => {
                return Err(ProtocolErrorDto {
                    code: "workflow_blanket_approval_rejected".into(),
                    message: "Approve each workflow side effect separately.".into(),
                    retryable: false,
                })
            }
            DecisionDto::Stop => unreachable!("handled before approval lookup"),
        };
        decide(id, decision).map_err(|error| ProtocolErrorDto {
            code: "workflow_approval_not_found".into(),
            message: error.to_string(),
            retryable: false,
        })
    }

    /// §2.2's routing table.
    fn apply_toggle(&self, surface: SurfaceId) -> LayoutDto {
        let mut layout = self.layout.lock().expect("layout lock");
        match surface {
            SurfaceId::Files
            | SurfaceId::Search
            | SurfaceId::Git
            | SurfaceId::Workflows
            | SurfaceId::Settings => {
                if layout.primary_sidebar_open && layout.activity_view == surface {
                    // Open and already showing this view → close the sidebar.
                    layout.primary_sidebar_open = false;
                } else {
                    layout.primary_sidebar_open = true;
                    layout.activity_view = surface;
                }
            }
            SurfaceId::Agent => {
                layout.agent_dock_open = !layout.agent_dock_open;
            }
            SurfaceId::Problems | SurfaceId::Output | SurfaceId::TerminalPlaceholder => {
                if layout.bottom_dock_open && layout.bottom_view == surface {
                    // Open and already showing this view → close the dock.
                    layout.bottom_dock_open = false;
                } else {
                    layout.bottom_dock_open = true;
                    layout.bottom_view = surface;
                }
            }
        }
        layout.clone()
    }

    /// Applies an accepted (already clamped) resize and returns the new
    /// layout, or `None` when the clamped value equals the stored value — an
    /// idempotent drag end that succeeds with no event (§2.3 item 5).
    /// Resizing a closed region is allowed: the size applies when it next
    /// opens. Sanitization runs after every accepted resize (§4.3) — with a
    /// clamped input and an already-sanitized stored layout it is
    /// idempotent, but it stands so a stale stored value can never survive
    /// an accepted resize.
    fn apply_resize(&self, region: RegionId, size_px: f32) -> Option<LayoutDto> {
        let mut layout = self.layout.lock().expect("layout lock");
        // §2.3 item 5: the no-op rule compares the clamped value against the
        // STORED value — before the mutation, not after it.
        let stored = match region {
            RegionId::PrimarySidebar => layout.primary_sidebar_width_px,
            RegionId::AgentDock => layout.agent_dock_width_px,
            RegionId::BottomDock => layout.bottom_dock_height_px,
        };
        if size_px == stored {
            return None;
        }
        match region {
            RegionId::PrimarySidebar => layout.primary_sidebar_width_px = size_px,
            RegionId::AgentDock => layout.agent_dock_width_px = size_px,
            RegionId::BottomDock => layout.bottom_dock_height_px = size_px,
        }
        *layout = layout.clone().sanitized();
        Some(layout.clone())
    }

    /// The one save path, shared by `SaveLayout` and the close-time save
    /// (§3.3): read-modify-write `settings.json` through the `LayoutStore`
    /// seam so every other section survives (§10), then the atomic
    /// temp+rename write (§4.1).
    pub fn persist_layout(&self) -> Result<(), selora_settings::SeloraError> {
        let layout = self.layout.lock().expect("layout lock").clone();
        let mut settings = self.layout_store.load_settings()?;
        self.layout_store.persist_layout(&mut settings, &layout)
    }

    /// The §2.4 emitter, now a delegation to the shared [`EventLog`] (Z3:
    /// the session relays record through the same seam). Runtime-generic
    /// so hermetic tests can drive it with `tauri::test`'s mock runtime
    /// while the Tauri command passes the real `AppHandle`.
    pub fn record_event<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        payload: FrontendEvent,
        request_id: Option<selora_protocol::RequestId>,
    ) {
        self.log.record(app, payload, request_id);
    }

    /// Record without emitting — the Z2 state tests' seam (the §4.4 boot
    /// toast path; the log's own `push` carries the session id).
    #[cfg(test)]
    fn push_event(
        &self,
        payload: FrontendEvent,
        request_id: Option<selora_protocol::RequestId>,
    ) -> EventEnvelope<FrontendEvent> {
        self.log.push(payload, request_id, None)
    }

    /// §3.4's replay backing store for the `get_events_after` command.
    pub fn events_after(&self, sequence: u64) -> Vec<EventEnvelope<FrontendEvent>> {
        self.log.events_after(sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use selora_protocol::{
        BufferSnapshotDto, EventEnvelope, FrontendCommand, RequestId, WorkflowSpecDto, WorkspaceId,
        WorkspacePath,
    };
    use selora_workflow::{CommandOutput, GitCommit, GrepMatch, Host, HostError};
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    /// A unique temp settings file per test — hermetic, never the real
    /// `%APPDATA%` profile (the settings crate's own test pattern).
    fn temp_settings_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "selora-desktop-state-test-{label}-{}",
            std::process::id()
        ))
    }

    /// A mock app handle so `dispatch`'s emit path runs under test
    /// (`tauri::test::mock_app` needs the tauri "test" dev feature).
    fn mock_app_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
        tauri::test::mock_app().handle().clone()
    }

    fn envelope(
        state: &DesktopState,
        command: FrontendCommand,
    ) -> RequestEnvelope<FrontendCommand> {
        RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            workspace_id: Some(WorkspaceId::new("no-workspace").expect("valid id")),
            core_instance_id: Some(state.core_instance_id()),
            request_id: RequestId::new("request-1").expect("valid id"),
            session_id: None,
            payload: command,
        }
    }

    fn toggle(surface: SurfaceId) -> FrontendCommand {
        FrontendCommand::ToggleSurface { surface }
    }

    /// The last recorded event's payload, for event-order assertions.
    fn last_event(state: &DesktopState) -> EventEnvelope<FrontendEvent> {
        state
            .events_after(0)
            .pop()
            .expect("at least one event recorded")
    }

    fn layout(state: &DesktopState) -> LayoutDto {
        state.layout.lock().expect("layout lock").clone()
    }

    // ------------------------------------------------------------------
    // §2.2's toggle routing table.
    // ------------------------------------------------------------------

    #[test]
    fn toggle_table_routes_each_surface() {
        let app = mock_app_handle();

        // Sidebar surfaces, from the default (open, activity_view = Files):
        // toggle(S) closes the sidebar when it already shows S, otherwise
        // switches to S; a closed sidebar opens on S. Three consecutive
        // toggles walk all three cases for every surface.
        for surface in [
            SurfaceId::Files,
            SurfaceId::Search,
            SurfaceId::Git,
            SurfaceId::Settings,
        ] {
            let state = DesktopState::with_settings(selora_settings::Settings::default());
            let step = |state: &DesktopState, expect_open: bool, note: &str| {
                state
                    .dispatch(&app, envelope(state, toggle(surface)))
                    .expect("toggle succeeds");
                let after = layout(state);
                assert_eq!(
                    after.primary_sidebar_open, expect_open,
                    "{surface:?}: {note}"
                );
                assert_eq!(
                    after.activity_view, surface,
                    "{surface:?}: {note} keeps the view"
                );
            };
            if surface == SurfaceId::Files {
                // (a) open + same view → closed.
                step(&state, false, "same-view open → closed");
                // (c) closed → open.
                step(&state, true, "closed → open");
                // (a) again.
                step(&state, false, "same-view open → closed (second lap)");
            } else {
                // (b) open + different view → switched.
                step(&state, true, "different view → switched");
                // (a) open + same view → closed.
                step(&state, false, "same-view open → closed");
                // (c) closed → open.
                step(&state, true, "closed → open");
            }
        }

        // Agent: flips the dock — from closed (the default) it opens, and
        // from open it closes.
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Agent)))
            .expect("toggle succeeds");
        assert!(layout(&state).agent_dock_open, "closed → open");

        let mut settings = selora_settings::Settings::default();
        settings.layout.agent_dock_open = true;
        let state = DesktopState::with_settings(settings);
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Agent)))
            .expect("toggle succeeds");
        assert!(!layout(&state).agent_dock_open, "open → closed");

        // Bottom-dock surfaces, from the default (closed, bottom_view =
        // Output): the same three cases.
        for surface in [
            SurfaceId::Problems,
            SurfaceId::Output,
            SurfaceId::TerminalPlaceholder,
        ] {
            let state = DesktopState::with_settings(selora_settings::Settings::default());
            let step = |state: &DesktopState, expect_open: bool, note: &str| {
                state
                    .dispatch(&app, envelope(state, toggle(surface)))
                    .expect("toggle succeeds");
                let after = layout(state);
                assert_eq!(after.bottom_dock_open, expect_open, "{surface:?}: {note}");
                assert_eq!(
                    after.bottom_view, surface,
                    "{surface:?}: {note} keeps the view"
                );
            };
            // (c) closed → open on this view.
            step(&state, true, "closed → open");
            // (a) open + same view → closed.
            step(&state, false, "same-view open → closed");
            // (c) closed → open.
            step(&state, true, "closed → open (second lap)");
        }
        // (b) open + different view → switched.
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Problems)))
            .expect("open problems");
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Output)))
            .expect("switch to output");
        let after = layout(&state);
        assert!(after.bottom_dock_open, "different view stays open");
        assert_eq!(after.bottom_view, SurfaceId::Output);
    }

    #[test]
    fn every_accepted_toggle_emits_layout_changed_with_request_id() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Agent)))
            .expect("toggle succeeds");
        let event = last_event(&state);
        assert_eq!(event.request_id, Some(RequestId::new("request-1").unwrap()));
        match event.payload {
            FrontendEvent::WorkspaceLayoutChanged(payload) => {
                assert_eq!(payload, layout(&state), "the event carries the new layout");
            }
            other => panic!("expected WorkspaceLayoutChanged, got {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // ResizeRegion: clamp + no-op + closed regions.
    // ------------------------------------------------------------------

    #[test]
    fn resize_clamps_into_bounds_and_emits() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let result = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ResizeRegion {
                        region: RegionId::PrimarySidebar,
                        size_px: 9999.0,
                    },
                ),
            )
            .expect("resize succeeds");
        assert_eq!(
            layout(&state).primary_sidebar_width_px,
            480.0,
            "clamped to max"
        );
        assert_eq!(
            result.layout.primary_sidebar_width_px, 480.0,
            "response carries it too"
        );
        match last_event(&state).payload {
            FrontendEvent::WorkspaceLayoutChanged(payload) => {
                assert_eq!(payload.primary_sidebar_width_px, 480.0);
            }
            other => panic!("expected WorkspaceLayoutChanged, got {other:?}"),
        }

        let state = DesktopState::with_settings(selora_settings::Settings::default());
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ResizeRegion {
                        region: RegionId::AgentDock,
                        size_px: 100.0,
                    },
                ),
            )
            .expect("resize succeeds");
        assert_eq!(layout(&state).agent_dock_width_px, 240.0, "clamped to min");

        let state = DesktopState::with_settings(selora_settings::Settings::default());
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ResizeRegion {
                        region: RegionId::BottomDock,
                        size_px: 0.0,
                    },
                ),
            )
            .expect("resize succeeds");
        assert_eq!(
            layout(&state).bottom_dock_height_px,
            120.0,
            "clamped to min"
        );
    }

    #[test]
    fn idempotent_resize_emits_no_event() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ResizeRegion {
                        region: RegionId::PrimarySidebar,
                        size_px: 480.0,
                    },
                ),
            )
            .expect("first resize succeeds");
        let events_before = state.events_after(0).len();
        assert_eq!(
            events_before, 1,
            "the changing resize emitted exactly one event"
        );

        // Same value → success, no event (idempotent drag end).
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ResizeRegion {
                        region: RegionId::PrimarySidebar,
                        size_px: 480.0,
                    },
                ),
            )
            .expect("no-op resize still succeeds");
        assert_eq!(
            state.events_after(0).len(),
            events_before,
            "an idempotent drag end emits nothing"
        );

        // A value that clamps to the stored one is equally a no-op.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ResizeRegion {
                        region: RegionId::PrimarySidebar,
                        size_px: 90_000.0,
                    },
                ),
            )
            .expect("clamped no-op succeeds");
        assert_eq!(state.events_after(0).len(), events_before);
        assert_eq!(layout(&state).primary_sidebar_width_px, 480.0);
    }

    #[test]
    fn resizing_a_closed_region_is_allowed() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        // Bottom dock is closed first-run; the size applies when it next opens.
        assert!(!layout(&state).bottom_dock_open);
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ResizeRegion {
                        region: RegionId::BottomDock,
                        size_px: 480.0,
                    },
                ),
            )
            .expect("resize succeeds");
        let after = layout(&state);
        assert!(!after.bottom_dock_open, "resize does not open the region");
        assert_eq!(after.bottom_dock_height_px, 480.0);
    }

    // ------------------------------------------------------------------
    // Envelope rejection (§2.3 steps 1–3) for the three Z2 commands.
    // ------------------------------------------------------------------

    #[test]
    fn z2_commands_reject_stale_envelopes() {
        let app = mock_app_handle();
        for command in [
            toggle(SurfaceId::Agent),
            FrontendCommand::ResizeRegion {
                region: RegionId::AgentDock,
                size_px: 300.0,
            },
            FrontendCommand::SaveLayout,
        ] {
            let state = DesktopState::with_settings(selora_settings::Settings::default());
            let base = envelope(&state, command.clone());

            // (1) protocol version mismatch.
            let mut bad = base.clone();
            bad.protocol_version = PROTOCOL_VERSION + 1;
            let error = state.dispatch(&app, bad).expect_err("version mismatch");
            assert_eq!(error.code, "protocol_version_mismatch");
            assert!(!error.retryable);

            // (2) stale core instance.
            let mut bad = base.clone();
            bad.core_instance_id = Some(CoreInstanceId::new("other-core").expect("valid id"));
            let error = state.dispatch(&app, bad).expect_err("stale core");
            assert_eq!(error.code, "stale_core_instance");
            assert!(error.retryable);

            // (3a) wrong workspace id.
            let mut bad = base.clone();
            bad.workspace_id = Some(WorkspaceId::new("a-real-workspace").expect("valid id"));
            let error = state.dispatch(&app, bad).expect_err("stale workspace");
            assert_eq!(error.code, "stale_workspace_or_session");

            // (3b) a session id present.
            let mut bad = base;
            bad.session_id = Some(selora_protocol::SessionId::new("session-1").expect("valid id"));
            let error = state.dispatch(&app, bad).expect_err("stale session");
            assert_eq!(error.code, "stale_workspace_or_session");

            // No mutation, no events on any rejection.
            assert_eq!(
                layout(&state),
                LayoutStore::load_layout(&selora_settings::Settings::default()),
                "rejections never mutate the layout"
            );
            assert!(
                state.events_after(0).is_empty(),
                "rejections never record events"
            );
        }
    }

    #[test]
    fn commands_outside_z4_stay_unsupported() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::FindInBuffer {
                        buffer_id: selora_protocol::BufferId::new("buf_test").expect("valid id"),
                        query: "fn".into(),
                        options: selora_protocol::FindOptionsDto::default(),
                    },
                ),
            )
            .expect_err("in-buffer find is not a Z4 protocol command");
        assert_eq!(error.code, "unsupported_phase_slice");
        assert!(!error.retryable);
        assert!(state.events_after(0).is_empty());
    }

    #[test]
    fn settings_update_round_trips_to_disk_and_bootstrap_event() {
        let app = mock_app_handle();
        let path = temp_settings_path("settings-update-round-trip");
        let _ = std::fs::remove_file(&path);
        let state = DesktopState::from_settings_path(path.clone());

        let returned = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::SettingsUpdate {
                        patch: SettingsPatchDto {
                            reduced_motion: Some(true),
                            theme: Some("light".into()),
                        },
                    },
                ),
            )
            .expect("settings update succeeds");

        assert!(returned.settings.reduced_motion);
        assert_eq!(returned.settings.theme, "light");
        assert!(state.bootstrap().settings.reduced_motion);
        assert_eq!(state.bootstrap().settings.theme, "light");

        let published = state
            .events_after(0)
            .into_iter()
            .find_map(|event| match event.payload {
                FrontendEvent::BootstrapState(snapshot) => Some(snapshot.settings),
                _ => None,
            })
            .expect("the refreshed bootstrap is published");
        assert!(published.reduced_motion);
        assert_eq!(published.theme, "light");

        let persisted = selora_settings::Settings::load_from(&path).expect("reload settings");
        assert!(persisted.ui.reduce_motion);
        assert_eq!(persisted.ui.theme, "light");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn settings_update_rejects_invalid_theme_without_mutation() {
        let app = mock_app_handle();
        let path = temp_settings_path("settings-update-invalid-theme");
        let _ = std::fs::remove_file(&path);
        let state = DesktopState::from_settings_path(path.clone());
        let before = state.bootstrap().settings;

        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::SettingsUpdate {
                        patch: SettingsPatchDto {
                            reduced_motion: Some(true),
                            theme: Some("not-a-theme".into()),
                        },
                    },
                ),
            )
            .expect_err("an unknown theme is rejected");

        assert_eq!(error.code, "invalid_settings_theme");
        assert!(!error.retryable);
        assert_eq!(state.bootstrap().settings, before, "memory is unchanged");
        assert!(state.events_after(0).is_empty(), "no event is published");
        assert!(!path.exists(), "invalid input never writes settings.json");
    }

    // ------------------------------------------------------------------
    // OpenExternal (About dialog + update toast): the pure allowlist check
    // and the typed rejection. No test reaches the process spawn.
    // ------------------------------------------------------------------

    #[test]
    fn open_external_allowlist_accepts_github_and_rejects_dangerous_urls() {
        assert_eq!(
            validate_external_url("https://github.com/ToxicAngelX/Selora-IDE")
                .expect("the repository link is allowlisted"),
            "https://github.com/ToxicAngelX/Selora-IDE"
        );
        assert_eq!(
            validate_external_url(
                "https://api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest"
            )
            .expect("the Releases API link is allowlisted"),
            "https://api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest"
        );

        assert_eq!(
            validate_external_url("javascript:alert(1)"),
            Err(ExternalUrlRejection::Scheme)
        );
        assert_eq!(
            validate_external_url("file:///C:/Windows"),
            Err(ExternalUrlRejection::Scheme)
        );
        assert_eq!(
            validate_external_url("data:text/html,<script>alert(1)</script>"),
            Err(ExternalUrlRejection::Scheme)
        );
        assert_eq!(
            validate_external_url("https://evil.example"),
            Err(ExternalUrlRejection::HostNotAllowed)
        );
        // A userinfo trick and a look-alike sibling host both fail.
        assert_eq!(
            validate_external_url("https://github.com@evil.example/"),
            Err(ExternalUrlRejection::Credentials)
        );
        assert_eq!(
            validate_external_url("https://github.com.evil.example/"),
            Err(ExternalUrlRejection::HostNotAllowed)
        );
        // UNC paths and backslash escapes never reach the URL parser.
        assert_eq!(
            validate_external_url(r"\\fileserver\share\evil.exe"),
            Err(ExternalUrlRejection::Backslash)
        );
        assert_eq!(
            validate_external_url("https://github.com\\@evil.example/"),
            Err(ExternalUrlRejection::Backslash)
        );
    }

    #[test]
    fn open_external_rejections_surface_as_typed_errors() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        for url in [
            "javascript:alert(1)",
            "file:///C:/Windows",
            "data:text/html,<script>alert(1)</script>",
            "https://evil.example",
            "https://github.com.evil.example/",
        ] {
            let error = state
                .dispatch(
                    &app,
                    envelope(&state, FrontendCommand::OpenExternal { url: url.into() }),
                )
                .expect_err("a rejected link never opens");
            assert_eq!(error.code, "external_url_rejected", "for {url}");
            assert!(!error.retryable);
        }
        assert!(
            state.events_after(0).is_empty(),
            "rejections never record events"
        );
    }

    // ------------------------------------------------------------------
    // Persistence: round-trip, bootstrap reconstruction, close-time save,
    // failure mapping.
    // ------------------------------------------------------------------

    #[test]
    fn dispatch_persists_round_trip_and_bootstrap_reconstruction() {
        let app = mock_app_handle();
        let path = temp_settings_path("round-trip");
        let _ = std::fs::remove_file(&path);

        // First run: defaults on disk absent → bootstrap carries §1.1 defaults.
        let state = DesktopState::from_settings_path(path.clone());
        assert_eq!(
            state.bootstrap().layout,
            LayoutDto::from(LayoutSettings::default())
        );

        // Mutate through the Z2 commands, then SaveLayout.
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Search)))
            .expect("toggle succeeds");
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ResizeRegion {
                        region: RegionId::AgentDock,
                        size_px: 512.0,
                    },
                ),
            )
            .expect("resize succeeds");
        state
            .dispatch(&app, envelope(&state, FrontendCommand::SaveLayout))
            .expect("save succeeds");

        // Bootstrap reconstruction after a save: a fresh state over the same
        // file reflects exactly what was saved.
        let reopened = DesktopState::from_settings_path(path.clone());
        let persisted = layout(&state);
        assert_eq!(reopened.bootstrap().layout, persisted);
        assert_eq!(persisted.agent_dock_width_px, 512.0);
        assert!(persisted.primary_sidebar_open);
        assert_eq!(persisted.activity_view, SurfaceId::Search);
        // ...and the other settings sections survive the layout save.
        let settings = LayoutStore::new(path.clone())
            .load_settings()
            .expect("settings reload");
        assert_eq!(settings.keybinds.toggle_chat, "Ctrl+L");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn close_time_persist_function_saves_the_live_layout() {
        let app = mock_app_handle();
        let path = temp_settings_path("close-persist");
        let _ = std::fs::remove_file(&path);

        let state = DesktopState::from_settings_path(path.clone());
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Agent)))
            .expect("toggle succeeds");
        // No SaveLayout — the close-time save (§3.3) owns this write.
        state.persist_layout().expect("close-time persist");

        let reopened = DesktopState::from_settings_path(path.clone());
        assert!(
            reopened.bootstrap().layout.agent_dock_open,
            "the live layout was saved"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_failure_maps_to_layout_save_failed_and_keeps_memory() {
        let app = mock_app_handle();
        // A settings path whose parent is a regular file: every write via
        // `save_to` fails at `create_dir_all`, deterministically.
        let parent = temp_settings_path("save-fail-parent");
        let _ = std::fs::remove_file(&parent);
        std::fs::write(&parent, b"i am a file, not a directory").expect("write blocker file");
        let settings_path = parent.join("settings.json");

        let state = DesktopState::from_settings_path(settings_path.clone());
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Agent)))
            .expect("toggle succeeds");
        let before = layout(&state);

        let error = state
            .dispatch(&app, envelope(&state, FrontendCommand::SaveLayout))
            .expect_err("SaveLayout fails on an unwritable path");
        assert_eq!(error.code, "layout_save_failed");
        assert!(
            error.retryable,
            "the caller may retry after fixing the cause"
        );
        // §2.3 item 7: the in-memory layout is unaffected.
        assert_eq!(layout(&state), before);
        assert!(!settings_path.exists(), "nothing was written");

        let _ = std::fs::remove_file(&parent);
    }

    #[test]
    fn persisted_out_of_range_values_are_sanitized_on_load() {
        let path = temp_settings_path("sanitize-on-load");
        let _ = std::fs::remove_file(&path);
        let mut settings = selora_settings::Settings::default();
        settings.layout.primary_sidebar_width_px = 9999.0;
        settings.layout.bottom_view = SurfaceId::Agent; // not a bottom view
        settings.layout.editor_splits = 0;
        settings.save_to(&path).expect("save wedged layout");

        let state = DesktopState::from_settings_path(path.clone());
        let boot = state.bootstrap().layout;
        assert_eq!(
            boot.primary_sidebar_width_px, 480.0,
            "clamped to max on load"
        );
        assert_eq!(
            boot.bottom_view,
            SurfaceId::Output,
            "reset into the valid set"
        );
        assert_eq!(boot.editor_splits, 1, "clamped to the split floor");

        let _ = std::fs::remove_file(&path);
    }

    // ------------------------------------------------------------------
    // Event log: ordering, retention cap, boot toast.
    // ------------------------------------------------------------------

    #[test]
    fn events_are_recorded_in_dispatch_order() {
        let app = mock_app_handle();
        let state = DesktopState::from_settings_path(temp_settings_path("ordering"));
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Agent)))
            .expect("toggle 1");
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ResizeRegion {
                        region: RegionId::PrimarySidebar,
                        size_px: 320.0,
                    },
                ),
            )
            .expect("resize");
        state
            .dispatch(&app, envelope(&state, toggle(SurfaceId::Output)))
            .expect("toggle 2");

        let events = state.events_after(0);
        assert_eq!(events.len(), 3);
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3],
            "sequences are monotonic in dispatch order"
        );
        // Every dispatch here emits, and each event carries the request_id of
        // the command that caused it (§2.4).
        for event in &events {
            assert_eq!(
                event.request_id,
                Some(RequestId::new("request-1").expect("valid id"))
            );
            assert!(matches!(
                event.payload,
                FrontendEvent::WorkspaceLayoutChanged(_)
            ));
        }
        // The final recorded layout is the last event's layout — rapid
        // successive commands converge on the last one (§3.2).
        match &events.last().expect("three events").payload {
            FrontendEvent::WorkspaceLayoutChanged(payload) => {
                assert_eq!(payload.bottom_view, SurfaceId::Output);
                assert_eq!(payload, &layout(&state));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn event_retention_cap_keeps_the_most_recent_1024() {
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let total = EVENT_RETENTION_CAP + 76;
        for i in 0..u64::try_from(total).expect("fits") {
            state.push_event(
                FrontendEvent::Toast(ToastDto {
                    severity: ToastSeverity::Info,
                    message: format!("event {i}"),
                }),
                None,
            );
        }
        let retained = state.events_after(0);
        assert_eq!(
            retained.len(),
            EVENT_RETENTION_CAP,
            "exactly the cap retained"
        );
        let first = retained.first().expect("non-empty").sequence;
        let last = retained.last().expect("non-empty").sequence;
        assert_eq!(
            first,
            u64::try_from(total - EVENT_RETENTION_CAP + 1).expect("fits")
        );
        assert_eq!(last, u64::try_from(total).expect("fits"));
        // The drop is prefix-only: sequences stay strictly increasing.
        assert!(retained
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence));
    }

    #[test]
    fn settings_load_error_is_recorded_as_the_boot_toast() {
        let path = temp_settings_path("boot-toast");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, br#"{"_v":2,"editor":{"font_size":99}}"#).expect("write v2 file");

        let state = DesktopState::from_settings_path(path.clone());
        let events = state.events_after(0);
        assert_eq!(events.len(), 1, "exactly the boot toast");
        match &events[0].payload {
            FrontendEvent::Toast(toast) => {
                assert_eq!(toast.severity, ToastSeverity::Error);
                assert!(!toast.message.is_empty());
            }
            other => panic!("expected a toast, got {other:?}"),
        }
        // §3.4: recorded during construction, so its sequence is ≤ the
        // bootstrap's snapshot_sequence — the replay flow delivers it.
        assert_eq!(events[0].sequence, 1);
        assert!(state.bootstrap().snapshot_sequence >= events[0].sequence);
        assert_eq!(events[0].request_id, None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clean_settings_load_records_no_boot_toast() {
        let path = temp_settings_path("no-boot-toast");
        let _ = std::fs::remove_file(&path);
        let state = DesktopState::from_settings_path(path.clone());
        assert!(state.events_after(0).is_empty());
        assert_eq!(state.bootstrap().snapshot_sequence, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bootstrap_carries_constraints_and_keybinds() {
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let boot = state.bootstrap();
        assert_eq!(
            boot.layout_constraints,
            LayoutConstraintsDto::from_constants()
        );
        assert_eq!(boot.settings.keybind_command_palette, "Ctrl+Shift+P");
        assert_eq!(boot.settings.keybind_toggle_chat, "Ctrl+L");
        // A user's egui-side rebind carries into the shell (§5.1).
        let mut settings = selora_settings::Settings::default();
        settings.keybinds.toggle_chat = "Ctrl+K".into();
        let state = DesktopState::with_settings(settings);
        assert_eq!(state.bootstrap().settings.keybind_toggle_chat, "Ctrl+K");
    }

    #[test]
    fn layout_store_routes_through_one_settings_path() {
        let path = temp_settings_path("store-path");
        let _ = std::fs::remove_file(&path);
        let store = LayoutStore::new(path.clone());
        let mut settings = store.load_settings().expect("first run = defaults");
        let layout = LayoutStore::load_layout(&settings);
        store
            .persist_layout(&mut settings, &layout)
            .expect("save through the seam");
        assert!(path.is_file(), "the store writes to exactly its own path");
        let reloaded = store.load_settings().expect("reload");
        assert_eq!(reloaded.layout, settings.layout);
        let _ = std::fs::remove_file(&path);
    }
    // ------------------------------------------------------------------
    // Phase Z4: the workspace service through the real dispatch path.
    // ------------------------------------------------------------------

    fn seed_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("hello.rs"), "fn main() {}\n").expect("seed");
        std::fs::create_dir(dir.path().join("src")).expect("dir");
        std::fs::write(
            dir.path().join("src").join("lib.rs"),
            "pub fn add(a: i32) -> i32 { a }\n",
        )
        .expect("seed lib");
        dir
    }

    fn open_workspace(
        state: &DesktopState,
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
    ) -> tempfile::TempDir {
        let dir = seed_workspace();
        state
            .dispatch(
                app,
                envelope(
                    state,
                    FrontendCommand::OpenFolderAt {
                        root: dir.path().to_string_lossy().to_string(),
                    },
                ),
            )
            .expect("open folder");
        dir
    }

    #[test]
    fn z4_open_folder_publishes_tree_and_bootstrap() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let bootstrap = state.bootstrap();
        assert_ne!(bootstrap.workspace.id.as_str(), "no-workspace");
        assert!(bootstrap.workspace.root.is_some());
        assert_eq!(
            bootstrap.workspace.display_name,
            dir.path()
                .file_name()
                .expect("tempdir name")
                .to_string_lossy()
                .to_string()
        );
        assert!(bootstrap.file_tree.root.is_some(), "the tree is real");
        assert!(bootstrap.feature_flags.editor && bootstrap.feature_flags.git);
        let events = state.events_after(0);
        assert!(events
            .iter()
            .any(|event| matches!(event.payload, FrontendEvent::BootstrapState(_))));
    }

    #[test]
    fn z4_close_buffer_publishes_a_bootstrap_event() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let opened = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFile {
                        path: selora_protocol::WorkspacePath::new("hello.rs").expect("path"),
                    },
                ),
            )
            .expect("open file");
        let buffer_id = opened.buffers[0].buffer_id.clone();
        let before = state.events_after(0).len();

        let after = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::CloseBuffer {
                        buffer_id: buffer_id.clone(),
                    },
                ),
            )
            .expect("close buffer");
        assert!(
            after
                .buffers
                .iter()
                .all(|buffer| buffer.buffer_id.as_str() != buffer_id.as_str()),
            "the returned bootstrap no longer lists the buffer"
        );

        let events = state.events_after(before as u64);
        let published = events.iter().any(|event| match &event.payload {
            FrontendEvent::BootstrapState(snapshot) => snapshot
                .buffers
                .iter()
                .all(|buffer| buffer.buffer_id.as_str() != buffer_id.as_str()),
            _ => false,
        });
        assert!(
            published,
            "the close emits a bootstrap the frontend can apply"
        );
    }

    #[test]
    fn z4_open_edit_save_round_trip() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFile {
                        path: selora_protocol::WorkspacePath::new("hello.rs").expect("path"),
                    },
                ),
            )
            .expect("open file");
        let buffer = opened
            .buffers
            .iter()
            .find(|buffer| buffer.path.as_str() == "hello.rs")
            .expect("buffer present")
            .clone();
        assert_eq!(buffer.text, "fn main() {}\n");
        assert_eq!(buffer.language, "rust");
        assert!(!buffer.dirty);

        let edited = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ApplyEditorChange {
                        buffer_id: buffer.buffer_id.clone(),
                        version: buffer.version,
                        changes: vec![selora_protocol::TextChange {
                            start: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 3,
                            },
                            end: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 7,
                            },
                            replacement: "boot".into(),
                        }],
                    },
                ),
            )
            .expect("apply change");
        let changed = edited
            .buffers
            .iter()
            .find(|candidate| candidate.buffer_id == buffer.buffer_id)
            .expect("buffer");
        assert_eq!(changed.text, "fn boot() {}\n");
        assert!(changed.dirty);
        assert_eq!(changed.version, buffer.version + 1);

        let saved = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::SaveBuffer {
                        buffer_id: buffer.buffer_id.clone(),
                    },
                ),
            )
            .expect("save");
        assert!(!saved.buffers[0].dirty);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("hello.rs")).expect("read"),
            "fn boot() {}\n",
            "the change reached the disk"
        );
        assert!(state
            .events_after(0)
            .iter()
            .any(|event| matches!(event.payload, FrontendEvent::BufferChanged(_))));
    }

    #[test]
    fn z4_multi_change_edit_round_trips_pre_edit_coordinates() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        std::fs::write(dir.path().join("multi.txt"), "alpha\nbeta\ngamma").expect("seed multi");
        let opened = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFile {
                        path: selora_protocol::WorkspacePath::new("multi.txt").expect("path"),
                    },
                ),
            )
            .expect("open multi");
        let buffer = opened
            .buffers
            .iter()
            .find(|candidate| candidate.path.as_str() == "multi.txt")
            .expect("buffer present")
            .clone();
        assert_eq!(buffer.version, 1);

        // Two selections from one CodeMirror transaction: both ranges are
        // expressed in the pre-edit document ("alpha" and "gamma").
        let edited = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ApplyEditorChange {
                        buffer_id: buffer.buffer_id.clone(),
                        version: buffer.version,
                        changes: vec![
                            selora_protocol::TextChange {
                                start: selora_protocol::TextPosition {
                                    line: 0,
                                    utf16_column: 0,
                                },
                                end: selora_protocol::TextPosition {
                                    line: 0,
                                    utf16_column: 5,
                                },
                                replacement: "one\ntwo".into(),
                            },
                            selora_protocol::TextChange {
                                start: selora_protocol::TextPosition {
                                    line: 2,
                                    utf16_column: 0,
                                },
                                end: selora_protocol::TextPosition {
                                    line: 2,
                                    utf16_column: 5,
                                },
                                replacement: "omega".into(),
                            },
                        ],
                    },
                ),
            )
            .expect("apply multi change");
        let changed = edited
            .buffers
            .iter()
            .find(|candidate| candidate.buffer_id == buffer.buffer_id)
            .expect("buffer after edit");
        assert_eq!(changed.text, "one\ntwo\nbeta\nomega");
        assert_eq!(changed.version, 2);
        assert!(changed.dirty);

        let echoed = state
            .events_after(0)
            .into_iter()
            .find_map(|event| match event.payload {
                FrontendEvent::BufferChanged(changed) => Some(changed),
                _ => None,
            })
            .expect("buffer_changed event");
        assert_eq!(echoed.version, 2);
        assert_eq!(echoed.changes.len(), 2);

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::SaveBuffer {
                        buffer_id: buffer.buffer_id.clone(),
                    },
                ),
            )
            .expect("save");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("multi.txt")).expect("read"),
            "one\ntwo\nbeta\nomega",
            "the full multi-selection edit reached the disk"
        );
    }

    #[test]
    fn z4_stale_version_is_rejected_but_dirty_state_survives() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let opened = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFile {
                        path: selora_protocol::WorkspacePath::new("hello.rs").expect("path"),
                    },
                ),
            )
            .expect("open file");
        let buffer_id = opened.buffers[0].buffer_id.clone();
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ApplyEditorChange {
                        buffer_id,
                        version: 999,
                        changes: Vec::new(),
                    },
                ),
            )
            .expect_err("stale version");
        assert_eq!(error.code, "stale_buffer_version");
        assert!(error.retryable);
    }

    #[test]
    fn z4_search_returns_real_hits() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::Search {
                        query: selora_protocol::SearchQueryDto {
                            query: "pub fn add".into(),
                            case_sensitive: false,
                            whole_word: false,
                            use_regex: false,
                        },
                    },
                ),
            )
            .expect("search");
        let results = state
            .events_after(0)
            .into_iter()
            .find_map(|event| match event.payload {
                FrontendEvent::SearchResults(results) => Some(results),
                _ => None,
            })
            .expect("search results event");
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].path.as_str(), "src/lib.rs");
    }

    #[test]
    fn z4_workspace_commands_without_a_folder_are_typed() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::OpenFile {
                        path: selora_protocol::WorkspacePath::new("hello.rs").expect("path"),
                    },
                ),
            )
            .expect_err("no workspace");
        assert_eq!(error.code, "no_workspace");
        assert!(!error.retryable);
    }

    // ------------------------------------------------------------------
    // Phase 8 wave 2 — workflow dispatch and event mapping.
    //
    // The engine/facade contract is `selora-workflow`'s own test suite; these
    // tests prove the desktop seam: validation before spawn, the event mapping
    // (run -> phase/step -> finished), the exactly-once terminal event, the
    // stop/decision addressing, and the concurrency policy.
    // ------------------------------------------------------------------

    /// A test-controlled park for `world.run`, so a run can be held live while
    /// the policy test dispatches more commands.
    #[derive(Default)]
    struct TestGate {
        open: Mutex<bool>,
        changed: std::sync::Condvar,
    }

    impl TestGate {
        fn wait(&self) {
            let mut open = self
                .open
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            while !*open {
                open = self
                    .changed
                    .wait(open)
                    .unwrap_or_else(|poison| poison.into_inner());
            }
        }

        fn open(&self) {
            *self
                .open
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = true;
            self.changed.notify_all();
        }
    }

    /// The dispatch-side fake: a working `Host` whose gates are controlled by
    /// the test (no gateway, no workspace writes).
    #[derive(Default)]
    struct FakeWorkflowHost {
        cancelled: AtomicBool,
        gate: Option<Arc<TestGate>>,
        logs: Mutex<Vec<String>>,
        world_calls: std::sync::atomic::AtomicU64,
        /// Make `ask` fail so a durable run can end with an unfinished ask
        /// (`incomplete_asks` then sees it).
        ask_fails: bool,
        /// Disk edits the fake performs when `world.run` starts: `Some` writes,
        /// `None` deletes. This is how a test makes a "workflow edits disk"
        /// happen while the tab is open.
        writes: Vec<(PathBuf, Option<Vec<u8>>)>,
        /// Extra dollars charged per host call after the baseline 0.25. Zero
        /// keeps the historical constant-spend fake; a positive value lets a
        /// test cross a workflow cost cap between steps.
        cost_per_call: f64,
        /// Host calls billed so far (`cost_per_call` multiplier).
        charged: std::sync::atomic::AtomicU64,
    }

    impl FakeWorkflowHost {
        fn with_gate(gate: Arc<TestGate>) -> Self {
            Self {
                gate: Some(gate),
                ..Self::default()
            }
        }

        fn with_failing_ask() -> Self {
            Self {
                ask_fails: true,
                ..Self::default()
            }
        }

        fn with_writes(writes: Vec<(PathBuf, Option<Vec<u8>>)>) -> Self {
            Self {
                writes,
                ..Self::default()
            }
        }

        fn with_cost_per_call(cost_per_call: f64) -> Self {
            Self {
                cost_per_call,
                ..Self::default()
            }
        }

        fn with_gate_and_writes(
            gate: Arc<TestGate>,
            writes: Vec<(PathBuf, Option<Vec<u8>>)>,
        ) -> Self {
            Self {
                gate: Some(gate),
                writes,
                ..Self::default()
            }
        }
    }

    impl Host for FakeWorkflowHost {
        fn ask(
            &self,
            _agent: &str,
            _persona: Option<&str>,
            _instructions: &str,
        ) -> Result<serde_json::Value, HostError> {
            self.charged.fetch_add(1, Ordering::SeqCst);
            if self.ask_fails {
                return Err(HostError::new("workflow ask failed"));
            }
            Ok(serde_json::json!({ "ok": true }))
        }

        fn world_run(
            &self,
            _cmd: &str,
            _args: &[String],
            _timeout_ms: Option<u64>,
        ) -> Result<CommandOutput, HostError> {
            self.world_calls.fetch_add(1, Ordering::SeqCst);
            self.charged.fetch_add(1, Ordering::SeqCst);
            for (path, contents) in &self.writes {
                match contents {
                    Some(bytes) => {
                        std::fs::write(path, bytes).expect("fake workflow disk write");
                    }
                    None => {
                        std::fs::remove_file(path).expect("fake workflow disk delete");
                    }
                }
            }
            if let Some(gate) = &self.gate {
                gate.wait();
            }
            Ok(CommandOutput {
                exit_code: 0,
                stdout: "ok".into(),
                stderr: String::new(),
            })
        }

        fn files_glob(&self, _pattern: &str) -> Result<Vec<String>, HostError> {
            Ok(Vec::new())
        }

        fn files_grep(
            &self,
            _pattern: &str,
            _glob: Option<&str>,
        ) -> Result<Vec<GrepMatch>, HostError> {
            Ok(Vec::new())
        }

        fn files_read(&self, path: &str) -> Result<String, HostError> {
            Ok(format!("// {path}"))
        }

        fn git_changed_files(&self, _base: Option<&str>) -> Result<Vec<String>, HostError> {
            Ok(Vec::new())
        }

        fn git_diff(&self, _base: Option<&str>, _path: Option<&str>) -> Result<String, HostError> {
            Ok(String::new())
        }

        fn git_status(&self) -> Result<String, HostError> {
            Ok("clean".into())
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

        fn artifact_file(
            &self,
            _id: &str,
            _title: Option<&str>,
            _path: &str,
        ) -> Result<(), HostError> {
            Ok(())
        }

        fn log(&self, message: &str) {
            self.logs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(message.to_owned());
        }

        fn cost_usd(&self) -> f64 {
            0.25 + self.cost_per_call * self.charged.load(Ordering::SeqCst) as f64
        }

        fn tokens(&self) -> u64 {
            128
        }

        fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::SeqCst)
        }
    }

    fn fake_workflow_factory(host: Arc<FakeWorkflowHost>) -> WorkflowHostFactory {
        Arc::new(move |_root, _settings| {
            let cancel_host = Arc::clone(&host);
            WorkflowHostBundle {
                host: Arc::clone(&host) as Arc<dyn Host>,
                cancel: Arc::new(move || {
                    cancel_host.cancelled.store(true, Ordering::SeqCst);
                }),
                decide: None,
            }
        })
    }

    const WORKFLOW_TEST_TIMEOUT: Duration = Duration::from_secs(10);

    fn wait_for_workflow(
        state: &DesktopState,
        predicate: impl Fn(&FrontendEvent) -> bool,
    ) -> Vec<EventEnvelope<FrontendEvent>> {
        let deadline = Instant::now() + WORKFLOW_TEST_TIMEOUT;
        loop {
            let events = state.events_after(0);
            if events.iter().any(|event| predicate(&event.payload)) {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for a workflow event; recorded so far:\n{events:#?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn finished_count(events: &[EventEnvelope<FrontendEvent>]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event.payload, FrontendEvent::WorkflowFinished(_)))
            .count()
    }

    fn wait_for_finished_runs(
        state: &DesktopState,
        count: usize,
    ) -> Vec<EventEnvelope<FrontendEvent>> {
        let deadline = Instant::now() + WORKFLOW_TEST_TIMEOUT;
        loop {
            let events = state.events_after(0);
            if finished_count(&events) >= count {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {count} finished runs; recorded so far:\n{events:#?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The worker removes its map entry just after the terminal event; poll so
    /// the assertion does not race that cleanup.
    fn wait_for_no_active_runs(state: &DesktopState) {
        let deadline = Instant::now() + WORKFLOW_TEST_TIMEOUT;
        loop {
            if state
                .workflow_runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_empty()
            {
                return;
            }
            assert!(Instant::now() < deadline, "the run map never cleared");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn workflow_run(id: &str, script: &str, commands: Vec<String>) -> FrontendCommand {
        workflow_run_with_cap(id, script, commands, 3.5)
    }

    fn workflow_run_with_cap(
        id: &str,
        script: &str,
        commands: Vec<String>,
        max_cost_usd: f64,
    ) -> FrontendCommand {
        FrontendCommand::WorkflowRun {
            request_id: RequestId::new(id).expect("valid id"),
            spec: WorkflowSpecDto {
                name: "review".into(),
                script: script.into(),
                commands,
                max_cost_usd,
            },
        }
    }

    /// Open one file through the real dispatch path and return its snapshot.
    fn open_state_buffer(
        state: &DesktopState,
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
        path: &str,
    ) -> BufferSnapshotDto {
        let snapshot = state
            .dispatch(
                app,
                envelope(
                    state,
                    FrontendCommand::OpenFile {
                        path: WorkspacePath::new(path).expect("path"),
                    },
                ),
            )
            .expect("open file");
        snapshot
            .buffers
            .into_iter()
            .find(|buffer| buffer.path.as_str() == path)
            .expect("the opened buffer is in the bootstrap")
    }

    /// Replace the first line through the real edit command.
    fn apply_local_edit(
        state: &DesktopState,
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
        buffer: &BufferSnapshotDto,
        replacement: &str,
    ) {
        state
            .dispatch(
                app,
                envelope(
                    state,
                    FrontendCommand::ApplyEditorChange {
                        buffer_id: buffer.buffer_id.clone(),
                        version: buffer.version,
                        changes: vec![selora_protocol::TextChange {
                            start: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 0,
                            },
                            end: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 12,
                            },
                            replacement: replacement.into(),
                        }],
                    },
                ),
            )
            .expect("local edit");
    }

    /// Poll until the fake workflow's disk write is visible, so a stop or a
    /// workspace switch provably happens while the run owns the file.
    fn wait_for_disk_text(path: &std::path::Path, expected: &str) {
        let deadline = Instant::now() + WORKFLOW_TEST_TIMEOUT;
        loop {
            if std::fs::read_to_string(path).is_ok_and(|text| text == expected) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for '{}' to contain {expected:?}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn snapshots_for(
        events: &[EventEnvelope<FrontendEvent>],
        id: &selora_protocol::BufferId,
    ) -> Vec<BufferSnapshotDto> {
        events
            .iter()
            .filter_map(|event| match &event.payload {
                FrontendEvent::BufferSnapshot(snapshot) if &snapshot.buffer_id == id => {
                    Some(snapshot.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn toast_messages(events: &[EventEnvelope<FrontendEvent>]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match &event.payload {
                FrontendEvent::Toast(toast) => Some(toast.message.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn p8_workflow_run_streams_started_then_phase_step_finished() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let host = Arc::new(FakeWorkflowHost::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::clone(&host)));

        let request_id = RequestId::new("wf-run-1").expect("valid id");
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-run-1",
                        r##"
                            phase("scan");
                            report({ finding: "confirmed" });
                            artifact.markdown("summary", "# Review", { title: "Review" });
                            phase("done");
                        "##,
                        Vec::new(),
                    ),
                ),
            )
            .expect("workflow accepted");

        let events = wait_for_workflow(&state, |payload| {
            matches!(payload, FrontendEvent::WorkflowFinished(_))
        });
        let workflow: Vec<_> = events
            .iter()
            .filter(|event| {
                matches!(
                    event.payload,
                    FrontendEvent::WorkflowStarted(_)
                        | FrontendEvent::WorkflowPhase(_)
                        | FrontendEvent::WorkflowStep(_)
                        | FrontendEvent::WorkflowReport(_)
                        | FrontendEvent::WorkflowArtifact(_)
                        | FrontendEvent::WorkflowFinished(_)
                )
            })
            .collect();
        assert!(
            matches!(
                workflow.first().map(|event| &event.payload),
                Some(FrontendEvent::WorkflowStarted(_))
            ),
            "started is the first workflow event: {workflow:#?}"
        );
        assert_eq!(finished_count(&events), 1, "finished exactly once");
        let finished = workflow
            .iter()
            .find_map(|event| match &event.payload {
                FrontendEvent::WorkflowFinished(dto) => Some(dto),
                _ => None,
            })
            .expect("finished payload");
        assert_eq!(finished.status, WorkflowStatus::Completed);
        assert_eq!(finished.request_id, request_id);
        assert_eq!(finished.max_cost_usd, 3.5);
        assert!(finished.error.is_none());

        assert!(workflow.iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::WorkflowPhase(dto) if dto.phase.name == "scan"
                && dto.phase.status == WorkflowStatus::Running
        )));
        assert!(workflow.iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::WorkflowStep(dto) if dto.step.phase == "scan"
                && dto.step.label == "scan"
        )));
        assert!(workflow.iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::WorkflowReport(dto)
                if dto.report.item == "{\"finding\":\"confirmed\"}"
        )));
        assert!(workflow.iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::WorkflowArtifact(dto)
                if dto.artifact.kind == "markdown" && dto.artifact.body == "# Review"
        )));
        // Every event names the same engine-minted run.
        let run_ids: Vec<_> = workflow
            .iter()
            .filter_map(|event| match &event.payload {
                FrontendEvent::WorkflowStarted(dto) => Some(dto.run_id.clone()),
                FrontendEvent::WorkflowFinished(dto) => Some(dto.run_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(run_ids.len(), 2);
        assert_eq!(run_ids[0], run_ids[1]);
        wait_for_no_active_runs(&state);
        assert!(!host.cancelled.load(Ordering::SeqCst));
    }

    #[test]
    fn p8_workflow_run_validates_script_size_and_commands_before_spawning() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::default(),
        )));

        let oversized = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-oversized",
                        &"x".repeat(selora_workflow::MAX_SCRIPT_BYTES + 1),
                        Vec::new(),
                    ),
                ),
            )
            .expect_err("the size cap rejects before a run exists");
        assert_eq!(oversized.code, "workflow_script_too_large");
        assert!(!oversized.retryable);

        let unconfirmed = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-unconfirmed",
                        r#"world.run("cargo", ["test"]);"#,
                        Vec::new(),
                    ),
                ),
            )
            .expect_err("an unconfirmed command never starts a run");
        assert_eq!(unconfirmed.code, "workflow_command_not_confirmed");
        assert!(!unconfirmed.retryable);

        assert!(
            state
                .workflow_runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_empty(),
            "a rejected run reserves nothing"
        );
    }

    #[test]
    fn p8_workflow_failure_emits_one_failed_finished() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::default(),
        )));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-fails",
                        r#"phase("scan"); throw new Error("boom");"#,
                        Vec::new(),
                    ),
                ),
            )
            .expect("accepted; the failure is a run event, not a dispatch error");

        let events = wait_for_workflow(&state, |payload| {
            matches!(payload, FrontendEvent::WorkflowFinished(_))
        });
        assert_eq!(finished_count(&events), 1, "finished exactly once");
        let finished = events
            .iter()
            .find_map(|event| match &event.payload {
                FrontendEvent::WorkflowFinished(dto) => Some(dto),
                _ => None,
            })
            .expect("finished payload");
        assert_eq!(finished.status, WorkflowStatus::Failed);
        assert!(
            finished
                .error
                .as_deref()
                .is_some_and(|error| error.contains("boom")),
            "the script's failure rides the terminal event: {finished:#?}"
        );
        wait_for_no_active_runs(&state);
    }

    #[test]
    fn p8_workflow_stop_unknown_run_is_typed() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::default(),
        )));

        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowStop {
                        request_id: RequestId::new("wf-missing").expect("valid id"),
                    },
                ),
            )
            .expect_err("stopping an unknown run is a typed error");
        assert_eq!(error.code, "workflow_run_not_found");
        assert!(!error.retryable);
    }

    #[test]
    fn p8_workflow_resume_and_unknown_decision_are_typed() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);

        let resume = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowResume {
                        request_id: RequestId::new("wf-resume").expect("valid id"),
                        run_id: "run-1".into(),
                        allow_incomplete_asks: false,
                    },
                ),
            )
            .expect_err("a missing persisted spec cannot be resumed");
        assert_eq!(resume.code, "workflow_resume_rejected");
        assert!(!resume.retryable);

        let decision = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowDecision {
                        request_id: RequestId::new("wf-decision").expect("valid id"),
                        approval_id: ApprovalId::new("approval-1").expect("valid id"),
                        decision: DecisionDto::Approve,
                    },
                ),
            )
            .expect_err("no live run owns that approval");
        assert_eq!(decision.code, "workflow_run_not_found");
    }

    #[test]
    fn p8_workflow_decision_leaves_a_live_run_intact() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let gate = Arc::new(TestGate::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_gate(Arc::clone(&gate)),
        )));
        let request_id = RequestId::new("wf-decide").expect("valid id");

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-decide",
                        r#"world.run("sleep", []);"#,
                        vec!["sleep".into()],
                    ),
                ),
            )
            .expect("run accepted");
        wait_for_workflow(&state, |payload| {
            matches!(payload, FrontendEvent::WorkflowStarted(_))
        });

        let error = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowDecision {
                        request_id: request_id.clone(),
                        approval_id: ApprovalId::new("approval-1").expect("valid id"),
                        decision: DecisionDto::Approve,
                    },
                ),
            )
            .expect_err("approvals are not represented on the wave-2 host");
        assert_eq!(error.code, "workflow_approval_unavailable");
        assert!(!error.retryable);
        assert!(
            state
                .workflow_runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .contains_key(&request_id),
            "an unsupported decision must not evict the run"
        );

        gate.open();
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
    }

    #[test]
    fn p8_workflow_stop_flips_the_active_host_cancel() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let gate = Arc::new(TestGate::default());
        let host = Arc::new(FakeWorkflowHost::with_gate(Arc::clone(&gate)));
        state.install_workflow_host_factory(fake_workflow_factory(Arc::clone(&host)));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-stop",
                        r#"world.run("sleep", []);"#,
                        vec!["sleep".into()],
                    ),
                ),
            )
            .expect("run accepted");
        wait_for_workflow(&state, |payload| {
            matches!(payload, FrontendEvent::WorkflowStarted(_))
        });
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowStop {
                        request_id: RequestId::new("wf-stop").expect("valid id"),
                    },
                ),
            )
            .expect("the live run's cancel is flipped");
        assert!(
            host.cancelled.load(Ordering::SeqCst),
            "WorkflowStop calls the active host's cancel()"
        );

        gate.open();
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
    }

    /// The documented policy: **concurrent runs are allowed** (events are
    /// run_id-scoped), but a live request id is unique because Stop/Decision
    /// address the run by it.
    #[test]
    fn p8_workflow_second_run_is_allowed_but_a_live_request_id_is_unique() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let gate = Arc::new(TestGate::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_gate(Arc::clone(&gate)),
        )));

        let run = |id: &str| workflow_run(id, r#"world.run("sleep", []);"#, vec!["sleep".into()]);

        state
            .dispatch(&app, envelope(&state, run("wf-a")))
            .expect("first run accepted");
        wait_for_workflow(&state, |payload| {
            matches!(payload, FrontendEvent::WorkflowStarted(_))
        });

        state
            .dispatch(&app, envelope(&state, run("wf-b")))
            .expect("a second, differently-addressed run is allowed concurrently");
        let duplicate = state
            .dispatch(&app, envelope(&state, run("wf-a")))
            .expect_err("the live request id stays unique");
        assert_eq!(duplicate.code, "workflow_run_already_active");
        assert!(!duplicate.retryable);

        gate.open();
        let events = wait_for_finished_runs(&state, 2);
        assert_eq!(finished_count(&events), 2);
        assert!(
            events
                .iter()
                .filter_map(|event| match &event.payload {
                    FrontendEvent::WorkflowFinished(dto) => Some(dto),
                    _ => None,
                })
                .all(|finished| finished.status == WorkflowStatus::Completed),
            "both concurrent runs complete"
        );
        wait_for_no_active_runs(&state);
    }

    #[test]
    fn p8_workflow_approval_maps_exact_diff_and_consumes_only_matching_id() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let gate = Arc::new(TestGate::default());
        let host = Arc::new(FakeWorkflowHost::with_gate(Arc::clone(&gate)));
        let callback = Arc::new(Mutex::new(
            None::<crate::workflows_host::WorkflowApprovalHandler>,
        ));
        let pending = Arc::new(AtomicBool::new(true));
        let approval = Uuid::new_v4();
        let capture = Arc::clone(&callback);
        let slot = Arc::clone(&pending);
        state.install_workflow_host_factory(Arc::new(move |_root, settings| {
            *capture.lock().unwrap() = Some(settings.on_approval);
            let cancel_host = Arc::clone(&host);
            let slot = Arc::clone(&slot);
            WorkflowHostBundle {
                host: Arc::clone(&host) as Arc<dyn Host>,
                cancel: Arc::new(move || cancel_host.cancelled.store(true, Ordering::SeqCst)),
                decide: Some(Arc::new(move |id, _decision| {
                    if id != approval || !slot.swap(false, Ordering::SeqCst) {
                        return Err(HostError::new("no matching pending approval"));
                    }
                    Ok(())
                })),
            }
        }));
        let request = RequestId::new("wf-approval").unwrap();
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        request.as_str(),
                        r#"world.run("sleep", []);"#,
                        vec!["sleep".into()],
                    ),
                ),
            )
            .unwrap();
        wait_for_workflow(&state, |event| {
            matches!(event, FrontendEvent::WorkflowStarted(_))
        });
        callback.lock().unwrap().clone().unwrap()(
            approval,
            selora_ai::ApprovalCard::Diff {
                path: "src/main.rs".into(),
                diff: "-old\n+new\n".into(),
            },
        );
        let events = state.events_after(0);
        let card = events
            .iter()
            .find_map(|event| match &event.payload {
                FrontendEvent::WorkflowApprovalNeeded(card) => Some(card),
                _ => None,
            })
            .expect("approval reached protocol");
        assert_eq!(card.request_id, request);
        assert_eq!(card.path_or_command, "src/main.rs");
        assert_eq!(card.diff_or_input.as_deref(), Some("-old\n+new\n"));
        assert!(!card.can_approve_all_this_turn);
        let id = ApprovalId::new(approval.to_string()).unwrap();
        assert_eq!(
            state
                .workflow_decision(&request, &id, DecisionDto::ApproveAllThisTurn)
                .unwrap_err()
                .code,
            "workflow_blanket_approval_rejected"
        );
        assert!(pending.load(Ordering::SeqCst));
        let wrong = ApprovalId::new(Uuid::new_v4().to_string()).unwrap();
        assert_eq!(
            state
                .workflow_decision(&request, &wrong, DecisionDto::Approve)
                .unwrap_err()
                .code,
            "workflow_approval_not_found"
        );
        assert!(pending.load(Ordering::SeqCst));
        state
            .workflow_decision(&request, &id, DecisionDto::Deny)
            .unwrap();
        assert!(!pending.load(Ordering::SeqCst));
        assert_eq!(
            state
                .workflow_decision(&request, &id, DecisionDto::Approve)
                .unwrap_err()
                .code,
            "workflow_approval_not_found"
        );
        gate.open();
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
    }

    #[test]
    fn p8_workflow_completed_run_is_durable_and_resume_never_reexecutes_it() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::default(),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-durable",
                        r#"agent("reviewer").ask("inspect"); report("done");"#,
                        vec![],
                    ),
                ),
            )
            .unwrap();
        let events = wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
        let summary = events
            .iter()
            .find_map(|event| match &event.payload {
                FrontendEvent::WorkflowFinished(summary) => Some(summary),
                _ => None,
            })
            .unwrap();
        let id = selora_workflow::RunId::from(summary.run_id.as_str());
        assert!(selora_workflow::journal::journal_path(dir.path(), &id).exists());
        let spec = selora_workflow::load_workflow_spec(dir.path(), &id).unwrap();
        assert_eq!(spec.name, "review");
        let resumed = state.dispatch(
            &app,
            envelope(
                &state,
                FrontendCommand::WorkflowResume {
                    request_id: RequestId::new("wf-completed-resume").unwrap(),
                    run_id: summary.run_id.clone(),
                    allow_incomplete_asks: false,
                },
            ),
        );
        assert_eq!(resumed.unwrap_err().code, "workflow_already_completed");
        assert_eq!(finished_count(&state.events_after(0)), 1);
        let recovered = state
            .bootstrap()
            .workflows
            .into_iter()
            .find(|run| run.run_id == summary.run_id)
            .expect("a completed run is offered as history");
        assert_eq!(recovered.status, WorkflowStatus::Completed);
        assert!(
            !recovered.resumable,
            "a completed run is never offered for resume"
        );
        assert_eq!(recovered.reports.len(), 1);
        assert_eq!(recovered.reports[0].item, "done");
        assert_eq!(recovered.tokens, summary.tokens);
        assert_eq!(recovered.cost_usd, summary.cost_usd);
    }

    #[test]
    fn p9_workflow_budget_exhausted_resume_is_refused_before_worker() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let host = Arc::new(FakeWorkflowHost::with_cost_per_call(1.0));
        state.install_workflow_host_factory(fake_workflow_factory(Arc::clone(&host)));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run_with_cap(
                        "wf-budget",
                        r#"phase("one"); agent("reviewer").ask("spend"); phase("two");"#,
                        vec![],
                        0.1,
                    ),
                ),
            )
            .expect("run accepted");
        let events = wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
        assert_eq!(finished_count(&events), 1);

        let saved = state
            .bootstrap()
            .workflows
            .into_iter()
            .find(|run| run.run_id == "wf-budget" || run.name == "review")
            .expect("the budget run is listed as history");
        assert_eq!(saved.status, WorkflowStatus::Failed);
        assert!(!saved.resumable, "a cap trip is never offered for resume");
        let detail = saved.detail.as_deref().unwrap_or_default();
        assert!(
            detail.contains("budget") && detail.contains("0.10"),
            "the cap reason rides `detail`: {detail}"
        );

        let before = state.log.snapshot_sequence();
        let refused = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowResume {
                        request_id: RequestId::new("wf-budget-resume").unwrap(),
                        run_id: saved.run_id.clone(),
                        allow_incomplete_asks: false,
                    },
                ),
            )
            .expect_err("a budget-exhausted run cannot resume");
        assert_eq!(refused.code, "workflow_budget_exhausted");
        assert!(!refused.retryable);
        assert!(
            refused.message.contains("cost cap")
                && refused.message.contains("new run")
                && refused.message.contains("budget"),
            "the refusal explains the cap and the only continuation: {}",
            refused.message
        );
        assert!(
            state
                .workflow_runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_empty(),
            "a refused budget resume starts no worker"
        );
        assert_eq!(
            state.log.snapshot_sequence(),
            before,
            "a refused budget resume emits no event"
        );
        assert_eq!(finished_count(&state.events_after(0)), 1);
    }

    #[test]
    fn p9_workflow_budget_raw_terminal_is_refused_and_listed_with_cap_reason() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let run_id = selora_workflow::RunId::from("budget-raw");
        let run_dir = selora_workflow::journal::run_dir(dir.path(), &run_id);
        std::fs::create_dir_all(&run_dir).unwrap();
        let spec = selora_workflow::prepare("raw-budget", r#"phase("one");"#, 2.0)
            .unwrap()
            .spec();
        std::fs::write(
            run_dir.join("spec.json"),
            serde_json::to_vec(&spec).unwrap(),
        )
        .unwrap();
        std::fs::write(run_dir.join("run.lock"), b"").unwrap();
        // A raw terminal record with no parseable report payload still proves
        // the cap fired; the preflight must not wait for the typed plan.
        std::fs::write(
            selora_workflow::journal::journal_path(dir.path(), &run_id),
            b"{\"seq\":1,\"kind\":\"engine_run_finished\",\"payload\":{\"status\":\"budget_exceeded\"}}\n",
        )
        .unwrap();

        let saved = state
            .bootstrap()
            .workflows
            .into_iter()
            .find(|run| run.run_id == "budget-raw")
            .expect("the raw budget run is listed");
        assert_eq!(saved.status, WorkflowStatus::Failed);
        assert!(!saved.resumable);
        assert!(
            saved
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("cost cap"),
            "the listing carries the cap reason: {:?}",
            saved.detail
        );

        // The listing itself must not move the event log.
        let before = state.log.snapshot_sequence();
        let refused = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowResume {
                        request_id: RequestId::new("wf-budget-raw-resume").unwrap(),
                        run_id: saved.run_id.clone(),
                        allow_incomplete_asks: false,
                    },
                ),
            )
            .expect_err("a raw budget terminal cannot resume");
        assert_eq!(refused.code, "workflow_budget_exhausted");
        assert!(!refused.retryable);
        assert!(refused.message.contains("2.00"), "{}", refused.message);
        assert_eq!(
            state.log.snapshot_sequence(),
            before,
            "the refusal happens before any event"
        );
        assert!(state
            .workflow_runs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .is_empty());
    }

    #[test]
    fn p9_workflow_budget_preflight_only_flags_a_budget_terminal() {
        use selora_workflow::journal::{JournalEntry, LoadedJournal, RunStatus};

        let journal_with = |status: &str| LoadedJournal {
            path: PathBuf::new(),
            entries: vec![JournalEntry {
                seq: 1,
                kind: "engine_run_finished".into(),
                payload: serde_json::json!({ "status": status }),
            }],
            corrupt: Vec::new(),
        };
        assert_eq!(
            workflow_terminal_status(&journal_with("completed")),
            Some(RunStatus::Completed)
        );
        assert_eq!(
            workflow_terminal_status(&journal_with("failed")),
            Some(RunStatus::Failed)
        );
        assert_eq!(
            workflow_terminal_status(&journal_with("stopped")),
            Some(RunStatus::Stopped)
        );
        assert_eq!(
            workflow_terminal_status(&journal_with("budget_exceeded")),
            Some(RunStatus::BudgetExceeded)
        );
        assert_eq!(
            workflow_terminal_status(&journal_with("BudgetExceeded")),
            Some(RunStatus::BudgetExceeded),
            "raw legacy spelling is still a cap trip"
        );
        assert_eq!(workflow_terminal_status(&journal_with("unknown")), None);
    }

    #[test]
    fn p9_workflow_budget_preflight_keeps_corrupt_and_missing_journals_typed() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::default(),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run("wf-corrupt", r#"phase("scan"); report("kept");"#, vec![]),
                ),
            )
            .unwrap();
        let events = wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
        let summary = events
            .iter()
            .find_map(|event| match &event.payload {
                FrontendEvent::WorkflowFinished(summary) => Some(summary),
                _ => None,
            })
            .unwrap();
        let id = selora_workflow::RunId::from(summary.run_id.as_str());
        let path = selora_workflow::journal::journal_path(dir.path(), &id);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<&str> = text.lines().collect();
        *lines.last_mut().expect("a terminal record") = "{not json";
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

        let before = state.log.snapshot_sequence();
        for (request_id, run_id) in [
            ("wf-corrupt-resume", summary.run_id.clone()),
            ("wf-missing-resume", "missing-run".to_owned()),
        ] {
            let refused = state
                .dispatch(
                    &app,
                    envelope(
                        &state,
                        FrontendCommand::WorkflowResume {
                            request_id: RequestId::new(request_id).unwrap(),
                            run_id,
                            allow_incomplete_asks: false,
                        },
                    ),
                )
                .expect_err("an unprovable journal cannot resume");
            assert_eq!(
                refused.code, "workflow_resume_rejected",
                "corruption is not misreported as a budget trip"
            );
            assert!(!refused.retryable);
        }
        assert_eq!(
            state.log.snapshot_sequence(),
            before,
            "refusals emit no event"
        );
        assert!(
            state
                .workflow_runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_empty(),
            "refusals start no worker"
        );
        // The damaged run stays listed but is not offered as resumable.
        let damaged = state
            .bootstrap()
            .workflows
            .into_iter()
            .find(|run| run.run_id == summary.run_id)
            .expect("a damaged journal stays listed");
        assert!(!damaged.resumable);
    }

    #[test]
    fn p9_workflow_stopped_run_still_resumes_without_the_budget_refusal() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let gate = Arc::new(TestGate::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_gate(Arc::clone(&gate)),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-stopped-resume",
                        r#"world.run("sleep", []);"#,
                        vec!["sleep".into()],
                    ),
                ),
            )
            .expect("run accepted");
        wait_for_workflow(&state, |payload| {
            matches!(payload, FrontendEvent::WorkflowStarted(_))
        });
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowStop {
                        request_id: RequestId::new("wf-stopped-resume").unwrap(),
                    },
                ),
            )
            .expect("stop accepted");
        gate.open();
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);

        let saved = state
            .bootstrap()
            .workflows
            .into_iter()
            .find(|run| run.name == "review")
            .expect("the stopped run is listed");
        assert_eq!(saved.status, WorkflowStatus::Stopped);
        assert!(saved.resumable, "a stopped run keeps its resume contract");

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowResume {
                        request_id: RequestId::new("wf-stopped-resume-again").unwrap(),
                        run_id: saved.run_id,
                        allow_incomplete_asks: false,
                    },
                ),
            )
            .expect("a stopped run is not budget-exhausted");
        let events = wait_for_finished_runs(&state, 2);
        wait_for_no_active_runs(&state);
        assert!(events.iter().any(|event| matches!(&event.payload,
            FrontendEvent::WorkflowFinished(done)
                if done.request_id.as_str() == "wf-stopped-resume-again")));
    }

    #[test]
    fn p8_workflow_bootstrap_offers_only_safe_interrupted_runs() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        let host = Arc::new(FakeWorkflowHost::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::clone(&host)));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-interrupted",
                        r#"phase("scan"); report("kept"); throw new Error("interrupted");"#,
                        vec![],
                    ),
                ),
            )
            .unwrap();
        let events = wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
        assert_eq!(finished_count(&events), 1);
        let recovered: Vec<_> = state.bootstrap().workflows;
        let saved = recovered
            .iter()
            .find(|run| run.name == "review")
            .expect("an interrupted durable run is offered");
        assert!(saved.resumable);
        assert_eq!(saved.status, WorkflowStatus::Failed);
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowResume {
                        request_id: RequestId::new("wf-interrupted-resume").unwrap(),
                        run_id: saved.run_id.clone(),
                        allow_incomplete_asks: false,
                    },
                ),
            )
            .expect("the safe interrupted run can resume");
        let events = wait_for_finished_runs(&state, 2);
        wait_for_no_active_runs(&state);
        assert!(events.iter().any(|event| matches!(&event.payload,
            FrontendEvent::WorkflowFinished(done)
                if done.request_id.as_str() == "wf-interrupted-resume")));
    }

    #[test]
    fn p8_workflow_resume_refuses_until_incomplete_asks_are_confirmed() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_failing_ask(),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-ask-failed",
                        r#"agent("reviewer").ask("inspect src/lib.rs");"#,
                        vec![],
                    ),
                ),
            )
            .unwrap();
        let events = wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
        assert_eq!(finished_count(&events), 1);
        let saved = state
            .bootstrap()
            .workflows
            .into_iter()
            .find(|run| run.name == "review")
            .expect("the failed run is listed");
        assert!(saved.resumable);
        assert_eq!(
            saved.detail.as_deref(),
            Some("workflow ask failed"),
            "the terminal journal error rides `detail` into the run model"
        );

        let refused = state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowResume {
                        request_id: RequestId::new("wf-ask-unconfirmed").unwrap(),
                        run_id: saved.run_id.clone(),
                        allow_incomplete_asks: false,
                    },
                ),
            )
            .expect_err("an unconfirmed re-dispatch is refused");
        assert_eq!(refused.code, "workflow_incomplete_asks");
        assert!(!refused.retryable);
        assert!(
            refused.message.contains("1 unfinished ask step"),
            "the refusal names the count: {}",
            refused.message
        );
        assert!(
            refused.message.contains("reviewer") && refused.message.contains("inspect"),
            "the refusal names the step/agent: {}",
            refused.message
        );
        assert!(
            refused.message.contains("may already have applied effects"),
            "the refusal states the effect risk: {}",
            refused.message
        );
        // No worker, no event: the refusal is a preflight.
        assert!(
            state
                .workflow_runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_empty(),
            "a refused resume starts no worker"
        );
        let events = state.events_after(0);
        assert_eq!(finished_count(&events), 1);
        assert!(
            !events.iter().any(|event| matches!(&event.payload,
                FrontendEvent::WorkflowStarted(started)
                    if started.request_id.as_str() == "wf-ask-unconfirmed")),
            "a refused resume emits no started event"
        );

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowResume {
                        request_id: RequestId::new("wf-ask-confirmed").unwrap(),
                        run_id: saved.run_id,
                        allow_incomplete_asks: true,
                    },
                ),
            )
            .expect("the explicit confirmation resumes");
        let events = wait_for_finished_runs(&state, 2);
        wait_for_no_active_runs(&state);
        assert!(events.iter().any(|event| matches!(&event.payload,
            FrontendEvent::WorkflowFinished(done)
                if done.request_id.as_str() == "wf-ask-confirmed")));
    }

    #[test]
    fn p8_workflow_completed_run_history_reconstructs_from_the_journal() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let _dir = open_workspace(&state, &app);
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::default(),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-history",
                        r###"phase("scan");
                           agent("reviewer").ask("inspect src/lib.rs");
                           artifact.markdown("summary", "## Findings", { title: "Review summary" });
                           report("kept", "summary");"###,
                        vec![],
                    ),
                ),
            )
            .unwrap();
        let events = wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
        let summary = events
            .iter()
            .find_map(|event| match &event.payload {
                FrontendEvent::WorkflowFinished(summary) => Some(summary),
                _ => None,
            })
            .unwrap();
        assert_eq!(summary.status, WorkflowStatus::Completed);

        let saved = state
            .bootstrap()
            .workflows
            .into_iter()
            .find(|run| run.run_id == summary.run_id)
            .expect("a completed run appears as history");
        assert_eq!(saved.status, WorkflowStatus::Completed);
        assert!(!saved.resumable, "completed history is not resumable");
        assert!(saved.detail.is_none());
        assert_eq!(saved.phases, vec!["scan".to_string()]);
        assert_eq!(saved.steps.len(), 4, "phase/ask/artifact/report steps");
        assert_eq!(saved.steps[0].label, "scan");
        assert_eq!(saved.steps[1].agent.as_deref(), Some("reviewer"));
        assert_eq!(saved.reports.len(), 1);
        assert_eq!(saved.reports[0].id, "report-1");
        assert_eq!(saved.reports[0].item, "kept");
        assert_eq!(saved.reports[0].artifact_id.as_deref(), Some("summary"));
        assert_eq!(saved.artifacts.len(), 1);
        assert_eq!(saved.artifacts[0].id, "summary");
        assert_eq!(saved.artifacts[0].kind, "markdown");
        assert_eq!(saved.artifacts[0].title, "Review summary");
        assert_eq!(saved.artifacts[0].body, "## Findings");
        assert_eq!(saved.tokens, summary.tokens);
        assert_eq!(saved.cost_usd, summary.cost_usd);
    }

    #[test]
    fn p8_workflow_damaged_history_is_not_fabricated() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::default(),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-damaged-history",
                        r#"phase("scan"); report("kept");"#,
                        vec![],
                    ),
                ),
            )
            .unwrap();
        let events = wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
        let summary = events
            .iter()
            .find_map(|event| match &event.payload {
                FrontendEvent::WorkflowFinished(summary) => Some(summary),
                _ => None,
            })
            .unwrap();
        let id = selora_workflow::RunId::from(summary.run_id.as_str());
        let path = selora_workflow::journal::journal_path(dir.path(), &id);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<&str> = text.lines().collect();
        *lines.last_mut().expect("a terminal record") = "{not json";
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

        let saved = state
            .bootstrap()
            .workflows
            .into_iter()
            .find(|run| run.run_id == summary.run_id)
            .expect("a damaged journal stays listed");
        assert!(!saved.resumable);
        assert!(saved.detail.is_some(), "the damage is explained");
        assert!(saved.phases.is_empty(), "no phase is invented");
        assert!(saved.steps.is_empty(), "no step is invented");
        assert!(saved.reports.is_empty(), "no report is invented");
        assert!(saved.artifacts.is_empty(), "no artifact is invented");
        assert_eq!(saved.tokens, 0);
        assert_eq!(saved.cost_usd, 0.0);
    }

    #[test]
    fn p8_workflow_persistence_failure_is_visible_and_executes_nothing() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        std::fs::write(dir.path().join(".selora"), "not a directory").unwrap();
        let host = Arc::new(FakeWorkflowHost::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::clone(&host)));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-no-journal",
                        r#"world.run("anything", []);"#,
                        vec!["anything".into()],
                    ),
                ),
            )
            .unwrap();
        let events = wait_for_finished_runs(&state, 1);
        let started = events
            .iter()
            .position(|event| matches!(&event.payload, FrontendEvent::WorkflowStarted(_)))
            .unwrap();
        let finished = events.iter().position(|event| matches!(
            &event.payload, FrontendEvent::WorkflowFinished(done) if done.status == WorkflowStatus::Failed)).unwrap();
        assert!(started < finished);
        assert_eq!(finished_count(&events), 1);
        assert_eq!(host.world_calls.load(Ordering::SeqCst), 0);
        wait_for_no_active_runs(&state);
    }

    // ------------------------------------------------------------------
    // Phase 8 gap 3 — the post-run disk refresh.
    //
    // A workflow edits files behind the editor's back; the terminal event must
    // reload clean tabs (one version bump each), never overwrite a dirty tab,
    // keep a deleted tab alive, and never touch a workspace the run did not
    // start in.
    // ------------------------------------------------------------------

    #[test]
    fn p8_workflow_completion_refreshes_clean_buffers_and_emits_snapshot() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = open_state_buffer(&state, &app, "hello.rs");
        let edited = "fn main() { println!(\"workflow wrote this\"); }\n";
        let baseline = state.log.snapshot_sequence();
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_writes(vec![(
                dir.path().join("hello.rs"),
                Some(edited.as_bytes().to_vec()),
            )]),
        )));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-refresh",
                        r#"world.run("edit", []);"#,
                        vec!["edit".into()],
                    ),
                ),
            )
            .expect("run accepted");
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);

        let events = state.events_after(baseline);
        let snapshots = snapshots_for(&events, &opened.buffer_id);
        assert_eq!(
            snapshots.len(),
            1,
            "exactly one reload snapshot: {events:#?}"
        );
        let snapshot = &snapshots[0];
        assert_eq!(snapshot.text, edited);
        assert_eq!(snapshot.version, opened.version + 1, "one version bump");
        assert!(!snapshot.dirty);
        assert_eq!(snapshot.language, opened.language, "language is preserved");
        assert_eq!(snapshot.path, opened.path);

        // The snapshot follows the terminal event so the panel sees the run
        // finish before the buffer changes.
        let finished_at = events
            .iter()
            .position(|event| matches!(event.payload, FrontendEvent::WorkflowFinished(_)))
            .expect("finished recorded");
        let refresh_at = events
            .iter()
            .position(|event| {
                matches!(&event.payload,
                    FrontendEvent::BufferSnapshot(dto) if dto.buffer_id == opened.buffer_id)
            })
            .expect("refresh recorded");
        assert!(finished_at < refresh_at, "{events:#?}");

        let stored = state
            .bootstrap()
            .buffers
            .into_iter()
            .find(|buffer| buffer.buffer_id == opened.buffer_id)
            .expect("tab still open");
        assert_eq!(stored.text, edited);
        assert_eq!(stored.version, 2);
        assert!(!stored.dirty);
    }

    #[test]
    fn p8_workflow_dirty_conflict_keeps_local_text_and_offers_a_choice() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = open_state_buffer(&state, &app, "hello.rs");
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ApplyEditorChange {
                        buffer_id: opened.buffer_id.clone(),
                        version: opened.version,
                        changes: vec![selora_protocol::TextChange {
                            start: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 0,
                            },
                            end: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 12,
                            },
                            replacement: "fn local() {}".into(),
                        }],
                    },
                ),
            )
            .expect("local edit");
        let baseline = state.log.snapshot_sequence();
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_writes(vec![(
                dir.path().join("hello.rs"),
                Some(b"fn main() { println!(\"workflow\"); }\n".to_vec()),
            )]),
        )));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run("wf-dirty", r#"world.run("edit", []);"#, vec!["edit".into()]),
                ),
            )
            .expect("run accepted");
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);

        let events = state.events_after(baseline);
        assert!(
            snapshots_for(&events, &opened.buffer_id).is_empty(),
            "a dirty buffer is never auto-reloaded: {events:#?}"
        );
        let conflicts: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.payload {
                FrontendEvent::BufferConflict(conflict) => Some(conflict),
                _ => None,
            })
            .collect();
        assert_eq!(conflicts.len(), 1, "one conflict banner: {events:#?}");
        assert_eq!(conflicts[0].buffer_id, opened.buffer_id);
        assert_eq!(conflicts[0].path.as_str(), "hello.rs");
        assert!(
            !events
                .iter()
                .any(|event| matches!(&event.payload, FrontendEvent::BufferConflictCleared(_))),
            "the conflict stays until the user chooses: {events:#?}"
        );
        assert!(
            !toast_messages(&events)
                .iter()
                .any(|message| message.contains("unsaved")),
            "the conflict is a banner, not a silent toast: {events:#?}"
        );

        let stored = state
            .bootstrap()
            .buffers
            .into_iter()
            .find(|buffer| buffer.buffer_id == opened.buffer_id)
            .expect("tab still open");
        assert_eq!(stored.text, "fn local() {}\n", "local text survives");
        assert_eq!(stored.version, 2, "the local edit's version is kept");
        assert!(stored.dirty);
    }

    #[test]
    fn p8_workflow_reload_from_disk_resolves_the_conflict_and_syncs_lsp() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = open_state_buffer(&state, &app, "hello.rs");
        let absolute = dir.path().join("hello.rs");
        {
            let guard = state
                .lsp
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let bridge = guard.as_ref().expect("OpenFile starts the bridge");
            bridge.seed_error_diagnostic_for_test(&absolute, "stale diagnostic");
        }
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ApplyEditorChange {
                        buffer_id: opened.buffer_id.clone(),
                        version: opened.version,
                        changes: vec![selora_protocol::TextChange {
                            start: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 0,
                            },
                            end: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 12,
                            },
                            replacement: "fn local() {}".into(),
                        }],
                    },
                ),
            )
            .expect("local edit");
        let edited = "fn main() { /* workflow text */ }\n";
        let gate = Arc::new(TestGate::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_gate_and_writes(
                Arc::clone(&gate),
                vec![(absolute.clone(), Some(edited.as_bytes().to_vec()))],
            ),
        )));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-reload",
                        r#"world.run("edit", []);"#,
                        vec!["edit".into()],
                    ),
                ),
            )
            .expect("run accepted");
        wait_for_disk_text(&absolute, edited);
        let baseline = state.log.snapshot_sequence();
        gate.open();
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);

        let before_choice = state.events_after(baseline);
        assert!(
            before_choice.iter().any(|event| matches!(
                &event.payload,
                FrontendEvent::BufferConflict(conflict)
                    if conflict.buffer_id == opened.buffer_id
            )),
            "the conflict is raised before the choice: {before_choice:#?}"
        );
        assert!(
            snapshots_for(&before_choice, &opened.buffer_id).is_empty(),
            "the refresh alone never overwrites: {before_choice:#?}"
        );

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ReloadBufferFromDisk {
                        buffer_id: opened.buffer_id.clone(),
                    },
                ),
            )
            .expect("reload from disk");

        let events = state.events_after(baseline);
        let snapshots = snapshots_for(&events, &opened.buffer_id);
        assert_eq!(snapshots.len(), 1, "one reload snapshot: {events:#?}");
        assert_eq!(snapshots[0].text, edited);
        assert_eq!(
            snapshots[0].version,
            opened.version + 2,
            "local edit + reload"
        );
        assert!(!snapshots[0].dirty);
        assert!(
            events.iter().any(|event| matches!(
                &event.payload,
                FrontendEvent::BufferConflictCleared(cleared)
                    if cleared.buffer_id == opened.buffer_id
            )),
            "the marker is cleared: {events:#?}"
        );

        let stored = state
            .bootstrap()
            .buffers
            .into_iter()
            .find(|buffer| buffer.buffer_id == opened.buffer_id)
            .expect("tab still open");
        assert_eq!(stored.text, edited);
        assert_eq!(stored.version, opened.version + 2);
        assert!(!stored.dirty);

        {
            let guard = state
                .lsp
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let bridge = guard.as_ref().expect("bridge");
            assert_eq!(
                bridge.text_for_test(&absolute).as_deref(),
                Some(edited),
                "the language server got the reloaded full text"
            );
        }
        assert!(
            events.iter().any(|event| matches!(
                &event.payload,
                FrontendEvent::DiagnosticsChanged(diagnostics)
                    if diagnostics.iter().all(|diagnostic| diagnostic.path.as_str() != "hello.rs")
            )),
            "the reload retracted the stale diagnostics: {events:#?}"
        );
    }

    #[test]
    fn p8_workflow_keep_my_edits_clears_the_conflict_without_touching_text() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = open_state_buffer(&state, &app, "hello.rs");
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ApplyEditorChange {
                        buffer_id: opened.buffer_id.clone(),
                        version: opened.version,
                        changes: vec![selora_protocol::TextChange {
                            start: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 0,
                            },
                            end: selora_protocol::TextPosition {
                                line: 0,
                                utf16_column: 12,
                            },
                            replacement: "fn local() {}".into(),
                        }],
                    },
                ),
            )
            .expect("local edit");
        let edited = "fn main() { /* workflow text */ }\n";
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_writes(vec![(
                dir.path().join("hello.rs"),
                Some(edited.as_bytes().to_vec()),
            )]),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run("wf-keep", r#"world.run("edit", []);"#, vec!["edit".into()]),
                ),
            )
            .expect("run accepted");
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
        let baseline = state.log.snapshot_sequence();

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::KeepBufferEdits {
                        buffer_id: opened.buffer_id.clone(),
                    },
                ),
            )
            .expect("keep my edits");

        let events = state.events_after(baseline);
        assert!(
            snapshots_for(&events, &opened.buffer_id).is_empty(),
            "keep never replaces the local text: {events:#?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                &event.payload,
                FrontendEvent::BufferConflictCleared(cleared)
                    if cleared.buffer_id == opened.buffer_id
            )),
            "the marker is cleared: {events:#?}"
        );
        let stored = state
            .bootstrap()
            .buffers
            .into_iter()
            .find(|buffer| buffer.buffer_id == opened.buffer_id)
            .expect("tab still open");
        assert_eq!(stored.text, "fn local() {}\n");
        assert_eq!(stored.version, opened.version + 1);
        assert!(stored.dirty);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("hello.rs")).unwrap(),
            edited,
            "keep never writes over the workflow's disk text"
        );
    }

    #[test]
    fn p8_workflow_deleted_file_leaves_the_tab_intact() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = open_state_buffer(&state, &app, "hello.rs");
        let baseline = state.log.snapshot_sequence();
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_writes(vec![(dir.path().join("hello.rs"), None)]),
        )));

        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run("wf-delete", r#"world.run("rm", []);"#, vec!["rm".into()]),
                ),
            )
            .expect("run accepted");
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);

        let events = state.events_after(baseline);
        assert!(
            snapshots_for(&events, &opened.buffer_id).is_empty(),
            "no reload for a deleted file: {events:#?}"
        );
        let toasts = toast_messages(&events);
        assert_eq!(toasts.len(), 1, "{events:#?}");
        assert!(toasts[0].contains("hello.rs"), "{}", toasts[0]);
        assert!(toasts[0].contains("no longer exists"), "{}", toasts[0]);

        let stored = state
            .bootstrap()
            .buffers
            .into_iter()
            .find(|buffer| buffer.buffer_id == opened.buffer_id)
            .expect("the deleted file's tab stays open");
        assert_eq!(stored.text, "fn main() {}\n", "last known text is kept");
        assert_eq!(stored.version, 1);
    }

    #[test]
    fn p8_workflow_workspace_switch_does_not_touch_the_new_workspaces_buffers() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir_a = open_workspace(&state, &app);
        open_state_buffer(&state, &app, "hello.rs");
        let gate = Arc::new(TestGate::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_gate_and_writes(
                Arc::clone(&gate),
                vec![(
                    dir_a.path().join("hello.rs"),
                    Some(b"fn main() { /* run A */ }\n".to_vec()),
                )],
            ),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-switch",
                        r#"world.run("edit", []);"#,
                        vec!["edit".into()],
                    ),
                ),
            )
            .expect("run accepted");
        wait_for_workflow(&state, |payload| {
            matches!(payload, FrontendEvent::WorkflowStarted(_))
        });
        wait_for_disk_text(
            &dir_a.path().join("hello.rs"),
            "fn main() { /* run A */ }\n",
        );

        // Switch to a second workspace and open its own hello.rs.
        let dir_b = open_workspace(&state, &app);
        let opened_b = open_state_buffer(&state, &app, "hello.rs");
        std::fs::write(
            dir_b.path().join("hello.rs"),
            "fn main() { /* run B must stay */ }\n",
        )
        .expect("external write in workspace B");
        let baseline = state.log.snapshot_sequence();

        gate.open();
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);

        let after_switch = state.events_after(baseline);
        assert!(
            snapshots_for(&after_switch, &opened_b.buffer_id).is_empty(),
            "the run must not refresh the new workspace: {after_switch:#?}"
        );
        assert!(
            !after_switch
                .iter()
                .any(|event| matches!(&event.payload, FrontendEvent::BufferSnapshot(_))),
            "no snapshot at all belongs to the switched workspace: {after_switch:#?}"
        );
        assert!(
            !after_switch.iter().any(|event| matches!(
                &event.payload,
                FrontendEvent::BufferConflict(_) | FrontendEvent::BufferConflictCleared(_)
            )),
            "the run must not raise a conflict in the new workspace: {after_switch:#?}"
        );
        let stored = state
            .bootstrap()
            .buffers
            .into_iter()
            .find(|buffer| buffer.buffer_id == opened_b.buffer_id)
            .expect("workspace B's tab");
        assert_eq!(stored.text, "fn main() {}\n");
        assert_eq!(stored.version, 1);
    }

    #[test]
    fn p8_workflow_failure_refreshes_clean_buffers() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = open_state_buffer(&state, &app, "hello.rs");
        let edited = "fn main() { /* written before the failure */ }\n";
        let baseline = state.log.snapshot_sequence();
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_writes(vec![(
                dir.path().join("hello.rs"),
                Some(edited.as_bytes().to_vec()),
            )]),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-failed-refresh",
                        r#"world.run("edit", []); throw new Error("boom");"#,
                        vec!["edit".into()],
                    ),
                ),
            )
            .expect("run accepted");
        let events = wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::WorkflowFinished(done) if done.status == WorkflowStatus::Failed
        )));
        let snapshots = snapshots_for(&state.events_after(baseline), &opened.buffer_id);
        assert_eq!(snapshots.len(), 1, "{events:#?}");
        assert_eq!(snapshots[0].text, edited);
        assert_eq!(snapshots[0].version, 2);
    }

    #[test]
    fn p8_workflow_stop_refreshes_clean_buffers() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = open_state_buffer(&state, &app, "hello.rs");
        let edited = "fn main() { /* written before the stop */ }\n";
        let baseline = state.log.snapshot_sequence();
        let gate = Arc::new(TestGate::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_gate_and_writes(
                Arc::clone(&gate),
                vec![(
                    dir.path().join("hello.rs"),
                    Some(edited.as_bytes().to_vec()),
                )],
            ),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-stopped-refresh",
                        r#"world.run("sleep", []);"#,
                        vec!["sleep".into()],
                    ),
                ),
            )
            .expect("run accepted");
        wait_for_disk_text(&dir.path().join("hello.rs"), edited);
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowStop {
                        request_id: RequestId::new("wf-stopped-refresh").expect("valid id"),
                    },
                ),
            )
            .expect("stop accepted");
        gate.open();
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);

        let snapshots = snapshots_for(&state.events_after(baseline), &opened.buffer_id);
        assert_eq!(snapshots.len(), 1, "a stopped run still refreshes");
        assert_eq!(snapshots[0].text, edited);
        assert_eq!(snapshots[0].version, 2);
    }

    #[test]
    fn p8_workflow_failure_raises_the_dirty_conflict_too() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = open_state_buffer(&state, &app, "hello.rs");
        apply_local_edit(&state, &app, &opened, "fn local() {}");
        let baseline = state.log.snapshot_sequence();
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_writes(vec![(
                dir.path().join("hello.rs"),
                Some(b"fn main() { /* before failure */ }\n".to_vec()),
            )]),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-failed-conflict",
                        r#"world.run("edit", []); throw new Error("boom");"#,
                        vec!["edit".into()],
                    ),
                ),
            )
            .expect("run accepted");
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);

        let events = state.events_after(baseline);
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::BufferConflict(conflict) if conflict.buffer_id == opened.buffer_id
        )));
        assert!(snapshots_for(&events, &opened.buffer_id).is_empty());
        assert_eq!(
            state
                .bootstrap()
                .buffers
                .into_iter()
                .find(|buffer| buffer.buffer_id == opened.buffer_id)
                .unwrap()
                .text,
            "fn local() {}\n"
        );
    }

    #[test]
    fn p8_workflow_stop_raises_the_dirty_conflict_too() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let dir = open_workspace(&state, &app);
        let opened = open_state_buffer(&state, &app, "hello.rs");
        let edited = "fn main() { /* before stop */ }\n";
        let gate = Arc::new(TestGate::default());
        state.install_workflow_host_factory(fake_workflow_factory(Arc::new(
            FakeWorkflowHost::with_gate_and_writes(
                Arc::clone(&gate),
                vec![(
                    dir.path().join("hello.rs"),
                    Some(edited.as_bytes().to_vec()),
                )],
            ),
        )));
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    workflow_run(
                        "wf-stopped-conflict",
                        r#"world.run("sleep", []);"#,
                        vec!["sleep".into()],
                    ),
                ),
            )
            .expect("run accepted");
        wait_for_disk_text(&dir.path().join("hello.rs"), edited);
        apply_local_edit(&state, &app, &opened, "fn local() {}");
        let baseline = state.log.snapshot_sequence();
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::WorkflowStop {
                        request_id: RequestId::new("wf-stopped-conflict").expect("valid id"),
                    },
                ),
            )
            .expect("stop accepted");
        gate.open();
        wait_for_finished_runs(&state, 1);
        wait_for_no_active_runs(&state);

        let events = state.events_after(baseline);
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            FrontendEvent::BufferConflict(conflict) if conflict.buffer_id == opened.buffer_id
        )));
        assert!(snapshots_for(&events, &opened.buffer_id).is_empty());
        assert_eq!(
            state
                .bootstrap()
                .buffers
                .into_iter()
                .find(|buffer| buffer.buffer_id == opened.buffer_id)
                .unwrap()
                .text,
            "fn local() {}\n"
        );
    }

    /// P1 hardening: the Tauri commands clone `DesktopState` into
    /// `tauri::async_runtime::spawn_blocking`, which only accepts a
    /// `Send + 'static` closure. This is the compile-time proof that the
    /// state (and the `AppHandle` it is paired with) can cross threads —
    /// every field is an `Arc`, so the clone shares the one instance.
    #[test]
    fn desktop_state_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<DesktopState>();
    }
}
