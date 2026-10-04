//! The [`Editor`] widget — the central editor canvas (UI_SPEC.md §3.2).
//!
//! One widget per pane, backed by a shared [`Buffer`] behind an
//! `Arc<RwLock<…>` (ARCHITECTURE.md §3.2: LSP, file IO, and the editor all
//! hold handles to the same buffer). The widget owns only view state:
//! cursor, selection anchor, and the scroll offset. Rendering is done by
//! hand on the [`egui::Painter`] — each visible line is laid out as a galley
//! and painted at its row position; the gutter, selection rectangles, caret
//! row, and blinking caret are painted underneath and on top of it.
//!
//! Usage (this is the shape Phase 1c embeds) — the widget is added by
//! mutable reference, like all stateful egui widgets, so the view state
//! (cursor, selection, scroll) lives with the caller and survives frames:
//!
//! ```
//! # use std::sync::{Arc, RwLock};
//! # use selora_core::Buffer;
//! # use selora_editor::Editor;
//! # let buffer = Arc::new(RwLock::new(Buffer::from_text("fn main() {}")));
//! # let mut editor = Editor::new(buffer);
//! egui::__run_test_ui(|ui| {
//!     ui.add(&mut editor);
//! });
//! ```

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use egui::{
    pos2, vec2, Align2, Event, FontId, Galley, Key, Painter, PointerButton, Pos2, Rangef, Rect,
    Sense, Stroke, Widget,
};
use selora_core::{Buffer, Position};

use crate::{layout, motion, theme};

/// An editor pane: gutter + text canvas for one shared [`Buffer`].
pub struct Editor {
    buffer: Arc<RwLock<Buffer>>,
    cursor: Position,
    /// Selection origin (drag start or shift-move origin). `None` means the
    /// selection is collapsed and the caret is a plain caret.
    anchor: Option<Position>,
    /// Index of the top visible line. Whole lines only — jump scroll, no
    /// smoothing (UI_SPEC.md §3.2).
    scroll: usize,
}

impl Editor {
    /// A new editor over a shared buffer. The cursor starts at `0,0`.
    pub fn new(buffer: Arc<RwLock<Buffer>>) -> Self {
        Self {
            buffer,
            cursor: Position::start(),
            anchor: None,
            scroll: 0,
        }
    }

    /// Replace the entire buffer content (e.g. a file was loaded into this
    /// pane). Resets the cursor and scroll.
    pub fn set_text(&mut self, text: &str) {
        *self.write_buffer() = Buffer::from_text(text);
        self.cursor = Position::start();
        self.anchor = None;
        self.scroll = 0;
    }

    /// The entire buffer text (allocates — fine for save/round-trip; the
    /// editor pane itself reads per-line).
    pub fn text(&self) -> String {
        self.read_buffer().text()
    }

    /// The caret position (zero-based line, code-point column). For the
    /// status bar's line/col readout.
    pub fn cursor(&self) -> Position {
        self.cursor
    }

    // ------------------------------------------------------------------
    // Buffer access
    //
    // A poisoned lock is recovered from rather than panicked on
    // (PLAN.md §8: no `unwrap()` in non-test code): the guard is still
    // usable, it just carries the pre-panic state.
    // ------------------------------------------------------------------

    fn read_buffer(&self) -> RwLockReadGuard<'_, Buffer> {
        self.buffer
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_buffer(&self) -> RwLockWriteGuard<'_, Buffer> {
        self.buffer
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // ------------------------------------------------------------------
    // Selection / cursor state
    // ------------------------------------------------------------------

    /// The ordered selection `(start, end)`, or `None` when collapsed.
    fn selection(&self) -> Option<(Position, Position)> {
        let anchor = self.anchor?;
        if anchor == self.cursor {
            None
        } else {
            Some((anchor.min(self.cursor), anchor.max(self.cursor)))
        }
    }

    /// Move the caret to `target`; with `extend`, keep (or start) a
    /// selection from the old caret; without it, collapse the selection.
    fn move_cursor(&mut self, target: Position, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = target;
    }

    // ------------------------------------------------------------------
    // Text edits
    // ------------------------------------------------------------------

    /// Insert `text` at the caret, replacing the selection if one exists.
    /// The caret lands just after the inserted text. (Used for typed input
    /// and for Enter, which inserts `"\n"`.)
    fn insert_text(&mut self, text: &str) {
        let selection = self.selection();
        let at = selection.map_or(self.cursor, |(start, _)| start);
        let new_cursor = {
            let mut buffer = self.write_buffer();
            if let Some((start, end)) = selection {
                buffer.remove(start, end);
            }
            buffer.insert(at, text);
            buffer.char_to_position(buffer.position_to_char(at) + text.chars().count())
        };
        self.cursor = new_cursor;
        self.anchor = None;
    }

    /// Backspace: remove the selection, or the character before the caret
    /// (joining lines at column 0).
    fn delete_backward(&mut self) {
        let range = self
            .selection()
            .or_else(|| motion::backspace_range(&self.read_buffer(), self.cursor));
        self.apply_delete(range);
    }

    /// Forward delete: remove the selection, or the character after the
    /// caret (joining lines at line end).
    fn delete_forward(&mut self) {
        let range = self
            .selection()
            .or_else(|| motion::delete_range(&self.read_buffer(), self.cursor));
        self.apply_delete(range);
    }

    fn apply_delete(&mut self, range: Option<(Position, Position)>) {
        let Some((start, end)) = range else { return };
        if start == end {
            return;
        }
        self.write_buffer().remove(start, end);
        self.cursor = start;
        self.anchor = None;
    }

    // ------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------

    /// Wheel scroll (jump scroll — the whole-line offset is applied
    /// immediately, no animation). `Ctrl` + wheel is egui's zoom gesture and
    /// is left alone.
    fn handle_wheel(&mut self, ctx: &egui::Context, response: &egui::Response, visible: usize) {
        if !response.hovered() {
            return;
        }
        let delta_lines = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|event| match event {
                    Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                        ..
                    } if !modifiers.ctrl => {
                        Some(layout::wheel_scroll_lines(*unit, *delta, visible))
                    }
                    _ => None,
                })
                .sum::<f32>()
        });
        if delta_lines == 0.0 {
            return;
        }
        let line_count = self.read_buffer().line_count();
        let target = self.scroll as f32 - delta_lines;
        self.scroll = layout::clamp_scroll(target.round().max(0.0) as usize, line_count, visible);
    }

    /// Click to position the caret; drag to select. A click in the gutter
    /// selects the whole line (ARCHITECTURE.md §9.3). Returns whether the
    /// cursor moved (for scroll-into-view).
    fn handle_pointer(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        painter: &Painter,
        font_id: &FontId,
        rect: Rect,
    ) -> bool {
        let shift = ctx.input(|i| i.modifiers.shift);

        if response.dragged_by(PointerButton::Primary) {
            // Extend (or start) a selection from wherever the press landed.
            if let Some(pointer) = response.interact_pointer_pos() {
                if let Some(position) = self.position_from_pointer(pointer, rect, painter, font_id)
                {
                    if self.anchor.is_none() {
                        self.anchor = Some(position);
                    }
                    self.cursor = position;
                    return true;
                }
            }
        } else if response.clicked_by(PointerButton::Primary) {
            if let Some(pointer) = response.interact_pointer_pos() {
                if pointer.x < rect.left() + layout::GUTTER_WIDTH {
                    self.select_line_at(pointer, rect, shift);
                } else if let Some(position) =
                    self.position_from_pointer(pointer, rect, painter, font_id)
                {
                    if !(shift && self.anchor.is_some()) {
                        self.anchor = Some(position);
                    }
                    self.cursor = position;
                }
                return true;
            }
        }
        false
    }

    /// Gutter click: select the line under the pointer (shift-click extends
    /// the existing selection to the end of that line).
    fn select_line_at(&mut self, pointer: Pos2, rect: Rect, shift: bool) {
        let line = {
            let buffer = self.read_buffer();
            (layout::line_index_from_y(pointer.y - rect.top()) + self.scroll)
                .min(buffer.line_count() - 1)
        };
        let line_len = self.read_buffer().line_len(line);
        if !(shift && self.anchor.is_some()) {
            self.anchor = Some(Position::new(line, 0));
        }
        self.cursor = Position::new(line, line_len);
    }

    /// Map a pointer position (screen coordinates) to a buffer position.
    fn position_from_pointer(
        &self,
        pointer: Pos2,
        rect: Rect,
        painter: &Painter,
        font_id: &FontId,
    ) -> Option<Position> {
        let line = {
            let buffer = self.read_buffer();
            (layout::line_index_from_y(pointer.y - rect.top()) + self.scroll)
                .min(buffer.line_count() - 1)
        };
        let line_text = self.read_buffer().line(line)?;
        let visuals = line_visuals(painter, font_id, &line_text);
        let x = pointer.x - (rect.left() + layout::GUTTER_WIDTH);
        Some(Position::new(
            line,
            layout::col_from_x(&visuals.x_offsets, visuals.width, x),
        ))
    }

    /// Keyboard input (only while the editor has focus). Returns whether
    /// the cursor moved or the buffer changed shape.
    fn handle_keyboard(&mut self, ctx: &egui::Context, response: &egui::Response) -> bool {
        if !response.has_focus() {
            return false;
        }
        let events = ctx.input(|i| i.events.clone());
        let mut interacted = false;
        for event in events {
            match event {
                Event::Text(text) => {
                    self.insert_text(&text);
                    interacted = true;
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if self.handle_key(key, modifiers.shift) => {
                    interacted = true;
                }
                _ => {}
            }
        }
        interacted
    }

    /// One keypress. Returns whether the cursor moved or text changed.
    fn handle_key(&mut self, key: Key, extend: bool) -> bool {
        match key {
            Key::ArrowLeft => {
                let target = {
                    let buffer = self.read_buffer();
                    motion::move_left(&buffer, self.cursor)
                };
                self.move_cursor(target, extend);
                true
            }
            Key::ArrowRight => {
                let target = {
                    let buffer = self.read_buffer();
                    motion::move_right(&buffer, self.cursor)
                };
                self.move_cursor(target, extend);
                true
            }
            Key::ArrowUp => {
                let target = {
                    let buffer = self.read_buffer();
                    motion::move_vertically(&buffer, self.cursor, -1)
                };
                self.move_cursor(target, extend);
                true
            }
            Key::ArrowDown => {
                let target = {
                    let buffer = self.read_buffer();
                    motion::move_vertically(&buffer, self.cursor, 1)
                };
                self.move_cursor(target, extend);
                true
            }
            Key::Home => {
                self.move_cursor(motion::line_start(self.cursor), extend);
                true
            }
            Key::End => {
                let target = {
                    let buffer = self.read_buffer();
                    motion::line_end(&buffer, self.cursor)
                };
                self.move_cursor(target, extend);
                true
            }
            Key::Enter => {
                self.insert_text("\n");
                true
            }
            Key::Backspace => {
                self.delete_backward();
                true
            }
            Key::Delete => {
                self.delete_forward();
                true
            }
            Key::Escape => {
                // Collapse the selection. (Ghost-text dismissal arrives with
                // selora-ai in Phase 4.)
                self.anchor = None;
                false
            }
            // Tab is reserved for ghost-text acceptance (Phase 4); left
            // unhandled for now so nothing is inserted silently.
            _ => false,
        }
    }

    // ------------------------------------------------------------------
    // Painting
    // ------------------------------------------------------------------

    fn paint(&self, painter: &Painter, rect: Rect, char_width: f32, focused: bool, time: f64) {
        // Canvas (UI_SPEC.md §3.2: background --ink-canvas, no padding).
        painter.rect_filled(rect, theme::radius::R0, theme::color::INK_CANVAS);

        let text_x = rect.left() + layout::GUTTER_WIDTH;
        let visible = layout::visible_line_count(rect.height());
        let rows = visible.min(self.read_buffer().line_count().saturating_sub(self.scroll));

        // Snapshot the visible lines once, then lay each out as a galley.
        // The galley's glyph positions double as the per-character x offsets
        // used below for selection and caret placement.
        let visuals: Vec<LineVisuals> = {
            let buffer = self.read_buffer();
            (self.scroll..self.scroll + rows)
                .map(|line| {
                    let text = buffer.line(line).unwrap_or_default();
                    line_visuals(painter, &self.font_id(), &text)
                })
                .collect()
        };

        let row_y = |row: usize| rect.top() + (row as f32) * layout::LINE_HEIGHT;
        let cursor_row = self.cursor.line.wrapping_sub(self.scroll);

        // Caret row: one ink-step lift, full canvas width (UI_SPEC.md §3.2).
        if cursor_row < rows {
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(rect.left(), row_y(cursor_row)),
                    vec2(rect.width(), layout::LINE_HEIGHT),
                ),
                theme::radius::R0,
                theme::color::INK_FAINT,
            );
        }

        // Selection: --ink-mid rectangles behind the glyphs.
        if let Some((start, end)) = self.selection() {
            for span in layout::selection_line_ranges(start, end, |line| {
                let buffer = self.read_buffer();
                buffer.line_len(line)
            }) {
                let row = span.line.wrapping_sub(self.scroll);
                if row >= rows {
                    continue;
                }
                let visuals = &visuals[row];
                let x_start = visuals
                    .x_offsets
                    .get(span.col_start)
                    .copied()
                    .unwrap_or(visuals.width);
                // A selection that runs to the line's end (i.e. includes the
                // newline) still shows one character-cell of highlight on
                // empty lines.
                let x_end = visuals
                    .x_offsets
                    .get(span.col_end)
                    .copied()
                    .unwrap_or_else(|| visuals.width.max(char_width));
                if x_end > x_start {
                    painter.rect_filled(
                        Rect::from_min_size(
                            pos2(text_x + x_start, row_y(row)),
                            vec2(x_end - x_start, layout::LINE_HEIGHT),
                        ),
                        theme::radius::R0,
                        theme::color::INK_MID,
                    );
                }
            }
        }

        // Text + gutter.
        let number_font = FontId::new(theme::type_scale::SM, theme::font::FAMILY_MONO);
        for (row, line_visual) in visuals.iter().enumerate() {
            let line = self.scroll + row;
            let y = row_y(row);

            // Galley centered in its line slot (the font's row height is a
            // few points shorter than the 1.5x line height).
            let galley_y = y + (layout::LINE_HEIGHT - line_visual.galley.size().y) / 2.0;
            painter.galley(
                pos2(text_x, galley_y),
                line_visual.galley.clone(),
                theme::color::FG_DEFAULT,
            );

            // Line number, right-aligned in the gutter. The active line's
            // number is --fg-default; the rest are --fg-faint.
            let color = if line == self.cursor.line {
                theme::color::FG_DEFAULT
            } else {
                theme::color::FG_FAINT
            };
            painter.text(
                pos2(
                    rect.left() + layout::GUTTER_WIDTH - layout::GUTTER_RIGHT_PADDING,
                    y + layout::LINE_HEIGHT / 2.0,
                ),
                Align2::RIGHT_CENTER,
                (line + 1).to_string(),
                number_font.clone(),
                color,
            );
        }

        // Active-line indicator: a 1px --ink-rule caret at the gutter's
        // right edge spanning the caret's row (UI_SPEC.md §3.2).
        if cursor_row < rows {
            let y = row_y(cursor_row);
            painter.vline(
                rect.left() + layout::GUTTER_WIDTH,
                Rangef::new(y, y + layout::LINE_HEIGHT),
                Stroke::new(1.0, theme::color::INK_RULE),
            );
        }

        // Caret: 2px, --fg-default, blinking at 500ms when focused
        // (UI_SPEC.md §3.2 — the only blink in the app).
        if cursor_row < rows && layout::cursor_visible(time, focused) {
            let visuals = &visuals[cursor_row];
            let x = visuals
                .x_offsets
                .get(self.cursor.col)
                .copied()
                .unwrap_or(visuals.width);
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(text_x + x, row_y(cursor_row)),
                    vec2(layout::CURSOR_WIDTH, layout::LINE_HEIGHT),
                ),
                theme::radius::R0,
                theme::color::FG_DEFAULT,
            );
        }
    }

    fn font_id(&self) -> FontId {
        FontId::new(layout::FONT_SIZE, theme::font::FAMILY_MONO)
    }
}

impl Widget for &mut Editor {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        // The canvas fills whatever space the parent layout left over —
        // the editor is the flex region of the workbench (UI_SPEC.md §2).
        let response = ui.allocate_response(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let painter = ui.painter_at(rect);

        if response.clicked() {
            response.request_focus();
        }

        let font_id = self.font_id();
        let char_width = painter.fonts_mut(|fonts| fonts.glyph_width(&font_id, '0'));
        let visible = layout::visible_line_count(rect.height());

        // Input first (order per ARCHITECTURE.md §2: gestures before paint,
        // so the frame shows the result of this frame's input).
        self.handle_wheel(ui.ctx(), &response, visible);
        let cursor_moved = self.handle_pointer(ui.ctx(), &response, &painter, &font_id, rect);
        let text_changed = self.handle_keyboard(ui.ctx(), &response);

        if cursor_moved || text_changed {
            self.scroll = layout::ensure_cursor_visible(self.scroll, self.cursor.line, visible);
        }

        // Edits may have shortened the buffer; re-clamp before painting.
        let line_count = self.read_buffer().line_count();
        self.scroll = layout::clamp_scroll(self.scroll, line_count, visible);

        let focused = response.has_focus();
        let time = ui.ctx().input(|i| i.time);
        self.paint(&painter, rect, char_width, focused, time);

        response
    }
}

/// One line's laid-out text plus the geometry derived from it.
struct LineVisuals {
    galley: Arc<Galley>,
    /// Left edge (relative to the text area) of each character on the line.
    x_offsets: Vec<f32>,
    /// Total line width.
    width: f32,
}

/// Lay a line out and derive its per-character x offsets from the galley's
/// glyph positions. egui emits one glyph per `char`, so the offsets line up
/// 1:1 with the buffer's code-point columns.
fn line_visuals(painter: &Painter, font_id: &FontId, text: &str) -> LineVisuals {
    let galley = painter.layout_no_wrap(text.to_owned(), font_id.clone(), theme::color::FG_DEFAULT);
    let mut x_offsets = Vec::with_capacity(text.chars().count());
    let mut width = 0.0;
    if let Some(row) = galley.rows.first() {
        let base = row.pos.x;
        for glyph in &row.glyphs {
            x_offsets.push(base + glyph.pos.x);
        }
        width = row.pos.x + row.size.x;
    }
    LineVisuals {
        galley,
        x_offsets,
        width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str) -> Editor {
        Editor::new(Arc::new(RwLock::new(Buffer::from_text(text))))
    }

    #[test]
    fn text_round_trips_and_set_text_resets_state() {
        let mut editor = editor("hello\nworld");
        editor.set_text("replaced\ncontent\nhere");
        assert_eq!(editor.text(), "replaced\ncontent\nhere");
        assert_eq!(editor.cursor(), Position::start());
        assert!(editor.selection().is_none());
    }

    #[test]
    fn insert_text_at_cursor_and_after_selection() {
        let mut editor = editor("hello world");
        // Typed input at the caret.
        editor.cursor = Position::new(0, 5);
        editor.insert_text(" brave");
        assert_eq!(editor.text(), "hello brave world");
        assert_eq!(editor.cursor(), Position::new(0, 11));

        // Typing with an active selection replaces it.
        editor.anchor = Some(Position::new(0, 6));
        editor.cursor = Position::new(0, 11); // selects "brave"
        editor.insert_text("SELORA");
        assert_eq!(editor.text(), "hello SELORA world");
        assert_eq!(editor.cursor(), Position::new(0, 12));
        assert!(editor.selection().is_none());
    }

    #[test]
    fn enter_splits_the_line() {
        let mut editor = editor("ab");
        editor.cursor = Position::new(0, 1);
        editor.insert_text("\n");
        assert_eq!(editor.text(), "a\nb");
        assert_eq!(editor.cursor(), Position::new(1, 0));
    }

    #[test]
    fn backspace_joins_lines_and_respects_selection() {
        let mut editor = editor("one\ntwo");
        // At the start of line 1, backspace removes the newline.
        editor.cursor = Position::new(1, 0);
        editor.delete_backward();
        assert_eq!(editor.text(), "onetwo");
        assert_eq!(editor.cursor(), Position::new(0, 3));

        // With a selection, backspace removes the selection (not the char).
        editor.anchor = Some(Position::new(0, 1));
        editor.cursor = Position::new(0, 3); // "ne" of "onetwo"
        editor.delete_backward();
        assert_eq!(editor.text(), "otwo");
    }

    #[test]
    fn delete_forward_joins_lines() {
        let mut editor = editor("one\ntwo");
        editor.cursor = Position::new(0, 3);
        editor.delete_forward();
        assert_eq!(editor.text(), "onetwo");
        assert_eq!(editor.cursor(), Position::new(0, 3));
    }

    #[test]
    fn move_cursor_extends_and_collapses_selection() {
        let mut editor = editor("abc\ndef");
        editor.cursor = Position::new(0, 1);
        // Shift-arrow extends from the current caret.
        editor.move_cursor(Position::new(0, 2), true);
        assert_eq!(
            editor.selection(),
            Some((Position::new(0, 1), Position::new(0, 2)))
        );
        // Plain arrow collapses.
        editor.move_cursor(Position::new(1, 0), false);
        assert_eq!(editor.cursor(), Position::new(1, 0));
        assert!(editor.selection().is_none());
    }

    /// The full paint path runs headlessly on egui's test UI — allocation,
    /// galley layout, selection rectangles, gutter, caret. If any of the
    /// painter plumbing is wrong against egui 0.36, this is the test that
    /// catches it without a window.
    #[test]
    fn paint_runs_headlessly() {
        let mut editor = editor("fn theme() -> Theme {\n    Theme::from_tokens(TOKENS)\n}");
        // A multi-line selection so the selection-rect path is exercised too.
        editor.cursor = Position::new(1, 4);
        editor.anchor = Some(Position::new(0, 3));
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            ui.add(&mut editor);
        });
        // State survives the frame (the widget borrows, never resets).
        assert_eq!(editor.cursor(), Position::new(1, 4));
        assert_eq!(
            editor.selection(),
            Some((Position::new(0, 3), Position::new(1, 4)))
        );
    }
}
