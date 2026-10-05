//! Search-in-files (Phase 3c) — ARCHITECTURE.md §10.2, UI_SPEC.md §9.
//!
//! [`search_in_files`] answers "which lines in the workspace contain this
//! text?" with plain substring semantics (UI_SPEC.md §9: the `case` and
//! `word` toggles only — regex is v1.x). Two engines implement the same
//! contract:
//!
//! 1. **ripgrep, when an `rg` binary is on PATH** (ARCHITECTURE.md §10.2:
//!    we shell out because a full grep engine is a big dependency and
//!    the subprocess is ~40 LOC). `--fixed-strings` keeps the pattern
//!    literal; `--json` gives line-delimited match records that
//!    [`parse_rg_json`] reduces into [`SearchHit`]s. `rg` respects
//!    `.gitignore` and skips hidden and binary files natively.
//! 2. **The built-in line scanner** over the `ignore`-crate walker (the
//!    same `hidden` / `git_ignore` / `ignore` configuration as
//!    [`crate::tree::build_tree`]) when `rg` is absent. Slow but
//!    functional; per-line matching is [`find_matches`].
//!
//! Synchronous and pure like `build_tree`: the app shell calls it off the
//! frame path (`spawn_blocking` + `Effect::Search`, ARCHITECTURE.md §2.2)
//! and delivers the finished `Vec<SearchHit>` to the UI through
//! `AppMessage::SearchResults`.
//!
//! Match positions are **char columns** (not byte offsets): the panel
//! paints highlights from them, and the rg path converts its byte spans
//! via [`byte_span_to_char_span`]. `line_number` is 1-based, `span` is
//! half-open `[start, end)` over [`SearchHit::line_text`].

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use ignore::WalkBuilder;

/// Total hits per search cap. A monorepo-wide search for `fn` can produce
/// millions; the panel is not virtualized in Lite, so the engine stops
/// here. (Result streaming is a v2 concern.)
pub const MAX_HITS: usize = 10_000;

/// Files above this size are skipped by the built-in scanner (ripgrep
/// applies its own large-file and binary heuristics natively).
pub const MAX_SCANNED_FILE_BYTES: u64 = 1024 * 1024;

// -----------------------------------------------------------------------------
// Query / results types
// -----------------------------------------------------------------------------

/// Identity of one search request, used to drop stale results (a second
/// `Enter` before the first search's worker replies). Generated from a
/// process-wide counter — no `uuid` dependency needed in this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SearchId(u64);

impl SearchId {
    /// A fresh, never-before-returned id.
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        SearchId(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// The raw counter value (debugging, logging).
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// What to search for (UI_SPEC.md §9 query bar: query text plus the
/// `case` and `word` toggles — plain substring, never regex).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// The literal substring to find.
    pub pattern: String,
    /// `case` toggle (`Aa`): distinguish upper/lower case.
    pub case_sensitive: bool,
    /// `word` toggle (`\b`): match only at word boundaries.
    pub whole_word: bool,
}

impl SearchQuery {
    /// A case-insensitive, substring query for `pattern`.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            case_sensitive: false,
            whole_word: false,
        }
    }

    /// Set the `case` toggle.
    pub fn case_sensitive(mut self, on: bool) -> Self {
        self.case_sensitive = on;
        self
    }

    /// Set the `word` toggle.
    pub fn whole_word(mut self, on: bool) -> Self {
        self.whole_word = on;
        self
    }

    /// Whether the pattern is empty (an empty pattern matches nothing).
    pub fn is_empty(&self) -> bool {
        self.pattern.is_empty()
    }
}

/// One matched substring inside a [`SearchHit::line_text`]: half-open char
/// columns `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchSpan {
    /// First matched char column (0-based).
    pub start: usize,
    /// One past the last matched char column.
    pub end: usize,
}

/// One matching line (ARCHITECTURE.md: "path, line number, line text,
/// match column spans").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// The file the line belongs to.
    pub path: PathBuf,
    /// 1-based line number.
    pub line_number: usize,
    /// The line's text (trailing `\n` / `\r\n` trimmed).
    pub line_text: String,
    /// Every occurrence on the line, left to right.
    pub spans: Vec<MatchSpan>,
}

/// Error type for the search engine.
///
/// Same crate-local shape as [`crate::tree::FsError`]: `selora-fs` does
/// not depend on `selora-core` (Cargo.toml is orchestrator-owned), so the
/// variants map one-to-one onto `SeloraError::Io` / `SeloraError::Other`.
#[derive(Debug)]
pub enum SearchError {
    /// A filesystem operation failed (spawning `rg`, reading a file).
    Io(io::Error),
    /// `rg` ran and failed (bad arguments, unreadable root, …).
    Rg(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchError::Io(err) => write!(f, "io error: {err}"),
            SearchError::Rg(message) => write!(f, "ripgrep failed: {message}"),
        }
    }
}

impl std::error::Error for SearchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SearchError::Io(err) => Some(err),
            SearchError::Rg(_) => None,
        }
    }
}

impl From<io::Error> for SearchError {
    fn from(err: io::Error) -> Self {
        SearchError::Io(err)
    }
}

// -----------------------------------------------------------------------------
// search_in_files
// -----------------------------------------------------------------------------

/// Search `root` for `query` (ARCHITECTURE.md §10.2).
///
/// Shells out to `rg` when one is on PATH, otherwise uses the built-in
/// scanner. `.gitignore` is respected and hidden entries are skipped on
/// both paths. An empty pattern returns no hits. Results are capped at
/// [`MAX_HITS`].
pub fn search_in_files(root: &Path, query: &SearchQuery) -> Result<Vec<SearchHit>, SearchError> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    if rg_on_path() {
        run_rg(root, query)
    } else {
        Ok(search_builtin(root, query))
    }
}

/// Whether an `rg` binary resolves on PATH (probed once per process).
fn rg_on_path() -> bool {
    static RG: OnceLock<bool> = OnceLock::new();
    *RG.get_or_init(|| {
        Command::new("rg")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

/// Run `rg` in JSON mode and reduce its output to hits.
fn run_rg(root: &Path, query: &SearchQuery) -> Result<Vec<SearchHit>, SearchError> {
    let mut command = Command::new("rg");
    command
        .arg("--json") // line-delimited JSON events
        .arg("--fixed-strings") // the pattern is literal (UI_SPEC.md §9)
        .arg("--no-config"); // never surprising RIPGREP_CONFIG_PATH behavior
    if !query.case_sensitive {
        command.arg("--ignore-case");
    }
    if query.whole_word {
        command.arg("--word-regexp");
    }
    command
        .arg("--") // the pattern is never parsed as a flag
        .arg(&query.pattern)
        .arg(root);

    let output = command.output()?;
    let status = output.status;
    // rg exits 1 for "no matches" (a normal outcome); anything other than
    // 0/1 is a real failure to surface (no silent fallback — PLAN.md §8).
    if !status.success() && status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SearchError::Rg(format!(
            "exit {}: {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into()),
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut hits = parse_rg_json(&stdout);
    hits.truncate(MAX_HITS);
    Ok(hits)
}

// -----------------------------------------------------------------------------
// The built-in scanner (rg absent)
// -----------------------------------------------------------------------------

/// Line-scan the whole tree under `root` with the `ignore` walker.
///
/// The walker mirrors [`crate::tree::build_tree`]: hidden entries and
/// `.gitignore`/`.ignore` rules are respected (with `require_git(false)`
/// so a plain folder behaves like a workspace). Files over
/// [`MAX_SCANNED_FILE_BYTES`] or containing NUL bytes (a cheap binary
/// heuristic) are skipped, like `rg` skips them.
fn search_builtin(root: &Path, query: &SearchQuery) -> Vec<SearchHit> {
    let needle: Vec<char> = if query.case_sensitive {
        query.pattern.chars().collect()
    } else {
        query.pattern.chars().map(fold_char).collect()
    };
    if needle.is_empty() {
        return Vec::new();
    }

    let mut hits: Vec<SearchHit> = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .ignore(true)
        .build();

    for entry in walker.flatten() {
        if hits.len() >= MAX_HITS {
            break;
        }
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.len() > MAX_SCANNED_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue; // unreadable (locked, permissions): skip, not fail
        };
        if bytes.contains(&0) {
            continue; // binary
        }
        let text = String::from_utf8_lossy(&bytes);
        for (index, line) in text.lines().enumerate() {
            let spans = find_matches(line, &needle, query.case_sensitive, query.whole_word);
            if !spans.is_empty() {
                hits.push(SearchHit {
                    path: path.to_path_buf(),
                    line_number: index + 1,
                    line_text: line.to_owned(),
                    spans,
                });
                if hits.len() >= MAX_HITS {
                    break;
                }
            }
        }
    }
    hits
}

/// Single-char case fold that preserves char count (and therefore column
/// indices): a char lowercases to exactly one char, or stays itself.
/// Multi-char expansions (`İ` → `i̇`) are left unfolded — matching them
/// case-insensitively is out of Lite scope.
fn fold_char(c: char) -> char {
    let mut folded = c.to_lowercase();
    let first = folded.next().unwrap_or(c);
    if folded.next().is_none() {
        first
    } else {
        c
    }
}

/// Word characters for the `word` toggle: alphanumeric plus `_` (the same
/// class a regex `\w` uses).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Find every occurrence of `needle` in `line`, returning half-open char
/// spans. `needle` must already carry the caller's case folding; pass the
/// original `case_sensitive` flag so an unfolded needle still matches
/// exactly.
fn find_matches(
    line: &str,
    needle: &[char],
    case_sensitive: bool,
    whole_word: bool,
) -> Vec<MatchSpan> {
    let haystack: Vec<char> = if case_sensitive {
        line.chars().collect()
    } else {
        line.chars().map(fold_char).collect()
    };
    let needle_len = needle.len();
    if needle_len == 0 || haystack.len() < needle_len {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut at = 0;
    while at + needle_len <= haystack.len() {
        if &haystack[at..at + needle_len] == needle {
            let boundary_ok = !whole_word
                || (at == 0 || !is_word_char(haystack[at - 1]))
                    && (at + needle_len == haystack.len()
                        || !is_word_char(haystack[at + needle_len]));
            if boundary_ok {
                spans.push(MatchSpan {
                    start: at,
                    end: at + needle_len,
                });
                at += needle_len;
            } else {
                at += 1;
            }
        } else {
            at += 1;
        }
    }
    spans
}

// -----------------------------------------------------------------------------
// rg JSON output
// -----------------------------------------------------------------------------

/// Reduce rg's `--json` output stream (one JSON object per line) into
/// hits. Malformed lines, `begin`/`end`/`summary`/`context` events, and
/// non-UTF-8 records (rg base64-encodes those in a `bytes` field instead
/// of `text`) are skipped — they cannot be displayed as text lines.
fn parse_rg_json(output: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = Json::parse(line) else {
            continue;
        };
        let Some(record) = value.as_object() else {
            continue;
        };
        if lookup(record, "type").and_then(Json::as_str) != Some("match") {
            continue;
        }
        let Some(data) = lookup(record, "data").and_then(Json::as_object) else {
            continue;
        };
        let Some(path) = json_text(lookup(data, "path")) else {
            continue;
        };
        let Some(line_text) = json_text(lookup(data, "lines")) else {
            continue;
        };
        let Some(line_number) = lookup(data, "line_number")
            .and_then(Json::as_u64)
            .and_then(|n| usize::try_from(n).ok())
        else {
            continue;
        };
        let Some(submatches) = lookup(data, "submatches").and_then(Json::as_array) else {
            continue;
        };

        // Display text: trim the trailing newline rg includes in `lines`.
        let line_text = line_text.trim_end_matches(['\r', '\n']);
        let mut spans = Vec::new();
        for submatch in submatches {
            let Some(fields) = submatch.as_object() else {
                continue;
            };
            let (Some(start), Some(end)) = (
                lookup(fields, "start").and_then(Json::as_u64),
                lookup(fields, "end").and_then(Json::as_u64),
            ) else {
                continue;
            };
            if let Some(span) = byte_span_to_char_span(line_text, start, end) {
                spans.push(span);
            }
        }
        if spans.is_empty() {
            continue; // a match whose spans we cannot place is useless
        }
        hits.push(SearchHit {
            path: PathBuf::from(path),
            line_number,
            line_text: line_text.to_owned(),
            spans,
        });
    }
    hits
}

/// The `{"text": "…"}` payload of an rg JSON field, when the value is
/// text (rg uses `{"bytes": "<base64>"}` for non-UTF-8 data, which we
/// skip).
fn json_text(value: Option<&Json>) -> Option<&str> {
    value
        .and_then(Json::as_object)
        .and_then(|fields| lookup(fields, "text"))
        .and_then(Json::as_str)
}

/// Convert a byte offset in `line` to a char index; `None` when the
/// offset is out of bounds or lands mid-char.
fn byte_to_char(line: &str, byte: usize) -> Option<usize> {
    if byte > line.len() {
        return None;
    }
    let mut chars = 0;
    for (offset, _) in line.char_indices() {
        match offset.cmp(&byte) {
            std::cmp::Ordering::Equal => return Some(chars),
            std::cmp::Ordering::Greater => return None, // mid-char boundary
            std::cmp::Ordering::Less => chars += 1,
        }
    }
    (byte == line.len()).then_some(chars)
}

/// Convert rg's byte span over `line` into a char span.
fn byte_span_to_char_span(line: &str, start: u64, end: u64) -> Option<MatchSpan> {
    let start = usize::try_from(start).ok()?;
    let end = usize::try_from(end).ok()?;
    if end <= start {
        return None;
    }
    let start = byte_to_char(line, start)?;
    let end = byte_to_char(line, end)?;
    (end > start).then_some(MatchSpan { start, end })
}

// -----------------------------------------------------------------------------
// A minimal JSON parser (rg --json records only)
// -----------------------------------------------------------------------------

/// A parsed JSON value. Just enough for [`parse_rg_json`]: no numbers
/// beyond `f64`, no streaming, no `serde` (the crate's dependencies are
/// orchestrator-owned; rg's records are a fixed, tiny shape).
#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn parse(src: &str) -> Result<Json, String> {
        let mut parser = JsonParser {
            src: src.as_bytes(),
            at: 0,
        };
        let value = parser.value()?;
        parser.skip_ws();
        if parser.at != parser.src.len() {
            return Err(format!("trailing input at byte {}", parser.at));
        }
        Ok(value)
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Number(n) if *n >= 0.0 && n.fract() == 0.0 => Some(*n as u64),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(fields) => Some(fields),
            _ => None,
        }
    }
}

/// Look up `key` in a parsed object (linear scan; rg records have a
/// handful of fields).
fn lookup<'a>(fields: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

/// Recursive-descent parser over the raw bytes of a `&str` (all JSON
/// structural syntax is ASCII, so byte indexing is safe; string contents
/// are validated as UTF-8 when assembled).
struct JsonParser<'a> {
    src: &'a [u8],
    at: usize,
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while let Some(&b) = self.src.get(self.at) {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.at).copied()
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected `{}` at byte {}", byte as char, self.at))
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.src[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(format!("expected `{word}` at byte {}", self.at))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'n') => self.literal("null", Json::Null),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'"') => {
                self.at += 1;
                self.string_body().map(Json::String)
            }
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            other => Err(format!("unexpected {other:?} at byte {}", self.at)),
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(format!("expected `,` or `]` at byte {}", self.at)),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_ws();
            self.expect(b'"')?;
            let key = self.string_body()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value()?;
            fields.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(format!("expected `,` or `}}` at byte {}", self.at)),
            }
        }
    }

    /// Parse a string body after the opening `"` (consumes the closing
    /// `"`). Escapes: `\" \\ \/ \b \f \n \r \t \uXXXX` (with surrogate
    /// pairs); an unpaired surrogate becomes U+FFFD.
    fn string_body(&mut self) -> Result<String, String> {
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&b) = self.src.get(self.at) else {
                return Err("unterminated string".to_owned());
            };
            match b {
                b'"' => {
                    self.at += 1;
                    return String::from_utf8(out)
                        .map_err(|_| "invalid UTF-8 in string".to_owned());
                }
                b'\\' => {
                    self.at += 1;
                    let Some(&escape) = self.src.get(self.at) else {
                        return Err("unterminated escape".to_owned());
                    };
                    self.at += 1;
                    match escape {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let first = self.hex4()?;
                            let code = if (0xD800..=0xDBFF).contains(&first) {
                                // High surrogate: must pair with \uDC00-\uDFFF.
                                if self.src.get(self.at) == Some(&b'\\')
                                    && self.src.get(self.at + 1) == Some(&b'u')
                                {
                                    self.at += 2;
                                    let second = self.hex4()?;
                                    if (0xDC00..=0xDFFF).contains(&second) {
                                        0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                                    } else {
                                        u32::MAX // invalid → replacement char below
                                    }
                                } else {
                                    u32::MAX
                                }
                            } else {
                                first
                            };
                            let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => {
                            return Err(format!("bad escape `\\{}`", other as char));
                        }
                    }
                }
                0x00..=0x1F => return Err("control character in string".to_owned()),
                _ => {
                    out.push(b);
                    self.at += 1;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(&b) = self.src.get(self.at) else {
                return Err("truncated \\u escape".to_owned());
            };
            let digit = (b as char)
                .to_digit(16)
                .ok_or_else(|| format!("bad hex digit `{}`", b as char))?;
            value = value * 16 + digit;
            self.at += 1;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        while self.peek().is_some_and(|b| {
            b.is_ascii_digit() || b == b'.' || b == b'e' || b == b'E' || b == b'+' || b == b'-'
        }) {
            self.at += 1;
        }
        let text = &self.src[start..self.at];
        let text = std::str::from_utf8(text).map_err(|_| "bad number".to_owned())?;
        text.parse::<f64>()
            .map(Json::Number)
            .map_err(|_| format!("bad number `{text}`"))
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A unique fixture directory under the system temp dir.
    fn temp_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = format!(
            "selora-fs-search-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).expect("create fixture root");
        dir
    }

    fn write(root: &Path, rel: &str, contents: &str) -> PathBuf {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }
        std::fs::write(&path, contents).expect("write fixture file");
        path
    }

    /// Checked-in `rg --json` output (two files, three matches, a
    /// non-`match` event, and a non-ASCII line whose byte spans differ
    /// from its char spans). Stand-in for a live rg run; the
    /// rg-on-PATH integration test covers the live shape.
    const RG_FIXTURE: &str = concat!(
        r#"{"type":"begin","data":{"path":{"text":"src/editor/buffer.rs"}}}"#,
        "\n",
        r#"{"type":"match","data":{"path":{"text":"src/editor/buffer.rs"},"lines":{"text":"pub fn insert(&mut self, pos: Position, text: &str) {\n"},"line_number":42,"absolute_offset":1123,"submatches":[{"match":{"text":"insert"},"start":7,"end":13}]}}"#,
        "\n",
        r#"{"type":"context","data":{"path":{"text":"src/editor/buffer.rs"},"lines":{"text":"}\n"},"line_number":43,"absolute_offset":1169}}"#,
        "\n",
        r#"{"type":"match","data":{"path":{"text":"src/editor/buffer.rs"},"lines":{"text":"    pub fn line_col(&self, pos: Position) -> (Line, Col) {\n"},"line_number":88,"absolute_offset":2981,"submatches":[{"match":{"text":"pos"},"start":28,"end":31}]}}"#,
        "\n",
        r#"{"type":"end","data":{"path":{"text":"src/editor/buffer.rs"},"binary_offset":null,"stats":{"deadlines":{"total":0,"met":0,"missed":0}}}}"#,
        "\n",
        r#"{"type":"match","data":{"path":{"text":"src/ui/theme.rs"},"lines":{"text":"let s = \"föhn\"; // insert here\n"},"line_number":12,"absolute_offset":55,"submatches":[{"match":{"text":"insert"},"start":20,"end":26}]}}"#,
        "\n",
        r#"{"type":"match","data":{"path":{"text":"src/ui/theme.rs"},"lines":{"text":"pub const INK_CANVAS: Color32 = Color32::from_rgb(0, 0, 0);\n"},"line_number":13,"absolute_offset":91,"submatches":[{"match":{"text":"Color32"},"start":21,"end":28},{"match":{"text":"Color32"},"start":31,"end":38}]}}"#,
        "\n",
        r#"{"type":"summary","data":{"elapsed_total":{"secs":0,"nanos":4,"human":"0s"},"stats":{}}}"#,
        "\n",
    );

    fn spans_of(hit: &SearchHit) -> Vec<(usize, usize)> {
        hit.spans
            .iter()
            .map(|span| (span.start, span.end))
            .collect()
    }

    // -- find_matches (the built-in matcher) ---------------------------------

    #[test]
    fn find_matches_case_insensitive_by_default() {
        // Case-insensitive: "Rope" folds to "rope" and matches.
        let needle: Vec<char> = "rope".chars().map(fold_char).collect();
        assert_eq!(
            spans_of_hit(find_matches("use ropey::Rope;", &needle, false, false)).len(),
            2
        );
        // Case-sensitive: only the exact case matches.
        let needle: Vec<char> = "rope".chars().collect();
        assert_eq!(
            spans_of_hit(find_matches("use ropey::Rope;", &needle, true, false)),
            vec![(4, 8)]
        );
    }

    /// Helper: map spans into (start, end) tuples.
    fn spans_of_hit(spans: Vec<MatchSpan>) -> Vec<(usize, usize)> {
        spans_of(&SearchHit {
            path: PathBuf::new(),
            line_number: 0,
            line_text: String::new(),
            spans,
        })
    }

    #[test]
    fn find_matches_case_sensitive_and_word_modes() {
        let folded: Vec<char> = "insert".chars().map(fold_char).collect();
        let exact: Vec<char> = "insert".chars().collect();

        // Case-sensitive: "Insert" does not match.
        assert!(find_matches("pub fn Insert()", &exact, true, false).is_empty());
        // Case-insensitive: it does.
        assert_eq!(
            spans_of_hit(find_matches("pub fn Insert()", &folded, false, false)),
            vec![(7, 13)]
        );

        // Word mode: "insertion" is not a word match…
        assert!(find_matches("fn insertion()", &folded, false, true).is_empty());
        // …but "insert" bounded by punctuation is.
        assert_eq!(
            spans_of_hit(find_matches("(insert)", &folded, false, true)),
            vec![(1, 7)]
        );
        // Underscore counts as a word character.
        assert!(find_matches("fn insert_x()", &folded, false, true).is_empty());
    }

    #[test]
    fn find_matches_reports_every_occurrence_without_overlap() {
        let needle: Vec<char> = "aa".chars().collect();
        // "aaaa" → two non-overlapping matches.
        assert_eq!(
            spans_of_hit(find_matches("aaaa", &needle, true, false)),
            vec![(0, 2), (2, 4)]
        );
        let needle: Vec<char> = "ab".chars().collect();
        assert_eq!(
            spans_of_hit(find_matches("abab", &needle, true, false)),
            vec![(0, 2), (2, 4)]
        );
    }

    // -- search_builtin (the rg-absent path) ---------------------------------

    #[test]
    fn builtin_finds_lines_with_spans_and_line_numbers() {
        let root = temp_root("basic");
        write(
            &root,
            "src/buffer.rs",
            "fn main() {}\nlet rope = ropey::Rope::new();\nlet other = rope.clone();\n",
        );
        write(&root, "notes.md", "ropey is a rope crate\n");

        let hits = search_builtin(&root, &SearchQuery::new("rope"));
        assert_eq!(hits.len(), 3, "hits: {hits:?}");

        let buffer: Vec<&SearchHit> = hits
            .iter()
            .filter(|hit| hit.path.ends_with("src/buffer.rs"))
            .collect();
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer[0].line_number, 2);
        assert_eq!(buffer[0].line_text, "let rope = ropey::Rope::new();");
        // Case-insensitive: "rope", the "rope" inside "ropey", and "Rope".
        assert_eq!(spans_of(buffer[0]), vec![(4, 8), (11, 15), (18, 22)]);
        assert_eq!(buffer[1].line_number, 3);
        assert_eq!(spans_of(buffer[1]), vec![(12, 16)]);

        let notes = hits
            .iter()
            .find(|hit| hit.path.ends_with("notes.md"))
            .unwrap();
        assert_eq!(notes.line_number, 1);
        // "rope" inside "ropey" and the standalone "rope".
        assert_eq!(spans_of(notes), vec![(0, 4), (11, 15)]);
    }

    #[test]
    fn builtin_respects_case_and_word_toggles() {
        let root = temp_root("toggles");
        write(&root, "a.txt", "Rope rope roperope\nrope_rope\n");

        // Case-insensitive, substring (default): lines 1 and 2.
        assert_eq!(search_builtin(&root, &SearchQuery::new("rope")).len(), 2);

        // Case-sensitive: only the lowercase hits — "Rope" at column 0 is
        // not reported, but the line's other hits are.
        let case = search_builtin(&root, &SearchQuery::new("rope").case_sensitive(true));
        assert_eq!(case.len(), 2);
        assert_eq!(spans_of(&case[0]), vec![(5, 9), (10, 14), (14, 18)]);

        // Word mode: "roperope" and "rope_rope" are not word matches; the
        // two standalone words on line 1 are ("Rope" is case-folded).
        let word = search_builtin(&root, &SearchQuery::new("rope").whole_word(true));
        assert_eq!(word.len(), 1);
        assert_eq!(spans_of(&word[0]), vec![(0, 4), (5, 9)]);
    }

    #[test]
    fn builtin_respects_gitignore_and_hidden_entries() {
        let root = temp_root("gitignore");
        write(&root, ".gitignore", "ignored.txt\nhidden_dir/\n");
        write(&root, "kept.txt", "ropey here\n");
        write(&root, "ignored.txt", "ropey ignored\n");
        write(&root, "hidden_dir/inner.txt", "ropey hidden\n");
        write(&root, ".hidden.txt", "ropey dotfile\n");

        let hits = search_builtin(&root, &SearchQuery::new("ropey"));
        assert_eq!(hits.len(), 1, "hits: {hits:?}");
        assert!(hits[0].path.ends_with("kept.txt"));
    }

    #[test]
    fn builtin_skips_binary_and_oversize_files() {
        let root = temp_root("binary");
        write(&root, "binary.bin", "ropey\x00ropey");
        let hits = search_builtin(&root, &SearchQuery::new("ropey"));
        assert!(hits.is_empty(), "NUL-containing file must be skipped");

        let big = root.join("big.txt");
        let file = std::fs::File::create(&big).expect("create big");
        file.set_len(MAX_SCANNED_FILE_BYTES + 1).expect("grow big");
        drop(file);
        let hits = search_builtin(&root, &SearchQuery::new("ropey"));
        assert!(hits.is_empty(), "oversize file must be skipped");
    }

    #[test]
    fn builtin_crlf_lines_and_unicode_columns() {
        let root = temp_root("crlf");
        write(&root, "win.txt", "f\u{f6}hn ropey\r\nplain ropey\r\n");

        let hits = search_builtin(&root, &SearchQuery::new("ropey"));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line_text, "f\u{f6}hn ropey");
        assert_eq!(spans_of(&hits[0]), vec![(5, 10)], "char columns, not bytes");
        assert_eq!(hits[1].line_number, 2);
    }

    // -- search_in_files (engine selection) ----------------------------------

    #[test]
    fn empty_pattern_returns_no_hits_on_both_engines() {
        let root = temp_root("empty");
        write(&root, "a.txt", "anything");
        assert!(search_in_files(&root, &SearchQuery::new(""))
            .unwrap()
            .is_empty());
        assert!(search_builtin(&root, &SearchQuery::new("")).is_empty());
    }

    #[test]
    fn search_ids_are_unique() {
        let a = SearchId::next();
        let b = SearchId::next();
        assert_ne!(a, b);
        assert!(b.as_u64() > a.as_u64());
    }

    // -- parse_rg_json (checked-in fixture output) ---------------------------

    #[test]
    fn parse_rg_fixture_reduces_to_hits_with_char_spans() {
        let hits = parse_rg_json(RG_FIXTURE);
        assert_eq!(hits.len(), 4, "hits: {hits:?}");

        let buffer: Vec<&SearchHit> = hits
            .iter()
            .filter(|hit| hit.path == *Path::new("src/editor/buffer.rs"))
            .collect();
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer[0].line_number, 42);
        assert_eq!(
            buffer[0].line_text,
            "pub fn insert(&mut self, pos: Position, text: &str) {"
        );
        assert_eq!(spans_of(buffer[0]), vec![(7, 13)]);
        assert_eq!(buffer[1].line_number, 88);
        assert_eq!(spans_of(buffer[1]), vec![(28, 31)]);

        let theme: Vec<&SearchHit> = hits
            .iter()
            .filter(|hit| hit.path == *Path::new("src/ui/theme.rs"))
            .collect();
        assert_eq!(theme.len(), 2);
        // Trailing newline trimmed from the display text.
        assert_eq!(theme[0].line_text, "let s = \"f\u{f6}hn\"; // insert here");
        // byte span (20, 26) over "föhn…" is char span (19, 25).
        assert_eq!(spans_of(theme[0]), vec![(19, 25)]);
        assert_eq!(spans_of(theme[1]), vec![(21, 28), (31, 38)]);
    }

    #[test]
    fn parse_rg_json_skips_junk_and_bytes_encoded_records() {
        let output = concat!(
            "not json\n",
            "\n",
            r#"{"type":"match","data":{"path":{"bytes":"aGk="},"lines":{"text":"hi\n"},"line_number":1,"submatches":[{"match":{"text":"hi"},"start":0,"end":2}]}}"#,
            "\n",
            r#"{"type":"match","data":{"path":{"text":"ok.rs"},"lines":{"bytes":"aGk="},"line_number":1,"submatches":[{"match":{"text":"hi"},"start":0,"end":2}]}}"#,
            "\n",
        );
        assert!(parse_rg_json(output).is_empty());
    }

    #[test]
    fn byte_span_conversion_rejects_mid_char_and_reversed_spans() {
        let line = "f\u{f6}hn"; // ö is 2 bytes: char 1 spans bytes 1-2
        assert_eq!(byte_to_char(line, 0), Some(0));
        assert_eq!(byte_to_char(line, 1), Some(1), "start of ö");
        assert_eq!(byte_to_char(line, 2), None, "mid-char offset");
        assert_eq!(byte_to_char(line, 3), Some(2));
        assert_eq!(byte_to_char(line, 5), Some(4), "end of line");
        assert!(byte_to_char(line, 9).is_none(), "out of bounds");
        assert_eq!(
            byte_span_to_char_span(line, 0, 5),
            Some(MatchSpan { start: 0, end: 4 })
        );
        assert!(byte_span_to_char_span(line, 3, 1).is_none(), "reversed");
        assert!(
            byte_span_to_char_span(line, 2, 4).is_none(),
            "mid-char start"
        );
    }

    // -- the JSON parser itself -----------------------------------------------

    #[test]
    fn json_parser_handles_the_full_grammar() {
        let value = Json::parse(r#"{"a":[1,-2.5e2,true,false,null,"x\"yDé"],"b":"\\/\b\f\n\r\t"}"#)
            .expect("parse");
        let fields = value.as_object().expect("object");
        let array = lookup(fields, "a").and_then(Json::as_array).expect("array");
        assert_eq!(array[0], Json::Number(1.0));
        assert_eq!(array[1], Json::Number(-250.0));
        assert_eq!(array[2], Json::Bool(true));
        assert_eq!(array[3], Json::Bool(false));
        assert_eq!(array[4], Json::Null);
        assert_eq!(
            lookup(fields, "a").unwrap().as_array().unwrap()[5],
            Json::String("x\"yD\u{e9}".to_owned())
        );
        assert_eq!(
            lookup(fields, "b").and_then(Json::as_str),
            Some("\\/\u{8}\u{c}\n\r\t")
        );

        assert!(Json::parse("{\"a\":1} extra").is_err());
        assert!(Json::parse("[1,]").is_err());
        assert!(Json::parse("{\"a\" 1}").is_err());
        assert!(Json::parse("\"unterminated").is_err());
    }

    #[test]
    fn json_parser_pairs_surrogates_and_replaces_lonely_ones() {
        // U+1F600 = 😀 = surrogates D83D DE00.
        let value = Json::parse(r#""😀""#).expect("parse");
        assert_eq!(value.as_str(), Some("\u{1F600}"));
        // A lone high surrogate becomes U+FFFD, not a panic.
        let value = Json::parse(r#""\ud83d""#).expect("parse");
        assert_eq!(value.as_str(), Some("\u{FFFD}"));
    }

    // -- rg integration (only when rg is actually on PATH) --------------------

    #[test]
    fn rg_path_matches_the_builtin_scanner_when_rg_is_available() {
        if !rg_on_path() {
            // rg is an optional PATH binary (ARCHITECTURE.md §10.2); on
            // machines without it the fallback above is the engine under
            // test and the checked-in fixture covers the parsing path.
            eprintln!("skipping: no `rg` on PATH");
            return;
        }
        let root = temp_root("rg");
        write(
            &root,
            "src/buffer.rs",
            "fn main() {}\nlet rope = ropey::Rope::new();\nlet other = rope.clone();\n",
        );
        write(&root, "notes.md", "ropey is a rope crate\n");
        write(&root, "ignored.txt", "ropey ignored\n");
        write(&root, ".gitignore", "ignored.txt\n");

        let query = SearchQuery::new("rope");
        let via_rg = run_rg(&root, &query).expect("rg run");
        let via_builtin = search_builtin(&root, &query);

        let key = |hit: &SearchHit| {
            (
                hit.path.clone(),
                hit.line_number,
                hit.line_text.clone(),
                spans_of(hit),
            )
        };
        let mut rg_sorted: Vec<_> = via_rg.iter().map(key).collect();
        let mut builtin_sorted: Vec<_> = via_builtin.iter().map(key).collect();
        rg_sorted.sort();
        builtin_sorted.sort();
        assert_eq!(rg_sorted, builtin_sorted);
    }

    // -- search_in_files end-to-end (whichever engine is live) ----------------

    #[test]
    fn search_in_files_finds_workspace_hits() {
        let root = temp_root("e2e");
        write(&root, "lib.rs", "mod rope_holder;\n");
        let hits = search_in_files(&root, &SearchQuery::new("rope_holder")).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line_number, 1);
        assert_eq!(spans_of(&hits[0]), vec![(4, 15)]);
    }

    #[test]
    fn search_error_display_is_informative() {
        let err = SearchError::Rg("exit 2: permission denied".into());
        assert!(err.to_string().contains("ripgrep failed"));
        let err: SearchError = std::io::Error::other("boom").into();
        assert!(err.to_string().contains("io error"));
    }
}
