//! Work-tree path containment.
//!
//! Every path that reaches a writing git operation (stage, discard, resolve,
//! log filters) passes through [`checked_relative`]. It rejects absolute paths
//! outside the work tree, `..` traversal, Windows drive/UNC prefixes, and any
//! `.git` component; it then canonicalizes the deepest existing ancestor and
//! verifies the result still sits under the canonical work tree, so a symlink
//! cannot be used to reach outside either.
//!
//! Paths are returned in libgit2's form: repo-relative, `/` separators.

use crate::GitError;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Strip the `\\?\` extended-length prefix Windows APIs can produce, so
/// user-visible paths and git's workdir compare equal.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    let stripped = text.strip_prefix(r"\\?\").unwrap_or(&text);
    PathBuf::from(stripped)
}

/// Repo-relative path with `/` separators (libgit2's path form; a no-op on
/// Unix).
pub(crate) fn forward_slashes(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().replace('\\', "/"))
}

/// Lenient variant used by the read-only diff helpers: `None` when the path
/// cannot be mapped into the repository.
#[cfg(test)]
pub(crate) fn repo_relative(workdir: &Path, path: &Path) -> Option<PathBuf> {
    checked_relative(workdir, path).ok()
}

/// Map `path` onto a repo-relative path and prove it stays inside the work
/// tree. Absolute paths must live under `workdir`; relative paths are taken as
/// repo-relative but still cleaned (no `..`, no `.git`).
pub(crate) fn checked_relative(workdir: &Path, path: &Path) -> Result<PathBuf, GitError> {
    let original = normalize(path);
    let raw = if original.is_absolute() {
        match relative_to(workdir, &original) {
            Some(rel) => rel,
            None => return Err(escape(path)),
        }
    } else {
        original.clone()
    };
    let clean = clean_relative(&raw, path)?;
    let abs = workdir.join(&clean);
    verify_canonical(workdir, &abs, path)?;
    Ok(clean)
}

fn escape(path: &Path) -> GitError {
    GitError::PathEscape {
        path: path.display().to_string(),
    }
}

/// Strip a work directory prefix from an absolute path. Windows folds case
/// and both separator styles; the sibling guard ("repo" vs "repository") is
/// the component-count boundary, and the remainder keeps its original case.
fn relative_to(workdir: &Path, absolute: &Path) -> Option<PathBuf> {
    let workdir = normalize(workdir);
    let absolute = normalize(absolute);
    if let Ok(rest) = absolute.strip_prefix(&workdir) {
        if !rest.as_os_str().is_empty() {
            return Some(rest.to_path_buf());
        }
        return None;
    }
    if cfg!(windows) {
        let workdir_parts: Vec<_> = workdir.components().collect();
        let absolute_parts: Vec<_> = absolute.components().collect();
        if absolute_parts.len() <= workdir_parts.len() {
            return None;
        }
        let same_prefix = workdir_parts
            .iter()
            .zip(&absolute_parts)
            .all(|(left, right)| {
                left.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
            });
        if same_prefix {
            let rest: PathBuf = absolute_parts[workdir_parts.len()..].iter().collect();
            if !rest.as_os_str().is_empty() {
                return Some(rest);
            }
        }
    }
    None
}

fn clean_relative(rel: &Path, original: &Path) -> Result<PathBuf, GitError> {
    let mut out = PathBuf::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text.eq_ignore_ascii_case(".git") {
                    // Never let a caller address the repository metadata:
                    // that is never a user file and a write there is not a
                    // work-tree operation.
                    return Err(escape(original));
                }
                out.push(part);
            }
            Component::CurDir => {}
            // RootDir, Prefix (drive/UNC) and ParentDir are all escapes.
            _ => return Err(escape(original)),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(escape(original));
    }
    Ok(forward_slashes(&out))
}

/// Canonicalize the deepest existing ancestor of `abs` and require it to stay
/// under the canonical work tree. A path whose parents do not exist yet is
/// accepted on the lexical check alone (there is nothing to traverse).
fn verify_canonical(workdir: &Path, abs: &Path, original: &Path) -> Result<(), GitError> {
    let workdir_canon = fs::canonicalize(workdir)
        .map(|path| normalize(&path))
        .unwrap_or_else(|_| normalize(workdir));
    let workdir_folded = fold(&workdir_canon);
    let mut probe = normalize(abs);
    loop {
        if !is_within(&workdir_folded, &fold(&probe)) && fold(&probe) != workdir_folded {
            // Walked above the work tree without finding an existing entry:
            // the lexical check already passed, so there is nothing to
            // resolve.
            return Ok(());
        }
        if let Ok(canon) = fs::canonicalize(&probe) {
            let canon = normalize(&canon);
            if is_within(&workdir_folded, &fold(&canon)) {
                return Ok(());
            }
            return Err(escape(original));
        }
        match probe.parent() {
            Some(parent) => probe = parent.to_path_buf(),
            None => return Ok(()),
        }
    }
}

fn is_within(base_folded: &str, candidate_folded: &str) -> bool {
    candidate_folded == base_folded
        || candidate_folded
            .strip_prefix(base_folded)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Compare form: forward slashes, no trailing separator, lowercased only on
/// Windows (Unix paths are case-sensitive).
fn fold(path: &Path) -> String {
    let text = normalize(path).to_string_lossy().replace('\\', "/");
    let text = text.trim_end_matches('/').to_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_rejects_parent_and_git_components() {
        assert!(clean_relative(Path::new("../x"), Path::new("../x")).is_err());
        assert!(clean_relative(Path::new("a/../../x"), Path::new("a/../../x")).is_err());
        assert!(clean_relative(Path::new(".git/config"), Path::new(".git/config")).is_err());
        assert!(clean_relative(Path::new("a/.git/hooks/x"), Path::new("a/.git/hooks/x")).is_err());
        assert!(clean_relative(Path::new(""), Path::new("")).is_err());
        // `.` components are harmless; the result is normalized.
        assert_eq!(
            clean_relative(Path::new("./a/./b.txt"), Path::new("./a/./b.txt")).unwrap(),
            PathBuf::from("a/b.txt")
        );
    }

    #[test]
    fn fold_is_case_insensitive_only_on_windows() {
        let folded = fold(Path::new("C:\\Repo\\Src\\A.RS"));
        if cfg!(windows) {
            assert_eq!(folded, "c:/repo/src/a.rs");
        } else {
            assert_eq!(folded, "C:/Repo/Src/A.RS");
        }
    }
}
