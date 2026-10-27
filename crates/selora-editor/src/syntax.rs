//! Syntax highlighting via tree-sitter (Phase 2d, ROADMAP.md §Phase 2).
//!
//! The [`SyntaxLayer`] wraps a tree-sitter [`Parser`] (incremental re-parse
//! on edit) and a tree-sitter-highlight [`Highlighter`] (scope extraction),
//! and turns highlight events into byte-addressed [`Span`]s colored by the
//! DESIGN_TOKENS.md §1.5 mono palette (`theme::color::SYN_*`). Comments are
//! the one italic scope in the IDE (DESIGN_TOKENS.md §2.5).
//!
//! The layer owns a shadow copy of the source text: tree-sitter works in byte
//! offsets and (byte) points, while the editor's [`Position`] is
//! line/code-point based. Edits are described as `(start, old_end, new_text)`
//! — the same shape an LSP `TextDocumentContentChangeEvent` carries — and the
//! layer keeps the shadow, the [`tree_sitter::InputEdit`]s, and the cached
//! spans in step with them.
//!
//! Incremental behavior on edit:
//! 1. apply the [`tree_sitter::InputEdit`] to the old tree,
//! 2. re-parse with the old tree as the hint (tree-sitter re-uses unchanged
//!    subtrees and only re-parses the changed ranges),
//! 3. diff old vs. new tree with [`tree_sitter::Tree::changed_ranges`]:
//!    - no structural change (typing inside a string/comment/identifier) —
//!      the cached spans are patched by pure offset shifting, no re-highlight;
//!    - structural change — the spans are recomputed from the highlighter.
//!
//! Large-file fallback: for buffers over [`LARGE_FILE_LINES`] lines,
//! re-parse is deferred (cheap timestamp check, no timer machinery) until
//! [`REPARSE_DEBOUNCE_SECS`] has elapsed since the last parse; the pending
//! [`tree_sitter::InputEdit`]s accumulate on the tree in the meantime and
//! [`SyntaxLayer::tick`] flushes them from the editor's frame loop.
//!
//! ## TOML and Markdown: lexical fallback, not tree-sitter (deviation)
//!
//! The pinned `tree-sitter-toml 0.20.0` and `tree-sitter-markdown 0.7.1`
//! crates predate the `LanguageFn` binding split: they return a `Language`
//! value tied to tree-sitter **0.20 / 0.19**, a different Rust *type* from
//! tree-sitter 0.27's, and their compiled C cores duplicate the 0.27 core's
//! `ts_*` symbols (MSVC `LNK2005`) — a binary cannot link both. Those two
//! languages therefore use a small pure-Rust lexical scanner ([`fallback`])
//! behind the *same* [`SyntaxLayer`] API, spans, and SYN_* palette. Swapping
//! the pinned crates for `LanguageFn`-style bindings (e.g. the
//! `tree-sitter-toml-updated` / `tree-sitter-markdown-updated` forks) lets
//! the tree-sitter path take over again: add the grammar to [`ts_language`]
//! and [`queries`] and delete the fallback arms.

use std::path::Path;

use egui::Color32;
use selora_core::Position;
use tree_sitter::{InputEdit, Language as TsLanguage, Parser, Point, Tree};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::theme;

/// The languages Selora IDE Lite highlights (ROADMAP.md Phase 2d: the 9
/// grammars). Unknown extensions fall back to plain `SYN_DEFAULT` text (no
/// layer at all — the editor holds `Option<SyntaxLayer>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    /// TypeScript proper (`*.ts`).
    TypeScript,
    /// TSX (`*.tsx`) — a separate grammar in tree-sitter-typescript.
    Tsx,
    JavaScript,
    Python,
    Json,
    Toml,
    Markdown,
    Html,
    Css,
}

impl Language {
    /// The language for a file extension (case-insensitive, without the dot).
    /// `None` for unknown extensions — plain text.
    pub fn from_extension(extension: &str) -> Option<Self> {
        Some(match extension.to_ascii_lowercase().as_str() {
            "rs" => Self::Rust,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "py" | "pyi" => Self::Python,
            "json" => Self::Json,
            "toml" => Self::Toml,
            "md" | "markdown" => Self::Markdown,
            "html" | "htm" => Self::Html,
            "css" => Self::Css,
            _ => return None,
        })
    }

    /// The language for a file path, by extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
    }

    /// Whether this language is highlighted by the lexical [`fallback`]
    /// instead of tree-sitter (see the module docs for why).
    fn is_fallback(self) -> bool {
        matches!(self, Self::Toml | Self::Markdown)
    }
}

/// One scope kind in the DESIGN_TOKENS.md §1.5 syntax palette. The kind is
/// the only thing the highlighter produces; the editor derives color (and
/// italics for comments) from it via [`SyntaxKind::color`] /
/// [`SyntaxKind::italics`] so no paint site can invent a color.
///
/// (`--syn-var` is `#e5e5e5` — the same value as `--syn-default` — and is
/// not exposed by `selora-tokens`; variable captures therefore map to
/// [`SyntaxKind::Default`], which is the same color the token defines.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SyntaxKind {
    Default,
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Punctuation,
    Attribute,
}

impl SyntaxKind {
    /// The DESIGN_TOKENS.md §1.5 color for this scope.
    pub const fn color(self) -> Color32 {
        match self {
            Self::Default => theme::color::SYN_DEFAULT,
            Self::Keyword => theme::color::SYN_KEYWORD,
            Self::Function => theme::color::SYN_FN,
            Self::Type => theme::color::SYN_TYPE,
            Self::String => theme::color::SYN_STRING,
            Self::Number => theme::color::SYN_NUMBER,
            Self::Comment => theme::color::SYN_COMMENT,
            Self::Punctuation => theme::color::SYN_PUNCT,
            Self::Attribute => theme::color::SYN_ATTR,
        }
    }

    /// Italics are reserved for comments (DESIGN_TOKENS.md §2.5).
    pub const fn italics(self) -> bool {
        matches!(self, Self::Comment)
    }
}

/// A highlighted byte range in the layer's source text. Spans are sorted,
/// non-overlapping, and never cover un-highlighted (default) text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    /// Byte offset of the first covered byte.
    pub start: u32,
    /// Byte offset one past the last covered byte.
    pub end: u32,
    /// The scope for the covered range.
    pub kind: SyntaxKind,
}

/// A highlighted character range within a single buffer line — the unit the
/// editor's paint path consumes (its columns are `Position::col`-style
/// code-point columns, matching the galley's per-glyph x offsets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSection {
    /// First covered character (code-point column).
    pub char_start: usize,
    /// One past the last covered character.
    pub char_end: usize,
    /// The scope for the covered range.
    pub kind: SyntaxKind,
}

/// Buffers with more lines than this skip per-keystroke re-parse.
pub const LARGE_FILE_LINES: usize = 50_000;
/// How long a large buffer waits between re-parses (seconds).
pub const REPARSE_DEBOUNCE_SECS: f64 = 0.5;

/// Whether a pending edit should trigger a re-parse at `now`. Small files
/// always re-parse immediately (a few hundred microseconds); large files wait
/// out the debounce so typing stays cheap on 50k+-line buffers.
pub fn reparse_due(line_count: usize, now: f64, last_parse: f64) -> bool {
    line_count <= LARGE_FILE_LINES || now - last_parse >= REPARSE_DEBOUNCE_SECS
}

/// The recognized highlight names, in the order that maps to the
/// `tree_sitter_highlight::Highlight` indices the highlighter reports
/// (`HighlightConfiguration::configure` matches each query capture to the
/// longest-prefix name in this list). The paired
/// [`SyntaxKind`] is what index → kind lookup returns.
const RECOGNIZED: [(&str, SyntaxKind); 18] = [
    ("comment", SyntaxKind::Comment),
    ("keyword", SyntaxKind::Keyword),
    ("label", SyntaxKind::Keyword),
    ("function", SyntaxKind::Function),
    ("constructor", SyntaxKind::Function),
    ("type", SyntaxKind::Type),
    ("class", SyntaxKind::Type),
    ("string", SyntaxKind::String),
    ("number", SyntaxKind::Number),
    ("constant", SyntaxKind::Number),
    ("punctuation", SyntaxKind::Punctuation),
    ("operator", SyntaxKind::Punctuation),
    ("attribute", SyntaxKind::Attribute),
    ("decorator", SyntaxKind::Attribute),
    ("tag", SyntaxKind::Attribute),
    ("variable", SyntaxKind::Default),
    ("property", SyntaxKind::Default),
    ("embedded", SyntaxKind::Default),
];

/// The tree-sitter language handle for a Selora language. TOML and Markdown
/// are absent on purpose — see the module docs (lexical [`fallback`]).
fn ts_language(language: Language) -> TsLanguage {
    match language {
        Language::Rust => TsLanguage::new(tree_sitter_rust::LANGUAGE),
        Language::TypeScript => TsLanguage::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
        Language::Tsx => TsLanguage::new(tree_sitter_typescript::LANGUAGE_TSX),
        Language::JavaScript => TsLanguage::new(tree_sitter_javascript::LANGUAGE),
        Language::Python => TsLanguage::new(tree_sitter_python::LANGUAGE),
        Language::Json => TsLanguage::new(tree_sitter_json::LANGUAGE),
        Language::Html => TsLanguage::new(tree_sitter_html::LANGUAGE),
        Language::Css => TsLanguage::new(tree_sitter_css::LANGUAGE),
        // Fallback languages never construct a tree-sitter engine (their
        // `SyntaxLayer` skips `TreeEngine` entirely); unreachable by
        // construction, but keep a safe fallback anyway.
        Language::Toml | Language::Markdown => TsLanguage::new(tree_sitter_json::LANGUAGE),
    }
}

/// The three query sources for a language (highlights / injections / locals).
/// TypeScript highlights combine the JavaScript query with the
/// TypeScript-specific one — the TypeScript grammar is a superset of the
/// JavaScript grammar and the crate's own query only covers the additions
/// (this is the standard combination, e.g. how nvim wires it).
fn queries(language: Language) -> (String, &'static str, &'static str) {
    match language {
        Language::Rust => (
            tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ),
        Language::TypeScript => (
            format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            ),
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_typescript::LOCALS_QUERY,
        ),
        Language::Tsx => (
            format!(
                "{}\n{}",
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            ),
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_typescript::LOCALS_QUERY,
        ),
        Language::JavaScript => (
            tree_sitter_javascript::HIGHLIGHT_QUERY.to_owned(),
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ),
        Language::Python => (tree_sitter_python::HIGHLIGHTS_QUERY.to_owned(), "", ""),
        Language::Json => (tree_sitter_json::HIGHLIGHTS_QUERY.to_owned(), "", ""),
        Language::Html => (
            tree_sitter_html::HIGHLIGHTS_QUERY.to_owned(),
            tree_sitter_html::INJECTIONS_QUERY,
            "",
        ),
        Language::Css => (tree_sitter_css::HIGHLIGHTS_QUERY.to_owned(), "", ""),
        // Fallback languages: no tree-sitter queries are used.
        Language::Toml | Language::Markdown => (String::new(), "", ""),
    }
}

/// Build the highlight configuration for a language: the parser + query pair
/// that maps its grammar onto the `RECOGNIZED` capture names.
fn configuration(language: Language, ts_lang: &TsLanguage) -> Option<HighlightConfiguration> {
    let (highlights, injections, locals) = queries(language);
    let name = format!("{language:?}").to_ascii_lowercase();
    let mut config =
        HighlightConfiguration::new(ts_lang.clone(), name, &highlights, injections, locals).ok()?;
    let names: Vec<&str> = RECOGNIZED.iter().map(|(name, _)| *name).collect();
    config.configure(&names);
    Some(config)
}

/// The tree-sitter machinery a layer owns for a grammar-driven language.
struct TreeEngine {
    parser: Parser,
    config: HighlightConfiguration,
    highlighter: Highlighter,
}

/// One buffer's syntax state: parser, tree, highlighter, and the highlight
/// spans derived from the current source.
///
/// Created per language (via [`SyntaxLayer::new`], `None` if a grammar's
/// tree-sitter engine cannot be initialized — the caller then shows plain
/// text). The source is set with [`SyntaxLayer::set_source`] (file open) and
/// kept in step with edits via [`SyntaxLayer::edit`].
pub struct SyntaxLayer {
    language: Language,
    /// tree-sitter engine (parser + queries + highlighter). `None` for the
    /// two lexical-fallback languages, TOML and Markdown (module docs).
    tree_engine: Option<TreeEngine>,
    tree: Option<Tree>,
    source: String,
    /// Byte offset of each line start. `len == line_count()`.
    line_starts: Vec<usize>,
    char_count: usize,
    spans: Vec<Span>,
    last_parse: f64,
    dirty: bool,
    /// The last edit's `(start_byte, old_end_byte, byte_delta)`, kept for
    /// [`SyntaxLayer::shift_spans`] in case the re-parse finds no
    /// structural change.
    pending_shift: Option<(usize, usize, isize)>,
}

impl SyntaxLayer {
    /// A layer for `language`, or `None` if its grammar or queries cannot be
    /// loaded (the caller falls back to un-highlighted text).
    pub fn new(language: Language) -> Option<Self> {
        let tree_engine = if language.is_fallback() {
            None
        } else {
            let ts_lang = ts_language(language);
            let mut parser = Parser::new();
            parser.set_language(&ts_lang).ok()?;
            let config = configuration(language, &ts_lang)?;
            Some(TreeEngine {
                parser,
                config,
                highlighter: Highlighter::new(),
            })
        };
        Some(Self {
            language,
            tree_engine,
            tree: None,
            source: String::new(),
            line_starts: vec![0],
            char_count: 0,
            spans: Vec::new(),
            last_parse: f64::NEG_INFINITY,
            dirty: false,
            pending_shift: None,
        })
    }

    /// This layer's language.
    pub fn language(&self) -> Language {
        self.language
    }

    /// The shadow source text (test/inspection access; the editor paints
    /// from the shared `Buffer`, which is kept in step by `edit`).
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether an edit is waiting for a deferred re-parse.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The cached highlight spans (byte offsets into the layer's source).
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// The layer's shadow source length in characters (for the editor's
    /// staleness guard — compare against `Buffer::len_chars`).
    pub fn char_count(&self) -> usize {
        self.char_count
    }

    /// The shadow source's line count (compare against
    /// `Buffer::line_count`).
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Replace the whole source (file open, buffer swap, or a detected
    /// desync). Full parse + full highlight, immediately.
    pub fn set_source(&mut self, text: &str) {
        self.source.clear();
        self.source.push_str(text);
        self.char_count = text.chars().count();
        self.rebuild_line_starts();
        self.spans.clear();
        self.tree = None;
        self.pending_shift = None;
        self.dirty = true;
        self.reparse();
        self.last_parse = 0.0;
    }

    /// Apply one edit, described LSP-style: the text between `start` and
    /// `old_end` is replaced by `new_text`. Positions refer to the document
    /// state *before* this edit (apply edits in document order).
    ///
    /// `now` is the frame clock in seconds (egui's `input.time`); it feeds
    /// the large-file re-parse debounce.
    pub fn edit(&mut self, start: Position, old_end: Position, new_text: &str, now: f64) {
        let start = start.min(old_end);
        let old_end = start.max(old_end);

        let start_byte = self.byte_offset(start);
        let old_end_byte = self.byte_offset(old_end);
        let old_len = old_end_byte - start_byte;
        let new_len = new_text.len();
        let delta = new_len as isize - old_len as isize;

        // Byte-column points for the InputEdit (tree-sitter points count
        // bytes within the row, not characters).
        let start_column = start_byte - self.line_start(start.line);
        let old_end_column = old_end_byte - self.line_start(old_end.line);
        let new_lines = new_text.bytes().filter(|byte| *byte == b'\n').count();
        let new_end_column = match new_text.rfind('\n') {
            Some(nl) => new_text.len() - nl - 1,
            None => start_column + new_len,
        };

        let edit = InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte: start_byte + new_len,
            start_position: Point {
                row: start.line,
                column: start_column,
            },
            old_end_position: Point {
                row: old_end.line,
                column: old_end_column,
            },
            new_end_position: Point {
                row: start.line + new_lines,
                column: new_end_column,
            },
        };

        // Splice the shadow source and its line starts. The line containing
        // the edit start never moves, so its start doubles as the scan
        // anchor when line structure changes.
        let start_line = self.line_index_of_byte(start_byte);
        let scan_from = self.line_start(start_line);
        let removed = self.source[start_byte..old_end_byte].to_owned();
        self.source
            .replace_range(start_byte..old_end_byte, new_text);
        let single_line = !new_text.contains('\n') && !removed.contains('\n');
        if single_line {
            // Fast path (the overwhelming majority of keystrokes): the edit
            // stays within one line, so only the line starts after it shift.
            for line_start in &mut self.line_starts[start_line + 1..] {
                *line_start = line_start.saturating_sub(old_len) + new_len;
            }
        } else {
            // Line structure changed: re-derive every line start from the
            // (still valid) start of the edited line onward.
            self.line_starts.truncate(start_line + 1);
            let mut byte = scan_from;
            while let Some(offset) = self.source[byte..].bytes().position(|byte| byte == b'\n') {
                self.line_starts.push(byte + offset + 1);
                byte += offset + 1;
            }
        }
        self.char_count =
            (self.char_count + new_text.chars().count()).saturating_sub(removed.chars().count());

        if let Some(tree) = &mut self.tree {
            tree.edit(&edit);
        }
        // Remember the edit extent: if the re-parse finds no structural
        // change, `shift_spans` replays it as a pure offset shift.
        self.pending_shift = Some((start_byte, old_end_byte, delta));
        self.dirty = true;
        if reparse_due(self.line_count(), now, self.last_parse) {
            self.reparse();
            self.last_parse = now;
        }
    }

    /// Pump the debounce from the editor's frame loop: flush a deferred
    /// re-parse once the interval has elapsed. Cheap when clean.
    pub fn tick(&mut self, now: f64) {
        if self.dirty && reparse_due(self.line_count(), now, self.last_parse) {
            self.reparse();
            self.last_parse = now;
        }
    }

    /// The highlight sections (character columns) for one buffer line.
    /// `line_text` must be that line's content without the terminator (what
    /// `Buffer::line` returns); the sections are clipped to it.
    pub fn line_sections(&self, line: usize, line_text: &str) -> Vec<LineSection> {
        let Some(line_start) = self.line_starts.get(line).copied() else {
            return Vec::new();
        };
        let line_end = line_start + line_text.len();
        let char_of_byte = |byte: usize| {
            line_text
                .char_indices()
                .take_while(|(offset, _)| *offset < byte)
                .count()
        };
        let mut sections: Vec<LineSection> = Vec::new();
        for span in &self.spans {
            let span_start = (span.start as usize).max(line_start);
            let span_end = (span.end as usize).min(line_end);
            if span_end <= span_start {
                continue;
            }
            let section = LineSection {
                char_start: char_of_byte(span_start - line_start),
                char_end: char_of_byte(span_end - line_start),
                kind: span.kind,
            };
            match sections.last_mut() {
                // Merge with the previous section when a span continues it
                // (tree-sitter-highlight emits adjacent Source events for
                // one logical scope).
                Some(last) if last.kind == section.kind && last.char_end == section.char_start => {
                    last.char_end = section.char_end;
                }
                _ => sections.push(section),
            }
        }
        sections
    }

    /// Re-parse the shadow source (incrementally, with the old tree as the
    /// hint) and refresh the cached spans when the syntax actually changed.
    fn reparse(&mut self) {
        let old_tree = self.tree.take();
        let new_tree = match self.tree_engine.as_mut() {
            Some(engine) => engine.parser.parse(self.source.as_str(), old_tree.as_ref()),
            // Fallback languages have no tree; their spans are recomputed
            // directly from the source.
            None => None,
        };

        // `changed_ranges` on the (edited) old tree vs. the new tree yields
        // the byte ranges whose syntax differs. No ranges + no error-state
        // flip means every token kept its identity — the edit landed inside
        // a single token and the spans only need offset shifting.
        let structural_change = match (&old_tree, &new_tree) {
            (Some(old), Some(new)) => {
                old.changed_ranges(new).next().is_some()
                    || old.root_node().has_error() != new.root_node().has_error()
            }
            // First parse, parse failure, or a fallback language: recompute.
            _ => true,
        };
        self.tree = new_tree;
        if structural_change || self.spans.is_empty() {
            self.pending_shift = None;
            self.rebuild_spans();
        } else {
            self.shift_spans();
        }
        self.dirty = false;
    }

    /// Recompute the whole span list: from the highlighter for
    /// grammar-driven languages, or from the lexical scanner for TOML and
    /// Markdown.
    fn rebuild_spans(&mut self) {
        let Some(engine) = self.tree_engine.as_mut() else {
            self.spans = fallback::spans(self.language, &self.source);
            return;
        };

        // (tree-sitter-highlight's `Highlighter` re-parses internally — it
        // does not accept an external tree — so this is the one full-cost
        // path; the `changed_ranges` guard keeps it off the common
        // keystroke.)
        let mut spans: Vec<Span> = Vec::new();
        let mut stack: Vec<SyntaxKind> = Vec::new();
        let events = engine.highlighter.highlight(
            &engine.config,
            self.source.as_bytes(),
            None,
            None,
            &|_: &str| None,
        );
        if let Ok(events) = events {
            for event in events.flatten() {
                match event {
                    HighlightEvent::HighlightStart(highlight) => {
                        if let Some((_, kind)) = RECOGNIZED.get(highlight.0) {
                            stack.push(*kind);
                        }
                    }
                    HighlightEvent::HighlightEnd => {
                        stack.pop();
                    }
                    HighlightEvent::Source { start, end } => {
                        let kind = stack.last().copied().unwrap_or(SyntaxKind::Default);
                        if kind == SyntaxKind::Default || end <= start {
                            continue;
                        }
                        match spans.last_mut() {
                            Some(last) if last.kind == kind && last.end as usize == start => {
                                last.end = end as u32;
                            }
                            _ => spans.push(Span {
                                start: start as u32,
                                end: end as u32,
                                kind,
                            }),
                        }
                    }
                }
            }
        }
        self.spans = spans;
    }

    /// Shift cached spans past an in-token edit (no structural change: the
    /// edited byte range lay inside a single token, so spans before it stay,
    /// spans after it shift by the length delta, and the covering span
    /// grows or shrinks). The edit extent was stashed in `pending_shift` by
    /// `edit`; a stale/absent entry (defensive) triggers nothing.
    fn shift_spans(&mut self) {
        if let Some((start_byte, old_end_byte, delta)) = self.pending_shift.take() {
            for span in &mut self.spans {
                if span.start as usize >= old_end_byte {
                    span.start = (span.start as isize + delta).max(0) as u32;
                    span.end = (span.end as isize + delta).max(0) as u32;
                } else if span.end as usize > start_byte {
                    let new_end = (span.end as isize + delta).max(span.start as isize) as usize;
                    span.end = new_end as u32;
                }
            }
            self.spans.retain(|span| span.end > span.start);
        }
    }

    /// Rebuild `line_starts` from scratch (after `set_source`).
    fn rebuild_line_starts(&mut self) {
        self.line_starts.clear();
        self.line_starts.push(0);
        let mut byte = 0;
        while let Some(offset) = self.source[byte..].bytes().position(|byte| byte == b'\n') {
            self.line_starts.push(byte + offset + 1);
            byte += offset + 1;
        }
    }

    /// The byte offset of the line start of `line` (clamped into range).
    fn line_start(&self, line: usize) -> usize {
        let line = line.min(self.line_starts.len().saturating_sub(1));
        self.line_starts[line]
    }

    /// The index of the line containing `byte`.
    fn line_index_of_byte(&self, byte: usize) -> usize {
        match self.line_starts.binary_search(&byte) {
            Ok(line) => line,
            Err(next) => next - 1,
        }
    }

    /// The byte offset of `pos` in the shadow source (clamped: a stale
    /// position maps to the nearest valid offset rather than panicking).
    fn byte_offset(&self, pos: Position) -> usize {
        let line = pos.line.min(self.line_starts.len().saturating_sub(1));
        let line_start = self.line_starts[line];
        let line_end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.source.len());
        let mut byte = line_start;
        let mut remaining = pos.col;
        for ch in self.source[line_start..line_end].chars() {
            if remaining == 0 || ch == '\n' {
                break;
            }
            byte += ch.len_utf8();
            remaining -= 1;
        }
        byte
    }
}

/// Lexical fallback scanners for TOML and Markdown — pure functions from
/// source text to [`Span`]s, used only because the pinned grammar crates
/// for those two languages cannot link with tree-sitter 0.27 (see the
/// module docs). They emit the same SYN_* kinds a grammar would: comments
/// faint+italic, strings dim, numbers slightly bright, structure keywords
/// bright, punctuation muted.
mod fallback {
    use super::{Language, Span, SyntaxKind};

    /// Compute the highlight spans for a fallback language's whole source.
    pub fn spans(language: Language, source: &str) -> Vec<Span> {
        match language {
            Language::Toml => toml_spans(source),
            Language::Markdown => markdown_spans(source),
            // Grammar-driven languages never route through here.
            _ => Vec::new(),
        }
    }

    /// Push a span, merging with the previous one when they touch and share
    /// a kind (keeps the list sorted and minimal).
    fn push(spans: &mut Vec<Span>, start: usize, end: usize, kind: SyntaxKind) {
        if end <= start {
            return;
        }
        match spans.last_mut() {
            Some(last) if last.kind == kind && last.end as usize == start => {
                last.end = end as u32;
            }
            _ => spans.push(Span {
                start: start as u32,
                end: end as u32,
                kind,
            }),
        }
    }

    /// Consume a TOML string starting at `start` (which is a `"` or `'`)
    /// and return the exclusive end offset of the literal.
    fn scan_toml_string(bytes: &[u8], start: usize) -> usize {
        let quote = bytes[start];
        // Multiline (triple-quote) strings first.
        if bytes[start..].starts_with(&[quote, quote, quote]) {
            let mut byte = start + 3;
            while byte < bytes.len() {
                if bytes[byte..].starts_with(&[quote, quote, quote]) {
                    return (byte + 3).min(bytes.len());
                }
                // Escapes are literal in '''-strings, skipped in """-ones.
                if quote == b'"' && bytes[byte] == b'\\' {
                    byte += 1;
                }
                byte += 1;
            }
            return bytes.len();
        }
        // Single-line string.
        let mut byte = start + 1;
        while byte < bytes.len() && bytes[byte] != quote && bytes[byte] != b'\n' {
            if quote == b'"' && bytes[byte] == b'\\' {
                byte += 1; // skip the escaped character
            }
            byte += 1;
        }
        // Include the closing quote when it was found.
        byte.min(bytes.len()) + usize::from(byte < bytes.len() && bytes[byte] == quote)
    }

    /// TOML: comments, strings, and value literals (numbers, booleans,
    /// dates). Keys stay default, matching the official query (it captures
    /// keys as `property`).
    fn toml_spans(source: &str) -> Vec<Span> {
        let bytes = source.as_bytes();
        let mut spans = Vec::new();
        let mut byte = 0;
        let mut after_equals = false;
        while byte < bytes.len() {
            let ch = bytes[byte];
            match ch {
                b'\n' => {
                    after_equals = false;
                    byte += 1;
                }
                b'#' => {
                    let start = byte;
                    while byte < bytes.len() && bytes[byte] != b'\n' {
                        byte += 1;
                    }
                    push(&mut spans, start, byte, SyntaxKind::Comment);
                }
                b'"' | b'\'' => {
                    let end = scan_toml_string(bytes, byte);
                    push(&mut spans, byte, end, SyntaxKind::String);
                    byte = end;
                }
                b'=' => {
                    after_equals = true;
                    byte += 1;
                }
                ch if ch.is_ascii_alphanumeric()
                    || matches!(ch, b'+' | b'-' | b'_' | b'.' | b':') =>
                {
                    // A word: a key before the `=` on its line, a value
                    // (number / boolean / date) after it.
                    let start = byte;
                    while byte < bytes.len()
                        && (bytes[byte].is_ascii_alphanumeric()
                            || matches!(bytes[byte], b'+' | b'-' | b'_' | b'.' | b':'))
                    {
                        byte += 1;
                    }
                    if after_equals {
                        push(&mut spans, start, byte, SyntaxKind::Number);
                    }
                }
                _ => byte += 1,
            }
        }
        spans
    }

    /// Markdown: headings and emphasis bright, code and link targets dim,
    /// block quotes faint+italic (the natural mono-palette reading of
    /// "quoted text"), list markers and rules muted.
    fn markdown_spans(source: &str) -> Vec<Span> {
        let mut spans = Vec::new();
        let mut in_fence = false;
        let mut paragraph: Option<(usize, usize)> = None;
        for (line_start, line) in source.split('\n').scan(0, |start, line| {
            let at = *start;
            *start += line.len() + 1;
            Some((at, line))
        }) {
            let line_end = line_start + line.len();
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();

            if in_fence {
                // Inside a fenced code block: everything is code until the
                // closing fence (a pure run of the same fence character).
                let fence_line = trimmed.trim_end();
                let is_close = fence_line.len() >= 3
                    && (fence_line.starts_with("```") || fence_line.starts_with("~~~"))
                    && fence_line
                        .chars()
                        .all(|ch| ch == fence_line.chars().next().unwrap_or('`'));
                if is_close {
                    in_fence = false;
                }
                push(&mut spans, line_start, line_end, SyntaxKind::String);
                continue;
            }

            // Fenced code opener: ``` or ~~~ (after up to 3 spaces).
            if indent <= 3 && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
                in_fence = true;
                push(&mut spans, line_start, line_end, SyntaxKind::String);
                continue;
            }
            // Indented code block (4+ spaces or a tab).
            if (indent >= 4 || line.starts_with('\t')) && !trimmed.is_empty() {
                push(&mut spans, line_start, line_end, SyntaxKind::String);
                paragraph = None;
                continue;
            }
            // ATX heading: `#`s at line start.
            if indent <= 3 && trimmed.starts_with('#') {
                let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
                if hashes <= 6 {
                    push(&mut spans, line_start, line_end, SyntaxKind::Keyword);
                    paragraph = None;
                    continue;
                }
            }
            // Block quote: `>` at line start (after up to 3 spaces).
            if indent <= 3 && trimmed.starts_with('>') {
                push(&mut spans, line_start, line_end, SyntaxKind::Comment);
                paragraph = None;
                continue;
            }
            // Setext underline: a line of `=` promotes the previous
            // paragraph line to a heading.
            if !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '=') {
                if let Some((start, end)) = paragraph.take() {
                    push(&mut spans, start, end, SyntaxKind::Keyword);
                }
                push(&mut spans, line_start, line_end, SyntaxKind::Keyword);
                continue;
            }
            // Thematic break: --- / *** / ___.
            if indent <= 3
                && !trimmed.is_empty()
                && matches!(trimmed.chars().next(), Some('-' | '*' | '_'))
                && trimmed
                    .chars()
                    .all(|ch| matches!(ch, '-' | '*' | '_' | ' '))
                && trimmed.trim_end().len() >= 3
            {
                push(&mut spans, line_start, line_end, SyntaxKind::Punctuation);
                paragraph = None;
                continue;
            }
            // List marker: -, +, *, or `N.` / `N)` before the item text.
            if indent <= 3 {
                let after_marker = trimmed
                    .strip_prefix(['-', '+', '*'])
                    .filter(|rest| rest.starts_with(' '))
                    .map(|rest| rest.len());
                let marker_len = after_marker.map(|rest| trimmed.len() - rest).or_else(|| {
                    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
                    let numbered = digits > 0
                        && trimmed[digits..].starts_with(['.', ')'])
                        && trimmed[digits + 1..].starts_with(' ');
                    numbered.then_some(digits + 1)
                });
                if let Some(marker_len) = marker_len {
                    push(
                        &mut spans,
                        line_start + indent,
                        line_start + indent + marker_len,
                        SyntaxKind::Punctuation,
                    );
                }
            }
            // Inline scopes on an ordinary line.
            inline_spans(&mut spans, line, line_start);
            paragraph = if trimmed.is_empty() {
                None
            } else {
                Some((line_start, line_end))
            };
        }
        spans.sort_unstable();
        spans
    }

    /// Inline markdown scopes within one line: code spans, emphasis pairs
    /// (incl. `**strong**`, `_em_`, `~~strike~~`), link destinations, and
    /// autolinks.
    fn inline_spans(spans: &mut Vec<Span>, line: &str, line_start: usize) {
        let bytes = line.as_bytes();
        let mut byte = 0;
        while byte < bytes.len() {
            let ch = bytes[byte];
            let run_len =
                |byte: usize, ch: u8| bytes[byte..].iter().take_while(|b| **b == ch).count();
            match ch {
                b'`' | b'*' | b'_' | b'~' => {
                    // A delimiter run pairs with the next run of the same
                    // character and (roughly) the same length.
                    let len = run_len(byte, ch).min(3);
                    let mut scan = byte + run_len(byte, ch);
                    let mut close = None;
                    while scan < bytes.len() {
                        if bytes[scan] == ch {
                            let found = run_len(scan, ch);
                            if found >= len {
                                close = Some(scan + len);
                                break;
                            }
                            scan += found;
                        } else {
                            scan += 1;
                        }
                    }
                    match close {
                        Some(close) => {
                            let kind = if ch == b'`' {
                                SyntaxKind::String
                            } else {
                                SyntaxKind::Keyword
                            };
                            push(spans, line_start + byte, line_start + close, kind);
                            byte = close;
                        }
                        None => byte += run_len(byte, ch),
                    }
                }
                b']' if bytes[byte..].starts_with(b"](") => {
                    // Link destination: the URL inside the parentheses.
                    let mut scan = byte + 2;
                    while scan < bytes.len() && bytes[scan] != b')' {
                        scan += 1;
                    }
                    let end = (scan + 1).min(bytes.len());
                    push(
                        spans,
                        line_start + byte + 1,
                        line_start + scan,
                        SyntaxKind::String,
                    );
                    byte = end;
                }
                b'<' => {
                    // Autolink / inline raw HTML tag: <…> with no spaces,
                    // closed on the same line. Prose like "a < b" fails the
                    // check and stays un-highlighted.
                    let mut scan = byte + 1;
                    let mut plain = false;
                    while scan < bytes.len() && bytes[scan] != b'>' {
                        plain |= bytes[scan] == b' ';
                        scan += 1;
                    }
                    if scan < bytes.len() && !plain && scan > byte + 1 {
                        let end = scan + 1;
                        push(
                            spans,
                            line_start + byte,
                            line_start + end,
                            SyntaxKind::Attribute,
                        );
                        byte = end;
                    } else {
                        byte += 1;
                    }
                }
                _ => byte += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind covering `byte` in `layer`'s source.
    fn kind_at(layer: &SyntaxLayer, byte: usize) -> SyntaxKind {
        layer
            .spans()
            .iter()
            .find(|span| (span.start as usize) <= byte && byte < span.end as usize)
            .map(|span| span.kind)
            .unwrap_or(SyntaxKind::Default)
    }

    fn layer_for(language: Language, source: &str) -> SyntaxLayer {
        let mut layer = SyntaxLayer::new(language).expect("grammar loads");
        layer.set_source(source);
        layer
    }

    #[test]
    fn rust_scopes_keyword_function_string_comment_number() {
        const SOURCE: &str =
            "pub fn theme() -> Theme {\n    // a comment\n    let s = \"str\";\n    let n = 42;\n}\n";
        let layer = layer_for(Language::Rust, SOURCE);

        // `pub fn` keyword light-gray, fn name pure white (ROADMAP 2d brief).
        assert_eq!(
            kind_at(&layer, SOURCE.find("pub").unwrap()),
            SyntaxKind::Keyword
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("fn").unwrap()),
            SyntaxKind::Keyword
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("theme").unwrap()),
            SyntaxKind::Function
        );
        // Return type dimmer than the name.
        assert_eq!(
            kind_at(&layer, SOURCE.find("Theme").unwrap()),
            SyntaxKind::Type
        );
        // Comment faint + italic.
        assert_eq!(
            kind_at(&layer, SOURCE.find("//").unwrap()),
            SyntaxKind::Comment
        );
        assert!(SyntaxKind::Comment.italics());
        assert!(!SyntaxKind::Keyword.italics());
        // Strings dim.
        assert_eq!(
            kind_at(&layer, SOURCE.find("str").unwrap()),
            SyntaxKind::String
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("42").unwrap()),
            SyntaxKind::Number
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("let").unwrap()),
            SyntaxKind::Keyword
        );
        // Plain text between spans stays default.
        assert_eq!(
            kind_at(&layer, SOURCE.find('\n').unwrap()),
            SyntaxKind::Default
        );
    }

    #[test]
    fn typescript_scopes_interface_function_comment() {
        const SOURCE: &str = "interface Point { x: number; }\nfunction area(p: Point): number {\n  return p.x;\n}\n// note\n";
        let layer = layer_for(Language::TypeScript, SOURCE);

        assert_eq!(
            kind_at(&layer, SOURCE.find("interface").unwrap()),
            SyntaxKind::Keyword
        );
        // `Point` after `interface` is a type, not a variable.
        assert_eq!(
            kind_at(&layer, SOURCE.find("Point").unwrap()),
            SyntaxKind::Type
        );
        // Function name white — the JS query contributes this capture.
        assert_eq!(
            kind_at(&layer, SOURCE.find("area").unwrap()),
            SyntaxKind::Function
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("number").unwrap()),
            SyntaxKind::Type
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("note").unwrap()),
            SyntaxKind::Comment
        );
    }

    #[test]
    fn python_scopes_def_string_comment_number() {
        const SOURCE: &str = "def main():\n    # a comment\n    s = \"hi\"\n    return 42\n";
        let layer = layer_for(Language::Python, SOURCE);

        assert_eq!(
            kind_at(&layer, SOURCE.find("def").unwrap()),
            SyntaxKind::Keyword
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("main").unwrap()),
            SyntaxKind::Function
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("comment").unwrap()),
            SyntaxKind::Comment
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("hi").unwrap()),
            SyntaxKind::String
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("42").unwrap()),
            SyntaxKind::Number
        );
    }

    #[test]
    fn toml_fallback_scopes() {
        const SOURCE: &str = "[package]\nname = \"cargo\"\nflag = true\ncount = 12\n# a comment\n";
        let layer = layer_for(Language::Toml, SOURCE);

        // The table header and the key stay default (the official query
        // captures keys as `property`); values are scoped.
        assert_eq!(
            kind_at(&layer, SOURCE.find("package").unwrap()),
            SyntaxKind::Default
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("cargo").unwrap()),
            SyntaxKind::String
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("true").unwrap()),
            SyntaxKind::Number
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("12").unwrap()),
            SyntaxKind::Number
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("comment").unwrap()),
            SyntaxKind::Comment
        );
    }

    #[test]
    fn markdown_fallback_scopes() {
        const SOURCE: &str = "# Heading\n\nplain *em* and `code`\n> quoted\n\n```\nfenced\n```\n";
        let layer = layer_for(Language::Markdown, SOURCE);

        assert_eq!(
            kind_at(&layer, SOURCE.find("Heading").unwrap()),
            SyntaxKind::Keyword
        );
        // Emphasis (markers included) reads as a keyword-bright run.
        assert_eq!(
            kind_at(&layer, SOURCE.find("*em*").unwrap() + 1),
            SyntaxKind::Keyword
        );
        assert_eq!(
            kind_at(&layer, SOURCE.find("code").unwrap()),
            SyntaxKind::String
        );
        // Block quotes are faint + italic, like comments.
        assert_eq!(
            kind_at(&layer, SOURCE.find("quoted").unwrap()),
            SyntaxKind::Comment
        );
        assert!(SyntaxKind::Comment.italics());
        // Fenced code block content (and the fences themselves) are string
        // dim — including on the line *after* the opening fence.
        assert_eq!(
            kind_at(&layer, SOURCE.find("fenced").unwrap()),
            SyntaxKind::String
        );
    }

    #[test]
    fn all_languages_load() {
        for language in [
            Language::Rust,
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Python,
            Language::Json,
            Language::Toml,
            Language::Markdown,
            Language::Html,
            Language::Css,
        ] {
            assert!(
                SyntaxLayer::new(language).is_some(),
                "{language:?} grammar failed to initialize"
            );
        }
    }

    #[test]
    fn language_detection_by_extension_and_path() {
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("TS"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::Tsx));
        assert_eq!(Language::from_extension("md"), Some(Language::Markdown));
        assert_eq!(Language::from_extension("xyz"), None);

        assert_eq!(
            Language::from_path(Path::new("C:\\repo\\crates\\main.rs")),
            Some(Language::Rust)
        );
        assert_eq!(
            Language::from_path(Path::new("src/config.TOML")),
            Some(Language::Toml)
        );
        assert_eq!(Language::from_path(Path::new("unknown.what")), None);
    }

    #[test]
    fn edit_updates_highlight_spans() {
        // Structural change: inserting "// " at the line start turns the
        // line into a comment, and the spans must reflect that.
        let source = "let x = 1;\nlet y = 2;\n";
        let mut layer = layer_for(Language::Rust, source);
        assert_eq!(
            kind_at(&layer, source.find("let").unwrap()),
            SyntaxKind::Keyword
        );
        assert_eq!(
            kind_at(&layer, source.find("1").unwrap()),
            SyntaxKind::Number
        );

        layer.edit(Position::new(0, 0), Position::new(0, 0), "// ", 1.0);
        let after = "// let x = 1;\nlet y = 2;\n";
        // The whole first line is now one comment span.
        assert_eq!(kind_at(&layer, 0), SyntaxKind::Comment);
        assert_eq!(
            kind_at(&layer, after.find("let").unwrap()),
            SyntaxKind::Comment
        );
        assert_eq!(
            kind_at(&layer, after.find("1").unwrap()),
            SyntaxKind::Comment
        );
        // The second line is untouched: still highlighted as code.
        assert_eq!(
            kind_at(&layer, after.find("let y").unwrap()),
            SyntaxKind::Keyword
        );
        assert_eq!(
            kind_at(&layer, after.find("2").unwrap()),
            SyntaxKind::Number
        );
    }

    #[test]
    fn edit_inside_a_string_extends_the_span_without_restructure() {
        // No structural change: typing inside the string body. The span
        // shift path must keep the string scope covering the typed char.
        let source = "let s = \"abc\";\n";
        let mut layer = layer_for(Language::Rust, source);
        let string_start = source.find("abc").unwrap();
        assert_eq!(kind_at(&layer, string_start), SyntaxKind::String);

        // Insert `d` just before the closing quote (col 12 is the `"`).
        layer.edit(Position::new(0, 12), Position::new(0, 12), "d", 1.0);
        assert_eq!(layer.char_count(), source.chars().count() + 1);
        // The string span (which includes both quotes) grew by one byte:
        // `"abc"` -> `"abcd"`.
        assert_eq!(
            layer
                .spans()
                .iter()
                .find(|span| span.kind == SyntaxKind::String)
                .map(|span| span.end),
            Some((string_start + 5) as u32)
        );
        // The keyword span before the string did not move.
        assert_eq!(kind_at(&layer, 0), SyntaxKind::Keyword);
    }

    #[test]
    fn line_sections_map_byte_spans_to_char_columns() {
        // Non-ASCII content: byte offsets and char columns diverge, so the
        // mapping must go through `char_indices`, not byte truncation.
        let source = "let s = \"héllo\"; // c\n";
        let layer = layer_for(Language::Rust, source);
        let sections = layer.line_sections(0, "let s = \"héllo\"; // c");
        assert!(!sections.is_empty());
        // The comment section starts at the `//` char column (17).
        let comment = sections
            .iter()
            .find(|section| section.kind == SyntaxKind::Comment)
            .expect("comment section exists");
        assert_eq!(comment.char_start, "let s = \"héllo\"; ".chars().count());
        assert_eq!(comment.char_end, "let s = \"héllo\"; // c".chars().count());
        // Every section is a valid non-degenerate char range.
        for section in &sections {
            assert!(section.char_start < section.char_end);
        }
    }

    #[test]
    fn multi_line_edits_keep_line_starts_consistent() {
        let mut layer = layer_for(Language::Rust, "let a = 1;\nlet b = 2;\n");
        assert_eq!(layer.line_count(), 3);

        // Join lines 0 and 1: delete the newline at the end of line 0
        // (positions (0,10) and (1,0) straddle it).
        layer.edit(Position::new(0, 10), Position::new(1, 0), "", 1.0);
        assert_eq!(layer.source(), "let a = 1;let b = 2;\n");
        assert_eq!(layer.line_count(), 2);
        assert!(!layer.line_sections(0, "let a = 1;let b = 2;").is_empty());

        // Split again: insert a newline after the `;` (col 10).
        layer.edit(Position::new(0, 10), Position::new(0, 10), "\n", 2.0);
        assert_eq!(layer.source(), "let a = 1;\nlet b = 2;\n");
        assert_eq!(layer.line_count(), 3);
    }

    #[test]
    fn large_file_reparse_debounce() {
        // Small buffers re-parse on every edit.
        assert!(reparse_due(10, 100.0, 99.9));
        // Large buffers only after the 500ms interval.
        assert!(!reparse_due(LARGE_FILE_LINES + 1, 100.2, 100.0));
        assert!(reparse_due(LARGE_FILE_LINES + 1, 100.6, 100.0));
        // The boundary itself is a small buffer.
        assert!(reparse_due(LARGE_FILE_LINES, 100.1, 100.0));
    }

    #[test]
    fn deferred_reparse_flushes_on_tick() {
        // A large buffer defers the re-parse; `tick` flushes it once the
        // debounce elapses. (Uses a small source for speed — the deferral
        // is decided by line count alone.)
        let big = "x\n".repeat(LARGE_FILE_LINES + 1);
        let mut layer = layer_for(Language::Rust, &big);
        let spans_before = layer.spans().len();

        // Typing inside line 0 at t=0.1: line count is over the threshold,
        // and last_parse is 0.0, so this edit is deferred.
        layer.edit(Position::new(0, 1), Position::new(0, 1), "y", 0.1);
        assert!(layer.is_dirty());
        assert_eq!(layer.spans().len(), spans_before, "no re-highlight yet");

        // Before the interval: still deferred.
        layer.tick(0.3);
        assert!(layer.is_dirty());

        // After the interval: flushed.
        layer.tick(0.6);
        assert!(!layer.is_dirty());
        assert_eq!(layer.source(), format!("xy{}", &big[1..]));
    }
}
