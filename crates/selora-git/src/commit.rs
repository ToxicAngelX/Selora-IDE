//! Commit history and commit inspection.
//!
//! [`GitRepo::log`] is a revwalk with an optional path filter, exactly the
//! daily "what changed, and what touched this file" view. [`GitRepo::show_commit`]
//! resolves one revision into its metadata, changed-file list with line stats,
//! and unified patch (bounded).

use crate::path::checked_relative;
use crate::{GitError, GitRepo};
use std::path::{Path, PathBuf};

/// How many log entries one request may return.
pub const MAX_LOG_LIMIT: usize = 500;
/// The patch returned by `show_commit` is capped so one huge commit cannot
/// make an event unbounded.
pub const MAX_PATCH_BYTES: usize = 1024 * 1024;

/// One commit's display metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    /// Full hex object id.
    pub id: String,
    /// First 7 digits, the panel's label.
    pub short_id: String,
    /// First line of the message.
    pub summary: String,
    /// Full message.
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    /// Author time, seconds since the Unix epoch.
    pub time_unix: i64,
    /// Author timezone offset in minutes east of UTC.
    pub time_offset_minutes: i32,
    /// Parent ids, in order.
    pub parents: Vec<String>,
}

impl CommitInfo {
    fn from_commit(commit: &git2::Commit<'_>) -> Self {
        let author = commit.author();
        Self {
            id: commit.id().to_string(),
            short_id: short(&commit.id()),
            summary: commit.summary().ok().flatten().unwrap_or("").to_owned(),
            message: commit.message().unwrap_or("").to_owned(),
            author_name: author.name().unwrap_or("").to_owned(),
            author_email: author.email().unwrap_or("").to_owned(),
            time_unix: commit.time().seconds(),
            time_offset_minutes: commit.time().offset_minutes(),
            parents: commit.parent_ids().map(|oid| oid.to_string()).collect(),
        }
    }
}

/// How a path changed in a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChange,
    Other,
}

impl ChangeKind {
    /// Stable wire/UI label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
            Self::TypeChange => "typechange",
            Self::Other => "other",
        }
    }
}

/// One changed file inside a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFileChange {
    /// New path (the old path for a deletion).
    pub path: PathBuf,
    /// The pre-image path when the file was renamed or copied.
    pub old_path: Option<PathBuf>,
    pub kind: ChangeKind,
    pub additions: usize,
    pub deletions: usize,
}

/// The answer to "show this commit".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitDetail {
    pub info: CommitInfo,
    pub files: Vec<CommitFileChange>,
    /// Unified patch (bounded by [`MAX_PATCH_BYTES`]).
    pub patch: String,
    /// Whether the patch was cut at the bound.
    pub patch_truncated: bool,
}

impl GitRepo {
    /// The most recent `limit` commits reachable from HEAD, newest first.
    /// With `path`, only commits whose diff against their first parent touches
    /// that path are returned (repo-relative or absolute inside the repo).
    pub fn log(&self, limit: usize, path: Option<&Path>) -> Result<Vec<CommitInfo>, GitError> {
        let limit = limit.min(MAX_LOG_LIMIT);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let rel = match path {
            Some(path) => Some(checked_relative(&self.workdir, path)?),
            None => None,
        };
        let mut revwalk = self.repo.revwalk()?;
        if revwalk.push_head().is_err() {
            // Unborn HEAD: there is no history yet, which is not an error.
            return Ok(Vec::new());
        }
        revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
        let mut out = Vec::new();
        for oid in revwalk {
            let commit = self.repo.find_commit(oid?)?;
            if let Some(rel) = &rel {
                let diff = self.commit_diff(&commit)?;
                let touches = diff.deltas().any(|delta| {
                    delta.new_file().path() == Some(rel.as_path())
                        || delta.old_file().path() == Some(rel.as_path())
                });
                if !touches {
                    continue;
                }
            }
            out.push(CommitInfo::from_commit(&commit));
            if out.len() == limit {
                break;
            }
        }
        Ok(out)
    }

    /// Resolve `rev` (a full/short oid, branch, tag, `HEAD~2`, …) and return
    /// its metadata, changed files, and patch. The patch is truncated at
    /// [`MAX_PATCH_BYTES`]; `patch_truncated` says so.
    pub fn show_commit(&self, rev: &str) -> Result<CommitDetail, GitError> {
        let rev = rev.trim();
        if rev.is_empty() {
            return Err(GitError::Other("commit is empty".to_owned()));
        }
        let object = self
            .repo
            .revparse_single(rev)
            .map_err(|_| GitError::Other(format!("`{rev}` does not resolve to a commit")))?;
        let commit = object
            .peel_to_commit()
            .map_err(|_| GitError::Other(format!("`{rev}` does not resolve to a commit")))?;
        let mut diff = self.commit_diff(&commit)?;
        let mut find = git2::DiffFindOptions::new();
        find.renames(true).copies(true);
        diff.find_similar(Some(&mut find))?;

        let mut files = Vec::new();
        let mut patch = String::new();
        let mut patch_truncated = false;
        for (index, delta) in diff.deltas().enumerate() {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let old_path = match delta.status() {
                git2::Delta::Renamed | git2::Delta::Copied => delta
                    .old_file()
                    .path()
                    .filter(|old| *old != path.as_path())
                    .map(Path::to_path_buf),
                _ => None,
            };
            let kind = change_kind(delta.status());
            let (additions, deletions) = match git2::Patch::from_diff(&diff, index)? {
                Some(mut file_patch) => {
                    let (_, additions, deletions) = file_patch.line_stats()?;
                    if !patch_truncated {
                        let text =
                            String::from_utf8_lossy(file_patch.to_buf()?.as_ref()).into_owned();
                        append_bounded(&mut patch, &text, &mut patch_truncated);
                    }
                    (additions, deletions)
                }
                None => (0, 0),
            };
            files.push(CommitFileChange {
                path,
                old_path,
                kind,
                additions,
                deletions,
            });
        }
        Ok(CommitDetail {
            info: CommitInfo::from_commit(&commit),
            files,
            patch,
            patch_truncated,
        })
    }

    fn commit_diff<'repo>(
        &'repo self,
        commit: &git2::Commit<'_>,
    ) -> Result<git2::Diff<'repo>, GitError> {
        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() == 0 {
            None
        } else {
            Some(commit.parent(0)?.tree()?)
        };
        let mut options = git2::DiffOptions::new();
        options.include_typechange(true);
        Ok(self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut options))?)
    }
}

fn short(oid: &git2::Oid) -> String {
    oid.to_string().chars().take(7).collect()
}

fn change_kind(delta: git2::Delta) -> ChangeKind {
    match delta {
        git2::Delta::Added => ChangeKind::Added,
        git2::Delta::Modified => ChangeKind::Modified,
        git2::Delta::Deleted => ChangeKind::Deleted,
        git2::Delta::Renamed => ChangeKind::Renamed,
        git2::Delta::Copied => ChangeKind::Copied,
        git2::Delta::Typechange => ChangeKind::TypeChange,
        _ => ChangeKind::Other,
    }
}

/// Append `text` until the patch bound. The cut happens on a char boundary so
/// the result is always valid UTF-8.
fn append_bounded(out: &mut String, text: &str, truncated: &mut bool) {
    let room = MAX_PATCH_BYTES.saturating_sub(out.len());
    if text.len() <= room {
        out.push_str(text);
        return;
    }
    let mut cut = room.min(text.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    out.push_str(&text[..cut]);
    *truncated = true;
}
