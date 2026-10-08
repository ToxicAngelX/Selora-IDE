#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adapter;
mod commands;
mod events;
mod security;
mod state;

fn main() {
    tauri::Builder::default()
        .manage(state::DesktopState::new())
        .setup(|app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Selora IDE · Phase Z diagnostic shell")
            .inner_size(1440.0, 900.0)
            .min_inner_size(960.0, 640.0)
            .on_navigation(security::allow_shell_navigation)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            commands::dispatch_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running Selora desktop shell");
}
