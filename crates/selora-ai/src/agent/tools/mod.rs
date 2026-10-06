//! Tool implementations (ARCHITECTURE.md §17.1) — Phase 5b's file.
//!
//! One file per tool family: the fs tools ([`fs`]: `read_file`, `list_dir`,
//! `fuzzy_find`, `search_files`, `write_file`, `edit_file`, `delete_file`),
//! the bash tool ([`bash`], §17.5), and [`web`] (Phase 5e). The guards live
//! next to the code they bound — every one is unit-tested and cites its §:
//! - **Path rooting** (§17.2): [`fs::resolve_rooted`] — every path tool
//!   resolves against the workspace root; `..` traversal, absolute-path
//!   escapes, and symlink/junction escapes are rejected as tool-result
//!   errors the model can see and correct.
//! - **Bounded I/O** (§17.2): [`fs::MAX_READ_BYTES`] (4MB reads and writes),
//!   [`MAX_OUTPUT_BYTES`] (256KB tool output, truncated with a notice).
//! - **Bash env** (§17.5): [`bash::build_child_env`] — a fixed allowlist;
//!   the Selora key and any app secret can never reach the child.
//! - **Bash timeout** (§17.5): [`bash::DEFAULT_TIMEOUT_SECS`] /
//!   [`bash::MAX_TIMEOUT_SECS`]; on timeout the child is killed.

pub mod bash;
pub mod fs;
pub mod web;

/// §17.2 "Bounded I/O": the 256KB cap on tool output (bash stdout+stderr,
/// list/search/fuzzy listings), truncated with a notice the model sees.
///
/// `read_file` is bounded by [`fs::MAX_READ_BYTES`] instead (a read's whole
/// point is returning file content); `web_fetch` caps itself (Phase 5e's
/// file, same limit).
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;

#[cfg(test)]
pub(crate) mod testutil {
    //! Shared fixtures: a unique temp workspace per test (same pattern as
    //! `selora-fs`'s tests).

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A unique fixture directory under the system temp dir.
    pub(crate) fn temp_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = format!(
            "selora-agent-5b-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).expect("create fixture root");
        dir
    }

    /// Write a fixture file (creating parent directories).
    pub(crate) fn touch(root: &std::path::Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }
        std::fs::write(&path, contents).expect("create fixture file");
    }
}
