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
//!
//! The menus ride egui's `menu_button` (its popup anchoring, mutual
//! exclusion, and hover-follows stay egui's). Their states come from the
//! DESIGN_TOKENS.md §10 matrix — hover `--ink-faint`, pressed/open
//! `--ink-low`, no borders (§5.2), text `--fg-default` in every state
//! (app chrome, like the wordmark) — via the scoped widget-visuals
//! override in [`menu_button_matrix`].

use egui::Ui;

use crate::palette::registry::CommandId;
use crate::theme::{color, font, radius, type_scale};
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
                // closes the menu. The buttons take the §10 matrix from
                // the widget visuals (no RichText tint — an explicit text
                // color would pin all four states to one color); the
                // scoped override below is what lands the theme on egui's
                // Button paint path.
                menu_button_matrix(ui);
                let mut event: Option<TitleBarEvent> = None;
                let file_menu = ui.menu_button("File", |ui| {
                    menu_button_matrix(ui);
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
                    menu_button_matrix(ui);
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
                    menu_button_matrix(ui);
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
                    menu_button_matrix(ui);
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

            // --- Fancy Centered Command Search Pill ---
            let panel_rect = ui.max_rect();
            let pill_width = 300.0f32.min(panel_rect.width() * 0.34);
            if pill_width > 140.0 {
                let pill_height = 22.0;
                let pill_rect = egui::Rect::from_center_size(
                    egui::pos2(panel_rect.center().x, panel_rect.center().y),
                    egui::vec2(pill_width, pill_height),
                );
                let resp = ui.interact(
                    pill_rect,
                    egui::Id::new("title_search_pill"),
                    egui::Sense::click(),
                );
                let fill = if resp.hovered() {
                    color::INK_LOW
                } else {
                    color::INK_FAINT
                };
                let stroke_color = if resp.hovered() {
                    color::FG_MUTED
                } else {
                    color::INK_HI
                };
                ui.painter().rect(
                    pill_rect,
                    radius::R1,
                    fill,
                    egui::Stroke::new(1.0, stroke_color),
                    egui::StrokeKind::Inside,
                );

                // Search icon + label
                let text_pos = egui::pos2(pill_rect.left() + 8.0, pill_rect.center().y);
                let text_col = if resp.hovered() {
                    color::FG_DEFAULT
                } else {
                    color::FG_MUTED
                };
                ui.painter().text(
                    text_pos,
                    egui::Align2::LEFT_CENTER,
                    "🔍 Search files or commands...",
                    egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                    text_col,
                );

                // Right kbd shortcut badge
                let kbd_pos = egui::pos2(pill_rect.right() - 8.0, pill_rect.center().y);
                ui.painter().text(
                    kbd_pos,
                    egui::Align2::RIGHT_CENTER,
                    "Ctrl+P",
                    egui::FontId::new(type_scale::XS, font::FAMILY_MONO),
                    color::FG_FAINT,
                );

                if resp.clicked() {
                    outcome.event = Some(TitleBarEvent::Command(CommandId::FileGoToFile));
                }
            }

            // --- Right-side AI Agent & Git Branch Indicator ---
            let agent_pill_w = 78.0;
            let agent_pill_h = 20.0;
            let agent_rect = egui::Rect::from_min_size(
                egui::pos2(
                    panel_rect.right() - agent_pill_w - 12.0,
                    panel_rect.center().y - agent_pill_h / 2.0,
                ),
                egui::vec2(agent_pill_w, agent_pill_h),
            );
            let agent_resp = ui.interact(
                agent_rect,
                egui::Id::new("title_agent_pill"),
                egui::Sense::click(),
            );
            let agent_bg = if agent_resp.hovered() {
                color::INK_MID
            } else {
                color::INK_LOW
            };
            ui.painter().rect(
                agent_rect,
                radius::R1,
                agent_bg,
                egui::Stroke::new(1.0, color::INK_HI),
                egui::StrokeKind::Inside,
            );

            // Pulsing live green presence dot
            let dot_pos = egui::pos2(agent_rect.left() + 9.0, agent_rect.center().y);
            ui.painter().circle_filled(dot_pos, 3.0, color::OK);
            let agent_text_pos = egui::pos2(agent_rect.left() + 18.0, agent_rect.center().y);
            ui.painter().text(
                agent_text_pos,
                egui::Align2::LEFT_CENTER,
                "Agent",
                egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                if agent_resp.hovered() {
                    color::FG_DEFAULT
                } else {
                    color::FG_MUTED
                },
            );
            if agent_resp.clicked() {
                outcome.event = Some(TitleBarEvent::Command(CommandId::ViewToggleChatSidebar));
            }

            // Hairline divider along the bottom edge of the title bar
            ui.painter().hline(
                panel_rect.x_range(),
                panel_rect.bottom() - 1.0,
                egui::Stroke::new(1.0, color::INK_HI),
            );
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

/// Land the §10 state matrix on egui's `Button` paint path, scoped to the
/// calling `Ui` (the menu row, or one popup). egui's button frame fill is
/// `weak_bg_fill` — not the `bg_fill` the global theme sets
/// (`theme::set_visuals`) — and its hover/press strokes are egui's dark
/// defaults, so without this the menus paint off-token grays. After the
/// override: hover `--ink-faint`, pressed/open `--ink-low`, no border
/// (DESIGN_TOKENS.md §5.2: menu bar items carry none), corners `--radius-1`
/// (§4: the button radius). Menu *text* is untouched: it rides the
/// widget-state `fg_stroke` the theme already sets (`--fg-default` in every
/// interactive state — app chrome, like the wordmark).
fn menu_button_matrix(ui: &mut Ui) {
    let widgets = &mut ui.style_mut().visuals.widgets;
    widgets.hovered.weak_bg_fill = color::INK_FAINT;
    widgets.active.weak_bg_fill = color::INK_LOW;
    widgets.open.weak_bg_fill = color::INK_LOW;
    widgets.hovered.bg_stroke = egui::Stroke::NONE;
    widgets.active.bg_stroke = egui::Stroke::NONE;
    widgets.open.bg_stroke = egui::Stroke::NONE;
    let button_radius = egui::CornerRadius::same(radius::R1 as u8);
    widgets.inactive.corner_radius = button_radius;
    widgets.hovered.corner_radius = button_radius;
    widgets.active.corner_radius = button_radius;
    widgets.open.corner_radius = button_radius;
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
        run_full(ctx, input).1
    }

    /// One headless frame, keeping the `FullOutput` for shape assertions
    /// (the §9 paint tests inspect `shapes`).
    fn run_full(ctx: &egui::Context, input: egui::RawInput) -> (egui::FullOutput, TitleBarOutcome) {
        let mut outcome = TitleBarOutcome::default();
        let mut output = ctx.run_ui(input, |ui| {
            outcome = show(ui);
        });
        output.textures_delta.clear();
        (output, outcome)
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

    /// Whether an `--ink-faint` filled rect covers `pos`.
    fn paints_ink_faint_at(output: &egui::FullOutput, pos: egui::Pos2) -> bool {
        output.shapes.iter().any(|clipped| {
            matches!(
                &clipped.shape,
                egui::Shape::Rect(shape)
                    if shape.fill == color::INK_FAINT && shape.rect.contains(pos)
            )
        })
    }

    /// REVAMP_SPEC.md §7/§10: the menu buttons take the DESIGN_TOKENS.md
    /// §10 matrix — the
    /// conformance proof is the hover *background* (`--ink-faint`), not a
    /// text-color flip (the menu text is `--fg-default` in every state).
    #[test]
    fn menu_button_hover_paints_ink_faint_background() {
        let ctx = egui::Context::default();
        theme::set_fonts(&ctx);
        theme::set_visuals(&ctx);
        let (output, outcome) = run_full(&ctx, screen());
        let file_rect = outcome
            .menu_rects
            .iter()
            .find(|(label, _)| *label == "File")
            .map(|(_, rect)| *rect)
            .expect("the File button");
        assert!(
            !paints_ink_faint_at(&output, file_rect.center()),
            "idle: no hover surface under the File button"
        );

        // The pointer parked over the button: the same frame paints the
        // hovered widget visuals.
        let mut raw = screen();
        raw.focused = true;
        raw.events = vec![egui::Event::PointerMoved(file_rect.center())];
        let (output, _) = run_full(&ctx, raw);
        assert!(
            paints_ink_faint_at(&output, file_rect.center()),
            "hover: the File button paints its --ink-faint background"
        );
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
