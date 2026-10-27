//! The fuzzy file finder (UI_SPEC.md §5.1) — Phase 2c.
//!
//! An overlay palette in the shared command-palette shape (UI_SPEC.md §5):
//! centered horizontally, pinned at the top 25% of the parent rect, 560px
//! wide, at most 320px tall (scrolls after 8 rows), background `--ink-mid`,
//! 1px `--ink-hi` hairline border, `--radius-2`, and — per DESIGN_TOKENS.md
//! §6, the single shadow exception — **no drop shadow**.
//!
//! Self-contained by contract: no import from `palette::command` (Phase 2e
//! builds the command palette in parallel; the orchestrator may unify the
//! shared overlay shell at merge). The widget owns its whole state —
//! visibility, query, selection, results, candidate source — and recomputes
//! results from [`selora_fs::fuzzy::fuzzy_find`] as the query changes.
//!
//! Every color, font, and spacing value comes from `crate::theme` (the
//! typed DESIGN_TOKENS.md §11 contract re-exported from `selora-tokens`);
//! no value is hardcoded here (DESIGN_TOKENS.md §12: the no-decoration
//! rule).

use std::path::PathBuf;

use selora_fs::fuzzy::{self, Match};

use crate::theme::{color, font, radius, space, type_scale};

/// Palette width (UI_SPEC.md §5: "560px wide").
const WIDTH: f32 = 560.0;
/// Total palette height budget (UI_SPEC.md §5: "max 320px tall").
const MAX_HEIGHT: f32 = 320.0;
/// Result row height (UI_SPEC.md §5: "Rows: 28px tall").
const ROW_HEIGHT: f32 = 28.0;
/// 8 visible rows before scrolling (UI_SPEC.md §5: "scrolls after 8 rows").
const VISIBLE_ROWS: usize = 8;
/// Vertical position of the palette top, as a fraction of the parent rect
/// (UI_SPEC.md §5: "top 25% of the workbench").
const TOP_FRACTION: f32 = 0.25;
/// Placeholder for the query input.
const HINT: &str = "Find a file…";
/// Header label (UI_SPEC.md §5.1: "Header reads 'GO TO FILE'").
const HEADER: &str = "GO TO FILE";

/// The `Ctrl+P` fuzzy file finder overlay.
///
/// State machine: hidden ⇄ shown. Call [`FuzzyFileFinder::show`] (typically
/// from the `Ctrl+P` keybind), feed the workspace files in with
/// [`FuzzyFileFinder::set_source`], then paint every open frame with
/// [`FuzzyFileFinder::show_ui`]. `show_ui` returns `Some(path)` on the frame
/// the user accepts a result (Enter or click); the caller maps that to
/// `Effect::OpenFile`.
///
/// Hidden frames are a no-op: `show_ui` returns `None` without touching
/// egui, so the call can sit unconditionally in the frame loop.
#[derive(Debug, Clone)]
pub struct FuzzyFileFinder {
    open: bool,
    query: String,
    /// Query the current `results` were computed for (recompute trigger).
    last_query: String,
    /// Set when `set_source` replaced the candidates under a live query.
    source_dirty: bool,
    selected: usize,
    results: Vec<Match>,
    source: Vec<(PathBuf, String)>,
    /// Focus the query input on the next frame after `show()`.
    want_focus: bool,
    /// Scroll the newly-selected row into view on the next frame.
    scroll_to_selection: bool,
}

impl Default for FuzzyFileFinder {
    fn default() -> Self {
        Self::new()
    }
}

impl FuzzyFileFinder {
    pub fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            last_query: String::new(),
            source_dirty: false,
            selected: 0,
            results: Vec::new(),
            source: Vec::new(),
            want_focus: false,
            scroll_to_selection: false,
        }
    }

    /// Open the palette (resetting query, selection, results).
    pub fn show(&mut self) {
        self.open = true;
        self.query.clear();
        self.last_query.clear();
        self.selected = 0;
        self.results.clear();
        self.source_dirty = true;
        self.want_focus = true;
        self.scroll_to_selection = false;
    }

    /// Close the palette. Query and results are dropped.
    pub fn hide(&mut self) {
        self.open = false;
        self.query.clear();
        self.last_query.clear();
        self.results.clear();
        self.selected = 0;
        self.want_focus = false;
        self.scroll_to_selection = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Replace the candidate files (path + category tag). Typically the
    /// workspace file list from the tree phase, refreshed on open-folder.
    pub fn set_source(&mut self, items: Vec<(PathBuf, String)>) {
        self.source = items;
        self.source_dirty = true;
    }

    /// The current query (the text in the input row).
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Set the query programmatically (tests, agent-mode integration).
    /// Results recompute on the next `show_ui` frame.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_owned();
    }

    /// The currently ranked results, after the most recent frame.
    pub fn results(&self) -> &[Match] {
        &self.results
    }

    /// Index of the selected row within [`Self::results`].
    pub fn selected_row(&self) -> usize {
        self.selected
    }

    /// Paint the overlay for this frame. Returns `Some(path)` exactly once,
    /// on the frame the user accepts a result (Enter / row click); the
    /// palette closes itself on accept and on Esc.
    pub fn show_ui(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        if !self.open {
            return None;
        }

        // Position: centered horizontally, top 25% of the parent rect
        // (UI_SPEC.md §5).
        let bounds = ui.max_rect();
        let top = bounds.top() + bounds.height() * TOP_FRACTION;
        let left = bounds.center().x - WIDTH / 2.0;

        let ctx = ui.ctx().clone();
        let mut emitted: Option<PathBuf> = None;
        egui::Area::new(egui::Id::new("selora_fuzzy_file_palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(left, top))
            .show(&ctx, |area_ui| {
                // DESIGN_TOKENS.md §6 exception: hairline border only.
                egui::Frame::new()
                    .fill(color::INK_MID)
                    .stroke(egui::Stroke::new(1.0, color::INK_HI))
                    .corner_radius(egui::CornerRadius::same(radius::R2 as u8))
                    .inner_margin(egui::Margin::symmetric(space::S4 as i8, space::S3 as i8))
                    .show(area_ui, |panel| {
                        if let Some(path) = self.paint_panel(panel) {
                            emitted = Some(path);
                        }
                    });
            });
        emitted
    }

    /// The palette body: header, input row, result rows, key handling.
    fn paint_panel(&mut self, panel: &mut egui::Ui) -> Option<PathBuf> {
        self.recompute_if_stale();

        // The frame's inner margins (S4 horizontal each side) are added on
        // top of this content width, totaling the spec's 560px.
        panel.set_width(WIDTH - 2.0 * space::S4);
        // Keep the height budget honest even with pathological content.
        panel.set_max_height(MAX_HEIGHT);

        let mut emitted: Option<PathBuf> = None;
        let mut close_after = false;

        // -- Header: "GO TO FILE" (type-xs, fg-faint; same treatment as the
        //    FILES / CHAT panel headers, UI_SPEC.md §2.2/§4).
        panel.label(
            egui::RichText::new(HEADER)
                .font(egui::FontId::new(type_scale::XS, font::FAMILY_SANS))
                .color(color::FG_FAINT),
        );
        panel.add_space(space::S1);

        // -- Input row: `>` prefix glyph (fg-faint) + single-line query.
        let want_focus = self.want_focus;
        panel.horizontal(|row| {
            let glyph = row.label(
                egui::RichText::new(">")
                    .font(egui::FontId::new(type_scale::LG, font::FAMILY_SANS))
                    .color(color::FG_FAINT),
            );
            let edit = egui::TextEdit::singleline(&mut self.query)
                .id(egui::Id::new("selora_fuzzy_query"))
                .hint_text(HINT)
                .font(egui::FontId::new(type_scale::LG, font::FAMILY_SANS))
                .desired_width(row.available_width() - glyph.rect.width() - space::S1);
            let resp = row.add(edit);
            if want_focus {
                resp.request_focus();
            }
        });
        self.want_focus = false;

        panel.add_space(space::S2);

        // -- Result rows: 28px tall, path in type-md mono, category tag in
        //    type-xs fg-faint on the right; selected row on ink-low
        //    (UI_SPEC.md §5.1). No per-row borders (DESIGN_TOKENS.md §5.2).
        let total = self.results.len();
        let scroll_to_selection = self.scroll_to_selection;
        egui::ScrollArea::vertical()
            .id_salt("selora_fuzzy_results")
            .auto_shrink([false, false])
            .max_height(ROW_HEIGHT * VISIBLE_ROWS as f32)
            .show_rows(panel, ROW_HEIGHT, total, |rows, range| {
                for i in range {
                    let is_selected = i == self.selected;
                    let size = egui::vec2(rows.available_width(), ROW_HEIGHT);
                    let (rect, resp) = rows.allocate_exact_size(size, egui::Sense::click());

                    // Row background: selected = ink-low; hover = ink-faint
                    // (DESIGN_TOKENS.md §10).
                    let bg = if is_selected {
                        color::INK_LOW
                    } else if resp.hovered() {
                        color::INK_FAINT
                    } else {
                        color::INK_MID
                    };
                    if bg != color::INK_MID {
                        rows.painter()
                            .rect_filled(rect, egui::CornerRadius::ZERO, bg);
                    }

                    let m = &self.results[i];

                    // Category tag, right-aligned (type-xs, fg-faint).
                    let tag_font = egui::FontId::new(type_scale::XS, font::FAMILY_SANS);
                    let tag_galley =
                        rows.painter()
                            .layout_no_wrap(m.tag.clone(), tag_font, color::FG_FAINT);
                    let tag_width = tag_galley.size().x;
                    rows.painter().galley(
                        rect.right_center()
                            - egui::vec2(tag_width + space::S1, tag_galley.size().y / 2.0),
                        tag_galley,
                        color::FG_FAINT,
                    );

                    // File path, left-aligned (type-md, mono, fg-default),
                    // truncated with an ellipsis before it can collide with
                    // the tag.
                    let path_font = egui::FontId::new(type_scale::MD, font::FAMILY_MONO);
                    let text = fuzzy::display_path(&m.path);
                    let budget = rect.width() - tag_width - 3.0 * space::S1;
                    let text = truncate_to_width(rows, text, &path_font, budget);
                    rows.painter().text(
                        rect.left_center() + egui::vec2(space::S1, 0.0),
                        egui::Align2::LEFT_CENTER,
                        text,
                        path_font,
                        color::FG_DEFAULT,
                    );

                    if resp.clicked() {
                        emitted = Some(m.path.clone());
                        close_after = true;
                    }
                    if is_selected && scroll_to_selection {
                        rows.scroll_to_rect(rect, None);
                    }
                }
            });
        self.scroll_to_selection = false;

        // -- Key handling: Esc closes; Up/Down move the selection; Enter
        //    accepts the selected row (UI_SPEC.md §5).
        let (enter, escape, up, down) = panel.ctx().input(|i| {
            (
                i.key_pressed(egui::Key::Enter) && i.modifiers.is_none(),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
            )
        });

        if up && !self.results.is_empty() {
            self.selected = self.selected.saturating_sub(1);
            self.scroll_to_selection = true;
        } else if down && !self.results.is_empty() {
            self.selected = (self.selected + 1).min(self.results.len() - 1);
            self.scroll_to_selection = true;
        }
        if escape {
            close_after = true;
        } else if enter && emitted.is_none() {
            emitted = self.results.get(self.selected).map(|m| m.path.clone());
            if emitted.is_some() {
                close_after = true;
            }
        }

        if close_after {
            self.hide();
        }

        emitted
    }

    /// Recompute `results` from [`fuzzy::fuzzy_find`] when the query or the
    /// candidate source changed since the last computation, resetting the
    /// selection into range.
    fn recompute_if_stale(&mut self) {
        let changed = self.source_dirty || self.query != self.last_query;
        if !changed {
            return;
        }
        self.results = fuzzy::fuzzy_find(&self.query, &self.source);
        self.last_query.clone_from(&self.query);
        self.source_dirty = false;
        self.selected = 0;
        self.scroll_to_selection = true;
    }
}

/// Truncate `text` (in `font`) to `max_width` px, suffixing an ellipsis.
/// One layout pass decides; the shrink loop only runs when needed.
fn truncate_to_width(
    ui: &egui::Ui,
    text: String,
    font_id: &egui::FontId,
    max_width: f32,
) -> String {
    if max_width <= 0.0 || text.is_empty() {
        return text;
    }
    let full = ui
        .painter()
        .layout_no_wrap(text.clone(), font_id.clone(), color::FG_DEFAULT);
    if full.size().x <= max_width {
        return text;
    }
    // Scale down by the measured width ratio, then walk off characters
    // until the ellipsized form fits.
    let mut len = ((text.chars().count() as f32) * max_width / full.size().x) as usize;
    loop {
        len = len.saturating_sub(1);
        let mut cut: String = text.chars().take(len).collect();
        if cut.is_empty() {
            return "…".into();
        }
        cut.push('…');
        let galley = ui
            .painter()
            .layout_no_wrap(cut.clone(), font_id.clone(), color::FG_DEFAULT);
        if galley.size().x <= max_width {
            return cut;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn source() -> Vec<(PathBuf, String)> {
        vec![
            (PathBuf::from("src/editor/mod.rs"), "editor".to_owned()),
            (PathBuf::from("src/editor/buffer.rs"), "editor".to_owned()),
            (PathBuf::from("src/main.rs"), "src".to_owned()),
            (PathBuf::from("assets/logo.svg"), "assets".to_owned()),
        ]
    }

    fn display_paths(finder: &FuzzyFileFinder) -> Vec<String> {
        finder
            .results()
            .iter()
            .map(|m| fuzzy::display_path(&m.path))
            .collect()
    }

    fn key_event(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn typing_a_query_filters_rows() {
        let mut finder = FuzzyFileFinder::new();
        finder.set_source(source());
        finder.show();
        assert!(finder.is_open());

        // "Typing": the query changes between frames.
        finder.set_query("mod");
        egui::__run_test_ui(|ui| {
            assert!(finder.show_ui(ui).is_none());
        });

        assert_eq!(display_paths(&finder), vec!["src/editor/mod.rs".to_owned()]);
        // Still open — nothing accepted yet.
        assert!(finder.is_open());

        // Narrow further: "buff" switches the filtered set.
        finder.set_query("buff");
        egui::__run_test_ui(|ui| {
            assert!(finder.show_ui(ui).is_none());
        });
        assert_eq!(
            display_paths(&finder),
            vec!["src/editor/buffer.rs".to_owned()]
        );
    }

    #[test]
    fn enter_emits_the_selected_path_and_closes() {
        let mut finder = FuzzyFileFinder::new();
        finder.set_source(source());
        finder.show();
        finder.set_query("buff");
        // Frame 1: filter + render rows.
        egui::__run_test_ui(|ui| {
            assert!(finder.show_ui(ui).is_none());
        });

        // Frame 2: Enter accepts the selected row.
        let mut emitted = None;
        egui::__run_test_ui(|ui| {
            ui.ctx()
                .input_mut(|i| i.events.push(key_event(egui::Key::Enter)));
            emitted = finder.show_ui(ui);
        });
        assert_eq!(emitted, Some(PathBuf::from("src/editor/buffer.rs")));
        assert!(!finder.is_open());
        // Hidden frames are a no-op.
        egui::__run_test_ui(|ui| {
            assert!(finder.show_ui(ui).is_none());
        });
    }

    #[test]
    fn arrow_keys_move_selection_and_escape_closes() {
        let mut finder = FuzzyFileFinder::new();
        finder.set_source(source());
        finder.show();
        finder.set_query("ed"); // matches both src/editor files
        egui::__run_test_ui(|ui| {
            assert!(finder.show_ui(ui).is_none());
        });
        assert_eq!(finder.results().len(), 2);
        assert_eq!(finder.selected_row(), 0);

        egui::__run_test_ui(|ui| {
            ui.ctx()
                .input_mut(|i| i.events.push(key_event(egui::Key::ArrowDown)));
            assert!(finder.show_ui(ui).is_none());
        });
        assert_eq!(finder.selected_row(), 1);

        // Enter now emits the second row.
        let second = finder.results()[1].path.clone();
        let mut emitted = None;
        egui::__run_test_ui(|ui| {
            ui.ctx()
                .input_mut(|i| i.events.push(key_event(egui::Key::Enter)));
            emitted = finder.show_ui(ui);
        });
        assert_eq!(emitted, Some(second));

        // Escape closes without emitting.
        let mut finder = FuzzyFileFinder::new();
        finder.set_source(source());
        finder.show();
        finder.set_query("mod");
        egui::__run_test_ui(|ui| {
            ui.ctx()
                .input_mut(|i| i.events.push(key_event(egui::Key::Escape)));
            assert!(finder.show_ui(ui).is_none());
        });
        assert!(!finder.is_open());
    }

    #[test]
    fn show_resets_previous_session_state() {
        let mut finder = FuzzyFileFinder::new();
        finder.set_source(source());
        finder.show();
        finder.set_query("mod");
        egui::__run_test_ui(|ui| {
            finder.show_ui(ui);
        });
        assert_eq!(finder.results().len(), 1);

        finder.hide();
        finder.show();
        assert!(finder.query().is_empty());
        assert!(finder.results().is_empty());
        assert_eq!(finder.selected_row(), 0);
    }

    #[test]
    fn hidden_palette_is_a_no_op_frame() {
        let mut finder = FuzzyFileFinder::new();
        finder.set_source(source());
        // Never shown: show_ui must not build any egui output.
        egui::__run_test_ui(|ui| {
            assert!(finder.show_ui(ui).is_none());
        });
        assert!(finder.results().is_empty());
    }
}
