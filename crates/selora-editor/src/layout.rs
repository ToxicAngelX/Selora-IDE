//! Pure layout geometry for the editor canvas (UI_SPEC.md §3.2).
//!
//! Everything here is arithmetic on plain numbers — no widget state, no
//! buffer lock — so the click-to-position mapping, selection extents, and
//! scroll rules are testable headlessly. The widget layer feeds these
//! functions per-line glyph offsets measured from the egui text layout.

use egui::{MouseWheelUnit, Vec2};
use selora_core::Position;

/// Gutter width (UI_SPEC.md §3.2: 44px, right-aligned line numbers).
pub const GUTTER_WIDTH: f32 = 44.0;
/// Padding between the line numbers and the gutter's right edge (4px).
pub const GUTTER_RIGHT_PADDING: f32 = 4.0;
/// Editor font size in points. JetBrains Mono at 14px (Phase 1 brief).
pub const FONT_SIZE: f32 = 14.0;
/// Line height = editor font size x 1.5 (UI_SPEC.md §3.2).
pub const LINE_HEIGHT: f32 = FONT_SIZE * 1.5;
/// Caret width in px (UI_SPEC.md §3.2: 2px).
pub const CURSOR_WIDTH: f32 = 2.0;
/// Caret blink half-period in seconds (UI_SPEC.md §3.2: 500ms blink).
pub const BLINK_ON: f64 = 0.5;
pub const BLINK_OFF: f64 = 0.5;

/// How many whole lines fit in `height` pixels (at least one, so a tiny
/// pane still shows the line the cursor is on).
pub fn visible_line_count(height: f32) -> usize {
    ((height / LINE_HEIGHT).floor().max(1.0)) as usize
}

/// The visible line index for a y coordinate relative to the canvas top.
/// Values beyond the bottom clamp to the last row of the viewport.
pub fn line_index_from_y(y: f32) -> usize {
    ((y / LINE_HEIGHT).floor().max(0.0)) as usize
}

/// The column for an x coordinate relative to the text-area left edge,
/// given the per-character left edges (`x_offsets`, one entry per char) and
/// the line's total width. Picks the column whose character midpoint the
/// pointer is over; beyond the line end yields the end-of-line column.
pub fn col_from_x(x_offsets: &[f32], line_width: f32, x: f32) -> usize {
    for (i, &offset) in x_offsets.iter().enumerate() {
        let next = x_offsets.get(i + 1).copied().unwrap_or(line_width);
        if x < (offset + next) / 2.0 {
            return i;
        }
    }
    x_offsets.len()
}

/// A selection's extent on a single line: columns `[col_start, col_end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    pub line: usize,
    pub col_start: usize,
    pub col_end: usize,
}

/// The per-line column spans covered by the selection between `anchor` and
/// `cursor` (order-insensitive — a backward drag produces the same spans as
/// a forward one). `line_len` supplies each line's length. A collapsed
/// selection (anchor == cursor) yields no spans.
pub fn selection_line_ranges(
    anchor: Position,
    cursor: Position,
    line_len: impl Fn(usize) -> usize,
) -> Vec<LineSpan> {
    let (start, end) = if anchor <= cursor {
        (anchor, cursor)
    } else {
        (cursor, anchor)
    };
    if start == end {
        return Vec::new();
    }
    let mut spans = Vec::with_capacity(end.line - start.line + 1);
    for line in start.line..=end.line {
        let len = line_len(line);
        let col_start = if line == start.line { start.col } else { 0 };
        let col_end = if line == end.line { end.col } else { len };
        spans.push(LineSpan {
            line,
            col_start: col_start.min(len),
            col_end: col_end.min(len),
        });
    }
    spans
}

/// Clamp a scroll offset (index of the top visible line) so the viewport
/// never scrolls past the end of the buffer.
pub fn clamp_scroll(scroll: usize, line_count: usize, visible: usize) -> usize {
    let visible = visible.min(line_count);
    scroll.min(line_count - visible)
}

/// Adjust `scroll` so `cursor_line` is inside the viewport; unchanged when
/// it already is. Called after any cursor-affecting input.
pub fn ensure_cursor_visible(scroll: usize, cursor_line: usize, visible: usize) -> usize {
    if cursor_line < scroll {
        cursor_line
    } else if cursor_line >= scroll + visible {
        cursor_line + 1 - visible
    } else {
        scroll
    }
}

/// Convert a mouse-wheel delta into lines to scroll (positive = content
/// moves down, i.e. scroll up). UI_SPEC.md §3.2: jump scroll, no smoothing —
/// the caller rounds and applies whole lines.
pub fn wheel_scroll_lines(unit: MouseWheelUnit, delta: Vec2, visible: usize) -> f32 {
    match unit {
        MouseWheelUnit::Line => delta.y,
        MouseWheelUnit::Page => delta.y * visible.max(1) as f32,
        MouseWheelUnit::Point => delta.y / LINE_HEIGHT,
    }
}

/// Whether the caret is drawn this frame: steady when the editor is not
/// focused, blinking on a 500ms on/off square wave when it is (UI_SPEC.md
/// §3.2 — the only blink in the app).
pub fn cursor_visible(time_seconds: f64, focused: bool) -> bool {
    if !focused {
        return true;
    }
    let phase = time_seconds % (BLINK_ON + BLINK_OFF);
    phase < BLINK_ON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_from_x_maps_midpoints() {
        // Four chars, 10px wide each, line 40px wide.
        let offsets = [0.0, 10.0, 20.0, 30.0];
        let width = 40.0;
        assert_eq!(col_from_x(&offsets, width, -5.0), 0);
        assert_eq!(col_from_x(&offsets, width, 4.0), 0); // left of char 0's midpoint
        assert_eq!(col_from_x(&offsets, width, 5.5), 1); // past the boundary
        assert_eq!(col_from_x(&offsets, width, 34.0), 3);
        assert_eq!(col_from_x(&offsets, width, 35.1), 4); // past the last midpoint -> eol
        assert_eq!(col_from_x(&offsets, width, 100.0), 4);
    }

    #[test]
    fn col_from_x_on_empty_line_is_zero() {
        assert_eq!(col_from_x(&[], 0.0, 37.0), 0);
    }

    #[test]
    fn selection_forward_and_backward_spans_match() {
        let lens = |line: usize| if line == 1 { 8 } else { 4 };
        let anchor = Position::new(0, 2);
        let cursor = Position::new(2, 1);
        let forward = selection_line_ranges(anchor, cursor, lens);
        let backward = selection_line_ranges(cursor, anchor, lens);
        assert_eq!(forward, backward);
        assert_eq!(
            forward,
            vec![
                LineSpan {
                    line: 0,
                    col_start: 2,
                    col_end: 4
                },
                LineSpan {
                    line: 1,
                    col_start: 0,
                    col_end: 8
                },
                LineSpan {
                    line: 2,
                    col_start: 0,
                    col_end: 1
                },
            ]
        );
    }

    #[test]
    fn selection_single_line_and_collapsed() {
        let lens = |_: usize| 10;
        assert_eq!(
            selection_line_ranges(Position::new(0, 3), Position::new(0, 7), lens),
            vec![LineSpan {
                line: 0,
                col_start: 3,
                col_end: 7
            }]
        );
        // Collapsed selection covers nothing.
        assert!(selection_line_ranges(Position::new(0, 3), Position::new(0, 3), lens).is_empty());
    }

    #[test]
    fn selection_clamps_cols_past_line_end() {
        // Cursor beyond the line's length (stale state) clamps instead of panicking.
        let lens = |_: usize| 4;
        assert_eq!(
            selection_line_ranges(Position::new(0, 99), Position::new(1, 2), lens),
            vec![
                LineSpan {
                    line: 0,
                    col_start: 4,
                    col_end: 4
                },
                LineSpan {
                    line: 1,
                    col_start: 0,
                    col_end: 2
                },
            ]
        );
    }

    #[test]
    fn scroll_never_leaves_the_buffer() {
        // 10 lines, 4 visible -> max top line is 6.
        assert_eq!(clamp_scroll(99, 10, 4), 6);
        assert_eq!(clamp_scroll(2, 10, 4), 2);
        // Buffer smaller than the viewport: pinned to the top.
        assert_eq!(clamp_scroll(5, 3, 8), 0);
        // Single-line buffer.
        assert_eq!(clamp_scroll(4, 1, 20), 0);
    }

    #[test]
    fn ensure_cursor_visible_pulls_viewport_to_cursor() {
        // Cursor above the viewport: scroll up to it.
        assert_eq!(ensure_cursor_visible(10, 3, 5), 3);
        // Cursor below the viewport: bottom-align.
        assert_eq!(ensure_cursor_visible(0, 7, 5), 3);
        // Cursor inside: untouched.
        assert_eq!(ensure_cursor_visible(2, 4, 5), 2);
    }

    #[test]
    fn wheel_units_convert_to_lines() {
        assert_eq!(
            wheel_scroll_lines(MouseWheelUnit::Line, Vec2::new(0.0, -3.0), 10),
            -3.0
        );
        assert_eq!(
            wheel_scroll_lines(MouseWheelUnit::Point, Vec2::new(0.0, -LINE_HEIGHT), 10),
            -1.0
        );
        assert_eq!(
            wheel_scroll_lines(MouseWheelUnit::Page, Vec2::new(0.0, -2.0), 30),
            -60.0
        );
    }

    #[test]
    fn cursor_blinks_when_focused_and_is_steady_when_not() {
        assert!(cursor_visible(0.0, false));
        assert!(cursor_visible(123.4, false));
        // On for the first half-second of each 1s cycle.
        assert!(cursor_visible(0.1, true));
        assert!(cursor_visible(1.1, true));
        // Off for the second half.
        assert!(!cursor_visible(0.6, true));
        assert!(!cursor_visible(3.75, true));
    }

    #[test]
    fn visible_line_count_fits_whole_lines_with_minimum_one() {
        assert_eq!(visible_line_count(LINE_HEIGHT * 10.0), 10);
        assert_eq!(visible_line_count(LINE_HEIGHT * 10.5), 10);
        assert_eq!(visible_line_count(1.0), 1);
    }

    #[test]
    fn line_index_from_y_clamps() {
        assert_eq!(line_index_from_y(-4.0), 0);
        assert_eq!(line_index_from_y(0.0), 0);
        assert_eq!(line_index_from_y(LINE_HEIGHT * 2.5), 2);
    }
}
