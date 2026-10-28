//! Bulk index operations: stage everything, unstage everything.
//!
//! `stage_all` is `git add -A` over the reported status set (adds, edits and
//! deletions; ignored files stay out) and skips conflicted entries so a merge
//! is never accidentally flattened into a resolution.

use crate::{GitError, GitRepo};
use std::path::PathBuf;

impl GitRepo {
    /// Stage every working-tree change, including deletions and untracked
    /// files. Returns how many paths were written. Conflicted entries are left
    /// for an explicit ours/theirs resolution.
    pub fn stage_all(&self) -> Result<usize, GitError> {
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_head_to_index(true);
        let statuses = self.repo.statuses(Some(&mut options))?;
        let mut paths = Vec::new();
        for entry in statuses.iter() {
            if entry.status().contains(git2::Status::CONFLICTED) {
                continue;
            }
            if let Ok(path) = entry.path() {
                paths.push(PathBuf::from(path));
            }
        }
        let count = paths.len();
        let mut index = self.repo.index()?;
        for rel in paths {
            if self.workdir.join(&rel).exists() {
                index.add_path(&rel)?;
            } else {
                index.remove_path(&rel)?;
            }
        }
        index.write()?;
        Ok(count)
    }

    /// Reset the whole index to HEAD (`git reset` with no paths): staged edits
    /// become working-tree edits, staged new files become untracked, and staged
    /// deletions reappear as working-tree deletions. On an unborn HEAD the
    /// index is cleared. Returns how many paths carried a staged change.
    pub fn unstage_all(&self) -> Result<usize, GitError> {
        let staged = self
            .status()?
            .entries
            .iter()
            .filter(|entry| entry.is_staged())
            .count();
        match self.repo.revparse_single("HEAD") {
            Ok(head) => self.repo.reset(&head, git2::ResetType::Mixed, None)?,
            Err(_) => {
                // Unborn HEAD: there is nothing to reset to, so an empty
                // index is exactly "nothing staged".
                let mut index = self.repo.index()?;
                index.clear()?;
                index.write()?;
            }
        }
        Ok(staged)
    }
}
