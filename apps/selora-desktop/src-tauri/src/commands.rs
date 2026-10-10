use crate::state::DesktopState;
use selora_protocol::{
    ApiKeyStatusDto, BootstrapState, ConnectResultDto, ConnectionStatus, EventEnvelope,
    FrontendCommand, FrontendEvent, ModelDto, ModelsDto, ProtocolErrorDto, RequestEnvelope,
    SeloraStatusDto, PROTOCOL_VERSION,
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

#[tauri::command]
pub async fn connect_api_key(
    api_key: String,
    base_url: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<ConnectResultDto, ProtocolErrorDto> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err(ProtocolErrorDto {
            code: "invalid_api_key".into(),
            message: "API key cannot be empty.".into(),
            retryable: false,
        });
    }

    let engine = state.sessions();
    let final_base_url = if let Some(ref url) = base_url {
        let trimmed_url = url.trim();
        if !trimmed_url.is_empty() {
            if !trimmed_url.starts_with("http://") && !trimmed_url.starts_with("https://") {
                return Err(ProtocolErrorDto {
                    code: "invalid_base_url".into(),
                    message: "Provider Gateway URL must start with http:// or https://".into(),
                    retryable: false,
                });
            }
            engine.set_base_url(trimmed_url.to_string());
            let _ = state.update_selora_settings(Some(trimmed_url.to_string()), None);
            trimmed_url.to_string()
        } else {
            engine.base_url()
        }
    } else {
        engine.base_url()
    };

    // Store in OS keyring (Windows Credential Manager)
    if let Err(err) = selora_settings::keyring::api_key_set(key) {
        return Err(ProtocolErrorDto {
            code: "keyring_write_failed".into(),
            message: format!("Could not save API key to secure credential manager: {err}"),
            retryable: true,
        });
    }

    // Keep current process env in sync so in-flight engine turns use the new key
    std::env::set_var("SELORA_API_KEY", key);

    // Probe the gateway with the new key and base_url
    let client = selora_ai::SeloraClient::new(&final_base_url, key);
    let models = match client.list_models().await {
        Ok(models) if !models.is_empty() => models,
        _ => synthesize_provider_models(&final_base_url),
    };

    let active_model = engine
        .active_model_id()
        .filter(|current| models.iter().any(|m| m.id == *current || m.id.eq_ignore_ascii_case(current)))
        .or_else(|| models.first().map(|m| m.id.clone()));

    let _ = state.update_selora_settings(Some(final_base_url.clone()), active_model.clone());

    engine.set_models_and_connection(
        models.clone(),
        ConnectionStatus::Connected,
        active_model.clone(),
    );

    let log = state.event_log();
    let model_dtos: Vec<ModelDto> = models.iter().map(crate::sessions::model_dto).collect();

    log.record(
        &app,
        FrontendEvent::ModelsChanged(ModelsDto {
            models: model_dtos,
            active_model_id: active_model.clone(),
        }),
        None,
    );
    log.record(
        &app,
        FrontendEvent::SeloraStatusChanged(SeloraStatusDto {
            status: ConnectionStatus::Connected,
            active_model_id: active_model.clone(),
        }),
        None,
    );

    Ok(ConnectResultDto {
        connected: true,
        models_count: models.len(),
        active_model_id: active_model,
        base_url: final_base_url,
    })
}

#[tauri::command]
pub async fn fetch_models(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<ModelDto>, ProtocolErrorDto> {
    let engine = state.sessions();
    let api_key = engine.fetch_api_key()?;
    let base_url = engine.base_url();
    let client = selora_ai::SeloraClient::new(&base_url, &api_key);
    let models = match client.list_models().await {
        Ok(models) if !models.is_empty() => models,
        _ => synthesize_provider_models(&base_url),
    };

    let active_model = engine
        .active_model_id()
        .filter(|current| models.iter().any(|m| m.id == *current || m.id.eq_ignore_ascii_case(current)))
        .or_else(|| models.first().map(|m| m.id.clone()));

    let _ = state.update_selora_settings(Some(base_url.clone()), active_model.clone());

    engine.set_models_and_connection(
        models.clone(),
        ConnectionStatus::Connected,
        active_model.clone(),
    );

    let log = state.event_log();
    let model_dtos: Vec<ModelDto> = models.iter().map(crate::sessions::model_dto).collect();

    log.record(
        &app,
        FrontendEvent::ModelsChanged(ModelsDto {
            models: model_dtos.clone(),
            active_model_id: active_model.clone(),
        }),
        None,
    );
    log.record(
        &app,
        FrontendEvent::SeloraStatusChanged(SeloraStatusDto {
            status: ConnectionStatus::Connected,
            active_model_id: active_model,
        }),
        None,
    );

    Ok(model_dtos)
}

#[tauri::command]
pub fn disconnect_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), ProtocolErrorDto> {
    if let Err(err) = selora_settings::keyring::api_key_delete() {
        return Err(ProtocolErrorDto {
            code: "keyring_delete_failed".into(),
            message: format!("Could not remove API key from keyring: {err}"),
            retryable: true,
        });
    }

    let engine = state.sessions();
    engine.set_models_and_connection(Vec::new(), ConnectionStatus::Disconnected, None);
    std::env::remove_var("SELORA_API_KEY");
    std::env::remove_var("OPENAI_API_KEY");

    let log = state.event_log();
    log.record(
        &app,
        FrontendEvent::ModelsChanged(ModelsDto {
            models: Vec::new(),
            active_model_id: None,
        }),
        None,
    );
    log.record(
        &app,
        FrontendEvent::SeloraStatusChanged(SeloraStatusDto {
            status: ConnectionStatus::Disconnected,
            active_model_id: None,
        }),
        None,
    );

    Ok(())
}

#[tauri::command]
pub fn get_api_key_status(
    state: tauri::State<'_, DesktopState>,
) -> Result<ApiKeyStatusDto, ProtocolErrorDto> {
    let engine = state.sessions();
    let base_url = engine.base_url();
    let active_model_id = engine.active_model_id();
    let connection = engine.connection();

    let (has_key, source, key_hint) = match check_keyring() {
        (true, source, key_hint) => (true, source, key_hint),
        _ => {
            if let Ok(env_key) = std::env::var("SELORA_API_KEY") {
                let trimmed = env_key.trim();
                if !trimmed.is_empty() {
                    (true, "env: SELORA_API_KEY".to_string(), Some(mask_key(trimmed)))
                } else {
                    (false, "none".to_string(), None)
                }
            } else if let Ok(env_key) = std::env::var("OPENAI_API_KEY") {
                let trimmed = env_key.trim();
                if !trimmed.is_empty() {
                    (true, "env: OPENAI_API_KEY".to_string(), Some(mask_key(trimmed)))
                } else {
                    (false, "none".to_string(), None)
                }
            } else {
                (false, "none".to_string(), None)
            }
        }
    };

    Ok(ApiKeyStatusDto {
        connected: connection == ConnectionStatus::Connected,
        has_key,
        source,
        base_url,
        key_hint,
        active_model_id,
    })
}

fn check_keyring() -> (bool, String, Option<String>) {
    match selora_settings::keyring::api_key_get() {
        Ok(Some(key)) => (true, "keyring".to_string(), Some(mask_key(&key))),
        Ok(None) => (false, "none".to_string(), None),
        Err(_) => (false, "keyring_error".to_string(), None),
    }
}

fn mask_key(key: &str) -> String {
    if key.len() > 8 {
        format!("{}••••{}", &key[..4], &key[key.len().saturating_sub(4)..])
    } else {
        "••••".to_string()
    }
}

pub(crate) fn synthesize_provider_models(base_url: &str) -> Vec<selora_ai::Model> {
    if base_url.contains("bigmodel.cn") || base_url.contains("glm") {
        vec![
            selora_ai::Model {
                id: "glm-4-plus".into(),
                display_name: "GLM-4 Plus".into(),
                context_window: 128_000,
                price_per_1k_input: 0.005,
                price_per_1k_output: 0.005,
                supports_streaming: true,
                supports_tools: true,
                family: "glm".into(),
            },
            selora_ai::Model {
                id: "glm-4-flash".into(),
                display_name: "GLM-4 Flash (Free)".into(),
                context_window: 128_000,
                price_per_1k_input: 0.0,
                price_per_1k_output: 0.0,
                supports_streaming: true,
                supports_tools: true,
                family: "glm".into(),
            },
            selora_ai::Model {
                id: "glm-zero-preview".into(),
                display_name: "GLM-Zero Reasoning".into(),
                context_window: 128_000,
                price_per_1k_input: 0.008,
                price_per_1k_output: 0.008,
                supports_streaming: true,
                supports_tools: true,
                family: "glm".into(),
            },
        ]
    } else if base_url.contains("deepseek") {
        vec![
            selora_ai::Model {
                id: "deepseek-chat".into(),
                display_name: "DeepSeek-V3".into(),
                context_window: 64_000,
                price_per_1k_input: 0.00014,
                price_per_1k_output: 0.00028,
                supports_streaming: true,
                supports_tools: true,
                family: "deepseek".into(),
            },
            selora_ai::Model {
                id: "deepseek-reasoner".into(),
                display_name: "DeepSeek-R1".into(),
                context_window: 64_000,
                price_per_1k_input: 0.00055,
                price_per_1k_output: 0.00219,
                supports_streaming: true,
                supports_tools: true,
                family: "deepseek".into(),
            },
        ]
    } else if base_url.contains("openrouter") {
        vec![
            selora_ai::Model {
                id: "anthropic/claude-3.7-sonnet".into(),
                display_name: "Claude 3.7 Sonnet".into(),
                context_window: 200_000,
                price_per_1k_input: 0.003,
                price_per_1k_output: 0.015,
                supports_streaming: true,
                supports_tools: true,
                family: "claude".into(),
            },
            selora_ai::Model {
                id: "openai/gpt-4o".into(),
                display_name: "GPT-4o".into(),
                context_window: 128_000,
                price_per_1k_input: 0.0025,
                price_per_1k_output: 0.01,
                supports_streaming: true,
                supports_tools: true,
                family: "gpt".into(),
            },
        ]
    } else {
        vec![
            selora_ai::Model {
                id: "gpt-4o".into(),
                display_name: "GPT-4o".into(),
                context_window: 128_000,
                price_per_1k_input: 0.0025,
                price_per_1k_output: 0.01,
                supports_streaming: true,
                supports_tools: true,
                family: "gpt".into(),
            },
            selora_ai::Model {
                id: "gpt-4o-mini".into(),
                display_name: "GPT-4o Mini".into(),
                context_window: 128_000,
                price_per_1k_input: 0.00015,
                price_per_1k_output: 0.0006,
                supports_streaming: true,
                supports_tools: true,
                family: "gpt".into(),
            },
            selora_ai::Model {
                id: "claude-3-5-sonnet".into(),
                display_name: "Claude 3.5 Sonnet".into(),
                context_window: 200_000,
                price_per_1k_input: 0.003,
                price_per_1k_output: 0.015,
                supports_streaming: true,
                supports_tools: true,
                family: "claude".into(),
            },
        ]
    }
}

