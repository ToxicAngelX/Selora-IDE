//! The status snapshot the git panel and file-tree decorations consume.
//!
//! [`GitStatus`] is a plain value type: the shell stores the latest
//! snapshot in `AppState`, the git panel paints it, and the file tree maps
//! entries onto tree rows by absolute path. Nothing here touches a
//! repository — [`GitRepo`](crate::GitRepo) builds it.

use std::path::PathBuf;

/// One side (index / working tree) of a file's status.
///
/// A file can have changes in both slots at once: staged edits
/// (`staged != Unmodified`) with *further* unstaged edits on top
/// (`worktree != Unmodified`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Slot {
    /// No change on this side.
    #[default]
    Unmodified,
    /// New on this side (staged add / untracked in the working tree).
    New,
    Modified,
    Deleted,
    Renamed,
}

impl Slot {
    /// The single status letter (UI_SPEC.md §8): `A` `M` `D` `R`, a space
    /// for "nothing on this side".
    pub fn letter(self) -> char {
        match self {
            Self::Unmodified => ' ',
            Self::New => 'A',
            Self::Modified => 'M',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
        }
    }

    pub(crate) fn from_index(flags: git2::Status) -> Self {
        if flags.contains(git2::Status::INDEX_NEW) {
            Self::New
        } else if flags.contains(git2::Status::INDEX_DELETED) {
            Self::Deleted
        } else if flags.contains(git2::Status::INDEX_RENAMED) {
            Self::Renamed
        } else if flags.contains(git2::Status::INDEX_MODIFIED)
            || flags.contains(git2::Status::INDEX_TYPECHANGE)
        {
            Self::Modified
        } else {
            Self::Unmodified
        }
    }

    pub(crate) fn from_worktree(flags: git2::Status) -> Self {
        if flags.contains(git2::Status::WT_NEW) {
            Self::New
        } else if flags.contains(git2::Status::WT_DELETED) {
            Self::Deleted
        } else if flags.contains(git2::Status::WT_RENAMED) {
            Self::Renamed
        } else if flags.contains(git2::Status::WT_MODIFIED)
            || flags.contains(git2::Status::WT_TYPECHANGE)
        {
            Self::Modified
        } else {
            Self::Unmodified
        }
    }
}

/// One changed file, with its index ("staged") and working-tree
/// ("unstaged") sides split so the panel can offer per-row
/// stage/unstage and gate the commit button on staged changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    /// Repo-relative path with `/` separators, exactly as git reports it.
    /// This is the display form the panel shows.
    pub path: PathBuf,
    /// The same path joined onto the repository work directory — the join
    /// key for file-tree rows and the argument for stage / unstage / open.
    pub abs_path: PathBuf,
    /// The index (staged) side.
    pub staged: Slot,
    /// The working-tree (unstaged) side.
    pub worktree: Slot,
}

impl FileStatus {
    /// An untracked file: nothing in the index, new in the working tree.
    pub fn is_untracked(&self) -> bool {
        self.staged == Slot::Unmodified && self.worktree == Slot::New
    }

    /// Staged changes exist for this file (the commit button's condition).
    pub fn is_staged(&self) -> bool {
        self.staged != Slot::Unmodified
    }

    /// Unstaged changes exist for this file (the `+` button's condition;
    /// true for untracked files too).
    pub fn has_worktree_changes(&self) -> bool {
        self.is_untracked() || self.worktree != Slot::Unmodified
    }

    /// The single display letter (UI_SPEC.md §8 uses `M` `A` `?` `D`):
    /// `?` for untracked, otherwise the staged letter when there are
    /// staged changes, else the working-tree letter.
    pub fn letter(&self) -> char {
        if self.is_untracked() {
            '?'
        } else if self.staged != Slot::Unmodified {
            self.staged.letter()
        } else {
            self.worktree.letter()
        }
    }
}

/// The repository status snapshot (ARCHITECTURE.md §2.1
/// `AppMessage::GitStatusChanged(GitStatus)`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// The checked-out branch, or `None` when HEAD cannot be resolved
    /// (should not happen for normal workdirs; detached HEAD reports the
    /// short commit id in parentheses).
    pub branch: Option<String>,
    /// Every file with any change, sorted by path.
    pub entries: Vec<FileStatus>,
    /// Unresolved merge/rebase conflicts, sorted by path, as absolute paths under
    /// the work directory (the `entries` key form). A conflicted path has no
    /// `entries` row (its index slots are stages 1/2/3, not 0/1), so this is
    /// the only place the panel learns about it.
    pub conflicts: Vec<PathBuf>,
}

impl GitStatus {
    /// How many files have any change (staged or not) — the header's
    /// "N changed" count.
    pub fn changed_count(&self) -> usize {
        self.entries.len() + self.conflicts.len()
    }

    /// How many files have staged changes — the commit gate.
    pub fn staged_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.is_staged())
            .count()
    }

    /// Whether anything at all is dirty.
    pub fn is_dirty(&self) -> bool {
        !self.entries.is_empty() || !self.conflicts.is_empty()
    }
}
