//! Workspace-state persistence (ARCHITECTURE.md §8.2, Phase 2 acceptance #9).
//!
//! `workspace.json` lives next to `settings.json` in `%APPDATA%\selora-ide\`
//! (same settings-dir logic — `SELORA_SETTINGS_DIR` override honored) and
//! holds `{ root, open_files (tab order), active_path }`. It is written on
//! graceful close (eframe exit) and every 5s while running, atomically
//! (temp file + rename in the same directory). On boot the state is restored
//! if the recorded root still exists on disk; a stale or unparsable file is
//! ignored (first-run semantics), never fatal.
//!
//! `sessions.json` (Phase R3) sits beside it with the chat's full session
//! history — timeline entries including agent tool cards, serde-mirrored in
//! `selora_ui::sidebar::chat`. Same atomic write, same 5s cadence; the
//! legacy `chat_turns` remain workspace.json's fallback for pre-R3 files.

use std::path::{Path, PathBuf};

use selora_core::SeloraError;
use selora_settings::Settings;
use selora_ui::sidebar::chat::ChatSession;
use serde::{Deserialize, Serialize};

/// The file name inside the settings directory (ARCHITECTURE.md §8.2).
pub const WORKSPACE_FILE_NAME: &str = "workspace.json";
/// The sessions file's name (Phase R3).
pub const SESSIONS_FILE_NAME: &str = "sessions.json";
/// The sessions file's schema version — a bump refuses to load (the
/// legacy `chat_turns` fallback covers the boot).
pub const SESSIONS_SCHEMA: u32 = 1;

/// One persisted chat turn (Phase 4c): the role tag ("user" /
/// "assistant") and the content. `workspace.json`'s chat unit — the shell
/// converts to/from the chat panel's own turn type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatTurnState {
    pub role: String,
    pub content: String,
}

/// The persisted session (ARCHITECTURE.md §8.2): the workspace root, the
/// open files in tab order, the active file, and the chat sidebar's
/// visibility + last turns (Phase 4c).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub root: Option<PathBuf>,
    pub open_files: Vec<PathBuf>,
    pub active_path: Option<PathBuf>,
    /// Whether the chat sidebar was open at close; older files default to
    /// closed.
    #[serde(default)]
    pub chat_open: bool,
    /// The last 20 chat turns (the Phase 4 brief's cap is enforced by the
    /// panel's `snapshot_turns`, not here).
    #[serde(default)]
    pub chat_turns: Vec<ChatTurnState>,
    /// Whether the chat header's Agent Mode pill was on (Phase 5d); older
    /// files default to off.
    #[serde(default)]
    pub agent_mode: bool,
}

/// `workspace.json`'s path inside the settings directory `dir`.
pub fn workspace_path(dir: &Path) -> PathBuf {
    dir.join(WORKSPACE_FILE_NAME)
}

/// `sessions.json` (Phase R3): the chat panel's session history with a
/// schema gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionsState {
    pub schema: u32,
    pub sessions: Vec<ChatSession>,
}

/// `sessions.json`'s path inside the settings directory `dir`.
pub fn sessions_path(dir: &Path) -> PathBuf {
    dir.join(SESSIONS_FILE_NAME)
}

/// Save the session history atomically (temp + rename, the
/// workspace.json pattern).
pub fn save_sessions(dir: &Path, sessions: &[ChatSession]) -> Result<(), SeloraError> {
    let state = SessionsState {
        schema: SESSIONS_SCHEMA,
        sessions: sessions.to_vec(),
    };
    let json = serde_json::to_string_pretty(&state)
        .map_err(|error| SeloraError::Config(format!("failed to serialize sessions: {error}")))?;

    std::fs::create_dir_all(dir)?;
    let target = sessions_path(dir);
    let mut tmp = target.clone().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    std::fs::write(&tmp, json.as_bytes())?;
    match std::fs::rename(&tmp, &target) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&tmp);
            Err(SeloraError::Io(error))
        }
    }
}

/// Load the persisted sessions. `None` when the file is missing,
/// unparsable, or schema-mismatched — callers fall back to the legacy
/// `chat_turns` (never fatal).
pub fn load_sessions(dir: &Path) -> Option<Vec<ChatSession>> {
    let text = std::fs::read_to_string(sessions_path(dir)).ok()?;
    let state: SessionsState = serde_json::from_str(&text).ok()?;
    if state.schema != SESSIONS_SCHEMA {
        return None;
    }
    Some(state.sessions)
}

/// Resolve the settings directory for workspace persistence: the
/// `SELORA_SETTINGS_DIR` override, else `%APPDATA%\selora-ide`. `None` when
/// neither is available (headless / tests) — persistence is then skipped,
/// not fatal.
pub fn state_dir() -> Option<PathBuf> {
    Settings::settings_dir().ok()
}

/// Save the session state atomically (temp + rename, same directory).
pub fn save(dir: &Path, state: &WorkspaceState) -> Result<(), SeloraError> {
    let json = serde_json::to_string_pretty(state).map_err(|error| {
        SeloraError::Config(format!("failed to serialize workspace state: {error}"))
    })?;

    std::fs::create_dir_all(dir)?;
    let target = workspace_path(dir);
    let mut tmp = target.clone().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    std::fs::write(&tmp, json.as_bytes())?;
    match std::fs::rename(&tmp, &target) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Never leave the temp file behind on a failed rename.
            let _ = std::fs::remove_file(&tmp);
            Err(SeloraError::Io(error))
        }
    }
}

/// Load the persisted session from `dir`. `None` when the file is missing,
/// unparsable, or records no root — callers treat all three as "no session"
/// (ARCHITECTURE.md §8.2: a stale file is ignored, not an error).
pub fn load(dir: &Path) -> Option<WorkspaceState> {
    let text = std::fs::read_to_string(workspace_path(dir)).ok()?;
    let state: WorkspaceState = serde_json::from_str(&text).ok()?;
    state.root.as_ref()?;
    Some(state)
}

/// The restorable form of a loaded session: the root only if it still exists
/// on disk, and the open files that still exist (a deleted file is dropped,
/// not fatal — it simply does not reopen).
pub fn restorable(state: &WorkspaceState) -> WorkspaceState {
    let root = state.root.as_ref().filter(|root| root.is_dir()).cloned();
    let open_files = match &root {
        Some(root) => state
            .open_files
            .iter()
            .filter(|path| path.is_file() && path.strip_prefix(root).is_ok())
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    let active_path = state
        .active_path
        .as_ref()
        .filter(|path| open_files.contains(path))
        .cloned();
    WorkspaceState {
        root,
        open_files,
        active_path,
        // Chat state is not path-dependent — it restores verbatim.
        chat_open: state.chat_open,
        chat_turns: state.chat_turns.clone(),
        agent_mode: state.agent_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let n = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "selora-app-persistence-test-{}-{n}",
            std::process::id()
        ))
    }

    fn fixture_session() -> (PathBuf, WorkspaceState) {
        // A real temp workspace so the restore filters pass.
        let root = temp_dir();
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("src").join("main.rs"), b"fn main() {}").expect("write");
        std::fs::write(root.join("Cargo.toml"), b"[package]").expect("write");

        let state = WorkspaceState {
            root: Some(root.clone()),
            open_files: vec![root.join("Cargo.toml"), root.join("src").join("main.rs")],
            active_path: Some(root.join("src").join("main.rs")),
            chat_open: false,
            chat_turns: Vec::new(),
            agent_mode: false,
        };
        (root, state)
    }

    #[test]
    fn round_trip_save_then_load_is_equal() {
        let (root, state) = fixture_session();
        let dir = temp_dir();
        save(&dir, &state).expect("save");

        assert!(workspace_path(&dir).is_file(), "workspace.json written");
        assert!(!dir.join(format!("{WORKSPACE_FILE_NAME}.tmp")).is_file());
        assert_eq!(load(&dir).as_ref(), Some(&state));

        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Phase 4c: the chat sidebar's visibility and turns round-trip, and a
    /// pre-Phase-4c file (no chat fields) still loads with defaults.
    #[test]
    fn chat_state_round_trips_and_old_files_default() {
        let (root, state) = fixture_session();
        let mut state = state;
        state.chat_open = true;
        state.chat_turns = vec![
            ChatTurnState {
                role: "user".into(),
                content: "write a haiku".into(),
            },
            ChatTurnState {
                role: "assistant".into(),
                content: "line one\nline two\nline three".into(),
            },
        ];
        let dir = temp_dir();
        save(&dir, &state).expect("save");
        let loaded = load(&dir).expect("load");
        assert!(loaded.chat_open);
        assert_eq!(loaded.chat_turns, state.chat_turns);

        // The pre-Phase-4c shape: no chat fields at all → defaults.
        let legacy = serde_json::json!({
            "root": root,
            "open_files": [],
            "active_path": null,
        });
        std::fs::write(workspace_path(&dir), legacy.to_string()).expect("write legacy");
        let loaded = load(&dir).expect("legacy load");
        assert!(!loaded.chat_open);
        assert!(loaded.chat_turns.is_empty());
        assert!(!loaded.agent_mode, "pre-5d files default the pill to off");

        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Phase 5d: the agent-mode pill round-trips like `chat_open`.
    #[test]
    fn agent_mode_round_trips() {
        let (root, state) = fixture_session();
        let state = WorkspaceState {
            agent_mode: true,
            ..state
        };
        let dir = temp_dir();
        save(&dir, &state).expect("save");
        assert_eq!(load(&dir).as_ref(), Some(&state));
        // The restore filter carries it verbatim (not path-dependent).
        assert!(restorable(&state).agent_mode);

        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn save_is_an_atomic_replace() {
        let (root, mut state) = fixture_session();
        let dir = temp_dir();
        state.open_files.push(root.join("Cargo.toml"));
        save(&dir, &state).expect("first save");
        state.open_files.pop();
        save(&dir, &state).expect("second save over the first");

        assert_eq!(load(&dir).as_ref(), Some(&state));
        assert!(!dir.join(format!("{WORKSPACE_FILE_NAME}.tmp")).is_file());

        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_or_garbage_file_loads_none() {
        let dir = temp_dir();
        assert_eq!(load(&dir), None);

        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(workspace_path(&dir), b"{ not json").expect("garbage");
        assert_eq!(load(&dir), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn state_without_root_loads_none() {
        let dir = temp_dir();
        save(&dir, &WorkspaceState::default()).expect("save");
        // Recorded nothing (never opened a folder): no session to restore.
        assert_eq!(load(&dir), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn restorable_drops_missing_root_and_files() {
        let (root, mut state) = fixture_session();
        // One file that no longer exists, one real one.
        state.open_files = vec![root.join("gone.rs"), root.join("src").join("main.rs")];
        state.active_path = Some(root.join("gone.rs"));

        let restored = restorable(&state);
        assert_eq!(restored.root, Some(root.clone()));
        assert_eq!(restored.open_files, vec![root.join("src").join("main.rs")]);
        // The active file must be one of the restored tabs.
        assert_eq!(restored.active_path, None);

        // A vanished root restores nothing at all.
        let mut stale = state.clone();
        stale.root = Some(PathBuf::from("/does/not/exist"));
        assert_eq!(restorable(&stale).root, None);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn restorable_rejects_files_outside_the_root() {
        let (root, mut state) = fixture_session();
        let outside_dir = temp_dir();
        std::fs::create_dir_all(&outside_dir).expect("mkdir outside dir");
        let outside = outside_dir.join("elsewhere.rs");
        std::fs::write(&outside, b"x").expect("write");
        state.open_files = vec![outside.clone(), root.join("Cargo.toml")];

        let restored = restorable(&state);
        assert_eq!(restored.open_files, vec![root.join("Cargo.toml")]);

        let _ = std::fs::remove_dir_all(outside_dir);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Phase R3: sessions round-trip including an agent tool card; a
    /// garbage or schema-mismatched file loads `None` (the legacy
    /// fallback), never an error.
    #[test]
    fn sessions_round_trip_with_cards_and_refuse_garbage() {
        use selora_ai::{AgentDecision, AgentSummary, ApprovalCard, ApprovalId, ToolResult};
        use selora_ui::sidebar::chat::{ChatSession, SessionCard, SessionEntry};

        let dir = temp_dir();
        let mut card = SessionCard {
            id: "call_1".into(),
            name: "edit_file".into(),
            arguments: r#"{"path":"main.rs"}"#.into(),
            approval: Some((
                ApprovalId::nil(),
                ApprovalCard::Diff {
                    path: "main.rs".into(),
                    diff: "@@ -1 +1 @@\n-fn main() {}\n+// entry\nfn main() {}".into(),
                },
                1,
            )),
            result: Some(ToolResult {
                id: "call_1".into(),
                ok: true,
                content: "edited".into(),
            }),
            decided: Some(AgentDecision::Approve),
            expanded: true,
        };
        let _ = &mut card;
        let sessions = vec![ChatSession {
            id: uuid::Uuid::new_v4(),
            title: "fix the build".into(),
            created_at: 1_700_000_000,
            entries: vec![
                SessionEntry::Bubble {
                    role: selora_ai::Role::User,
                    content: "fix the build".into(),
                    error: None,
                },
                SessionEntry::Card(card),
                SessionEntry::Summary(AgentSummary {
                    iterations: 2,
                    tools_used: 1,
                    approved: 1,
                    denied: 0,
                    tokens: 500,
                    cost_usd: 0.01,
                    completed: true,
                }),
            ],
        }];
        save_sessions(&dir, &sessions).expect("save sessions");
        assert!(sessions_path(&dir).is_file());
        assert_eq!(load_sessions(&dir).as_ref(), Some(&sessions));

        // Garbage and schema mismatch both load None (the fallback).
        std::fs::write(sessions_path(&dir), b"{ nope").expect("garbage");
        assert_eq!(load_sessions(&dir), None);
        let wrong_schema = serde_json::json!({ "schema": 99, "sessions": [] });
        std::fs::write(sessions_path(&dir), wrong_schema.to_string()).expect("wrong schema");
        assert_eq!(load_sessions(&dir), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_failure_is_a_clean_error_not_a_panic() {
        // A file where the directory should be: create_dir_all fails.
        let blocker = temp_dir();
        std::fs::create_dir_all(&blocker).expect("mkdir");
        let not_a_dir = blocker.join("blocker");
        std::fs::write(&not_a_dir, b"").expect("write blocker");

        let state = WorkspaceState::default();
        let result = save(&not_a_dir, &state);
        assert!(result.is_err());
        assert!(matches!(result, Err(SeloraError::Io(_))), "got: {result:?}");

        let _ = std::fs::remove_dir_all(blocker);
    }
}
