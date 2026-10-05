//! Git error type (PLAN.md §8: every fallible operation returns a
//! `Result`; no silent failures).

/// Everything that can go wrong in the Lite git surface.
///
/// The app shell maps this onto `SeloraError::Git` at the wiring seam
/// (`selora-git` deliberately depends on no other workspace crate, per the
/// inter-crate rules in PLAN.md §5).
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// A libgit2 operation failed.
    #[error("git: {0}")]
    Git(#[from] git2::Error),
    /// A Selora-side precondition failed (empty commit message, a path
    /// outside the repository, a bare repo, no git identity configured…).
    #[error("{0}")]
    Other(String),
}
