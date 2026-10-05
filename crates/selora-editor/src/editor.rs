//! The [`Editor`] widget — the central editor canvas (UI_SPEC.md §3.2).
//!
//! One widget per pane, backed by a shared [`Buffer`] behind an
//! `Arc<RwLock<…>` (ARCHITECTURE.md §3.2: LSP, file IO, and the editor all
//! hold handles to the same buffer). The widget owns only view state:
//! cursor, selection anchor, and the scroll offset. Rendering is done by
//! hand on the [`egui::Painter`] — each visible line is laid out as a
//! per-glyph [`egui::text::LayoutJob`] whose sections carry the SYN_*
//! colors of the active [`crate::syntax::SyntaxLayer`] (Phase 2d), and
//! painted at its row position; the gutter, selection rectangles, caret
//! row, and blinking caret are painted underneath and on top of it.
//!
//! The job's glyph positions double as the per-character x offsets used
//! for selection, caret placement, and click→column mapping — colors and
//! italics don't change glyph advances, so the click math is exactly the
//! math for what is on screen (see
//! `colored_sections_preserve_click_geometry`).
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

use std::path::Path;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use egui::{
    pos2, vec2, Align2, Event, FontId, Galley, Key, Painter, PointerButton, Pos2, Rangef, Rect,
    Sense, Stroke, Widget,
};
use selora_core::{Buffer, Position};

use crate::syntax::{Language, LineSection, Span, SyntaxKind, SyntaxLayer};
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
    /// A line the next paint must scroll into view (set by
    /// [`Editor::goto_line`]; the visible-row count is only known at paint).
    pending_reveal: Option<usize>,
    /// Syntax state for this pane (Phase 2d). `None` = plain text (no
    /// language detected, or the grammar failed to load).
    syntax: Option<SyntaxLayer>,
    /// The frame clock (egui `input.time`), fed to the syntax layer for its
    /// large-file re-parse debounce.
    frame_time: f64,
}

impl Editor {
    /// A new editor over a shared buffer. The cursor starts at `0,0`.
    pub fn new(buffer: Arc<RwLock<Buffer>>) -> Self {
        Self {
            buffer,
            cursor: Position::start(),
            anchor: None,
            scroll: 0,
            pending_reveal: None,
            syntax: None,
            frame_time: 0.0,
        }
    }

    /// Replace the entire buffer content (e.g. a file was loaded into this
    /// pane). Resets the cursor and scroll.
    pub fn set_text(&mut self, text: &str) {
        *self.write_buffer() = Buffer::from_text(text);
        self.cursor = Position::start();
        self.anchor = None;
        self.scroll = 0;
        self.pending_reveal = None;
        if let Some(layer) = &mut self.syntax {
            layer.set_source(text);
        }
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
    // Syntax highlighting (Phase 2d)
    // ------------------------------------------------------------------

    /// Select the highlighting language for this pane. `None` (or a grammar
    /// that fails to load) shows plain `SYN_DEFAULT` text. Call when a file
    /// is opened into the pane, or to override the language by hand.
    pub fn set_language(&mut self, language: Option<Language>) {
        self.syntax = None; // keep a plain pane while (re)building
        if let Some(language) = language {
            if let Some(mut layer) = SyntaxLayer::new(language) {
                layer.set_source(&self.read_buffer().text());
                self.syntax = Some(layer);
            }
        }
    }

    /// Select the highlighting language from a file path's extension
    /// (unknown extensions fall back to plain text).
    pub fn set_language_from_path(&mut self, path: &Path) {
        self.set_language(Language::from_path(path));
    }

    /// The active highlighting language, if any (`None` for plain text).
    pub fn language(&self) -> Option<Language> {
        self.syntax.as_ref().map(SyntaxLayer::language)
    }

    /// The current syntax spans (byte offsets into the buffer text). Empty
    /// when no language is active. Read-only view for tests and future
    /// consumers (Phase 2c symbols read the tree, not these).
    pub fn syntax_spans(&self) -> &[Span] {
        self.syntax.as_ref().map_or(&[], SyntaxLayer::spans)
    }

    /// Force a full re-parse of the buffer text into the syntax layer (e.g.
    /// after an external buffer mutation the editor wasn't told about —
    /// Agent Mode's buffer edits in Phase 5 should call this).
    pub fn resync_syntax(&mut self) {
        if self.syntax.is_some() {
            let text = self.read_buffer().text();
            if let Some(layer) = &mut self.syntax {
                layer.set_source(&text);
            }
        }
    }

    /// Report one text edit to the syntax layer (LSP-style: the text between
    /// `start` and `old_end` is replaced by `new_text`). Must be called
    /// *before* the buffer is mutated, with positions valid for the
    /// pre-mutation document. The layer keeps its tree and spans in step
    /// incrementally.
    fn syntax_edit(&mut self, start: Position, old_end: Position, new_text: &str) {
        if let Some(layer) = &mut self.syntax {
            layer.edit(start, old_end, new_text, self.frame_time);
        }
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

    /// Jump the caret to a **1-based** line (search results, the problems
    /// panel, go-to-definition): collapses the selection, clamps to the
    /// buffer, and asks the next paint to scroll the line into view (the
    /// viewport's visible-row count is only known there). Called on every
    /// pane's editor that shows this buffer.
    pub fn goto_line(&mut self, line_1based: usize) {
        let line_count = self.read_buffer().line_count();
        let line = line_1based
            .saturating_sub(1)
            .min(line_count.saturating_sub(1));
        self.move_cursor(Position::new(line, 0), false);
        self.pending_reveal = Some(line);
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
        // Tell the syntax layer about the edit before mutating the buffer,
        // with positions valid for the current document: a selection
        // replacement is a removal followed by an insert at the same spot.
        if let Some((start, end)) = selection {
            self.syntax_edit(start, end, "");
        }
        self.syntax_edit(at, at, text);
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
        self.syntax_edit(start, end, "");
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
        // The same layout path the paint side uses (colors don't change
        // glyph advances, so this matches what's on screen exactly).
        let sections = self.line_sections(line, &line_text);
        let visuals = line_visuals(painter, font_id, &line_text, &sections);
        let x = pointer.x - (rect.left() + layout::GUTTER_WIDTH);
        Some(Position::new(
            line,
            layout::col_from_x(&visuals.x_offsets, visuals.width, x),
        ))
    }

    /// The syntax sections for one visible line (empty when the pane has no
    /// highlighting language).
    fn line_sections(&self, line: usize, line_text: &str) -> Vec<LineSection> {
        self.syntax
            .as_ref()
            .map_or_else(Vec::new, |layer| layer.line_sections(line, line_text))
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

        // Snapshot the visible lines once, then lay each out as a per-glyph
        // LayoutJob whose sections carry the SYN_* colors (Phase 2d). The
        // galley's glyph positions double as the per-character x offsets
        // used below for selection and caret placement.
        let visuals: Vec<LineVisuals> = {
            let buffer = self.read_buffer();
            (self.scroll..self.scroll + rows)
                .map(|line| {
                    let text = buffer.line(line).unwrap_or_default();
                    let sections = self.line_sections(line, &text);
                    line_visuals(painter, &self.font_id(), &text, &sections)
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
            // few points shorter than the 1.5x line height). Glyph colors
            // come baked into the job; the fallback only covers un-colored
            // glyphs.
            let galley_y = y + (layout::LINE_HEIGHT - line_visual.galley.size().y) / 2.0;
            painter.galley(
                pos2(text_x, galley_y),
                line_visual.galley.clone(),
                theme::color::SYN_DEFAULT,
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
        let time = ui.ctx().input(|i| i.time);
        self.frame_time = time;

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

        // A goto_line request from this frame (or just before): pull the
        // viewport to the revealed line now that `visible` is known.
        if let Some(line) = self.pending_reveal.take() {
            self.scroll = layout::ensure_cursor_visible(self.scroll, line, visible);
        }

        // Keep the syntax layer in step with the buffer and pump its
        // re-parse debounce. The char/line counts are a cheap O(log n)
        // staleness guard against buffer mutations nobody told the editor
        // about (Phase 5 agent edits should call `resync_syntax` — this
        // catches everything that changed the buffer's shape).
        if let Some(mut layer) = self.syntax.take() {
            let (chars, lines) = {
                let buffer = self.read_buffer();
                (buffer.len_chars(), buffer.line_count())
            };
            if layer.char_count() != chars || layer.line_count() != lines {
                layer.set_source(&self.read_buffer().text());
            } else {
                layer.tick(time);
            }
            self.syntax = Some(layer);
        }

        let focused = response.has_focus();
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

/// Lay a line out as a per-glyph [`egui::text::LayoutJob`] (Phase 2d) and
/// derive its per-character x offsets from the galley's glyph positions.
/// egui emits one glyph per `char`, so the offsets line up 1:1 with the
/// buffer's code-point columns.
///
/// Sections split the line into SYN_*-colored runs (DESIGN_TOKENS.md §1.5);
/// comment runs render italic. Color and italics never change glyph
/// advances — epaint applies italics as a mesh-time skew — so the geometry
/// is byte-for-byte the geometry of an unsectioned layout, which is what
/// keeps click→column mapping exact (see
/// `colored_sections_preserve_click_geometry`).
fn line_visuals(
    painter: &Painter,
    font_id: &FontId,
    text: &str,
    sections: &[LineSection],
) -> LineVisuals {
    // Byte offset of each char boundary (plus the end), so sections
    // specified in code-point columns can slice `&str` ranges.
    let mut byte_of: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    byte_of.push(text.len());
    let char_count = byte_of.len() - 1;

    let mut job = egui::text::LayoutJob::default(); // no wrap (max_width = inf)
    let append =
        |job: &mut egui::text::LayoutJob, char_start: usize, char_end: usize, kind: SyntaxKind| {
            let (start, end) = (
                byte_of[char_start.min(char_count)],
                byte_of[char_end.min(char_count)],
            );
            if start >= end {
                return;
            }
            let mut format = egui::text::TextFormat::simple(font_id.clone(), kind.color());
            format.italics = kind.italics();
            job.append(&text[start..end], 0.0, format);
        };

    let mut cursor = 0; // char column up to which the job is filled
    for section in sections {
        if section.char_start > cursor {
            append(&mut job, cursor, section.char_start, SyntaxKind::Default);
        }
        append(&mut job, section.char_start, section.char_end, section.kind);
        cursor = section.char_end.max(cursor);
    }
    if cursor < char_count {
        append(&mut job, cursor, char_count, SyntaxKind::Default);
    }

    let galley = painter.layout_job(job);
    let mut x_offsets = Vec::with_capacity(char_count);
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
    fn goto_line_clamps_collapses_and_flags_a_reveal() {
        let mut editor = editor("one\ntwo\nthree\nfour");
        // A mid-file line: 1-based → 0-based, selection collapsed.
        editor.anchor = Some(Position::new(0, 0));
        editor.goto_line(3);
        assert_eq!(editor.cursor(), Position::new(2, 0));
        assert!(editor.selection().is_none());
        assert_eq!(editor.pending_reveal, Some(2));

        // Beyond the end clamps to the last line; before the start to the
        // first (a 1-based line 0 is really "line 1").
        editor.goto_line(99);
        assert_eq!(editor.cursor(), Position::new(3, 0));
        editor.goto_line(0);
        assert_eq!(editor.cursor(), Position::new(0, 0));

        // And set_text clears any pending reveal with the rest of the view.
        editor.set_text("x");
        assert_eq!(editor.pending_reveal, None);
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

    /// The core risk of the Phase 2d LayoutJob migration: per-section colors
    /// (and italics) must not move a single glyph, or click→column mapping
    /// would drift from what is painted. Colors don't participate in layout
    /// and epaint skews italic glyphs at mesh time, so the x offsets of a
    /// fully-sectioned line must equal those of the plain layout.
    ///
    /// `egui::__run_test_ui` deliberately installs *no* fonts (all galleys
    /// would come out glyph-less), so this drives its own context with the
    /// bundled JetBrains Mono — the font the editor actually paints with.
    #[test]
    fn colored_sections_preserve_click_geometry() {
        let text = "pub fn theme() -> Theme { // note";
        let font_id = FontId::new(layout::FONT_SIZE, theme::font::FAMILY_MONO);
        // Sections splitting the line into colored runs, deliberately
        // misaligned with word boundaries and including an italic comment
        // run at the end.
        let sections = vec![
            LineSection {
                char_start: 0,
                char_end: 6,
                kind: SyntaxKind::Keyword,
            },
            LineSection {
                char_start: 7,
                char_end: 9,
                kind: SyntaxKind::Keyword,
            },
            LineSection {
                char_start: 10,
                char_end: 15,
                kind: SyntaxKind::Function,
            },
            LineSection {
                char_start: 16,
                char_end: 25,
                kind: SyntaxKind::Punctuation,
            },
            LineSection {
                char_start: 26,
                char_end: 31,
                kind: SyntaxKind::Type,
            },
            LineSection {
                char_start: 33,
                char_end: 38,
                kind: SyntaxKind::Comment,
            },
        ];
        run_with_bundled_mono(|ui| {
            let painter = ui.painter();
            let colored = line_visuals(painter, &font_id, text, &sections);
            let plain = line_visuals(painter, &font_id, text, &[]);
            assert_eq!(colored.x_offsets, plain.x_offsets);
            assert_eq!(colored.width, plain.width);
            // The offset count matches the code-point count, so
            // `layout::col_from_x` (the click mapping) sees one entry per
            // column.
            assert_eq!(colored.x_offsets.len(), text.chars().count());
            // A monospace line advances uniformly (within pixel rounding),
            // so the click mapping's cell assumptions hold on real metrics.
            for window in colored.x_offsets.windows(2) {
                let advance = window[1] - window[0];
                assert!(
                    advance > 4.0 && advance < 12.0,
                    "monospace advance: {advance}"
                );
            }
        });
    }

    /// Run a UI closure on a context loaded with the bundled JetBrains Mono
    /// (what the editor paints with), so layout produces real glyphs.
    fn run_with_bundled_mono(add_contents: impl FnMut(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        let font_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts/JetBrainsMono-Regular.ttf");
        let bytes = std::fs::read(font_path).expect("bundled JetBrains Mono font");
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "JetBrainsMono-Regular".to_owned(),
            Arc::new(egui::text::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "JetBrainsMono-Regular".to_owned());
        ctx.set_fonts(fonts);
        ctx.run_ui(egui::RawInput::default(), add_contents)
            .drop_without_applying_deltas();
    }

    /// The full paint path with syntax highlighting active: the LayoutJob
    /// carries colored + italic sections, and the frame still preserves
    /// cursor/selection state.
    #[test]
    fn paint_runs_headlessly_with_syntax() {
        let mut editor = editor("fn theme() -> Theme {\n    Theme::from_tokens(TOKENS) // done\n}");
        editor.set_language(Some(Language::Rust));
        assert_eq!(editor.language(), Some(Language::Rust));
        assert!(!editor.syntax_spans().is_empty());
        editor.cursor = Position::new(1, 4);
        editor.anchor = Some(Position::new(0, 3));
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            ui.add(&mut editor);
        });
        assert_eq!(editor.cursor(), Position::new(1, 4));
        assert_eq!(
            editor.selection(),
            Some((Position::new(0, 3), Position::new(1, 4)))
        );
    }

    /// Typing flows into the syntax layer: the buffer edit is reported with
    /// pre-edit positions, the layer's shadow stays in step, and the spans
    /// reflect the new text.
    #[test]
    fn edits_flow_into_the_syntax_layer() {
        let mut editor = editor("let x = 1;\n");
        editor.set_language(Some(Language::Rust));
        // Typing at the caret (0,0): the layer is notified before the
        // buffer mutation.
        editor.insert_text("// ");
        assert_eq!(editor.text(), "// let x = 1;\n");
        // The layer's spans now cover the whole line as a comment.
        let spans = editor.syntax_spans().to_vec();
        assert!(!spans.is_empty());
        assert_eq!(spans[0].kind, SyntaxKind::Comment);
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].end, "// let x = 1;".len() as u32);

        // A deletion is reported too: remove the "// " again via the same
        // path a Backspace takes.
        editor.cursor = Position::new(0, 0);
        editor.anchor = Some(Position::new(0, 3));
        editor.delete_backward();
        assert_eq!(editor.text(), "let x = 1;\n");
        let spans = editor.syntax_spans().to_vec();
        assert_eq!(spans[0].kind, SyntaxKind::Keyword);
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].end, 3);
    }

    /// The editor selects a language from a path, and unknown extensions
    /// degrade to plain text (no layer, no crash).
    #[test]
    fn language_selection_from_path_and_unknown() {
        let mut editor = editor("fn main() {}");
        assert_eq!(editor.language(), None);
        editor.set_language_from_path(Path::new("crates/src/main.rs"));
        assert_eq!(editor.language(), Some(Language::Rust));
        assert!(!editor.syntax_spans().is_empty());

        editor.set_language_from_path(Path::new("notes.txt"));
        assert_eq!(editor.language(), None);
        assert!(editor.syntax_spans().is_empty());
    }

    /// An external buffer mutation (outside the editor's input) that changes
    /// the buffer's shape is caught by the per-frame staleness guard and
    /// re-synced.
    #[test]
    fn external_buffer_mutation_resyncs_the_layer() {
        let mut editor = editor("let x = 1;\n");
        editor.set_language(Some(Language::Rust));
        // Mutate the shared buffer behind the editor's back.
        *editor.write_buffer() = Buffer::from_text("fn theme() {}\n");
        // One frame is enough for the guard to notice (char count changed).
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            ui.add(&mut editor);
        });
        assert!(!editor.syntax_spans().is_empty());
        // "theme" (a function name) is highlighted — the spans are from the
        // *new* text, not the stale one.
        let spans = editor.syntax_spans().to_vec();
        let text = editor.text();
        let theme = text.find("theme").unwrap() as u32;
        assert!(spans.iter().any(|span| {
            span.kind == SyntaxKind::Function && span.start <= theme && theme < span.end
        }));
    }
}
