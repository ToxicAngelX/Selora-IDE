#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adapter;
mod commands;
mod events;
mod lsp_bridge;
mod security;
mod sessions;
mod state;
mod terminal;
mod workflow_broker;
mod workflow_command;
mod workflows_host;
mod workflows_process;
mod workspace;

use tauri::Manager;

/// Native Save / Discard / Cancel prompt for the close path. Lives in the
/// shell (not the WebView) so the prompt still appears while the editor is
/// dirty and no IPC is allowed to run.
fn prompt_close_choice(dirty: &[selora_protocol::BufferSnapshotDto]) -> state::CloseChoice {
    const MAX_LISTED: usize = 8;
    let mut description = format!(
        "{} unsaved {}:\n\n",
        dirty.len(),
        if dirty.len() == 1 {
            "file has"
        } else {
            "files have"
        }
    );
    for buffer in dirty.iter().take(MAX_LISTED) {
        description.push_str("  • ");
        description.push_str(buffer.path.as_str());
        description.push('\n');
    }
    if dirty.len() > MAX_LISTED {
        description.push_str(&format!("  … and {} more\n", dirty.len() - MAX_LISTED));
    }
    description.push_str("\nSave all changes before closing Selora?");

    match rfd::MessageDialog::new()
        .set_title("Unsaved changes")
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
            "Save".into(),
            "Discard".into(),
            "Cancel".into(),
        ))
        .show()
    {
        rfd::MessageDialogResult::Custom(label) if label == "Save" => state::CloseChoice::Save,
        rfd::MessageDialogResult::Custom(label) if label == "Discard" => {
            state::CloseChoice::Discard
        }
        _ => state::CloseChoice::Cancel,
    }
}

fn log_boot(msg: &str) {
    let log_path = std::env::temp_dir().join("selora_boot.log");
    let line = format!("[pid {}] {}\n", std::process::id(), msg);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

fn main() {
    // Note: Do not bind a restrictive JobObject here without breakaway flags;
    // WebView2 creates its own child process JobObjects and fails with 0x800700AA if restricted.

    std::panic::set_hook(Box::new(|info| {
        let log_path = std::env::temp_dir().join("selora_last_panic.log");
        let trace = format!(
            "PANIC: {info}\nBacktrace: {:?}",
            std::backtrace::Backtrace::capture()
        );
        let _ = std::fs::write(log_path, trace);
    }));

    log_boot("main() started");
    let state = state::DesktopState::new();
    log_boot("DesktopState created");
    tauri::Builder::default()
        // Must be the first plugin: a second process (for example a double-click
        // on an associated text file) hands its argv to this one and exits.
        // Without it, both processes race for the same WebView2 profile and one
        // dies during setup with a blank/frozen window.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        // About-dialog links leave through `FrontendCommand::OpenExternal`
        // (state.rs), which allowlists the host and calls this plugin's
        // `open_url` (ShellExecute, no shell string). The plugin's injected
        // JS click handler is off: a raw `<a target="_blank">` must stay
        // inert so every link crosses the Rust allowlist.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .manage(state)
        // §3.3 (docs/ui/PHASE_Z2_ARCHITECTURE.md): the close path runs in Rust
        // on `CloseRequested` — an IPC command cannot be guaranteed to flush
        // during webview teardown (WebView2 `beforeunload` is unreliable), so
        // the process that owns the files does the writes. Dirty buffers get a
        // native Save / Discard / Cancel prompt; cancel and failed saves call
        // `prevent_close`, so edits are never silently lost. The layout save
        // stays best-effort after the close is allowed.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app_state = window.app_handle().state::<state::DesktopState>();
                match app_state.handle_close_request(prompt_close_choice) {
                    state::CloseOutcome::Close { .. } => {
                        // Best effort: the layout must never block a close.
                        let _ = app_state.persist_layout();
                        // The app is going away: stop every ConPTY session and
                        // its process tree now rather than leaving them to the
                        // OS handle close.
                        app_state.shutdown_terminals();
                    }
                    state::CloseOutcome::Stay => api.prevent_close(),
                    state::CloseOutcome::SaveFailed { saved, message } => {
                        api.prevent_close();
                        let _ = rfd::MessageDialog::new()
                            .set_title("Could not save")
                            .set_description(format!(
                                "{message}\n\n{saved} buffer(s) were already saved. The window stays open."
                            ))
                            .set_buttons(rfd::MessageButtons::Ok)
                            .show();
                    }
                }
            }
        })
        .setup(|app| {
            log_boot("setup() started");
            #[cfg(debug_assertions)]
            let webview_url = tauri::WebviewUrl::default();
            #[cfg(not(debug_assertions))]
            let webview_url = tauri::WebviewUrl::App("index.html".into());

            let window = tauri::WebviewWindowBuilder::new(app, "main", webview_url)
                .title("Selora IDE")
                .inner_size(1440.0, 900.0)
                .min_inner_size(960.0, 640.0)
                .center()
                // §6.3: the builder is authoritative for this manually created
                // window — tauri.conf.json mirrors it for the same reason the
                // existing code duplicates title/size in both places.
                .decorations(false)
                .visible(true)
                .skip_taskbar(false)
                .devtools(true)
                .on_navigation(security::allow_shell_navigation)
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;

            log_boot("Window built successfully!");
            let _ = window.show();
            let _ = window.set_focus();
            log_boot("Window shown & focused!");

            // Phase Z3: the boot connection probe — read the keyring key,
            // probe the gateway (`list_models`), and announce the catalog
            // through `ModelsChanged` + `SeloraStatusChanged` (the agent
            // dock's model picker data path). A missing key or an
            // unreachable gateway is a silent `Disconnected` (the egui
            // boot's precedent).
            let desktop = app.state::<state::DesktopState>().inner().clone();
            let log = desktop.event_log();
            let engine = desktop.sessions();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                engine.connect_probe(log, handle).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            commands::dispatch_command,
            commands::update_settings,
            commands::get_events_after,
            commands::connect_api_key,
            commands::disconnect_api_key,
            commands::get_api_key_status,
            commands::fetch_models,
            commands::pick_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running Selora desktop shell");
}
