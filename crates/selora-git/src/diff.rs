//! The working-tree-vs-index diff for one file (the git panel's diff view).
//!
//! Structured, not text: the shell can paint it in any read-only surface
//! later. Origin letters are git's own: `'+'` added, `'-'` removed,
//! `' '` context, `'\\'` "no newline at end of file".

use std::path::PathBuf;

/// The diff of a single file, working tree against the index (unstaged
/// changes only — staged changes vs HEAD are out of Lite scope).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// Repo-relative path with `/` separators.
    pub path: PathBuf,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    /// Whether any hunk has content.
    pub fn is_empty(&self) -> bool {
        self.hunks.iter().all(|hunk| hunk.lines.is_empty())
    }
}

/// One hunk: the `@@ -old_start,old_lines +new_start,new_lines @@` range
/// plus its lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

/// One diff line. `old_no` / `new_no` are the 1-based line numbers on the
/// respective side (`None` for added / removed / meta lines).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    /// `'+` `'-'` `' '` `'\\'`.
    pub origin: char,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// The line text without the trailing newline.
    pub content: String,
}
