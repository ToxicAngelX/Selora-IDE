//! The text buffer (ARCHITECTURE.md §10.1): a ropey wrapper with line/column
//! as first-class coordinates. No IO, no async, no UI.

use ropey::Rope;
use uuid::Uuid;

/// Identifies a buffer across the workspace (ARCHITECTURE.md §3.2).
/// A `Uuid` assigned on creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferId(Uuid);

impl BufferId {
    /// Fresh unique id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BufferId {
    fn default() -> Self {
        Self::new()
    }
}

/// A zero-based line/column position into a [`Buffer`].
///
/// `col` counts characters (UTF-32 code points, how ropey counts), not bytes
/// or display columns — gestures and layout convert to this coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Position {
    pub line: usize,
    pub col: usize,
}

impl Position {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }

    /// The buffer start (`0,0`).
    pub fn start() -> Self {
        Self { line: 0, col: 0 }
    }

    /// Order-insensitive comparison helper: the earlier position.
    pub fn min(self, other: Self) -> Self {
        if self <= other {
            self
        } else {
            other
        }
    }

    /// Order-insensitive comparison helper: the later position.
    pub fn max(self, other: Self) -> Self {
        if self >= other {
            self
        } else {
            other
        }
    }
}

/// A text buffer over a rope. Created empty or from a file's contents;
/// callers own the IO (ARCHITECTURE.md §10.1: "no IO" in this crate).
#[derive(Debug, Clone, Default)]
pub struct Buffer {
    id: BufferId,
    rope: Rope,
}

impl Buffer {
    /// A new empty buffer.
    pub fn new() -> Self {
        Self {
            id: BufferId::new(),
            rope: Rope::new(),
        }
    }

    /// A buffer pre-loaded with `text`.
    pub fn from_text(text: &str) -> Self {
        Self {
            id: BufferId::new(),
            rope: Rope::from_str(text),
        }
    }

    /// This buffer's unique id.
    pub fn id(&self) -> BufferId {
        self.id
    }

    /// The whole buffer text. Allocates; fine for save/spike use, and the
    /// editor pane reads per-line instead.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Number of lines. A buffer `"a"` has 1 line; `"a\n"` has 2 (the empty
    /// trailing line counts, matching how an editor behaves).
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Number of characters (code points) in the buffer.
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    /// One line's text, without its trailing newline. `None` if out of range.
    pub fn line(&self, line: usize) -> Option<String> {
        if line >= self.line_count() {
            return None;
        }
        Some(
            self.rope
                .line(line)
                .to_string()
                .trim_end_matches(['\n', '\r'])
                .to_string(),
        )
    }

    /// Length of `line` in characters, excluding the line terminator.
    /// `line` is clamped into range; out-of-range returns 0.
    pub fn line_len(&self, line: usize) -> usize {
        match self.line(line) {
            Some(s) => s.chars().count(),
            None => 0,
        }
    }

    /// Insert `text` at `pos` (clamped into the buffer).
    pub fn insert(&mut self, pos: Position, text: &str) {
        let char_idx = self.clamp_char(self.position_to_char(pos));
        self.rope.insert(char_idx, text);
    }

    /// Remove the text between `start` and `end` (order-insensitive, clamped).
    /// Removing nothing (start == end) is a no-op.
    pub fn remove(&mut self, start: Position, end: Position) {
        let a = self.position_to_char(start.min(end));
        let b = self.position_to_char(start.max(end));
        let a = self.clamp_char(a);
        let b = self.clamp_char(b);
        if a < b {
            self.rope.remove(a..b);
        }
    }

    /// Convert a `Position` to a ropey char index. Out-of-range positions are
    /// clamped to valid spots (an editor should never panic on a stale cursor).
    pub fn position_to_char(&self, pos: Position) -> usize {
        let line = pos.line.min(self.line_count().saturating_sub(1));
        let line_start = self.rope.line_to_char(line);
        let col = pos.col.min(self.line_len(line));
        (line_start + col).min(self.len_chars())
    }

    /// Convert a char index to a `Position` (clamped).
    pub fn char_to_position(&self, char_idx: usize) -> Position {
        let char_idx = self.clamp_char(char_idx);
        let line = self.rope.char_to_line(char_idx);
        let col = char_idx - self.rope.line_to_char(line);
        Position { line, col }
    }

    /// Clamp a char index to `[0, len_chars]`.
    fn clamp_char(&self, char_idx: usize) -> usize {
        char_idx.min(self.len_chars())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_has_one_line() {
        let b = Buffer::new();
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.text(), "");
        assert_eq!(b.line(0).as_deref(), Some(""));
    }

    #[test]
    fn insert_at_cursor_and_read_back() {
        let mut b = Buffer::from_text("hello world");
        b.insert(Position::new(0, 5), " brave");
        assert_eq!(b.text(), "hello brave world");
    }

    #[test]
    fn insert_clamps_out_of_range() {
        let mut b = Buffer::from_text("ab");
        // Line 99 does not exist — clamps to the last line, col clamps to its end.
        b.insert(Position::new(99, 99), "!");
        assert_eq!(b.text(), "ab!");
        b.insert(Position::new(0, 9999), "!");
        assert_eq!(b.text(), "ab!!");
    }

    #[test]
    fn remove_range_order_insensitive() {
        let mut b = Buffer::from_text("hello brave world");
        // Same range, both directions.
        let mut b2 = b.clone();
        b.remove(Position::new(0, 5), Position::new(0, 11));
        b2.remove(Position::new(0, 11), Position::new(0, 5));
        assert_eq!(b.text(), "hello world");
        assert_eq!(b2.text(), "hello world");
    }

    #[test]
    fn line_col_round_trip() {
        let b = Buffer::from_text("one\ntwo\nthree");
        for (line, expected) in [(0, "one"), (1, "two"), (2, "three")] {
            let text = b.line(line).unwrap();
            assert_eq!(text, expected);
            // position_to_char → char_to_position is the identity for every col.
            for col in 0..=text.chars().count() {
                let p = Position::new(line, col);
                assert_eq!(b.char_to_position(b.position_to_char(p)), p);
            }
        }
    }

    #[test]
    fn line_count_counts_trailing_empty_line() {
        let b = Buffer::from_text("a\n");
        assert_eq!(b.line_count(), 2);
        assert_eq!(b.line(1).as_deref(), Some(""));
    }

    #[test]
    fn cols_count_code_points_not_bytes() {
        // "é" is 2 UTF-8 bytes, 1 code point; "🦀" is 4 bytes, 1 code point.
        let mut b = Buffer::from_text("é🦀x");
        b.insert(Position::new(0, 2), "!");
        assert_eq!(b.text(), "é🦀!x");
        assert_eq!(b.len_chars(), 4);
    }

    #[test]
    fn ids_are_unique() {
        let a = Buffer::new();
        let b = Buffer::new();
        assert_ne!(a.id(), b.id());
    }
}
