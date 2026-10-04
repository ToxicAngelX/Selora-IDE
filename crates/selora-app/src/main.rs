//! Selora IDE — application binary.
//!
//! Phase 0 (foundation): boots eframe and paints the base window — a title
//! strip with the brand mark on the left, and an otherwise empty workbench on
//! the ink canvas. Panels, editors, chat, and the rest arrive in later phases
//! (see `ROADMAP.md`).

use eframe::egui;
use selora_ui::{brand, theme};

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

fn main() -> eframe::Result<()> {
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
        Box::new(|cc| {
            theme::set_fonts(&cc.egui_ctx);
            theme::set_visuals(&cc.egui_ctx);
            Ok(Box::new(SeloraApp))
        }),
    )
}

/// The application shell. Phase 0 state: nothing yet — later phases add
/// `AppState` (ARCHITECTURE.md §3.1) behind this struct.
struct SeloraApp;

impl eframe::App for SeloraApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        title_strip(ui);
        // Empty workbench on the ink canvas — the rest arrives in later phases.
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::color::INK_CANVAS))
            .show(ui, |_ui| {});
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
