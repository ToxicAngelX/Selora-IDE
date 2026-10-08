use selora_protocol::{
    BootstrapState, CoreInstanceId, EventEnvelope, FrontendCommand, FrontendEvent,
    ProtocolErrorDto, RequestEnvelope, SettingsViewDto, PROTOCOL_VERSION,
};
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use uuid::Uuid;

#[derive(Clone)]
pub struct DesktopState {
    core_instance_id: CoreInstanceId,
    sequence: Arc<Mutex<u64>>,
    events: Arc<Mutex<Vec<EventEnvelope<FrontendEvent>>>>,
    settings: SettingsViewDto,
}

impl DesktopState {
    pub fn new() -> Self {
        let (settings, _) = selora_settings::Settings::load_or_default();
        Self::with_settings(SettingsViewDto {
            reduced_motion: settings.ui.reduce_motion,
            theme: settings.ui.theme,
        })
    }

    pub fn with_settings(settings: SettingsViewDto) -> Self {
        Self {
            core_instance_id: CoreInstanceId::new(Uuid::new_v4().to_string())
                .expect("uuid is valid"),
            sequence: Arc::new(Mutex::new(0)),
            events: Arc::new(Mutex::new(Vec::new())),
            settings,
        }
    }

    /// Core epoch accessor retained for reconnect/event adapter wiring.
    #[allow(dead_code)]
    pub fn core_instance_id(&self) -> CoreInstanceId {
        self.core_instance_id.clone()
    }

    pub fn bootstrap(&self) -> BootstrapState {
        BootstrapState {
            protocol_version: PROTOCOL_VERSION,
            core_instance_id: self.core_instance_id.clone(),
            snapshot_sequence: *self.sequence.lock().expect("sequence lock"),
            workspace: selora_protocol::WorkspaceDto {
                id: selora_protocol::WorkspaceId::new("no-workspace").expect("valid id"),
                core_instance_id: self.core_instance_id.clone(),
                root: None,
                display_name: "No workspace open".into(),
            },
            buffers: Vec::new(),
            active_buffer_id: None,
            file_tree: selora_protocol::FileTreeDto { root: None },
            models: Vec::new(),
            active_model_id: None,
            connection: selora_protocol::ConnectionStatus::Disconnected,
            chat: selora_protocol::ChatStateDto {
                sessions: Vec::new(),
                messages: Vec::new(),
                active_session_id: None,
                active_request_id: None,
                active_turn_id: None,
                pending_approval: None,
            },
            settings: self.settings.clone(),
            layout: selora_protocol::LayoutDto {
                primary_sidebar_width_px: 280.0,
                agent_dock_width_px: 360.0,
                agent_dock_open: true,
                bottom_dock_height_px: 220.0,
                bottom_dock_open: false,
                bottom_view: selora_protocol::SurfaceId::Output,
                editor_splits: 1,
            },
            git: selora_protocol::GitStatusDto {
                branch: None,
                changed_paths: Vec::new(),
                staged_paths: Vec::new(),
            },
            diagnostics: Vec::new(),
            feature_flags: selora_protocol::FeatureFlagsDto {
                agent: false,
                git: false,
                search: false,
                editor: false,
                later_slices: vec![
                    "workspace-picker".into(),
                    "editor-bridge".into(),
                    "agent-dock".into(),
                ],
            },
        }
    }

    pub fn dispatch(
        &self,
        envelope: RequestEnvelope<FrontendCommand>,
    ) -> Result<BootstrapState, ProtocolErrorDto> {
        if envelope.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolErrorDto {
                code: "protocol_version_mismatch".into(),
                message: "Unsupported protocol version.".into(),
                retryable: false,
            });
        }
        if envelope.core_instance_id.as_ref() != Some(&self.core_instance_id) {
            return Err(ProtocolErrorDto {
                code: "stale_core_instance".into(),
                message: "Request a fresh bootstrap snapshot.".into(),
                retryable: true,
            });
        }
        if envelope
            .workspace_id
            .as_ref()
            .map(selora_protocol::WorkspaceId::as_str)
            != Some("no-workspace")
            || envelope.session_id.is_some()
        {
            return Err(ProtocolErrorDto {
                code: "stale_workspace_or_session".into(),
                message: "Request a fresh bootstrap snapshot.".into(),
                retryable: true,
            });
        }
        match envelope.payload {
            FrontendCommand::GetBootstrapState => Ok(self.bootstrap()),
            other => Err(crate::adapter::unsupported(&other)),
        }
    }

    #[allow(dead_code)]
    pub fn record_event(
        &self,
        app: &tauri::AppHandle,
        payload: FrontendEvent,
        request_id: Option<selora_protocol::RequestId>,
    ) {
        let mut sequence = self.sequence.lock().expect("sequence lock");
        *sequence = sequence.saturating_add(1);
        self.events.lock().expect("event lock").push(EventEnvelope {
            protocol_version: PROTOCOL_VERSION,
            workspace_id: None,
            core_instance_id: self.core_instance_id.clone(),
            sequence: *sequence,
            request_id,
            session_id: None,
            payload,
        });
        if let Some(event) = self.events.lock().expect("event lock").last() {
            let _ = app.emit("selora://event", event);
        }
    }

    #[allow(dead_code)]
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
