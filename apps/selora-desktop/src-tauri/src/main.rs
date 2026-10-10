#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adapter;
mod commands;
mod events;
mod security;
mod sessions;
mod state;

use tauri::Manager;

#[cfg(windows)]
fn bind_to_job_object() {
    #[repr(C)]
    struct JobobjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }
    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }
    #[repr(C)]
    struct JobobjectExtendedLimitInformation {
        basic_limit_information: JobobjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_limit: usize,
        peak_job_memory_limit: usize,
    }
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

    extern "system" {
        fn CreateJobObjectW(lp_job_attributes: *const std::ffi::c_void, lp_name: *const u16) -> *mut std::ffi::c_void;
        fn SetInformationJobObject(
            h_job: *mut std::ffi::c_void,
            job_object_info_class: i32,
            lp_job_object_info: *const std::ffi::c_void,
            cb_job_object_info_length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(h_job: *mut std::ffi::c_void, h_process: *mut std::ffi::c_void) -> i32;
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !job.is_null() {
            let mut info: JobobjectExtendedLimitInformation = std::mem::zeroed();
            info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JobobjectExtendedLimitInformation>() as u32,
            );
            if ok != 0 {
                let _ = AssignProcessToJobObject(job, GetCurrentProcess());
            }
            // Retain job handle so it remains active for process duration
            let _ = job;
        }
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
        let trace = format!("PANIC: {info}\nBacktrace: {:?}", std::backtrace::Backtrace::capture());
        let _ = std::fs::write(log_path, trace);
    }));

    log_boot("main() started");
    let state = state::DesktopState::new();
    log_boot("DesktopState created");
    tauri::Builder::default()
        .manage(state)
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
            log_boot("setup() started");
            #[cfg(debug_assertions)]
            let webview_url = tauri::WebviewUrl::default();
            #[cfg(not(debug_assertions))]
            let webview_url = tauri::WebviewUrl::App("index.html".into());

            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                webview_url,
            )
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
            commands::get_events_after,
            commands::connect_api_key,
            commands::disconnect_api_key,
            commands::get_api_key_status,
            commands::fetch_models
        ])
        .run(tauri::generate_context!())
        .expect("error while running Selora desktop shell");
}
