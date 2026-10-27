//! Fuzzy matching via `nucleo-matcher` (Phase 2c).
//!
//! The matching engine behind the `Ctrl+P` fuzzy file finder
//! (ARCHITECTURE.md §10.2, UI_SPEC.md §5.1). [`fuzzy_find`] takes the
//! needle the user typed plus the candidate files — supplied by the caller
//! (the file tree / workspace walker; `selora_fs::tree` is a sibling phase
//! and is deliberately not imported here) — and returns ranked matches.
//!
//! Contract:
//! - Matching runs over the item's path (normalized to `/` separators, so a
//!   Windows `\`-separated path matches the way the user types it).
//! - The needle is matched as a `nucleo-matcher` fuzzy pattern: subsequence,
//!   not regex, not substring-only.
//! - Results are ranked by score (descending) and, for equal scores, by path
//!   (ascending) so ordering is deterministic across runs and platforms.
//! - Empty needle → empty result (the palette shows nothing until typing).
//! - At most [`MAX_RESULTS`] (50) matches are returned.

use std::path::{Path, PathBuf};

use nucleo_matcher::{Matcher, Utf32Str};

/// Maximum number of ranked matches returned (ARCHITECTURE.md §10.2).
pub const MAX_RESULTS: usize = 50;

/// A ranked match for one candidate file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The original candidate path, as passed in (separators untouched).
    pub path: PathBuf,
    /// The category tag passed alongside the path (UI_SPEC.md §5.1 right
    /// column; e.g. the parent directory name).
    pub tag: String,
    /// The `nucleo-matcher` score. Higher is better.
    pub score: u16,
}

/// Fuzzy-find `needle` among `items` (path + category tag).
///
/// Empty needle → empty result. Results are capped at [`MAX_RESULTS`].
pub fn fuzzy_find(needle: &str, items: &[(PathBuf, String)]) -> Vec<Match> {
    if needle.is_empty() || items.is_empty() {
        return Vec::new();
    }

    // One matcher (and one scratch buffer) for the whole pass: the matcher
    // allocates ~135 KB of scratch up front and is meant to be reused
    // (nucleo-matcher docs).
    let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);

    // The needle stays alive across the whole loop, so it owns its buffer.
    let mut needle_chars = Vec::new();
    let needle = Utf32Str::new(needle, &mut needle_chars);

    let mut haystack_chars = Vec::new();
    let mut matches = items
        .iter()
        .filter_map(|(path, tag)| {
            let display = display_path(path);
            let haystack = Utf32Str::new(&display, &mut haystack_chars);
            let score = matcher.fuzzy_match(haystack, needle)?;
            Some(Match {
                path: path.clone(),
                tag: tag.clone(),
                score,
            })
        })
        .collect::<Vec<Match>>();

    // Deterministic ranking: score desc, then path asc (a stable total
    // order over equal scores, independent of input order or hash noise).
    matches.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.tag.cmp(&b.tag))
    });
    matches.truncate(MAX_RESULTS);
    matches
}

/// The string a path is matched and displayed as: lossy, with `\`
/// separators normalized to `/` so matching and display agree regardless of
/// platform (the user types `/`, not `\`).
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(paths: &[&str]) -> Vec<(PathBuf, String)> {
        paths
            .iter()
            .map(|p| (PathBuf::from(p), String::from("tag")))
            .collect()
    }

    fn paths_of(matches: &[Match]) -> Vec<String> {
        matches.iter().map(|m| display_path(&m.path)).collect()
    }

    #[test]
    fn empty_needle_returns_empty() {
        let items = items(&["src/main.rs", "src/theme.rs"]);
        assert!(fuzzy_find("", &items).is_empty());
    }

    #[test]
    fn empty_items_returns_empty() {
        assert!(fuzzy_find("main", &[]).is_empty());
    }

    #[test]
    fn non_matching_needle_excluded() {
        let items = items(&["src/main.rs", "src/theme.rs"]);
        assert!(fuzzy_find("zzz", &items).is_empty());
    }

    #[test]
    fn exact_substring_ranks_above_scattered_subsequence() {
        let items = items(&["src/other.rs", "src/t_h_e_m_e.rs", "src/theme.rs"]);
        let matches = fuzzy_find("theme", &items);
        assert_eq!(matches.len(), 2);
        // The contiguous substring match must be ranked first.
        assert_eq!(paths_of(&matches)[0], "src/theme.rs");
        assert!(matches[0].score > matches[1].score);
    }

    #[test]
    fn subsequence_match_finds_theme_rs() {
        let items = items(&["src/main.rs", "src/lib.rs", "src/theme.rs"]);
        let matches = fuzzy_find("thm", &items);
        assert!(paths_of(&matches).contains(&"src/theme.rs".to_string()));
        // "thm" is not a subsequence of the other two.
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn ranking_is_deterministic() {
        let input = items(&[
            "src/theme.rs",
            "src/tamed.rs",
            "src/theme_ui.rs",
            "src/main.rs",
        ]);
        let first = fuzzy_find("them", &input);
        let second = fuzzy_find("them", &input);
        assert_eq!(first, second);

        // Equal-score ties fall back to path order, not input order.
        let tied = items(&["b/a.rs", "a/a.rs", "c/a.rs"]);
        let matches = fuzzy_find("a", &tied);
        assert_eq!(paths_of(&matches), vec!["a/a.rs", "b/a.rs", "c/a.rs"]);
    }

    #[test]
    fn results_capped_at_50() {
        let paths: Vec<(PathBuf, String)> = (0..60)
            .map(|i| {
                (
                    PathBuf::from(format!("src/file_{i:02}.rs")),
                    String::from("tag"),
                )
            })
            .collect();
        let matches = fuzzy_find("file", &paths);
        assert_eq!(matches.len(), MAX_RESULTS);
        // The top 50 ranked (deterministically: highest scores first, then
        // path ascending) — with near-equal scores the cap keeps the lowest
        // path-sorted tail out.
        assert!(paths_of(&matches).contains(&"src/file_00.rs".to_string()));
        assert!(!paths_of(&matches).contains(&"src/file_59.rs".to_string()));
    }

    #[test]
    fn windows_separators_are_normalized_for_matching() {
        let items = vec![(PathBuf::from("src\\editor\\mod.rs"), String::from("editor"))];
        let matches = fuzzy_find("src/ed", &items);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].tag, "editor");
        // The returned path is the original, untouched.
        assert_eq!(matches[0].path, PathBuf::from("src\\editor\\mod.rs"));
    }
}
