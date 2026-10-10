fn main() {
    // `WindowsAttributes::new()` embeds tauri-build's default application
    // manifest, which declares the Common-Controls v6 side-by-side
    // dependency. Without it the loader binds comctl32 v5 and the binary dies
    // at process start with STATUS_ENTRYPOINT_NOT_FOUND — tauri's default
    // `common-controls-v6` feature imports `TaskDialogIndirect` (verified on
    // the Z1 binary: exit code 0xC0000139 before this fix). Z1's
    // `new_without_app_manifest()` is superseded; the icon is kept.
    //
    // `static_vc_runtime(false)`: the config default (true) makes
    // tauri-build's `static_vcruntime` write a stub COFF object NAMED
    // `msvcrt.lib` into OUT_DIR and register that dir as a native link path
    // (`tauri-build-2.7.1/src/static_vcruntime.rs`). Cargo propagates
    // build-script link-search paths to every doctest compiled in the same
    // workspace session, so `cargo test --workspace` links e.g.
    // selora-editor's doctest with that path, link.exe picks the stub for
    // `/defaultlib:msvcrt`, rejects it (LNK4003 invalid library format), and
    // the real C runtime never resolves — 68 unresolved externals
    // (`__CxxFrameHandler3`, `_CxxThrowException`). Dynamic CRT instead: the
    // release binary then wants vcruntime140.dll, which the Windows
    // packaging slice owns alongside the WebView2 prerequisite policy
    // (PHASE_Z_TAURI_ARCHITECTURE.md).
    let windows = tauri_build::WindowsAttributes::new()
        .window_icon_path("icons/icon.ico")
        .static_vc_runtime(false);
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(windows)
            .app_manifest(tauri_build::AppManifest::new().commands(&[
                "get_bootstrap_state",
                "dispatch_command",
                "get_events_after",
                "connect_api_key",
                "disconnect_api_key",
                "get_api_key_status",
                "fetch_models",
            ])),
    )
    .expect("Tauri configuration and capability manifest must be valid");
}
