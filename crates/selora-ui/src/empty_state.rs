//! Empty states (UI_SPEC.md §12) — the workbench's two placeholder
//! surfaces: "no folder open" and "a pane with no tab". Phase R0:
//! extracted from `selora-app`'s `shell.rs` so the shell composes chrome
//! instead of hand-painting it.
//!
//! The pane hint's keybind chips come from the live settings — the caller
//! passes the rebindable pair (`fuzzy_find`, `command_palette`) so the
//! hints follow user rebinds.

use egui::Ui;

use crate::brand;
use crate::theme::{color, font, radius, space, type_scale};

/// What the no-folder state asks the shell for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyStateEvent {
    /// "Open Folder" was clicked — run the folder picker.
    OpenFolder,
    /// "Clone Repo" was clicked — the v1.x clone flow (a toast for now).
    CloneRepo,
}

/// The "no folder open" state (UI_SPEC.md §12): title, muted hint, and the
/// Open Folder / Clone Repo buttons. Returns the clicked action, if any.
pub fn no_folder(ui: &mut Ui) -> Option<EmptyStateEvent> {
    let mut action = None;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).max(16.0));

        // Breathing orbital brand mark
        let time = ui.input(|i| i.time);
        let pulse = ((time * 1.6).sin() * 0.5 + 0.5) as f32;
        let mark_size = 56.0;
        let (mark_rect, _) =
            ui.allocate_exact_size(egui::vec2(mark_size, mark_size), egui::Sense::hover());
        let glow_alpha = (18.0 + pulse * 28.0) as u8;
        ui.painter().circle_filled(
            mark_rect.center(),
            40.0 + pulse * 5.0,
            egui::Color32::from_rgba_unmultiplied(34, 197, 94, glow_alpha),
        );
        brand::paint_mark(ui.painter(), mark_rect);
        ui.ctx().request_repaint();

        ui.add_space(space::S3);
        ui.label(
            egui::RichText::new("Selora IDE")
                .color(color::FG_DEFAULT)
                .font(egui::FontId::new(type_scale::XL, font::FAMILY_SANS)),
        );
        ui.add_space(space::S1);
        ui.label(
            egui::RichText::new("No folder open")
                .color(color::FG_DEFAULT)
                .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS)),
        );
        ui.add_space(space::S1);
        ui.label(
            egui::RichText::new("Open a folder to start editing.")
                .italics()
                .color(color::FG_MUTED)
                .font(egui::FontId::new(type_scale::SM, font::FAMILY_SANS)),
        );
        ui.add_space(space::S3);
        // The centered pair (UI_SPEC.md §12). egui centers only *direct*
        // children of `vertical_centered` — a plain `ui.horizontal` child
        // spans the full width and left-aligns its items (the buttons
        // painted at the panel's left edge, a pre-existing mis-centering
        // the R0 extraction fixed). So the row is measured first and the
        // leftover space padded in front.
        ui.horizontal(|ui| {
            let pad = ui.style().spacing.button_padding.x;
            let measure = |ui: &mut Ui, text: &str| {
                ui.painter()
                    .layout_no_wrap(
                        text.to_owned(),
                        egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
                        color::FG_DEFAULT,
                    )
                    .size()
                    .x
                    + 2.0 * pad
            };
            let row = measure(ui, "Open Folder") + space::S2 + measure(ui, "Clone Repo");
            ui.add_space(((ui.available_width() - row) / 2.0).max(0.0));
            let button =
                egui::Button::new(egui::RichText::new("Open Folder").color(color::FG_DEFAULT))
                    .fill(color::INK_MID)
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_unmultiplied(34, 197, 94, 140),
                    ))
                    .corner_radius(radius::R1 as u8);
            if ui.add(button).clicked() {
                action = Some(EmptyStateEvent::OpenFolder);
            }
            ui.add_space(space::S2);
            let button =
                egui::Button::new(egui::RichText::new("Clone Repo").color(color::FG_DEFAULT))
                    .fill(color::INK_MID)
                    .stroke(egui::Stroke::new(1.0, color::INK_HI))
                    .corner_radius(radius::R1 as u8);
            if ui.add(button).clicked() {
                action = Some(EmptyStateEvent::CloneRepo);
            }
        });

        ui.add_space(space::S4);

        // Keybind hints row
        ui.horizontal(|ui| {
            let row_width = 320.0;
            ui.add_space(((ui.available_width() - row_width) / 2.0).max(0.0));
            keybind_chip(ui, "Ctrl+P");
            ui.label(
                egui::RichText::new("Go to file")
                    .color(color::FG_MUTED)
                    .font(egui::FontId::new(type_scale::XS, font::FAMILY_SANS)),
            );
            ui.add_space(space::S2);
            keybind_chip(ui, "Ctrl+Shift+P");
            ui.label(
                egui::RichText::new("Commands")
                    .color(color::FG_MUTED)
                    .font(egui::FontId::new(type_scale::XS, font::FAMILY_SANS)),
            );
            ui.add_space(space::S2);
            keybind_chip(ui, "Ctrl+L");
            ui.label(
                egui::RichText::new("Agent")
                    .color(color::FG_MUTED)
                    .font(egui::FontId::new(type_scale::XS, font::FAMILY_SANS)),
            );
        });
    });
    action
}

/// The "folder open, pane has no tab" state (UI_SPEC.md §12): the keybind
/// hint line, with chips from the live settings. Painted into the pane's
/// content rect.
pub fn pane_no_tabs(ui: &mut Ui, content: egui::Rect, fuzzy_key: &str, palette_key: &str) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.4);
            ui.horizontal(|ui| {
                keybind_chip(ui, fuzzy_key);
                ui.label(
                    egui::RichText::new("to find a file")
                        .italics()
                        .color(color::FG_MUTED)
                        .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS)),
                );
                ui.add_space(space::S2);
                keybind_chip(ui, palette_key);
                ui.label(
                    egui::RichText::new("for commands")
                        .italics()
                        .color(color::FG_MUTED)
                        .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS)),
                );
            });
        });
    });
}

/// A keybind chip for the empty-state hint (UI_SPEC.md §12): `--ink-mid`
/// background, radius-1, `--type-xs` mono.
fn keybind_chip(ui: &mut Ui, key: &str) {
    egui::Frame::default()
        .fill(color::INK_MID)
        .corner_radius(radius::R1 as u8)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(key)
                    .color(color::FG_DEFAULT)
                    .font(egui::FontId::new(type_scale::XS, font::FAMILY_MONO)),
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        }
    }

    /// One headless frame with its texture deltas handled (the chat
    /// panel's `frame` helper pattern).
    fn run(ctx: &egui::Context, raw: egui::RawInput, paint: impl FnMut(&mut Ui)) {
        let mut output = ctx.run_ui(raw, paint);
        output.textures_delta.clear();
    }

    /// Both states paint headlessly; a click on the button row reports the
    /// matching event (the shell's old inline behavior).
    #[test]
    fn no_folder_reports_open_folder_on_click() {
        let ctx = egui::Context::default();
        // Two settle frames so the layout is stable.
        let mut action = None;
        for _ in 0..2 {
            run(&ctx, screen(), |ui| {
                action = no_folder(ui);
            });
        }
        assert_eq!(action, None, "no click, no event");

        // The button pair sits centered on the 1280×800 screen, in the
        // band under the title + hint (~y 300–380); "Open Folder" is the
        // left button. Sweep the band left-to-right with the four-frame
        // click pattern (move, move, press, release) — the first hit is
        // the Open Folder button.
        'probe: for y in (300..=380).step_by(12) {
            for x in (400..=880).step_by(16) {
                let pos = egui::pos2(x as f32, y as f32);
                let frames = [
                    vec![egui::Event::PointerMoved(pos)],
                    vec![egui::Event::PointerMoved(pos)],
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ],
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Default::default(),
                        },
                    ],
                ];
                for events in frames {
                    let mut raw = screen();
                    raw.events = events;
                    run(&ctx, raw, |ui| {
                        action = no_folder(ui);
                    });
                }
                if action.is_some() {
                    break 'probe;
                }
            }
        }
        assert_eq!(
            action,
            Some(EmptyStateEvent::OpenFolder),
            "the leftmost button in the row is Open Folder"
        );
    }

    #[test]
    fn pane_no_tabs_paints_into_the_content_rect() {
        let ctx = egui::Context::default();
        let content = egui::Rect::from_min_size(egui::pos2(400.0, 100.0), egui::vec2(480.0, 600.0));
        run(&ctx, screen(), |ui| {
            pane_no_tabs(ui, content, "Ctrl+P", "Ctrl+Shift+P");
        });
    }
}
