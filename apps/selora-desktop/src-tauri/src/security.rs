use selora_protocol::{
    validate_external_url, validate_workspace_relative_path, ProtocolValidationError,
};
use std::path::{Path, PathBuf};

/// Workspace path guard used by the workspace adapter in the next shell slice.
#[allow(dead_code)]
pub fn resolve_workspace_path(
    root: &Path,
    relative: &str,
) -> Result<PathBuf, ProtocolValidationError> {
    validate_workspace_relative_path(relative)?;
    let root = root
        .canonicalize()
        .map_err(|_| ProtocolValidationError::InvalidWorkspacePath)?;
    let candidate = root.join(relative);
    let canonical = candidate
        .canonicalize()
        .map_err(|_| ProtocolValidationError::InvalidWorkspacePath)?;
    if canonical.starts_with(&root) {
        Ok(canonical)
    } else {
        Err(ProtocolValidationError::InvalidWorkspacePath)
    }
}

/// External URL guard retained for the future link-opening command.
#[allow(dead_code)]
pub fn validate_external(value: &str) -> Result<(), ProtocolValidationError> {
    validate_external_url(value)
}

pub fn allow_shell_navigation(url: &tauri::Url) -> bool {
    (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"))
        || (cfg!(debug_assertions)
            && url.scheme() == "http"
            && url.host_str() == Some("localhost")
            && url.port() == Some(1420))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rejects_escape_and_accepts_existing_child() {
        let root = std::env::temp_dir().join(format!("selora-shell-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).expect("mkdir");
        fs::write(root.join("src/main.rs"), "fn main() {}").expect("write");
        assert!(resolve_workspace_path(&root, "src/main.rs").is_ok());
        assert!(resolve_workspace_path(&root, "../secret").is_err());
        let _ = fs::remove_dir_all(root);
    }
}
