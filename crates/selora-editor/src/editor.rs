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
use std::time::Duration;

use egui::{
    pos2, vec2, Align2, Event, FontId, Galley, Key, Painter, PointerButton, Pos2, Rangef, Rect,
    Sense, Stroke, Widget,
};
use selora_core::{Buffer, Position};

use crate::ghost_text::{self, GhostText};
use crate::lsp_overlay::{self, OverlayState};
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
    /// The LSP overlay (Phase 3a): diagnostics to squiggle, document-sync
    /// bookkeeping, and the hover-hold gesture. See `lsp_overlay.rs`.
    lsp: OverlayState,
    /// The ghost-text suggestion (Phase 4d), painted at the caret until Tab
    /// accepts it or any other input dismisses it. See `ghost_text.rs`.
    ghost: Option<ghost_text::GhostText>,
    /// When the last buffer-mutating edit happened, on the egui frame clock
    /// (`input.time`). The shell's 350 ms ghost debounce (ARCHITECTURE.md
    /// §2 stage 2) compares this against the current frame time — see
    /// [`Editor::last_edit_time`].
    last_edit: Option<f64>,
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
            lsp: OverlayState::default(),
            ghost: None,
            last_edit: None,
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
        // The old document's diagnostics no longer apply to the new text;
        // the commit bump makes the shell emit a didChange so the server
        // re-diagnoses (Phase 3a).
        self.lsp.diagnostics.clear();
        self.lsp.commits += 1;
        self.ghost = None; // the suggestion belonged to the replaced text
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
    // LSP overlay (Phase 3a — the editor side of language intelligence;
    // the types and pure logic live in `lsp_overlay.rs`)
    // ------------------------------------------------------------------

    /// Replace this editor's diagnostics (applied from an LSP
    /// `Diagnostics` event for this buffer's path). The next paint
    /// squiggles them (UI_SPEC.md §3.6).
    pub fn set_diagnostics(&mut self, diagnostics: Vec<lsp_overlay::LspDiagnostic>) {
        self.lsp.diagnostics = diagnostics;
    }

    /// The diagnostics currently overlaid (read-only view for tests and the
    /// future problems panel).
    pub fn diagnostics(&self) -> &[lsp_overlay::LspDiagnostic] {
        &self.lsp.diagnostics
    }

    /// First call returns `true` — the signal for the shell to send the
    /// `didOpen` effect on the first paint of a tab (deliverable 4).
    pub fn note_lsp_opened(&mut self) -> bool {
        if self.lsp.opened {
            false
        } else {
            self.lsp.opened = true;
            true
        }
    }

    /// The buffer's commit counter: every buffer-mutating edit (typed text,
    /// deletes, completions, `set_text`) bumps it. The shell compares it
    /// across frames to emit the debounced `didChange` effect.
    pub fn document_version(&self) -> u64 {
        self.lsp.commits
    }

    /// Drain the hover requests raised by the 600 ms hover-hold gesture
    /// (deliverable 6); the shell forwards each as a hover effect.
    pub fn take_lsp_requests(&mut self) -> Vec<Position> {
        std::mem::take(&mut self.lsp.requests)
    }

    /// Where the caret was last painted (screen coordinates), for anchoring
    /// the hover tooltip and the completion popup above the caret.
    pub fn caret_screen_pos(&self) -> Option<Pos2> {
        self.lsp.caret_screen
    }

    /// Apply a completion (Enter/Tab in the popup): the identifier prefix
    /// ending at the caret is replaced by `label`.
    pub fn insert_completion(&mut self, label: &str) {
        let line = self.cursor.line;
        let line_text = self.read_buffer().line(line).unwrap_or_default();
        let start = lsp_overlay::completion_word_start(&line_text, self.cursor.col);
        // Reuse the typed-input path: a selection from the word start to the
        // caret is replaced by the label (syntax layer + undo-friendly edit
        // + caret placement all included).
        self.anchor = Some(Position::new(line, start));
        self.insert_text(label);
    }

    // ------------------------------------------------------------------
    // Ghost text (Phase 4d — the editor side of PLAN.md §6.4; state,
    // the comet, and painting live in `ghost_text.rs`)
    // ------------------------------------------------------------------

    /// Set (or clear) the ghost suggestion: the shell calls this when a
    /// `GhostTextReady` reply lands.
    ///
    /// `request_caret` is the caret position the shell captured when it
    /// *sent* the request (the caret the up-to-caret prefix was taken at).
    /// A reply arriving after the caret moved is stale — the suggestion was
    /// computed for a different document position — and is dropped rather
    /// than painted (same spirit as ARCHITECTURE.md §5.5: stale responses
    /// are dropped silently). A `None` or blank suggestion clears any open
    /// ghost (blank replies mean "nothing to suggest").
    pub fn set_ghost(&mut self, suggestion: Option<String>, request_caret: Position) {
        match suggestion {
            Some(text) if !text.trim().is_empty() && request_caret == self.cursor => {
                self.ghost = Some(GhostText::new(text, request_caret));
            }
            _ => self.ghost = None,
        }
    }

    /// The open ghost suggestion, if any (`None` when dismissed/cleared).
    pub fn ghost(&self) -> Option<&str> {
        self.ghost.as_ref().map(GhostText::suggestion)
    }

    /// Dismiss the ghost without inserting it (Esc, or any input that is
    /// not Tab). Idempotent.
    pub fn dismiss_ghost(&mut self) {
        self.ghost = None;
    }

    /// Accept the ghost (Tab): insert the suggestion at the caret as one
    /// edit and clear it. The buffer has no undo log yet, so this is a
    /// plain insert through the normal edit path — one commit, syntax-layer
    /// notification, and caret placement included (the moment a undo log
    /// lands, the single `insert_text` here becomes one undo step for
    /// free). Returns whether a ghost was open to accept.
    pub fn accept_ghost(&mut self) -> bool {
        let Some(ghost) = self.ghost.take() else {
            return false;
        };
        let suggestion = ghost.into_suggestion();
        self.insert_text(&suggestion);
        true
    }

    /// The up-to-caret prefix a ghost request is made of (PLAN.md §6.4:
    /// the prompt is the text preceding the caret). Everything before the
    /// caret, across lines.
    pub fn text_to_caret(&self) -> String {
        let buffer = self.read_buffer();
        let chars = buffer.position_to_char(self.cursor);
        buffer.text().chars().take(chars).collect()
    }

    /// When the last buffer-mutating edit happened, in seconds on the egui
    /// frame clock (`input.time`); `None` if this pane has never been
    /// edited.
    ///
    /// This is the idle-detection seam for the shell's 350 ms ghost
    /// debounce (ARCHITECTURE.md §2 stage 2 — the timer itself belongs to
    /// the shell, the editor only reports when it last saw edit input):
    ///
    /// ```text
    /// let now = ui.ctx().input(|i| i.time);
    /// let idle = editor.last_edit_time().is_none_or(|t| now - t >= 0.350);
    /// ```
    ///
    /// The shell then captures `editor.cursor()`, emits the ghost request
    /// with `editor.text_to_caret()`, and routes the reply back through
    /// [`Editor::set_ghost`] with that captured caret.
    pub fn last_edit_time(&self) -> Option<f64> {
        self.last_edit
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
        // One buffer-mutating commit (Phase 3a: the shell watches this to
        // emit the debounced LSP didChange) and one edit-input timestamp
        // (Phase 4d: the shell's 350 ms ghost debounce idles on this).
        self.lsp.commits += 1;
        self.last_edit = Some(self.frame_time);
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
        self.lsp.commits += 1;
        self.last_edit = Some(self.frame_time);
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
                // Any input that is not Tab dismisses the ghost (Phase 4d):
                // a click that moves the caret invalidates the suggestion's
                // position even though the buffer is untouched.
                self.ghost = None;
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
                    // Typed input is edit input: it dismisses the ghost
                    // (Phase 4d — any keypress other than Tab).
                    self.ghost = None;
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
        // Tab accepts an open ghost; every other keypress dismisses it
        // (Phase 4d, UI_SPEC.md §3.5) *before* its normal handling runs,
        // so the dismiss happens even for keys that are otherwise no-ops.
        if key != Key::Tab {
            self.ghost = None;
        }
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
                // Collapse the selection; an open ghost was already
                // dismissed by the any-other-key rule above.
                self.anchor = None;
                false
            }
            // Tab accepts the ghost (Phase 4d, UI_SPEC.md §15); without a
            // ghost it stays unhandled so nothing is inserted silently.
            Key::Tab if self.ghost.is_some() => {
                self.accept_ghost();
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------------
    // Painting
    // ------------------------------------------------------------------

    fn paint(&mut self, painter: &Painter, rect: Rect, char_width: f32, focused: bool, time: f64) {
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
            // Remember where the caret was painted — the hover tooltip and
            // the completion popup anchor here (Phase 3a).
            self.lsp.caret_screen = Some(pos2(text_x + x, row_y(cursor_row)));
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(text_x + x, row_y(cursor_row)),
                    vec2(layout::CURSOR_WIDTH, layout::LINE_HEIGHT),
                ),
                theme::radius::R0,
                theme::color::FG_DEFAULT,
            );
        } else {
            self.lsp.caret_screen = None;
        }

        // Ghost text (Phase 4d, UI_SPEC.md §3.5): the suggestion paints at
        // the caret in --fg-faint with a 2px --ink-rule underline —
        // paint-only, the buffer is untouched until Tab. The comet clock is
        // driven from this paint loop (the frame time), so a ghost whose
        // caret row is scrolled off-screen keeps its first appearance
        // un-animated for when it scrolls back in.
        if cursor_row < rows {
            let caret_x = visuals[cursor_row]
                .x_offsets
                .get(self.cursor.col)
                .copied()
                .unwrap_or(visuals[cursor_row].width);
            let font_id = self.font_id();
            if let Some(ghost) = self.ghost.as_mut() {
                ghost_text::paint_ghost(
                    painter,
                    ghost,
                    time,
                    ghost_text::GhostGeometry {
                        caret_x: text_x + caret_x,
                        line_start_x: text_x,
                        row_top: row_y(cursor_row),
                        rows_below: rows - cursor_row - 1,
                    },
                    &font_id,
                );
            }
        }

        // Diagnostic squiggles (UI_SPEC.md §3.6): a 3px wave under each
        // diagnostic's range, at least one character cell wide (servers
        // report zero-width ranges at the error position). Multi-line
        // diagnostics underline each visible line of the range.
        for diagnostic in &self.lsp.diagnostics {
            let color = diagnostic.severity.color();
            let start_line = diagnostic.range.start.line as usize;
            let end_line = diagnostic.range.end.line as usize;
            for line in start_line..=end_line {
                let row = line.wrapping_sub(self.scroll);
                if row >= rows {
                    continue; // also skips lines "before" the viewport (wrapping)
                }
                let Some(line_text) = self.read_buffer().line(line) else {
                    continue;
                };
                let line_chars = line_text.chars().count();
                let col_start = if line == start_line {
                    lsp_overlay::utf16_col_to_char(&line_text, diagnostic.range.start.character)
                        .min(line_chars)
                } else {
                    0
                };
                let col_end = if line == end_line {
                    lsp_overlay::utf16_col_to_char(&line_text, diagnostic.range.end.character)
                        .min(line_chars)
                        .max(col_start)
                } else {
                    line_chars
                };
                let visuals = &visuals[row];
                let x_start = visuals
                    .x_offsets
                    .get(col_start)
                    .copied()
                    .unwrap_or(visuals.width);
                let x_end = visuals
                    .x_offsets
                    .get(col_end)
                    .copied()
                    .unwrap_or_else(|| visuals.width.max(char_width));
                lsp_overlay::paint_squiggle(
                    painter,
                    text_x + x_start,
                    text_x + x_end.max(x_start + char_width),
                    row_y(row) + layout::LINE_HEIGHT - 2.0,
                    color,
                );
            }
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

        // While focused, keep the keyboard: egui's focus traversal would
        // otherwise steal Tab / arrows / Esc and move focus away — but the
        // editor handles all of these itself (Tab accepts the ghost, arrows
        // move the caret, Esc collapses the selection). The same lock egui's
        // own TextEdit and Slider widgets take.
        if response.has_focus() {
            ui.ctx().memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
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
        // The LSP hover gesture (Phase 3a): a 600 ms hold over one buffer
        // position raises a hover request for the shell to forward. The
        // repaint request keeps the frame clock running while the pointer
        // rests (egui otherwise sleeps between frames).
        let hover_position = if response.hovered() && !response.dragged() {
            ui.ctx()
                .pointer_hover_pos()
                .and_then(|pointer| self.position_from_pointer(pointer, rect, &painter, &font_id))
        } else {
            None
        };
        if response.hovered() {
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        }
        if let Some(position) = self.lsp.hover_gesture.tick(
            response.hovered(),
            response.dragged(),
            hover_position,
            time,
        ) {
            self.lsp.requests.push(position);
        }

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

        // The comet needs frames while it runs (Phase 4d): egui otherwise
        // sleeps between input and the fade-in would freeze mid-flight.
        // Same pattern as the caret blink and hover-hold repaints above.
        if let Some(until_settled) = self
            .ghost
            .as_ref()
            .and_then(|ghost| ghost.needs_repaint(time))
        {
            ui.ctx().request_repaint_after(until_settled);
        }

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

    // --- LSP overlay (Phase 3a — see lsp_overlay.rs) --------------------------

    fn lsp_diagnostic(
        line: u32,
        start: u32,
        end: u32,
        severity: lsp_overlay::LspSeverity,
    ) -> lsp_overlay::LspDiagnostic {
        lsp_overlay::LspDiagnostic {
            range: lsp_overlay::LspRange {
                start: lsp_overlay::LspPosition {
                    line,
                    character: start,
                },
                end: lsp_overlay::LspPosition {
                    line,
                    character: end,
                },
            },
            severity,
            source: Some("rust-analyzer".into()),
            message: "unclosed delimiter".into(),
        }
    }

    /// The full squiggle paint path runs headlessly: diagnostics on visible
    /// lines (single-line, another line, zero-width, and below the viewport)
    /// paint without panicking, and the state survives the frame.
    #[test]
    fn squiggles_paint_headlessly_for_all_range_shapes() {
        let mut editor = editor("fn broken( {\n    Vec::new()\n}\n");
        editor.set_diagnostics(vec![
            lsp_diagnostic(0, 3, 11, lsp_overlay::LspSeverity::Error),
            lsp_diagnostic(1, 4, 7, lsp_overlay::LspSeverity::Warning),
            lsp_diagnostic(0, 4, 4, lsp_overlay::LspSeverity::Information),
            lsp_diagnostic(9, 0, 2, lsp_overlay::LspSeverity::Error),
        ]);
        assert_eq!(editor.diagnostics().len(), 4);
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            ui.add(&mut editor);
        });
        assert_eq!(editor.diagnostics().len(), 4);
    }

    #[test]
    fn note_lsp_opened_fires_exactly_once() {
        let mut editor = editor("fn main() {}");
        assert!(
            editor.note_lsp_opened(),
            "the first paint opens the document"
        );
        assert!(!editor.note_lsp_opened());
        assert!(!editor.note_lsp_opened());
    }

    #[test]
    fn document_version_bumps_on_every_commit_kind() {
        let mut editor = editor("let x = 1;\n");
        let before = editor.document_version();

        editor.cursor = Position::new(0, 10);
        editor.insert_text(" + 1"); // typed input
        assert_eq!(editor.document_version(), before + 1);

        editor.cursor = Position::new(0, 0);
        editor.delete_forward(); // delete "l"
        assert_eq!(editor.document_version(), before + 2);

        editor.set_text("fn a() {}"); // wholesale replace
        assert_eq!(editor.document_version(), before + 3);

        // A pure read (no edit) does not bump.
        let _ = editor.text();
        assert_eq!(editor.document_version(), before + 3);
    }

    #[test]
    fn insert_completion_replaces_the_typed_prefix() {
        let mut editor = editor("    let v: Vec<u8> = ve");
        editor.cursor = Position::new(0, 23); // just after "ve"
        editor.insert_completion("vec![]");
        assert_eq!(editor.text(), "    let v: Vec<u8> = vec![]");
        // The caret lands after the inserted label.
        assert_eq!(editor.cursor(), Position::new(0, 27));
        // And it counts as a commit (the shell re-syncs the server).
        assert_eq!(editor.document_version(), 1);
    }

    #[test]
    fn caret_screen_position_is_reported_after_paint() {
        let mut editor = editor("fn a() {}");
        assert!(editor.caret_screen_pos().is_none(), "nothing painted yet");
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            ui.add(&mut editor);
        });
        let caret = editor.caret_screen_pos().expect("caret painted");
        // The caret sits in the text area (past the 44px gutter), row 0.
        assert!(caret.x >= 44.0, "caret x: {}", caret.x);
        assert!(caret.y >= 0.0 && caret.y < 48.0, "caret y: {}", caret.y);
    }

    #[test]
    fn take_lsp_requests_drains() {
        let mut editor = editor("fn a() {}");
        assert!(editor.take_lsp_requests().is_empty());
        // The gesture pushes requests (the gesture itself is unit-tested in
        // lsp_overlay).
        editor.lsp.requests.push(Position::new(0, 3));
        assert_eq!(editor.take_lsp_requests(), vec![Position::new(0, 3)]);
        assert!(editor.take_lsp_requests().is_empty(), "drained");
    }

    /// The squiggle overlay and the syntax layer coexist: a frame with
    /// highlighting, a selection, and diagnostics paints cleanly.
    #[test]
    fn paint_runs_headlessly_with_syntax_and_diagnostics() {
        let mut editor = editor("fn theme() {\n    Vec::new()\n} // done\n");
        editor.set_language(Some(Language::Rust));
        editor.set_diagnostics(vec![lsp_diagnostic(
            1,
            4,
            7,
            lsp_overlay::LspSeverity::Error,
        )]);
        editor.cursor = Position::new(1, 4);
        editor.anchor = Some(Position::new(0, 3));
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            ui.add(&mut editor);
        });
        assert_eq!(editor.diagnostics().len(), 1);
        assert!(editor.caret_screen_pos().is_some());
    }

    // --- Ghost text (Phase 4d — see ghost_text.rs) ----------------------------

    /// The ghost is paint-only: a full frame with a multi-line ghost paints
    /// headlessly and leaves the buffer, the commit counter, and the ghost
    /// itself untouched.
    #[test]
    fn ghost_paints_headlessly_without_touching_the_buffer() {
        let mut editor = editor("fn theme() {\n");
        editor.cursor = Position::new(1, 0);
        editor.set_ghost(
            Some("    Theme::from_tokens(TOKENS)\n}".into()),
            Position::new(1, 0),
        );
        assert_eq!(editor.ghost(), Some("    Theme::from_tokens(TOKENS)\n}"));
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            ui.add(&mut editor);
        });
        // Nothing about the buffer moved.
        assert_eq!(editor.text(), "fn theme() {\n");
        assert_eq!(editor.document_version(), 0);
        assert_eq!(editor.cursor(), Position::new(1, 0));
        // And the ghost survived the frame (the widget borrows, never resets).
        assert_eq!(editor.ghost(), Some("    Theme::from_tokens(TOKENS)\n}"));
    }

    /// Tab accepts: the suggestion is inserted at the caret as one edit and
    /// the ghost clears — one commit, caret after the text.
    #[test]
    fn tab_accepts_the_ghost_into_the_buffer() {
        let mut editor = editor("fn main()");
        editor.cursor = Position::new(0, 9);
        editor.set_ghost(Some(" {}".into()), Position::new(0, 9));
        assert!(editor.handle_key(Key::Tab, false));
        assert_eq!(editor.text(), "fn main() {}");
        assert_eq!(editor.cursor(), Position::new(0, 12));
        assert_eq!(editor.ghost(), None);
        assert_eq!(editor.document_version(), 1);
        // Accepting with no ghost open is a no-op (Tab falls through
        // unhandled, nothing inserts silently).
        assert!(!editor.handle_key(Key::Tab, false));
        assert_eq!(editor.text(), "fn main() {}");
    }

    /// Any other keypress dismisses the ghost — editing keys, motion keys,
    /// and keys the editor does not even handle.
    #[test]
    fn non_tab_keys_dismiss_the_ghost() {
        let mut editor = editor("let x = 1;");
        editor.cursor = Position::new(0, 10);
        editor.set_ghost(Some(" // done".into()), Position::new(0, 10));

        // An editing key: dismissed AND handled.
        assert!(editor.handle_key(Key::Enter, false));
        assert_eq!(editor.ghost(), None);
        assert_eq!(editor.text(), "let x = 1;\n");

        // A motion key: dismissed, caret moved, buffer unchanged.
        editor.set_ghost(Some(" again".into()), editor.cursor());
        assert!(editor.handle_key(Key::ArrowLeft, false));
        assert_eq!(editor.ghost(), None);
        assert_eq!(editor.text(), "let x = 1;\n");

        // A key the editor does not handle at all: still dismissed.
        editor.set_ghost(Some(" once more".into()), editor.cursor());
        assert!(!editor.handle_key(Key::PageDown, false));
        assert_eq!(editor.ghost(), None);
    }

    /// Esc dismisses the ghost (UI_SPEC.md §15) and still collapses the
    /// selection, as it did before Phase 4d.
    #[test]
    fn escape_dismisses_the_ghost_and_collapses_the_selection() {
        let mut editor = editor("select me");
        editor.cursor = Position::new(0, 9);
        editor.set_ghost(Some(" now".into()), Position::new(0, 9));
        editor.anchor = Some(Position::new(0, 0));
        assert!(!editor.handle_key(Key::Escape, false));
        assert_eq!(editor.ghost(), None);
        assert!(editor.selection().is_none());
        assert_eq!(editor.text(), "select me", "Esc never edits the buffer");
    }

    /// A reply for a caret that has since moved is stale and is dropped —
    /// never painted, and any previously open ghost is cleared too.
    #[test]
    fn stale_ghost_reply_is_dropped() {
        let mut editor = editor("fn a() {}");
        editor.cursor = Position::new(0, 8);
        editor.set_ghost(Some(" x".into()), Position::new(0, 8));
        assert_eq!(editor.ghost(), Some(" x"));
        // The caret moves (arrow) — the old ghost was dismissed by the key;
        // a reply for the old position now lands.
        editor.handle_key(Key::ArrowLeft, false);
        editor.set_ghost(Some(" stale".into()), Position::new(0, 8));
        assert_eq!(editor.ghost(), None, "stale reply dropped");
        // A fresh reply for the current caret is kept.
        editor.set_ghost(Some(" fresh".into()), editor.cursor());
        assert_eq!(editor.ghost(), Some(" fresh"));
    }

    /// `None` and blank replies clear an open ghost (a blank reply is the
    /// provider saying "nothing to suggest").
    #[test]
    fn none_and_blank_ghost_replies_clear() {
        let mut editor = editor("fn a() {}");
        editor.cursor = Position::new(0, 8);
        editor.set_ghost(Some(" x".into()), Position::new(0, 8));
        editor.set_ghost(None, Position::new(0, 8));
        assert_eq!(editor.ghost(), None);
        editor.set_ghost(Some(" x".into()), Position::new(0, 8));
        editor.set_ghost(Some("   ".into()), Position::new(0, 8));
        assert_eq!(editor.ghost(), None);
        editor.set_ghost(Some(" x".into()), Position::new(0, 8));
        editor.set_ghost(Some(String::new()), Position::new(0, 8));
        assert_eq!(editor.ghost(), None);
    }

    /// The full input path through real frames: click to focus (which
    /// dismisses, per the any-input rule), type (dismisses + edits), then
    /// a ghost arrives and Tab accepts it through the focused widget.
    #[test]
    fn ghost_input_flows_through_real_frames() {
        let ctx = egui::Context::default();
        let mut editor = editor("let v = ");
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 480.0));
        let raw = |events: Vec<Event>| egui::RawInput {
            events,
            screen_rect: Some(screen),
            ..Default::default()
        };

        // Frame 1: the pane takes focus (a click does this in the real app;
        // requesting it directly keeps the test off pointer-hit-testing).
        ctx.run_ui(raw(vec![]), |ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            ui.add(&mut editor).request_focus();
        })
        .drop_without_applying_deltas();

        // Frame 2: focused typing inserts at the caret and would dismiss
        // any ghost.
        editor.cursor = Position::new(0, 8); // "let v = |"
        ctx.run_ui(raw(vec![Event::Text("1".into())]), |ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            ui.add(&mut editor);
        })
        .drop_without_applying_deltas();
        assert_eq!(editor.text(), "let v = 1");

        // The reply lands for the current caret; Tab in the next frame
        // accepts it (the widget is still focused from frame 1).
        editor.set_ghost(Some("; // done".into()), editor.cursor());
        ctx.run_ui(
            raw(vec![Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]),
            |ui| {
                ui.set_max_size(vec2(640.0, 480.0));
                ui.add(&mut editor);
            },
        )
        .drop_without_applying_deltas();
        assert_eq!(editor.text(), "let v = 1; // done");
        assert_eq!(editor.ghost(), None);
    }

    /// The idle seam: every buffer-mutating edit stamps the frame clock;
    /// reads never do.
    #[test]
    fn last_edit_time_tracks_edits_not_reads() {
        let mut editor = editor("let x = 1;\n");
        assert_eq!(editor.last_edit_time(), None, "never edited");
        editor.frame_time = 5.0;
        editor.cursor = Position::new(0, 10);
        editor.insert_text(" + 1");
        assert_eq!(editor.last_edit_time(), Some(5.0));
        editor.delete_backward();
        assert_eq!(editor.last_edit_time(), Some(5.0));
        // A read is not an edit.
        let _ = editor.text();
        assert_eq!(editor.last_edit_time(), Some(5.0));
    }

    /// The request prefix: everything before the caret, across lines.
    #[test]
    fn text_to_caret_returns_the_prefix() {
        let mut editor = editor("fn theme() {\n    Theme::from_");
        editor.cursor = Position::new(1, 17);
        assert_eq!(editor.text_to_caret(), "fn theme() {\n    Theme::from_");
        editor.cursor = Position::start();
        assert_eq!(editor.text_to_caret(), "");
    }
}
