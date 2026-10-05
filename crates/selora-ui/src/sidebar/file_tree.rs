//! The file tree sidebar content (UI_SPEC.md §2.2). Phase 2b.
//!
//! The shell owns the left panel itself (240px default, 160px min,
//! 480px max, hairline borders); this widget paints the panel's content:
//!
//! - a 24px "FILES" header (`--type-xs`, `--fg-faint`, wide tracking),
//! - virtualization-ready rows from [`FileTreeView::rows`] (22px tall,
//!   12px indent per level, disclosure chevron in `--fg-faint`,
//!   folder/file icons painted in code per DESIGN_TOKENS.md §8),
//! - the right-click context menu (New File, New Folder, Rename, Delete,
//!   Copy Path, Reveal in Explorer) on an `--ink-mid` / 1px `--ink-hi` /
//!   `--radius-2` popup,
//! - an inline name editor for New File / New Folder / Rename that
//!   performs the disk operations via `selora_fs::tree`,
//! - the bottom-right `+` (new file) affordance.
//!
//! Events ([`TreeEvent`]) are returned to the shell each frame; the
//! shell routes them (open buffers, `Effect::RefreshFileTree`, toasts).

use std::path::{Path, PathBuf};

use egui::{pos2, vec2, Align2, Key, Rect, Sense, Ui, Vec2};

use selora_fs::tree::{
    create_dir, create_file, delete_path, rename, FileTreeNode, FileTreeView, FsError, TreeRow,
    TreeRowKind, LOAD_MORE_LABEL,
};

use crate::sidebar::icons;
use crate::theme::{color, font, radius, space, type_scale};

/// Header bar height (UI_SPEC.md §2.2).
pub const HEADER_HEIGHT: f32 = 24.0;
/// Row height (UI_SPEC.md §2.2).
pub const ROW_HEIGHT: f32 = 22.0;
/// Indent per tree level (UI_SPEC.md §2.2).
pub const INDENT_PER_LEVEL: f32 = 12.0;
/// Tap target of the bottom-right new-file button (UI_SPEC.md §2.2).
pub const FOOTER_HEIGHT: f32 = 24.0;
/// The panel's sidebar label (UI_SPEC.md §2.2).
pub const HEADER_LABEL: &str = "FILES";

/// Row font: `--type-sm` in the mono family (DESIGN_TOKENS.md §2.1 —
/// the file tree is a `--font-mono` surface).
fn row_font_id() -> egui::FontId {
    egui::FontId::new(type_scale::SM, font::FAMILY_MONO)
}

// -----------------------------------------------------------------------------
// Events
// -----------------------------------------------------------------------------

/// What the sidebar tells the shell each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeEvent {
    /// A file was activated (clicked) — the shell opens it in the editor.
    OpenFile(PathBuf),
    /// A disk operation succeeded; the tree is stale — the shell should
    /// enqueue `Effect::RefreshFileTree` and deliver a fresh
    /// `AppMessage::FileTreeUpdated`.
    TreeChanged,
    /// An operation failed; surface as a toast (PLAN.md §8: no silent
    /// failures).
    Error(String),
}

/// Result of one `show` call.
#[derive(Debug, Default)]
pub struct ShowOutcome {
    /// Events for the shell, in frame order.
    pub events: Vec<TreeEvent>,
    /// How many tree rows were painted (exposed for the headless paint
    /// test; the shell ignores it).
    pub rows_painted: usize,
}

// -----------------------------------------------------------------------------
// Inline editor (New File / New Folder / Rename)
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    NewFile,
    NewFolder,
    Rename,
}

/// An in-progress inline name editor.
#[derive(Clone, Debug)]
struct PendingEdit {
    kind: EditKind,
    /// Directory the new entry is created in (Create) or the path being
    /// renamed (Rename).
    target: PathBuf,
    /// The draft name, bound to the inline [`egui::TextEdit`].
    name: String,
    /// Grab keyboard focus on the first frame.
    focus: bool,
}

// -----------------------------------------------------------------------------
// Context menu
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuAction {
    NewFile,
    NewFolder,
    Rename,
    Delete,
    CopyPath,
    Reveal,
}

/// The context-menu frame: `--ink-mid` fill, 1px `--ink-hi` hairline,
/// `--radius-2` (UI_SPEC.md §2.2), no shadow (DESIGN_TOKENS.md §6).
fn menu_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(color::INK_MID)
        .stroke(egui::Stroke::new(1.0, color::INK_HI))
        .corner_radius(radius::R2)
        .inner_margin(egui::Margin::same(space::S1 as i8))
}

/// Spawn `explorer.exe` for `path` (Reveal in Explorer; Windows only —
/// no-op elsewhere). Selects the file when `path` is a file, opens the
/// folder when it is a directory.
fn reveal_in_explorer(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("explorer.exe");
        if path.is_dir() {
            command.arg(path);
        } else {
            command.arg(format!("/select,{}", path.display()));
        }
        command.spawn()?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// FileTreePanel
// -----------------------------------------------------------------------------

/// The file tree sidebar: a [`FileTreeView`] plus the transient UI state
/// (inline editor). Create one per app and call [`FileTreePanel::show`]
/// every frame from the shell's left panel.
pub struct FileTreePanel {
    /// The tree UI state (expanded set, selection). Public so the shell
    /// can drive it (apply `AppMessage::FileTreeUpdated`, keybinds).
    pub view: FileTreeView,
    /// The active inline name editor, if any.
    pending: Option<PendingEdit>,
}

impl Default for FileTreePanel {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTreePanel {
    /// An empty panel; install a tree with [`FileTreePanel::set_tree`].
    pub fn new() -> Self {
        Self {
            view: FileTreeView::default(),
            pending: None,
        }
    }

    /// Install a freshly built tree (from `AppMessage::FileTreeUpdated`),
    /// cancelling any in-flight editor.
    pub fn set_tree(&mut self, root: FileTreeNode) {
        self.view.set_root(root);
        self.pending = None;
    }

    /// The tree root, if a folder is open.
    pub fn tree(&self) -> Option<&FileTreeNode> {
        self.view.root.as_ref()
    }

    /// Paint the panel content. Returns the frame's events plus a paint
    /// count (used by the headless test).
    pub fn show(&mut self, ui: &mut Ui) -> ShowOutcome {
        let mut events = Vec::new();

        self.paint_header(ui);
        let (rows_painted, row_events) = self.paint_pending_and_rows(ui);
        events.extend(row_events);
        events.extend(self.paint_footer(ui));

        ShowOutcome {
            events,
            rows_painted,
        }
    }

    /// The 24px header: "FILES" in `--type-xs` `--fg-faint` with wide
    /// tracking (the label is uppercase; egui has no letter-spacing
    /// primitive — the 0.04em token is imperceptible at 11px, so the
    /// uppercase form carries the affordance).
    fn paint_header(&mut self, ui: &mut Ui) {
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
        ui.painter().text(
            pos2(rect.left() + space::S3, rect.center().y),
            Align2::LEFT_CENTER,
            HEADER_LABEL,
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_FAINT,
        );
    }

    /// The inline editor (if pending) and the scrolled rows.
    fn paint_pending_and_rows(&mut self, ui: &mut Ui) -> (usize, Vec<TreeEvent>) {
        let mut events = Vec::new();

        // -- inline editor --------------------------------------------------
        if let Some(mut edit) = self.pending.take() {
            let response = self.paint_inline_editor(ui, &mut edit);
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let escape = response.has_focus() && ui.input(|i| i.key_pressed(Key::Escape));
            if enter {
                self.apply_edit(&edit, &mut events);
            } else if escape || response.lost_focus() {
                // Cancelled: Esc or focus moved elsewhere.
            } else {
                self.pending = Some(edit);
            }
        }

        // -- rows -----------------------------------------------------------
        let mut rows_painted = 0;
        let rows = self.view.rows();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in rows {
                    self.paint_row(ui, &row, &mut events);
                    rows_painted += 1;
                }
            });

        (rows_painted, events)
    }

    /// One row: 22px tall, chevron + folder/file icon + label, hover and
    /// selection surfaces, click/toggle/open, right-click context menu.
    fn paint_row(&mut self, ui: &mut Ui, row: &TreeRow, events: &mut Vec<TreeEvent>) {
        let width = ui.available_width();
        let is_node = matches!(row.kind, TreeRowKind::Node);
        let (rect, response) = ui.allocate_exact_size(
            vec2(width, ROW_HEIGHT),
            if is_node {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let selected = self.view.selected.as_deref() == Some(row.path.as_path());
        let hovered = response.hovered();

        // Row background: hover --ink-faint, selected --ink-low, otherwise
        // nothing (the canvas shows through). No row borders
        // (DESIGN_TOKENS.md §5.2).
        let bg = if selected {
            Some(color::INK_LOW)
        } else if hovered && is_node {
            Some(color::INK_FAINT)
        } else {
            None
        };
        if let Some(fill) = bg {
            ui.painter().rect_filled(rect, radius::R0, fill);
        }

        // Icon color follows the row's text color (currentColor,
        // DESIGN_TOKENS.md §8); the chevron is always --fg-faint
        // (UI_SPEC.md §2.2).
        let text_color = if selected || hovered {
            color::FG_DEFAULT
        } else {
            color::FG_MUTED
        };

        let x = rect.left() + space::S1 + row.depth as f32 * INDENT_PER_LEVEL;
        let icon_y = rect.top() + (ROW_HEIGHT - 16.0) / 2.0;
        let slot = |ix: f32| Rect::from_min_size(pos2(ix, icon_y), Vec2::splat(16.0));

        // Geometry: [chevron 16][4][folder 16][8][label] for directories;
        // files align their sheet with the folder column so labels line up.
        let icon_x = x + 16.0 + space::S1;
        let label_x = icon_x + 16.0 + space::S2;

        if row.is_dir {
            icons::paint_chevron(ui.painter(), slot(x), color::FG_FAINT, row.is_expanded);
            icons::paint_folder(ui.painter(), slot(icon_x), text_color, row.is_expanded);
        } else if is_node {
            icons::paint_sheet(ui.painter(), slot(icon_x), text_color);
        }

        let label_color = if is_node { text_color } else { color::FG_FAINT };
        let label_x = if is_node { label_x } else { icon_x };
        ui.painter().text(
            pos2(label_x, rect.center().y),
            Align2::LEFT_CENTER,
            if is_node {
                row.name.as_str()
            } else {
                LOAD_MORE_LABEL
            },
            row_font_id(),
            label_color,
        );

        if !is_node {
            return; // "load more" marker is display-only in Lite.
        }

        // -- interaction ------------------------------------------------------
        if response.clicked() {
            self.view.select(&row.path);
            if row.is_dir {
                self.view.toggle(&row.path);
            } else {
                events.push(TreeEvent::OpenFile(row.path.clone()));
            }
        }

        // -- context menu (UI_SPEC.md §2.2) -----------------------------------
        let mut action: Option<MenuAction> = None;
        egui::Popup::context_menu(&response)
            .frame(menu_frame())
            .show(|ui| {
                ui.set_min_width(170.0);
                if ui.button("New File").clicked() {
                    action = Some(MenuAction::NewFile);
                }
                if ui.button("New Folder").clicked() {
                    action = Some(MenuAction::NewFolder);
                }
                if ui.button("Rename").clicked() {
                    action = Some(MenuAction::Rename);
                }
                if ui.button("Delete").clicked() {
                    action = Some(MenuAction::Delete);
                }
                if ui.button("Copy Path").clicked() {
                    action = Some(MenuAction::CopyPath);
                }
                if ui.button("Reveal in Explorer").clicked() {
                    action = Some(MenuAction::Reveal);
                }
            });
        if let Some(menu_action) = action {
            self.apply_menu_action(menu_action, row, ui.ctx().clone(), events);
        }
    }

    /// The inline name editor row.
    fn paint_inline_editor(&mut self, ui: &mut Ui, edit: &mut PendingEdit) -> egui::Response {
        let hint = match edit.kind {
            EditKind::NewFile => "file name",
            EditKind::NewFolder => "folder name",
            EditKind::Rename => "new name",
        };
        let editor = egui::Frame::default()
            .fill(color::INK_LOW)
            .stroke(egui::Stroke::new(1.0, color::INK_HI))
            .corner_radius(radius::R1)
            .inner_margin(egui::Margin::symmetric((space::S1 + 2.0) as i8, 2))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut edit.name)
                        .font(row_font_id())
                        .desired_width(ui.available_width() - 8.0)
                        .hint_text(hint),
                )
            })
            .inner;

        if edit.focus {
            editor.request_focus();
            edit.focus = false;
        }
        editor
    }

    /// Commit an inline editor: run the disk operation and report.
    fn apply_edit(&mut self, edit: &PendingEdit, events: &mut Vec<TreeEvent>) {
        if edit.name.trim().is_empty() {
            return; // Empty name: treat as cancel, no error toast.
        }
        let result: Result<PathBuf, FsError> = match edit.kind {
            EditKind::NewFile => create_file(&edit.target, &edit.name),
            EditKind::NewFolder => create_dir(&edit.target, &edit.name),
            EditKind::Rename => rename(&edit.target, &edit.name),
        };
        match result {
            Ok(new_path) => {
                events.push(TreeEvent::TreeChanged);
                self.view.select(&new_path);
                if let Some(parent) = new_path.parent() {
                    self.view.expand(parent);
                }
            }
            Err(err) => events.push(TreeEvent::Error(err.to_string())),
        }
    }

    /// Execute a context-menu choice.
    fn apply_menu_action(
        &mut self,
        action: MenuAction,
        row: &TreeRow,
        ctx: egui::Context,
        events: &mut Vec<TreeEvent>,
    ) {
        // New entries land inside the clicked directory, or next to a
        // clicked file.
        let parent = if row.is_dir {
            row.path.clone()
        } else {
            row.path.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        match action {
            MenuAction::NewFile => {
                self.pending = Some(PendingEdit {
                    kind: EditKind::NewFile,
                    target: parent,
                    name: String::new(),
                    focus: true,
                });
            }
            MenuAction::NewFolder => {
                self.pending = Some(PendingEdit {
                    kind: EditKind::NewFolder,
                    target: parent,
                    name: String::new(),
                    focus: true,
                });
            }
            MenuAction::Rename => {
                self.pending = Some(PendingEdit {
                    kind: EditKind::Rename,
                    target: row.path.clone(),
                    name: row.name.clone(),
                    focus: true,
                });
            }
            MenuAction::Delete => match delete_path(&row.path) {
                Ok(()) => {
                    if self.view.selected.as_deref() == Some(row.path.as_path()) {
                        self.view.selected = None;
                    }
                    events.push(TreeEvent::TreeChanged);
                }
                Err(err) => events.push(TreeEvent::Error(err.to_string())),
            },
            MenuAction::CopyPath => ctx.copy_text(row.path.display().to_string()),
            MenuAction::Reveal => {
                if let Err(err) = reveal_in_explorer(&row.path) {
                    events.push(TreeEvent::Error(format!(
                        "reveal in explorer failed: {err}"
                    )));
                }
            }
        }
    }

    /// The bottom-right `+` (new file) affordance: 24px tap target,
    /// `--fg-faint`, hover `--fg-default`.
    fn paint_footer(&mut self, ui: &mut Ui) -> Vec<TreeEvent> {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), FOOTER_HEIGHT), Sense::click());
        if response.clicked() {
            let parent = self.selected_dir_or_root();
            self.pending = Some(PendingEdit {
                kind: EditKind::NewFile,
                target: parent,
                name: String::new(),
                focus: true,
            });
        }
        let icon_color = if response.hovered() {
            color::FG_DEFAULT
        } else {
            color::FG_FAINT
        };
        let icon_rect = Rect::from_center_size(
            pos2(rect.right() - space::S3, rect.center().y),
            Vec2::splat(16.0),
        );
        icons::paint_plus(ui.painter(), icon_rect, icon_color);
        Vec::new()
    }

    /// Where the footer `+` creates files: the selected directory, the
    /// selected file's parent, or the workspace root.
    fn selected_dir_or_root(&self) -> PathBuf {
        if let Some(selected) = &self.view.selected {
            if let Some(node) = self.view.node(selected) {
                if node.is_dir {
                    return node.path.clone();
                }
            }
            if let Some(parent) = selected.parent() {
                return parent.to_path_buf();
            }
        }
        self.view
            .root_path()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_tree() -> FileTreeNode {
        fn node(path: &str, is_dir: bool, children: Vec<FileTreeNode>) -> FileTreeNode {
            FileTreeNode {
                name: Path::new(path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                path: PathBuf::from(path),
                is_dir,
                children,
                truncated: false,
            }
        }
        node(
            "/proj",
            true,
            vec![
                node(
                    "/proj/src",
                    true,
                    vec![
                        node("/proj/src/main.rs", false, vec![]),
                        node("/proj/src/ui", true, vec![]),
                    ],
                ),
                node("/proj/Cargo.toml", false, vec![]),
            ],
        )
    }

    #[test]
    fn paints_rows_headlessly() {
        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(240.0, 400.0));
            let mut panel = FileTreePanel::new();
            panel.set_tree(fixture_tree());
            // Root expanded by default: proj, src, Cargo.toml.
            let outcome = panel.show(ui);
            assert_eq!(outcome.rows_painted, 3);
            assert!(outcome.events.is_empty());
        });
    }

    #[test]
    fn row_count_follows_expansion() {
        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(240.0, 400.0));
            let mut panel = FileTreePanel::new();
            panel.set_tree(fixture_tree());
            panel.view.expand(Path::new("/proj/src"));
            let outcome = panel.show(ui);
            // proj, src, main.rs, ui, Cargo.toml
            assert_eq!(outcome.rows_painted, 5);
        });
    }

    #[test]
    fn empty_panel_paints_no_rows() {
        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(240.0, 400.0));
            let mut panel = FileTreePanel::new();
            let outcome = panel.show(ui);
            assert_eq!(outcome.rows_painted, 0);
        });
    }

    #[test]
    fn paint_load_more_marker_row() {
        let mut truncated_root = fixture_tree();
        let src = truncated_root
            .children
            .iter_mut()
            .find(|child| child.name == "src")
            .expect("src");
        src.truncated = true;

        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(240.0, 400.0));
            let mut panel = FileTreePanel::new();
            panel.set_tree(truncated_root.clone());
            panel.view.expand(Path::new("/proj/src"));
            let outcome = panel.show(ui);
            // proj, src, main.rs, ui, "load more", Cargo.toml
            assert_eq!(outcome.rows_painted, 6);
        });
    }

    #[test]
    fn set_tree_resets_state() {
        let mut panel = FileTreePanel::new();
        panel.set_tree(fixture_tree());
        panel.view.select(Path::new("/proj/Cargo.toml"));
        panel.set_tree(fixture_tree());
        assert!(panel.view.selected.is_none());
        assert!(panel.view.is_expanded(Path::new("/proj")));
        assert!(!panel.view.is_expanded(Path::new("/proj/src")));
    }
}
