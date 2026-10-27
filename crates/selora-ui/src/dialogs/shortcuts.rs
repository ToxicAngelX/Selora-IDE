//! The keyboard cheat sheet (Phase R7): a modal listing every global
//! keybind (the palette's "Help: Show Keyboard Shortcuts" command). The
//! rows arrive pre-built from the shell (the fixed table + the rebindable
//! four live there); the widget only paints. Esc or the backdrop closes.
//!
//! `--ink-mid` surface, 1px `--ink-hi` hairline, radius-2 (the modal
//! surface, §6), the scrim backdrop.

use crate::theme::{color, font, radius, space, type_scale};

/// One row of the cheat sheet: the action and its binding (already
/// formatted).
#[derive(Clone, Debug)]
pub struct ShortcutRow {
    pub action: String,
    pub binding: String,
}

/// The modal's dimensions (the settings dialog's precedents).
const WIDTH: f32 = 420.0;
const ROW_HEIGHT: f32 = 26.0;

/// Paint the cheat sheet (a modal); returns whether it dismissed itself
/// (backdrop click or topmost Esc — the Modal's `should_close`).
pub fn show(ctx: &egui::Context, rows: &[ShortcutRow]) -> bool {
    let frame = egui::Frame::default()
        .fill(color::INK_MID)
        .stroke(egui::Stroke::new(1.0, color::INK_HI))
        .corner_radius(radius::R2)
        .inner_margin(egui::Margin::same(space::S4 as i8));
    let response = egui::Modal::new(egui::Id::new("selora-shortcuts"))
        .backdrop_color(color::SCRIM)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_min_width(WIDTH);
            ui.set_max_width(WIDTH);
            ui.label(
                egui::RichText::new("Keyboard shortcuts")
                    .color(color::FG_DEFAULT)
                    .font(egui::FontId::new(
                        type_scale::LG,
                        font::family_sans_semibold(),
                    )),
            );
            ui.add_space(space::S2);
            egui::ScrollArea::vertical()
                .id_salt("selora_shortcuts_rows")
                .max_height(480.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for row in rows {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), ROW_HEIGHT),
                            egui::Sense::hover(),
                        );
                        ui.painter().text(
                            pos2(rect.left(), rect.center().y),
                            Align2::LEFT_CENTER,
                            row.action.as_str(),
                            egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                            color::FG_MUTED,
                        );
                        // The binding chip (the empty states' style).
                        let galley = ui.painter().layout_no_wrap(
                            row.binding.clone(),
                            egui::FontId::new(type_scale::XS, font::FAMILY_MONO),
                            color::FG_DEFAULT,
                        );
                        let chip = egui::Rect::from_min_size(
                            pos2(
                                rect.right() - galley.size().x - 2.0 * space::S1,
                                rect.center().y - galley.size().y / 2.0 - space::S1 / 2.0,
                            ),
                            egui::vec2(
                                galley.size().x + 2.0 * space::S1,
                                galley.size().y + space::S1,
                            ),
                        );
                        ui.painter().rect_filled(chip, radius::R1, color::INK_LOW);
                        ui.painter().galley(
                            pos2(chip.left() + space::S1, chip.top() + space::S1 / 2.0),
                            galley,
                            color::FG_DEFAULT,
                        );
                    }
                });
        });
    response.should_close()
}

use egui::{pos2, Align2};

#[cfg(test)]
mod tests {
    use super::*;

    /// The modal paints headlessly with the bundled fonts.
    #[test]
    fn the_cheat_sheet_paints() {
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        crate::theme::set_visuals(&ctx);
        let rows = vec![
            ShortcutRow {
                action: "Save".into(),
                binding: "Ctrl+S".into(),
            },
            ShortcutRow {
                action: "Find a file".into(),
                binding: "Ctrl+P".into(),
            },
        ];
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                let _ = show(ui.ctx(), &rows);
            },
        );
        output.textures_delta.clear();
    }
}
