//! The activity bar — the 48px leftmost strip (UI_SPEC.md §2.1).
//!
//! Four 16×16 icon buttons (files / search / git / chat) laid out as a
//! group centered vertically. The selected activity sits on a 28×28
//! `--ink-low` pill with `--radius-1`; hover lifts the pill to
//! `--ink-faint`. No labels.
//!
//! The shell owns the strip's panel (and its width); this widget paints
//! its content and reports activity switches. The icons are painted in
//! code per DESIGN_TOKENS.md §8 (`crate::sidebar::icons`).

use egui::{vec2, Align, Layout, Sense, Ui, Vec2};

use crate::sidebar::icons;
use crate::theme::{color, radius, space};

/// The strip width the shell should allocate (UI_SPEC.md §2.1).
pub const STRIP_WIDTH: f32 = 48.0;

/// The selected-activity pill: 28×28, `--radius-1` (UI_SPEC.md §2.1).
const PILL: f32 = 28.0;

/// Vertical gap between the four buttons.
const GAP: f32 = space::S1;

/// Which workbench activity the left region shows (UI_SPEC.md §2.1).
/// The shell routes this: files → file tree, search → search panel,
/// git → git panel, chat → chat sidebar toggle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Activity {
    /// File tree (the default).
    #[default]
    Files,
    /// Search in files.
    Search,
    /// Git panel.
    Git,
    /// Chat sidebar.
    Chat,
}

impl Activity {
    const ALL: [Activity; 4] = [
        Activity::Files,
        Activity::Search,
        Activity::Git,
        Activity::Chat,
    ];

    /// Paint this activity's 16×16 glyph.
    fn paint_icon(&self, painter: &egui::Painter, slot: egui::Rect, color: egui::Color32) {
        match self {
            Activity::Files => icons::paint_sheet(painter, slot, color),
            Activity::Search => icons::paint_search(painter, slot, color),
            Activity::Git => icons::paint_git_branch(painter, slot, color),
            Activity::Chat => icons::paint_chat(painter, slot, color),
        }
    }
}

/// Stateful activity bar: remembers the selected activity.
pub struct ActivityBar {
    /// The currently-selected activity.
    pub selected: Activity,
}

impl Default for ActivityBar {
    fn default() -> Self {
        Self::new(Activity::Files)
    }
}

impl ActivityBar {
    /// A bar with `selected` active (the default is [`Activity::Files`],
    /// per UI_SPEC.md §2.1).
    pub fn new(selected: Activity) -> Self {
        Self { selected }
    }

    /// Paint the strip into `ui` (the shell hands us a 48px-wide,
    /// full-height Ui). Returns `Some(activity)` on the frame the user
    /// switches activity; `None` otherwise.
    pub fn show(&mut self, ui: &mut Ui) -> Option<Activity> {
        let strip_height = ui.available_height();
        let group_height =
            Activity::ALL.len() as f32 * PILL + (Activity::ALL.len() - 1) as f32 * GAP;
        let top_pad = ((strip_height - group_height) / 2.0).max(0.0);

        let mut switched = None;
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.set_min_width(ui.available_width());
            ui.add_space(top_pad);
            for activity in Activity::ALL {
                let is_selected = self.selected == activity;
                let (rect, response) = ui.allocate_exact_size(vec2(PILL, PILL), Sense::click());

                let pill_fill = if is_selected {
                    Some(color::INK_LOW)
                } else if response.hovered() {
                    Some(color::INK_FAINT)
                } else {
                    None
                };
                if let Some(fill) = pill_fill {
                    ui.painter().rect_filled(rect, radius::R1, fill);
                }

                // Idle icon text is --fg-muted; selected/hover lifts to
                // --fg-default (DESIGN_TOKENS.md §10).
                let icon_color = if is_selected || response.hovered() {
                    color::FG_DEFAULT
                } else {
                    color::FG_MUTED
                };
                let icon = egui::Rect::from_center_size(rect.center(), Vec2::splat(16.0));
                activity.paint_icon(ui.painter(), icon, icon_color);

                if response.clicked() && !is_selected {
                    self.selected = activity;
                    switched = Some(activity);
                }
                ui.add_space(GAP);
            }
        });
        switched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Headless paint test: the strip paints without panicking and
    /// reports no switch (no input in the test harness).
    #[test]
    fn activity_bar_paints_headlessly() {
        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(STRIP_WIDTH, 160.0));
            let mut bar = ActivityBar::new(Activity::Files);
            assert_eq!(bar.show(ui), None);
            assert_eq!(bar.selected, Activity::Files);
        });
    }

    #[test]
    fn activity_default_is_files() {
        assert_eq!(Activity::default(), Activity::Files);
    }
}
