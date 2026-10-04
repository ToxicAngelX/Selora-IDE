//! Selora IDE — application binary.
//!
//! Phase 1c (the risk spike, PLAN.md §9): wires the Phase 1a editor widget
//! and the Phase 1b gateway client into one window. The Phase 1 exit gate is
//! "open a temp file, edit it, see SSE text stream into a panel" — this file
//! is that window.
//!
//! Process model (ARCHITECTURE.md §1): the tokio runtime is built *before*
//! the eframe loop on the main thread; UI state stays on the main thread;
//! workers talk back over an unbounded `mpsc` channel drained at the top of
//! every frame (§2). No theming work, no persistence, no chrome beyond the
//! spike — this is a tech demo, not the product.

mod app_state;
mod wiring;

use std::sync::{Arc, RwLock};

use eframe::egui;
use selora_core::Buffer;
use selora_editor::Editor;
use selora_ui::{brand, theme};
use tokio::sync::mpsc;

use wiring::{AppMessage, ConnectionState, SpikeState};

/// Default window size (UI_SPEC.md §1: 1280x800 reference window).
const WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];
/// Minimum window size (UI_SPEC.md: layouts stay correct to 960x600).
const MIN_WINDOW_SIZE: [f32; 2] = [960.0, 600.0];
/// Title bar height (UI_SPEC.md §1.1).
const TITLE_BAR_HEIGHT: f32 = 32.0;
/// Brand mark size in the title bar (UI_SPEC.md §1.1: 16px).
const MARK_SIZE: f32 = 16.0;
/// Brand mark inset from the window edge (UI_SPEC.md §1.1: 8px).
const EDGE_PADDING: f32 = 8.0;
/// Gap between the mark and the app title (UI_SPEC.md §1.1: 8px).
const TITLE_GAP: f32 = 8.0;
/// The temp file the spike pre-loads into the editor (PLAN.md §9: "open a
/// temp file"). Written to the OS temp dir on boot; never read again.
const SPIKE_FILE_NAME: &str = "selora-spike.rs";

/// The spike's sample buffer content.
const SPIKE_TEXT: &str = r#"// Selora IDE — Phase 1 risk spike (PLAN.md §9: "open a temp file, edit
// it, see SSE text stream into a panel").
//
// This buffer is the editor half of the spike: type here and the ropey
// buffer behind it updates. The right panel is the network half: paste a
// Selora key, connect, and a chat completion streams in over SSE — in
// process, no webview.

fn main() {
    println!("Selora IDE spike: editor + gateway, one window.");
}
"#;

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

    // Worker → main-thread message channel (the §2 Message-drain pattern).
    let (tx, rx) = mpsc::unbounded_channel();

    // Pre-load a temp file into the editor (the "open a temp file" half of
    // the Phase 1 exit gate).
    let (spike_path, spike_text) = load_spike_text();
    let editor = Editor::new(Arc::new(RwLock::new(Buffer::from_text(&spike_text))));

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
            // Boot the bundled fonts so the editor's monospace family resolves
            // to JetBrains Mono (the Phase 1a widget contract).
            theme::set_fonts(&cc.egui_ctx);
            theme::set_visuals(&cc.egui_ctx);
            Ok(Box::new(SpikeApp::new(
                cc.egui_ctx.clone(),
                handle,
                tx,
                rx,
                editor,
                spike_path,
            )))
        }),
    )
    // The egui loop has ended (window closed); the runtime — and every
    // worker task still parked on it — is dropped here, before exit.
}

/// Write the spike text to a temp file and read it back. IO failures degrade
/// to the in-memory text rather than aborting the demo (this is a spike, not
/// a file manager).
fn load_spike_text() -> (std::path::PathBuf, String) {
    let path = std::env::temp_dir().join(SPIKE_FILE_NAME);
    let text = std::fs::write(&path, SPIKE_TEXT)
        .and_then(|()| std::fs::read_to_string(&path))
        .unwrap_or_else(|_| SPIKE_TEXT.to_owned());
    (path, text)
}

/// The spike window: editor on the left (CentralPanel), gateway panel on the
/// right (SidePanel). Owns all UI state on the main thread; the workers only
/// ever reach it through the message channel.
struct SpikeApp {
    /// Cloned egui context, handed to worker tasks so they can
    /// `request_repaint()` when a message arrives — without it the UI sleeps
    /// between frames and streaming deltas would never appear.
    ctx: egui::Context,
    /// Handle onto the tokio runtime the workers are spawned onto (§1.4).
    handle: tokio::runtime::Handle,
    /// Effect side: clones of this go to spawned workers.
    tx: mpsc::UnboundedSender<AppMessage>,
    /// Message side: drained at the top of every frame (§2).
    rx: mpsc::UnboundedReceiver<AppMessage>,
    /// Main-thread state reduced from those messages (§3.1, spike subset).
    state: SpikeState,
    /// The Phase 1a editor widget, pre-loaded with the temp file's text.
    editor: Editor,
    spike_path: std::path::PathBuf,
    /// UI-side input state. Nothing here persists (spike rule).
    api_key: String,
    base_url: String,
    prompt: String,
}

impl SpikeApp {
    fn new(
        ctx: egui::Context,
        handle: tokio::runtime::Handle,
        tx: mpsc::UnboundedSender<AppMessage>,
        rx: mpsc::UnboundedReceiver<AppMessage>,
        editor: Editor,
        spike_path: std::path::PathBuf,
    ) -> Self {
        Self {
            ctx,
            handle,
            tx,
            rx,
            state: SpikeState::default(),
            editor,
            spike_path,
            api_key: String::new(),
            base_url: wiring::DEFAULT_BASE_URL.to_owned(),
            prompt: String::new(),
        }
    }

    /// Right panel: the gateway half of the spike — key, connect, prompt,
    /// send, and the stream output that grows as deltas arrive.
    fn chat_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("spike_chat_panel")
            .default_size(380.0)
            .min_size(280.0)
            .max_size(600.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(theme::space::S3);

                    ui.label("Selora key:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.api_key).desired_width(f32::INFINITY),
                    );

                    ui.label("Base URL:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.base_url).desired_width(f32::INFINITY),
                    );

                    if ui.button("Connect").clicked() {
                        self.state.begin_connect();
                        let ctx = self.ctx.clone();
                        wiring::spawn_connect(
                            &self.handle,
                            move || ctx.request_repaint(),
                            self.tx.clone(),
                            self.base_url.trim().to_owned(),
                            self.api_key.trim().to_owned(),
                        );
                    }

                    let status_color = match &self.state.connection {
                        ConnectionState::Connected(_) => theme::color::OK,
                        ConnectionState::Failed(_) => theme::color::ERR,
                        _ => theme::color::FG_MUTED,
                    };
                    ui.label(
                        egui::RichText::new(self.state.connection.label()).color(status_color),
                    );

                    ui.separator();

                    ui.label("Test prompt:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.prompt).desired_width(f32::INFINITY),
                    );

                    // Send needs a model id; the spike takes the first one the
                    // gateway listed. One stream at a time.
                    let can_send = !self.state.models.is_empty() && !self.state.streaming;
                    if ui
                        .add_enabled(can_send, egui::Button::new("Send"))
                        .clicked()
                    {
                        if let Some(model) = self.state.models.first().map(|model| model.id.clone())
                        {
                            self.state.begin_chat();
                            let ctx = self.ctx.clone();
                            wiring::spawn_chat(
                                &self.handle,
                                move || ctx.request_repaint(),
                                self.tx.clone(),
                                self.base_url.trim().to_owned(),
                                self.api_key.trim().to_owned(),
                                model,
                                self.prompt.clone(),
                            );
                        }
                    }
                    if self.state.models.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                "connect first — Send needs a model from the gateway",
                            )
                            .color(theme::color::FG_FAINT),
                        );
                    } else if self.state.streaming {
                        ui.label("streaming…");
                    }

                    ui.separator();

                    ui.label("Stream output:");
                    // The output grows with the content (clamped), and stays
                    // read-only — it is a log, not an input.
                    let rows = self.state.stream.split('\n').count().clamp(4, 30);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.state.stream)
                            .desired_width(f32::INFINITY)
                            .desired_rows(rows)
                            .interactive(false),
                    );

                    if let Some(receipt) = &self.state.receipt {
                        let estimate = if receipt.estimated {
                            " (estimated)"
                        } else {
                            ""
                        };
                        ui.label(format!(
                            "tokens: {} in / {} out / cost: ${:.4}{}",
                            receipt.prompt_tokens,
                            receipt.completion_tokens,
                            receipt.cost_usd,
                            estimate
                        ));
                    }
                    if let Some(error) = &self.state.chat_error {
                        ui.label(
                            egui::RichText::new(format!("error: {error}")).color(theme::color::ERR),
                        );
                    }
                });
            });
    }

    /// Central panel: the editor half of the spike, pre-loaded with the temp
    /// file's text. The widget fills all remaining space (Phase 1a contract).
    fn editor_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("temp file: {}", self.spike_path.display()))
                        .color(theme::color::FG_MUTED)
                        .font(egui::FontId::new(
                            theme::type_scale::XS,
                            theme::font::FAMILY_MONO,
                        )),
                );
                ui.add(&mut self.editor);
            });
    }
}

impl eframe::App for SpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // §2 stage 1: drain worker messages before any painting, so this
        // frame already shows what the workers produced since the last one.
        while let Ok(message) = self.rx.try_recv() {
            self.state.apply(message);
        }

        title_strip(ui);
        self.chat_panel(ui);
        self.editor_panel(ui);
    }
}

/// The title strip: brand mark on the left, 8px from the window edge, then an
/// 8px gap, then "Selora IDE" in `--type-xl` / `--weight-semibold`
/// (UI_SPEC.md §1.1). The OS draws the native window chrome above this.
fn title_strip(ui: &mut egui::Ui) {
    egui::Panel::top("title_strip")
        .exact_size(TITLE_BAR_HEIGHT)
        .show_separator_line(false)
        .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add_space(EDGE_PADDING);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(MARK_SIZE, MARK_SIZE), egui::Sense::hover());
                brand::paint_mark(ui.painter(), rect);
                ui.add_space(TITLE_GAP);
                let title = egui::RichText::new("Selora IDE")
                    .color(theme::color::FG_DEFAULT)
                    .font(egui::FontId::new(
                        theme::type_scale::XL,
                        theme::font::family_sans_semibold(),
                    ));
                ui.add(egui::Label::new(title));
            });
        });
}
