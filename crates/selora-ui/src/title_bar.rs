//! The title strip (UI_SPEC.md §1.1): the 32px top bar — brand mark on the
//! left, 8px from the window edge, then an 8px gap, then "Selora IDE" in
//! `--type-xl` / `--weight-semibold`, then the menu bar (File / Edit /
//! View / Help — Phase R6 wired the §1.1 menus at last). The OS draws the
//! native window chrome above this.
//!
//! Phase R0 extracted the strip from `selora-app`'s `shell.rs`; Phase R6
//! added the menus. Menu clicks come back to the shell as
//! [`TitleBarEvent`]s — command-palette ids where a command exists, plus
//! the app-level actions that do not (Exit).

use egui::Ui;

use crate::palette::registry::CommandId;
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

/// What a menu click asks the shell for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleBarEvent {
    /// A palette command (the shell's existing `run_command` routes it).
    Command(CommandId),
    /// File → Exit: close the window (the shell sends the viewport
    /// command; the graceful-close hook persists).
    Exit,
}

/// One frame's title-bar outcome: the menu event (if a menu item was
/// clicked) plus the menu buttons' rects (the headless-test seam — the
/// model picker's `model_row_rects` precedent).
#[derive(Default, Debug)]
pub struct TitleBarOutcome {
    pub event: Option<TitleBarEvent>,
    /// `(label, rect)` for the four menu buttons.
    pub menu_rects: Vec<(&'static str, egui::Rect)>,
}

/// Paint the title strip.
pub fn show(ui: &mut Ui) -> TitleBarOutcome {
    let mut outcome = TitleBarOutcome::default();
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
                ui.add_space(space_gutter());

                // The menu bar (§1.1). egui's `menu_button` owns the
                // popup; an item's click lands in `outcome.event` and
                // closes the menu.
                let mut event: Option<TitleBarEvent> = None;
                let file_menu = ui.menu_button("File", |ui| {
                    menu_item(ui, "Open File…", &mut event, CommandId::FileOpenFile);
                    menu_item(ui, "Open Folder…", &mut event, CommandId::FileOpenFolder);
                    menu_item(ui, "Save", &mut event, CommandId::FileSave);
                    menu_item(ui, "Save All", &mut event, CommandId::FileSaveAll);
                    menu_item(ui, "Close Editor", &mut event, CommandId::FileCloseEditor);
                    menu_item(
                        ui,
                        "Close All Editors",
                        &mut event,
                        CommandId::FileCloseAllEditors,
                    );
                    menu_item(ui, "Go to File…", &mut event, CommandId::FileGoToFile);
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        event = Some(TitleBarEvent::Exit);
                        ui.close();
                    }
                });
                let edit_menu = ui.menu_button("Edit", |ui| {
                    menu_item(ui, "Find in File…", &mut event, CommandId::EditFindInFile);
                    menu_item(
                        ui,
                        "Replace in File…",
                        &mut event,
                        CommandId::EditReplaceInFile,
                    );
                    menu_item(
                        ui,
                        "Go to Definition",
                        &mut event,
                        CommandId::EditGoToDefinition,
                    );
                    menu_item(
                        ui,
                        "Toggle Comment",
                        &mut event,
                        CommandId::EditToggleComment,
                    );
                });
                let view_menu = ui.menu_button("View", |ui| {
                    menu_item(
                        ui,
                        "Toggle File Tree",
                        &mut event,
                        CommandId::ViewToggleFileTree,
                    );
                    menu_item(
                        ui,
                        "Toggle Chat Sidebar",
                        &mut event,
                        CommandId::ViewToggleChatSidebar,
                    );
                    menu_item(ui, "Toggle Panel", &mut event, CommandId::ViewTogglePanel);
                    menu_item(ui, "Split Editor", &mut event, CommandId::ViewSplitEditor);
                    menu_item(ui, "Go to Symbol…", &mut event, CommandId::ViewGoToSymbol);
                    menu_item(
                        ui,
                        "Command Palette…",
                        &mut event,
                        CommandId::ViewCommandPalette,
                    );
                });
                let help_menu = ui.menu_button("Help", |ui| {
                    menu_item(ui, "Settings…", &mut event, CommandId::SettingsOpen);
                    menu_item(ui, "About Selora IDE", &mut event, CommandId::HelpAbout);
                });
                outcome.event = event;

                // The menu buttons' rects (the test seam): each
                // `menu_button` returns its button's response.
                for (label, menu) in [
                    ("File", &file_menu),
                    ("Edit", &edit_menu),
                    ("View", &view_menu),
                    ("Help", &help_menu),
                ] {
                    outcome.menu_rects.push((label, menu.response.rect));
                }
            });
        });
    outcome
}

/// One menu item: a click sets `event` and closes the menu.
fn menu_item(ui: &mut Ui, label: &str, event: &mut Option<TitleBarEvent>, command: CommandId) {
    if ui.button(label).clicked() {
        *event = Some(TitleBarEvent::Command(command));
        ui.close();
    }
}

/// The gutter between the wordmark and the menus.
fn space_gutter() -> f32 {
    crate::theme::space::S4
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

    /// One headless frame with its texture deltas handled.
    fn run(ctx: &egui::Context, input: egui::RawInput) -> TitleBarOutcome {
        let mut outcome = TitleBarOutcome::default();
        let mut output = ctx.run_ui(input, |ui| {
            outcome = show(ui);
        });
        output.textures_delta.clear();
        outcome
    }

    /// The strip paints headlessly with the bundled fonts.
    #[test]
    fn title_strip_paints_at_the_token_height() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let outcome = run(&ctx, screen());
        assert!(outcome.event.is_none());
        assert_eq!(outcome.menu_rects.len(), 4, "four menu buttons");
    }

    /// Phase R6: opening the File menu and clicking an item emits its
    /// command (press/release on the button, then the sweep crosses the
    /// popup's items).
    #[test]
    fn file_menu_items_click_through() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        run(&ctx, screen());
        let outcome = run(&ctx, screen());
        let file_rect = outcome
            .menu_rects
            .iter()
            .find(|(label, _)| *label == "File")
            .map(|(_, rect)| *rect)
            .expect("the File button");

        // Open the menu (press + release on the button).
        let center = file_rect.center();
        for pressed in [true, false] {
            let mut raw = screen();
            raw.events = vec![
                egui::Event::PointerMoved(center),
                egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ];
            run(&ctx, raw);
        }

        // A settle frame: the popup paints one frame after the click
        // that opens it — the probe must find it already laid out.
        run(&ctx, screen());

        // The popup opens below the button: probe for the Exit item
        // (near the bottom of the item list, under the button's x).
        let mut got = None;
        'probe: for dy in (8..=200).step_by(6) {
            for dx in [0.0, 40.0, 90.0] {
                let pos = egui::pos2(center.x + 30.0 + dx, file_rect.bottom() + dy as f32);
                for pressed in [true, false] {
                    let mut raw = screen();
                    raw.events = vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ];
                    let outcome = run(&ctx, raw);
                    if let Some(event) = outcome.event {
                        got = Some(event);
                        break 'probe;
                    }
                }
            }
        }
        // The sweep's first hit is the first item it crosses; any File
        // command proves the full path (button → popup → item → event).
        // Exit is the same mechanics one separator lower.
        match got {
            Some(TitleBarEvent::Command(id)) => {
                assert!(
                    matches!(
                        id,
                        CommandId::FileOpenFile
                            | CommandId::FileOpenFolder
                            | CommandId::FileSave
                            | CommandId::FileSaveAll
                            | CommandId::FileCloseEditor
                            | CommandId::FileCloseAllEditors
                            | CommandId::FileGoToFile
                    ),
                    "a File-menu item fired: {id:?}"
                );
            }
            other => panic!("expected a File-menu command, got {other:?}"),
        }
    }
}
