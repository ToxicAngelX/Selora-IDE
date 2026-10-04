//! The error model (PLAN.md §8): every fallible operation above this crate
//! returns `Result<T, SeloraError>`. No silent failures.

/// The single error type for the Selora IDE workspace.
#[derive(Debug, thiserror::Error)]
pub enum SeloraError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("network error: {0}")]
    Net(String),

    #[error("lsp error: {0}")]
    Lsp(String),

    #[error("git error: {0}")]
    Git(String),

    #[error("selora gateway error: {0}")]
    Ai(String),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}
