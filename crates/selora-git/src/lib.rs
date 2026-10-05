//! Selora Git — git integration via `git2` (PLAN.md §4.6, §5).
//!
//! The Lite surface (ARCHITECTURE.md §10.5): open the repository a folder
//! belongs to, snapshot its status, stage / unstage single paths, commit
//! the index, and diff one file's working tree against the index (what the
//! git panel shows). `push` / `pull` / `fetch` / remotes / branch
//! management / rebase / stash do not exist here (PLAN.md §2 non-goals).
//!
//! Everything is synchronous and cheap enough to run inside a
//! `spawn_blocking` worker — that is how the app shell invokes it (never
//! on the frame path). No shelling out to the `git` CLI.
//!
//! `selora-git` depends on no other workspace crate (PLAN.md §5
//! inter-crate rules); its [`GitError`] is mapped to
//! `SeloraError::Git` at the app wiring seam.

#![forbid(unsafe_code)]

mod diff;
mod error;
mod status;

pub use diff::{DiffLine, FileDiff, Hunk};
pub use error::GitError;
pub use status::{FileStatus, GitStatus, Slot};

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
            .map(normalize)
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
        Ok(GitStatus { branch, entries })
    }

    /// Stage `path` (working tree → index). Accepts an absolute path (the
    /// panel's rows carry `FileStatus::abs_path`) or a repo-relative one.
    /// Staging a file that no longer exists on disk records the deletion,
    /// exactly like `git add` does.
    pub fn stage(&self, path: &Path) -> Result<(), GitError> {
        let rel = self.repo_relative(path).ok_or_else(|| {
            GitError::Other(format!("`{}` is outside the repository", path.display()))
        })?;
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
        let rel = self.repo_relative(path).ok_or_else(|| {
            GitError::Other(format!("`{}` is outside the repository", path.display()))
        })?;
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
    pub fn commit(&self, message: &str) -> Result<git2::Oid, GitError> {
        let message = message.trim();
        if message.is_empty() {
            return Err(GitError::Other("commit message is empty".to_owned()));
        }
        let mut index = self.repo.index()?;
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;
        let parent = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok());
        let nothing_staged = match &parent {
            Some(commit) => commit
                .tree()
                .is_ok_and(|head_tree| head_tree.id() == tree_id),
            // Unborn HEAD: an empty tree would produce an empty root commit.
            None => tree.is_empty(),
        };
        if nothing_staged {
            return Err(GitError::Other("nothing staged to commit".to_owned()));
        }
        let signature = self.repo.signature().map_err(|err| {
            GitError::Other(format!(
                "{err} — set user.name and user.email in your git config"
            ))
        })?;
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        let oid = self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )?;
        Ok(oid)
    }

    /// Diff one file's working tree against the index (the unstaged
    /// changes — what the git panel shows; ARCHITECTURE.md assigns the
    /// panel "working tree vs index"). Untracked files diff as fully
    /// added. Accepts an absolute or repo-relative path.
    pub fn diff_workdir(&self, path: &Path) -> Result<FileDiff, GitError> {
        let rel = self.repo_relative(path).ok_or_else(|| {
            GitError::Other(format!("`{}` is outside the repository", path.display()))
        })?;
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
        let mut out = FileDiff {
            path: rel,
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

    /// Map a path onto a repo-relative path with `/` separators (the form
    /// libgit2 expects). Absolute paths must live under the workdir; a
    /// relative path is taken as already repo-relative.
    fn repo_relative(&self, path: &Path) -> Option<PathBuf> {
        let path = normalize(path);
        if path.is_absolute() {
            if let Ok(rel) = path.strip_prefix(&self.workdir) {
                return Some(forward_slashes(rel));
            }
            // Windows is case-insensitive and git reports the workdir with
            // forward slashes while callers arrive with backslashes: fold
            // case *and* separators, ignoring the workdir's trailing
            // separator, and guard against a sibling whose name merely
            // starts with the workdir's ("repo" vs "repository").
            let folded = path
                .as_os_str()
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase();
            let workdir_folded = self
                .workdir
                .as_os_str()
                .to_string_lossy()
                .replace('\\', "/")
                .trim_end_matches('/')
                .to_lowercase();
            if let Some(rest) = folded.strip_prefix(&workdir_folded) {
                if rest.is_empty() || rest.starts_with('/') {
                    let rel = rest.trim_start_matches('/');
                    return Some(PathBuf::from(rel.to_owned()));
                }
            }
            return None;
        }
        Some(forward_slashes(&path))
    }
}

/// The checked-out branch, the short id for a detached HEAD, or the
/// unborn branch name read from HEAD's symbolic target.
fn branch_name(repo: &git2::Repository) -> Option<String> {
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

/// Strip the `\\?\` extended-length prefix Windows APIs can produce, so
/// user-visible paths and git's workdir compare equal.
fn normalize(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    let stripped = text.strip_prefix(r"\\?\").unwrap_or(&text);
    PathBuf::from(stripped)
}

/// Repo-relative path with `/` separators (libgit2's path form; a no-op on
/// Unix).
fn forward_slashes(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().replace('\\', "/"))
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
        let raw = git2::Repository::init_opts(dir.path(), &mut options).expect("git init");
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
}
