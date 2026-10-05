//! The Phase 2 app-state spine (ARCHITECTURE.md §2, §3.1).
//!
//! `Effect` (main thread → runtime) and `AppMessage` (runtime → main thread)
//! are the Phase 2 contract. **This file is orchestrator-owned**: parallel
//! Phase 2 subagents propose additions as diffs in their reports; the
//! orchestrator applies them here (AGENT_ORCHESTRATION.md §10, shared-file
//! rule). Only variants whose payload types already exist are declared now —
//! fs/git/lsp/settings variants land with the crates that define their types.
//!
//! The spike's `wiring.rs` message set (selora-ai subset) is subsumed here;
//! `wiring.rs` stays until the Phase 2 shell replaces the spike UI.
//!
//! Nothing in this module is consumed by the spike UI yet — the Phase 2
//! subagent merges wire it in. Silence dead-code until then.
#![expect(dead_code)]

use std::path::PathBuf;

use selora_ai::{ChatReceipt, ChatRequest, Model};
use selora_core::{BufferId, SeloraError, Workspace};
use selora_fs::tree::FileTreeView;
use selora_settings::Settings;
use selora_ui::editor::split::EditorArea;
use selora_ui::palette::command::CommandPalette;
use uuid::Uuid;

/// Effects the main thread sends to the tokio runtime (ARCHITECTURE.md §2.2).
#[derive(Debug)]
pub enum Effect {
    // File system
    OpenFile(PathBuf),
    OpenFolder(PathBuf),
    RefreshFileTree(PathBuf),
    SaveBuffer(BufferId),

    // Selora
    SeloraConnect(String),
    SeloraListModels,
    SeloraChat(Uuid, ChatRequest),
    /// Write the Selora key to the OS keyring (ARCHITECTURE.md §2.2; the
    /// runtime side lands in Phase 4, with the keyring crate).
    SeloraKeyringSet(String),

    // Settings (selora-settings, Phase 2f)
    SettingsLoad,
    /// Persist `Settings` (ARCHITECTURE.md §2.2) — the runtime calls
    /// `Settings::save` off the frame path.
    SettingsSave(Settings),
}

/// Messages workers send to the main thread (ARCHITECTURE.md §2.1).
#[derive(Debug)]
pub enum AppMessage {
    // Selora
    ModelsLoaded(Result<Vec<Model>, SeloraError>),
    ChatDelta(Uuid, String),
    ChatDone(Uuid, ChatReceipt),
    ChatError(Uuid, SeloraError),

    // File system (payload type lands with Phase 2b)
    FolderOpened(PathBuf),
    /// A fresh file tree for `root` (ARCHITECTURE.md §2.1; built by the
    /// worker via `selora_fs::tree::build_tree`).
    FileTreeUpdated {
        root: PathBuf,
        tree: selora_fs::tree::FileTreeNode,
    },

    // Settings (Phase 2f)
    SettingsLoaded(Settings),
    /// A save completed — the shell toasts "Settings saved" (UI_SPEC.md §6).
    SettingsSaved,
    /// A load/save failed (including the `_v`-mismatch wipe error) — toast.
    SettingsError(SeloraError),

    // App
    /// A keyring operation failed (ARCHITECTURE.md §2.1; runtime side in
    /// Phase 4).
    KeyringError(SeloraError),
}

/// The main-thread state (ARCHITECTURE.md §3.1, Phase 2 subset). Owned by
/// the UI thread; workers never touch it — they only send [`AppMessage`]s.
#[derive(Debug, Default)]
pub struct AppState {
    pub workspace: Option<Workspace>,
    /// Editor-area chrome: the split tree with one tab set per pane
    /// (UI_SPEC.md §3.1, §3.3). `Default` = one empty pane.
    pub editor_area: EditorArea,
    /// The file tree view state (expanded set, selection) for the open
    /// folder. Fed by [`AppMessage::FileTreeUpdated`].
    pub file_tree: FileTreeView,
    /// Command palette visibility + selection (UI_SPEC.md §5). The palette
    /// emits `CommandId`s; routing lives in the shell frame loop.
    pub palette: CommandPalette,
    /// User settings (ARCHITECTURE.md §8.1). Loaded at boot via
    /// `Effect::SettingsLoad`; the Settings dialog mutates it and saves
    /// via `Effect::SettingsSave`.
    pub settings: Settings,
}

impl AppState {
    /// Open a file into the active pane; keep `workspace.open_files` in sync
    /// (ARCHITECTURE.md §3.3 — the same focus-order semantics on both sides).
    pub fn open_file(&mut self, path: PathBuf) {
        self.editor_area.open_in_active(path.clone());
        if let Some(workspace) = &mut self.workspace {
            workspace.open(path);
        }
    }

    /// `Ctrl+\` (UI_SPEC.md §15): split the active editor pane.
    pub fn split_editor(&mut self) {
        self.editor_area.request_split();
    }

    /// `Ctrl+W`: close the active tab (prunes an emptied pane). Returns the
    /// closed path so the shell can drop the buffer (and later fire LspClose).
    pub fn close_editor(&mut self) -> Option<PathBuf> {
        let closed = self.editor_area.request_close();
        if let (Some(path), Some(workspace)) = (&closed, &mut self.workspace) {
            workspace.close(path);
        }
        closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_open_and_close_round_trip() {
        let mut state = AppState::default();
        assert!(state.workspace.is_none());
        state.workspace = Some(Workspace::new("/repo"));
        state.workspace.as_mut().unwrap().open("a.rs");
        assert_eq!(
            state.workspace.as_ref().unwrap().active_path,
            Some(PathBuf::from("a.rs"))
        );
    }
}
