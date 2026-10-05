//! `file://` URI ↔ filesystem path, without a URL crate (PLAN.md's "thin
//! client" posture: one less dependency for a two-direction mapping).
//!
//! Windows paths are drive-letter paths (`C:\x y\a.rs` ↔
//! `file:///C:/x%20y/a.rs`); percent-encoding covers everything outside the
//! RFC 3986 unreserved set so URIs stay ASCII-clean and servers can parse
//! them with any URI library.

use std::path::{Path, PathBuf};

/// Filesystem path → `file://` URI string.
///
/// Percent-encoding covers everything outside the RFC 3986
/// unreserved/path-allowed set, so URIs stay ASCII-clean and any server's
/// URI parser can read them back.
pub fn path_to_uri(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    // A POSIX path already carries its leading '/', so the prefix stays
    // `file://` (three slashes total); a Windows drive path gets the third
    // slash from the prefix.
    let prefixed = if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        format!("file:///{normalized}")
    };
    let mut uri = String::with_capacity(prefixed.len());
    for byte in prefixed.as_bytes() {
        if is_uri_safe(*byte) {
            uri.push(*byte as char);
        } else {
            uri.push_str(&format!("%{:02X}", byte));
        }
    }
    uri
}

/// `file://` URI string → filesystem path. `None` for non-file URIs or
/// paths that are not absolute (we never resolve relative LSP URIs).
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let (path_part, _fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (path_part, _query) = path_part.split_once('?').unwrap_or((path_part, ""));
    let decoded = percent_decode(path_part);
    // `file:///C:/x` → "C:/x" (Windows drive); `file:///home/x` → "/home/x".
    // On Windows a leading "/C:/" is the same drive-letter form.
    let path = if let Some(drive) = decoded
        .strip_prefix('/')
        .filter(|s| s.as_bytes().get(1) == Some(&b':'))
    {
        drive.to_owned()
    } else {
        decoded
    };
    let path = PathBuf::from(path);
    path.is_absolute().then(|| normalize_separators(path))
}

/// Unreserved characters plus the path-allowed sub-delims and `:`
/// (RFC 3986 §3.3) — everything else is percent-encoded.
fn is_uri_safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'_'
                | b'.'
                | b'~'
                | b'/'
                | b':'
                | b'@'
                | b'&'
                | b'='
                | b'+'
                | b'$'
                | b','
                | b';'
        )
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            // Valid percent-encoding decodes; anything else (a lone '%' or
            // a non-hex escape) is kept as literal bytes.
            let decoded = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(value) = decoded {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// URIs always use `/`; on Windows convert back to the platform separator.
fn normalize_separators(path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().replace('/', "\\"))
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_plain_paths() {
        for path in ["/home/user/a.rs", "/repo/Cargo.toml"] {
            let uri = path_to_uri(Path::new(path));
            assert_eq!(uri, format!("file://{path}"));
            if cfg!(windows) {
                // A POSIX path is not a valid Windows path: converting it
                // back declines (None) rather than inventing a drive.
                assert_eq!(uri_to_path(&uri), None);
            } else {
                assert_eq!(uri_to_path(&uri).as_deref(), Some(Path::new(path)));
            }
        }
    }

    #[test]
    fn windows_drive_letter_paths() {
        let path = Path::new("C:\\repo\\crates\\buffer.rs");
        let uri = path_to_uri(path);
        assert_eq!(uri, "file:///C:/repo/crates/buffer.rs");
        if cfg!(windows) {
            assert_eq!(uri_to_path(&uri).as_deref(), Some(path));
        } else {
            assert_eq!(
                uri_to_path(&uri).as_deref(),
                Some(Path::new("C:/repo/crates/buffer.rs"))
            );
        }
    }

    #[test]
    fn percent_encodes_spaces_and_unicode_and_decodes_back() {
        let path = if cfg!(windows) {
            PathBuf::from("C:\\Users\\Toxic\\x y\\main.rs")
        } else {
            PathBuf::from("/home/toxic/x y/main.rs")
        };
        let uri = path_to_uri(&path);
        assert!(uri.contains("x%20y"));
        // Non-ASCII is percent-encoded UTF-8 (ASCII-clean URIs).
        let unicode = Path::new("/tmp/café.rs");
        let uri = path_to_uri(unicode);
        assert_eq!(uri, "file:///tmp/caf%C3%A9.rs");
        if !cfg!(windows) {
            assert_eq!(uri_to_path(&uri).as_deref(), Some(unicode));
        }
        assert_eq!(
            uri_to_path(&path_to_uri(&path)).as_deref(),
            Some(path.as_path())
        );
    }

    #[test]
    fn rejects_non_file_and_relative_uris_and_keeps_broken_escapes() {
        assert!(uri_to_path("https://example.com/a.rs").is_none());
        assert!(uri_to_path("file://relative").is_none());
        // A lone '%' that isn't an escape survives as a literal byte.
        let broken = uri_to_path("file:///a%b.rs");
        if cfg!(windows) {
            assert_eq!(broken, None); // POSIX path, not a Windows path
        } else {
            assert_eq!(broken.as_deref(), Some(Path::new("/a%b.rs")));
        }
    }

    #[test]
    fn strips_fragments_and_queries() {
        let stripped = uri_to_path("file:///a.rs?foo=1#L12");
        if cfg!(windows) {
            assert_eq!(stripped, None);
        } else {
            assert_eq!(stripped.as_deref(), Some(Path::new("/a.rs")));
        }
    }
}
