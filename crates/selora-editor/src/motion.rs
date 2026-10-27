//! Pure cursor-motion rules (UI_SPEC.md §3.2 keyboard model, Lite subset).
//!
//! Every function here takes a [`Buffer`] by reference and returns new
//! positions — no egui, no state, no allocation beyond the returned
//! [`Position`]. That is what makes the editor's movement rules testable
//! headlessly: the widget layer only chooses *which* rule to apply.

use selora_core::{Buffer, Position};

/// One character left; wraps to the end of the previous line at column 0.
/// Stops (returns the input unchanged) at the buffer start.
pub fn move_left(buffer: &Buffer, pos: Position) -> Position {
    if pos.col > 0 {
        Position::new(pos.line, pos.col - 1)
    } else if pos.line > 0 {
        let prev = pos.line - 1;
        Position::new(prev, buffer.line_len(prev))
    } else {
        pos
    }
}

/// One character right; wraps to the start of the next line at line end.
/// Stops (returns the input unchanged) at the buffer end.
pub fn move_right(buffer: &Buffer, pos: Position) -> Position {
    if pos.col < buffer.line_len(pos.line) {
        Position::new(pos.line, pos.col + 1)
    } else if pos.line + 1 < buffer.line_count() {
        Position::new(pos.line + 1, 0)
    } else {
        pos
    }
}

/// `delta` lines down (positive) or up (negative). The column is kept and
/// clamped to the target line's length; the line is clamped to the buffer.
pub fn move_vertically(buffer: &Buffer, pos: Position, delta: i32) -> Position {
    let line = if delta < 0 {
        pos.line.saturating_sub(delta.unsigned_abs() as usize)
    } else {
        pos.line.saturating_add(delta as usize)
    };
    let line = line.min(buffer.line_count() - 1);
    Position::new(line, pos.col.min(buffer.line_len(line)))
}

/// Column 0 of the same line (`Home`).
pub fn line_start(pos: Position) -> Position {
    Position::new(pos.line, 0)
}

/// End of the same line (`End`).
pub fn line_end(buffer: &Buffer, pos: Position) -> Position {
    Position::new(pos.line, buffer.line_len(pos.line))
}

/// The character range a Backspace at `pos` would remove, or `None` at the
/// buffer start. At column 0 the range spans the previous line's terminator
/// (i.e. Backspace joins lines).
pub fn backspace_range(buffer: &Buffer, pos: Position) -> Option<(Position, Position)> {
    if pos.col > 0 {
        Some((Position::new(pos.line, pos.col - 1), pos))
    } else if pos.line > 0 {
        let prev = pos.line - 1;
        Some((Position::new(prev, buffer.line_len(prev)), pos))
    } else {
        None
    }
}

/// The character range a forward Delete at `pos` would remove, or `None` at
/// the buffer end. At line end the range spans the line terminator (i.e.
/// Delete joins lines).
pub fn delete_range(buffer: &Buffer, pos: Position) -> Option<(Position, Position)> {
    if pos.col < buffer.line_len(pos.line) {
        Some((pos, Position::new(pos.line, pos.col + 1)))
    } else if pos.line + 1 < buffer.line_count() {
        Some((pos, Position::new(pos.line + 1, 0)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer() -> Buffer {
        Buffer::from_text("one\ntwo\nthree")
    }

    #[test]
    fn left_wraps_to_previous_line_end() {
        let b = buffer();
        // Start of line 1 -> end of line 0.
        assert_eq!(move_left(&b, Position::new(1, 0)), Position::new(0, 3));
        // Mid-line moves within the line.
        assert_eq!(move_left(&b, Position::new(1, 2)), Position::new(1, 1));
    }

    #[test]
    fn left_stops_at_buffer_start() {
        let b = buffer();
        let start = Position::start();
        assert_eq!(move_left(&b, start), start);
    }

    #[test]
    fn right_wraps_to_next_line_start_and_stops_at_end() {
        let b = buffer();
        // End of line 0 -> start of line 1.
        assert_eq!(move_right(&b, Position::new(0, 3)), Position::new(1, 0));
        // End of last line stays put.
        let end = Position::new(2, 5);
        assert_eq!(move_right(&b, end), end);
    }

    #[test]
    fn vertical_moves_clamp_line_and_column() {
        let b = buffer();
        // Col 5 clamps to line 0's length (3).
        assert_eq!(
            move_vertically(&b, Position::new(1, 5), -1),
            Position::new(0, 3)
        );
        // Line -2 from line 1 clamps to line 0.
        assert_eq!(
            move_vertically(&b, Position::new(1, 1), -2),
            Position::new(0, 1)
        );
        // Line +99 clamps to the last line, col 5 is valid there.
        assert_eq!(
            move_vertically(&b, Position::new(0, 1), 99),
            Position::new(2, 1)
        );
    }

    #[test]
    fn backspace_joins_lines_at_column_zero() {
        let b = buffer();
        // At start of line 1, Backspace removes "\n" at end of line 0.
        let (start, end) = backspace_range(&b, Position::new(1, 0)).unwrap();
        assert_eq!((start, end), (Position::new(0, 3), Position::new(1, 0)));
        // At buffer start there is nothing to remove.
        assert_eq!(backspace_range(&b, Position::start()), None);
    }

    #[test]
    fn delete_joins_lines_at_line_end() {
        let b = buffer();
        // At end of line 0, Delete removes "\n" before line 1.
        let (start, end) = delete_range(&b, Position::new(0, 3)).unwrap();
        assert_eq!((start, end), (Position::new(0, 3), Position::new(1, 0)));
        // At the buffer end there is nothing to remove.
        assert_eq!(delete_range(&b, Position::new(2, 5)), None);
    }

    #[test]
    fn home_and_end() {
        let b = buffer();
        assert_eq!(line_start(Position::new(2, 3)), Position::new(2, 0));
        assert_eq!(line_end(&b, Position::new(1, 0)), Position::new(1, 3));
    }
}
