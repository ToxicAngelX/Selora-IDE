//! Selora IDE — application binary.
//!
//! Phase 2: the full workbench shell (PLAN.md §9 Phase 2). The merged
//! Phase 2 components — tabs/splits, file tree, fuzzy finder, syntax
//! highlighting, command palette, settings — are wired together in
//! [`shell::SeloraApp`], which replaces the Phase 1c spike window. See
//! the module docs of [`shell`], [`app_state`], and [`wiring`] for the
//! frame-pipeline and process-model contracts.
//!
//! Process model (ARCHITECTURE.md §1): the multi-threaded tokio runtime is
//! built *before* the eframe loop on the main thread; UI state stays on the
//! main thread; workers talk back over an unbounded `mpsc` channel drained
//! at the top of every frame (§2), waking the loop with
//! `ctx.request_repaint()` after every send.

mod app_state;
mod persistence;
mod shell;
mod wiring;

use eframe::egui;
use tokio::sync::mpsc;

use app_state::{AppMessage, Effect};

/// Default window size (UI_SPEC.md §1: 1280x800 reference window).
const WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];
/// Minimum window size (UI_SPEC.md: layouts stay correct to 960x600).
const MIN_WINDOW_SIZE: [f32; 2] = [960.0, 600.0];

fn main() -> eframe::Result<()> {
    // ARCHITECTURE.md §1.4: build the multi-threaded runtime BEFORE the
    // eframe loop, on the main thread. It keeps running on its worker
    // threads for the lifetime of the process; `runtime` is dropped only
    // after `run_native` returns (window closed) — clean shutdown.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");

    // The two §2 channels: worker → main-thread messages, and main-thread →
    // runtime effects. Both unbounded — a dropped frame of latency is fine,
    // a blocked worker is not.
    let (msg_tx, msg_rx) = mpsc::unbounded_channel::<AppMessage>();
    let (effect_tx, effect_rx) = mpsc::unbounded_channel::<Effect>();

    let handle = runtime.handle().clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Selora IDE")
            .with_inner_size(WINDOW_SIZE)
            .with_min_inner_size(MIN_WINDOW_SIZE),
        ..Default::default()
    };

    eframe::run_native(
        "Selora IDE",
        options,
        Box::new(move |cc| {
            // Boot the bundled fonts and the Selora dark visuals (the typed
            // DESIGN_TOKENS.md §11 contract) before the first frame.
            selora_ui::theme::set_fonts(&cc.egui_ctx);
            selora_ui::theme::set_visuals(&cc.egui_ctx);
            Ok(Box::new(shell::SeloraApp::new(
                cc.egui_ctx.clone(),
                handle,
                msg_tx,
                msg_rx,
                effect_tx,
                effect_rx,
            )))
        }),
    )
    // The egui loop has ended (window closed); the runtime — and every
    // worker task still parked on it — is dropped here, before exit.
}
