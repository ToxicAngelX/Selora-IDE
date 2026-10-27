//! The workspace model (ARCHITECTURE.md §3.3): what "Open Folder" produces
//! and the unit of session the user can return to. Pure data — no IO here.

use std::path::PathBuf;

/// A opened folder + the last-known editor session inside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Workspace {
    /// The folder root (what the user picked in Open Folder).
    pub root: PathBuf,
    /// Last-known open file set, in tab order.
    pub open_files: Vec<PathBuf>,
    /// The path of the active editor, if any.
    pub active_path: Option<PathBuf>,
}

impl Workspace {
    /// A workspace rooted at `root` with nothing open.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            open_files: Vec::new(),
            active_path: None,
        }
    }

    /// Open (or focus) `path`: moves it to the end of `open_files` and makes
    /// it active. Duplicate opens focus rather than duplicate.
    pub fn open(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.open_files.retain(|p| *p != path);
        self.open_files.push(path.clone());
        self.active_path = Some(path);
    }

    /// Close `path`; the active editor falls back to the most recent
    /// remaining open file.
    pub fn close(&mut self, path: &PathBuf) {
        self.open_files.retain(|p| p != path);
        self.active_path = match &self.active_path {
            Some(active) if active == path => self.open_files.last().cloned(),
            other => other.clone(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_focuses_and_orders() {
        let mut ws = Workspace::new("/repo");
        ws.open("a.rs");
        ws.open("b.rs");
        ws.open("a.rs");
        assert_eq!(
            ws.open_files,
            vec![PathBuf::from("b.rs"), PathBuf::from("a.rs")]
        );
        assert_eq!(
            ws.active_path.as_deref(),
            Some(std::path::Path::new("a.rs"))
        );
    }

    #[test]
    fn close_falls_back_to_most_recent() {
        let mut ws = Workspace::new("/repo");
        ws.open("a.rs");
        ws.open("b.rs");
        ws.close(&PathBuf::from("b.rs"));
        assert_eq!(
            ws.active_path.as_deref(),
            Some(std::path::Path::new("a.rs"))
        );
        ws.close(&PathBuf::from("a.rs"));
        assert_eq!(ws.active_path, None);
    }
}
