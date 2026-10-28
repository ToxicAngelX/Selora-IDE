//! Selora Git — git integration via `git2` (PLAN.md §4.6, §5).
//!
//! The full daily surface (ARCHITECTURE.md §10.5 and the daily-git slice):
//! open the repository a folder belongs to, snapshot status, stage / unstage
//! (single path or everything), commit, discard working-tree / index changes,
//! diff, browse and inspect history, manage local branches, stash, and merge
//! with explicit conflict entries and ours/theirs resolution.
//!
//! Network operations (`fetch` / `pull` / `push`) are the one exception to
//! "no CLI": they run the real `git` executable with a fixed argv so the OS
//! credential helper works unchanged. There is no shell string, no force
//! push, and output is redacted and bounded before it reaches an error or an
//! event.
//!
//! Everything else is synchronous and cheap enough to run inside a
//! `spawn_blocking` worker — that is how the app shell invokes it (never on
//! the frame path).
//!
//! `selora-git` depends on no other workspace crate (PLAN.md §5
//! inter-crate rules); its [`GitError`] is mapped to
//! `SeloraError::Git` at the app wiring seam.

#![forbid(unsafe_code)]

mod branch;
mod commit;
mod diff;
mod discard;
mod error;
mod index;
mod merge;
mod net;
mod path;
mod stash;
mod status;

pub use branch::BranchInfo;
pub use commit::{ChangeKind, CommitDetail, CommitFileChange, CommitInfo};
pub use diff::{DiffLine, FileDiff, Hunk};
pub use discard::DiscardTarget;
pub use error::GitError;
pub use merge::{ConflictEntry, ConflictSide, MergeOutcome, MergeStatus};
pub use net::{sanitize_remote_text, RemoteOperation, RemoteOutcome};
pub use stash::StashEntry;
pub use status::{FileStatus, GitStatus, Slot};

use path::checked_relative;
use std::path::{Path, PathBuf};

/// An open git repository plus its (normalized) work directory.
///
/// Cheap to open: the app opens one per git effect rather than keeping a
/// long-lived handle in app state, so a folder switch or a repo deleted
/// mid-session can never leave a stale handle behind.
pub struct GitRepo {
    repo: git2::Repository,
    workdir: PathBuf,
}

/// Spec'd free constructor (ARCHITECTURE.md §10.5) — see [`GitRepo::open`].
pub fn open_repo(path: &Path) -> Result<GitRepo, GitError> {
    GitRepo::open(path)
}

impl GitRepo {
    /// Open the repository that contains `path`, walking up like
    /// `git rev-parse --show-toplevel` (Open Folder may land inside a
    /// subdirectory of the repo). Rejects bare repositories: there is no
    /// working tree for the panel to talk about.
    pub fn open(path: &Path) -> Result<GitRepo, GitError> {
        let repo = git2::Repository::discover(path).map_err(|err| {
            GitError::Other(format!(
                "`{}` is not inside a git repository ({err})",
                path.display()
            ))
        })?;
        let workdir = repo
            .workdir()
            .map(path::normalize)
            .ok_or_else(|| {
                GitError::Other(
                    "bare repositories are not supported by the Lite git panel".to_owned(),
                )
            })?
            .to_path_buf();
        Ok(Self { repo, workdir })
    }

    /// The repository's work directory (root of the checked-out files).
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Snapshot the status (ARCHITECTURE.md §10.5). Untracked files are
    /// included (`?`), ignored files are not; rename detection runs for
    /// the head→index side. Entries come back sorted by path so the panel
    /// is stable frame to frame.
    pub fn status(&self) -> Result<GitStatus, GitError> {
        let branch = branch_name(&self.repo);
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_head_to_index(true);
        let statuses = self.repo.statuses(Some(&mut options))?;
        let mut entries = Vec::new();
        for entry in statuses.iter() {
            let flags = entry.status();
            // A conflicted entry has no stage-0 index entry, so neither slot
            // maps to a change; it is reported through `conflicts` instead.
            if flags.contains(git2::Status::CONFLICTED) {
                continue;
            }
            let staged = Slot::from_index(flags);
            let worktree = Slot::from_worktree(flags);
            if staged == Slot::Unmodified && worktree == Slot::Unmodified {
                continue; // e.g. a conflicted/ignored-only flag we do not surface
            }
            let path = match entry.path() {
                Ok(path) => PathBuf::from(path),
                Err(_) => continue, // non-UTF-8 path: not representable in Lite
            };
            entries.push(FileStatus {
                abs_path: self.workdir.join(&path),
                path,
                staged,
                worktree,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let conflicts = self
            .merge_conflicts()?
            .into_iter()
            .map(|conflict| self.workdir.join(conflict.path))
            .collect();
        Ok(GitStatus {
            branch,
            entries,
            conflicts,
        })
    }

    /// Stage `path` (working tree → index). Accepts an absolute path (the
    /// panel's rows carry `FileStatus::abs_path`) or a repo-relative one.
    /// Staging a file that no longer exists on disk records the deletion,
    /// exactly like `git add` does.
    pub fn stage(&self, path: &Path) -> Result<(), GitError> {
        let rel = checked_relative(&self.workdir, path)?;
        let mut index = self.repo.index()?;
        if self.workdir.join(&rel).exists() {
            index.add_path(&rel)?;
        } else {
            index.remove_path(&rel)?;
        }
        index.write()?;
        Ok(())
    }

    /// Unstage `path` (index → HEAD, like `git reset -- <path>`): the
    /// entry is reset to its HEAD state, or removed from the index when
    /// HEAD does not have it (a staged new file becomes untracked again —
    /// including on an unborn HEAD, where there is no HEAD at all).
    pub fn unstage(&self, path: &Path) -> Result<(), GitError> {
        let rel = checked_relative(&self.workdir, path)?;
        let target = self.repo.revparse_single("HEAD").ok();
        self.repo
            .reset_default(target.as_ref(), [rel.to_string_lossy().into_owned()])?;
        Ok(())
    }

    /// Commit the index (ARCHITECTURE.md §10.5). The identity comes from
    /// the repository config (`user.name` / `user.email`); a missing
    /// identity is surfaced with the fix, not papered over with a fake
    /// author. Refuses an empty message and an empty commit — the panel
    /// disables the button for both, this is the backstop.
    ///
    /// When a merge is in progress (`MERGE_HEAD` exists), the merge head is
    /// added as the second parent and an unchanged tree is allowed: that is
    /// the merge commit itself. A successful merge commit clears the merge
    /// state, so the next commit is an ordinary one.
    pub fn commit(&self, message: &str) -> Result<git2::Oid, GitError> {
        let message = message.trim();
        if message.is_empty() {
            return Err(GitError::Other("commit message is empty".to_owned()));
        }
        let mut index = self.repo.index()?;
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;

        let merge_heads = self.merge_head_ids();
        let mut parent_commits: Vec<git2::Commit<'_>> = Vec::new();
        if let Some(head) = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok())
        {
            parent_commits.push(head);
        }
        for oid in &merge_heads {
            if parent_commits.iter().any(|commit| commit.id() == *oid) {
                continue;
            }
            if let Ok(commit) = self.repo.find_commit(*oid) {
                parent_commits.push(commit);
            }
        }

        let nothing_staged = match parent_commits.first() {
            Some(commit) => commit
                .tree()
                .is_ok_and(|head_tree| head_tree.id() == tree_id),
            // Unborn HEAD: an empty tree would produce an empty root commit.
            None => tree.is_empty(),
        };
        // A merge commit may legitimately keep the same tree as one parent.
        if nothing_staged && parent_commits.len() <= 1 {
            return Err(GitError::Other("nothing staged to commit".to_owned()));
        }
        let signature = self.repo.signature().map_err(|err| {
            GitError::Other(format!(
                "{err} — set user.name and user.email in your git config"
            ))
        })?;
        let parents: Vec<&git2::Commit<'_>> = parent_commits.iter().collect();
        let oid = self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )?;
        if !merge_heads.is_empty() {
            self.repo.cleanup_state()?;
        }
        Ok(oid)
    }

    /// Diff one file's working tree against the index (the unstaged
    /// changes — what the git panel shows; ARCHITECTURE.md assigns the
    /// panel "working tree vs index"). Untracked files diff as fully
    /// added. Accepts an absolute or repo-relative path.
    ///
    /// This is the *unstaged* side of a row. A fully staged file has no
    /// working-tree changes and therefore an empty diff here; use
    /// [`Self::diff_index`] for the staged side (its docs explain how to
    /// choose per row).
    pub fn diff_workdir(&self, path: &Path) -> Result<FileDiff, GitError> {
        let rel = checked_relative(&self.workdir, path)?;
        let index = self.repo.index()?;
        let mut options = git2::DiffOptions::new();
        options
            .pathspec(rel.to_string_lossy().into_owned())
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            // Without this libgit2 lists an untracked delta but prints no
            // content for it (plain `git diff` semantics).
            .show_untracked_content(true);
        let diff = self
            .repo
            .diff_index_to_workdir(Some(&index), Some(&mut options))?;
        collect_file_diff(&diff, rel)
    }

    /// Diff one file's staged changes: HEAD tree → index (`git diff
    /// --cached`, i.e. what a commit would record). The index is the
    /// "new" side, so added lines are staged additions and a staged
    /// deletion comes back as all removals. Accepts an absolute or
    /// repo-relative path, with the same path handling as
    /// [`Self::diff_workdir`].
    ///
    /// **Which diff does a row need?** Choose by the row's
    /// [`FileStatus`] slots: `staged != Slot::Unmodified` means the row
    /// has an index-side diff (call this); `worktree != Slot::Unmodified`
    /// or an untracked file means it has a working-tree diff (call
    /// [`Self::diff_workdir`]). A file can be dirty on both sides (`MM`);
    /// render both, index side first, or let the view pick the side it
    /// displays. Neither call errors on a clean side — it returns an
    /// empty [`FileDiff`].
    ///
    /// An unborn HEAD (no commits yet) diffs against an empty tree, so a
    /// staged new file appears as all additions rather than erroring.
    pub fn diff_index(&self, path: &Path) -> Result<FileDiff, GitError> {
        let rel = checked_relative(&self.workdir, path)?;
        let index = self.repo.index()?;
        // `None` is libgit2's empty tree, which is exactly the unborn-HEAD
        // case: every index entry is an addition. Any other HEAD failure
        // (corruption, a missing object) is real and propagates.
        let head_tree = match self.repo.head() {
            Ok(head) => Some(head.peel_to_tree()?),
            Err(err) if err.code() == git2::ErrorCode::UnbornBranch => None,
            Err(err) => return Err(err.into()),
        };
        let mut options = git2::DiffOptions::new();
        options.pathspec(rel.to_string_lossy().into_owned());
        let diff =
            self.repo
                .diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut options))?;
        collect_file_diff(&diff, rel)
    }

    /// Map a path onto a repo-relative path with `/` separators (the form
    /// libgit2 expects). Absolute paths must live under the workdir; a
    /// relative path is taken as already repo-relative. Escaping paths (and
    /// `.git` components) are `None`.
    #[cfg(test)]
    fn repo_relative(&self, path: &Path) -> Option<PathBuf> {
        path::repo_relative(&self.workdir, path)
    }
}

/// Collect one file's patch out of a libgit2 diff into a [`FileDiff`].
/// Shared by both diff directions so the panel gets the same hunk and
/// line shape whichever side of a row it shows.
fn collect_file_diff(diff: &git2::Diff<'_>, path: PathBuf) -> Result<FileDiff, GitError> {
    let mut out = FileDiff {
        path,
        hunks: Vec::new(),
    };
    diff.print(git2::DiffFormat::Patch, |_delta, hunk, line| {
        match line.origin() {
            'H' => {
                if let Some(hunk) = hunk {
                    out.hunks.push(Hunk {
                        old_start: hunk.old_start(),
                        old_lines: hunk.old_lines(),
                        new_start: hunk.new_start(),
                        new_lines: hunk.new_lines(),
                        lines: Vec::new(),
                    });
                }
            }
            '+' | '-' | ' ' | '\\' => {
                let content = String::from_utf8_lossy(line.content());
                let content = content.trim_end_matches(['\n', '\r']).to_owned();
                if let Some(current) = out.hunks.last_mut() {
                    current.lines.push(DiffLine {
                        origin: line.origin(),
                        old_no: line.old_lineno(),
                        new_no: line.new_lineno(),
                        content,
                    });
                }
            }
            _ => {} // 'F' file headers etc.
        }
        true
    })?;
    Ok(out)
}

/// The checked-out branch, the short id for a detached HEAD, or the
/// unborn branch name read from HEAD's symbolic target.
pub(crate) fn branch_name(repo: &git2::Repository) -> Option<String> {
    match repo.head() {
        Ok(head) => {
            if head.is_branch() {
                head.shorthand().ok().map(str::to_owned)
            } else {
                head.target().map(|oid| format!("({})", short_id(oid)))
            }
        }
        Err(_) => {
            let head_ref = repo.find_reference("HEAD").ok()?;
            head_ref
                .symbolic_target()
                .ok()
                .flatten()?
                .strip_prefix("refs/heads/")
                .map(str::to_owned)
        }
    }
}

/// The first 7 hex digits of `oid` (git's default short id length).
fn short_id(oid: git2::Oid) -> String {
    oid.to_string().chars().take(7).collect()
}

// -----------------------------------------------------------------------------
// Tests — real repositories in temp dirs, driven through git2 itself
// (init, write, stage, commit) and asserted through the public API.
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Init a repo with `main` as the unborn branch and a test identity.
    fn init_repo() -> (tempfile::TempDir, GitRepo) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut options = git2::RepositoryInitOptions::new();
        options.initial_head("main");
        let raw = git2::Repository::init_opts(dir.path(), &options).expect("git init");
        let mut config = raw.config().expect("config");
        config
            .set_str("user.name", "Selora Test")
            .expect("user.name");
        config
            .set_str("user.email", "selora-test@example.com")
            .expect("user.email");
        drop(config);
        let repo = GitRepo::open(dir.path()).expect("open");
        (dir, repo)
    }

    /// Write a file (creating parent dirs) inside the repo workdir.
    fn write(dir: &tempfile::TempDir, rel: &str, text: &str) {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, text).expect("write");
    }

    /// Commit everything currently in the workdir (helper for setup).
    fn commit_all(repo: &GitRepo, message: &str) -> git2::Oid {
        let status = repo.status().expect("status");
        for entry in &status.entries {
            repo.stage(&entry.abs_path).expect("stage");
        }
        repo.commit(message).expect("commit")
    }

    fn find_entry<'a>(status: &'a GitStatus, rel: &str) -> &'a FileStatus {
        status
            .entries
            .iter()
            .find(|entry| entry.path == Path::new(rel))
            .unwrap_or_else(|| panic!("no status entry for `{rel}`"))
    }

    #[test]
    fn unborn_repo_reports_branch_and_untracked_files() {
        let (dir, repo) = init_repo();
        write(&dir, "notes.txt", "hello");

        let status = repo.status().expect("status");
        // Unborn HEAD: the branch name comes from HEAD's symbolic target.
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.changed_count(), 1);
        assert!(status.is_dirty());

        let entry = find_entry(&status, "notes.txt");
        assert!(entry.is_untracked());
        assert!(!entry.is_staged());
        assert_eq!(entry.letter(), '?');
        // The abs path is the workdir join — the key the file tree matches on.
        assert_eq!(entry.abs_path, dir.path().join("notes.txt"));
    }

    #[test]
    fn untracked_files_in_subdirectories_are_recurse_listed() {
        let (dir, repo) = init_repo();
        write(&dir, "src/ui/theme.rs", "// tokens");

        let status = repo.status().expect("status");
        assert_eq!(status.changed_count(), 1);
        assert_eq!(find_entry(&status, "src/ui/theme.rs").letter(), '?');
    }

    #[test]
    fn stage_untracked_commit_then_modify_reports_a_then_m_then_clean() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");

        // Untracked → stage → 'A', staged, no worktree remainder.
        repo.stage(&dir.path().join("a.txt")).expect("stage");
        let status = repo.status().expect("status");
        let entry = find_entry(&status, "a.txt");
        assert!(entry.is_staged());
        assert!(!entry.has_worktree_changes());
        assert_eq!(entry.letter(), 'A');
        assert_eq!(status.staged_count(), 1);

        // Commit → the tree matches HEAD again: clean, branch still main.
        let oid = repo.commit("init").expect("commit");
        assert!(!oid.is_zero());
        let status = repo.status().expect("status");
        assert!(status.entries.is_empty());
        assert_eq!(status.branch.as_deref(), Some("main"));

        // Modify → unstaged 'M'.
        write(&dir, "a.txt", "one\ntwo\n");
        let status = repo.status().expect("status");
        let entry = find_entry(&status, "a.txt");
        assert!(!entry.is_staged());
        assert!(entry.has_worktree_changes());
        assert_eq!(entry.letter(), 'M');

        // Stage → staged 'M' with a clean worktree side.
        repo.stage(&dir.path().join("a.txt")).expect("stage");
        let status = repo.status().expect("status");
        let entry = find_entry(&status, "a.txt");
        assert_eq!(entry.staged, Slot::Modified);
        assert_eq!(entry.worktree, Slot::Unmodified);

        // Stage again after another edit → both sides modified at once.
        write(&dir, "a.txt", "one\ntwo\nthree\n");
        repo.stage(&dir.path().join("a.txt")).expect("stage");
        write(&dir, "a.txt", "one\ntwo\nthree\nfour\n");
        let status = repo.status().expect("status");
        let entry = find_entry(&status, "a.txt");
        assert_eq!(entry.staged, Slot::Modified);
        assert_eq!(entry.worktree, Slot::Modified);

        // Commit → only the unstaged "four" edit remains (the index was
        // committed; the working tree was not touched by the commit).
        repo.commit("second").expect("commit");
        let status = repo.status().expect("status");
        assert_eq!(status.changed_count(), 1);
        let entry = find_entry(&status, "a.txt");
        assert!(!entry.is_staged());
        assert_eq!(entry.worktree, Slot::Modified);

        // Stage that too and commit → clean.
        repo.stage(&dir.path().join("a.txt")).expect("stage");
        repo.commit("third").expect("commit");
        assert!(repo.status().expect("status").entries.is_empty());
    }

    #[test]
    fn unstage_returns_changes_to_the_worktree_side() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");
        write(&dir, "a.txt", "one\neditted\n");

        repo.stage(&dir.path().join("a.txt")).expect("stage");
        assert!(repo.status().expect("status").entries[0].is_staged());

        repo.unstage(&dir.path().join("a.txt")).expect("unstage");
        let status = repo.status().expect("status");
        let entry = find_entry(&status, "a.txt");
        assert!(!entry.is_staged());
        assert_eq!(entry.worktree, Slot::Modified);
        assert_eq!(entry.letter(), 'M');
        // The file is back to exactly how it was before staging.
        assert_eq!(status.staged_count(), 0);
        assert_eq!(status.changed_count(), 1);
    }

    #[test]
    fn unstage_a_staged_new_file_makes_it_untracked_again() {
        let (dir, repo) = init_repo();
        write(&dir, "new.txt", "fresh");
        repo.stage(&dir.path().join("new.txt")).expect("stage");
        assert_eq!(repo.status().expect("status").entries[0].letter(), 'A');

        // Unborn HEAD: unstage removes the index entry entirely.
        repo.unstage(&dir.path().join("new.txt")).expect("unstage");
        let status = repo.status().expect("status");
        assert_eq!(find_entry(&status, "new.txt").letter(), '?');
    }

    #[test]
    fn deleted_files_report_d_and_stage_the_deletion() {
        let (dir, repo) = init_repo();
        write(&dir, "keep.txt", "keep");
        write(&dir, "gone.txt", "gone");
        commit_all(&repo, "init");

        fs::remove_file(dir.path().join("gone.txt")).expect("remove");
        let status = repo.status().expect("status");
        let entry = find_entry(&status, "gone.txt");
        assert_eq!(entry.worktree, Slot::Deleted);
        assert!(!entry.is_staged());
        assert_eq!(entry.letter(), 'D');

        // Staging a deleted file records the deletion.
        repo.stage(&dir.path().join("gone.txt")).expect("stage");
        let status = repo.status().expect("status");
        let entry = find_entry(&status, "gone.txt");
        assert_eq!(entry.staged, Slot::Deleted);
        assert!(!entry.has_worktree_changes());

        // Committing it removes the file from the tree: clean.
        repo.commit("remove gone").expect("commit");
        let status = repo.status().expect("status");
        assert!(status.entries.is_empty());
    }

    #[test]
    fn diff_workdir_reports_removed_and_added_lines_with_numbers() {
        let (dir, repo) = init_repo();
        write(&dir, "text.txt", "one\ntwo\nthree\n");
        commit_all(&repo, "init");
        write(&dir, "text.txt", "one\nTWO\nthree\nfour\n");

        let diff = repo
            .diff_workdir(&dir.path().join("text.txt"))
            .expect("diff");
        assert!(!diff.hunks.is_empty(), "expected at least one hunk");
        let lines: Vec<&DiffLine> = diff.hunks.iter().flat_map(|h| h.lines.iter()).collect();

        let removed = lines
            .iter()
            .find(|line| line.origin == '-')
            .expect("a removed line");
        assert_eq!(removed.content, "two");
        assert_eq!(removed.old_no, Some(2));
        assert_eq!(removed.new_no, None);

        let added: Vec<&DiffLine> = lines
            .iter()
            .copied()
            .filter(|line| line.origin == '+')
            .collect();
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].content, "TWO");
        assert_eq!(added[0].new_no, Some(2));
        assert_eq!(added[0].old_no, None);
        assert_eq!(added[1].content, "four");
        assert_eq!(added[1].new_no, Some(4));

        // Context lines carry both side numbers.
        let context = lines
            .iter()
            .find(|line| line.origin == ' ')
            .expect("a context line");
        assert_eq!(context.content, "one");
        assert_eq!(context.old_no, Some(1));
        assert_eq!(context.new_no, Some(1));

        // A clean file diffs to nothing.
        repo.stage(&dir.path().join("text.txt")).expect("stage");
        let clean = repo
            .diff_workdir(&dir.path().join("text.txt"))
            .expect("diff");
        assert!(clean.hunks.is_empty());
    }

    #[test]
    fn diff_workdir_of_untracked_file_is_all_additions() {
        let (dir, repo) = init_repo();
        write(&dir, "fresh.txt", "brand\nnew\n");

        let diff = repo
            .diff_workdir(&dir.path().join("fresh.txt"))
            .expect("diff");
        let lines: Vec<&DiffLine> = diff.hunks.iter().flat_map(|h| h.lines.iter()).collect();
        assert!(lines.iter().all(|line| line.origin == '+'));
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn diff_index_reports_a_staged_modification_the_workdir_diff_cannot_see() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");
        write(&dir, "a.txt", "one\ntwo\n");
        repo.stage(&dir.path().join("a.txt")).expect("stage");

        let diff = repo
            .diff_index(&dir.path().join("a.txt"))
            .expect("diff index");
        assert_eq!(diff.path, Path::new("a.txt"));
        let lines: Vec<&DiffLine> = diff.hunks.iter().flat_map(|h| h.lines.iter()).collect();
        let added: Vec<&DiffLine> = lines
            .iter()
            .copied()
            .filter(|line| line.origin == '+')
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].content, "two");
        assert_eq!(added[0].old_no, None);
        assert_eq!(added[0].new_no, Some(2));
        assert!(lines
            .iter()
            .any(|line| line.origin == ' ' && line.content == "one"));

        // Fully staged: the row's worktree side is clean, so only the
        // index diff has anything to show.
        let workdir = repo
            .diff_workdir(&dir.path().join("a.txt"))
            .expect("diff workdir");
        assert!(workdir.is_empty());

        // A repo-relative path resolves identically to the absolute one.
        assert_eq!(repo.diff_index(Path::new("a.txt")).expect("relative"), diff);
    }

    #[test]
    fn diff_index_of_a_staged_new_file_in_an_unborn_repo_is_all_additions() {
        let (dir, repo) = init_repo();
        write(&dir, "fresh.txt", "brand\nnew\n");
        repo.stage(&dir.path().join("fresh.txt")).expect("stage");
        assert_eq!(repo.status().expect("status").entries[0].letter(), 'A');

        // No HEAD exists: the empty tree makes the whole file an addition.
        let diff = repo
            .diff_index(&dir.path().join("fresh.txt"))
            .expect("diff index");
        let lines: Vec<&DiffLine> = diff.hunks.iter().flat_map(|h| h.lines.iter()).collect();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.origin == '+'));
        assert_eq!(lines[0].content, "brand");
        assert_eq!(lines[0].new_no, Some(1));
        assert_eq!(lines[1].content, "new");
        assert_eq!(lines[1].new_no, Some(2));

        // Fully staged, so the worktree side stays clean.
        let workdir = repo
            .diff_workdir(&dir.path().join("fresh.txt"))
            .expect("diff workdir");
        assert!(workdir.is_empty());
    }

    #[test]
    fn diff_index_is_empty_for_a_path_that_is_not_staged() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");

        // Modified but not staged: HEAD and the index still agree.
        write(&dir, "a.txt", "one\ntwo\n");
        let index_diff = repo
            .diff_index(&dir.path().join("a.txt"))
            .expect("diff index");
        assert!(index_diff.is_empty());
        let workdir_diff = repo
            .diff_workdir(&dir.path().join("a.txt"))
            .expect("diff workdir");
        assert!(!workdir_diff.is_empty());

        // Never tracked at all: absent from both HEAD and the index.
        write(&dir, "fresh.txt", "new\n");
        assert!(repo
            .diff_index(&dir.path().join("fresh.txt"))
            .expect("diff index untracked")
            .is_empty());
    }

    #[test]
    fn diff_index_of_a_staged_deletion_is_all_removals() {
        let (dir, repo) = init_repo();
        write(&dir, "gone.txt", "bye\n");
        commit_all(&repo, "init");
        fs::remove_file(dir.path().join("gone.txt")).expect("remove");
        repo.stage(&dir.path().join("gone.txt")).expect("stage");
        assert_eq!(
            find_entry(&repo.status().expect("status"), "gone.txt").staged,
            Slot::Deleted
        );

        let diff = repo
            .diff_index(&dir.path().join("gone.txt"))
            .expect("diff index");
        let lines: Vec<&DiffLine> = diff.hunks.iter().flat_map(|h| h.lines.iter()).collect();
        assert!(!lines.is_empty());
        assert!(lines.iter().all(|line| line.origin == '-'));
        assert_eq!(lines[0].content, "bye");
        assert_eq!(lines[0].old_no, Some(1));
        assert_eq!(lines[0].new_no, None);

        // The deletion is fully staged; the worktree side is empty.
        assert!(repo
            .diff_workdir(&dir.path().join("gone.txt"))
            .expect("diff workdir")
            .is_empty());
    }

    #[test]
    fn diff_index_and_diff_workdir_split_a_file_staged_then_edited_again() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");
        write(&dir, "a.txt", "one\nstaged\n");
        repo.stage(&dir.path().join("a.txt")).expect("stage");
        write(&dir, "a.txt", "one\nstaged\nunstaged\n");

        let index = repo
            .diff_index(&dir.path().join("a.txt"))
            .expect("diff index");
        let index_lines: Vec<&DiffLine> = index.hunks.iter().flat_map(|h| h.lines.iter()).collect();
        assert!(index_lines
            .iter()
            .any(|line| line.origin == '+' && line.content == "staged"));
        assert!(!index_lines
            .iter()
            .any(|line| line.origin == '+' && line.content == "unstaged"));

        let workdir = repo
            .diff_workdir(&dir.path().join("a.txt"))
            .expect("diff workdir");
        let workdir_lines: Vec<&DiffLine> =
            workdir.hunks.iter().flat_map(|h| h.lines.iter()).collect();
        assert!(workdir_lines
            .iter()
            .any(|line| line.origin == '+' && line.content == "unstaged"));
        // The staged line is unchanged between index and workdir, so it
        // shows up as context there, never as an addition.
        assert!(workdir_lines
            .iter()
            .any(|line| line.origin == ' ' && line.content == "staged"));
        assert!(!workdir_lines
            .iter()
            .any(|line| line.origin == '+' && line.content == "staged"));
    }

    #[test]
    fn open_discovers_the_repo_from_a_subdirectory_and_rejects_non_repos() {
        let (dir, repo) = init_repo();
        write(&dir, "sub/nested.txt", "x");
        assert_eq!(repo.workdir(), dir.path());

        // Opening from deep inside finds the same workdir.
        let nested = GitRepo::open(&dir.path().join("sub")).expect("discover");
        assert_eq!(nested.workdir(), dir.path());
        let status = nested.status().expect("status");
        assert_eq!(find_entry(&status, "sub/nested.txt").letter(), '?');

        // A plain folder outside any repo is an error, not a panic.
        let outside = tempfile::tempdir().expect("tempdir");
        assert!(GitRepo::open(outside.path()).is_err());
    }

    #[test]
    fn stage_accepts_relative_and_case_folded_absolute_paths() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "x");
        // Repo-relative form (forward slashes).
        repo.stage(Path::new("a.txt")).expect("stage relative");
        assert_eq!(repo.status().expect("status").entries[0].letter(), 'A');
        repo.unstage(Path::new("a.txt")).expect("unstage relative");

        // A different case of the same workdir prefix still resolves
        // (Windows is case-insensitive).
        let dir_text = dir.path().to_string_lossy().to_uppercase();
        let folded = PathBuf::from(dir_text).join("a.txt");
        repo.stage(&folded).expect("stage case-folded");
        assert_eq!(repo.status().expect("status").entries[0].letter(), 'A');
    }

    #[test]
    fn commit_refuses_empty_message_and_nothing_staged() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "x");

        let empty = repo.commit("   \n").expect_err("empty message");
        assert!(empty.to_string().contains("commit message is empty"));

        // A repo with a HEAD whose index matches the tree: nothing to commit.
        commit_all(&repo, "init");
        let nothing = repo.commit("again").expect_err("nothing staged");
        assert!(nothing.to_string().contains("nothing staged"));

        // An unborn repo with an empty index refuses the empty root commit.
        let (_empty_dir, empty_repo) = init_repo();
        let root = empty_repo.commit("root").expect_err("empty root commit");
        assert!(root.to_string().contains("nothing staged"));
    }

    #[test]
    fn repo_relative_normalizes_windows_prefixes() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "x");
        let extended = PathBuf::from(format!(r"\\?\{}", dir.path().join("a.txt").display()));
        assert_eq!(repo.repo_relative(&extended), Some(PathBuf::from("a.txt")));
        // Outside the repo: None.
        let elsewhere = tempfile::tempdir().expect("tempdir");
        assert_eq!(repo.repo_relative(&elsewhere.path().join("a.txt")), None);
    }

    // -------------------------------------------------------------------------
    // Daily git surface: branches, history, discard, stash, merge, paths,
    // network.
    // -------------------------------------------------------------------------

    fn read(dir: &tempfile::TempDir, rel: &str) -> String {
        // Git for Windows may apply core.autocrlf on checkout; compare text.
        fs::read_to_string(dir.path().join(rel))
            .expect("read")
            .replace("\r\n", "\n")
    }

    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn branches_create_switch_list_and_delete() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");

        let created = repo.create_branch("feature/git", None).expect("create");
        assert_eq!(created.name, "feature/git");
        let branches = repo.branches().expect("branches");
        assert_eq!(
            branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            vec!["feature/git", "main"]
        );
        let main = branches.iter().find(|b| b.name == "main").unwrap();
        let feature = branches.iter().find(|b| b.name == "feature/git").unwrap();
        assert!(main.is_current);
        assert!(!feature.is_current);
        assert_eq!(feature.tip, main.tip);

        repo.switch_branch("feature/git").expect("switch");
        assert_eq!(
            repo.status().expect("status").branch.as_deref(),
            Some("feature/git")
        );
        write(&dir, "b.txt", "feature\n");
        commit_all(&repo, "on feature");

        repo.switch_branch("main").expect("switch back");
        assert!(!dir.path().join("b.txt").exists());
        assert_eq!(
            repo.status().expect("status").branch.as_deref(),
            Some("main")
        );

        // The feature commit is not reachable from main yet → safe delete
        // refuses instead of orphaning it.
        let refused = repo.delete_branch("feature/git").expect_err("unmerged");
        assert!(
            refused.to_string().contains("not fully merged"),
            "{refused}"
        );

        let merged = repo.merge_branch("feature/git").expect("merge");
        assert_eq!(merged.status, MergeStatus::FastForward);
        assert!(dir.path().join("b.txt").exists());
        repo.delete_branch("feature/git").expect("delete merged");
        assert_eq!(repo.branches().expect("branches").len(), 1);

        // The checked-out branch can never be deleted.
        assert!(repo.delete_branch("main").is_err());
        // Invalid names are rejected before touching a ref.
        assert!(repo.create_branch("../escape", None).is_err());
        assert!(repo.create_branch("bad name", None).is_err());
        assert!(repo.switch_branch("../escape").is_err());
    }

    #[test]
    fn discard_restores_worktree_and_moves_index_back_to_head() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");

        // Working-tree discard: tracked file is rebuilt from the index.
        write(&dir, "a.txt", "dirty\n");
        let count = repo
            .discard(&[dir.path().join("a.txt")], DiscardTarget::Worktree)
            .expect("discard worktree");
        assert_eq!(count, 1);
        assert_eq!(read(&dir, "a.txt"), "one\n");
        assert!(repo.status().expect("status").entries.is_empty());

        // Untracked discard removes that one file.
        write(&dir, "fresh.txt", "new\n");
        repo.discard(&[dir.path().join("fresh.txt")], DiscardTarget::Worktree)
            .expect("discard untracked");
        assert!(!dir.path().join("fresh.txt").exists());
        assert!(repo.status().expect("status").entries.is_empty());

        // Index discard: the index returns to HEAD, the working tree stays.
        write(&dir, "a.txt", "staged\n");
        repo.stage(&dir.path().join("a.txt")).expect("stage");
        repo.discard(&[dir.path().join("a.txt")], DiscardTarget::Index)
            .expect("discard index");
        let status = repo.status().expect("status");
        let entry = find_entry(&status, "a.txt");
        assert!(!entry.is_staged());
        assert_eq!(entry.worktree, Slot::Modified);
        assert_eq!(read(&dir, "a.txt"), "staged\n");
    }

    #[test]
    fn discard_all_clears_both_sides_and_deletes_untracked_or_new_files() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        write(&dir, "gone.txt", "gone\n");
        commit_all(&repo, "init");

        // Staged AND further edited: one discard returns to the committed text.
        write(&dir, "a.txt", "staged\n");
        repo.stage(&dir.path().join("a.txt")).expect("stage");
        write(&dir, "a.txt", "dirty again\n");
        repo.discard_all(&dir.path().join("a.txt"))
            .expect("discard all");
        assert_eq!(read(&dir, "a.txt"), "one\n");
        assert!(repo.status().expect("status").entries.is_empty());

        // An untracked file is deleted (the path the panel's row carries).
        write(&dir, "fresh.txt", "new\n");
        repo.discard_all(&dir.path().join("fresh.txt"))
            .expect("discard untracked");
        assert!(!dir.path().join("fresh.txt").exists());

        // A newly staged file is removed from the index and from disk.
        write(&dir, "added.txt", "added\n");
        repo.stage(&dir.path().join("added.txt"))
            .expect("stage add");
        repo.discard_all(&dir.path().join("added.txt"))
            .expect("discard added");
        assert!(!dir.path().join("added.txt").exists());
        assert!(repo.status().expect("status").entries.is_empty());

        // A deleted tracked file comes back from HEAD.
        fs::remove_file(dir.path().join("gone.txt")).expect("remove");
        repo.discard_all(&dir.path().join("gone.txt"))
            .expect("discard deletion");
        assert_eq!(read(&dir, "gone.txt"), "gone\n");
        assert!(repo.status().expect("status").entries.is_empty());

        // A conflicted path is refused: resolve it explicitly instead.
        repo.create_branch("other", None).expect("create");
        repo.switch_branch("other").expect("switch");
        write(&dir, "a.txt", "theirs\n");
        commit_all(&repo, "theirs");
        repo.switch_branch("main").expect("switch");
        write(&dir, "a.txt", "ours\n");
        commit_all(&repo, "ours");
        repo.merge_branch("other").expect("merge");
        let refused = repo
            .discard_all(&dir.path().join("a.txt"))
            .expect_err("conflict refused");
        assert!(refused.to_string().contains("merge conflict"), "{refused}");
    }

    #[test]
    fn log_limits_filters_by_path_and_show_commit_carries_metadata_and_patch() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "one");
        write(&dir, "b.txt", "bee\n");
        commit_all(&repo, "two");
        write(&dir, "a.txt", "one\ntwo\n");
        commit_all(&repo, "three");

        let recent = repo.log(2, None).expect("log");
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].summary, "three");
        assert_eq!(recent[1].summary, "two");
        assert_eq!(recent[0].short_id.len(), 7);
        assert_eq!(recent[0].author_name, "Selora Test");
        assert_eq!(recent[0].parents.len(), 1);

        // Path filter: only the commits that touched a.txt, still newest first.
        let only_a = repo.log(10, Some(Path::new("a.txt"))).expect("log path");
        assert_eq!(
            only_a
                .iter()
                .map(|c| c.summary.as_str())
                .collect::<Vec<_>>(),
            vec!["three", "one"]
        );
        assert!(repo.log(0, None).expect("zero").is_empty());
        assert!(repo
            .log(10, Some(Path::new("missing.txt")))
            .expect("missing path")
            .is_empty());

        let head = repo.log(1, None).expect("head").remove(0);
        let detail = repo.show_commit("HEAD").expect("show");
        assert_eq!(detail.info.id, head.id);
        assert_eq!(detail.files.len(), 1);
        assert_eq!(detail.files[0].path, PathBuf::from("a.txt"));
        assert_eq!(detail.files[0].kind, ChangeKind::Modified);
        assert!(detail.files[0].additions >= 1);
        assert!(!detail.patch_truncated);
        assert!(detail.patch.contains("+two"), "{}", detail.patch);

        // The root commit diffs against the empty tree, not an error.
        let root = repo.log(10, None).expect("log").pop().expect("root");
        let root_detail = repo.show_commit(&root.id).expect("show root");
        assert_eq!(root_detail.info.parents.len(), 0);
        assert_eq!(root_detail.files.len(), 1);
        assert!(root_detail.patch.contains("+one"));
        assert!(repo.show_commit("not-a-rev").is_err());
    }

    #[test]
    fn stage_all_and_unstage_all_cover_edits_adds_and_deletions() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");

        write(&dir, "a.txt", "two\n");
        write(&dir, "new.txt", "new\n");
        assert_eq!(repo.stage_all().expect("stage all"), 2);
        assert_eq!(repo.status().expect("status").staged_count(), 2);

        assert_eq!(repo.unstage_all().expect("unstage all"), 2);
        let status = repo.status().expect("status");
        assert_eq!(status.staged_count(), 0);
        assert_eq!(find_entry(&status, "a.txt").worktree, Slot::Modified);
        assert!(find_entry(&status, "new.txt").is_untracked());

        // A deletion travels the same path.
        fs::remove_file(dir.path().join("a.txt")).expect("remove");
        repo.stage_all().expect("stage deletion");
        assert_eq!(
            find_entry(&repo.status().expect("status"), "a.txt").staged,
            Slot::Deleted
        );
        repo.unstage_all().expect("unstage deletion");
        assert_eq!(
            find_entry(&repo.status().expect("status"), "a.txt").worktree,
            Slot::Deleted
        );
    }

    #[test]
    fn stash_push_list_pop_and_drop_round_trip() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");
        write(&dir, "a.txt", "two\n");

        let entry = repo.stash_push(Some("park work")).expect("stash push");
        assert_eq!(entry.index, 0);
        assert!(entry.message.contains("park work"), "{}", entry.message);
        assert_eq!(entry.oid.len(), 40);
        assert!(repo.status().expect("status").entries.is_empty());
        let list = repo.stash_list().expect("stash list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].index, 0);

        repo.stash_pop(0).expect("stash pop");
        assert_eq!(read(&dir, "a.txt"), "two\n");
        assert!(repo.stash_list().expect("stash list").is_empty());
        assert_eq!(
            find_entry(&repo.status().expect("status"), "a.txt").worktree,
            Slot::Modified
        );

        // Drop leaves the already-cleaned working tree alone (stash push
        // parked the edit; dropping does not re-apply it).
        repo.stash_push(None).expect("second stash");
        assert_eq!(repo.stash_list().expect("list").len(), 1);
        repo.stash_drop(0).expect("drop");
        assert!(repo.stash_list().expect("list").is_empty());
        assert_eq!(read(&dir, "a.txt"), "one\n");

        // Empty stash is a precondition failure, not libgit2 noise.
        let (empty_dir, empty) = init_repo();
        write(&empty_dir, "a.txt", "clean\n");
        commit_all(&empty, "init");
        assert!(empty
            .stash_push(None)
            .unwrap_err()
            .to_string()
            .contains("nothing to stash"));
        assert!(empty.stash_pop(0).is_err());
        assert!(empty.stash_drop(0).is_err());
    }

    #[test]
    fn merge_conflict_entries_resolve_ours_and_theirs_then_commit_merge() {
        let (dir, repo) = init_repo();
        write(&dir, "conflict.txt", "base\n");
        commit_all(&repo, "base");

        repo.create_branch("other", None).expect("create other");
        repo.switch_branch("other").expect("switch other");
        write(&dir, "conflict.txt", "theirs\n");
        commit_all(&repo, "theirs");

        repo.switch_branch("main").expect("switch main");
        write(&dir, "conflict.txt", "ours\n");
        commit_all(&repo, "ours");

        let outcome = repo.merge_branch("other").expect("merge");
        assert_eq!(outcome.status, MergeStatus::Conflicts);
        assert_eq!(outcome.conflicts.len(), 1);
        let conflict = &outcome.conflicts[0];
        assert_eq!(conflict.path, PathBuf::from("conflict.txt"));
        assert!(conflict.ours && conflict.theirs && conflict.ancestor);

        // The status snapshot reports the conflict as its own list: a stage-0
        // free path is not a normal `entries` row, so the panel needs it here.
        let conflicted = repo.status().expect("status");
        assert_eq!(conflicted.conflicts, vec![dir.path().join("conflict.txt")]);
        assert!(conflicted.entries.is_empty());
        assert!(conflicted.is_dirty());
        assert_eq!(conflicted.changed_count(), 1);

        // Abort restores HEAD and clears the merge state.
        repo.merge_abort().expect("abort");
        assert!(repo.merge_conflicts().expect("conflicts").is_empty());
        assert_eq!(read(&dir, "conflict.txt"), "ours\n");

        // Resolve theirs, then finish with a real two-parent merge commit.
        let outcome = repo.merge_branch("other").expect("merge again");
        assert_eq!(outcome.status, MergeStatus::Conflicts);
        let remaining = repo
            .resolve_conflict(Path::new("conflict.txt"), ConflictSide::Theirs)
            .expect("resolve theirs");
        assert!(remaining.is_empty());
        assert_eq!(read(&dir, "conflict.txt"), "theirs\n");
        assert!(repo.merge_conflicts().expect("conflicts").is_empty());

        let oid = repo.commit("merge other").expect("merge commit");
        let raw = git2::Repository::open(dir.path()).expect("raw");
        let commit = raw.find_commit(oid).expect("commit");
        assert_eq!(commit.parent_count(), 2, "merge commit needs two parents");
        assert!(repo.status().expect("status").entries.is_empty());

        // There is no merge in progress after the commit: abort refuses instead
        // of hard-resetting the working tree.
        assert!(repo.merge_abort().is_err());
        assert!(repo.merge_conflicts().expect("conflicts").is_empty());
        assert!(repo
            .resolve_conflict(Path::new("../escape.txt"), ConflictSide::Ours)
            .is_err());

        // The other branch is now an ancestor: nothing left to merge.
        let outcome = repo.merge_branch("other").expect("already merged");
        assert_eq!(outcome.status, MergeStatus::UpToDate);
    }

    #[test]
    fn conflicting_merge_resolves_ours_in_a_fresh_repo() {
        let (dir, repo) = init_repo();
        write(&dir, "conflict.txt", "base\n");
        commit_all(&repo, "base");
        repo.create_branch("other", None).expect("create");
        repo.switch_branch("other").expect("switch");
        write(&dir, "conflict.txt", "theirs\n");
        commit_all(&repo, "theirs");
        repo.switch_branch("main").expect("switch");
        write(&dir, "conflict.txt", "ours\n");
        commit_all(&repo, "ours");

        let outcome = repo.merge_branch("other").expect("merge");
        assert_eq!(outcome.status, MergeStatus::Conflicts);
        repo.resolve_conflict(Path::new("conflict.txt"), ConflictSide::Ours)
            .expect("resolve ours");
        assert_eq!(read(&dir, "conflict.txt"), "ours\n");
        repo.commit("merge ours").expect("merge commit");
        assert!(repo.status().expect("status").entries.is_empty());
    }

    #[test]
    fn path_escape_is_rejected_before_any_git_write() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");
        let outside = tempfile::tempdir().expect("tempdir");
        fs::write(outside.path().join("secret.txt"), "outside").expect("write outside");

        for path in [
            outside.path().join("secret.txt"),
            PathBuf::from("../escape.txt"),
            PathBuf::from("nested/../../escape.txt"),
            PathBuf::from(".git/config"),
        ] {
            let err = repo.stage(&path).expect_err("stage escape");
            assert_eq!(err.code(), "path_escape", "{path:?}: {err}");
            assert!(err.to_string().contains("outside the repository"), "{err}");
        }
        assert!(repo
            .discard(&[PathBuf::from("../escape.txt")], DiscardTarget::Worktree)
            .is_err());
        assert!(repo.log(5, Some(Path::new("../escape.txt"))).is_err());
        assert!(repo
            .resolve_conflict(Path::new("../escape.txt"), ConflictSide::Ours)
            .is_err());
        // The outside file is untouched.
        assert_eq!(
            fs::read_to_string(outside.path().join("secret.txt")).expect("read"),
            "outside"
        );
    }

    #[test]
    fn network_errors_are_typed_and_redact_credentials() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");
        git2::Repository::open(dir.path())
            .expect("raw")
            .remote("origin", "http://alice:supersecret@127.0.0.1:1/repo.git")
            .expect("remote");

        let error = repo.fetch(None).expect_err("fetch must fail");
        assert_eq!(error.code(), "network", "{error}");
        let text = error.to_string();
        assert!(!text.contains("supersecret"), "{text}");
        assert!(!text.contains("alice:"), "{text}");

        // A remote name that looks like an option never reaches the CLI.
        let injected = repo.fetch(Some("-oProxyCommand=calc")).expect_err("inject");
        assert_eq!(injected.code(), "invalid_name", "{injected}");
        // Unknown but well-formed remote names are refused too.
        let missing = repo.fetch(Some("upstream")).expect_err("missing");
        assert_eq!(missing.code(), "invalid_name", "{missing}");
    }

    #[test]
    fn network_requires_a_configured_remote() {
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");
        let error = repo.push(None, None).expect_err("no remote");
        assert!(error.to_string().contains("no git remote"), "{error}");
        assert!(repo.remotes().expect("remotes").is_empty());
    }

    #[test]
    fn fetch_and_push_work_against_a_local_bare_remote() {
        if !git_available() {
            return; // the network half needs the OS git; skip on a bare runner
        }
        let bare_dir = tempfile::tempdir().expect("bare dir");
        git2::Repository::init_bare(bare_dir.path()).expect("init bare");

        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");
        git2::Repository::open(dir.path())
            .expect("raw")
            .remote("origin", bare_dir.path().to_string_lossy().as_ref())
            .expect("remote");

        let fetched = repo.fetch(None).expect("fetch local");
        assert_eq!(fetched.operation, RemoteOperation::Fetch);
        assert_eq!(fetched.remote, "origin");
        assert!(fetched.conflicts.is_empty());

        let pushed = repo.push(None, None).expect("push local");
        assert_eq!(pushed.operation, RemoteOperation::Push);
        assert!(!pushed.summary.is_empty());

        let bare = git2::Repository::open(bare_dir.path()).expect("open bare");
        assert!(
            bare.find_branch("main", git2::BranchType::Local).is_ok(),
            "main must exist in the bare remote"
        );
    }

    #[test]
    fn pull_fast_forwards_a_clone_from_a_local_bare_remote() {
        if !git_available() {
            return; // the network half needs the OS git; skip on a bare runner
        }
        let bare_dir = tempfile::tempdir().expect("bare dir");
        let bare = git2::Repository::init_bare(bare_dir.path()).expect("init bare");
        bare.set_head("refs/heads/main").expect("bare HEAD main");
        let (dir, repo) = init_repo();
        write(&dir, "a.txt", "one\n");
        commit_all(&repo, "init");
        git2::Repository::open(dir.path())
            .expect("raw")
            .remote("origin", bare_dir.path().to_string_lossy().as_ref())
            .expect("remote");
        repo.push(None, None).expect("seed push");
        assert!(bare.find_branch("main", git2::BranchType::Local).is_ok());

        // A second working copy of the same remote.
        let clone_parent = tempfile::tempdir().expect("clone parent");
        let clone_path = clone_parent.path().join("clone");
        git2::Repository::clone(bare_dir.path().to_string_lossy().as_ref(), &clone_path)
            .expect("clone");
        let clone = GitRepo::open(&clone_path).expect("open clone");

        // The origin advances after the clone was made.
        write(&dir, "a.txt", "one\ntwo\n");
        commit_all(&repo, "second");
        repo.push(None, None).expect("second push");

        let pulled = clone.pull(None, None).expect("pull");
        assert_eq!(pulled.operation, RemoteOperation::Pull);
        assert_eq!(pulled.remote, "origin");
        assert!(pulled.conflicts.is_empty(), "{:?}", pulled.conflicts);
        assert_eq!(
            read_text(&clone_path.join("a.txt")),
            "one\ntwo\n",
            "pull must update the working tree"
        );
        assert!(clone.status().expect("status").entries.is_empty());
    }

    fn read_text(path: &std::path::Path) -> String {
        std::fs::read_to_string(path)
            .expect("read")
            .replace("\r\n", "\n")
    }
}
