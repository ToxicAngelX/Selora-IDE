use selora_protocol::{FrontendCommand, ProtocolErrorDto};

pub fn unsupported(_command: &FrontendCommand) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "unsupported_phase_slice".into(),
        message: "This command is scaffolded for a later Phase Z slice.".into(),
        retryable: false,
    }
}
