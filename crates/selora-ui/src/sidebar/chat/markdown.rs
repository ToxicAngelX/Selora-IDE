//! Markdown-lite for assistant messages (Phase R2).
//!
//! The chat renders assistant replies as structure, not walls of prose:
//! headings, bullet/ordered lists, block quotes, rules, fenced code
//! (carried over from the Phase 4c parser), and inline runs — `**strong**`
//! (Space Grotesk SemiBold; 700-weight bold is banned, DESIGN_TOKENS.md
//! §2.2), `*emphasis*` (Space Grotesk Medium; italics are banned, §2.5),
//! `` `code` `` (JetBrains Mono on `--ink-canvas`), and `[text](url)`
//! links (underlined, click opens the browser).
//!
//! The parser is deliberately small: no nesting, no tables, no reference
//! links — anything it does not recognize renders as plain text, never a
//! panic. A fenced code block that never closes (mid-stream) is an
//! incomplete block, rendered with the fence's language tag and no Copy
//! button.
//!
//! Nothing here paints: parsing produces [`Block`]s / [`Run`]s; the paint
//! site (chat `mod.rs`) turns them into egui layout jobs. The code-fence
//! language tag maps onto [`Language`] for tree-sitter highlighting
//! (§1.5's mono-tinted palette — code reads like structure).

use selora_editor::Language;

// -----------------------------------------------------------------------------
// The rendering model
// -----------------------------------------------------------------------------

/// One styled inline run of text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub style: RunStyle,
    /// The target for [`RunStyle::Link`].
    pub url: Option<String>,
}

/// The inline styles markdown-lite understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStyle {
    /// Plain prose: `--fg-default`, the UI sans.
    Plain,
    /// `**strong**` — Space Grotesk SemiBold (bold 700 is banned, §2.2).
    Strong,
    /// `*emphasis*` — Space Grotesk Medium (italics are banned, §2.5).
    Emphasis,
    /// `` `code` `` — JetBrains Mono on `--ink-canvas`.
    Code,
    /// `[text](url)` — underlined; a click opens the url.
    Link,
}

impl Run {
    /// A plain prose run (list markers use it too).
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: RunStyle::Plain,
            url: None,
        }
    }
}

/// One block of a message (block-level structure).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// A paragraph — consecutive non-blank lines joined with single
    /// spaces (markdown soft breaks).
    Paragraph { runs: Vec<Run> },
    /// `###` (level 3, `--type-xl`) / `####` (level 4, `--type-lg`)
    /// headings; `#`/`##` render as level 3 (chat never needs h1/h2).
    Heading { level: u8, runs: Vec<Run> },
    /// A consecutive group of `- ` / `N. ` lines.
    List { items: Vec<Vec<Run>>, ordered: bool },
    /// A consecutive group of `> ` lines (rendered fg-muted with a left
    /// rule).
    Quote { runs: Vec<Run> },
    /// `---` / `***` / `___` (3+): a 1px hairline.
    Rule,
    /// A fenced code block; `complete` is `false` while the closing fence
    /// has not arrived (mid-stream).
    Code {
        lang: Option<String>,
        body: String,
        complete: bool,
    },
}

// -----------------------------------------------------------------------------
// Block parsing
// -----------------------------------------------------------------------------

/// Parse a message's content into blocks. Malformed markdown degrades to
/// plain paragraphs — never an error.
pub fn parse(content: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    // The paragraph/list/quote group being accumulated (lines not yet
    // flushed into a block).
    let mut paragraph: Vec<String> = Vec::new();
    let mut list: Option<(bool, Vec<String>)> = None;
    let mut quote: Vec<String> = Vec::new();
    let flush = |paragraph: &mut Vec<String>,
                 list: &mut Option<(bool, Vec<String>)>,
                 quote: &mut Vec<String>,
                 blocks: &mut Vec<Block>| {
        flush_prose(paragraph, list, quote, blocks);
    };

    let mut offset = 0usize;
    let mut in_fence: Option<Option<String>> = None; // Some(lang) while open
    let mut code_body = String::new();
    for line in content.split_inclusive('\n') {
        let next = offset + line.len();
        let bare = line.strip_suffix('\n').unwrap_or(line);
        offset = next;

        // Fences: open or close a code block, regardless of any pending
        // prose (a fence interrupts everything, like GitHub).
        if let Some(rest) = bare.trim_start().strip_prefix("```") {
            match in_fence.take() {
                Some(lang) => {
                    flush(&mut paragraph, &mut list, &mut quote, &mut blocks);
                    blocks.push(Block::Code {
                        lang,
                        body: std::mem::take(&mut code_body),
                        complete: true,
                    });
                }
                None => {
                    flush(&mut paragraph, &mut list, &mut quote, &mut blocks);
                    let lang = rest.trim();
                    in_fence = Some(if lang.is_empty() {
                        None
                    } else {
                        Some(lang.to_owned())
                    });
                }
            }
            continue;
        }
        if in_fence.is_some() {
            code_body.push_str(bare);
            code_body.push('\n');
            continue;
        }

        // Blank line: ends the paragraph/list/quote group.
        if bare.trim().is_empty() {
            flush(&mut paragraph, &mut list, &mut quote, &mut blocks);
            continue;
        }

        // A heading line: `#### Title`.
        let heading_level = bare.chars().take_while(|c| *c == '#').count();
        if (1..=4).contains(&heading_level) {
            let rest = &bare[heading_level..];
            if rest.starts_with(' ') || rest.starts_with('\t') {
                flush(&mut paragraph, &mut list, &mut quote, &mut blocks);
                // # and ## render as ### (the type scale has no chat h1).
                let level = heading_level.clamp(3, 4) as u8;
                blocks.push(Block::Heading {
                    level,
                    runs: parse_inline(rest.trim()),
                });
                continue;
            }
        }

        // A rule: `---` / `***` / `___` (3+ of one marker, alone).
        if is_rule(bare) {
            flush(&mut paragraph, &mut list, &mut quote, &mut blocks);
            blocks.push(Block::Rule);
            continue;
        }

        // A list item: `- ` / `* ` / `+ ` (bullet) or `N. ` (ordered).
        if let Some(item) = bare
            .strip_prefix("- ")
            .or_else(|| bare.strip_prefix("* ").or_else(|| bare.strip_prefix("+ ")))
        {
            flush_prose(&mut paragraph, &mut None, &mut quote, &mut blocks);
            let entry = list.get_or_insert_with(|| (false, Vec::new()));
            entry.1.push(item.trim().to_owned());
            continue;
        }
        if let Some(rest) = ordered_item(bare) {
            flush_prose(&mut paragraph, &mut None, &mut quote, &mut blocks);
            let entry = list.get_or_insert_with(|| (true, Vec::new()));
            // A run of bullets interrupted by an ordered line flips the
            // group (never happens in sane markdown; degrade sanely).
            entry.0 = true;
            entry.1.push(rest.to_owned());
            continue;
        }

        // A quote line: `> text`.
        if let Some(text) = bare.strip_prefix('>') {
            flush_prose(&mut paragraph, &mut list, &mut Vec::new(), &mut blocks);
            quote.push(text.trim_start().to_owned());
            continue;
        }

        // Plain paragraph line (interrupts lists and quotes).
        flush_prose(&mut Vec::new(), &mut list, &mut quote, &mut blocks);
        paragraph.push(bare.trim().to_owned());
    }

    // EOF: flush, and an unterminated fence is an incomplete code block.
    flush(&mut paragraph, &mut list, &mut quote, &mut blocks);
    if let Some(lang) = in_fence {
        blocks.push(Block::Code {
            lang,
            body: code_body,
            complete: false,
        });
    }
    blocks
}

/// Flush the accumulated prose groups into blocks: the paragraph, the
/// list, and the quote, in that order. Callers pass `&mut Vec::new()` /
/// `&mut None` for the groups they want to keep accumulating.
fn flush_prose(
    paragraph: &mut Vec<String>,
    list: &mut Option<(bool, Vec<String>)>,
    quote: &mut Vec<String>,
    blocks: &mut Vec<Block>,
) {
    if !paragraph.is_empty() {
        let runs = parse_inline(&paragraph.join(" "));
        blocks.push(Block::Paragraph { runs });
        paragraph.clear();
    }
    if let Some((ordered, items)) = list.take() {
        if !items.is_empty() {
            blocks.push(Block::List {
                items: items.iter().map(|item| parse_inline(item)).collect(),
                ordered,
            });
        }
    }
    if !quote.is_empty() {
        let runs = parse_inline(&quote.join(" "));
        blocks.push(Block::Quote { runs });
        quote.clear();
    }
}

/// Whether a line is a thematic rule (`---` / `***` / `___`, 3+ alone).
fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    for marker in ['-', '*', '_'] {
        let run: String = std::iter::repeat_n(marker, 3).collect();
        if trimmed.starts_with(&run)
            && trimmed.chars().all(|c| c == marker || c == ' ')
            && trimmed.len() >= 3
        {
            return true;
        }
    }
    false
}

/// The item text of an ordered-list line (`3. text`), if it is one.
fn ordered_item(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    line.get(digits..digits + 2)
        .filter(|sep| *sep == ". ")
        .map(|_| {
            // The number itself is dropped; the renderer paints its own
            // ordinals (1. 2. 3.) so gaps in the source renumber cleanly.
            line[digits + 2..].trim()
        })
}

// -----------------------------------------------------------------------------
// Inline parsing
// -----------------------------------------------------------------------------

/// Parse one line of prose into styled runs. Unmatched markers stay
/// literal; runs never nest (the inner text of `**…**` is plain).
pub fn parse_inline(text: &str) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut plain = String::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < text.len() {
        // `` `code` `` — the earliest-closing backtick pair.
        if bytes[i] == b'`' {
            if let Some(close) = text[i + 1..].find('`') {
                if close > 0 {
                    push_plain(&mut runs, &mut plain);
                    runs.push(Run {
                        text: text[i + 1..i + 1 + close].to_owned(),
                        style: RunStyle::Code,
                        url: None,
                    });
                    i += close + 2;
                    continue;
                }
            }
        }
        // `**strong**` — a closing pair of asterisks ahead.
        if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'*') {
            if let Some(close) = find_marker(text, i + 2, "**") {
                if close > i + 2 {
                    push_plain(&mut runs, &mut plain);
                    runs.push(Run {
                        text: text[i + 2..close].to_owned(),
                        style: RunStyle::Strong,
                        url: None,
                    });
                    i = close + 2;
                    continue;
                }
            }
        }
        // `*emphasis*` — single asterisks (checked after `**`).
        if bytes[i] == b'*' {
            if let Some(close) = find_marker(text, i + 1, "*") {
                if close > i + 1 {
                    push_plain(&mut runs, &mut plain);
                    runs.push(Run {
                        text: text[i + 1..close].to_owned(),
                        style: RunStyle::Emphasis,
                        url: None,
                    });
                    i = close + 1;
                    continue;
                }
            }
        }
        // `[text](url)`.
        if bytes[i] == b'[' {
            if let Some(link) = parse_link(text, i) {
                push_plain(&mut runs, &mut plain);
                let end = link.end;
                runs.push(link.run);
                i = end;
                continue;
            }
        }
        // Plain byte: copy the whole char (multi-byte safe).
        let char_len = text[i..].chars().next().expect("in-bounds").len_utf8();
        plain.push_str(&text[i..i + char_len]);
        i += char_len;
    }
    push_plain(&mut runs, &mut plain);
    runs
}

/// Flush the pending plain-text buffer into a run.
fn push_plain(runs: &mut Vec<Run>, plain: &mut String) {
    if !plain.is_empty() {
        runs.push(Run::plain(std::mem::take(plain)));
    }
}

/// The byte index of the next `marker` at or after `from`, `None` if
/// absent (used by the `**`/`*` closers).
fn find_marker(text: &str, from: usize, marker: &str) -> Option<usize> {
    text.get(from..)?.find(marker).map(|found| from + found)
}

/// Parse a `[text](url)` starting at `open`; the returned run carries its
/// own end index (`None` if the shape does not close).
fn parse_link(text: &str, open: usize) -> Option<ParsedLink> {
    let close = text[open + 1..].find(']')? + open + 1;
    if text.get(close + 1..close + 2) != Some("(") {
        return None;
    }
    let url_end = text[close + 2..].find(')')? + close + 2;
    let label = &text[open + 1..close];
    let url = &text[close + 2..url_end];
    if label.is_empty() || url.is_empty() || url.contains(' ') {
        return None;
    }
    Some(ParsedLink {
        run: Run {
            text: label.to_owned(),
            style: RunStyle::Link,
            url: Some(url.to_owned()),
        },
        end: url_end + 1,
    })
}

/// The private link-parse carrier.
struct ParsedLink {
    run: Run,
    end: usize,
}

// -----------------------------------------------------------------------------
// Code fences -> highlighting languages
// -----------------------------------------------------------------------------

/// The tree-sitter language for a fence tag (full names first, then file
/// extensions; `None` = plain, unhighlighted).
pub fn fence_language(tag: &str) -> Option<Language> {
    match tag.to_ascii_lowercase().as_str() {
        "rust" => Some(Language::Rust),
        "python" | "py3" => Some(Language::Python),
        "typescript" => Some(Language::TypeScript),
        "javascript" => Some(Language::JavaScript),
        "tsx" => Some(Language::Tsx),
        _ => Language::from_extension(tag),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(text: &str) -> Vec<Run> {
        vec![Run::plain(text)]
    }

    fn strong(text: &str) -> Run {
        Run {
            text: text.into(),
            style: RunStyle::Strong,
            url: None,
        }
    }

    fn emphasis(text: &str) -> Run {
        Run {
            text: text.into(),
            style: RunStyle::Emphasis,
            url: None,
        }
    }

    fn code(text: &str) -> Run {
        Run {
            text: text.into(),
            style: RunStyle::Code,
            url: None,
        }
    }

    fn link(text: &str, url: &str) -> Run {
        Run {
            text: text.into(),
            style: RunStyle::Link,
            url: Some(url.into()),
        }
    }

    // -- inline ----------------------------------------------------------------

    #[test]
    fn plain_text_is_one_run() {
        assert_eq!(parse_inline("just text"), runs("just text"));
        assert_eq!(parse_inline(""), Vec::<Run>::new());
    }

    #[test]
    fn strong_and_emphasis_and_code() {
        assert_eq!(
            parse_inline("a **bold** b"),
            vec![Run::plain("a "), strong("bold"), Run::plain(" b"),]
        );
        assert_eq!(
            parse_inline("*em* tail"),
            vec![emphasis("em"), Run::plain(" tail")]
        );
        assert_eq!(
            parse_inline("call `foo()` now"),
            vec![Run::plain("call "), code("foo()"), Run::plain(" now")]
        );
    }

    #[test]
    fn unmatched_markers_stay_literal() {
        assert_eq!(parse_inline("2 * 3 = 6"), runs("2 * 3 = 6"));
        assert_eq!(parse_inline("a ** b"), runs("a ** b"));
        assert_eq!(parse_inline("`no close"), runs("`no close"));
        // Empty emphasis (`**`) is not emphasis.
        assert_eq!(parse_inline("a****b"), runs("a****b"));
    }

    #[test]
    fn links_parse_with_labels_and_urls() {
        assert_eq!(
            parse_inline("see [docs](https://selora.dev) for more"),
            vec![
                Run::plain("see "),
                link("docs", "https://selora.dev"),
                Run::plain(" for more"),
            ]
        );
        // A label-less or url-less shape stays literal.
        assert_eq!(parse_inline("[](x)"), runs("[](x)"));
        assert_eq!(parse_inline("[a](b c)"), runs("[a](b c)"));
    }

    #[test]
    fn multibyte_text_never_splits_chars() {
        assert_eq!(
            parse_inline("héllo **wörld** ✅"),
            vec![Run::plain("héllo "), strong("wörld"), Run::plain(" ✅")]
        );
    }

    // -- blocks ----------------------------------------------------------------

    #[test]
    fn paragraphs_join_soft_lines() {
        let blocks = parse("line one\nline two\n\nsecond para");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph {
                    runs: runs("line one line two")
                },
                Block::Paragraph {
                    runs: runs("second para")
                },
            ]
        );
    }

    #[test]
    fn headings_levels_and_lists() {
        let blocks = parse("## Title\n#### Sub\n- one\n- two\n\n1. first\n2. second");
        assert_eq!(
            blocks,
            vec![
                Block::Heading {
                    level: 3,
                    runs: runs("Title")
                },
                Block::Heading {
                    level: 4,
                    runs: runs("Sub")
                },
                Block::List {
                    items: vec![runs("one"), runs("two")],
                    ordered: false,
                },
                Block::List {
                    items: vec![runs("first"), runs("second")],
                    ordered: true,
                },
            ]
        );
    }

    #[test]
    fn quote_rule_and_fences() {
        let blocks = parse("> quoted\n> more\n\n---\n\n```rust\nfn main() {}\n```\n");
        assert_eq!(
            blocks,
            vec![
                Block::Quote {
                    runs: runs("quoted more")
                },
                Block::Rule,
                Block::Code {
                    lang: Some("rust".into()),
                    body: "fn main() {}\n".into(),
                    complete: true,
                },
            ]
        );
    }

    #[test]
    fn unterminated_fence_is_incomplete() {
        let blocks = parse("intro\n```python\nx = 1");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph {
                    runs: runs("intro")
                },
                Block::Code {
                    lang: Some("python".into()),
                    body: "x = 1\n".into(),
                    complete: false,
                },
            ]
        );
    }

    #[test]
    fn fence_interrupts_prose() {
        let blocks = parse("para\n```text\ncode\n```\ntail");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph { runs: runs("para") },
                Block::Code {
                    lang: Some("text".into()),
                    body: "code\n".into(),
                    complete: true,
                },
                Block::Paragraph { runs: runs("tail") },
            ]
        );
    }

    #[test]
    fn lists_interrupt_paragraphs() {
        let blocks = parse("intro\n- item");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph {
                    runs: runs("intro")
                },
                Block::List {
                    items: vec![runs("item")],
                    ordered: false,
                },
            ]
        );
    }

    // -- fences ----------------------------------------------------------------

    #[test]
    fn fence_tags_map_to_languages() {
        assert_eq!(fence_language("rust"), Some(Language::Rust));
        assert_eq!(fence_language("Python"), Some(Language::Python));
        assert_eq!(fence_language("ts"), Some(Language::TypeScript));
        assert_eq!(fence_language("json"), Some(Language::Json));
        assert_eq!(fence_language("text"), None);
        assert_eq!(fence_language(""), None);
    }
}
