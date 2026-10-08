fn main() {
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest()
        .window_icon_path("icons/icon.ico");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(windows)
            .app_manifest(
                tauri_build::AppManifest::new()
                    .commands(&["get_bootstrap_state", "dispatch_command"]),
            ),
    )
    .expect("Tauri configuration and capability manifest must be valid");
}
