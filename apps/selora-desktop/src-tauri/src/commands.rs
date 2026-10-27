use crate::state::DesktopState;
use selora_protocol::{
    ApiKeyStatusDto, BootstrapState, ConnectResultDto, ConnectionStatus, EventEnvelope,
    FrontendCommand, FrontendEvent, ModelDto, ModelsDto, ProtocolErrorDto, RequestEnvelope,
    SeloraStatusDto, SettingsPatchDto, PROTOCOL_VERSION,
};

/// P1 hardening: a synchronous Tauri command runs on the main/IPC thread, so
/// every blocking body (tree walks, libgit2 status, file reads/writes + fsync,
/// the `rg` subprocess, settings and keyring round-trips) froze the WebView
/// while it ran. The protocol commands are `async` now and hand that body to
/// [`tauri::async_runtime::spawn_blocking`] — the `pick_folder` pattern — so
/// the IPC thread only marshals the envelope.
///
/// `spawn_blocking` needs a `Send + 'static` closure, which is why the
/// commands clone the `DesktopState` (it is `Clone`, all fields are `Arc`s)
/// and the `AppHandle` instead of borrowing the `tauri::State` across the
/// await. A join failure becomes a typed, retryable protocol error; the wire
/// shape of the other commands is unchanged.
async fn offload<T, F>(work: F) -> Result<T, ProtocolErrorDto>
where
    F: FnOnce() -> Result<T, ProtocolErrorDto> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|cause| ProtocolErrorDto {
            code: "command_task_failed".into(),
            message: format!("The command task could not be joined: {cause}"),
            retryable: true,
        })?
}

/// The bootstrap body, kept out of the command so §2.3's version gate and the
/// snapshot generation can be exercised directly through [`offload`].
fn bootstrap_snapshot(
    state: &DesktopState,
    protocol_version: u32,
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
pub async fn get_bootstrap_state(
    protocol_version: u32,
    state: tauri::State<'_, DesktopState>,
) -> Result<BootstrapState, ProtocolErrorDto> {
    // The bootstrap walks the tree and reads libgit2 status, so it is not the
    // cheap accessor it looks like: generate it off the IPC thread too.
    let state = state.inner().clone();
    offload(move || bootstrap_snapshot(&state, protocol_version)).await
}

/// Phase Z4 — the native folder picker for the Open Folder flow. A plain
/// Tauri command (not a protocol command): it only chooses a path; the
/// frontend then sends OpenFolderAt { root }, which is what the typed
/// boundary validates and the workspace service opens.
#[tauri::command]
pub async fn pick_folder() -> Result<Option<String>, ProtocolErrorDto> {
    let picked = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Open Folder in Selora")
            .pick_folder()
    })
    .await
    .map_err(|cause| ProtocolErrorDto {
        code: "picker_failed".into(),
        message: format!("Folder picker failed: {cause}"),
        retryable: true,
    })?;
    Ok(picked.map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn dispatch_command(
    envelope: RequestEnvelope<FrontendCommand>,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<Option<BootstrapState>, ProtocolErrorDto> {
    // §2.4: the app handle rides the command so accepted commands can emit
    // `WorkspaceLayoutChanged` through the existing `record_event` path.
    // `dispatch` is synchronous and does the command's whole body (tree walks,
    // libgit2, file I/O, the `rg` subprocess), so it runs on the blocking pool.
    //
    // The bridge ignores the return value of the two per-keystroke commands.
    // They take the quiet path so the IPC body is a bare `null` instead of a
    // full bootstrap snapshot (which clones every open buffer's text).
    let state = state.inner().clone();
    let quiet = matches!(
        &envelope.payload,
        FrontendCommand::ApplyEditorChange { .. } | FrontendCommand::SetSelection { .. }
    );
    offload(move || {
        if quiet {
            state.dispatch_quiet(&app, envelope)?;
            Ok(None)
        } else {
            state.dispatch(&app, envelope).map(Some)
        }
    })
    .await
}

/// Persist a settings patch through the same validated path as the protocol
/// command and publish the refreshed bootstrap to every consumer.
#[tauri::command]
pub async fn update_settings(
    patch: SettingsPatchDto,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<BootstrapState, ProtocolErrorDto> {
    // `apply_settings_update` re-reads and atomically rewrites `settings.json`
    // (temp file + fsync + rename): disk work, so never on the IPC thread.
    let state = state.inner().clone();
    offload(move || state.apply_settings_update(&app, patch, None)).await
}

/// Boot-trace marker: the frontend calls this once its bootstrap + replay flow
/// reaches `ready`. It writes one line to `%TEMP%\selora_boot.log`, so support
/// logs and `scripts/perf.ps1` can separate process/window creation from the
/// first usable shell without changing app state.
#[tauri::command]
pub fn frontend_ready() {
    crate::log_boot("frontend bootstrap ready");
}

/// §3.4's replay command — the only Tauri command Z2 adds. Backed by the
/// recorded event log and its 1024-event retention cap; the gate (seeded
/// `null`) lives on the frontend.
#[tauri::command]
pub async fn get_events_after(
    after_sequence: u64,
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<EventEnvelope<FrontendEvent>>, ProtocolErrorDto> {
    // Replay clones up to 1024 envelopes under the log mutex; keep it off the
    // IPC thread so the lock is never held against the UI.
    let state = state.inner().clone();
    offload(move || Ok(state.events_after(after_sequence))).await
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

    // The engine reads the key from the keyring at send time (or from its
    // own config) — it is deliberately never copied into the process env.

    // Probe the gateway with the new key and base_url
    let client = selora_ai::SeloraClient::new(&final_base_url, key);
    let models = match client.list_models().await {
        Ok(models) if !models.is_empty() => models,
        _ => synthesize_provider_models(&final_base_url),
    };

    let active_model = engine
        .active_model_id()
        .filter(|current| {
            models
                .iter()
                .any(|m| m.id == *current || m.id.eq_ignore_ascii_case(current))
        })
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
        .filter(|current| {
            models
                .iter()
                .any(|m| m.id == *current || m.id.eq_ignore_ascii_case(current))
        })
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
pub async fn disconnect_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), ProtocolErrorDto> {
    let state = state.inner().clone();
    offload(move || disconnect_api_key_now(&state, &app)).await
}

/// The blocking half of [`disconnect_api_key`]: the Credential Manager delete
/// and the in-memory engine reset, run off the IPC thread.
fn disconnect_api_key_now<R: tauri::Runtime>(
    state: &DesktopState,
    app: &tauri::AppHandle<R>,
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

    let log = state.event_log();
    log.record(
        app,
        FrontendEvent::ModelsChanged(ModelsDto {
            models: Vec::new(),
            active_model_id: None,
        }),
        None,
    );
    log.record(
        app,
        FrontendEvent::SeloraStatusChanged(SeloraStatusDto {
            status: ConnectionStatus::Disconnected,
            active_model_id: None,
        }),
        None,
    );

    Ok(())
}

#[tauri::command]
pub async fn get_api_key_status(
    state: tauri::State<'_, DesktopState>,
) -> Result<ApiKeyStatusDto, ProtocolErrorDto> {
    // A keyring read can take a syscall round-trip through the Credential
    // Manager; resolve the status off the IPC thread.
    let state = state.inner().clone();
    offload(move || Ok(api_key_status(&state))).await
}

fn api_key_status(state: &DesktopState) -> ApiKeyStatusDto {
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
                    (
                        true,
                        "env: SELORA_API_KEY".to_string(),
                        Some(mask_key(trimmed)),
                    )
                } else {
                    (false, "none".to_string(), None)
                }
            } else if let Ok(env_key) = std::env::var("OPENAI_API_KEY") {
                let trimmed = env_key.trim();
                if !trimmed.is_empty() {
                    (
                        true,
                        "env: OPENAI_API_KEY".to_string(),
                        Some(mask_key(trimmed)),
                    )
                } else {
                    (false, "none".to_string(), None)
                }
            } else {
                (false, "none".to_string(), None)
            }
        }
    };

    ApiKeyStatusDto {
        connected: connection == ConnectionStatus::Connected,
        has_key,
        source,
        base_url,
        key_hint,
        active_model_id,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use selora_protocol::{CoreInstanceId, RequestId, SurfaceId, WorkspaceId};

    /// A hermetic state: the fixed test key means no test touches the real
    /// Credential Manager (the `DesktopState` test precedent).
    fn test_state() -> DesktopState {
        DesktopState::with_settings(selora_settings::Settings::default())
    }

    fn mock_app_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
        tauri::test::mock_app().handle().clone()
    }

    fn envelope(
        state: &DesktopState,
        payload: FrontendCommand,
    ) -> RequestEnvelope<FrontendCommand> {
        RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            workspace_id: Some(WorkspaceId::new("no-workspace").expect("valid id")),
            core_instance_id: Some(state.core_instance_id()),
            request_id: RequestId::new("request-1").expect("valid id"),
            session_id: None,
            payload,
        }
    }

    // ------------------------------------------------------------------
    // P1: the offloaded path answers exactly what the inline path did.
    // ------------------------------------------------------------------

    #[test]
    fn dispatch_offload_returns_the_same_bootstrap() {
        let state = test_state();
        let app = mock_app_handle();
        let request = envelope(&state, FrontendCommand::GetBootstrapState);

        let inline = state
            .dispatch(&app, request.clone())
            .expect("inline dispatch");
        let offloaded = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            let app = app.clone();
            move || state.dispatch(&app, request)
        }))
        .expect("offloaded dispatch");

        assert_eq!(offloaded, inline);
        assert_eq!(offloaded.protocol_version, PROTOCOL_VERSION);
        assert_eq!(offloaded.snapshot_sequence, inline.snapshot_sequence);
    }

    #[test]
    fn dispatch_offload_preserves_the_typed_error() {
        let state = test_state();
        let app = mock_app_handle();
        let mut request = envelope(&state, FrontendCommand::GetBootstrapState);
        request.core_instance_id = Some(CoreInstanceId::new("core_stale").expect("valid id"));

        let inline = state
            .dispatch(&app, request.clone())
            .expect_err("a stale core instance is rejected");
        let offloaded = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            let app = app.clone();
            move || state.dispatch(&app, request)
        }))
        .expect_err("a stale core instance is rejected off-thread");

        assert_eq!(offloaded, inline);
        assert_eq!(offloaded.code, "stale_core_instance");
    }

    #[test]
    fn bootstrap_offload_keeps_the_version_gate_and_the_snapshot() {
        let state = test_state();

        let mismatch = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            move || bootstrap_snapshot(&state, PROTOCOL_VERSION + 1)
        }))
        .expect_err("an unsupported protocol version is rejected");
        assert_eq!(mismatch.code, "protocol_version_mismatch");

        let snapshot = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            move || bootstrap_snapshot(&state, PROTOCOL_VERSION)
        }))
        .expect("bootstrap");
        assert_eq!(snapshot, state.bootstrap());
    }

    #[test]
    fn events_after_offload_replays_the_recorded_log() {
        let state = test_state();
        let app = mock_app_handle();
        // A toggle records exactly one `WorkspaceLayoutChanged`.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ToggleSurface {
                        surface: SurfaceId::Agent,
                    },
                ),
            )
            .expect("toggle dispatch");

        let inline = state.events_after(0);
        let offloaded = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            move || Ok::<_, ProtocolErrorDto>(state.events_after(0))
        }))
        .expect("replay");

        assert_eq!(offloaded, inline);
        assert_eq!(offloaded.len(), 1);
    }
}
