use crate::state::DesktopState;
use selora_protocol::{
    BootstrapState, EventEnvelope, FrontendCommand, FrontendEvent, ProtocolErrorDto,
    RequestEnvelope, PROTOCOL_VERSION,
};

#[tauri::command]
pub fn get_bootstrap_state(
    protocol_version: u32,
    state: tauri::State<'_, DesktopState>,
) -> Result<BootstrapState, ProtocolErrorDto> {
    if protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolErrorDto {
            code: "protocol_version_mismatch".into(),
            message: "Unsupported protocol version.".into(),
            retryable: false,
        });
    }
    Ok(state.bootstrap())
}

#[tauri::command]
pub fn dispatch_command(
    envelope: RequestEnvelope<FrontendCommand>,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<BootstrapState, ProtocolErrorDto> {
    // §2.4: the app handle rides the command so accepted commands can emit
    // `WorkspaceLayoutChanged` through the existing `record_event` path.
    state.dispatch(&app, envelope)
}

/// §3.4's replay command — the only Tauri command Z2 adds. Backed by the
/// recorded event log and its 1024-event retention cap; the gate (seeded
/// `null`) lives on the frontend.
#[tauri::command]
pub fn get_events_after(
    after_sequence: u64,
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<EventEnvelope<FrontendEvent>>, ProtocolErrorDto> {
    Ok(state.events_after(after_sequence))
}
