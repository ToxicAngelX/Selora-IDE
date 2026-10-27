//! The DiffView widget (Phase R4) — the shared read-only unified-diff
//! renderer. The chat's approval cards paint through it; Phase R6's git
//! working-tree tabs will too.
//!
//! Rows: gutter line numbers (old for context/`-`, new for `+`) in
//! `--fg-faint`, removed lines `--err`, added `--ok`, context
//! `--fg-default` (the chat's only saturated uses — semantic, §4.2:266),
//! header/hunk/notice rows faint and full-width.
//!
//! R4 upgrades:
//!
//! - **Syntax highlighting** — the new-side text (added + context lines)
//!   is reassembled and parsed once by [`SyntaxLayer`]; the resulting
//!   spans paint in the §1.5 mono-tinted palette, marker prefix and
//!   gutter kept in the diff colors. The parse is cached (the diffs are
//!   stable strings; a global 32-entry map keyed by content hash) so
//!   steady frames cost no tree-sitter work.
//! - **Word-level intra-line emphasis** — a removed/added pair's
//!   common-prefix/suffix trim marks the changed span with a 12% alpha
//!   tint of the same semantic color (a DESIGN_TOKENS §12 bend, flagged
//!   for sign-off in the R4 follow-up: no new token values — alpha of
//!   existing `--err`/`--ok` only, applied at paint time the
//!   `with_alpha` way).
//!
//! Every color, font, and spacing value comes from [`crate::theme`]
//! (§12). Paint-only; no threads; no per-frame allocation beyond the
//! row text runs the painter lays out.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use egui::{pos2, vec2, Align2, Rect, Sense, Ui};
use selora_editor::{syntax::SyntaxKind, Language};

use crate::presence::with_alpha;
use crate::theme::{color, font, radius, space, type_scale};

/// One diff body row's height (the 18px row pitch, §4.2:265).
pub const DIFF_ROW_HEIGHT: f32 = 18.0;
/// The card-body pad the rows align to (agent_cards' CARD_PAD).
pub const CARD_PAD: f32 = space::S2;
/// The emphasis tint's alpha (12% of the semantic color).
const EMPHASIS_ALPHA: f32 = 0.12;

/// One classified line of the router's unified diff (fs.rs emits `--- `
/// / `+++ ` headers, `@@ -a,b +c,d @@` hunks, `' '`/`-`/`+` body lines,
/// and a `… (diff truncated at N lines)` notices).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    /// `--- a/x`, `--- /dev/null`, `+++ b/x` — painted faint, never as a
    /// removed/added line.
    Header(String),
    /// `@@ -a,b +c,d @@` — painted faint; carries the gutter counters.
    Hunk(String),
    /// A context line (` text`) — both numbers advance.
    Context { old: u32, new: u32, text: String },
    /// A removed line (`-text`) — the old number advances.
    Removed { old: u32, text: String },
    /// An added line (`+text`) — the new number advances.
    Added { new: u32, text: String },
    /// The router's `… (diff truncated at N lines)` notice.
    Notice(String),
    /// Anything else (a blank row, a malformed line) — faint, no gutter.
    Other(String),
}

/// Parse the router's unified diff (fs.rs's emitter). Header rows are
/// classified BEFORE the single-char `'-'`/`'+'` test so a create's
/// `--- /dev/null` never lands as a removed line; the `@@` counters seed
/// the gutter numbers (old for context/`-`, new for `+`).
pub fn parse_unified_diff(diff: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let mut old_no = 0u32;
    let mut new_no = 0u32;
    for raw in diff.lines() {
        if raw.starts_with("--- ") || raw.starts_with("+++ ") {
            lines.push(DiffLine::Header(raw.to_owned()));
            continue;
        }
        if raw.starts_with("@@") {
            if let Some((old_start, new_start)) = parse_hunk_header(raw) {
                old_no = old_start;
                new_no = new_start;
            }
            lines.push(DiffLine::Hunk(raw.to_owned()));
            continue;
        }
        if raw.starts_with('…') {
            lines.push(DiffLine::Notice(raw.to_owned()));
            continue;
        }
        match raw.as_bytes().first() {
            Some(b' ') => {
                lines.push(DiffLine::Context {
                    old: old_no,
                    new: new_no,
                    text: raw.to_owned(),
                });
                old_no = old_no.saturating_add(1);
                new_no = new_no.saturating_add(1);
            }
            Some(b'-') => {
                lines.push(DiffLine::Removed {
                    old: old_no,
                    text: raw.to_owned(),
                });
                old_no = old_no.saturating_add(1);
            }
            Some(b'+') => {
                lines.push(DiffLine::Added {
                    new: new_no,
                    text: raw.to_owned(),
                });
                new_no = new_no.saturating_add(1);
            }
            _ => lines.push(DiffLine::Other(raw.to_owned())),
        }
    }
    lines
}

/// The tree-sitter language for a diff's file — from its `+++ b/path`
/// header (a create's `+++ b/new.rs`; `/dev/null` deletes have none).
pub fn language_for_diff(lines: &[DiffLine]) -> Option<Language> {
    let header = lines.iter().rev().find_map(|line| match line {
        DiffLine::Header(text) => text
            .strip_prefix("+++ ")
            .and_then(|path| path.split('/').next_back().map(str::to_owned))
            .map(|file| (text.clone(), file)),
        _ => None,
    });
    let file = header.map(|(_, file)| file)?;
    Language::from_extension(file.rsplit('.').next().unwrap_or_default())
}

/// The gutter number for a body line (old for context/`-`, new for
/// `+`); `None` for header/hunk/notice rows.
pub fn gutter_number(line: &DiffLine) -> Option<u32> {
    match line {
        DiffLine::Context { old, .. } | DiffLine::Removed { old, .. } => Some(*old),
        DiffLine::Added { new, .. } => Some(*new),
        _ => None,
    }
}

/// The line's text color (§4.2:266 — the chat's only saturated uses,
/// semantic).
pub fn diff_line_color(line: &DiffLine) -> egui::Color32 {
    match line {
        DiffLine::Removed { .. } => color::ERR,
        DiffLine::Added { .. } => color::OK,
        DiffLine::Context { .. } => color::FG_DEFAULT,
        _ => color::FG_FAINT,
    }
}

/// The line's raw text for painting.
fn diff_line_text(line: &DiffLine) -> &str {
    match line {
        DiffLine::Header(text)
        | DiffLine::Hunk(text)
        | DiffLine::Notice(text)
        | DiffLine::Other(text) => text,
        DiffLine::Context { text, .. }
        | DiffLine::Removed { text, .. }
        | DiffLine::Added { text, .. } => text,
    }
}

/// The first line the diff changes on the NEW side (the first added
/// line's number, else the first hunk's new start) — the card's
/// "open at line" target. 1 when nothing parses.
pub fn first_changed_line(lines: &[DiffLine]) -> usize {
    if let Some(DiffLine::Added { new, .. }) = lines
        .iter()
        .find(|line| matches!(line, DiffLine::Added { .. }))
    {
        return (*new).max(1) as usize;
    }
    for line in lines {
        if let DiffLine::Hunk(text) = line {
            if let Some((_, new_start)) = parse_hunk_header(text) {
                return new_start.max(1) as usize;
            }
        }
    }
    1
}

/// The `@@ -a,b +c,d @@` hunk counters.
fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    let inner = line.strip_prefix("@@")?.trim_start();
    let mut parts = inner.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let old_start = old.split(',').next()?.parse().ok()?;
    let new_start = new.split(',').next()?.parse().ok()?;
    Some((old_start, new_start))
}

// -----------------------------------------------------------------------------
// The prepared form (highlighting + emphasis, cached)
// -----------------------------------------------------------------------------

/// One highlighted char range within a body line's text (after the
/// marker prefix): `None` = default color (the line's diff color).
#[derive(Clone, Debug, PartialEq, Eq)]
struct RowSection {
    char_start: usize,
    char_end: usize,
    kind: Option<SyntaxKind>,
}

/// One body line's prepared row.
#[derive(Clone, Debug, Default)]
struct PreparedRow {
    /// Highlighted sections over the text AFTER the marker char; the
    /// marker char itself paints in the line color.
    sections: Vec<RowSection>,
    /// The changed-span char range (after the marker) for the word-level
    /// emphasis; `None` on unpaired lines.
    emphasis: Option<(usize, usize)>,
}

/// A whole diff's prepared rows.
#[derive(Debug, Default)]
struct PreparedDiff {
    rows: Vec<PreparedRow>,
}

/// The prepared-diff cache: diffs are stable strings painted every
/// frame; the tree-sitter parse happens once. A dumb 32-entry map (the
/// whole map clears past the cap — sessions hold a handful of diffs).
fn prepared_cache() -> &'static Mutex<HashMap<u64, Arc<PreparedDiff>>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Arc<PreparedDiff>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Hash a diff's identity: the language + every line's raw text.
fn hash_lines(lines: &[DiffLine], language: Option<Language>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    language.hash(&mut hasher);
    for line in lines {
        std::mem::discriminant(line).hash(&mut hasher);
        diff_line_text(line).hash(&mut hasher);
    }
    hasher.finish()
}

/// Build (or fetch from the cache) the diff's prepared rows: the
/// new-side syntax highlighting + the word-level emphasis.
fn prepare(lines: &[DiffLine], language: Option<Language>) -> Arc<PreparedDiff> {
    let key = hash_lines(lines, language);
    if let Some(hit) = prepared_cache().lock().expect("cache lock").get(&key) {
        return hit.clone();
    }

    let mut rows: Vec<PreparedRow> = lines.iter().map(|_| PreparedRow::default()).collect();

    // 1. Reassemble the new side (context + added lines, marker stripped)
    //    and map each contributing line to its byte range in the source.
    let mut source = String::new();
    let mut line_ranges: Vec<(usize, usize, usize)> = Vec::new(); // (row, byte_start, byte_end)
    for (index, line) in lines.iter().enumerate() {
        let (text, marker_width) = match line {
            DiffLine::Context { text, .. } => (text.strip_prefix(' ').unwrap_or(text), 1),
            DiffLine::Added { text, .. } => (text.strip_prefix('+').unwrap_or(text), 1),
            _ => continue,
        };
        let start = source.len();
        source.push_str(text);
        source.push('\n');
        line_ranges.push((index, start, source.len() - 1));
        let _ = marker_width;
    }

    // 2. Parse + map the spans back to per-row char columns.
    if let Some(language) = language {
        if let Some(mut layer) = selora_editor::SyntaxLayer::new(language) {
            layer.set_source(&source);
            for (row, start, end) in &line_ranges {
                let row_text = &source[*start..*end];
                let char_of_byte = |byte: usize| -> usize {
                    row_text
                        .char_indices()
                        .take_while(|(offset, _)| *offset + *start < byte)
                        .count()
                };
                let prepared = &mut rows[*row];
                for span in layer.spans() {
                    let span_start = (span.start as usize).max(*start);
                    let span_end = (span.end as usize).min(*end);
                    if span_end <= span_start {
                        continue;
                    }
                    let section = RowSection {
                        char_start: char_of_byte(span_start),
                        char_end: char_of_byte(span_end),
                        kind: Some(span.kind),
                    };
                    match prepared.sections.last_mut() {
                        // Merge adjacent same-kind sections (the tree-sitter
                        // emitter splits one logical scope).
                        Some(last)
                            if last.kind == section.kind && last.char_end == section.char_start =>
                        {
                            last.char_end = section.char_end;
                        }
                        _ => prepared.sections.push(section),
                    }
                }
                prepared.sections.retain(|s| s.char_end > s.char_start);
            }
        }
    }

    // 3. Word-level emphasis: pair consecutive removed/added runs (a
    //    context or hunk row ends a run); the i-th removed pairs with
    //    the i-th added; the common prefix/suffix trims to the changed
    //    span. Unpaired lines get nothing.
    let mut index = 0usize;
    while index < lines.len() {
        let run_start = index;
        let removed: Vec<usize> = lines[run_start..]
            .iter()
            .enumerate()
            .take_while(|(_, line)| matches!(line, DiffLine::Removed { .. }))
            .map(|(offset, _)| run_start + offset)
            .collect();
        if removed.is_empty() {
            index = run_start + 1;
            continue;
        }
        let after_removed = run_start + removed.len();
        let added: Vec<usize> = lines[after_removed..]
            .iter()
            .enumerate()
            .take_while(|(_, line)| matches!(line, DiffLine::Added { .. }))
            .map(|(offset, _)| after_removed + offset)
            .collect();
        for (pair, removed_row) in removed.iter().enumerate() {
            let Some(added_row) = added.get(pair) else {
                break;
            };
            let removed_text = lines[*removed_row].strip_marker().unwrap_or_default();
            let added_text = lines[*added_row].strip_marker().unwrap_or_default();
            if let Some((start, end)) = changed_span(removed_text, added_text) {
                rows[*removed_row].emphasis = Some((start, end));
                rows[*added_row].emphasis = Some((start, end));
            }
        }
        index = after_removed + added.len();
    }

    let prepared = Arc::new(PreparedDiff { rows });
    let mut cache = prepared_cache().lock().expect("cache lock");
    if cache.len() > 32 {
        cache.clear();
    }
    cache.insert(key, prepared.clone());
    prepared
}

impl DiffLine {
    /// The text after the `' '`/`'+'`/`'-'` marker.
    fn strip_marker(&self) -> Option<&str> {
        match self {
            DiffLine::Context { text, .. } => Some(text.strip_prefix(' ').unwrap_or(text)),
            DiffLine::Removed { text, .. } => Some(text.strip_prefix('-').unwrap_or(text)),
            DiffLine::Added { text, .. } => Some(text.strip_prefix('+').unwrap_or(text)),
            _ => None,
        }
    }
}

/// The changed span (char range within BOTH texts) after the common
/// prefix/suffix trim; `None` when one side is empty (pure add/remove —
/// the whole line already carries its color).
fn changed_span(a: &str, b: &str) -> Option<(usize, usize)> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    // The maximal common prefix/suffix (any characters), then both ends
    // back off to word edges so the span covers whole changed words.
    let mut prefix = 0usize;
    while prefix < a_chars.len() && prefix < b_chars.len() && a_chars[prefix] == b_chars[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < a_chars.len() - prefix
        && suffix < b_chars.len() - prefix
        && a_chars[a_chars.len() - 1 - suffix] == b_chars[b_chars.len() - 1 - suffix]
    {
        suffix += 1;
    }
    // The maximal trim covered everything: the lines are equal.
    if prefix + suffix >= a_chars.len() {
        return None;
    }
    while prefix > 0 && a_chars[prefix - 1].is_alphanumeric() {
        prefix -= 1;
    }
    while suffix > 0 && a_chars[a_chars.len() - suffix].is_alphanumeric() {
        suffix -= 1;
    }
    let end = a_chars.len() - suffix;
    Some((prefix, end))
}

// -----------------------------------------------------------------------------
// The widget
// -----------------------------------------------------------------------------

/// One diff, ready to paint. Create per frame (cheap — the prepared
/// rows come from the cache) and call [`DiffView::paint`] from the
/// scroll body.
pub struct DiffView<'a> {
    lines: &'a [DiffLine],
    prepared: Arc<PreparedDiff>,
}

impl<'a> DiffView<'a> {
    /// A diff over `lines`, highlighting for `language` (the new side).
    pub fn new(lines: &'a [DiffLine], language: Option<Language>) -> Self {
        Self {
            lines,
            prepared: prepare(lines, language),
        }
    }

    /// The body height at `cap` (rows × pitch).
    pub fn height(&self, cap: f32) -> f32 {
        (self.lines.len() as f32 * DIFF_ROW_HEIGHT).min(cap)
    }

    /// Paint the rows into `ui` (the scroll body; rows allocate
    /// themselves). `fade` is the card-entrance alpha.
    pub fn paint(&self, ui: &mut Ui, width: f32, fade: f32) {
        let max_number = self
            .lines
            .iter()
            .filter_map(gutter_number)
            .max()
            .unwrap_or(0);
        let digits = max_number.to_string().len().max(1);
        let gutter_width = ui
            .painter()
            .layout_no_wrap(
                "8".repeat(digits),
                code_font(),
                with_alpha(color::FG_FAINT, fade),
            )
            .size()
            .x
            + space::S2;
        let text_x = CARD_PAD + gutter_width + space::S1;
        let mono = code_font();

        for (index, line) in self.lines.iter().enumerate() {
            let (row, _) = ui.allocate_exact_size(vec2(width, DIFF_ROW_HEIGHT), Sense::hover());
            let painter = ui.painter();
            let clipped = painter.with_clip_rect(row);
            let line_color = with_alpha(diff_line_color(line), fade);
            let prepared = &self.prepared.rows[index];

            match line {
                DiffLine::Context { .. } | DiffLine::Removed { .. } | DiffLine::Added { .. } => {
                    if let Some(number) = gutter_number(line) {
                        clipped.text(
                            pos2(CARD_PAD + gutter_width, row.center().y),
                            Align2::RIGHT_CENTER,
                            number.to_string(),
                            mono.clone(),
                            with_alpha(color::FG_FAINT, fade),
                        );
                    }
                    // The emphasis tint behind the changed span, before
                    // the text runs (the gutter is never covered).
                    if let Some((start, end)) = prepared.emphasis {
                        let marker = 1usize; // ' '/'-'/'+'
                        let text = line.strip_marker().unwrap_or_default();
                        let before: String = text.chars().take(start).collect();
                        let span: String = text.chars().take(end).skip(start).collect();
                        if !span.is_empty() {
                            let before_width = clipped
                                .layout_no_wrap(before, mono.clone(), line_color)
                                .size()
                                .x;
                            let span_width = clipped
                                .layout_no_wrap(span, mono.clone(), line_color)
                                .size()
                                .x;
                            let tint = with_alpha(diff_line_color(line), EMPHASIS_ALPHA);
                            clipped.rect_filled(
                                Rect::from_min_max(
                                    pos2(text_x + before_width, row.top() + 2.0),
                                    pos2(text_x + before_width + span_width, row.bottom() - 2.0),
                                ),
                                radius::R0,
                                tint,
                            );
                            let _ = marker;
                        }
                    }
                    // The marker char, then the highlighted sections:
                    // walk the boundary set (section edges) painting one
                    // text run per segment.
                    let marker_text: String = diff_line_text(line).chars().take(1).collect();
                    let marker_galley =
                        clipped.layout_no_wrap(marker_text, mono.clone(), line_color);
                    clipped.galley(
                        pos2(text_x, row.center().y - marker_galley.size().y / 2.0),
                        marker_galley,
                        line_color,
                    );
                    let mut x = text_x + marker_gap(digits, gutter_width, mono.clone(), &clipped);
                    let chars: Vec<char> =
                        line.strip_marker().unwrap_or_default().chars().collect();
                    let mut cursor = 0usize;
                    while cursor < chars.len() {
                        let (kind, end) = run_boundary(&prepared.sections, cursor, chars.len());
                        let run: String = chars[cursor..end].iter().collect();
                        let run_color = match kind {
                            Some(kind) => with_alpha(kind.color(), fade),
                            None => line_color,
                        };
                        let galley = clipped.layout_no_wrap(run, mono.clone(), run_color);
                        clipped.galley(
                            pos2(x, row.center().y - galley.size().y / 2.0),
                            galley.clone(),
                            run_color,
                        );
                        x += galley.size().x;
                        cursor = end;
                    }
                }
                _ => {
                    // Header / hunk / truncation-notice rows: full width,
                    // faint, no gutter (§4.2: a create's `--- /dev/null`
                    // is a header, never a removed line).
                    clipped.text(
                        pos2(CARD_PAD, row.center().y),
                        Align2::LEFT_CENTER,
                        diff_line_text(line),
                        mono.clone(),
                        line_color,
                    );
                }
            }
        }
    }
}

/// The run starting at `cursor`: its highlight kind (when inside a
/// section) and the char index it ends at (the next boundary).
fn run_boundary(
    sections: &[RowSection],
    cursor: usize,
    text_len: usize,
) -> (Option<SyntaxKind>, usize) {
    for section in sections {
        if section.char_start <= cursor && cursor < section.char_end {
            return (section.kind, section.char_end.min(text_len));
        }
        if section.char_start > cursor {
            return (None, section.char_start.min(text_len));
        }
    }
    (None, text_len)
}

/// The gap after the marker char (a quarter-space keeps the glyph from
/// kissing the marker; plain rows painted it as part of the text).
fn marker_gap(
    digits: usize,
    gutter_width: f32,
    mono: egui::FontId,
    painter: &egui::Painter,
) -> f32 {
    let _ = (digits, gutter_width);
    painter
        .layout_no_wrap(" ".to_owned(), mono, egui::Color32::TRANSPARENT)
        .size()
        .x
        .min(4.0)
}

/// Code font: `--type-sm` in `--font-mono` (the cards' body font).
fn code_font() -> egui::FontId {
    egui::FontId::new(type_scale::SM, font::FAMILY_MONO)
}

#[cfg(test)]
mod tests {
    use super::*;

    // parse_unified_diff's own tests live in agent_cards' suite (they
    // moved with the widget); these cover the R4 additions.

    #[test]
    fn first_changed_line_reads_the_new_side() {
        let lines = parse_unified_diff(
            "--- a/main.rs
+++ b/main.rs
@@ -10,3 +10,3 @@
 context
-old
+new
+newer",
        );
        assert_eq!(first_changed_line(&lines), 11); // the first `+` line
        let only_hunk = parse_unified_diff(
            "@@ -5 +9 @@
-same
+diff",
        );
        assert_eq!(first_changed_line(&only_hunk), 9);
        assert_eq!(first_changed_line(&[]), 1);
    }

    #[test]
    fn language_for_diff_reads_the_b_header() {
        let lines = parse_unified_diff("--- a/main.rs\n+++ b/main.rs\n@@ -1 +1 @@\n-old\n+new");
        assert_eq!(language_for_diff(&lines), Some(Language::Rust));
        let none = parse_unified_diff("--- /dev/null\n+++ b/notes\n@@ -0,0 +1 @@\n+x");
        assert_eq!(language_for_diff(&none), None);
    }

    #[test]
    fn changed_span_trims_common_prefix_and_suffix() {
        // fn main() {} -> fn main() -> println; the changed span is the
        // middle.
        let (start, end) = changed_span("fn main() {}", "fn main() -> u32 {").expect("a span");
        let text = "fn main() {}";
        let span: String = text.chars().skip(start).take(end - start).collect();
        assert_eq!(span, "{}");
    }

    #[test]
    fn changed_span_refuses_empty_sides_and_identical_words() {
        assert_eq!(changed_span("", "added"), None);
        assert_eq!(changed_span("removed", ""), None);
        assert_eq!(changed_span("same", "same"), None);
    }

    /// The prepared rows: paired lines carry the same emphasis span; a
    /// pure add carries none.
    #[test]
    fn emphasis_pairs_removed_and_added_runs() {
        let lines = parse_unified_diff(
            "@@ -1,2 +1,2 @@\n-let value = old;\n+let value = new;\n context\n+pure addition\n",
        );
        let prepared = prepare(&lines, None);
        let removed = prepared
            .rows
            .iter()
            .position(|row| row.emphasis.is_some())
            .expect("a paired row");
        // Both the removed and added line of the pair carry the span.
        assert!(prepared.rows[removed].emphasis.is_some());
        assert!(prepared.rows[removed + 1].emphasis.is_some());
        assert_eq!(
            prepared.rows[removed].emphasis,
            prepared.rows[removed + 1].emphasis
        );
        // The context row and the trailing pure-add carry none.
        assert!(prepared.rows[removed + 2].emphasis.is_none());
        assert!(prepared.rows[removed + 3].emphasis.is_none());
    }

    /// The cache returns the same Arc for the same diff identity.
    #[test]
    fn the_cache_hits_the_same_diff() {
        let lines = parse_unified_diff("@@ -1 +1 @@\n-a\n+b");
        let first = prepare(&lines, None);
        let second = prepare(&lines, None);
        assert!(Arc::ptr_eq(&first, &second));
    }

    /// The widget paints headlessly with and without a language.
    #[test]
    fn diff_view_paints_headlessly() {
        let lines = parse_unified_diff(
            "--- a/main.rs\n+++ b/main.rs\n@@ -1,2 +1,2 @@\n-fn main() {}\n+fn main() -> u32 {\n context\n",
        );
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 600.0));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                DiffView::new(&lines, language_for_diff(&lines)).paint(ui, 340.0, 1.0);
            },
        );
        output.textures_delta.clear();
    }
}
