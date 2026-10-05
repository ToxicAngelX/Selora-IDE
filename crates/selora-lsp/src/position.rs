//! LSP position ↔ selora-core position conversion.
//!
//! LSP positions are (line, UTF-16 code units) — the `character` field
//! counts UTF-16 units, *not* code points, and that is the default encoding
//! servers assume unless the client negotiates otherwise
//! (lsp-types `Position::character` docs). `selora_core::Position` counts
//! code points (ropey's coordinate). These helpers translate between the
//! two against the document text; a code-point column never drifts from
//! what the server meant, even on lines containing astral characters
//! (`🦀` is 2 UTF-16 units, 1 code point, 4 bytes).

use selora_core::{Buffer, Position};

/// Our (code-point) position → the LSP wire position, against full text.
pub fn to_lsp(text: &str, pos: Position) -> lsp_types::Position {
    let line_text = nth_line(text, pos.line);
    lsp_types::Position::new(pos.line as u32, char_col_to_utf16(line_text, pos.col))
}

/// The LSP wire position → our (code-point) position, against full text.
/// Out-of-range lines/columns are clamped like `Buffer` clamps (an editor
/// should never panic on a stale server position).
pub fn to_core(text: &str, lsp: lsp_types::Position) -> Position {
    let line = lsp.line as usize;
    let line_count = line_count(text);
    if line >= line_count {
        // Clamp to the last line's end (or the very end of the buffer).
        let last = line_count.saturating_sub(1);
        let line_text = nth_line(text, last);
        return Position::new(last, line_text.chars().count());
    }
    let line_text = nth_line(text, line);
    Position::new(line, utf16_col_to_char(line_text, lsp.character))
}

/// The buffer variant of [`to_lsp`] (reads the line off the rope).
pub fn to_lsp_buffer(buffer: &Buffer, pos: Position) -> lsp_types::Position {
    let line_text = buffer.line(pos.line).unwrap_or_default();
    lsp_types::Position::new(pos.line as u32, char_col_to_utf16(&line_text, pos.col))
}

/// The buffer variant of [`to_core`].
pub fn to_core_buffer(buffer: &Buffer, lsp: lsp_types::Position) -> Position {
    let line = (lsp.line as usize).min(buffer.line_count().saturating_sub(1));
    let line_text = buffer.line(line).unwrap_or_default();
    let col = utf16_col_to_char(&line_text, lsp.character).min(buffer.line_len(line));
    Position::new(line, col)
}

/// `str::lines` semantics: split on `\n`, strip a trailing `\r`. This is the
/// line model both `Buffer::line` and LSP use.
fn nth_line(text: &str, line: usize) -> &str {
    text.lines().nth(line).unwrap_or("")
}

fn line_count(text: &str) -> usize {
    // `""` has 1 line (matching Buffer); "a\n" has 2 (the empty trailing
    // line counts). `str::lines` yields n-1 for text with n-1 newlines, so
    // add 1 when the text ends with a newline (or is empty).
    if text.is_empty() {
        return 1;
    }
    text.lines().count() + usize::from(text.ends_with('\n'))
}

/// Code-point column → UTF-16 unit count.
fn char_col_to_utf16(line: &str, col: usize) -> u32 {
    line.chars().take(col).map(char::len_utf16).sum::<usize>() as u32
}

/// UTF-16 unit offset → code-point column (clamped; an offset that lands
/// inside a surrogate pair rounds down to the containing character).
fn utf16_col_to_char(line: &str, units: u32) -> usize {
    let mut seen = 0u32;
    for (col, ch) in line.chars().enumerate() {
        if seen >= units {
            return col;
        }
        seen += ch.len_utf16() as u32;
    }
    line.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line with every coordinate-width mismatch: `é` is 1 code point but
    /// 2 UTF-8 bytes, `🦀` is 1 code point but 2 UTF-16 units.
    const MIXED: &str = "fn é(🦀: u8) {}\nnext line\n";

    #[test]
    fn round_trips_code_point_columns() {
        let text = "plain ascii line";
        for col in 0..=text.chars().count() {
            let lsp = to_lsp(text, Position::new(0, col));
            assert_eq!(lsp, lsp_types::Position::new(0, col as u32));
            assert_eq!(to_core(text, lsp), Position::new(0, col));
        }
    }

    #[test]
    fn utf16_units_differ_from_code_points_only_for_astral_chars() {
        // "🦀" at code-point col 1 is at UTF-16 unit 2.
        let text = "🦀x";
        assert_eq!(
            to_lsp(text, Position::new(0, 1)),
            lsp_types::Position::new(0, 2)
        );
        // And back: unit 2 → code-point col 1.
        assert_eq!(
            to_core(text, lsp_types::Position::new(0, 2)),
            Position::new(0, 1)
        );
        // BMP chars (é) are 1 UTF-16 unit — no skew.
        let text = "éé";
        assert_eq!(
            to_lsp(text, Position::new(0, 2)),
            lsp_types::Position::new(0, 2)
        );
    }

    #[test]
    fn mixed_line_round_trips_both_directions() {
        let line = MIXED.lines().next().expect("first line");
        for col in 0..=line.chars().count() {
            let position = Position::new(0, col);
            assert_eq!(
                to_core(MIXED, to_lsp(MIXED, position)),
                position,
                "col {col}"
            );
        }
        for units in 0..=char_col_to_utf16(line, line.chars().count()) {
            let lsp = lsp_types::Position::new(0, units);
            let back = to_lsp(MIXED, to_core(MIXED, lsp));
            // Round-tripping a unit offset that splits a surrogate pair
            // normalizes up to the pair's end; otherwise it is exact.
            assert!(
                back == lsp || back.character == lsp.character + 1,
                "units {units}: {back:?} vs {lsp:?}"
            );
        }
    }

    #[test]
    fn clamps_out_of_range_positions_like_the_buffer() {
        let text = "one\ntwo";
        // Line beyond the end clamps to the last line's end.
        assert_eq!(
            to_core(text, lsp_types::Position::new(99, 0)),
            Position::new(1, 3)
        );
        // Column beyond the line's end clamps to the line's end.
        assert_eq!(
            to_core(text, lsp_types::Position::new(0, 99)),
            Position::new(0, 3)
        );
        // Our side never exceeds the line either.
        assert_eq!(
            to_lsp(text, Position::new(0, 99)),
            lsp_types::Position::new(0, 3)
        );
    }

    #[test]
    fn empty_and_trailing_newline_line_models_match_the_buffer() {
        let buffer = Buffer::from_text("");
        assert_eq!(line_count(""), buffer.line_count());
        let buffer = Buffer::from_text("a\n");
        assert_eq!(line_count("a\n"), buffer.line_count());
        // Position on the trailing empty line is valid on both sides.
        let text = "a\n";
        assert_eq!(
            to_core(text, lsp_types::Position::new(1, 0)),
            Position::new(1, 0)
        );
        assert_eq!(
            to_lsp(text, Position::new(1, 0)),
            lsp_types::Position::new(1, 0)
        );
    }

    #[test]
    fn buffer_variants_agree_with_text_variants() {
        let buffer = Buffer::from_text(MIXED);
        for line in 0..buffer.line_count() {
            for col in 0..=buffer.line_len(line) {
                let position = Position::new(line, col);
                assert_eq!(
                    to_lsp_buffer(&buffer, position),
                    to_lsp(MIXED, position),
                    "position {position:?}"
                );
                let lsp = to_lsp(MIXED, position);
                assert_eq!(to_core_buffer(&buffer, lsp), to_core(MIXED, lsp));
            }
        }
        // Stale server positions clamp, never panic: line 99 clamps to the
        // buffer's trailing empty line.
        assert_eq!(
            to_core_buffer(&buffer, lsp_types::Position::new(99, 99)),
            Position::new(2, 0)
        );
    }
}
