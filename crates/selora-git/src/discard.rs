//! Discard local changes: restore working-tree files from the index, or move
//! index entries back to HEAD.
//!
//! Both directions are explicit destructive actions; every path is
//! containment-checked first, and a directory is never removed recursively.
//! Untracked file discarding removes that single file (the `git clean -f`
//! case) — the panel only ever passes file rows.

use crate::path::checked_relative;
use crate::{GitError, GitRepo};
use std::fs;
use std::path::{Path, PathBuf};

/// Which side of a changed file to discard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscardTarget {
    /// Working tree → index (`git restore -- <path>`).
    Worktree,
    /// Index → HEAD (`git restore --staged -- <path>`).
    Index,
}

impl GitRepo {
    /// Discard `paths` on the requested side. Returns how many paths were
    /// handled. Paths may be absolute (inside the repo) or repo-relative; an
    /// escaping path fails the whole call before anything is touched.
    pub fn discard(&self, paths: &[PathBuf], target: DiscardTarget) -> Result<usize, GitError> {
        let rels = self.checked_paths(paths)?;
        for rel in &rels {
            match target {
                DiscardTarget::Worktree => self.discard_worktree_one(rel)?,
                DiscardTarget::Index => {}
            }
        }
        if target == DiscardTarget::Index {
            if rels.is_empty() {
                return Ok(0);
            }
            let target = self.repo.revparse_single("HEAD").ok();
            let paths: Vec<String> = rels
                .iter()
                .map(|rel| rel.to_string_lossy().into_owned())
                .collect();
            self.repo.reset_default(target.as_ref(), paths)?;
        }
        Ok(rels.len())
    }

    /// Discard working-tree changes only (`git restore -- <paths>`).
    pub fn discard_worktree(&self, paths: &[PathBuf]) -> Result<usize, GitError> {
        self.discard(paths, DiscardTarget::Worktree)
    }

    /// Discard every local change to one path, on both sides: the index entry
    /// returns to HEAD and the working-tree file is rebuilt from it. A path
    /// that HEAD does not track (untracked, or newly staged) is removed from
    /// the index and deleted from disk — the panel's "discard all changes to
    /// this file" gesture. A conflicted path is refused until it is resolved,
    /// so a discard can never silently choose a merge side.
    pub fn discard_all(&self, path: &Path) -> Result<(), GitError> {
        let rel = checked_relative(&self.workdir, path)?;
        {
            let index = self.repo.index()?;
            if index.conflict_get(&rel).is_ok() {
                return Err(GitError::Other(format!(
                    "`{}` has an unresolved merge conflict; resolve it first",
                    rel.display()
                )));
            }
        }
        let head = self.repo.revparse_single("HEAD").ok();
        let tracked = head
            .as_ref()
            .and_then(|object| object.peel_to_tree().ok())
            .is_some_and(|tree| tree.get_path(&rel).is_ok());
        if tracked {
            // Index back to HEAD first, then the working tree from the index:
            // one content source, so both sides end at the committed text.
            self.repo
                .reset_default(head.as_ref(), [rel.to_string_lossy().into_owned()])?;
            let mut options = git2::build::CheckoutBuilder::new();
            options.force().path(&rel);
            self.repo.checkout_index(None, Some(&mut options))?;
            return Ok(());
        }
        {
            let mut index = self.repo.index()?;
            if index.get_path(&rel, 0).is_some() {
                index.remove_path(&rel)?;
                index.write()?;
            }
        }
        let abs = self.workdir.join(&rel);
        match fs::symlink_metadata(&abs) {
            Ok(meta) if meta.is_dir() => Err(GitError::Other(format!(
                "refusing to discard `{}`: it is a directory",
                rel.display()
            ))),
            Ok(_) => fs::remove_file(&abs).map_err(|err| {
                GitError::Other(format!("cannot discard `{}`: {err}", rel.display()))
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(GitError::Other(format!(
                "cannot discard `{}`: {err}",
                rel.display()
            ))),
        }
    }

    /// Discard staged changes only (`git restore --staged -- <paths>`).
    pub fn discard_index(&self, paths: &[PathBuf]) -> Result<usize, GitError> {
        self.discard(paths, DiscardTarget::Index)
    }

    fn checked_paths(&self, paths: &[PathBuf]) -> Result<Vec<PathBuf>, GitError> {
        paths
            .iter()
            .map(|path| checked_relative(&self.workdir, path))
            .collect()
    }

    fn discard_worktree_one(&self, rel: &Path) -> Result<(), GitError> {
        let index = self.repo.index()?;
        if index.conflict_get(rel).is_ok() {
            return Err(GitError::Other(format!(
                "`{}` has an unresolved merge conflict; resolve it first",
                rel.display()
            )));
        }
        let abs = self.workdir.join(rel);
        if index.get_path(rel, 0).is_some() {
            // Tracked: rebuild the file from the index (this is the discard).
            // Checkout paths are repo-relative pathspecs.
            let mut options = git2::build::CheckoutBuilder::new();
            options.force().path(rel);
            self.repo.checkout_index(None, Some(&mut options))?;
            return Ok(());
        }
        // Untracked: remove that one file, never a directory tree.
        match fs::symlink_metadata(&abs) {
            Ok(meta) if meta.is_dir() => Err(GitError::Other(format!(
                "refusing to discard `{}`: it is a directory",
                rel.display()
            ))),
            Ok(_) => {
                fs::remove_file(&abs).map_err(|err| {
                    GitError::Other(format!("cannot discard `{}`: {err}", rel.display()))
                })?;
                Ok(())
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(GitError::Other(format!(
                "cannot discard `{}`: {err}",
                rel.display()
            ))),
        }
    }
}
