use selora_protocol::{
    clamp_region_size, BootstrapState, CoreInstanceId, EventEnvelope, FrontendCommand,
    FrontendEvent, LayoutConstraintsDto, LayoutDto, ProtocolErrorDto, RegionId, RequestEnvelope,
    SessionId, SettingsViewDto, SurfaceId, ToastDto, ToastSeverity, PROTOCOL_VERSION,
};
use selora_settings::LayoutSettings;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::Emitter;
use uuid::Uuid;

use crate::sessions::{ApiKeySource, EngineConfig, SessionEngine};

/// §3.4 (docs/ui/PHASE_Z2_ARCHITECTURE.md): keep the most recent 1024
/// events; a replay request older than the oldest retained event simply
/// yields a gap at the gate and the frontend re-bootstraps — the failure
/// mode is already designed for.
const EVENT_RETENTION_CAP: usize = 1024;

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

#[derive(Clone)]
pub struct DesktopState {
    log: Arc<EventLog>,
    settings: SettingsViewDto,
    /// §2.4: the authoritative in-memory layout, seeded at startup from
    /// settings (§4) and mutated only by accepted `ToggleSurface` /
    /// `ResizeRegion` commands. The frontend applies changes only from the
    /// `WorkspaceLayoutChanged` event.
    layout: Arc<Mutex<LayoutDto>>,
    /// The persistence seam (§4.5).
    layout_store: Arc<LayoutStore>,
    /// The Z3 session engine (chat + agent turns) behind the agent dock.
    sessions: Arc<SessionEngine>,
}

impl DesktopState {
    /// The production constructor: `%APPDATA%\selora-ide\settings.json` (or
    /// the `SELORA_SETTINGS_DIR` override).
    pub fn new() -> Self {
        let state = Self::from_settings_path(selora_settings::Settings::default_path());
        // Z3's stand-in for the Z4 folder picker: point the agent's tools at
        // a real tree via `SELORA_WORKSPACE_ROOT` (an existing directory).
        // Without it `SendAgent` answers the typed `no_workspace` error.
        if let Ok(root) = std::env::var("SELORA_WORKSPACE_ROOT") {
            let root = PathBuf::from(root.trim());
            if !root.as_os_str().is_empty() && root.is_dir() {
                state.sessions().set_workspace_root(Some(root));
            }
        } else if let Ok(root) = std::env::current_dir() {
            state.sessions().set_workspace_root(Some(root));
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
            settings: SettingsViewDto {
                reduced_motion: settings.ui.reduce_motion,
                theme: settings.ui.theme.clone(),
                // §5.1: the rebindable pair, populated from settings.keybinds
                // so a user's egui-side rebind carries into the web shell.
                keybind_command_palette: settings.keybinds.command_palette.clone(),
                keybind_toggle_chat: settings.keybinds.toggle_chat.clone(),
            },
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
        }
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

    pub fn bootstrap(&self) -> BootstrapState {
        let ws_root = self.sessions.workspace_root();
        let display_name = ws_root
            .as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "No workspace open".into());
        let root_str = ws_root.map(|p| p.to_string_lossy().to_string());
        BootstrapState {
            protocol_version: PROTOCOL_VERSION,
            core_instance_id: self.log.core_instance_id(),
            snapshot_sequence: self.log.snapshot_sequence(),
            workspace: selora_protocol::WorkspaceDto {
                id: selora_protocol::WorkspaceId::new("no-workspace").expect("valid id"),
                core_instance_id: self.log.core_instance_id(),
                root: root_str,
                display_name,
            },
            buffers: Vec::new(),
            active_buffer_id: None,
            file_tree: selora_protocol::FileTreeDto { root: None },
            models: self.sessions.models_dto(),
            active_model_id: self.sessions.active_model_id(),
            connection: self.sessions.connection(),
            chat: self.sessions.chat_state(),
            settings: self.settings.clone(),
            // §1.1: the Z1 stub values (280/220, agent dock open) were
            // diagnostic placeholders — the layout now comes from the
            // persisted settings (§4).
            layout: self.layout.lock().expect("layout lock").clone(),
            // §5.1: the bounds ride the bootstrap so no bound is ever a JS
            // literal.
            layout_constraints: LayoutConstraintsDto::from_constants(),
            git: selora_protocol::GitStatusDto {
                branch: None,
                changed_paths: Vec::new(),
                staged_paths: Vec::new(),
            },
            diagnostics: Vec::new(),
            feature_flags: selora_protocol::FeatureFlagsDto {
                // Z3: the agent dock is live (chat + agent turns + approvals).
                agent: true,
                git: false,
                search: false,
                editor: false,
                later_slices: vec!["workspace-picker".into(), "editor-bridge".into()],
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
        if envelope
            .workspace_id
            .as_ref()
            .map(selora_protocol::WorkspaceId::as_str)
            != Some("no-workspace")
            || (envelope.session_id.is_some() && !session_scoped)
        {
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
            // Everything outside Z2 keeps the typed unsupported_phase_slice
            // error (the contract's "Explicit non-goals" list; adapter.rs).
            other => Err(crate::adapter::unsupported(&other)),
        }
    }

    /// §2.2's routing table.
    fn apply_toggle(&self, surface: SurfaceId) -> LayoutDto {
        let mut layout = self.layout.lock().expect("layout lock");
        match surface {
            SurfaceId::Files | SurfaceId::Search | SurfaceId::Git | SurfaceId::Settings => {
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
    use selora_protocol::{FrontendCommand, RequestId, WorkspaceId};

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
    fn commands_outside_z2_stay_unsupported() {
        let app = mock_app_handle();
        let state = DesktopState::with_settings(selora_settings::Settings::default());
        let error = state
            .dispatch(&app, envelope(&state, FrontendCommand::OpenFolder))
            .expect_err("OpenFolder is a later slice");
        assert_eq!(error.code, "unsupported_phase_slice");
        assert!(!error.retryable);
        assert!(state.events_after(0).is_empty());
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
}
