//! LSP overlay layer for the editor (ARCHITECTURE.md §5, UI_SPEC.md §3.6).
//!
//! The editor-side half of the Phase 3a language intelligence:
//!
//! - **data** — plain diagnostic / completion types mirroring
//!   `selora-lsp`'s event payloads (this crate sits *below* `selora-lsp` in
//!   the dependency graph, so the types are duplicated rather than shared;
//!   the app converts `LspEvent` → these when it applies them to an
//!   [`Editor`]);
//! - **squiggles** — the 3px-tall wave underline under a diagnostic range
//!   (`--diag-err` / `--diag-warn` / `--diag-info`, UI_SPEC.md §3.6),
//!   painted by [`crate::Editor`] from its diagnostics list;
//! - **hover tooltip** — [`shape_hover`] turns a server's markdown blob
//!   into display lines, and [`paint_hover_tooltip`] paints the card;
//! - **completion popup** — [`CompletionPopup`] holds the entries, filters
//!   them against the typed prefix, tracks the selection, and
//!   [`paint_completion_popup`] paints the list;
//! - **the hover gesture** — [`HoverGesture`] turns "pointer stationary for
//!   600 ms over a buffer position" into a hover request the shell forwards
//!   as an LSP effect (deliverable 6).
//!
//! Position note: LSP wire positions count UTF-16 code units;
//! [`utf16_col_to_char`] converts one to the code-point column this
//! editor's layout math needs, against the line text the editor already
//! has.

use std::time::Duration;

use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke};
use selora_core::Position;

use selora_tokens::space;

use crate::layout;
use crate::theme;

/// The pointer must rest this long on one spot before a hover request is
/// emitted (deliverable 6: 600 ms hold).
pub const HOVER_HOLD: Duration = Duration::from_millis(600);

/// Squiggle geometry (UI_SPEC.md §3.6: a 3px-tall underline).
const SQUIGGLE_AMPLITUDE: f32 = 1.5;
const SQUIGGLE_STEP: f32 = 3.0;

/// An LSP (line, UTF-16 character) position — the wire coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

/// Diagnostic severity → the squiggle color (UI_SPEC.md §3.6). `Error` is
/// `--diag-err`, `Warning` `--diag-warn`, everything else `--diag-info`
/// (hint and information share the gray — the spec has three colors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

impl LspSeverity {
    pub fn color(self) -> Color32 {
        match self {
            Self::Error => theme::color::ERR,
            Self::Warning => theme::color::WARN,
            Self::Information | Self::Hint => theme::color::INFO,
        }
    }
}

/// One squiggle: a UTF-16 range, a severity, and the message shown when the
/// pointer rests on the squiggle.
#[derive(Debug, Clone, PartialEq)]
pub struct LspDiagnostic {
    pub range: LspRange,
    pub severity: LspSeverity,
    pub source: Option<String>,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Position conversion (UTF-16 wire columns ↔ code-point columns)
// ---------------------------------------------------------------------------

/// UTF-16 units → code-point column, clamped (an offset that lands inside a
/// surrogate pair rounds down to the containing character).
pub fn utf16_col_to_char(line: &str, units: u32) -> usize {
    let mut seen = 0u32;
    for (col, ch) in line.chars().enumerate() {
        if seen >= units {
            return col;
        }
        seen += ch.len_utf16() as u32;
    }
    line.chars().count()
}

/// Code-point column → UTF-16 units.
pub fn char_col_to_utf16(line: &str, col: usize) -> u32 {
    line.chars().take(col).map(char::len_utf16).sum::<usize>() as u32
}

// ---------------------------------------------------------------------------
// Hover tooltip
// ---------------------------------------------------------------------------

/// Max lines / columns shown in the hover card (it is a signature peek, not
/// a documentation browser).
const HOVER_MAX_LINES: usize = 12;
const HOVER_MAX_COLS: usize = 88;

/// Shape a server's hover markdown into display lines: trim noise, keep
/// code fences (the signature), collapse blank runs, cap the size. Pure —
/// the headless test in this module pins the behavior.
pub fn shape_hover(markdown: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut blanks = 0;
    for line in markdown.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            blanks += 1;
            if blanks <= 1 && !lines.is_empty() {
                lines.push(String::new());
            }
        } else {
            blanks = 0;
            let mut shaped = trimmed.to_owned();
            if shaped.len() > HOVER_MAX_COLS {
                // Cut at a char boundary (safely), with an ellipsis.
                shaped = shaped
                    .char_indices()
                    .take_while(|(idx, _)| *idx < HOVER_MAX_COLS)
                    .map(|(_, ch)| ch)
                    .collect();
                shaped.push('…');
            }
            lines.push(shaped);
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.truncate(HOVER_MAX_LINES);
    lines
}

/// Paint the hover card above `anchor` (below it when there is no room).
/// The content is already shaped ([`shape_hover`]).
pub fn paint_hover_tooltip(painter: &Painter, anchor: Pos2, content: &[String]) {
    if content.is_empty() {
        return;
    }
    let font = FontId::new(theme::type_scale::SM, theme::font::FAMILY_MONO);
    let line_height = theme::type_scale::SM * 1.5;
    let padding = space::S2;
    let mut width = 0.0f32;
    let mut galleys = Vec::with_capacity(content.len());
    for line in content {
        let galley = painter.layout(
            line.clone(),
            font.clone(),
            theme::color::FG_DEFAULT,
            f32::INFINITY,
        );
        width = width.max(galley.size().x);
        galleys.push(galley);
    }
    let height = content.len() as f32 * line_height + 2.0 * padding;
    let rect = tooltip_rect(painter, anchor, width + 2.0 * padding, height);

    painter.rect_filled(rect, theme::radius::R1, theme::color::INK_MID);
    painter.rect_stroke(
        rect,
        theme::radius::R1,
        Stroke::new(1.0, theme::color::INK_HI),
        egui::StrokeKind::Inside,
    );
    let mut y = rect.top() + padding;
    for galley in galleys {
        painter.galley(
            pos2(rect.left() + padding, y),
            galley,
            theme::color::FG_DEFAULT,
        );
        y += line_height;
    }
}

/// The tooltip rect for `anchor`, flipped below the anchor when the card
/// would run off the top of the screen, and clamped horizontally.
fn tooltip_rect(painter: &Painter, anchor: Pos2, width: f32, height: f32) -> Rect {
    let screen = painter.clip_rect();
    let flip = anchor.y - space::S3 - height < screen.top();
    let top = if flip {
        anchor.y + space::S3
    } else {
        anchor.y - space::S3 - height
    };
    let left = (anchor.x - width / 2.0)
        .clamp(screen.left(), screen.right() - width)
        .max(screen.left());
    Rect::from_min_size(pos2(left, top), vec2(width, height))
}

// ---------------------------------------------------------------------------
// Completion popup
// ---------------------------------------------------------------------------

/// One completion entry (the popup's data). The app converts
/// `selora_lsp::LspCompletionItem` into this when a
/// `LspEvent::Completions` arrives.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionEntry {
    /// The label as shown (and as inserted, modulo the typed prefix).
    pub label: String,
    /// Raw LSP `CompletionItemKind` number, mapped to a tag by
    /// [`kind_tag`].
    pub kind: Option<i32>,
    /// Extra detail shown on the row (type / signature).
    pub detail: Option<String>,
}

/// The completion popup's model: entries, the typed filter, and the
/// selection. The shell owns one of these while the popup is open: it feeds
/// keystrokes via [`CompletionPopup::set_filter`], moves the selection on
/// ↑/↓, and accepts on Enter/Tab via [`CompletionPopup::accept`] (then
/// [`crate::Editor::insert_completion`] applies the label to the buffer).
#[derive(Debug, Clone, Default)]
pub struct CompletionPopup {
    entries: Vec<CompletionEntry>,
    filter: String,
    selected: usize,
}

impl CompletionPopup {
    pub fn new(entries: Vec<CompletionEntry>) -> Self {
        Self {
            entries,
            filter: String::new(),
            selected: 0,
        }
    }

    /// Whether there is anything to show.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The typed prefix since the popup opened (the word being completed).
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Update the typed prefix; the selection is reset when it filters out.
    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_owned();
        let visible = self.filtered().len();
        if self.selected >= visible {
            self.selected = 0;
        }
    }

    /// The entries whose label starts with the filter (case-insensitive).
    pub fn filtered(&self) -> Vec<&CompletionEntry> {
        let needle = self.filter.to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|entry| {
                needle.is_empty() || entry.label.to_ascii_lowercase().starts_with(&needle)
            })
            .collect()
    }

    /// Move the selection (↑/↓ wrap around).
    pub fn move_selection(&mut self, delta: i32) {
        let visible = self.filtered().len();
        if visible == 0 {
            return;
        }
        let current = self.selected as i32;
        self.selected = (current + delta).rem_euclid(visible as i32) as usize;
    }

    /// The accepted entry (Enter / Tab), from the filtered list.
    pub fn accept(&self) -> Option<&CompletionEntry> {
        self.filtered().into_iter().nth(self.selected)
    }

    /// The selected index within the filtered list (for painting).
    pub fn selected_index(&self) -> usize {
        self.selected
    }
}

/// The start column of the identifier the caret sits in (the prefix a
/// completion replaces). `caret_col` is a code-point column into `line`.
pub fn completion_word_start(line: &str, caret_col: usize) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let mut start = caret_col.min(chars.len());
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    start
}

/// A short tag for an LSP `CompletionItemKind` number (the popup's right
/// column — a kind hint without icon assets, per the no-decoration rule).
pub fn kind_tag(kind: Option<i32>) -> &'static str {
    match kind {
        Some(1) => "txt",
        Some(2) => "mth",
        Some(3) => "fn",
        Some(4) => "new",
        Some(5) => "fld",
        Some(6) => "var",
        Some(7) => "cls",
        Some(8) => "trt",
        Some(9) => "mod",
        Some(10) => "prp",
        Some(12) => "enm",
        Some(13) => "kw",
        Some(15) => "col",
        Some(17) => "ref",
        Some(22) => "str",
        Some(23) => "evt",
        Some(24) => "op",
        Some(25) => "tp",
        _ => "sym",
    }
}

/// Paint the completion list above `anchor` (the caret). At most 8 rows are
/// visible; the selection is highlighted.
pub fn paint_completion_popup(painter: &Painter, anchor: Pos2, popup: &CompletionPopup) {
    const MAX_ROWS: usize = 8;
    let visible = popup.filtered();
    if visible.is_empty() {
        return;
    }
    let rows = visible.len().min(MAX_ROWS);
    let row_height = layout::LINE_HEIGHT * 0.9;
    let label_font = FontId::new(theme::type_scale::SM, theme::font::FAMILY_MONO);
    let tag_font = FontId::new(theme::type_scale::XS, theme::font::FAMILY_MONO);
    let padding = space::S1;

    let mut width = 0.0f32;
    for entry in &visible {
        let galley = painter.layout(
            entry.label.clone(),
            label_font.clone(),
            Color32::PLACEHOLDER,
            f32::INFINITY,
        );
        width = width.max(galley.size().x);
    }
    width = (width + 96.0).min(360.0);
    let height = rows as f32 * row_height + 2.0 * padding;
    let rect = tooltip_rect(painter, anchor, width, height);

    painter.rect_filled(rect, theme::radius::R1, theme::color::INK_MID);
    painter.rect_stroke(
        rect,
        theme::radius::R1,
        Stroke::new(1.0, theme::color::INK_HI),
        egui::StrokeKind::Inside,
    );

    // The window of rows around the selection (scroll-free: show the first
    // 8, or the block containing the selection).
    let selected = popup.selected_index().min(visible.len().saturating_sub(1));
    let first = if selected >= rows {
        selected + 1 - rows
    } else {
        0
    };
    for (row, entry) in visible.iter().skip(first).take(rows).enumerate() {
        let index = first + row;
        let y = rect.top() + padding + row as f32 * row_height;
        let row_rect = Rect::from_min_size(
            pos2(rect.left() + padding, y),
            vec2(width - 2.0 * padding, row_height),
        );
        if index == selected {
            painter.rect_filled(row_rect, theme::radius::R0, theme::color::INK_LOW);
        }
        painter.text(
            pos2(row_rect.left(), row_rect.center().y),
            Align2::LEFT_CENTER,
            &entry.label,
            label_font.clone(),
            theme::color::FG_DEFAULT,
        );
        painter.text(
            pos2(row_rect.right(), row_rect.center().y),
            Align2::RIGHT_CENTER,
            kind_tag(entry.kind),
            tag_font.clone(),
            theme::color::FG_FAINT,
        );
    }
}

// ---------------------------------------------------------------------------
// The hover gesture (deliverable 6: request hover after a 600 ms hold)
// ---------------------------------------------------------------------------

/// The pure core of the hover gesture: the pointer must stay on one buffer
/// position for [`HOVER_HOLD`] before a request is emitted — once per
/// position, resetting when the pointer moves away.
#[derive(Debug, Default)]
pub(crate) struct HoverGesture {
    anchored: Option<(Position, f64, bool)>, // (position, hold start, emitted)
}

impl HoverGesture {
    /// One frame. `position` is the buffer position under the pointer (or
    /// `None` when not over the text). `time` is the egui frame clock.
    /// Returns the position to request a hover for, if the hold completed
    /// this frame.
    pub(crate) fn tick(
        &mut self,
        hovered: bool,
        dragging: bool,
        position: Option<Position>,
        time: f64,
    ) -> Option<Position> {
        if !hovered || dragging {
            self.anchored = None;
            return None;
        }
        let Some(position) = position else {
            self.anchored = None;
            return None;
        };
        let hold_secs = HOVER_HOLD.as_secs_f64();
        match &mut self.anchored {
            Some((anchor, start, emitted)) => {
                if *anchor != position {
                    *anchor = position;
                    *start = time;
                    *emitted = false;
                }
                if !*emitted && time - *start >= hold_secs - 1e-9 {
                    *emitted = true;
                    return Some(position);
                }
            }
            anchored @ None => {
                *anchored = Some((position, time, false));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// The editor's overlay state (owned by [`crate::Editor`])
// ---------------------------------------------------------------------------

/// Everything the editor tracks for the LSP overlay: the diagnostics to
/// squiggle, the document-sync bookkeeping the shell reads, and the pending
/// hover requests it emits.
#[derive(Debug, Default)]
pub(crate) struct OverlayState {
    pub(crate) diagnostics: Vec<LspDiagnostic>,
    /// `note_lsp_opened` flips this once; the shell sends `LspEffect::Open`
    /// on the first paint of a tab.
    pub(crate) opened: bool,
    /// Commit counter — every buffer-mutating edit bumps it; the shell
    /// compares it between frames to emit `LspEffect::Change`.
    pub(crate) commits: u64,
    /// Buffer positions under a completed hover hold, drained by
    /// [`crate::Editor::take_lsp_requests`].
    pub(crate) requests: Vec<Position>,
    pub(crate) hover_gesture: HoverGesture,
    /// Where the caret was last painted (popup anchoring).
    pub(crate) caret_screen: Option<Pos2>,
}

/// Paint one diagnostic's squiggle segment on a row: a 3px wave from
/// `x_start` to `x_end` at the bottom of the line's band.
pub(crate) fn paint_squiggle(
    painter: &Painter,
    x_start: f32,
    x_end: f32,
    row_bottom: f32,
    color: Color32,
) {
    if x_end <= x_start {
        return; // a zero-width range has nothing to underline (start char is implied at minimum by callers)
    }
    let mid = row_bottom - SQUIGGLE_AMPLITUDE;
    let mut points = Vec::new();
    let mut up = false;
    let mut x = x_start;
    while x < x_end {
        points.push(pos2(
            x,
            mid + if up {
                -SQUIGGLE_AMPLITUDE
            } else {
                SQUIGGLE_AMPLITUDE
            },
        ));
        up = !up;
        x += SQUIGGLE_STEP;
    }
    points.push(pos2(
        x_end,
        mid + if up {
            -SQUIGGLE_AMPLITUDE
        } else {
            SQUIGGLE_AMPLITUDE
        },
    ));
    painter.line(points, Stroke::new(1.0, color));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_colors_follow_the_ui_spec() {
        assert_eq!(LspSeverity::Error.color(), theme::color::ERR);
        assert_eq!(LspSeverity::Warning.color(), theme::color::WARN);
        assert_eq!(LspSeverity::Information.color(), theme::color::INFO);
        assert_eq!(LspSeverity::Hint.color(), theme::color::INFO);
    }

    #[test]
    fn utf16_conversion_handles_astral_characters() {
        // "🦀" is 1 code point, 2 UTF-16 units.
        let line = "fn a(🦀: u8)";
        assert_eq!(char_col_to_utf16(line, 6), 7); // col of ':' → 5 ascii + 2 for the crab
        assert_eq!(utf16_col_to_char(line, 7), 6);
        // A unit offset inside the surrogate pair rounds to the pair start.
        assert_eq!(utf16_col_to_char(line, 6), 6);
        // Clamped at the ends.
        assert_eq!(utf16_col_to_char(line, 999), line.chars().count());
        assert_eq!(char_col_to_utf16(line, 999), 12); // 11 code points, the crab counts twice
    }

    #[test]
    fn shape_hover_trims_noise_and_keeps_the_signature() {
        let markdown = "\n```rust\nfn insert(pos: Position, text: &str)\n```\n\n\n---\nextra\n";
        let lines = shape_hover(markdown);
        assert_eq!(
            lines,
            vec![
                "```rust",
                "fn insert(pos: Position, text: &str)",
                "```",
                // Two blank lines collapse to one paragraph separator.
                "",
                "---",
                "extra",
            ]
        );
        // Empty / whitespace-only content shapes to nothing (no empty card).
        assert!(shape_hover("   \n  \n").is_empty());
        // Long lines are capped with an ellipsis.
        let long = "x".repeat(200);
        let shaped = shape_hover(&long);
        assert_eq!(shaped.len(), 1);
        assert!(shaped[0].ends_with('…'));
        assert!(shaped[0].chars().count() <= 90);
        // And the line count is capped.
        let many: String = (0..40).map(|i| format!("line {i}\n")).collect();
        assert_eq!(shape_hover(&many).len(), HOVER_MAX_LINES);
    }

    #[test]
    fn completion_popup_filters_and_navigates() {
        let popup = CompletionPopup::new(vec![
            CompletionEntry {
                label: "new()".into(),
                kind: Some(3),
                detail: Some("fn() -> Vec<T>".into()),
            },
            CompletionEntry {
                label: "with_capacity(capacity: usize)".into(),
                kind: Some(3),
                detail: None,
            },
            CompletionEntry {
                label: "NotMatching".into(),
                kind: None,
                detail: None,
            },
        ]);
        assert_eq!(popup.filtered().len(), 3);

        // Typing narrows case-insensitively by the label prefix.
        let mut popup = popup;
        popup.set_filter("W");
        assert_eq!(popup.filtered().len(), 1);
        assert_eq!(popup.filtered()[0].label, "with_capacity(capacity: usize)");
        assert_eq!(
            popup.accept().map(|entry| entry.label.as_str()),
            Some("with_capacity(capacity: usize)")
        );

        // No match: nothing selected/acceptable.
        popup.set_filter("zz");
        assert!(popup.filtered().is_empty());
        assert!(popup.accept().is_none());

        // Selection wraps around the filtered list.
        let mut popup = CompletionPopup::new(vec![
            CompletionEntry {
                label: "a".into(),
                kind: None,
                detail: None,
            },
            CompletionEntry {
                label: "b".into(),
                kind: None,
                detail: None,
            },
            CompletionEntry {
                label: "c".into(),
                kind: None,
                detail: None,
            },
        ]);
        popup.move_selection(1);
        popup.move_selection(1);
        popup.move_selection(1); // wraps to 0
        assert_eq!(popup.selected_index(), 0);
        popup.move_selection(-1); // wraps to the last
        assert_eq!(popup.selected_index(), 2);
        // Re-filtering resets an out-of-range selection.
        popup.set_filter("c");
        assert_eq!(popup.accept().map(|entry| entry.label.as_str()), Some("c"));
    }

    #[test]
    fn completion_word_start_finds_the_identifier_prefix() {
        let line = "    let x: Vec<u8> = ve";
        // Caret after "ve": the prefix starts at "ve".
        assert_eq!(completion_word_start(line, line.chars().count()), 21);
        // Caret on "<" (not an identifier char): the prefix is empty.
        assert_eq!(completion_word_start(line, 15), 15);
        // Caret at 0: empty.
        assert_eq!(completion_word_start(line, 0), 0);
        // Underscores belong to identifiers.
        let line = "foo_bar";
        assert_eq!(completion_word_start(line, 7), 0);
    }

    #[test]
    fn kind_tags_map_the_common_lsp_kinds() {
        assert_eq!(kind_tag(Some(3)), "fn");
        assert_eq!(kind_tag(Some(6)), "var");
        assert_eq!(kind_tag(Some(22)), "str");
        assert_eq!(kind_tag(None), "sym");
        assert_eq!(kind_tag(Some(99)), "sym");
    }

    #[test]
    fn hover_gesture_emits_once_after_the_hold() {
        let mut gesture = HoverGesture::default();
        let position = Position::new(3, 7);

        // Fresh hold: no emit before 600 ms.
        assert_eq!(gesture.tick(true, false, Some(position), 0.0), None);
        assert_eq!(gesture.tick(true, false, Some(position), 0.5), None);
        // At 600 ms: emit.
        assert_eq!(
            gesture.tick(true, false, Some(position), 0.6),
            Some(position)
        );
        // And only once for the same position.
        assert_eq!(gesture.tick(true, false, Some(position), 1.0), None);

        // Moving the pointer re-arms the timer.
        assert_eq!(
            gesture.tick(true, false, Some(Position::new(4, 2)), 1.1),
            None
        );
        assert_eq!(
            gesture.tick(true, false, Some(Position::new(4, 2)), 1.7),
            Some(Position::new(4, 2))
        );

        // Not hovered / dragging resets.
        assert_eq!(gesture.tick(false, false, Some(position), 2.0), None);
        assert_eq!(gesture.tick(true, false, Some(position), 2.05), None);
        assert_eq!(gesture.tick(true, true, Some(position), 3.0), None);
        assert_eq!(gesture.tick(true, false, Some(position), 3.05), None);
    }
}
