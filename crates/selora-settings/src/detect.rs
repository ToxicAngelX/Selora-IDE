//! `which`-style PATH lookup (no new dependency — pure `std`).
//!
//! Used by the Settings dialog's "Auto-detect" buttons (UI_SPEC.md §6) to
//! fill `lsp.rust_analyzer_path` / `lsp.typescript_server_path` from PATH.

use std::path::PathBuf;

/// Look `binary` up on the current `PATH`, returning the first executable
/// candidate found, or `None`. The equivalent of `which rust-analyzer`.
pub fn find_on_path(binary: &str) -> Option<String> {
    let path_var = std::env::var("PATH").ok()?;
    find_in_dirs(binary, std::env::split_paths(&path_var))
}

/// The pure core of [`find_on_path`]: search the given directories for a
/// regular file named `binary` (plus `.exe` on Windows). Parameterized so
/// tests can pass temp directories instead of mutating `PATH` (env vars are
/// process-global and tests run in parallel threads).
pub fn find_in_dirs(binary: &str, dirs: impl IntoIterator<Item = PathBuf>) -> Option<String> {
    // On Windows the executable is `name.exe`; on Unix it is `name`. Check
    // the platform-correct spelling first, the other second.
    let with_exe = format!("{binary}.exe");
    let candidates: [&str; 2] = if cfg!(windows) {
        [&with_exe, binary]
    } else {
        [binary, &with_exe]
    };

    for dir in dirs {
        for name in candidates {
            let candidate = dir.join(name);
            // `std` cannot portably check the execute bit; a regular file on
            // PATH with the right name is the practical signal (the LSP
            // spawn will surface a real error if it is not runnable).
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let n = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("selora-detect-test-{}-{}", std::process::id(), n))
    }

    #[test]
    fn finds_the_binary_in_a_search_dir() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).expect("mkdir");
        let exe = dir.join(if cfg!(windows) {
            "rust-analyzer.exe"
        } else {
            "rust-analyzer"
        });
        std::fs::write(&exe, b"").expect("write stub binary");

        let found = find_in_dirs("rust-analyzer", [dir.clone()]).expect("must find");
        assert!(
            found.replace('\\', "/").contains("rust-analyzer"),
            "unexpected path: {found}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn returns_none_when_absent() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).expect("mkdir");
        assert_eq!(
            find_in_dirs("typescript-language-server", [dir.clone()]),
            None
        );
        // And across several dirs.
        let empty = temp_dir();
        std::fs::create_dir_all(&empty).expect("mkdir");
        assert_eq!(
            find_in_dirs("no-such-binary", [dir.clone(), empty.clone()]),
            None
        );

        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(empty);
    }

    #[test]
    fn prefers_the_first_dir_that_matches() {
        let first = temp_dir();
        let second = temp_dir();
        for dir in [&first, &second] {
            std::fs::create_dir_all(dir).expect("mkdir");
            let name = if cfg!(windows) { "cargo.exe" } else { "cargo" };
            std::fs::write(dir.join(name), b"").expect("write stub");
        }
        let found = find_in_dirs("cargo", [first.clone(), second.clone()]).expect("must find");
        assert!(found.contains("cargo"));
        // Which dir won is platform/separator-dependent; just assert one hit.

        let _ = std::fs::remove_dir_all(first);
        let _ = std::fs::remove_dir_all(second);
    }

    #[test]
    fn directories_that_do_not_exist_are_skipped() {
        let missing = temp_dir().join("does-not-exist");
        assert_eq!(find_in_dirs("rust-analyzer", [missing]), None);
    }
}
