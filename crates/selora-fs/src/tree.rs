//! File tree data layer (Phase 2b).
//!
//! Three concerns live here:
//!
//! 1. [`build_tree`] — walking a folder into a [`FileTreeNode`] via the
//!    `ignore` crate (ARCHITECTURE.md §4.3: `.gitignore` respected, hidden
//!    entries skipped). Synchronous and pure; the app shell calls it off
//!    the frame path and hands the finished tree to the UI through
//!    `AppMessage::FileTreeUpdated`.
//! 2. [`FileTreeView`] — the UI state (expanded set + selection) and the
//!    flatten-to-rows function that feeds the virtualized sidebar
//!    rendering (ARCHITECTURE.md §4.4: rows of
//!    `(depth, name, is_dir, is_expanded)`).
//! 3. The tree-context filesystem operations (New File, New Folder,
//!    Rename, Delete) used by the sidebar's context menu.
//!
//! Folders with more than [`MAX_CHILDREN`] entries are returned partial
//! with a "load more" marker (ARCHITECTURE.md §4.4 — rendering 50k nodes
//! is a v2 problem).

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// Per-directory cap after which [`build_tree`] returns a partial node
/// (ARCHITECTURE.md §4.4).
pub const MAX_CHILDREN: usize = 2000;

/// Label rendered by the view for a truncated directory's marker row.
pub const LOAD_MORE_LABEL: &str = "Load more";

// -----------------------------------------------------------------------------
// FileTreeNode
// -----------------------------------------------------------------------------

/// One node of the workspace file tree (ARCHITECTURE.md §4.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTreeNode {
    /// File or directory name (the last path component).
    pub name: String,
    /// Absolute or workspace-rooted path.
    pub path: PathBuf,
    /// Directories have children; files never do.
    pub is_dir: bool,
    /// Sorted children: directories first, then files, alphabetical.
    pub children: Vec<FileTreeNode>,
    /// `true` when the directory had more than [`MAX_CHILDREN`] entries
    /// and `children` was capped. The view renders a "load more" marker
    /// row after the children (ARCHITECTURE.md §4.4).
    pub truncated: bool,
}

impl FileTreeNode {
    /// Number of direct children (0 for files).
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Direct child by name, if present.
    pub fn child(&self, name: &str) -> Option<&FileTreeNode> {
        self.children.iter().find(|child| child.name == name)
    }
}

// -----------------------------------------------------------------------------
// build_tree
// -----------------------------------------------------------------------------

/// Internal walk entry; children reference parents by index.
struct ArenaNode {
    name: String,
    path: PathBuf,
    is_dir: bool,
    children: Vec<usize>,
}

/// Build the file tree for `root` with the default [`MAX_CHILDREN`] cap.
///
/// `.gitignore` rules are respected (including outside a git repository,
/// so a plain folder behaves like a workspace) and hidden entries —
/// dot-files and dot-directories such as `.git` and `.gitignore` itself —
/// are skipped (ARCHITECTURE.md §4.3). A missing or unreadable `root`
/// yields an empty tree, not an error: the caller shows an empty sidebar.
pub fn build_tree(root: &Path) -> FileTreeNode {
    build_tree_with_limit(root, MAX_CHILDREN)
}

/// [`build_tree`] with an explicit per-directory cap (used by tests and,
/// later, by "load more").
pub fn build_tree_with_limit(root: &Path, max_children: usize) -> FileTreeNode {
    let root_name = file_name_of(root);
    if !root.exists() {
        return FileTreeNode {
            name: root_name,
            path: root.to_path_buf(),
            is_dir: true,
            children: Vec::new(),
            truncated: false,
        };
    }
    if !root.is_dir() {
        return FileTreeNode {
            name: root_name,
            path: root.to_path_buf(),
            is_dir: false,
            children: Vec::new(),
            truncated: false,
        };
    }

    // Arena of walk entries; children reference parents by index.
    let mut arena: Vec<ArenaNode> = vec![ArenaNode {
        name: root_name,
        path: root.to_path_buf(),
        is_dir: true,
        children: Vec::new(),
    }];
    // Stack of arena indices of the currently-open directory chain; the
    // walker yields depth-first pre-order, so an entry at depth `d` is a
    // child of the directory pushed at depth `d - 1`.
    let mut stack: Vec<usize> = vec![0];

    let walker = WalkBuilder::new(root)
        .hidden(true) // skip dot-entries (includes `.git`)
        .git_ignore(true) // respect .gitignore
        .require_git(false) // …also in non-git folders
        .ignore(true) // respect .ignore files
        .build();

    for entry in walker.flatten() {
        if entry.depth() == 0 {
            continue; // the root itself
        }
        let depth = entry.depth();
        while stack.len() > depth {
            stack.pop();
        }
        if stack.len() != depth {
            // Walker invariant broken (never observed with `ignore`); skip
            // rather than attach the entry to the wrong parent.
            continue;
        }
        let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
        let idx = arena.len();
        arena.push(ArenaNode {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().to_path_buf(),
            is_dir,
            children: Vec::new(),
        });
        if let Some(&parent) = stack.last() {
            arena[parent].children.push(idx);
        }
        stack.push(idx);
    }

    materialize(&arena, 0, max_children)
}

/// Sort a node's children (dirs first, then files, alphabetical),
/// apply the cap, and convert the arena into owned [`FileTreeNode`]s.
fn materialize(arena: &[ArenaNode], idx: usize, max_children: usize) -> FileTreeNode {
    let node = &arena[idx];
    let mut children: Vec<usize> = node.children.clone();
    children.sort_by(|&a, &b| compare_children(arena, a, b));
    let truncated = node.is_dir && children.len() > max_children;
    if children.len() > max_children {
        children.truncate(max_children);
    }
    FileTreeNode {
        name: node.name.clone(),
        path: node.path.clone(),
        is_dir: node.is_dir,
        children: children
            .into_iter()
            .map(|child| materialize(arena, child, max_children))
            .collect(),
        truncated,
    }
}

/// Directories first, then files; alphabetical (case-insensitive, exact
/// as the tie-break).
fn compare_children(arena: &[ArenaNode], a: usize, b: usize) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (x, y) = (&arena[a], &arena[b]);
    match (x.is_dir, y.is_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => x
            .name
            .to_lowercase()
            .cmp(&y.name.to_lowercase())
            .then_with(|| x.name.cmp(&y.name)),
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

// -----------------------------------------------------------------------------
// FileTreeView
// -----------------------------------------------------------------------------

/// What a flattened row represents (ARCHITECTURE.md §4.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeRowKind {
    /// A real file or directory entry.
    Node,
    /// The "load more" marker shown after a truncated directory's children.
    LoadMore { parent: PathBuf },
}

/// One flattened row for the virtualized tree rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    /// 0 for the root row, +1 per directory level.
    pub depth: usize,
    /// Node path (for `LoadMore`, the parent directory's path).
    pub path: PathBuf,
    /// Display name.
    pub name: String,
    /// Directory rows are collapsible.
    pub is_dir: bool,
    /// Whether a directory row is expanded (always `false` for files and
    /// marker rows).
    pub is_expanded: bool,
    /// Node vs "load more" marker.
    pub kind: TreeRowKind,
}

/// UI state for the tree: the current tree, the expanded set, and the
/// selected row (ARCHITECTURE.md §3.1 `AppState::file_tree`).
///
/// Pure data — no egui dependency — so the app state, the sidebar, and
/// tests all share one implementation.
#[derive(Clone, Debug, Default)]
pub struct FileTreeView {
    /// The current tree; `None` until a folder is opened.
    pub root: Option<FileTreeNode>,
    /// Expanded directory paths. The root is expanded by [`FileTreeView::set_root`].
    pub expanded: BTreeSet<PathBuf>,
    /// Selected row path (single selection; multi-select is out of Lite).
    pub selected: Option<PathBuf>,
}

impl FileTreeView {
    /// Install a freshly built tree, resetting expansion and selection.
    /// The root folder is expanded so its contents are visible
    /// (ARCHITECTURE.md §4.3: "lazily expanded").
    pub fn set_root(&mut self, root: FileTreeNode) {
        self.expanded.clear();
        self.expanded.insert(root.path.clone());
        self.selected = None;
        self.root = Some(root);
    }

    /// The root folder's path, if a tree is loaded.
    pub fn root_path(&self) -> Option<&Path> {
        self.root.as_ref().map(|node| node.path.as_path())
    }

    /// Whether `path` is an expanded directory.
    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// Expand a directory. No-op for files or unknown paths.
    pub fn expand(&mut self, path: &Path) {
        if let Some(node) = self.node(path) {
            if node.is_dir {
                self.expanded.insert(path.to_path_buf());
            }
        }
    }

    /// Collapse a directory.
    pub fn collapse(&mut self, path: &Path) {
        self.expanded.remove(path);
    }

    /// Toggle a directory's expansion. No-op for files or unknown paths.
    pub fn toggle(&mut self, path: &Path) {
        if let Some(node) = self.node(path) {
            if node.is_dir && !self.expanded.remove(path) {
                self.expanded.insert(path.to_path_buf());
            }
        }
    }

    /// Collapse every directory (the root included).
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }

    /// Select a row if it exists in the tree.
    pub fn select(&mut self, path: &Path) {
        if self.node(path).is_some() {
            self.selected = Some(path.to_path_buf());
        }
    }

    /// Flatten the tree into renderable rows: only expanded directories
    /// contribute their children (ARCHITECTURE.md §4.4).
    pub fn rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        if let Some(root) = &self.root {
            self.flatten_into(root, 0, &mut rows);
        }
        rows
    }

    /// Find a node by path (a linear prefix-guided walk; the tree is
    /// shallow and this runs once per interaction, not per row).
    pub fn node(&self, path: &Path) -> Option<&FileTreeNode> {
        fn walk<'a>(node: &'a FileTreeNode, path: &Path) -> Option<&'a FileTreeNode> {
            if node.path == path {
                return Some(node);
            }
            node.children
                .iter()
                .filter(|child| path.starts_with(&child.path))
                .find_map(|child| walk(child, path))
        }
        self.root.as_ref().and_then(|root| walk(root, path))
    }

    fn flatten_into(&self, node: &FileTreeNode, depth: usize, rows: &mut Vec<TreeRow>) {
        let is_expanded = node.is_dir && self.expanded.contains(&node.path);
        rows.push(TreeRow {
            depth,
            path: node.path.clone(),
            name: node.name.clone(),
            is_dir: node.is_dir,
            is_expanded,
            kind: TreeRowKind::Node,
        });
        if is_expanded {
            for child in &node.children {
                self.flatten_into(child, depth + 1, rows);
            }
            if node.truncated {
                rows.push(TreeRow {
                    depth: depth + 1,
                    path: node.path.clone(),
                    name: LOAD_MORE_LABEL.to_owned(),
                    is_dir: false,
                    is_expanded: false,
                    kind: TreeRowKind::LoadMore {
                        parent: node.path.clone(),
                    },
                });
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Tree filesystem operations (context menu)
// -----------------------------------------------------------------------------

/// Error type for the tree filesystem operations.
///
/// The plan's error contract is `Result<T, SeloraError>` (PLAN.md §8), but
/// `selora-fs` does not depend on `selora-core` (Cargo.toml is
/// orchestrator-owned), so the operations return this crate-local error.
/// It implements [`std::error::Error`] and maps one-to-one onto
/// `SeloraError::Io` / `SeloraError::Other` — see the Phase 2b report for
/// the proposed `selora-app` wiring.
#[derive(Debug)]
pub enum FsError {
    /// The underlying filesystem operation failed.
    Io(std::io::Error),
    /// Create/rename target already exists.
    PathExists(PathBuf),
    /// Delete/rename source does not exist.
    PathNotFound(PathBuf),
    /// The supplied file name is empty or contains path separators.
    InvalidName(String),
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsError::Io(err) => write!(f, "io error: {err}"),
            FsError::PathExists(path) => write!(f, "path already exists: {}", path.display()),
            FsError::PathNotFound(path) => write!(f, "path not found: {}", path.display()),
            FsError::InvalidName(name) => write!(f, "invalid file name: {name}"),
        }
    }
}

impl std::error::Error for FsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FsError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for FsError {
    fn from(err: std::io::Error) -> Self {
        FsError::Io(err)
    }
}

/// Result alias for the tree filesystem operations.
pub type FsResult<T> = Result<T, FsError>;

/// A file name that can be joined onto a directory without escaping it.
fn validate_name(name: &str) -> FsResult<()> {
    if name.trim().is_empty() {
        Err(FsError::InvalidName("name is empty".to_owned()))
    } else if name == "." || name == ".." {
        Err(FsError::InvalidName(
            "`{name}` is not a valid name".to_owned(),
        ))
    } else if name.contains('/') || name.contains('\\') {
        Err(FsError::InvalidName(format!(
            "name must not contain path separators: `{name}`"
        )))
    } else {
        Ok(())
    }
}

/// Create an empty file in `parent`. Fails if the name is invalid or the
/// target exists. Returns the created path.
pub fn create_file(parent: &Path, name: &str) -> FsResult<PathBuf> {
    validate_name(name)?;
    let path = parent.join(name);
    if path.exists() {
        return Err(FsError::PathExists(path));
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    Ok(path)
}

/// Create a directory in `parent`. Fails if the name is invalid or the
/// target exists. Returns the created path.
pub fn create_dir(parent: &Path, name: &str) -> FsResult<PathBuf> {
    validate_name(name)?;
    let path = parent.join(name);
    if path.exists() {
        return Err(FsError::PathExists(path));
    }
    std::fs::create_dir(&path)?;
    Ok(path)
}

/// Rename a file or directory within its parent directory to `new_name`.
/// Fails if the name is invalid, the source is missing, or the target
/// exists. Returns the new path.
pub fn rename(from: &Path, new_name: &str) -> FsResult<PathBuf> {
    validate_name(new_name)?;
    let parent = from
        .parent()
        .ok_or_else(|| FsError::InvalidName("no parent directory".to_owned()))?;
    let to = parent.join(new_name);
    if to == from {
        return Ok(to); // renaming to itself is a no-op
    }
    if !from.exists() {
        return Err(FsError::PathNotFound(from.to_path_buf()));
    }
    if to.exists() {
        return Err(FsError::PathExists(to));
    }
    std::fs::rename(from, &to)?;
    Ok(to)
}

/// Delete a file or a directory tree from disk.
pub fn delete_path(path: &Path) -> FsResult<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => FsError::PathNotFound(path.to_path_buf()),
        _ => FsError::Io(err),
    })?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A unique fixture directory under the system temp dir.
    fn temp_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = format!(
            "selora-fs-tree-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).expect("create fixture root");
        dir
    }

    fn touch(root: &Path, rel: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }
        std::fs::write(&path, b"fixture").expect("create fixture file");
    }

    fn touch_owned(root: &Path, rel: String) {
        touch(root, &rel);
    }

    fn names(children: &[FileTreeNode]) -> Vec<String> {
        children.iter().map(|child| child.name.clone()).collect()
    }

    // -- build_tree -----------------------------------------------------------

    #[test]
    fn build_tree_respects_gitignore() {
        let root = temp_root("gitignore");
        touch(&root, ".gitignore");
        touch(&root, "kept.txt");
        touch(&root, "ignored.txt");
        touch(&root, "src/kept.rs");
        touch(&root, "src/ignored.rs");
        std::fs::write(root.join(".gitignore"), "ignored.txt\nsrc/ignored.rs\n")
            .expect("write gitignore");

        let tree = build_tree(&root);
        let root_names = names(&tree.children);
        assert!(
            !root_names.contains(&"ignored.txt".to_owned()),
            "gitignored file must not appear: {root_names:?}"
        );
        let src = tree.child("src").expect("src dir present");
        let src_names = names(&src.children);
        assert_eq!(src_names, vec!["kept.rs".to_owned()]);
    }

    #[test]
    fn build_tree_skips_hidden_entries() {
        let root = temp_root("hidden");
        touch(&root, "visible.txt");
        touch(&root, ".hidden.txt");
        touch(&root, ".git/config");

        let tree = build_tree(&root);
        let root_names = names(&tree.children);
        assert_eq!(root_names, vec!["visible.txt".to_owned()]);
    }

    #[test]
    fn build_tree_sorts_dirs_first_then_files_alphabetical() {
        let root = temp_root("sort");
        touch(&root, "b.txt");
        touch(&root, "A.txt");
        touch(&root, "zeta/mod.rs");
        touch(&root, "Alpha/lib.rs");
        touch(&root, "Cargo.toml");

        let tree = build_tree(&root);
        // Case-insensitive alphabetical: Alpha < zeta, a.txt? A.txt < b.txt
        assert_eq!(
            names(&tree.children),
            vec![
                "Alpha".to_owned(),
                "zeta".to_owned(),
                "A.txt".to_owned(),
                "b.txt".to_owned(),
                "Cargo.toml".to_owned()
            ]
        );
        assert!(tree.child("Alpha").expect("Alpha").is_dir);
        assert!(!tree.child("A.txt").expect("A.txt").is_dir);
    }

    #[test]
    fn build_tree_returns_partial_children_with_marker() {
        let root = temp_root("cap");
        for i in 0..8 {
            touch_owned(&root, format!("f{i}.txt"));
        }

        let tree = build_tree_with_limit(&root, 3);
        assert!(tree.truncated, "root must be marked truncated");
        assert_eq!(tree.children.len(), 3);
        // The alphabetically-first chunk is kept.
        assert_eq!(
            names(&tree.children),
            vec![
                "f0.txt".to_owned(),
                "f1.txt".to_owned(),
                "f2.txt".to_owned()
            ]
        );
    }

    #[test]
    fn build_tree_default_cap_is_2000() {
        assert_eq!(MAX_CHILDREN, 2000);
    }

    #[test]
    fn build_tree_missing_root_is_empty() {
        let tree = build_tree(&temp_root("missing").join("nope"));
        assert!(tree.is_dir);
        assert!(tree.children.is_empty());
        assert!(!tree.truncated);
    }

    // -- FileTreeView flattening ----------------------------------------------

    fn fixture_tree() -> FileTreeNode {
        fn dir(path: &str, children: Vec<FileTreeNode>) -> FileTreeNode {
            FileTreeNode {
                name: Path::new(path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                path: PathBuf::from(path),
                is_dir: true,
                children,
                truncated: false,
            }
        }
        fn file(path: &str) -> FileTreeNode {
            FileTreeNode {
                name: Path::new(path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                path: PathBuf::from(path),
                is_dir: false,
                children: Vec::new(),
                truncated: false,
            }
        }
        dir(
            "/proj",
            vec![
                dir(
                    "/proj/src",
                    vec![
                        file("/proj/src/main.rs"),
                        dir("/proj/src/ui", vec![file("/proj/src/ui/theme.rs")]),
                    ],
                ),
                file("/proj/Cargo.toml"),
            ],
        )
    }

    fn row_summary(view: &FileTreeView) -> Vec<(usize, String, bool, bool)> {
        view.rows()
            .into_iter()
            .map(|row| (row.depth, row.name, row.is_dir, row.is_expanded))
            .collect()
    }

    #[test]
    fn flatten_root_only_shows_top_level() {
        let mut view = FileTreeView::default();
        view.set_root(fixture_tree());
        assert_eq!(
            row_summary(&view),
            vec![
                (0, "proj".to_owned(), true, true),
                (1, "src".to_owned(), true, false),
                (1, "Cargo.toml".to_owned(), false, false),
            ]
        );
    }

    #[test]
    fn flatten_expands_and_collapses() {
        let mut view = FileTreeView::default();
        view.set_root(fixture_tree());
        view.expand(Path::new("/proj/src"));
        assert_eq!(
            row_summary(&view),
            vec![
                (0, "proj".to_owned(), true, true),
                (1, "src".to_owned(), true, true),
                (2, "main.rs".to_owned(), false, false),
                (2, "ui".to_owned(), true, false),
                (1, "Cargo.toml".to_owned(), false, false),
            ]
        );
        view.toggle(Path::new("/proj/src"));
        assert_eq!(
            row_summary(&view),
            vec![
                (0, "proj".to_owned(), true, true),
                (1, "src".to_owned(), true, false),
                (1, "Cargo.toml".to_owned(), false, false),
            ]
        );
    }

    #[test]
    fn toggle_is_noop_for_files_and_unknown_paths() {
        let mut view = FileTreeView::default();
        view.set_root(fixture_tree());
        view.toggle(Path::new("/proj/Cargo.toml"));
        view.toggle(Path::new("/does/not/exist"));
        assert_eq!(view.rows().len(), 3);
    }

    #[test]
    fn flatten_emits_load_more_marker_after_truncated_children() {
        let mut truncated_src = FileTreeNode {
            name: "src".to_owned(),
            path: PathBuf::from("/proj/src"),
            is_dir: true,
            children: vec![FileTreeNode {
                name: "a.rs".to_owned(),
                path: PathBuf::from("/proj/src/a.rs"),
                is_dir: false,
                children: Vec::new(),
                truncated: false,
            }],
            truncated: true,
        };
        let mut root = fixture_tree();
        // Replace src with the truncated variant.
        for child in &mut root.children {
            if child.name == "src" {
                std::mem::swap(child, &mut truncated_src);
            }
        }

        let mut view = FileTreeView::default();
        view.set_root(root);
        view.expand(Path::new("/proj/src"));
        let rows = view.rows();
        let marker = rows
            .iter()
            .find(|row| {
                row.kind
                    == TreeRowKind::LoadMore {
                        parent: PathBuf::from("/proj/src"),
                    }
            })
            .expect("marker row after truncated children");
        assert_eq!(marker.name, LOAD_MORE_LABEL);
        assert_eq!(marker.depth, 2);
        // proj, src, a.rs, marker, Cargo.toml — sibling files still follow.
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn select_and_node_lookup() {
        let mut view = FileTreeView::default();
        view.set_root(fixture_tree());
        view.select(Path::new("/proj/src/main.rs"));
        assert_eq!(view.selected, Some(PathBuf::from("/proj/src/main.rs")));
        view.select(Path::new("/nope"));
        assert_eq!(view.selected, Some(PathBuf::from("/proj/src/main.rs")));
        assert!(view.node(Path::new("/proj/src/ui")).is_some());
        assert!(view.node(Path::new("/proj/nope")).is_none());
        assert_eq!(view.root_path(), Some(Path::new("/proj")));
    }

    // -- fs operations ---------------------------------------------------------

    #[test]
    fn create_file_and_dir_succeed_then_conflict() {
        let root = temp_root("create");
        let created = create_file(&root, "new.rs").expect("create file");
        assert_eq!(created, root.join("new.rs"));
        assert!(created.exists());
        assert!(matches!(
            create_file(&root, "new.rs"),
            Err(FsError::PathExists(_))
        ));

        let dir = create_dir(&root, "nested").expect("create dir");
        assert!(dir.is_dir());
        assert!(matches!(
            create_dir(&root, "nested"),
            Err(FsError::PathExists(_))
        ));
    }

    #[test]
    fn create_rejects_invalid_names() {
        let root = temp_root("names");
        for bad in ["", "  ", "..", ".", "a/b", "a\\b"] {
            assert!(
                matches!(create_file(&root, bad), Err(FsError::InvalidName(_))),
                "`{bad}` must be rejected"
            );
        }
    }

    #[test]
    fn rename_moves_and_reports_conflicts() {
        let root = temp_root("rename");
        touch(&root, "old.rs");
        let to = rename(&root.join("old.rs"), "new.rs").expect("rename");
        assert_eq!(to, root.join("new.rs"));
        assert!(to.exists());
        assert!(!root.join("old.rs").exists());

        touch(&root, "other.rs");
        assert!(matches!(
            rename(&root.join("new.rs"), "other.rs"),
            Err(FsError::PathExists(_))
        ));
        assert!(matches!(
            rename(&root.join("ghost.rs"), "x.rs"),
            Err(FsError::PathNotFound(_))
        ));
    }

    #[test]
    fn rename_directories() {
        let root = temp_root("rename-dir");
        touch(&root, "pkg/inner.rs");
        let to = rename(&root.join("pkg"), "crate").expect("rename dir");
        assert!(to.join("inner.rs").exists());
    }

    #[test]
    fn delete_removes_files_and_directories() {
        let root = temp_root("delete");
        touch(&root, "file.txt");
        touch(&root, "dir/child.txt");
        delete_path(&root.join("file.txt")).expect("delete file");
        assert!(!root.join("file.txt").exists());
        delete_path(&root.join("dir")).expect("delete dir");
        assert!(!root.join("dir").exists());
        assert!(matches!(
            delete_path(&root.join("file.txt")),
            Err(FsError::PathNotFound(_))
        ));
    }

    #[test]
    fn fs_error_display_is_informative() {
        let err = FsError::InvalidName("nope".to_owned());
        assert!(err.to_string().contains("invalid file name"));
        let err: FsError = std::io::Error::other("boom").into();
        assert!(err.to_string().contains("io error"));
    }
}
