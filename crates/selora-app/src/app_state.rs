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

use std::path::PathBuf;

use selora_ai::{ChatReceipt, ChatRequest, Model};
use selora_core::{BufferId, SeloraError, Workspace};
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

    // Settings (payload type lands with Phase 2f)
    SettingsLoad,
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
}

/// The main-thread state (ARCHITECTURE.md §3.1, Phase 2 subset). Owned by
/// the UI thread; workers never touch it — they only send [`AppMessage`]s.
#[derive(Debug, Default)]
pub struct AppState {
    pub workspace: Option<Workspace>,
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
