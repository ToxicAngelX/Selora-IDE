//! Git error type (PLAN.md §8: every fallible operation returns a
//! `Result`; no silent failures).

/// Everything that can go wrong in the git surface.
///
/// The app shell maps this onto `SeloraError::Git` at the wiring seam
/// (`selora-git` deliberately depends on no other workspace crate, per the
/// inter-crate rules in PLAN.md §5).
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// A libgit2 operation failed.
    #[error("git: {0}")]
    Git(#[from] git2::Error),
    /// A path that must stay inside the work tree escaped it (an absolute
    /// path outside the repository, a `..` component, a `.git` component).
    #[error("`{path}` is outside the repository")]
    PathEscape { path: String },
    /// A branch, remote, or revision name failed validation before it was
    /// used. The name is echoed — it is caller input, never a credential.
    #[error("invalid {kind} name `{name}`: {reason}")]
    InvalidName {
        kind: &'static str,
        name: String,
        reason: String,
    },
    /// A network operation (`fetch` / `pull` / `push`) failed. The message is
    /// already redacted: remote URLs never carry userinfo or tokens into an
    /// error or an event, and the length is bounded.
    #[error("git {operation} failed: {message}")]
    Network {
        operation: &'static str,
        message: String,
    },
    /// A Selora-side precondition failed (empty commit message, a bare repo,
    /// no git identity configured, no remote, a conflict that needs an
    /// explicit resolution…).
    #[error("{0}")]
    Other(String),
}

impl GitError {
    /// The stable error code the shell can branch on without parsing prose.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Git(_) => "git",
            Self::PathEscape { .. } => "path_escape",
            Self::InvalidName { .. } => "invalid_name",
            Self::Network { .. } => "network",
            Self::Other(_) => "git",
        }
    }
}
