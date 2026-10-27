//! Workspace, file-buffer, search, and git service for the Tauri shell.
//!
//! The frontend never touches the filesystem directly. This module owns the
//! canonical workspace root, the open-buffer table, and the optional git
//! handle, and it converts every result into the versioned DTOs from
//! selora-protocol.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};

use selora_protocol::{
    BufferChangedDto, BufferConflictClearedDto, BufferConflictDto, BufferId, BufferSnapshotDto,
    FileTreeDto, FileTreeNodeDto, FileTreeNodeKind, GitDiffDto, GitFileDto, GitStatusDto,
    SearchHitDto, SearchQueryDto, SearchResultsDto, Selection, TextChange, TextPosition,
    WorkspacePath,
};

/// Maximum size of a file that may be opened in the text editor.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Maximum number of search hits returned to the frontend.
const MAX_SEARCH_HITS: usize = 500;

/// A stable, machine-readable workspace service error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceError {
    /// Stable error code consumed by the frontend.
    pub code: &'static str,
    /// Human-readable detail (never required for control flow).
    pub message: String,
}

impl WorkspaceError {
    /// Construct an error with a stable code and a human-readable message.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WorkspaceError {}

/// Result type returned by [Workspace] operations.
pub type WsResult<T> = Result<T, WorkspaceError>;

/// One open buffer a disk refresh could not reload.
///
/// The tab stays open: the caller reports the reason, never drops the buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedBufferRefresh {
    /// The workspace-relative path that could not be reloaded.
    pub path: WorkspacePath,
    /// Stable reason code (`file_missing`, `binary_file`, `file_too_large`,
    /// `io_error`).
    pub code: &'static str,
    /// Human-readable detail for the toast.
    pub message: String,
}

/// The result of [Workspace::refresh_clean_buffers].
///
/// `updated` holds the bumped snapshots, `dirty` the buffers left untouched
/// because they hold unsaved edits, `conflicts` the dirty buffers whose disk
/// file also changed (the caller must offer a Reload/Keep choice), and
/// `skipped` the buffers whose disk file disappeared or can no longer be read
/// as editor text.
#[derive(Clone, Debug, Default)]
pub struct BufferRefreshOutcome {
    /// Clean buffers whose text changed on disk, each with one version bump.
    pub updated: Vec<BufferSnapshotDto>,
    /// Dirty buffers (local edits preserved) — reported so the user can save.
    pub dirty: Vec<WorkspacePath>,
    /// Dirty buffers that changed on disk; their local text is preserved.
    pub conflicts: Vec<BufferConflictDto>,
    /// Conflicts cleared because the disk change reverted or was reloaded.
    pub cleared: Vec<BufferConflictClearedDto>,
    /// Buffers the refresh could not reload; their tabs stay intact.
    pub skipped: Vec<SkippedBufferRefresh>,
}

/// One open text buffer and its saved baseline.
struct BufferState {
    id: BufferId,
    path: WorkspacePath,
    abs_path: PathBuf,
    language: String,
    text: String,
    saved_text: String,
    version: u64,
    selection: Option<Selection>,
}

impl BufferState {
    fn snapshot(&self) -> BufferSnapshotDto {
        BufferSnapshotDto {
            buffer_id: self.id.clone(),
            path: self.path.clone(),
            language: self.language.clone(),
            text: self.text.clone(),
            version: self.version,
            dirty: self.text != self.saved_text,
            selection: self.selection.clone(),
        }
    }
}

/// One clean buffer captured for a disk refresh, before the read.
///
/// The capture carries the version and text the refresh saw, so the apply
/// phase can detect an edit that landed while the file was being read.
struct RefreshCandidate {
    key: String,
    path: WorkspacePath,
    abs_path: PathBuf,
    version: u64,
    /// The local buffer text at capture time.
    text: String,
}

/// The workspace service behind the Tauri command layer.
pub struct Workspace {
    root: PathBuf,
    buffers: Mutex<BTreeMap<String, BufferState>>,
/// P2 gap 3: in-memory external-change markers, keyed by the buffer path.
    /// Nothing here is persisted; a restart simply has no conflict.
    conflicts: Mutex<BTreeMap<String, BufferConflictDto>>,
    /// The buffer the user most recently opened or selected. Bootstrap
    /// reports this instead of the first path-sorted buffer, so a tab switch
    /// or a close never jumps to an unrelated tab. Lock order is always
    /// `buffers` then `active`; no path takes them in the other order.
    active: Mutex<Option<BufferId>>,
    git: Mutex<Option<selora_git::GitRepo>>,
}

impl Workspace {
    /// Open a canonical workspace root.
    ///
    /// The path must exist and be a directory. A git repository is opened
    /// when one is discoverable; a plain folder is a valid workspace.
    pub fn open(root: PathBuf) -> WsResult<Self> {
        let metadata = fs::metadata(&root).map_err(|err| {
            let code = if err.kind() == io::ErrorKind::NotFound {
                "not_a_directory"
            } else {
                "io_error"
            };
            WorkspaceError::new(
                code,
                format!("cannot open workspace root '{}': {err}", root.display()),
            )
        })?;
        if !metadata.is_dir() {
            return Err(WorkspaceError::new(
                "not_a_directory",
                format!("workspace root '{}' is not a directory", root.display()),
            ));
        }
        let root = fs::canonicalize(&root).map_err(|err| {
            WorkspaceError::new(
                "io_error",
                format!(
                    "cannot canonicalize workspace root '{}': {err}",
                    root.display()
                ),
            )
        })?;
        let git = selora_git::GitRepo::open(&root).ok();
        Ok(Self {
            root,
            buffers: Mutex::new(BTreeMap::new()),
conflicts: Mutex::new(BTreeMap::new()),
            active: Mutex::new(None),
            git: Mutex::new(git),
        })
    }

    /// The canonical workspace root.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The workspace folder name, or the full path when it has no file name.
    pub fn display_name(&self) -> String {
        self.root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.root.to_string_lossy().into_owned())
    }

    /// Build the workspace tree, with relative slash-separated paths.
    pub fn tree(&self) -> WsResult<FileTreeDto> {
        let tree = selora_fs::tree::build_tree(&self.root);
        Ok(FileTreeDto {
            root: Some(self.tree_node_to_dto(&tree, true)?),
        })
    }

    /// Open a workspace-relative file as a text buffer.
    ///
    /// Re-opening an already open file returns its current buffer without
    /// touching the disk, so unsaved edits are preserved.
    pub fn open_file(&self, rel: &str) -> WsResult<BufferSnapshotDto> {
        let rel = Self::normalize_relative(rel)?;
        if let Some(snapshot) = {
            let buffers = self.buffers_lock();
            buffers.get(&rel).map(BufferState::snapshot)
        } {
            // Re-selecting an already-open file made it active in the old
            // single-buffer day; keep that behavior for bootstrap.
            self.set_active_buffer(&snapshot.buffer_id)?;
            return Ok(snapshot);
        }

        let abs_path = self.resolve_relative(&rel, false)?;
        let text = Self::read_editor_text(&abs_path, &rel)?;

        let id = Self::buffer_id_for(&rel)?;
        let path = WorkspacePath::new(&rel).map_err(invalid_path_error)?;
        let state = BufferState {
            id: id.clone(),
            path,
            abs_path,
            language: language_for(Path::new(&rel)).to_owned(),
            text: text.clone(),
            saved_text: text,
            version: 1,
            selection: None,
        };
        let snapshot = state.snapshot();
        let mut buffers = self.buffers_lock();
        if let Some(existing) = buffers.get(&rel) {
            let snapshot = existing.snapshot();
            drop(buffers);
            self.set_active_buffer(&snapshot.buffer_id)?;
            return Ok(snapshot);
        }
        buffers.insert(rel, state);
        drop(buffers);
        *self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(id);
        Ok(snapshot)
    }

    /// Every open buffer, sorted by workspace-relative path.
    pub fn buffers(&self) -> Vec<BufferSnapshotDto> {
        self.buffers_lock()
            .values()
            .map(BufferState::snapshot)
            .collect()
    }

    /// The buffer bootstrap should report as active. `None` only when no
    /// buffer is open (or the recorded id somehow is not in the table).
    pub fn active_buffer_id(&self) -> Option<selora_protocol::BufferId> {
        let buffers = self.buffers_lock();
        let active = self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        active
            .as_ref()
            .filter(|id| buffers.values().any(|state| &state.id == *id))
            .cloned()
            .or_else(|| buffers.values().next().map(|state| state.id.clone()))
    }

    /// Record the user's tab selection. Unknown ids are rejected, so a stale
    /// frontend cannot make bootstrap point at a closed buffer.
    pub fn set_active_buffer(&self, id: &selora_protocol::BufferId) -> WsResult<()> {
        {
            let buffers = self.buffers_lock();
            if !buffers.values().any(|state| &state.id == id) {
                return Err(unknown_buffer_error(id));
            }
        }
        *self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(id.clone());
        Ok(())
    }

    /// Return one open buffer by id.
    #[allow(dead_code)]
    pub fn buffer(&self, id: &selora_protocol::BufferId) -> WsResult<BufferSnapshotDto> {
        self.buffers_lock()
            .values()
            .find(|state| &state.id == id)
            .map(BufferState::snapshot)
            .ok_or_else(|| unknown_buffer_error(id))
    }

    /// Close an open buffer without saving it.
    ///
    /// When the closed buffer was active, selection moves to its nearest
    /// remaining neighbor (the previous path, else the next), never to the
    /// alphabetically first tab.
    pub fn close_buffer(&self, id: &selora_protocol::BufferId) -> WsResult<()> {
        let mut buffers = self.buffers_lock();
        let key = buffers
            .iter()
            .find(|(_, state)| &state.id == id)
            .map(|(key, _)| key.clone())
            .ok_or_else(|| unknown_buffer_error(id))?;

        let was_active = {
            let active = self
                .active
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            active.as_ref() == Some(id)
        };
        buffers.remove(&key);

        if was_active {
            let next = buffers
                .range(..key.clone())
                .next_back()
                .or_else(|| buffers.range(key..).next())
                .map(|(_, state)| state.id.clone());
            drop(buffers);
            *self
                .active
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = next;
        }
        Ok(())
    }

    /// Save a buffer atomically and clear its dirty flag.
    pub fn save_buffer(
        &self,
        id: &selora_protocol::BufferId,
    ) -> WsResult<selora_protocol::BufferSnapshotDto> {
        let mut buffers = self.buffers_lock();
        let state = buffers
            .values_mut()
            .find(|state| &state.id == id)
            .ok_or_else(|| unknown_buffer_error(id))?;
        atomic_write(&state.abs_path, state.text.as_bytes()).map_err(|err| {
            WorkspaceError::new(
                "io_error",
                format!("cannot save '{}': {err}", state.path.as_str()),
            )
        })?;
        state.saved_text.clone_from(&state.text);
        Ok(state.snapshot())
    }

    /// Remove and return one buffer's external-change marker.
    ///
    /// Saving the local text makes it the disk text, so the conflict is
    /// resolved: the command layer emits `BufferConflictCleared` when this
    /// returns `Some`.
    pub fn take_conflict(
        &self,
        id: &selora_protocol::BufferId,
    ) -> Option<BufferConflictClearedDto> {
        let buffers = self.buffers_lock();
        let (key, path) = buffers
            .iter()
            .find(|(_, state)| &state.id == id)
            .map(|(key, state)| (key.clone(), state.path.clone()))?;
        drop(buffers);
        self.conflicts_lock()
            .remove(&key)
            .map(|conflict| BufferConflictClearedDto {
                buffer_id: conflict.buffer_id,
                path,
            })
    }

    /// Apply one versioned batch of UTF-16 text edits.
    pub fn apply_change(
        &self,
        id: &selora_protocol::BufferId,
        version: u64,
        changes: &[selora_protocol::TextChange],
    ) -> WsResult<selora_protocol::BufferChangedDto> {
        let mut buffers = self.buffers_lock();
        let state = buffers
            .values_mut()
            .find(|state| &state.id == id)
            .ok_or_else(|| unknown_buffer_error(id))?;
        if state.version != version {
            return Err(WorkspaceError::new(
                "stale_buffer_version",
                format!(
                    "buffer version {version} does not match current version {}",
                    state.version
                ),
            ));
        }

        let mut text = state.text.clone();
        apply_changes_to_text(&mut text, changes)?;
        let next_version = state
            .version
            .checked_add(1)
            .ok_or_else(|| WorkspaceError::new("io_error", "buffer version counter overflowed"))?;
        state.text = text;
        state.version = next_version;
        Ok(BufferChangedDto {
            buffer_id: state.id.clone(),
            version: state.version,
            changes: changes.to_vec(),
        })
    }

    /// Store the last known selection for a buffer.
    pub fn set_selection(
        &self,
        id: &selora_protocol::BufferId,
        selection: selora_protocol::Selection,
    ) -> WsResult<()> {
        let mut buffers = self.buffers_lock();
        let state = buffers
            .values_mut()
            .find(|state| &state.id == id)
            .ok_or_else(|| unknown_buffer_error(id))?;
        state.selection = Some(selection);
        Ok(())
    }

    /// Reload every **clean** open buffer from disk, leaving dirty buffers
    /// untouched.
    ///
    /// This is the post-run refresh: a workflow (or any external writer) may
    /// have rewritten files while their tabs were open. A clean buffer whose
    /// disk text differs is replaced and its version bumps exactly once; a
    /// buffer with unsaved local edits keeps that text. When such a dirty
    /// buffer's disk text also changed since its saved baseline, a
    /// [`BufferConflictDto`] is reported so the UI can offer Reload/Keep.
    /// A file that disappeared or can no longer be read as editor text is
    /// reported in [`BufferRefreshOutcome::skipped`] and its tab stays open,
    /// so the user can still save, close, or inspect it.
    ///
    /// Disk reads happen outside the buffer lock; the second phase re-checks
    /// that each candidate is still at the version and text it was captured
    /// at, so an edit racing the refresh is never overwritten.
    pub fn refresh_clean_buffers(&self) -> BufferRefreshOutcome {
        let candidates = {
            let buffers = self.buffers_lock();
            buffers
                .iter()
                .map(|(key, state)| RefreshCandidate {
                    key: key.clone(),
                    path: state.path.clone(),
                    abs_path: state.abs_path.clone(),
                    version: state.version,
                    text: state.text.clone(),
                })
                .collect::<Vec<_>>()
        };

        let mut outcome = BufferRefreshOutcome::default();
        let mut reloaded = Vec::new();
        for candidate in candidates {
            let rel = candidate.path.as_str().to_owned();
            match fs::metadata(&candidate.abs_path) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    outcome.skipped.push(SkippedBufferRefresh {
                        path: candidate.path,
                        code: "file_missing",
                        message: format!("'{rel}' no longer exists on disk"),
                    });
                    continue;
                }
                Err(err) => {
                    outcome.skipped.push(SkippedBufferRefresh {
                        path: candidate.path,
                        code: "io_error",
                        message: format!("cannot stat '{rel}': {err}"),
                    });
                    continue;
                }
                Ok(metadata) if !metadata.is_file() => {
                    outcome.skipped.push(SkippedBufferRefresh {
                        path: candidate.path,
                        code: "io_error",
                        message: format!("'{rel}' is no longer a regular file"),
                    });
                    continue;
                }
                Ok(_) => {}
            }
            match Self::read_editor_text(&candidate.abs_path, &rel) {
                Ok(text) => reloaded.push((candidate, text)),
                Err(error) => outcome.skipped.push(SkippedBufferRefresh {
                    path: candidate.path,
                    code: error.code,
                    message: error.message,
                }),
            }
        }

        let mut buffers = self.buffers_lock();
        let mut conflicts = self.conflicts_lock();
        for (candidate, text) in reloaded {
            let Some(state) = buffers.get_mut(&candidate.key) else {
                // The tab was closed while the disk read ran; nothing to update.
                continue;
            };
            if state.version != candidate.version || state.text != candidate.text {
                // Edited (or already refreshed) while the disk read ran: leave
                // the current text and version alone. The next refresh sees it.
                continue;
            }
            let dirty = state.text != state.saved_text;
            if dirty {
                outcome.dirty.push(state.path.clone());
                if text == state.saved_text {
                    // The disk still matches the saved baseline: the local
                    // edits are the only difference. Any old marker is stale.
                    if conflicts.remove(&candidate.key).is_some() {
                        outcome.cleared.push(BufferConflictClearedDto {
                            buffer_id: state.id.clone(),
                            path: state.path.clone(),
                        });
                    }
                    continue;
                }
                let conflict = BufferConflictDto {
                    buffer_id: state.id.clone(),
                    path: state.path.clone(),
                };
                conflicts.insert(candidate.key.clone(), conflict.clone());
                outcome.conflicts.push(conflict);
                continue;
            }
            let cleared = conflicts
                .remove(&candidate.key)
                .map(|_| BufferConflictClearedDto {
                    buffer_id: state.id.clone(),
                    path: state.path.clone(),
                });
            if let Some(cleared) = cleared {
                outcome.cleared.push(cleared);
            }
            if state.text == text {
                continue;
            }
            let Some(next_version) = state.version.checked_add(1) else {
                outcome.skipped.push(SkippedBufferRefresh {
                    path: state.path.clone(),
                    code: "io_error",
                    message: format!(
                        "buffer version counter overflowed for '{}'",
                        state.path.as_str()
                    ),
                });
                continue;
            };
            state.text = text.clone();
            state.saved_text = text;
            state.version = next_version;
            outcome.updated.push(state.snapshot());
        }
        outcome
    }

    /// Reload one conflicting buffer from disk, discarding the local edits.
    ///
    /// The caller only offers this while a conflict is stored; asking for a
    /// buffer with no conflict is a typed error rather than a silent reload,
    /// so a stale banner cannot throw away edits.
    pub fn reload_buffer_from_disk(
        &self,
        id: &selora_protocol::BufferId,
    ) -> WsResult<BufferSnapshotDto> {
        let (key, path, abs_path) = {
            let buffers = self.buffers_lock();
            let (key, state) = buffers
                .iter()
                .find(|(_, state)| &state.id == id)
                .ok_or_else(|| unknown_buffer_error(id))?;
            (key.clone(), state.path.clone(), state.abs_path.clone())
        };
        {
            let conflicts = self.conflicts_lock();
            if !conflicts.contains_key(&key) {
                return Err(WorkspaceError::new(
                    "no_buffer_conflict",
                    format!("'{}' has no external change to reload", path.as_str()),
                ));
            }
        }

        let text = Self::read_editor_text(&abs_path, path.as_str())?;
        let mut buffers = self.buffers_lock();
        let mut conflicts = self.conflicts_lock();
        // Keep wins if the two decisions race: never discard text after the
        // user cleared the marker while the disk read was in flight.
        if !conflicts.contains_key(&key) {
            return Err(WorkspaceError::new(
                "no_buffer_conflict",
                format!("'{}' has no external change to reload", path.as_str()),
            ));
        }
        let state = buffers
            .get_mut(&key)
            .ok_or_else(|| unknown_buffer_error(id))?;
        // Keep the conflict until the disk read succeeds; a failed read leaves
        // the banner up so the user can retry or keep the local edits.
        let next_version = state
            .version
            .checked_add(1)
            .ok_or_else(|| WorkspaceError::new("io_error", "buffer version counter overflowed"))?;
        state.text = text.clone();
        state.saved_text = text;
        state.version = next_version;
        let snapshot = state.snapshot();
        conflicts.remove(&key);
        Ok(snapshot)
    }

    /// Keep the local edits for a conflicting buffer and clear the marker.
    pub fn keep_buffer_edits(&self, id: &selora_protocol::BufferId) -> WsResult<BufferSnapshotDto> {
        let buffers = self.buffers_lock();
        let (key, path) = buffers
            .iter()
            .find(|(_, state)| &state.id == id)
            .map(|(key, state)| (key.clone(), state.path.clone()))
            .ok_or_else(|| unknown_buffer_error(id))?;
        let mut conflicts = self.conflicts_lock();
        if conflicts.remove(&key).is_none() {
            return Err(WorkspaceError::new(
                "no_buffer_conflict",
                format!("'{}' has no external change to keep", path.as_str()),
            ));
        }
        buffers
            .get(&key)
            .map(BufferState::snapshot)
            .ok_or_else(|| unknown_buffer_error(id))
    }

    /// This buffer's external-change marker, if any (test/diagnostic seam).
    #[cfg(test)]
    pub fn conflict_for(&self, id: &selora_protocol::BufferId) -> Option<BufferConflictDto> {
        self.conflicts_lock()
            .values()
            .find(|conflict| &conflict.buffer_id == id)
            .cloned()
    }

    /// Search workspace files, literal or regular-expression per the query.
    pub fn search(&self, query: &SearchQueryDto) -> WsResult<selora_protocol::SearchResultsDto> {
        let fs_query = selora_fs::search::SearchQuery {
            pattern: query.query.clone(),
            case_sensitive: query.case_sensitive,
            whole_word: query.whole_word,
        };
        let hits = if query.use_regex {
            selora_fs::search::search_in_files_regex(&self.root, &fs_query)
        } else {
            selora_fs::search::search_in_files(&self.root, &fs_query)
        }
        .map_err(search_error)?;
        let mut mapped = Vec::new();
        for hit in hits.into_iter().take(MAX_SEARCH_HITS) {
            let Some(rel) = self.workspace_relative(&hit.path) else {
                continue;
            };
            let Ok(path) = WorkspacePath::new(&rel) else {
                continue;
            };
            let line = u32::try_from(hit.line_number).unwrap_or(u32::MAX);
            let column = hit
                .spans
                .first()
                .map(|span| u32::try_from(span.start.saturating_add(1)).unwrap_or(u32::MAX))
                .unwrap_or(1);
            mapped.push(SearchHitDto {
                path,
                line,
                column,
                preview: hit.line_text,
            });
        }
        Ok(SearchResultsDto {
            request_id: None,
            query: query.clone(),
            hits: mapped,
        })
    }

    /// Snapshot git status; a non-repository workspace is empty, not an error.
    pub fn git_status(&self) -> WsResult<selora_protocol::GitStatusDto> {
        let git = self.git_lock();
        match git.as_ref() {
            Some(repo) => {
                let status = repo.status().map_err(git_error)?;
                Ok(self.git_status_to_dto(status))
            }
            None => Ok(GitStatusDto {
                branch: None,
                changed_paths: Vec::new(),
                staged_paths: Vec::new(),
                files: Vec::new(),
            }),
        }
    }

    /// Stage one workspace-relative path and return fresh status.
    pub fn git_stage(&self, rel: &str) -> WsResult<selora_protocol::GitStatusDto> {
        let rel = Self::normalize_relative(rel)?;
        let abs_path = self.resolve_relative(&rel, true)?;
        let git = self.git_lock();
        let repo = git.as_ref().ok_or_else(not_a_git_repo)?;
        repo.stage(&abs_path).map_err(git_error)?;
        let status = repo.status().map_err(git_error)?;
        Ok(self.git_status_to_dto(status))
    }

    /// Unstage one workspace-relative path and return fresh status.
    pub fn git_unstage(&self, rel: &str) -> WsResult<selora_protocol::GitStatusDto> {
        let rel = Self::normalize_relative(rel)?;
        let abs_path = self.resolve_relative(&rel, true)?;
        let git = self.git_lock();
        let repo = git.as_ref().ok_or_else(not_a_git_repo)?;
        repo.unstage(&abs_path).map_err(git_error)?;
        let status = repo.status().map_err(git_error)?;
        Ok(self.git_status_to_dto(status))
    }

    /// Commit the index and return fresh status.
    pub fn git_commit(&self, message: &str) -> WsResult<selora_protocol::GitStatusDto> {
        let git = self.git_lock();
        let repo = git.as_ref().ok_or_else(not_a_git_repo)?;
        repo.commit(message).map_err(git_error)?;
        let status = repo.status().map_err(git_error)?;
        Ok(self.git_status_to_dto(status))
    }

    /// Return the unified diff for one workspace-relative path.
    ///
    /// A staged row shows the HEAD→index side; an unstaged row shows the
    /// index→worktree side. A file with both (git `MM`) shows both, staged
    /// first, so the panel never presents an empty diff for a staged row.
    pub fn git_diff(&self, rel: &str) -> WsResult<selora_protocol::GitDiffDto> {
        let rel = Self::normalize_relative(rel)?;
        let abs_path = self.resolve_relative(&rel, true)?;
        let git = self.git_lock();
        let repo = git.as_ref().ok_or_else(not_a_git_repo)?;
        let status = repo.status().map_err(git_error)?;
        let entry = status
            .entries
            .iter()
            .find(|entry| self.workspace_relative(&entry.abs_path).as_deref() == Some(&rel));

        let mut parts = Vec::new();
        if let Some(entry) = entry {
            if entry.is_staged() {
                let diff = repo.diff_index(&abs_path).map_err(git_error)?;
                parts.push(diff.to_unified());
            }
            if entry.has_worktree_changes() {
                let diff = repo.diff_workdir(&abs_path).map_err(git_error)?;
                parts.push(diff.to_unified());
            }
        }
        if parts.iter().all(|part| part.is_empty()) {
            // Not in status (a stale row or a path outside the repo): keep
            // the old worktree behavior rather than returning nothing.
            let diff = repo.diff_workdir(&abs_path).map_err(git_error)?;
            parts.push(diff.to_unified());
        }
        let unified_diff = parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        let path = WorkspacePath::new(&rel).map_err(invalid_path_error)?;
        Ok(GitDiffDto { path, unified_diff })
    }

    fn buffers_lock(&self) -> MutexGuard<'_, BTreeMap<String, BufferState>> {
        self.buffers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn conflicts_lock(&self) -> MutexGuard<'_, BTreeMap<String, BufferConflictDto>> {
        self.conflicts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn git_lock(&self) -> MutexGuard<'_, Option<selora_git::GitRepo>> {
        self.git
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn normalize_relative(rel: &str) -> WsResult<String> {
        if rel.is_empty()
            || rel.chars().any(char::is_control)
            || rel.starts_with(['/', '\\'])
            || rel.as_bytes().get(1).is_some_and(|byte| *byte == b':')
        {
            return Err(invalid_rel(rel));
        }
        let normalized = rel.replace('\\', "/");
        if Path::new(&normalized).is_absolute()
            || normalized.split('/').any(|segment| {
                segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
            })
        {
            return Err(invalid_rel(rel));
        }
        Ok(normalized)
    }

    fn resolve_relative(&self, normalized: &str, allow_missing: bool) -> WsResult<PathBuf> {
        let candidate = self.root.join(normalized);
        if allow_missing {
            match fs::canonicalize(&candidate) {
                Ok(path) => return self.ensure_inside(path),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    let parent = candidate.parent().ok_or_else(|| invalid_rel(normalized))?;
                    let name = candidate
                        .file_name()
                        .ok_or_else(|| invalid_rel(normalized))?;
                    let parent = fs::canonicalize(parent).map_err(|err| {
                        WorkspaceError::new(
                            "io_error",
                            format!("cannot resolve '{normalized}': {err}"),
                        )
                    })?;
                    let parent = self.ensure_inside(parent)?;
                    let joined = parent.join(name);
                    if joined.starts_with(&self.root) {
                        return Ok(joined);
                    }
                    return Err(path_escape_error(normalized));
                }
                Err(err) => {
                    return Err(WorkspaceError::new(
                        "io_error",
                        format!("cannot resolve '{normalized}': {err}"),
                    ));
                }
            }
        }
        let path = fs::canonicalize(&candidate).map_err(|err| {
            WorkspaceError::new("io_error", format!("cannot resolve '{normalized}': {err}"))
        })?;
        self.ensure_inside(path)
    }

    fn ensure_inside(&self, path: PathBuf) -> WsResult<PathBuf> {
        if path.starts_with(&self.root) {
            Ok(path)
        } else {
            Err(path_escape_error(&path.to_string_lossy()))
        }
    }

    /// Read a file as editor text, applying the same size and text checks as
    /// [`Workspace::open_file`]. Shared so a reload can never accept a file the
    /// editor would have refused.
    fn read_editor_text(abs_path: &Path, rel: &str) -> WsResult<String> {
        let metadata = fs::metadata(abs_path).map_err(|err| {
            WorkspaceError::new("io_error", format!("cannot stat '{rel}': {err}"))
        })?;
        if !metadata.is_file() {
            return Err(WorkspaceError::new(
                "io_error",
                format!("'{rel}' is not a regular file"),
            ));
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(WorkspaceError::new(
                "file_too_large",
                format!("'{rel}' exceeds the {MAX_FILE_BYTES} byte editor limit"),
            ));
        }
        let bytes = fs::read(abs_path).map_err(|err| {
            WorkspaceError::new("io_error", format!("cannot read '{rel}': {err}"))
        })?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(WorkspaceError::new(
                "file_too_large",
                format!("'{rel}' exceeds the {MAX_FILE_BYTES} byte editor limit"),
            ));
        }
        if bytes.contains(&0) {
            return Err(WorkspaceError::new(
                "binary_file",
                format!("'{rel}' contains NUL bytes and cannot be edited as text"),
            ));
        }
        String::from_utf8(bytes).map_err(|_| {
            WorkspaceError::new("binary_file", format!("'{rel}' is not valid UTF-8 text"))
        })
    }

    fn buffer_id_for(rel: &str) -> WsResult<BufferId> {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in rel.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        BufferId::new(format!("buf_{hash:016x}")).map_err(|err| {
            WorkspaceError::new(
                "io_error",
                format!("generated invalid buffer id for '{rel}': {err}"),
            )
        })
    }

    fn plain_root(&self) -> PathBuf {
        #[cfg(windows)]
        {
            let text = self.root.as_os_str().to_string_lossy();
            if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{rest}"));
            }
            if let Some(rest) = text.strip_prefix(r"\\?\") {
                return PathBuf::from(rest);
            }
        }
        self.root.clone()
    }

    fn workspace_relative(&self, path: &Path) -> Option<String> {
        let plain_root = self.plain_root();
        let relative = path
            .strip_prefix(&self.root)
            .or_else(|_| path.strip_prefix(&plain_root))
            .ok()?;
        Some(path_to_forward_slashes(relative))
    }

    fn tree_node_to_dto(
        &self,
        node: &selora_fs::tree::FileTreeNode,
        is_root: bool,
    ) -> WsResult<FileTreeNodeDto> {
        let rel = if is_root {
            let name = self.display_name();
            if WorkspacePath::new(&name).is_ok() {
                name
            } else {
                "workspace".to_owned()
            }
        } else {
            self.workspace_relative(&node.path).ok_or_else(|| {
                WorkspaceError::new(
                    "io_error",
                    format!("tree node '{}' escaped the workspace", node.path.display()),
                )
            })?
        };
        let path = WorkspacePath::new(&rel).map_err(invalid_path_error)?;
        let children = node
            .children
            .iter()
            .map(|child| self.tree_node_to_dto(child, false))
            .collect::<WsResult<Vec<_>>>()?;
        Ok(FileTreeNodeDto {
            path,
            name: node.name.clone(),
            kind: if node.is_dir {
                FileTreeNodeKind::Directory
            } else {
                FileTreeNodeKind::File
            },
            children,
        })
    }

    fn git_status_to_dto(&self, status: selora_git::GitStatus) -> GitStatusDto {
        let mut changed_paths = Vec::new();
        let mut staged_paths = Vec::new();
        let mut files = Vec::new();
        for entry in status.entries {
            let Some(rel) = self.workspace_relative(&entry.abs_path) else {
                continue;
            };
            let Ok(path) = WorkspacePath::new(rel) else {
                continue;
            };
            // P2: the two slots are independent — a file can be staged *and*
            // carry further working-tree edits. Record both facts per file so
            // the panel can render (and act on) each side separately.
            let staged = entry.staged != selora_git::Slot::Unmodified;
            let worktree = entry.has_worktree_changes();
            if staged {
                staged_paths.push(path.clone());
            }
            changed_paths.push(path.clone());
            files.push(GitFileDto {
                path,
                staged,
                worktree,
            });
        }
        GitStatusDto {
            branch: status.branch,
            changed_paths,
            staged_paths,
            files,
        }
    }
}

/// Apply one batch of UTF-16 text edits.
///
/// The wire contract resolves every change against the document *before* the
/// batch, so all ranges are converted to byte offsets against a snapshot of
/// the original text first. Changes must be sorted and non-overlapping; the
/// result is then produced in a single pass. Applying each change against the
/// already-mutated text (the previous behaviour) shifted later ranges and
/// corrupted documents with more than one selection.
fn apply_changes_to_text(text: &mut String, changes: &[TextChange]) -> WsResult<()> {
    let mut resolved: Vec<(usize, usize, &str)> = Vec::with_capacity(changes.len());
    for change in changes {
        let start = byte_offset_for(text, change.start)?;
        let end = byte_offset_for(text, change.end)?;
        if start > end {
            return Err(WorkspaceError::new(
                "invalid_change",
                "text change start is after its end",
            ));
        }
        resolved.push((start, end, change.replacement.as_str()));
    }

    for pair in resolved.windows(2) {
        if pair[1].0 < pair[0].1 {
            return Err(WorkspaceError::new(
                "invalid_change",
                "text changes overlap or are out of order",
            ));
        }
    }

    if resolved.is_empty() {
        return Ok(());
    }

    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, replacement) in resolved {
        output.push_str(&text[cursor..start]);
        output.push_str(replacement);
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    *text = output;
    Ok(())
}

fn byte_offset_for(text: &str, position: TextPosition) -> WsResult<usize> {
    let mut line_start = 0;
    for _ in 0..position.line {
        let Some(newline) = text[line_start..].find('\n') else {
            return Err(out_of_bounds_change(position));
        };
        line_start += newline + 1;
    }
    let line_end = text[line_start..]
        .find('\n')
        .map(|offset| line_start + offset)
        .unwrap_or(text.len());
    let content_end = if line_end > line_start && text.as_bytes()[line_end - 1] == b'\r' {
        line_end - 1
    } else {
        line_end
    };
    if position.utf16_column == 0 {
        return Ok(line_start);
    }

    let line = &text[line_start..content_end];
    let mut units = 0_u32;
    for (byte_offset, character) in line.char_indices() {
        let next = units + character.len_utf16() as u32;
        if next == position.utf16_column {
            return Ok(line_start + byte_offset + character.len_utf8());
        }
        if position.utf16_column < next {
            return Err(out_of_bounds_change(position));
        }
        units = next;
    }
    if units == position.utf16_column {
        Ok(content_end)
    } else {
        Err(out_of_bounds_change(position))
    }
}

fn language_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("rs") => "rust",
        Some("ts" | "tsx") => "typescript",
        Some("js" | "jsx") => "javascript",
        Some("css") => "css",
        Some("json") => "json",
        Some("md") => "markdown",
        Some("toml") => "toml",
        _ => "text",
    }
}

fn path_to_forward_slashes(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no parent"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no name"))?;

    for _ in 0..100 {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{}.selora-{}-{sequence}.tmp",
            file_name.to_string_lossy(),
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(mut file) => {
                if let Err(err) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                    drop(file);
                    let _ = fs::remove_file(&temporary);
                    return Err(err);
                }
                drop(file);
                if let Err(err) = fs::rename(&temporary, path) {
                    let _ = fs::remove_file(&temporary);
                    return Err(err);
                }
                return Ok(());
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary save file",
    ))
}

fn invalid_rel(rel: &str) -> WorkspaceError {
    WorkspaceError::new(
        "invalid_path",
        format!("'{rel}' is not a safe workspace-relative path"),
    )
}

fn invalid_path_error(err: selora_protocol::ProtocolValidationError) -> WorkspaceError {
    WorkspaceError::new("invalid_path", err.to_string())
}

/// Map a search-engine failure: a bad regex is the user's typo
/// (`invalid_query`), everything else is I/O.
fn search_error(err: selora_fs::search::SearchError) -> WorkspaceError {
    let code = match &err {
        selora_fs::search::SearchError::InvalidPattern(_) => "invalid_query",
        _ => "io_error",
    };
    WorkspaceError::new(code, err.to_string())
}

fn path_escape_error(path: &str) -> WorkspaceError {
    WorkspaceError::new(
        "path_escape",
        format!("'{path}' resolves outside the workspace root"),
    )
}

fn unknown_buffer_error(id: &BufferId) -> WorkspaceError {
    WorkspaceError::new(
        "unknown_buffer",
        format!("buffer '{}' is not open", id.as_str()),
    )
}

fn out_of_bounds_change(position: TextPosition) -> WorkspaceError {
    WorkspaceError::new(
        "invalid_change",
        format!(
            "text position {}:{} is outside the buffer",
            position.line, position.utf16_column
        ),
    )
}

fn not_a_git_repo() -> WorkspaceError {
    WorkspaceError::new(
        "not_a_git_repo",
        "the current workspace is not inside a git repository",
    )
}

fn git_error(err: selora_git::GitError) -> WorkspaceError {
    WorkspaceError::new("git_error", err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::{tempdir, TempDir};

    struct Fixture {
        _directory: TempDir,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempdir().expect("tempdir");
            let root = directory.path().join("project");
            fs::create_dir_all(root.join("src/nested")).expect("nested dirs");
            fs::write(
                root.join("src/main.rs"),
                "fn main() {\n    let needle = 1;\n}\n",
            )
            .expect("main.rs");
            fs::write(root.join("src/nested/note.txt"), "nested\n").expect("note");
            fs::write(root.join("README.md"), "# Project\n").expect("readme");
            Self {
                _directory: directory,
                root,
            }
        }

        fn workspace(&self) -> Workspace {
            Workspace::open(self.root.clone()).expect("workspace")
        }
    }

    fn position(line: u32, utf16_column: u32) -> TextPosition {
        TextPosition { line, utf16_column }
    }

    fn change(start: TextPosition, end: TextPosition, replacement: &str) -> TextChange {
        TextChange {
            start,
            end,
            replacement: replacement.to_owned(),
        }
    }

    #[test]
    fn open_rejects_files_and_missing_directories() {
        let fixture = Fixture::new();
        let file_error = Workspace::open(fixture.root.join("README.md"))
            .err()
            .expect("file error");
        assert_eq!(file_error.code, "not_a_directory");
        let missing_error = Workspace::open(fixture.root.join("missing"))
            .err()
            .expect("missing error");
        assert_eq!(missing_error.code, "not_a_directory");
    }

    #[test]
    fn tree_is_relative_and_directories_come_first() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let tree = workspace.tree().expect("tree");
        let root = tree.root.expect("root node");
        assert_eq!(root.path.as_str(), "project");
        assert_eq!(root.kind, FileTreeNodeKind::Directory);
        assert_eq!(root.children[0].path.as_str(), "src");
        assert_eq!(root.children[0].kind, FileTreeNodeKind::Directory);
        let src = &root.children[0];
        assert_eq!(src.children[0].path.as_str(), "src/nested");
        assert_eq!(src.children[1].path.as_str(), "src/main.rs");
        assert_eq!(src.children[1].kind, FileTreeNodeKind::File);
        assert!(src.children[1].children.is_empty());
        assert!(root
            .children
            .iter()
            .any(|child| child.path.as_str() == "README.md"));
    }

    #[test]
    fn open_file_sets_language_and_stable_buffer_id() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let snapshot = workspace.open_file("src\\main.rs").expect("open");
        assert_eq!(snapshot.path.as_str(), "src/main.rs");
        assert_eq!(snapshot.language, "rust");
        assert_eq!(snapshot.version, 1);
        assert!(!snapshot.dirty);
        assert!(snapshot.selection.is_none());
        assert!(snapshot.buffer_id.as_str().starts_with("buf_"));
        assert_eq!(snapshot.buffer_id.as_str().len(), 20);

        let again = workspace.open_file("src/main.rs").expect("reopen");
        assert_eq!(again.buffer_id, snapshot.buffer_id);
    }

    #[test]
    fn language_mapping_covers_common_extensions() {
        assert_eq!(language_for(Path::new("a.tsx")), "typescript");
        assert_eq!(language_for(Path::new("a.jsx")), "javascript");
        assert_eq!(language_for(Path::new("a.css")), "css");
        assert_eq!(language_for(Path::new("a.json")), "json");
        assert_eq!(language_for(Path::new("a.md")), "markdown");
        assert_eq!(language_for(Path::new("a.toml")), "toml");
        assert_eq!(language_for(Path::new("a.unknown")), "text");
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        for path in [
            "",
            "/etc/passwd",
            "\\windows\\system32",
            "C:\\secret.txt",
            "../outside.txt",
            "./README.md",
            "src/../README.md",
            "src//main.rs",
        ] {
            let error = workspace.open_file(path).unwrap_err();
            assert_eq!(error.code, "invalid_path", "path {path:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn symlink_escapes_are_rejected_when_symlinks_are_available() {
        use std::os::windows::fs::symlink_file;

        let fixture = Fixture::new();
        let outside = fixture._directory.path().join("outside.txt");
        fs::write(&outside, "outside\n").expect("outside file");
        let link = fixture.root.join("escape.txt");
        if symlink_file(&outside, &link).is_err() {
            return;
        }
        let error = fixture.workspace().open_file("escape.txt").unwrap_err();
        assert_eq!(error.code, "path_escape");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escapes_are_rejected() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let outside = fixture._directory.path().join("outside.txt");
        fs::write(&outside, "outside\n").expect("outside file");
        symlink(&outside, fixture.root.join("escape.txt")).expect("symlink");
        let error = fixture.workspace().open_file("escape.txt").unwrap_err();
        assert_eq!(error.code, "path_escape");
    }

    #[test]
    fn binary_and_oversized_files_are_rejected() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("binary.dat"), [0_u8, 1, 2, 3]).expect("binary");
        fs::write(fixture.root.join("invalid.txt"), [0xff_u8, 0xfe]).expect("invalid");
        let large = fs::File::create(fixture.root.join("large.txt")).expect("large file");
        large
            .set_len(MAX_FILE_BYTES + 1)
            .expect("resize large file");

        let workspace = fixture.workspace();
        assert_eq!(
            workspace.open_file("binary.dat").unwrap_err().code,
            "binary_file"
        );
        assert_eq!(
            workspace.open_file("invalid.txt").unwrap_err().code,
            "binary_file"
        );
        assert_eq!(
            workspace.open_file("large.txt").unwrap_err().code,
            "file_too_large"
        );
    }

    #[test]
    fn apply_change_updates_single_and_multiline_ranges() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("edit.txt"), "alpha\nbeta\ngamma\n").expect("edit file");
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("edit.txt").expect("open");

        let changed = workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 0), position(0, 5), "ALPHA")],
            )
            .expect("single line change");
        assert_eq!(changed.version, 2);
        assert_eq!(changed.changes.len(), 1);
        let buffer = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert_eq!(buffer.text, "ALPHA\nbeta\ngamma\n");
        assert!(buffer.dirty);

        let changed = workspace
            .apply_change(
                &buffer.buffer_id,
                2,
                &[change(position(0, 0), position(2, 0), "one\ntwo\n")],
            )
            .expect("multiline change");
        assert_eq!(changed.version, 3);
        let buffer = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert_eq!(buffer.text, "one\ntwo\ngamma\n");
        assert!(buffer.dirty);
    }

    #[test]
    fn apply_change_resolves_every_change_against_the_pre_edit_text() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("multi.txt"), "alpha\nbeta\ngamma").expect("multi file");
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("multi.txt").expect("open");

        let changed = workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[
                    change(position(0, 0), position(0, 5), "one\ntwo"),
                    change(position(2, 0), position(2, 5), "omega"),
                ],
            )
            .expect("multi selection change");
        assert_eq!(changed.version, 2);
        assert_eq!(changed.changes.len(), 2);
        let buffer = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert_eq!(buffer.text, "one\ntwo\nbeta\nomega");
        assert!(buffer.dirty);
    }

    #[test]
    fn apply_change_rejects_overlapping_or_unsorted_changes_without_mutation() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("overlap.txt"), "alpha\nbeta\ngamma").expect("overlap file");
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("overlap.txt").expect("open");

        let overlapping = workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[
                    change(position(0, 0), position(1, 2), "x"),
                    change(position(1, 0), position(1, 4), "y"),
                ],
            )
            .unwrap_err();
        assert_eq!(overlapping.code, "invalid_change");

        let unsorted = workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[
                    change(position(2, 0), position(2, 5), "y"),
                    change(position(0, 0), position(0, 5), "x"),
                ],
            )
            .unwrap_err();
        assert_eq!(unsorted.code, "invalid_change");

        let buffer = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert_eq!(buffer.text, "alpha\nbeta\ngamma");
        assert_eq!(buffer.version, 1);
        assert!(!buffer.dirty);
    }

    #[test]
    fn apply_change_counts_utf16_columns_and_handles_crlf() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("astral.txt"), "a😀b\r\nsecond\n").expect("astral file");
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("astral.txt").expect("open");

        workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 3), position(0, 4), "X")],
            )
            .expect("astral change");
        let buffer = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert_eq!(buffer.text, "a😀X\r\nsecond\n");
        assert_eq!(buffer.version, 2);

        workspace
            .apply_change(
                &buffer.buffer_id,
                2,
                &[change(position(1, 0), position(1, 0), "> ")],
            )
            .expect("crlf line insert");
        let buffer = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert_eq!(buffer.text, "a😀X\r\n> second\n");
    }

    #[test]
    fn stale_versions_and_invalid_columns_are_rejected_without_mutation() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 0), position(0, 0), "x")],
            )
            .expect("first change");

        let stale = workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 0), position(0, 0), "y")],
            )
            .unwrap_err();
        assert_eq!(stale.code, "stale_buffer_version");

        let bad_column = workspace
            .apply_change(
                &buffer.buffer_id,
                2,
                &[change(position(0, 999), position(0, 999), "z")],
            )
            .unwrap_err();
        assert_eq!(bad_column.code, "invalid_change");
        let buffer = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert_eq!(buffer.text, "x# Project\n");
        assert_eq!(buffer.version, 2);
    }

    #[test]
    fn save_writes_disk_and_clears_dirty_without_bumping_version() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 0), position(0, 9), "# Saved")],
            )
            .expect("change");

        let saved = workspace.save_buffer(&buffer.buffer_id).expect("save");
        assert_eq!(saved.version, 2);
        assert!(!saved.dirty);
        assert_eq!(
            fs::read_to_string(fixture.root.join("README.md")).expect("disk read"),
            "# Saved\n"
        );
        let reopened = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert!(!reopened.dirty);
        assert_eq!(reopened.text, "# Saved\n");
    }

    #[test]
    fn close_buffer_forgets_it_and_buffers_are_sorted() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let readme = workspace.open_file("README.md").expect("readme");
        let main = workspace.open_file("src/main.rs").expect("main");
        let paths: Vec<_> = workspace
            .buffers()
            .into_iter()
            .map(|buffer| buffer.path.as_str().to_owned())
            .collect();
        assert_eq!(
            paths,
            vec!["README.md".to_owned(), "src/main.rs".to_owned()]
        );

        workspace.close_buffer(&main.buffer_id).expect("close");
        assert_eq!(
            workspace.buffer(&main.buffer_id).unwrap_err().code,
            "unknown_buffer"
        );
        assert_eq!(
            workspace.close_buffer(&main.buffer_id).unwrap_err().code,
            "unknown_buffer"
        );
        assert_eq!(workspace.buffers().len(), 1);
        assert_eq!(workspace.buffers()[0].buffer_id, readme.buffer_id);
    }

    #[test]
    fn active_buffer_tracks_switches_and_nearest_neighbor_on_close() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let readme = workspace.open_file("README.md").expect("readme");
        let main = workspace.open_file("src/main.rs").expect("main");
        let note = workspace.open_file("src/nested/note.txt").expect("note");

        // The last open is active, not the first path.
        assert_eq!(workspace.active_buffer_id(), Some(note.buffer_id.clone()));

        // Switching tabs records the choice for the next bootstrap.
        workspace
            .set_active_buffer(&main.buffer_id)
            .expect("select main");
        assert_eq!(workspace.active_buffer_id(), Some(main.buffer_id.clone()));

        // Closing the active last tab moves to its previous neighbor, not to
        // the alphabetically first README buffer.
        workspace
            .set_active_buffer(&note.buffer_id)
            .expect("select note");
        workspace.close_buffer(&note.buffer_id).expect("close note");
        assert_eq!(workspace.active_buffer_id(), Some(main.buffer_id.clone()));
        assert_ne!(workspace.active_buffer_id(), Some(readme.buffer_id.clone()));

        // Closing a non-active tab leaves the active one alone.
        workspace
            .set_active_buffer(&main.buffer_id)
            .expect("select main");
        workspace
            .close_buffer(&readme.buffer_id)
            .expect("close readme");
        assert_eq!(workspace.active_buffer_id(), Some(main.buffer_id.clone()));

        // A stale id is rejected and never becomes the active buffer.
        assert_eq!(
            workspace
                .set_active_buffer(&readme.buffer_id)
                .unwrap_err()
                .code,
            "unknown_buffer"
        );
        assert_eq!(workspace.active_buffer_id(), Some(main.buffer_id));
    }

    #[test]
    fn selection_is_kept_on_the_buffer_snapshot() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        let selection = Selection {
            anchor: position(0, 0),
            active: position(0, 2),
            version: 1,
        };
        workspace
            .set_selection(&buffer.buffer_id, selection.clone())
            .expect("selection");
        assert_eq!(
            workspace
                .buffer(&buffer.buffer_id)
                .expect("buffer")
                .selection,
            Some(selection)
        );
    }

    #[test]
    fn search_maps_literal_hits_to_relative_paths_and_one_based_columns() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let results = workspace
            .search(&SearchQueryDto {
                query: "needle".to_owned(),
                case_sensitive: true,
                whole_word: true,
                use_regex: false,
            })
            .expect("search");
        assert!(results.request_id.is_none());
        assert_eq!(results.hits.len(), 1);
        let hit = &results.hits[0];
        assert_eq!(hit.path.as_str(), "src/main.rs");
        assert_eq!(hit.line, 2);
        assert_eq!(hit.column, 9);
        assert!(hit.preview.contains("needle"));
    }

    #[test]
    fn search_maps_regex_hits_to_relative_paths_and_one_based_columns() {
        let fixture = Fixture::new();
        let results = fixture
            .workspace()
            .search(&SearchQueryDto {
                query: r"need\w+".to_owned(),
                case_sensitive: true,
                whole_word: true,
                use_regex: true,
            })
            .expect("regex search");
        assert!(results.request_id.is_none());
        assert_eq!(results.hits.len(), 1);
        let hit = &results.hits[0];
        assert_eq!(hit.path.as_str(), "src/main.rs");
        assert_eq!(hit.line, 2);
        assert_eq!(hit.column, 9);
        assert!(hit.preview.contains("needle"));
    }

    #[test]
    fn regex_search_reports_an_invalid_pattern_as_invalid_query() {
        let fixture = Fixture::new();
        let error = fixture
            .workspace()
            .search(&SearchQueryDto {
                query: "(unclosed".to_owned(),
                case_sensitive: false,
                whole_word: false,
                use_regex: true,
            })
            .unwrap_err();
        assert_eq!(error.code, "invalid_query");
        assert!(error.message.contains("invalid regular expression"));
    }

    /// Init a repo at `root` whose initial commit holds the fixture files.
    fn init_repo_with_fixture_commit(root: &Path) {
        let repo = git2::Repository::init(root).expect("init repo");
        {
            let mut index = repo.index().expect("index");
            for path in ["README.md", "src/main.rs", "src/nested/note.txt"] {
                index.add_path(Path::new(path)).expect("add fixture file");
            }
            index.write().expect("write index");
            let tree_id = index.write_tree().expect("write tree");
            let tree = repo.find_tree(tree_id).expect("find tree");
            let signature =
                git2::Signature::now("Selora Test", "test@selora.invalid").expect("signature");
            repo.commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])
                .expect("initial commit");
        }
    }

    #[test]
    fn staged_git_diff_shows_the_head_to_index_side() {
        let fixture = Fixture::new();
        init_repo_with_fixture_commit(&fixture.root);

        fs::write(fixture.root.join("README.md"), "# Project\nchanged\n").expect("edit readme");
        let workspace = fixture.workspace();
        let staged = workspace.git_stage("README.md").expect("stage");
        assert_eq!(staged.staged_paths.len(), 1);

        // A staged-only row must show the HEAD→index diff, not an empty
        // index→worktree diff.
        let diff = workspace.git_diff("README.md").expect("staged diff");
        assert!(
            diff.unified_diff.contains("+changed"),
            "staged diff was empty: {}",
            diff.unified_diff
        );
    }

    #[test]
    fn git_status_keeps_a_partially_staged_file_in_both_slots() {
        let fixture = Fixture::new();
        init_repo_with_fixture_commit(&fixture.root);
        let workspace = fixture.workspace();

        // Stage one edit, then layer a *further* unstaged edit on top — the
        // `MM` case the flattened arrays cannot describe.
        fs::write(fixture.root.join("README.md"), "# Project\nstaged\n").expect("staged edit");
        workspace.git_stage("README.md").expect("stage");
        fs::write(
            fixture.root.join("README.md"),
            "# Project\nstaged\nunstaged\n",
        )
        .expect("unstaged edit");

        let status = workspace.git_status().expect("status");
        let file = status
            .files
            .iter()
            .find(|file| file.path.as_str() == "README.md")
            .expect("README.md in files");
        assert!(file.staged, "index side must stay dirty");
        assert!(file.worktree, "worktree side must stay dirty");
        // The legacy arrays keep their old (flattened) meaning.
        assert!(status
            .staged_paths
            .iter()
            .any(|path| path.as_str() == "README.md"));
        assert!(status
            .changed_paths
            .iter()
            .any(|path| path.as_str() == "README.md"));

        // The unstaged delta is reachable: staging again clears the worktree
        // slot while the staged slot stays set.
        let restaged = workspace.git_stage("README.md").expect("restage");
        let file = restaged
            .files
            .iter()
            .find(|file| file.path.as_str() == "README.md")
            .expect("README.md in files");
        assert!(file.staged);
        assert!(!file.worktree, "the unstaged delta has been staged");
    }

    #[test]
    fn git_status_separates_staged_only_and_untracked_files() {
        let fixture = Fixture::new();
        init_repo_with_fixture_commit(&fixture.root);
        fs::write(fixture.root.join("src/untracked.rs"), "fn new() {}\n").expect("untracked");
        let workspace = fixture.workspace();

        fs::write(fixture.root.join("README.md"), "# Project\nstaged only\n").expect("staged edit");
        let status = workspace.git_stage("README.md").expect("stage");
        assert_eq!(status.files.len(), 2, "{:?}", status.files);

        let file = |path: &str| {
            status
                .files
                .iter()
                .find(|file| file.path.as_str() == path)
                .cloned()
        };
        let readme = file("README.md").expect("README.md");
        assert!(readme.staged);
        assert!(!readme.worktree, "a fully staged file has no worktree side");
        let untracked = file("src/untracked.rs").expect("untracked file");
        assert!(!untracked.staged, "untracked files are not staged");
        assert!(untracked.worktree, "untracked files are worktree-only");
        assert_eq!(
            status
                .staged_paths
                .iter()
                .map(WorkspacePath::as_str)
                .collect::<Vec<_>>(),
            vec!["README.md"]
        );
        assert!(status
            .changed_paths
            .iter()
            .any(|path| path.as_str() == "src/untracked.rs"));
    }

    #[test]
    fn git_is_empty_without_a_repository() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let status = workspace.git_status().expect("status");
        assert!(status.branch.is_none());
        assert!(status.changed_paths.is_empty());
        assert!(status.staged_paths.is_empty());
        assert!(status.files.is_empty());
        assert_eq!(
            workspace.git_diff("README.md").unwrap_err().code,
            "not_a_git_repo"
        );
        assert_eq!(
            workspace.git_stage("README.md").unwrap_err().code,
            "not_a_git_repo"
        );
        assert_eq!(
            workspace.git_commit("message").unwrap_err().code,
            "not_a_git_repo"
        );
    }

    #[test]
    fn refresh_reloads_a_clean_buffer_once_and_keeps_its_identity() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        let selection = Selection {
            anchor: position(0, 0),
            active: position(0, 4),
            version: 1,
        };
        workspace
            .set_selection(&buffer.buffer_id, selection.clone())
            .expect("selection");
        fs::write(fixture.root.join("README.md"), "# Changed on disk\n").expect("external write");

        let outcome = workspace.refresh_clean_buffers();
        assert_eq!(outcome.updated.len(), 1, "{outcome:#?}");
        assert!(outcome.dirty.is_empty(), "{outcome:#?}");
        assert!(outcome.skipped.is_empty(), "{outcome:#?}");
        let snapshot = &outcome.updated[0];
        assert_eq!(snapshot.buffer_id, buffer.buffer_id);
        assert_eq!(snapshot.path.as_str(), "README.md");
        assert_eq!(snapshot.language, "markdown");
        assert_eq!(snapshot.text, "# Changed on disk\n");
        assert_eq!(
            snapshot.version, 2,
            "a reload bumps the version exactly once"
        );
        assert!(!snapshot.dirty);
        assert_eq!(
            snapshot.selection,
            Some(selection),
            "selection is preserved"
        );

        let stored = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert_eq!(stored.text, "# Changed on disk\n");
        assert_eq!(stored.version, 2);

        // Nothing changed since the reload: no second bump, no snapshot.
        let again = workspace.refresh_clean_buffers();
        assert!(again.updated.is_empty(), "{again:#?}");
        assert_eq!(workspace.buffer(&buffer.buffer_id).unwrap().version, 2);
    }

    #[test]
    fn refresh_leaves_a_dirty_buffer_alone_and_reports_a_conflict() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 0), position(0, 9), "# Local")],
            )
            .expect("local edit");
        fs::write(fixture.root.join("README.md"), "# Workflow won\n").expect("external write");

        let outcome = workspace.refresh_clean_buffers();
        assert!(outcome.updated.is_empty(), "{outcome:#?}");
        assert_eq!(
            outcome
                .dirty
                .iter()
                .map(WorkspacePath::as_str)
                .collect::<Vec<_>>(),
            vec!["README.md"]
        );
        assert_eq!(outcome.conflicts.len(), 1, "{outcome:#?}");
        assert_eq!(outcome.conflicts[0].buffer_id, buffer.buffer_id);
        assert_eq!(outcome.conflicts[0].path.as_str(), "README.md");
        assert!(outcome.skipped.is_empty(), "{outcome:#?}");
        let stored = workspace.buffer(&buffer.buffer_id).expect("buffer");
        assert_eq!(stored.text, "# Local\n", "unsaved local edits survive");
        assert_eq!(stored.version, 2, "a dirty buffer is not reloaded");
        assert!(stored.dirty);
        assert!(
            workspace.conflict_for(&buffer.buffer_id).is_some(),
            "the conflict is tracked in memory"
        );
    }

    #[test]
    fn refresh_does_not_flag_a_dirty_buffer_whose_disk_baseline_is_unchanged() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 0), position(0, 9), "# Local")],
            )
            .expect("local edit");

        let outcome = workspace.refresh_clean_buffers();
        assert!(outcome.updated.is_empty(), "{outcome:#?}");
        assert!(outcome.conflicts.is_empty(), "{outcome:#?}");
        assert_eq!(outcome.dirty.len(), 1, "{outcome:#?}");
        assert!(workspace.conflict_for(&buffer.buffer_id).is_none());
        assert_eq!(
            workspace.buffer(&buffer.buffer_id).unwrap().text,
            "# Local\n"
        );
    }

    #[test]
    fn reload_from_disk_discards_the_conflict_and_bumps_the_version() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 0), position(0, 9), "# Local")],
            )
            .expect("local edit");
        fs::write(fixture.root.join("README.md"), "# Workflow won\n").expect("external write");
        workspace.refresh_clean_buffers();

        let reloaded = workspace
            .reload_buffer_from_disk(&buffer.buffer_id)
            .expect("reload");
        assert_eq!(reloaded.text, "# Workflow won\n");
        assert_eq!(reloaded.version, 3);
        assert!(!reloaded.dirty);
        assert!(workspace.conflict_for(&buffer.buffer_id).is_none());
        assert_eq!(
            fs::read_to_string(fixture.root.join("README.md")).unwrap(),
            "# Workflow won\n",
            "reload never writes the discarded local text to disk"
        );
        assert_eq!(
            workspace
                .reload_buffer_from_disk(&buffer.buffer_id)
                .unwrap_err()
                .code,
            "no_buffer_conflict",
            "a second reload cannot discard edits after the decision"
        );
    }

    #[test]
    fn keep_edits_clears_the_conflict_and_preserves_text_and_version() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 0), position(0, 9), "# Local")],
            )
            .expect("local edit");
        fs::write(fixture.root.join("README.md"), "# Workflow won\n").expect("external write");
        workspace.refresh_clean_buffers();

        let kept = workspace
            .keep_buffer_edits(&buffer.buffer_id)
            .expect("keep edits");
        assert_eq!(kept.text, "# Local\n", "local text survives");
        assert_eq!(kept.version, 2, "keep does not bump the version");
        assert!(kept.dirty);
        assert!(workspace.conflict_for(&buffer.buffer_id).is_none());
        assert_eq!(
            fs::read_to_string(fixture.root.join("README.md")).unwrap(),
            "# Workflow won\n",
            "keep never overwrites the disk"
        );
        assert_eq!(
            workspace
                .keep_buffer_edits(&buffer.buffer_id)
                .unwrap_err()
                .code,
            "no_buffer_conflict"
        );
    }

    #[test]
    fn save_after_a_conflict_clears_the_marker_and_writes_local_text() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        workspace
            .apply_change(
                &buffer.buffer_id,
                1,
                &[change(position(0, 0), position(0, 9), "# Local")],
            )
            .expect("local edit");
        fs::write(fixture.root.join("README.md"), "# Workflow won\n").expect("external write");
        workspace.refresh_clean_buffers();

        let saved = workspace.save_buffer(&buffer.buffer_id).expect("save");
        assert_eq!(saved.text, "# Local\n");
        assert!(!saved.dirty);
        let cleared = workspace
            .take_conflict(&buffer.buffer_id)
            .expect("save leaves the marker for the command layer to clear");
        assert_eq!(cleared.buffer_id, buffer.buffer_id);
        assert!(workspace.conflict_for(&buffer.buffer_id).is_none());
        assert_eq!(
            fs::read_to_string(fixture.root.join("README.md")).unwrap(),
            "# Local\n",
        );
    }

    #[test]
    fn refresh_keeps_a_tab_whose_file_was_deleted() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let buffer = workspace.open_file("README.md").expect("open");
        fs::remove_file(fixture.root.join("README.md")).expect("delete");

        let outcome = workspace.refresh_clean_buffers();
        assert!(outcome.updated.is_empty(), "{outcome:#?}");
        assert!(outcome.dirty.is_empty(), "{outcome:#?}");
        assert_eq!(outcome.skipped.len(), 1, "{outcome:#?}");
        let skipped = &outcome.skipped[0];
        assert_eq!(skipped.path.as_str(), "README.md");
        assert_eq!(skipped.code, "file_missing");
        assert!(skipped.message.contains("no longer exists"), "{skipped:#?}");
        let stored = workspace.buffer(&buffer.buffer_id).expect("tab stays open");
        assert_eq!(stored.text, "# Project\n");
        assert_eq!(stored.version, 1);
        assert_eq!(workspace.buffers().len(), 1);
    }

    #[test]
    fn refresh_skips_binary_and_oversized_files_with_a_reason() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let readme = workspace.open_file("README.md").expect("open readme");
        let main = workspace.open_file("src/main.rs").expect("open main");
        fs::write(fixture.root.join("README.md"), [0xff_u8, 0xfe]).expect("invalid utf8");
        fs::File::create(fixture.root.join("src/main.rs"))
            .expect("large file")
            .set_len(MAX_FILE_BYTES + 1)
            .expect("resize");

        let outcome = workspace.refresh_clean_buffers();
        assert!(outcome.updated.is_empty(), "{outcome:#?}");
        let codes: Vec<_> = outcome
            .skipped
            .iter()
            .map(|skipped| (skipped.path.as_str(), skipped.code))
            .collect();
        assert_eq!(
            codes,
            vec![
                ("README.md", "binary_file"),
                ("src/main.rs", "file_too_large"),
            ]
        );
        // Both tabs stay open with their last known text.
        assert_eq!(
            workspace.buffer(&readme.buffer_id).unwrap().text,
            "# Project\n"
        );
        assert_eq!(
            workspace.buffer(&main.buffer_id).unwrap().text,
            "fn main() {\n    let needle = 1;\n}\n"
        );
    }
}
