use crate::state::DesktopState;
use selora_protocol::{
    BootstrapState, FrontendCommand, ProtocolErrorDto, RequestEnvelope, PROTOCOL_VERSION,
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
    state: tauri::State<'_, DesktopState>,
) -> Result<BootstrapState, ProtocolErrorDto> {
    state.dispatch(envelope)
}
