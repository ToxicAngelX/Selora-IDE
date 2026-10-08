//! The activity bar — the 48px leftmost strip (UI_SPEC.md §2.1).
//!
//! Four 16×16 icon buttons (files / search / git / chat) laid out as a
//! group centered vertically. The selected activity sits on a 28×28
//! `--ink-low` pill with `--radius-1`; hover lifts the pill to
//! `--ink-faint`, press to `--ink-mid` (DESIGN_TOKENS.md §10 — the press
//! must read one step above the selected pill's `--ink-low`). The selected
//! pill also carries the 2×16 leading edge mark (`--fg-muted`,
//! `--radius-1`, `--space-1` from the strip's left edge — a painted accent
//! rect, not a border; the REVAMP_SPEC.md §11 bend-3 sign-off). No labels.
//!
//! The shell owns the strip's panel (and its width); this widget paints
//! its content and reports activity switches. The icons are painted in
//! code per DESIGN_TOKENS.md §8 (`crate::sidebar::icons`).

use egui::{vec2, Align, Layout, Pos2, Rect, Sense, Ui, Vec2};

use crate::sidebar::icons;
use crate::theme::{color, radius, space, type_scale};

/// The strip width the shell should allocate (UI_SPEC.md §2.1).
pub const STRIP_WIDTH: f32 = 48.0;

/// The selected-activity pill: 28×28, `--radius-1` (UI_SPEC.md §2.1).
const PILL: f32 = 28.0;

/// Vertical gap between the four buttons.
const GAP: f32 = space::S1;

/// The selected activity's leading edge mark (REVAMP_SPEC.md §1
/// `EDGE_MARK`): 2×16, `--radius-1`, `--fg-muted`, `--space-1` from the
/// strip's left edge, vertically centered on the pill — a painted accent
/// rect, not a border (the §11 bend-3 sign-off).
const EDGE_MARK_W: f32 = 2.0;
const EDGE_MARK_H: f32 = 16.0;

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
    /// The problems panel (Phase R6) — LSP diagnostics.
    Problems,
    /// Git panel.
    Git,
    /// Chat sidebar.
    Chat,
}

impl Activity {
    const ALL: [Activity; 5] = [
        Activity::Files,
        Activity::Search,
        Activity::Problems,
        Activity::Git,
        Activity::Chat,
    ];

    /// Paint this activity's 16×16 glyph.
    fn paint_icon(&self, painter: &egui::Painter, slot: egui::Rect, color: egui::Color32) {
        match self {
            Activity::Files => icons::paint_sheet(painter, slot, color),
            Activity::Search => icons::paint_search(painter, slot, color),
            Activity::Problems => icons::paint_warning(painter, slot, color),
            Activity::Git => icons::paint_git_branch(painter, slot, color),
            Activity::Chat => icons::paint_chat(painter, slot, color),
        }
    }
}

/// Stateful activity bar: remembers the selected activity.
pub struct ActivityBar {
    /// The problems count for the badge (Phase R6; set per frame by the
    /// shell via `show_with_problems`).
    problems_badge: usize,
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
        Self {
            selected,
            problems_badge: 0,
        }
    }

    /// Paint the bar with a problems badge (Phase R6): the count rides
    /// the problems icon's bottom-right (the `--info` tone, mono XS) —
    /// VS Code's affordance, in ink. Switches selection like [`show`].
    pub fn show_with_problems(&mut self, ui: &mut Ui, problems: usize) -> Option<Activity> {
        self.problems_badge = problems;
        self.show(ui)
    }

    /// Paint the strip into `ui` (the shell hands us a 48px-wide,
    /// full-height Ui). Returns `Some(activity)` on the frame the user
    /// switches activity; `None` otherwise.
    pub fn show(&mut self, ui: &mut Ui) -> Option<Activity> {
        let strip_height = ui.available_height();
        let strip_left = ui.max_rect().left();
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

                // DESIGN_TOKENS.md §10: pressed `--ink-mid` (one step above
                // the selected `--ink-low`), selected `--ink-low`, hover
                // `--ink-faint`, idle transparent.
                let pill_fill = if response.is_pointer_button_down_on() {
                    Some(color::INK_MID)
                } else if is_selected {
                    Some(color::INK_LOW)
                } else if response.hovered() {
                    Some(color::INK_FAINT)
                } else {
                    None
                };
                if let Some(fill) = pill_fill {
                    ui.painter().rect_filled(rect, radius::R1, fill);
                }

                // The selected pill's leading edge mark (REVAMP_SPEC.md
                // §7): 2×16 `--fg-muted` at `--space-1` from the strip's
                // left edge, vertically centered on the pill.
                if is_selected {
                    let mark = Rect::from_min_size(
                        Pos2::new(strip_left + space::S1, rect.center().y - EDGE_MARK_H / 2.0),
                        vec2(EDGE_MARK_W, EDGE_MARK_H),
                    );
                    ui.painter().rect_filled(mark, radius::R1, color::FG_MUTED);
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

                // The problems badge (Phase R6): the count on the icon's
                // bottom-right corner. The type floor is --type-xs (§4);
                // the badge is one grid cell tall (S4), the text centered.
                if matches!(activity, Activity::Problems) && self.problems_badge > 0 {
                    let badge_text = if self.problems_badge > 99 {
                        "99+".to_owned()
                    } else {
                        self.problems_badge.to_string()
                    };
                    let badge_font =
                        egui::FontId::new(type_scale::XS, crate::theme::font::FAMILY_MONO);
                    let badge_galley = ui.painter().layout_no_wrap(
                        badge_text,
                        badge_font,
                        crate::theme::color::FG_DEFAULT,
                    );
                    let badge_size = vec2(badge_galley.size().x + space::S2, space::S4);
                    let badge_rect = Rect::from_min_size(
                        Pos2::new(rect.right() - badge_size.x, rect.bottom() - badge_size.y),
                        badge_size,
                    );
                    ui.painter()
                        .rect_filled(badge_rect, radius::R1, crate::theme::color::INK_MID);
                    ui.painter().galley(
                        badge_rect.center() - badge_galley.size() / 2.0,
                        badge_galley,
                        crate::theme::color::FG_DEFAULT,
                    );
                }

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

    /// REVAMP_SPEC.md §10: the selected pill paints the 2×16 `--fg-muted`
    /// edge mark at `--space-1` from the strip's left edge, vertically
    /// centered on the pill — and the unselected pills paint none.
    #[test]
    fn selected_pill_paints_the_edge_mark() {
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(STRIP_WIDTH, 400.0));

        let mut bar = ActivityBar::new(Activity::Git);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                ui.set_min_size(screen.size());
                assert_eq!(bar.show(ui), None);
            },
        );
        output.textures_delta.clear();

        // The selected pill: the one 28×28 `--ink-low` fill.
        let pill = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(shape)
                    if shape.fill == color::INK_LOW
                        && (shape.rect.width() - PILL).abs() < 0.5
                        && (shape.rect.height() - PILL).abs() < 0.5 =>
                {
                    Some(shape.rect)
                }
                _ => None,
            })
            .expect("the selected pill paints its --ink-low fill");

        let marks: Vec<Rect> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(shape)
                    if shape.fill == color::FG_MUTED
                        && (shape.rect.width() - EDGE_MARK_W).abs() < 0.5
                        && (shape.rect.height() - EDGE_MARK_H).abs() < 0.5 =>
                {
                    Some(shape.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(marks.len(), 1, "exactly one edge mark (selected only)");
        let mark = marks[0];
        assert!(
            (mark.left() - (screen.left() + space::S1)).abs() < 0.5,
            "the mark sits --space-1 from the strip's left edge: {mark:?}"
        );
        assert!(
            (mark.center().y - pill.center().y).abs() < 0.5,
            "the mark is vertically centered on the pill: {mark:?} vs {pill:?}"
        );
        assert!(
            mark.right() <= pill.left(),
            "the mark leads the pill, never overlaps it: {mark:?} vs {pill:?}"
        );
    }
}
