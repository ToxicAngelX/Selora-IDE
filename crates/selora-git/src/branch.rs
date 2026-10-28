//! Local branch management: list, create, switch, delete.
//!
//! All of it runs through libgit2 — no `git` CLI, no shell. Delete is the
//! safe form only: the checked-out branch is refused and an unmerged branch is
//! refused, so the operation can never orphan commits.

use crate::{branch_name, GitError, GitRepo};

/// One local branch, as the branch picker shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    /// Short name (`main`, `feature/git`).
    pub name: String,
    /// Whether HEAD points at this branch.
    pub is_current: bool,
    /// Configured upstream (`origin/main`), when one exists.
    pub upstream: Option<String>,
    /// Tip commit as full hex, or `None` for an unborn branch.
    pub tip: Option<String>,
}

impl GitRepo {
    /// The checked-out branch name, or the short detached id in parentheses
    /// when HEAD is detached; `None` only when HEAD cannot be read.
    pub fn current_branch(&self) -> Option<String> {
        branch_name(&self.repo)
    }

    /// Every local branch, sorted by name.
    pub fn branches(&self) -> Result<Vec<BranchInfo>, GitError> {
        let current = branch_name(&self.repo);
        let mut out = Vec::new();
        for entry in self.repo.branches(Some(git2::BranchType::Local))? {
            let (branch, _) = entry?;
            let Some(name) = branch.get().shorthand().ok().map(str::to_owned) else {
                continue;
            };
            let upstream = branch
                .upstream()
                .ok()
                .and_then(|upstream| upstream.get().shorthand().ok().map(str::to_owned));
            let tip = branch.get().target().map(|oid| oid.to_string());
            out.push(BranchInfo {
                is_current: current.as_deref() == Some(name.as_str()),
                name,
                upstream,
                tip,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Create a local branch at `start_point` (default: HEAD). Returns the new
    /// branch. Refuses to overwrite an existing branch.
    pub fn create_branch(
        &self,
        name: &str,
        start_point: Option<&str>,
    ) -> Result<BranchInfo, GitError> {
        validate_branch_name(name)?;
        let commit = match start_point {
            Some(rev) => self
                .repo
                .revparse_single(rev)
                .map_err(|_| {
                    GitError::Other(format!("start point `{rev}` does not resolve to a commit"))
                })?
                .peel_to_commit()?,
            None => self
                .repo
                .head()
                .map_err(|_| {
                    GitError::Other(
                        "cannot create a branch before the repository has a first commit"
                            .to_owned(),
                    )
                })?
                .peel_to_commit()?,
        };
        let branch = self.repo.branch(name, &commit, false)?;
        let tip = branch.get().target().map(|oid| oid.to_string());
        Ok(BranchInfo {
            name: name.to_owned(),
            is_current: false,
            upstream: None,
            tip,
        })
    }

    /// Check out a local branch. The checkout is libgit2's safe mode: local
    /// edits that the switch would overwrite make it fail instead of being
    /// lost.
    pub fn switch_branch(&self, name: &str) -> Result<(), GitError> {
        validate_branch_name(name)?;
        let refname = format!("refs/heads/{name}");
        let (object, _reference) = self
            .repo
            .revparse_ext(&refname)
            .map_err(|_| GitError::Other(format!("branch `{name}` does not exist")))?;
        let mut options = git2::build::CheckoutBuilder::new();
        options.safe().allow_conflicts(false);
        self.repo.checkout_tree(&object, Some(&mut options))?;
        self.repo.set_head(&refname)?;
        Ok(())
    }

    /// Delete a local branch. Refuses the checked-out branch and any branch
    /// whose tip is not reachable from HEAD (the safe `git branch -d` rule).
    pub fn delete_branch(&self, name: &str) -> Result<(), GitError> {
        validate_branch_name(name)?;
        if branch_name(&self.repo).as_deref() == Some(name) {
            return Err(GitError::Other(format!(
                "cannot delete the checked-out branch `{name}`"
            )));
        }
        let branch = self
            .repo
            .find_branch(name, git2::BranchType::Local)
            .map_err(|_| GitError::Other(format!("branch `{name}` does not exist")))?;
        let tip = branch
            .get()
            .target()
            .ok_or_else(|| GitError::Other(format!("branch `{name}` has no commit")))?;
        let head = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok())
            .map(|commit| commit.id());
        let merged = match head {
            Some(head) => head == tip || self.repo.graph_descendant_of(head, tip)?,
            None => false,
        };
        if !merged {
            return Err(GitError::Other(format!(
                "branch `{name}` is not fully merged into HEAD"
            )));
        }
        let mut branch = branch;
        branch.delete()?;
        Ok(())
    }
}

/// Branch names are validated twice: once by libgit2's own refname rules and
/// once for the characters that would be ambiguous in a push refspec. A name
/// is rendered in errors verbatim, never through a shell.
pub(crate) fn validate_branch_name(name: &str) -> Result<(), GitError> {
    let invalid = |reason: &str| GitError::InvalidName {
        kind: "branch",
        name: name.to_owned(),
        reason: reason.to_owned(),
    };
    if name.is_empty() {
        return Err(invalid("name is empty"));
    }
    if name.len() > 255 {
        return Err(invalid("name is longer than 255 bytes"));
    }
    if name.starts_with('-') {
        return Err(invalid("name must not start with `-`"));
    }
    if name.eq_ignore_ascii_case("HEAD") || name == "@" {
        return Err(invalid("`HEAD` and `@` are reserved"));
    }
    if name.contains([' ', '\t', '\n', '\r', ':', '~', '^', '?', '*', '\\', '['])
        || name.contains("..")
        || name.contains("@{")
        || name.contains("//")
        || name.ends_with('/')
        || name.ends_with('.')
        || name.ends_with(".lock")
    {
        return Err(invalid("name contains a forbidden character or sequence"));
    }
    if !git2::Reference::is_valid_name(&format!("refs/heads/{name}")) {
        return Err(invalid("name is not a valid git refname"));
    }
    // Belt and braces: the safe API above always passes a plain name, never a
    // path, but a name is echoed into events so keep it single-line.
    if name.chars().any(char::is_control) {
        return Err(invalid("name contains a control character"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_name_validation_rejects_unsafe_forms() {
        for bad in [
            "", "-x", "HEAD", "a..b", "a b", "a:b", "a~b", "a^b", "a?b", "a*b", "a\\b", "a[",
            "a//b", "a/", "a.", "a.lock", "a@{1}",
        ] {
            assert!(validate_branch_name(bad).is_err(), "{bad:?} should fail");
        }
        for good in ["main", "feature/git", "release-1.2", "user_name"] {
            assert!(validate_branch_name(good).is_ok(), "{good:?} should pass");
        }
    }
}
