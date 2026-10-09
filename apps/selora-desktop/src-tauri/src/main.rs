#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adapter;
mod commands;
mod events;
mod security;
mod state;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .manage(state::DesktopState::new())
        // §3.3 (docs/ui/PHASE_Z2_ARCHITECTURE.md): the close-time save runs
        // in Rust on `CloseRequested` — an IPC command cannot be guaranteed
        // to flush during webview teardown (WebView2 `beforeunload` is
        // unreliable), so the process that owns the file does the write.
        // The save is synchronous, before the close is allowed; a failure
        // here has no surface left to toast to, and the last explicit
        // `SaveLayout` is the fallback.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let app_state = window.app_handle().state::<state::DesktopState>();
                let _ = app_state.persist_layout();
            }
        })
        .setup(|app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Selora IDE")
            .inner_size(1440.0, 900.0)
            .min_inner_size(960.0, 640.0)
            // §6.3: the builder is authoritative for this manually created
            // window — tauri.conf.json mirrors it for the same reason the
            // existing code duplicates title/size in both places.
            .decorations(false)
            .on_navigation(security::allow_shell_navigation)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            commands::dispatch_command,
            commands::get_events_after
        ])
        .run(tauri::generate_context!())
        .expect("error while running Selora desktop shell");
}
