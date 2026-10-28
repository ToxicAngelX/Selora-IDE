//! Merge with explicit conflict entries, plus ours/theirs resolution.
//!
//! `merge_branch` never commits: a fast-forward updates the branch, a true
//! merge writes the index and `MERGE_HEAD` and reports whether conflicts remain
//! ([`MergeStatus::Conflicts`]). The caller resolves each path with
//! [`GitRepo::resolve_conflict`] and finishes with the ordinary commit call,
//! which picks up `MERGE_HEAD` as the second parent.

use crate::path::checked_relative;
use crate::{GitError, GitRepo};
use std::fs;
use std::path::{Path, PathBuf};

/// Which side of a merge conflict to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictSide {
    Ours,
    Theirs,
}

/// One unresolved index conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictEntry {
    /// Repo-relative path with `/` separators.
    pub path: PathBuf,
    /// Whether our side has this path (false = we deleted it).
    pub ours: bool,
    /// Whether their side has this path (false = they deleted it).
    pub theirs: bool,
    /// Whether a common ancestor has this path.
    pub ancestor: bool,
}

/// Result of merging one branch into the current HEAD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeStatus {
    /// Both branches already point at the same history.
    UpToDate,
    /// The branch was moved forward without a merge commit.
    FastForward,
    /// A merge commit is staged and ready for `commit`.
    Merged,
    /// The merge is staged but at least one path conflicts.
    Conflicts,
}

impl MergeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UpToDate => "up_to_date",
            Self::FastForward => "fast_forward",
            Self::Merged => "merged",
            Self::Conflicts => "conflicts",
        }
    }
}

/// The typed answer to a merge request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeOutcome {
    pub status: MergeStatus,
    /// Non-empty exactly when `status` is [`MergeStatus::Conflicts`].
    pub conflicts: Vec<ConflictEntry>,
}

impl GitRepo {
    /// Merge the local branch `branch` into the current HEAD.
    pub fn merge_branch(&self, branch: &str) -> Result<MergeOutcome, GitError> {
        crate::branch::validate_branch_name(branch)?;
        let refname = format!("refs/heads/{branch}");
        let target = self
            .repo
            .revparse_single(&refname)
            .map_err(|_| GitError::Other(format!("branch `{branch}` does not exist")))?;
        let target = target.peel_to_commit()?;

        // Unborn HEAD: there is no merge base and nothing to lose; adopt the
        // other branch's history on the current (unborn) branch.
        if self.repo.head().is_err() {
            self.checkout_commit(&target)?;
            let head_ref = self
                .repo
                .find_reference("HEAD")
                .ok()
                .and_then(|head| head.symbolic_target().ok().flatten().map(str::to_owned))
                .ok_or_else(|| {
                    GitError::Other("HEAD is unborn and has no symbolic target".to_owned())
                })?;
            self.repo
                .reference(&head_ref, target.id(), true, "merge: fast-forward")?;
            self.repo.set_head(&head_ref)?;
            return Ok(MergeOutcome {
                status: MergeStatus::FastForward,
                conflicts: Vec::new(),
            });
        }

        let annotated = self.repo.find_annotated_commit(target.id())?;
        let (analysis, _preference) = self.repo.merge_analysis(&[&annotated])?;

        if analysis.is_up_to_date() {
            return Ok(MergeOutcome {
                status: MergeStatus::UpToDate,
                conflicts: Vec::new(),
            });
        }
        if analysis.is_fast_forward() {
            self.checkout_commit(&target)?;
            self.update_head_ref(target.id())?;
            return Ok(MergeOutcome {
                status: MergeStatus::FastForward,
                conflicts: Vec::new(),
            });
        }

        self.repo.merge(&[&annotated], None, None)?;
        let conflicts = self.merge_conflicts()?;
        Ok(MergeOutcome {
            status: if conflicts.is_empty() {
                MergeStatus::Merged
            } else {
                MergeStatus::Conflicts
            },
            conflicts,
        })
    }

    /// Alias for [`Self::merge_branch`] (the daily-git command name).
    pub fn merge(&self, branch: &str) -> Result<MergeOutcome, GitError> {
        self.merge_branch(branch)
    }

    /// The current unresolved conflicts in the index, sorted by path.
    pub fn merge_conflicts(&self) -> Result<Vec<ConflictEntry>, GitError> {
        let index = self.repo.index()?;
        let mut out = Vec::new();
        for conflict in index.conflicts()? {
            let conflict = conflict?;
            let path = conflict
                .our
                .as_ref()
                .or(conflict.their.as_ref())
                .or(conflict.ancestor.as_ref())
                .map(|entry| PathBuf::from(String::from_utf8_lossy(&entry.path).into_owned()))
                .unwrap_or_default();
            out.push(ConflictEntry {
                path,
                ours: conflict.our.is_some(),
                theirs: conflict.their.is_some(),
                ancestor: conflict.ancestor.is_some(),
            });
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// Keep `side`'s version of one conflicted path: write that blob to the
    /// working tree and stage it as resolved. A deletion on the chosen side
    /// removes the file. Returns the conflicts that remain.
    pub fn resolve_conflict(
        &self,
        path: &Path,
        side: ConflictSide,
    ) -> Result<Vec<ConflictEntry>, GitError> {
        let rel = checked_relative(&self.workdir, path)?;
        let mut index = self.repo.index()?;
        let conflict = index
            .conflict_get(&rel)
            .map_err(|_| GitError::Other(format!("`{}` has no merge conflict", rel.display())))?;
        let chosen = match side {
            ConflictSide::Ours => conflict.our,
            ConflictSide::Theirs => conflict.their,
        };
        let abs = self.workdir.join(&rel);
        match chosen {
            Some(mut entry) => {
                let blob = self.repo.find_blob(entry.id)?;
                if let Some(parent) = abs.parent() {
                    fs::create_dir_all(parent).map_err(|err| {
                        GitError::Other(format!("cannot create `{}`: {err}", rel.display()))
                    })?;
                }
                fs::write(&abs, blob.content()).map_err(|err| {
                    GitError::Other(format!("cannot write `{}`: {err}", rel.display()))
                })?;
                // Drop the conflict stages first: `IndexEntry` stage 0 must be
                // the only entry left for this path, otherwise libgit2 still
                // refuses to write a tree ("not fully merged index").
                index.remove_path(&rel)?;
                // Stage 0 is "resolved"; the conflict entry carries its side
                // in the top two flag bits.
                entry.flags &= !0x3000;
                index.add(&entry)?;
            }
            None => {
                // The chosen side deleted the file.
                match fs::remove_file(&abs) {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => {
                        return Err(GitError::Other(format!(
                            "cannot remove `{}`: {err}",
                            rel.display()
                        )))
                    }
                }
                index.remove_path(&rel)?;
            }
        }
        index.write()?;
        self.merge_conflicts()
    }

    /// Keep our side of one conflicted path (alias for
    /// [`Self::resolve_conflict`]).
    pub fn resolve_ours(&self, path: &Path) -> Result<Vec<ConflictEntry>, GitError> {
        self.resolve_conflict(path, ConflictSide::Ours)
    }

    /// Keep their side of one conflicted path.
    pub fn resolve_theirs(&self, path: &Path) -> Result<Vec<ConflictEntry>, GitError> {
        self.resolve_conflict(path, ConflictSide::Theirs)
    }

    /// Abort an in-progress merge: reset the index and working tree to HEAD,
    /// then clear `MERGE_HEAD` / the merge state.
    pub fn merge_abort(&self) -> Result<(), GitError> {
        if self.repo.state() != git2::RepositoryState::Merge && self.merge_head_ids().is_empty() {
            return Err(GitError::Other("no merge is in progress".to_owned()));
        }
        let head = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok())
            .ok_or_else(|| {
                GitError::Other("cannot abort a merge without a HEAD commit".to_owned())
            })?;
        self.repo
            .reset(head.as_object(), git2::ResetType::Hard, None)?;
        self.repo.cleanup_state()?;
        Ok(())
    }

    /// The second parents of a merge commit in progress (normally one).
    pub(crate) fn merge_head_ids(&self) -> Vec<git2::Oid> {
        let path = self.repo.path().join("MERGE_HEAD");
        let Ok(text) = fs::read_to_string(path) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|line| git2::Oid::from_str(line.trim()).ok())
            .collect()
    }

    fn checkout_commit(&self, commit: &git2::Commit<'_>) -> Result<(), GitError> {
        let mut options = git2::build::CheckoutBuilder::new();
        options.safe().allow_conflicts(false);
        self.repo
            .checkout_tree(commit.as_object(), Some(&mut options))?;
        Ok(())
    }

    fn update_head_ref(&self, oid: git2::Oid) -> Result<(), GitError> {
        match self.repo.head() {
            Ok(head) if head.is_branch() => {
                let refname = head
                    .name()
                    .map_err(|_| GitError::Other("HEAD has no refname".to_owned()))?;
                self.repo
                    .reference(refname, oid, true, "merge: fast-forward")?;
                self.repo.set_head(refname)?;
            }
            Ok(_) => self.repo.set_head_detached(oid)?,
            Err(_) => {
                let head_ref = self
                    .repo
                    .find_reference("HEAD")
                    .ok()
                    .and_then(|head| head.symbolic_target().ok().flatten().map(str::to_owned))
                    .ok_or_else(|| {
                        GitError::Other("HEAD is unborn and has no symbolic target".to_owned())
                    })?;
                self.repo
                    .reference(&head_ref, oid, true, "merge: fast-forward")?;
                self.repo.set_head(&head_ref)?;
            }
        }
        Ok(())
    }
}
