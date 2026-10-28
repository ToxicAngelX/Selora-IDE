//! Git panel (UI_SPEC.md §8) — the bottom-panel content shown when the
//! "git" activity is selected.
//!
//! A data-driven widget in the Phase 2 pattern: the shell owns the data
//! (the latest `GitStatus` snapshot in `AppState`) and drives this panel
//! with it; the panel owns only its own view state (the commit-message
//! draft, the selected row, the collapsed sections). Every frame it emits
//! the actions the user took — [`GitAction::Stage`] / [`GitAction::Unstage`]
//! / [`GitAction::Commit`] / [`GitAction::OpenDiff`] — through
//! [`GitOutcome`], and the shell turns those into `Effect`s
//! (`Effect::GitStage` / `Effect::GitUnstage` / `Effect::GitCommit` /
//! open-diff) exactly like the file tree's [`TreeEvent`]s
//! (`crate::sidebar::file_tree`).
//!
//! Everything visual comes from [`crate::theme`] — no hardcoded colors,
//! fonts, or spacing.

use std::path::PathBuf;

use egui::{pos2, vec2, Align2, Key, Rect, Sense, Ui, Vec2};

use selora_git::GitStatus;

use crate::theme::{color, font, radius, space, type_scale};

/// Header bar height (matches the file-tree header).
pub const HEADER_HEIGHT: f32 = 24.0;
/// One changed-file row (matches the file-tree row height).
pub const ROW_HEIGHT: f32 = 22.0;
/// The commit button's height (UI_SPEC.md §8).
pub const COMMIT_BUTTON_HEIGHT: f32 = 32.0;
/// The panel's label (UI_SPEC.md §8 header).
pub const HEADER_LABEL: &str = "GIT";
/// The section label (UI_SPEC.md §8).
pub const CHANGES_LABEL: &str = "Changes";
/// How many rows the changes list shows before it scrolls — the panel is
/// 200px by default (UI_SPEC.md §8); the list caps so the commit box
/// always stays reachable below it.
const MAX_VISIBLE_ROWS: f32 = 6.0;
/// The stage / unstage tap target (16px glyph slot, DESIGN_TOKENS.md §8).
const ACTION_SLOT: f32 = 16.0;

/// Row font: `--type-sm` mono (the panel is a `--font-mono` surface like
/// the file tree).
fn row_font_id() -> egui::FontId {
    egui::FontId::new(type_scale::SM, font::FAMILY_MONO)
}

/// Label font: `--type-xs` sans with the header's micro-label role.
fn label_font_id() -> egui::FontId {
    egui::FontId::new(type_scale::XS, font::FAMILY_SANS)
}

/// Section / body label font: `--type-sm` sans.
fn section_font_id() -> egui::FontId {
    egui::FontId::new(type_scale::SM, font::FAMILY_SANS)
}

// -----------------------------------------------------------------------------
// Actions (the panel's event contract with the shell)
// -----------------------------------------------------------------------------

/// What the user did this frame. The shell routes each variant to an
/// `Effect` (ARCHITECTURE.md §2.2) and re-requests a status snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitAction {
    /// `+` on a row (or a staged-everything flow): stage this file.
    Stage(PathBuf),
    /// `−` on a row: unstage this file.
    Unstage(PathBuf),
    /// The commit button / `Ctrl+Enter`: commit the index with this
    /// message.
    Commit(String),
    /// A row was clicked: open the working-tree-vs-index diff in a
    /// read-only view (UI_SPEC.md §8 "Click").
    OpenDiff(PathBuf),
}

/// Result of one `show` call.
#[derive(Debug, Default)]
pub struct GitOutcome {
    /// Actions for the shell, in frame order.
    pub actions: Vec<GitAction>,
    /// Entry rows painted (exposed for the headless paint test; the shell
    /// ignores it).
    pub rows_painted: usize,
}

// -----------------------------------------------------------------------------
// GitPanel
// -----------------------------------------------------------------------------

/// The git panel: view state only. Create one per app, install it in
/// `AppState`, and call [`GitPanel::show`] every frame with the current
/// status snapshot (`None` when the open folder is not a repository).
pub struct GitPanel {
    /// The commit-message draft (multi-line, UI_SPEC.md §8).
    pub commit_message: String,
    /// The selected row (highlighted; the click target for the diff view).
    pub selected: Option<PathBuf>,
    /// Whether the "Changes" section is expanded.
    pub changes_expanded: bool,
    /// Rows painted by the last `show` (reporting only, for tests).
    last_rows_painted: usize,
}

impl Default for GitPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl GitPanel {
    pub fn new() -> Self {
        Self {
            commit_message: String::new(),
            selected: None,
            changes_expanded: true,
            last_rows_painted: 0,
        }
    }

    /// Whether the commit action is available: a non-empty message and at
    /// least one staged change (UI_SPEC.md §8 disables the button
    /// otherwise). Public so tests and the shell can ask.
    pub fn commit_ready(&self, status: &GitStatus) -> bool {
        !self.commit_message.trim().is_empty() && status.staged_count() > 0
    }

    /// Paint the panel. `status` is the latest snapshot, or `None` when
    /// the workspace is not inside a git repository.
    pub fn show(&mut self, ui: &mut Ui, status: Option<&GitStatus>) -> GitOutcome {
        let mut actions = Vec::new();

        self.paint_header(ui, status);
        match status {
            Some(status) => {
                actions.extend(self.paint_changes(ui, status));
                actions.extend(self.paint_commit_box(ui, status));
            }
            None => self.paint_not_a_repo(ui),
        }

        GitOutcome {
            actions,
            rows_painted: self.last_rows_painted,
        }
    }

    /// The 24px header: "GIT" on the left; branch + change count on the
    /// right (UI_SPEC.md §8).
    fn paint_header(&mut self, ui: &mut Ui, status: Option<&GitStatus>) {
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
        ui.painter().text(
            pos2(rect.left() + space::S3, rect.center().y),
            Align2::LEFT_CENTER,
            HEADER_LABEL,
            label_font_id(),
            color::FG_FAINT,
        );
        let summary = match status {
            Some(status) => {
                let count = format!("{} changed", status.changed_count());
                match status.branch.as_deref() {
                    Some(branch) => format!("{branch} · {count}"),
                    None => count,
                }
            }
            None => "no repository".to_owned(),
        };
        ui.painter().text(
            pos2(rect.right() - space::S3, rect.center().y),
            Align2::RIGHT_CENTER,
            summary,
            label_font_id(),
            color::FG_MUTED,
        );
    }

    /// The "Changes (N)" section: a collapsible header row plus the file
    /// rows, capped and scrolled.
    fn paint_changes(&mut self, ui: &mut Ui, status: &GitStatus) -> Vec<GitAction> {
        let mut actions = Vec::new();

        // -- section header -------------------------------------------------
        let label = format!("{CHANGES_LABEL} ({})", status.changed_count());
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
        if response.clicked() {
            self.changes_expanded = !self.changes_expanded;
        }
        let text_color = if response.hovered() {
            color::FG_DEFAULT
        } else {
            color::FG_MUTED
        };
        let chevron = Rect::from_min_size(
            pos2(
                rect.left() + space::S1,
                rect.top() + (ROW_HEIGHT - 16.0) / 2.0,
            ),
            Vec2::splat(16.0),
        );
        paint_disclosure(ui, chevron, color::FG_FAINT, self.changes_expanded);
        ui.painter().text(
            pos2(chevron.right() + space::S2, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            section_font_id(),
            text_color,
        );

        if !self.changes_expanded {
            self.last_rows_painted = 0;
            return actions;
        }

        // -- empty state ------------------------------------------------------
        if status.entries.is_empty() {
            self.last_rows_painted = 0;
            ui.painter().text(
                pos2(
                    ui.cursor().left() + space::S3 + space::S4,
                    ui.cursor().top() + ROW_HEIGHT / 2.0,
                ),
                Align2::LEFT_CENTER,
                "No changes",
                section_font_id(),
                color::FG_FAINT,
            );
            ui.add_space(ROW_HEIGHT);
            return actions;
        }

        // -- rows ---------------------------------------------------------------
        let mut rows_painted = 0;
        let mut row_actions = Vec::new();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ROW_HEIGHT * MAX_VISIBLE_ROWS)
            .show(ui, |ui| {
                for (index, entry) in status.entries.iter().enumerate() {
                    self.paint_entry(ui, index, entry, &mut row_actions);
                    rows_painted += 1;
                }
            });
        self.last_rows_painted = rows_painted;
        actions.extend(row_actions);
        actions
    }

    /// One changed-file row: status letter + path, hover / selection
    /// surfaces, click-to-open-diff, and the hover `+` / `−` buttons
    /// (UI_SPEC.md §8).
    fn paint_entry(
        &mut self,
        ui: &mut Ui,
        index: usize,
        entry: &selora_git::FileStatus,
        actions: &mut Vec<GitAction>,
    ) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
        let selected = self.selected.as_deref() == Some(entry.abs_path.as_path());
        let hovered = response.hovered();

        // Hover --ink-faint, selected --ink-low (DESIGN_TOKENS.md §10).
        let bg = if selected {
            Some(color::INK_LOW)
        } else if hovered {
            Some(color::INK_FAINT)
        } else {
            None
        };
        if let Some(fill) = bg {
            ui.painter().rect_filled(rect, radius::R0, fill);
        }

        // [pad][letter][gap][path]
        let letter_x = rect.left() + space::S3;
        let path_x = letter_x + ACTION_SLOT + space::S2;
        ui.painter().text(
            pos2(letter_x, rect.center().y),
            Align2::LEFT_CENTER,
            entry.letter().to_string(),
            row_font_id(),
            color::FG_MUTED,
        );
        ui.painter().text(
            pos2(path_x, rect.center().y),
            Align2::LEFT_CENTER,
            entry.path.display().to_string(),
            row_font_id(),
            color::FG_DEFAULT,
        );

        // -- hover stage / unstage (right-aligned, 16px slots) -----------------
        let mut action_clicked = false;
        let mut slot_x = rect.right() - space::S3 - ACTION_SLOT;
        if hovered && entry.is_staged() {
            let slot = Rect::from_min_size(
                pos2(slot_x, rect.top() + (ROW_HEIGHT - ACTION_SLOT) / 2.0),
                Vec2::splat(ACTION_SLOT),
            );
            if self.paint_action_button(ui, index, "unstage", slot, '-') {
                actions.push(GitAction::Unstage(entry.abs_path.clone()));
                action_clicked = true;
            }
            slot_x -= ACTION_SLOT + space::S1;
        }
        if hovered && entry.has_worktree_changes() {
            let slot = Rect::from_min_size(
                pos2(slot_x, rect.top() + (ROW_HEIGHT - ACTION_SLOT) / 2.0),
                Vec2::splat(ACTION_SLOT),
            );
            if self.paint_action_button(ui, index, "stage", slot, '+') {
                actions.push(GitAction::Stage(entry.abs_path.clone()));
                action_clicked = true;
            }
        }

        // -- row click: select + open the diff ---------------------------------
        if !action_clicked && response.clicked() {
            self.selected = Some(entry.abs_path.clone());
            actions.push(GitAction::OpenDiff(entry.abs_path.clone()));
        }
    }

    /// One 16px hover action (`+` stage / `−` unstage). Returns whether it
    /// was clicked.
    fn paint_action_button(
        &mut self,
        ui: &mut Ui,
        index: usize,
        kind: &str,
        slot: Rect,
        glyph: char,
    ) -> bool {
        let id = ui.id().with(("git_action", kind, index));
        let response = ui.interact(slot, id, Sense::click());
        let glyph_color = if response.hovered() {
            color::FG_DEFAULT
        } else {
            color::FG_MUTED
        };
        ui.painter().text(
            slot.center(),
            Align2::CENTER_CENTER,
            glyph.to_string(),
            row_font_id(),
            glyph_color,
        );
        response.clicked()
    }

    /// The commit box (UI_SPEC.md §8): label, multi-line message input,
    /// right-aligned button, `Ctrl+Enter` binding.
    fn paint_commit_box(&mut self, ui: &mut Ui, status: &GitStatus) -> Vec<GitAction> {
        let mut actions = Vec::new();

        ui.add_space(space::S2);
        ui.painter().text(
            pos2(
                ui.cursor().left() + space::S3,
                ui.cursor().top() + space::S2 + ROW_HEIGHT / 2.0,
            ),
            Align2::LEFT_CENTER,
            "Commit message:",
            section_font_id(),
            color::FG_MUTED,
        );
        ui.add_space(ROW_HEIGHT);

        // Min 2 lines, max 6 (UI_SPEC.md §8): the input grows with the
        // draft, clamped.
        let desired_rows = self.commit_message.lines().count().clamp(2, 6);
        let editor = egui::Frame::default()
            .fill(color::INK_LOW)
            .stroke(egui::Stroke::new(1.0, color::INK_HI))
            .corner_radius(radius::R1)
            .inner_margin(egui::Margin::symmetric((space::S1 + 2.0) as i8, 2))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.commit_message)
                        .font(egui::FontId::new(type_scale::MD, font::FAMILY_MONO))
                        .desired_width(f32::INFINITY)
                        .desired_rows(desired_rows)
                        .hint_text("Commit message"),
                )
            })
            .inner;

        // Ctrl+Enter commits (UI_SPEC.md §8 / §16).
        let ctrl_enter = ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter));
        let commit_from_keyboard = ctrl_enter && editor.has_focus();

        ui.add_space(space::S2);
        let clicked = self.paint_commit_button(ui, status);
        if (clicked || commit_from_keyboard) && self.commit_ready(status) {
            actions.push(GitAction::Commit(self.commit_message.trim().to_owned()));
            self.commit_message.clear();
            self.selected = None;
        }
        actions
    }

    /// The right-aligned 32px commit button: `--ink-mid` fill, `--radius-1`,
    /// `--fg-default` label; disabled state per DESIGN_TOKENS.md §10
    /// (transparent fill, `--fg-disabled` label).
    fn paint_commit_button(&mut self, ui: &mut Ui, status: &GitStatus) -> bool {
        let enabled = self.commit_ready(status);
        let label = "Commit (Ctrl+Enter)";
        let text_color = if enabled {
            color::FG_DEFAULT
        } else {
            color::FG_DISABLED
        };
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
            text_color,
        );
        let size = vec2(galley.size().x + space::S3 * 2.0, COMMIT_BUTTON_HEIGHT);
        let (rect, response) = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.allocate_exact_size(size, Sense::click())
            })
            .inner;
        let _ = rect;
        if !enabled {
            return false; // display-only; no hover surface, no click
        }
        ui.painter().rect_filled(rect, radius::R1, color::INK_MID);
        if response.hovered() {
            // Focused-control border (DESIGN_TOKENS.md §5): 1px --fg-default,
            // painted inside the fill so it stays a full hairline.
            ui.painter().rect_stroke(
                rect,
                radius::R1,
                egui::Stroke::new(1.0, color::FG_DEFAULT),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, text_color);
        response.clicked()
    }

    /// The not-a-repository empty state.
    fn paint_not_a_repo(&mut self, ui: &mut Ui) {
        ui.add_space(space::S3);
        ui.painter().text(
            pos2(
                ui.cursor().left() + space::S3 + space::S4,
                ui.cursor().top() + ROW_HEIGHT / 2.0,
            ),
            Align2::LEFT_CENTER,
            "The open folder is not inside a git repository",
            section_font_id(),
            color::FG_FAINT,
        );
        self.last_rows_painted = 0;
    }
}

/// The "Changes" disclosure chevron: a two-line 16px glyph matching the
/// file tree's (DESIGN_TOKENS.md §8 chevron-down / chevron-right). The
/// sidebar's icon painters are module-private, so this is a local copy of
/// the same construction.
fn paint_disclosure(ui: &mut Ui, slot: Rect, color: egui::Color32, down: bool) {
    let center = slot.center();
    let half = slot.width().min(16.0) / 4.0;
    let top = center.y - half;
    let bottom = center.y + half;
    let (left, right) = (center.x - half, center.x + half);
    let stroke = egui::Stroke::new(1.5, color); // DESIGN_TOKENS.md §8 stroke
    let painter = ui.painter();
    if down {
        painter.line_segment([pos2(left, top), pos2(center.x, bottom)], stroke);
        painter.line_segment([pos2(center.x, bottom), pos2(right, top)], stroke);
    } else {
        painter.line_segment([pos2(left, bottom), pos2(center.x, top)], stroke);
        painter.line_segment([pos2(center.x, top), pos2(right, bottom)], stroke);
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use selora_git::Slot;

    fn file(path: &str, staged: Slot, worktree: Slot) -> selora_git::FileStatus {
        selora_git::FileStatus {
            path: PathBuf::from(path),
            abs_path: PathBuf::from(format!("/repo/{path}")),
            staged,
            worktree,
        }
    }

    fn fixture_status() -> GitStatus {
        GitStatus {
            branch: Some("main".to_owned()),
            entries: vec![
                file("src/editor/mod.rs", Slot::Unmodified, Slot::Modified),
                file("src/ui/theme.rs", Slot::Unmodified, Slot::Modified),
                file("notes.md", Slot::Unmodified, Slot::New),
            ],
            conflicts: Vec::new(),
        }
    }

    fn committed_status() -> GitStatus {
        GitStatus {
            branch: Some("main".to_owned()),
            entries: vec![file("src/editor/mod.rs", Slot::Modified, Slot::Unmodified)],
            conflicts: Vec::new(),
        }
    }

    #[test]
    fn commit_ready_requires_message_and_staged_changes() {
        let mut panel = GitPanel::new();
        let staged = committed_status();
        let unstaged = fixture_status();

        assert!(!panel.commit_ready(&unstaged)); // changes, none staged
        panel.commit_message = "fix: tokens".into();
        assert!(!panel.commit_ready(&unstaged)); // staged still 0

        panel.commit_message.clear();
        assert!(!panel.commit_ready(&staged)); // staged, no message
        panel.commit_message = "   ".into();
        assert!(!panel.commit_ready(&staged)); // whitespace-only message

        panel.commit_message = "feat: panel".into();
        assert!(panel.commit_ready(&staged));
    }

    #[test]
    fn paints_rows_headlessly() {
        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(400.0, 300.0));
            let mut panel = GitPanel::new();
            let status = fixture_status();
            let outcome = panel.show(ui, Some(&status));
            assert_eq!(outcome.rows_painted, 3);
            assert!(outcome.actions.is_empty());
        });
    }

    #[test]
    fn no_repo_paints_no_rows_and_no_actions() {
        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(400.0, 300.0));
            let mut panel = GitPanel::new();
            panel.commit_message = "message".into();
            let outcome = panel.show(ui, None);
            assert_eq!(outcome.rows_painted, 0);
            assert!(outcome.actions.is_empty());
        });
    }

    #[test]
    fn empty_status_paints_the_empty_state() {
        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(400.0, 300.0));
            let mut panel = GitPanel::new();
            let status = GitStatus::default();
            let outcome = panel.show(ui, Some(&status));
            assert_eq!(outcome.rows_painted, 0);
            assert!(outcome.actions.is_empty());
        });
    }

    #[test]
    fn collapsed_section_paints_no_rows() {
        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(400.0, 300.0));
            let mut panel = GitPanel::new();
            panel.changes_expanded = false;
            let status = fixture_status();
            let outcome = panel.show(ui, Some(&status));
            assert_eq!(outcome.rows_painted, 0);
            assert!(outcome.actions.is_empty());
        });
    }

    #[test]
    fn letters_follow_the_entry_model() {
        // The panel is fed by `selora_git`'s letter logic; sanity-check the
        // fixture shaping it paints.
        assert_eq!(fixture_status().entries[0].letter(), 'M');
        assert_eq!(fixture_status().entries[2].letter(), '?');
        assert_eq!(committed_status().entries[0].letter(), 'M');
    }
}
