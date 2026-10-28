//! Stash operations: push, list, pop, apply, drop.
//!
//! The stash is the daily "park this work" gesture. Push requires a
//! configured identity (the stash commit is a commit), and an empty stash is
//! reported as a precondition failure rather than a libgit2 noise message.
//!
//! libgit2's stash API takes `&mut Repository`; these methods open a short
//! live handle for the mutation, so a `&GitRepo` stays usable (the same
//! handle-per-effect pattern the app already uses).

use crate::{GitError, GitRepo};

/// One stash entry (`stash@{index}`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    pub index: usize,
    /// The full stash message ("WIP on main: …" or the caller's message).
    pub message: String,
    /// The stash commit id, full hex.
    pub oid: String,
    pub time_unix: i64,
}

impl GitRepo {
    /// Save the working tree + index as a new stash. Returns the new entry
    /// (`stash@{0}`). Untracked files are not included, matching
    /// `git stash push` without `-u`.
    pub fn stash_push(&self, message: Option<&str>) -> Result<StashEntry, GitError> {
        if !self.has_stashable_changes()? {
            return Err(GitError::Other("nothing to stash".to_owned()));
        }
        let mut repo = self.live_repo()?;
        let signature = repo.signature().map_err(|err| {
            GitError::Other(format!(
                "{err} — set user.name and user.email in your git config"
            ))
        })?;
        let message = message
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(|message| message.chars().take(500).collect::<String>());
        repo.stash_save(&signature, message.as_deref().unwrap_or("WIP"), None)?;
        self.stash_list()?
            .into_iter()
            .next()
            .ok_or_else(|| GitError::Other("the stash was saved but is not listed".to_owned()))
    }

    /// Every stash entry, newest first.
    pub fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        let mut repo = self.live_repo()?;
        let mut raw = Vec::new();
        repo.stash_foreach(|index, message, oid| {
            raw.push((index, message.to_owned(), *oid));
            true
        })?;
        let mut entries: Vec<StashEntry> = raw
            .into_iter()
            .map(|(index, message, oid)| {
                let time_unix = self
                    .repo
                    .find_commit(oid)
                    .map(|commit| commit.time().seconds())
                    .unwrap_or(0);
                StashEntry {
                    index,
                    message,
                    oid: oid.to_string(),
                    time_unix,
                }
            })
            .collect();
        entries.sort_by_key(|entry| entry.index);
        Ok(entries)
    }

    /// Apply `stash@{index}` and drop it. Conflicts are reported by libgit2
    /// and leave the stash in place.
    pub fn stash_pop(&self, index: usize) -> Result<(), GitError> {
        self.check_stash_index(index)?;
        self.live_repo()?.stash_pop(index, None)?;
        Ok(())
    }

    /// Apply `stash@{index}` but keep the entry.
    pub fn stash_apply(&self, index: usize) -> Result<(), GitError> {
        self.check_stash_index(index)?;
        self.live_repo()?.stash_apply(index, None)?;
        Ok(())
    }

    /// Drop `stash@{index}` without applying it.
    pub fn stash_drop(&self, index: usize) -> Result<(), GitError> {
        self.check_stash_index(index)?;
        self.live_repo()?.stash_drop(index)?;
        Ok(())
    }

    fn check_stash_index(&self, index: usize) -> Result<(), GitError> {
        let len = self.stash_list()?.len();
        if index >= len {
            return Err(GitError::Other(format!("stash@{{{index}}} does not exist")));
        }
        Ok(())
    }

    /// A second live handle on the same repository (libgit2 reuses its object
    /// cache, so this is cheap) for APIs that need `&mut`.
    fn live_repo(&self) -> Result<git2::Repository, GitError> {
        Ok(git2::Repository::open(self.repo.path())?)
    }

    fn has_stashable_changes(&self) -> Result<bool, GitError> {
        Ok(self.status()?.entries.iter().any(|entry| {
            entry.is_staged()
                || (!entry.is_untracked() && entry.worktree != crate::Slot::Unmodified)
        }))
    }
}
