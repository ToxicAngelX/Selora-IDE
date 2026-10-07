//! The title strip (UI_SPEC.md §1.1): the 32px top bar — brand mark on the
//! left, 8px from the window edge, then an 8px gap, then "Selora IDE" in
//! `--type-xl` / `--weight-semibold`. The OS draws the native window chrome
//! above this.
//!
//! Phase R0: extracted from `selora-app`'s `shell.rs` so the shell composes
//! chrome instead of hand-painting it (menus arrive in Phase R6).

use egui::Ui;

use crate::theme::{color, font, type_scale};
use crate::{brand, theme};

/// Title bar height (UI_SPEC.md §1.1).
pub const HEIGHT: f32 = 32.0;
/// Brand mark size in the title bar (UI_SPEC.md §1.1: 16px).
const MARK_SIZE: f32 = 16.0;
/// Brand mark inset from the window edge (UI_SPEC.md §1.1: 8px).
const EDGE_PADDING: f32 = 8.0;
/// Gap between the mark and the app title (UI_SPEC.md §1.1: 8px).
const TITLE_GAP: f32 = 8.0;

/// Paint the title strip.
pub fn show(ui: &mut Ui) {
    egui::Panel::top("title_strip")
        .exact_size(HEIGHT)
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
                    .color(color::FG_DEFAULT)
                    .font(egui::FontId::new(
                        type_scale::XL,
                        font::family_sans_semibold(),
                    ));
                ui.add(egui::Label::new(title));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    /// The strip paints headlessly at its token height with the bundled
    /// fonts (the shell's smoke test previously covered this inline).
    #[test]
    fn title_strip_paints_at_the_token_height() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, show);
        output.textures_delta.clear();
    }
}
