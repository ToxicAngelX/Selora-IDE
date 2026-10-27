//! Versioned DTOs for the Selora desktop shell boundary.
//!
//! Rust owns files, buffers, gateway credentials and approvals. A frontend
//! receives a safe snapshot and ordered incremental events through this crate.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};
use url::Url;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_ID_LENGTH: usize = 128;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct WorkspacePath(String);
impl WorkspacePath {
    pub fn new(value: impl Into<String>) -> Result<Self, ProtocolValidationError> {
        let value = value.into();
        validate_workspace_relative_path(&value)?;
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for WorkspacePath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl From<WorkspacePath> for String {
    fn from(value: WorkspacePath) -> Self {
        value.0
    }
}
impl<'de> Deserialize<'de> for WorkspacePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

macro_rules! stable_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ProtocolValidationError> {
                let value = value.into();
                validate_stable_id(&value)?;
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
    };
}
stable_id!(WorkspaceId);
stable_id!(CoreInstanceId);
stable_id!(SessionId);
stable_id!(BufferId);
stable_id!(RequestId);
stable_id!(TurnId);
stable_id!(ApprovalId);
stable_id!(MessageId);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolValidationError {
    EmptyId,
    IdTooLong {
        max: usize,
    },
    IdContainsControlCharacter,
    InvalidExternalUrl,
    InvalidWorkspacePath,
    /// A region size that is not a finite number (Phase Z2, ARCHITECTURE
    /// §1.1). Unreachable from the wire — JSON has no NaN/±Inf spelling and
    /// serde_json rejects out-of-range number literals at the parse boundary —
    /// so the check stands as a Rust-internal invariant for non-wire callers.
    InvalidRegionSize,
}
impl fmt::Display for ProtocolValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId => f.write_str("stable id must not be empty"),
            Self::IdTooLong { max } => write!(f, "stable id exceeds {max} characters"),
            Self::IdContainsControlCharacter => {
                f.write_str("stable id contains a control character")
            }
            Self::InvalidExternalUrl => f.write_str("external URL must be an absolute HTTPS URL"),
            Self::InvalidWorkspacePath => f.write_str("path must be workspace-relative and safe"),
            Self::InvalidRegionSize => f.write_str("region size must be a finite number"),
        }
    }
}
impl std::error::Error for ProtocolValidationError {}
pub fn validate_stable_id(value: &str) -> Result<(), ProtocolValidationError> {
    if value.is_empty() {
        return Err(ProtocolValidationError::EmptyId);
    }
    if value.chars().count() > MAX_ID_LENGTH {
        return Err(ProtocolValidationError::IdTooLong { max: MAX_ID_LENGTH });
    }
    if value.chars().any(char::is_control) {
        return Err(ProtocolValidationError::IdContainsControlCharacter);
    }
    Ok(())
}
pub fn validate_external_url(value: &str) -> Result<(), ProtocolValidationError> {
    // WHATWG URL parsing normalizes `https:///host`; require an explicit
    // authority in the original spelling before accepting that parse.
    if !value.starts_with("https://") || value[8..].starts_with(['/', '\\']) {
        return Err(ProtocolValidationError::InvalidExternalUrl);
    }
    if value.chars().any(char::is_whitespace) || value.chars().any(char::is_control) {
        return Err(ProtocolValidationError::InvalidExternalUrl);
    }
    let url = Url::parse(value).map_err(|_| ProtocolValidationError::InvalidExternalUrl)?;
    if url.scheme() != "https"
        || url.host_str().filter(|host| !host.is_empty()).is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ProtocolValidationError::InvalidExternalUrl);
    }
    Ok(())
}
pub fn validate_workspace_relative_path(value: &str) -> Result<(), ProtocolValidationError> {
    if value.is_empty()
        || value.chars().any(char::is_control)
        || value.starts_with(['/', '\\'])
        || value.starts_with("//")
        || value.starts_with("\\\\")
        || value.starts_with("\\\\?\\")
        || value.starts_with("\\\\.\\")
        || value.as_bytes().get(1).is_some_and(|byte| *byte == b':')
    {
        return Err(ProtocolValidationError::InvalidWorkspacePath);
    }
    if value.split(['/', '\\']).any(|segment| {
        segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
    }) {
        return Err(ProtocolValidationError::InvalidWorkspacePath);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TextPosition {
    pub line: u32,
    pub utf16_column: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TextChange {
    pub start: TextPosition,
    pub end: TextPosition,
    pub replacement: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub anchor: TextPosition,
    pub active: TextPosition,
    pub version: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatRequestDto {
    pub session_id: SessionId,
    pub message: String,
    pub model_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentRequestDto {
    pub session_id: SessionId,
    pub prompt: String,
    pub model_id: Option<String>,
    /// Phase Z3: the agent dock's permission mode. `execute` is the default
    /// (and pre-Z3 wire value) — the full §17.1 manifest with approvals
    /// gating side effects. `explore` arms the read-only tools only (the
    /// model cannot request a write). Plan-mode UX beyond this rides
    /// Phase 9's plan/apply split.
    #[serde(default)]
    pub mode: AgentModeDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Where the user was when the turn was sent. Optional: old clients and
    /// old turn records carry no editor context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_context: Option<EditorContextDto>,
}
/// Where the user is, at the moment the turn was sent. Never the file
/// contents: the agent reads those with `read_file`, so the prompt stays
/// small and the turn sees the same bytes the tools do.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EditorContextDto {
    /// Workspace-relative path of the active buffer.
    pub path: String,
    /// Language id of the active buffer (e.g. "rust", "typescript").
    pub language: String,
    /// Last known selection, if any. Empty selections are omitted by the sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
    /// Whether the buffer has unsaved edits at send time.
    pub dirty: bool,
}
/// The `SendAgent` permission mode (Phase Z3). Serde-rejected spellings
/// never reach a Rust arm — the boundary is the validator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModeDto {
    #[default]
    Execute,
    Explore,
    /// Session full access: the full manifest, and file writes/edits/deletes
    /// apply without an approval card. Bash still cards. Never persisted.
    Auto,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionDto {
    Approve,
    Answer,
    Deny,
    ApproveAllThisTurn,
    /// Phase 9b: request a revised plan. A plan card answers with this; the
    /// session stays in plan mode and the model proposes again.
    Revise,
    Stop,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentDecisionResolutionDto {
    Question {
        option_id: Option<String>,
        text: Option<String>,
    },
    Plan {
        decision: String,
        feedback: Option<String>,
    },
}

/// One option on a structured question (Phase 9a/§19.4).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QuestionPromptOptionDto {
    /// Stable option id; the UI echoes it in `AgentDecisionResolution`.
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The prompt that rides an `approval_needed` card when the model asked a
/// structured question. It mirrors the frontend's `AgentQuestionPrompt`
/// verbatim, so the question card is one route: the normal approval park, the
/// normal `AgentDecision` answer. The plan prompt is carried by
/// `ApprovalNeededDto::plan` and built by the UI, so only questions need this
/// payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionPromptDto {
    Question {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        header: Option<String>,
        question: String,
        #[serde(default)]
        options: Vec<QuestionPromptOptionDto>,
        /// Free text is offered unless the model explicitly disabled it; a
        /// question with no options is always answerable in text.
        #[serde(default = "default_true")]
        allow_free_text: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        free_text_placeholder: Option<String>,
    },
}

fn default_true() -> bool {
    true
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchQueryDto {
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FindOptionsDto {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceId {
    Files,
    Search,
    Git,
    Problems,
    Settings,
    Agent,
    /// Phase 8: the Workflows run panel in the primary sidebar.
    Workflows,
    Output,
    TerminalPlaceholder,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionId {
    PrimarySidebar,
    AgentDock,
    BottomDock,
}

// ---------------------------------------------------------------------------
// Phase Z2 — shell layout geometry (docs/ui/PHASE_Z2_ARCHITECTURE.md §1.1).
//
// These constants are the single source of truth for region bounds and
// defaults: `bootstrap.layout_constraints` carries the bounds to the frontend
// (no bound is ever a JS literal) and `selora-settings`' `layout.*` defaults
// delegate to the same constants (§4.2). tokens.css intentionally carries no
// region values for exactly this reason. Sources: bounds 160/480, 240/600 and
// the 400px editor floor from UI_SPEC §2's region table; defaults 240/360/200
// and the closed first-run agent/bottom states from the same table.
// ---------------------------------------------------------------------------

/// Primary sidebar width bounds (UI_SPEC §2).
pub const PRIMARY_SIDEBAR_MIN_PX: f32 = 160.0;
pub const PRIMARY_SIDEBAR_MAX_PX: f32 = 480.0;
/// Primary sidebar stored default (§1.1's table: 240px, open first-run).
pub const PRIMARY_SIDEBAR_DEFAULT_PX: f32 = 240.0;

/// Agent dock width bounds (UI_SPEC §2).
pub const AGENT_DOCK_MIN_PX: f32 = 240.0;
pub const AGENT_DOCK_MAX_PX: f32 = 600.0;
/// Agent dock stored default (§1.1's table: 360px, closed first-run).
pub const AGENT_DOCK_DEFAULT_PX: f32 = 360.0;

/// Bottom dock height bounds. UI_SPEC §2's "min 0" meant *hidden*, which the
/// protocol models as `bottom_dock_open: false`; an open dock gets a usable
/// 120px floor (Z2 decision, §1.1). Rust enforces the static 800px ceiling —
/// the 50%-of-workbench rule is dynamic and cannot be validated in Rust, so
/// the frontend additionally clamps to it before dispatching (§1.1, §2.3).
pub const BOTTOM_DOCK_MIN_PX: f32 = 120.0;
pub const BOTTOM_DOCK_MAX_PX: f32 = 800.0;
/// Bottom dock stored default (§1.1's table: 200px, closed first-run).
pub const BOTTOM_DOCK_DEFAULT_PX: f32 = 200.0;

/// Bottom-dock ceiling as a fraction of the measured workbench height
/// (frontend-side clamp; Rust does not know the webview's workbench height).
pub const BOTTOM_DOCK_MAX_WORKBENCH_RATIO: f32 = 0.5;

/// The editor region never goes below this width (UI_SPEC §2 region ②'s min).
/// A frontend-enforced derived constraint — no protocol validation.
pub const EDITOR_MIN_WIDTH_PX: f32 = 400.0;

/// Resize handle hit-zone width (the brief §6's ≥6px rule).
pub const RESIZE_HIT_ZONE_MIN_PX: f32 = 6.0;

/// Editor split count bounds (UI_SPEC §3.3).
pub const EDITOR_SPLITS_MIN: u8 = 1;
pub const EDITOR_SPLITS_MAX: u8 = 4;

/// The surfaces the primary sidebar can show (§2.2's routing table).
/// `Problems` is a bottom-dock view in Z2, so it is excluded here
/// (ARCHITECTURE §4.3 — brief §2's sidebar parenthetical is superseded).
pub const ACTIVITY_VIEWS: [SurfaceId; 5] = [
    SurfaceId::Files,
    SurfaceId::Search,
    SurfaceId::Git,
    SurfaceId::Workflows,
    SurfaceId::Settings,
];

/// The surfaces the bottom dock can show (§2.2's routing table).
pub const BOTTOM_VIEWS: [SurfaceId; 3] = [
    SurfaceId::Output,
    SurfaceId::TerminalPlaceholder,
    SurfaceId::Problems,
];

/// Clamp a ResizeRegion value into the region's bounds (§1.1). Finite
/// out-of-range values are clamped, never rejected (drag-end values
/// routinely land a fraction past a bound). The non-finite check is a
/// Rust-internal invariant guard: JSON cannot carry NaN/±Inf and serde_json
/// rejects out-of-range number literals at the parse boundary, so no wire
/// input can reach it — it exists for future non-wire callers, not as a wire
/// threat model.
pub fn clamp_region_size(region: RegionId, size_px: f32) -> Result<f32, ProtocolValidationError> {
    if !size_px.is_finite() {
        return Err(ProtocolValidationError::InvalidRegionSize);
    }
    let (min, max) = match region {
        RegionId::PrimarySidebar => (PRIMARY_SIDEBAR_MIN_PX, PRIMARY_SIDEBAR_MAX_PX),
        RegionId::AgentDock => (AGENT_DOCK_MIN_PX, AGENT_DOCK_MAX_PX),
        RegionId::BottomDock => (BOTTOM_DOCK_MIN_PX, BOTTOM_DOCK_MAX_PX),
    };
    Ok(size_px.clamp(min, max))
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsPatchDto {
    #[serde(default)]
    pub reduced_motion: Option<bool>,
    #[serde(default)]
    pub theme: Option<String>,
    /// Editor preferences (P1 settings slice). Every field is optional and
    /// serde-defaulted, so a pre-P1 patch (`{"theme": "light"}`) still parses.
    #[serde(default)]
    pub editor_font_size: Option<u32>,
    #[serde(default)]
    pub editor_tab_size: Option<u32>,
    #[serde(default)]
    pub editor_word_wrap: Option<bool>,
    /// Rebindable keybinds (the same grammar as `Keybind::parse`). The
    /// desktop validates and canonicalizes each one before persisting.
    #[serde(default)]
    pub keybind_command_palette: Option<String>,
    #[serde(default)]
    pub keybind_fuzzy_find: Option<String>,
    #[serde(default)]
    pub keybind_toggle_chat: Option<String>,
    #[serde(default)]
    pub keybind_save: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum FrontendCommand {
    GetBootstrapState,
    OpenFolder,
    /// Phase Z4: open a workspace at an absolute root path. The folder
    /// picker (pick_folder) supplies the path; tests use this arm
    /// directly so no native dialog runs in the gate.
    OpenFolderAt {
        root: String,
    },
    /// P1: close the open folder. Drops every buffer, stops the language
    /// servers and clears their diagnostics; the shell returns to the
    /// no-workspace state.
    CloseFolder,
    /// Phase Z4: rebuild the file tree from disk and emit
    /// FileTreeChanged (the explorer's refresh affordance).
    RefreshTree,
    /// Phase Z4: select a workspace-relative file in the OS file manager.
    RevealInExplorer {
        path: WorkspacePath,
    },
    OpenFile {
        path: WorkspacePath,
    },
    CloseBuffer {
        buffer_id: BufferId,
    },
    /// P1 active-buffer fix: the web shell's tab switch. Bootstrap then
    /// reports the buffer the user actually selected, not the first one in
    /// path order.
    SetActiveBuffer {
        buffer_id: BufferId,
    },
    SaveBuffer {
        buffer_id: BufferId,
    },
    /// P2 gap 3: a dirty buffer changed on disk. Reload it from disk,
    /// discarding the local edits, and sync the language server.
    ReloadBufferFromDisk {
        buffer_id: BufferId,
    },
    /// P2 gap 3: keep the local edits and clear the external-change marker.
    KeepBufferEdits {
        buffer_id: BufferId,
    },
    ApplyEditorChange {
        buffer_id: BufferId,
        version: u64,
        changes: Vec<TextChange>,
    },
    SetSelection {
        buffer_id: BufferId,
        selection: Selection,
    },
    /// Phase Z4b: resolve hover at a buffer position (the editor's 600 ms
    /// hold). The answer is a `HoverResult` event.
    HoverAt {
        buffer_id: BufferId,
        position: TextPosition,
    },
    /// Phase Z4b: go-to-definition at a buffer position.
    DefinitionAt {
        buffer_id: BufferId,
        position: TextPosition,
    },
    /// Phase Z4b: completions at a buffer position.
    CompletionAt {
        buffer_id: BufferId,
        position: TextPosition,
    },
    /// P1: restart one language server (truthful status follows as
    /// `LspServerStateChanged` events). `language` is a wire language id
    /// (`rust`, `typescript`, `javascript`) or a server name.
    LspRestart {
        language: String,
    },
    SendChat {
        request_id: RequestId,
        request: ChatRequestDto,
    },
    StopChat {
        request_id: RequestId,
    },
    SendAgent {
        request_id: RequestId,
        request: AgentRequestDto,
    },
    AgentDecision {
        request_id: RequestId,
        approval_id: ApprovalId,
        decision: DecisionDto,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolution: Option<AgentDecisionResolutionDto>,
    },
    RetryMessage {
        session_id: SessionId,
        message_id: MessageId,
    },
    SwitchSession {
        session_id: SessionId,
    },
    NewSession,
    /// Phase 9d: refresh the resume picker. The answer is a full bootstrap
    /// whose `agent_sessions` is freshly scanned from disk.
    ListAgentSessions,
    /// Phase 9d: arm/disarm session full access. The flag lives only in the
    /// engine's session memory: it is never written to disk and resets on
    /// switch/new/restart.
    SetAgentAutoApprove {
        session_id: SessionId,
        enabled: bool,
    },
    Search {
        query: SearchQueryDto,
    },
    FindInBuffer {
        buffer_id: BufferId,
        query: String,
        options: FindOptionsDto,
    },
    ReplaceCurrent {
        buffer_id: BufferId,
        replacement: String,
    },
    ReplaceAll {
        buffer_id: BufferId,
        replacement: String,
    },
    GitStage {
        path: WorkspacePath,
    },
    GitUnstage {
        path: WorkspacePath,
    },
    GitCommit {
        message: String,
    },
    OpenGitDiff {
        path: WorkspacePath,
    },
    ToggleSurface {
        surface: SurfaceId,
    },
    ResizeRegion {
        region: RegionId,
        size_px: f32,
    },
    SaveLayout,
    OpenExternal {
        url: String,
    },
    SettingsUpdate {
        patch: SettingsPatchDto,
    },
    // Phase 8 — workflows. `WorkflowRun` is the confirmation card's Approve;
    // `WorkflowResume` replays/unpauses a journaled run; `WorkflowStop` kills
    // the run and its in-flight children; `WorkflowDecision` answers a
    // `WorkflowApprovalNeeded` card (single-use, like `AgentDecision`).
    WorkflowRun {
        request_id: RequestId,
        spec: WorkflowSpecDto,
    },
    WorkflowResume {
        request_id: RequestId,
        run_id: String,
        /// Whether the user has accepted that resume may dispatch an `ask`
        /// again whose tools already applied effects. False by default, so a
        /// pre-flag payload resumes fail-closed.
        #[serde(default)]
        allow_incomplete_asks: bool,
        // No `max_cost_usd` here on purpose: resume replays the confirmed
        // grant and never raises it. A run that exhausted its cap is refused
        // with `workflow_budget_exhausted`; continuing needs a new run and a
        // newly confirmed budget.
        /// Whether the user has accepted durable evidence that tool effects
        /// may already exist for this run (an unfinished `started` record, or
        /// an effect owned by an unfinished ask). This is a separate action
        /// from ordinary Resume; false by default so old payloads refuse.
        #[serde(default)]
        acknowledge_partial_effects: bool,
    },
    WorkflowStop {
        request_id: RequestId,
    },
    WorkflowDecision {
        request_id: RequestId,
        approval_id: ApprovalId,
        decision: DecisionDto,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceDto {
    pub id: WorkspaceId,
    pub core_instance_id: CoreInstanceId,
    pub root: Option<String>,
    pub display_name: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BufferSnapshotDto {
    pub buffer_id: BufferId,
    pub path: WorkspacePath,
    pub language: String,
    pub text: String,
    pub version: u64,
    pub dirty: bool,
    pub selection: Option<Selection>,
}
/// P2 gap 3: an open dirty buffer also changed on disk. The local text is
/// still the buffer's text; the banner offers Reload or Keep.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BufferConflictDto {
    pub buffer_id: BufferId,
    pub path: WorkspacePath,
}
/// P2 gap 3: the conflict for this buffer was resolved (reload or keep).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BufferConflictClearedDto {
    pub buffer_id: BufferId,
    pub path: WorkspacePath,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileTreeDto {
    pub root: Option<FileTreeNodeDto>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileTreeNodeDto {
    pub path: WorkspacePath,
    pub name: String,
    pub kind: FileTreeNodeKind,
    pub children: Vec<FileTreeNodeDto>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTreeNodeKind {
    Directory,
    File,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionDto {
    pub session_id: SessionId,
    pub title: String,
    pub message_ids: Vec<MessageId>,
    pub active: bool,
    /// Phase 9b: the session's plan phase, hydrated into the dock's plan
    /// spine. Defaulted so a pre-9b snapshot still loads (as executing).
    #[serde(default)]
    pub plan_phase: PlanPhaseDto,
    /// Phase 9b: the approved/proposed plan, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanApprovalDto>,
    /// Phase 9b: the session's stored todos.
    #[serde(default)]
    pub todos: Vec<TodoDto>,
}

// ---------------------------------------------------------------------------
// Phase 9b — plan mode and live todos (ROADMAP Phase 9b). Additive to
// protocol_version 1; every field has a serde default so pre-9b payloads
// still load.
// ---------------------------------------------------------------------------

/// The session's plan phase. `executing` is the pre-9b behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanPhaseDto {
    Planning,
    #[default]
    Executing,
}

/// The plan card's payload: optional one-line context and the ordered steps.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlanApprovalDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub steps: Vec<String>,
}

/// One stored todo item.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TodoDto {
    pub id: String,
    #[serde(alias = "text")]
    pub title: String,
    #[serde(default)]
    pub status: TodoStatusDto,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatusDto {
    #[default]
    Pending,
    InProgress,
    Completed,
}

/// `PlanStateChanged` — the session's phase or proposed/approved plan moved.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlanStateDto {
    pub session_id: SessionId,
    #[serde(default)]
    pub phase: PlanPhaseDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanApprovalDto>,
}

/// `TodosChanged` — the session's stored list was replaced.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TodosDto {
    pub session_id: SessionId,
    #[serde(default)]
    pub todos: Vec<TodoDto>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MessageDto {
    pub message_id: MessageId,
    pub session_id: SessionId,
    pub role: MessageRole,
    pub text: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApprovalDto {
    pub approval_id: ApprovalId,
    pub request_id: RequestId,
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub summary: String,
    pub can_approve_all_this_turn: bool,
    /// Phase 9b: the plan payload for a parked `exit_plan_mode` card, so a
    /// reload can rebuild the same plan card. Absent on every other card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanApprovalDto>,
    /// Phase 9a/§19.4: the parked question's prompt, so a reload rebuilds the
    /// same question card. Absent on every non-question card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_prompt: Option<DecisionPromptDto>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelDto {
    pub id: String,
    pub display_name: String,
    pub available: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatStateDto {
    pub sessions: Vec<SessionDto>,
    pub messages: Vec<MessageDto>,
    pub active_session_id: Option<SessionId>,
    pub active_request_id: Option<RequestId>,
    pub active_turn_id: Option<TurnId>,
    pub pending_approval: Option<ApprovalDto>,
    /// Every session's park; the singular field remains the selected session projection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_approvals: Vec<ApprovalDto>,
    /// Phase 9 session slice: the active session's authoritative todo list.
    /// Additive and serde-defaulted, so a pre-Phase-9 bootstrap loads with no
    /// todos rather than failing. Per-session copies ride [`SessionDto`].
    #[serde(default)]
    pub todos: Vec<TodoDto>,
}

// ---------------------------------------------------------------------------
// Phase 9d — durable agent sessions (ARCHITECTURE §19.1).
//
// The resume picker's data comes from `.selora/agent-sessions/{id}/`. It is a
// read-only projection: a switch/resume restores the transcript, todos, plan,
// model, mode and parked cards; the session auto-approve flag has no wire
// field because it must never survive a process or a switch.
// ---------------------------------------------------------------------------

/// One durable session in the resume picker (newest first).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSessionSummaryDto {
    pub session_id: SessionId,
    pub title: String,
    /// Wall-clock milliseconds since the Unix epoch (the store's unit).
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `execute` / `explore` / `plan`; `auto` is never persisted.
    #[serde(default)]
    pub mode: String,
    /// The restored park, if any. It is shown once and never auto-executes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<AgentParkSummaryDto>,
    /// True once a compaction cursor was recorded (full transcript preserved).
    #[serde(default)]
    pub compacted: bool,
    #[serde(default)]
    pub message_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentParkSummaryDto {
    /// `approval` or `question`.
    pub kind: String,
    pub summary: String,
}

/// The typed compaction failure. `partial_summary` is the text streamed before
/// the summarize round failed — surfaced, never silently dropped.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentCompactionFailedDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_summary: Option<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Connected,
    Connecting,
    Disconnected,
}
/// Mirrors `Keybind::parse`'s default grammar spelling for the rebindable
/// pair (the same values as selora-settings' `default_command_palette` /
/// `default_toggle_chat`, settings.rs). String constants, not settings
/// types — the protocol crate stays settings-free.
fn default_keybind_command_palette() -> String {
    "Ctrl+Shift+P".into()
}
fn default_keybind_toggle_chat() -> String {
    "Ctrl+L".into()
}
fn default_keybind_fuzzy_find() -> String {
    "Ctrl+P".into()
}
fn default_keybind_save() -> String {
    "Ctrl+S".into()
}
fn default_editor_font_size() -> u32 {
    14
}
fn default_editor_tab_size() -> u32 {
    4
}
fn default_editor_word_wrap() -> bool {
    false
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsViewDto {
    pub reduced_motion: bool,
    pub theme: String,
    /// §7.1's rebindable pair, populated from `settings.keybinds` (§5.1) so a
    /// user's egui-side rebind carries into the web shell.
    #[serde(default = "default_keybind_command_palette")]
    pub keybind_command_palette: String,
    #[serde(default = "default_keybind_toggle_chat")]
    pub keybind_toggle_chat: String,
    /// The other two settings keybinds (`fuzzy_find`, `save`), now resolved by
    /// the web shell's command keymap.
    #[serde(default = "default_keybind_fuzzy_find")]
    pub keybind_fuzzy_find: String,
    #[serde(default = "default_keybind_save")]
    pub keybind_save: String,
    /// Editor preferences applied to CodeMirror at runtime.
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: u32,
    #[serde(default = "default_editor_tab_size")]
    pub editor_tab_size: u32,
    #[serde(default = "default_editor_word_wrap")]
    pub editor_word_wrap: bool,
}
/// Phase Z2 serde defaults (§5.1 / §4.2): every field is defaulted so a
/// partial or pre-Z2 `LayoutDto` JSON deserializes — the values are §1.1's
/// table. These functions are also `sanitized()`'s fallbacks, so the wire
/// defaults and the sanitization floors cannot diverge.
fn default_primary_sidebar_width_px() -> f32 {
    PRIMARY_SIDEBAR_DEFAULT_PX
}
fn default_agent_dock_width_px() -> f32 {
    AGENT_DOCK_DEFAULT_PX
}
fn default_agent_dock_open() -> bool {
    false
}
fn default_bottom_dock_height_px() -> f32 {
    BOTTOM_DOCK_DEFAULT_PX
}
fn default_bottom_dock_open() -> bool {
    false
}
fn default_bottom_view() -> SurfaceId {
    SurfaceId::Output
}
fn default_editor_splits() -> u8 {
    EDITOR_SPLITS_MIN
}
fn default_activity_view() -> SurfaceId {
    SurfaceId::Files
}
fn default_primary_sidebar_open() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutDto {
    #[serde(default = "default_primary_sidebar_width_px")]
    pub primary_sidebar_width_px: f32,
    #[serde(default = "default_agent_dock_width_px")]
    pub agent_dock_width_px: f32,
    #[serde(default = "default_agent_dock_open")]
    pub agent_dock_open: bool,
    #[serde(default = "default_bottom_dock_height_px")]
    pub bottom_dock_height_px: f32,
    #[serde(default = "default_bottom_dock_open")]
    pub bottom_dock_open: bool,
    #[serde(default = "default_bottom_view")]
    pub bottom_view: SurfaceId,
    #[serde(default = "default_editor_splits")]
    pub editor_splits: u8,
    /// The activity bar's active view (§5.1 — brief §6's "active activity
    /// view"). `bottom_view` above covers the bottom dock's active view.
    #[serde(default = "default_activity_view")]
    pub activity_view: SurfaceId,
    /// Rust-owned sidebar open state (§5.1 — a collapsed sidebar must
    /// restore; `Ctrl+B` toggling needs it).
    #[serde(default = "default_primary_sidebar_open")]
    pub primary_sidebar_open: bool,
}

impl LayoutDto {
    /// §4.3 load-time sanitization: clamp all sizes into the §1.1 bounds
    /// (non-finite persisted values fall to the defaults), clamp
    /// `editor_splits` into `EDITOR_SPLITS_MIN..=EDITOR_SPLITS_MAX`, and
    /// reset `activity_view` / `bottom_view` if they name a surface outside
    /// the valid sets — a persisted value can never wedge the shell. Runs on
    /// settings load and after every accepted resize.
    pub fn sanitized(self) -> Self {
        Self {
            primary_sidebar_width_px: sanitize_region_size(
                RegionId::PrimarySidebar,
                self.primary_sidebar_width_px,
                default_primary_sidebar_width_px(),
            ),
            agent_dock_width_px: sanitize_region_size(
                RegionId::AgentDock,
                self.agent_dock_width_px,
                default_agent_dock_width_px(),
            ),
            bottom_dock_height_px: sanitize_region_size(
                RegionId::BottomDock,
                self.bottom_dock_height_px,
                default_bottom_dock_height_px(),
            ),
            agent_dock_open: self.agent_dock_open,
            bottom_dock_open: self.bottom_dock_open,
            bottom_view: if BOTTOM_VIEWS.contains(&self.bottom_view) {
                self.bottom_view
            } else {
                default_bottom_view()
            },
            editor_splits: self
                .editor_splits
                .clamp(EDITOR_SPLITS_MIN, EDITOR_SPLITS_MAX),
            activity_view: if ACTIVITY_VIEWS.contains(&self.activity_view) {
                self.activity_view
            } else {
                default_activity_view()
            },
            primary_sidebar_open: self.primary_sidebar_open,
        }
    }
}

/// Clamp a persisted size, falling to the default for non-finite values
/// (§4.3: "non-finite persisted values to the defaults" — an error, as
/// `clamp_region_size` treats it, would have nowhere to go here).
fn sanitize_region_size(region: RegionId, value: f32, fallback: f32) -> f32 {
    clamp_region_size(region, value).unwrap_or(fallback)
}

/// Field-for-field mirror of the §1.1 constants, delivered in
/// `bootstrap.layout_constraints` so no bound is ever a JS literal (§5.1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutConstraintsDto {
    pub primary_sidebar_min_px: f32,
    pub primary_sidebar_max_px: f32,
    pub agent_dock_min_px: f32,
    pub agent_dock_max_px: f32,
    pub bottom_dock_min_px: f32,
    pub bottom_dock_max_px: f32,
    pub bottom_dock_max_workbench_ratio: f32,
    pub editor_min_width_px: f32,
    pub resize_hit_zone_min_px: f32,
}

impl LayoutConstraintsDto {
    /// The only constructor — field-for-field from the §1.1 constants, so the
    /// DTO and the constants cannot drift (§10's protocol test asserts
    /// exactly this).
    pub fn from_constants() -> Self {
        Self {
            primary_sidebar_min_px: PRIMARY_SIDEBAR_MIN_PX,
            primary_sidebar_max_px: PRIMARY_SIDEBAR_MAX_PX,
            agent_dock_min_px: AGENT_DOCK_MIN_PX,
            agent_dock_max_px: AGENT_DOCK_MAX_PX,
            bottom_dock_min_px: BOTTOM_DOCK_MIN_PX,
            bottom_dock_max_px: BOTTOM_DOCK_MAX_PX,
            bottom_dock_max_workbench_ratio: BOTTOM_DOCK_MAX_WORKBENCH_RATIO,
            editor_min_width_px: EDITOR_MIN_WIDTH_PX,
            resize_hit_zone_min_px: RESIZE_HIT_ZONE_MIN_PX,
        }
    }
}
/// P2: one changed file's two *independent* git slots.
///
/// `changed_paths` / `staged_paths` below are the legacy flattened view and
/// cannot express a file that has a staged edit and *further* unstaged
/// edits on top. `staged` / `worktree` keep the index and working-tree
/// sides separate so the panel can show such a file in both of its
/// sections (and offer the right stage/unstage action on each).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitFileDto {
    pub path: WorkspacePath,
    /// Index side — `FileStatus.staged != Slot::Unmodified`.
    pub staged: bool,
    /// Working-tree side — `FileStatus::has_worktree_changes()`; an
    /// untracked file is worktree-only (`staged: false`, `worktree: true`).
    pub worktree: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitStatusDto {
    pub branch: Option<String>,
    /// Legacy flattened view: every changed file, staged or not.
    pub changed_paths: Vec<WorkspacePath>,
    /// Legacy flattened view: files with an index (staged) change.
    pub staged_paths: Vec<WorkspacePath>,
    /// P2: the precise per-slot source. Additive and defaulted so old
    /// payloads (which omit the key) still deserialize; when empty, readers
    /// fall back to `changed_paths` / `staged_paths`.
    #[serde(default)]
    pub files: Vec<GitFileDto>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticDto {
    pub path: WorkspacePath,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
}
/// Phase Z4b — an LSP range in wire coordinates (0-based line, UTF-16
/// column).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TextRangeDto {
    pub start: TextPosition,
    pub end: TextPosition,
}
/// The lifecycle of one language server as the bridge observes it.
///
/// `failed` carries the truthful reason in [`LspServerStateDto::error`] (a
/// spawn failure, a dead process, or an unresolvable command). There is no
/// "unknown" state: a server that has not started yet is absent from the
/// bootstrap list rather than reported healthy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LspServerHealth {
    Starting,
    Ready,
    Failed,
}
/// One language server's live state (bootstrap and `LspServerStateChanged`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LspServerStateDto {
    /// Wire language id (`rust`, `typescript`, `javascript`).
    pub language: String,
    /// Display/server name (`rust-analyzer`, `typescript-language-server`).
    pub server: String,
    pub state: LspServerHealth,
    /// Why the server failed; `None` for starting/ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
/// The answer to `HoverAt`; `contents: None` means the server had nothing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HoverResultDto {
    pub buffer_id: BufferId,
    pub contents: Option<String>,
    pub range: Option<TextRangeDto>,
}
/// One completion candidate, flattened for the editor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompletionItemDto {
    pub label: String,
    pub detail: Option<String>,
    pub insert_text: Option<String>,
    pub kind: Option<String>,
}
/// The answer to `CompletionAt`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompletionResultDto {
    pub buffer_id: BufferId,
    pub items: Vec<CompletionItemDto>,
}
/// One definition target (possibly another file).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DefinitionLocationDto {
    pub path: WorkspacePath,
    pub line: u32,
    pub utf16_column: u32,
}
/// The answer to `DefinitionAt`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DefinitionResultDto {
    pub buffer_id: BufferId,
    pub locations: Vec<DefinitionLocationDto>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FeatureFlagsDto {
    pub agent: bool,
    pub git: bool,
    pub search: bool,
    pub editor: bool,
    pub later_slices: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BootstrapState {
    pub protocol_version: u32,
    pub core_instance_id: CoreInstanceId,
    pub snapshot_sequence: u64,
    pub workspace: WorkspaceDto,
    pub buffers: Vec<BufferSnapshotDto>,
    pub active_buffer_id: Option<BufferId>,
    pub file_tree: FileTreeDto,
    pub models: Vec<ModelDto>,
    pub active_model_id: Option<String>,
    pub connection: ConnectionStatus,
    pub chat: ChatStateDto,
    pub settings: SettingsViewDto,
    pub layout: LayoutDto,
    /// §5.1: the §1.1 bounds as delivered to the frontend — no bound is ever
    /// a JS literal. No serde default: the bootstrap is always generated
    /// whole.
    pub layout_constraints: LayoutConstraintsDto,
    pub git: GitStatusDto,
    pub diagnostics: Vec<DiagnosticDto>,
    /// P1: live language-server state from the bridge store. Additive and
    /// defaulted so pre-P1 payloads still deserialize.
    #[serde(default)]
    pub lsp_servers: Vec<LspServerStateDto>,
    pub feature_flags: FeatureFlagsDto,
    /// Phase 8: unmounted durable runs found in the open workspace. Live runs
    /// arrive as `workflow_*` events instead.
    #[serde(default)]
    pub workflows: Vec<WorkflowPersistedRunDto>,
    /// Phase 9d: durable agent sessions found in the open workspace (the
    /// resume picker). Read-only projection — switching loads the session.
    #[serde(default)]
    pub agent_sessions: Vec<AgentSessionSummaryDto>,
}

/// The reply shape of `dispatch_command` (P1 state slice).
///
/// Additive strategy: the union is untagged, so every reply is still one JSON
/// object. `GetBootstrapState` (and the two folder-open commands) keep the
/// exact pre-existing `BootstrapState` shape, so a protocol client that reads
/// those replies is unaffected. Commands that previously answered
/// `unsupported_phase_slice`, plus event-backed mutations, may answer the
/// small [`CommandAckDto`] instead: a client that only used the event stream
/// (the shipped frontend) sees no difference, and a client that reads the
/// reply can distinguish the two by `acknowledged`. `protocol_version` stays
/// 1 because the variants are additive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandResultDto {
    Snapshot(Box<BootstrapState>),
    Ack(CommandAckDto),
}

/// A mutation acknowledgement: the core identity, the log position after the
/// mutation, and the session the engine now considers active (if any). It is
/// intentionally much smaller than a `BootstrapState` (no tree walk, no
/// libgit2 status, no transcript).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandAckDto {
    pub protocol_version: u32,
    pub core_instance_id: CoreInstanceId,
    pub snapshot_sequence: u64,
    /// Always true: the command was accepted and the mutation applied (or was
    /// an idempotent no-op). Failures come back as `ProtocolErrorDto`.
    pub acknowledged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
}

/// One durable run discovered under `.selora/workflow-runs/` at bootstrap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowPersistedRunDto {
    pub run_id: String,
    pub name: String,
    pub status: WorkflowStatus,
    /// False when the journal proved the run ambiguous (an interrupted command)
    /// and replay would be unsafe, when the tool-effect ledger proves a partial
    /// effect, or when the run already completed.
    pub resumable: bool,
    /// True when durable ledger evidence says a side effect may already have
    /// applied. Resume stays unavailable; a separate explicit
    /// `acknowledge_partial_effects` action is required.
    #[serde(default)]
    pub partial_effects: bool,
    /// Why resume is unavailable, or the terminal journal's error text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Phase names from the run's `phase_started` records, in first-marker
    /// order. Empty when the journal does not carry them.
    #[serde(default)]
    pub phases: Vec<String>,
    /// Steps reconstructed from `engine_step_started`/`engine_step_finished`
    /// records, with the phase recorded for each step.
    #[serde(default)]
    pub steps: Vec<WorkflowStepDto>,
    /// Report items reconstructed from `engine_report_item` records, keyed by
    /// their run-wide ordinal. A journal that predates those records falls
    /// back to the successful report steps' inputs.
    #[serde(default)]
    pub reports: Vec<WorkflowReportDto>,
    /// Artifacts reconstructed from `engine_artifact` records (or the terminal
    /// report on an older journal), with their full content.
    #[serde(default)]
    pub artifacts: Vec<WorkflowArtifactDto>,
    /// Receipt tokens: the terminal report's total, or the step records' sum.
    #[serde(default = "default_tokens")]
    pub tokens: u64,
    /// Spend: the terminal report's total, or the step records' sum.
    #[serde(default = "default_cost_usd")]
    pub cost_usd: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatDeltaDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub message_id: MessageId,
    /// The ordinal of this delta within the streaming message (1-based,
    /// reset per `message_id`) — an in-request ordering check under the
    /// envelope's global monotonic `sequence`. Phase Z3 pins the
    /// semantics: before Z3 no producer existed.
    pub sequence: u64,
    pub text: String,
}
/// Phase Z3 serde defaults: a pre-Z3 `ChatCompletedDto` JSON (the four
/// id-and-text fields) deserializes with zeroed metering — the cost footer
/// reads 0 tokens / $0 rather than failing to load.
fn default_tokens() -> u64 {
    0
}
fn default_cost_usd() -> f64 {
    0.0
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletedDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub message_id: MessageId,
    pub text: String,
    /// Phase Z3: the completion's metering (the receipt's token total and
    /// cost — gateway-reported when present, else the chars/4 estimate the
    /// client applies) so the frontend's cost footer needs no second event.
    #[serde(default = "default_tokens")]
    pub total_tokens: u64,
    #[serde(default = "default_cost_usd")]
    pub cost_usd: f64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatFailedDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub error: ProtocolErrorDto,
}
// ---------------------------------------------------------------------------
// Phase Z3 — the agent dock's event payloads (docs/ui/PHASE_Z_MIGRATION.md).
//
// Every DTO carries the routing triple (request_id, session_id, turn_id) the
// same way `ChatDeltaDto` does, so the frontend's stale-request dropping is
// one rule across chat and agent events. `call_id` is the model's tool-call
// id (the `ToolCall.id` string), not a protocol stable-id type: the model
// generates it, Rust only relays it.
// ---------------------------------------------------------------------------

/// The loop started one tool call — the tool card header.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolCallStartedDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub call_id: String,
    pub tool_name: String,
    /// The call's `arguments` JSON, verbatim as the model produced it.
    pub args_json: String,
}
/// One tool finished (executed, denied, or errored) — the tool card body.
/// `result_summary` carries the tool's full output (reads ≤4MB, bash/fetch
/// capped at 256KB by the router); "summary" is the Z3 brief's wire name.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolResultDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub call_id: String,
    pub result_summary: String,
    pub is_err: bool,
}
/// The loop parked on an approval card (§17.4) — the approval card.
/// `approval_id` is single-use: Rust validates and consumes it before the
/// decision reaches the turn. `PartialEq` only (no `Eq`): the Phase 8
/// `workflow` payload carries an f64 cost cap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalNeededDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub approval_id: ApprovalId,
    pub tool_name: String,
    /// The card's target: a workspace-relative path for write/edit/delete,
    /// the full command line for bash.
    pub path_or_command: String,
    /// The unified diff for write/edit cards (`None` for delete/bash — a
    /// bash card has no diff and a delete needs none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_or_input: Option<String>,
    /// Phase 8 (§18.1): the `run_workflow` confirmation payload — the script
    /// body, the confirmed command set and the run's cost cap. Additive and
    /// defaulted, so a pre-8 `approval_needed` JSON still loads (with the
    /// field absent) and every non-workflow card omits the key entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<WorkflowApprovalDto>,
    /// Phase 9b: the `exit_plan_mode` plan card — steps plus optional
    /// summary. Additive/defaulted like `workflow`; absent elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanApprovalDto>,
    /// Phase 9a/§19.4: the structured question prompt. Additive and
    /// defaulted like `workflow`; absent on every non-question card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<DecisionPromptDto>,
    /// §19.3: the background child that raised this card, when it is a
    /// subagent's write/bash/question rather than the parent turn's own call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_id: Option<String>,
}
/// The agent status line's phase (drives the dock's ThinkingOrb state).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    /// Streaming the model's reply.
    Thinking,
    /// Executing one tool (`tool_name` names it).
    RunningTool,
    /// Parked on an approval card.
    AwaitingApproval,
    /// The turn ended.
    Done,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentStateDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub state: AgentPhase,
    /// The running tool's name — present only in `running_tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}
/// The turn's summary footer: token/cost totals, tool counts, and how it
/// ended (`completed: false` follows a partial answer — cap, failure, or
/// the Stop button; `end_message` is the toast-ready reason).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentTurnDoneDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub num_tools: u32,
    pub approved: u32,
    pub denied: u32,
    pub completed: bool,
    pub end_message: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentEventDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub approval: Option<ApprovalDto>,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Phase 9e - background subagent completion (ARCHITECTURE 19.3).
//
// One event per accepted terminal result, keyed by `child_id` for frontend
// dedupe. `parent_session_id` is explicit so a toast/badge can select the
// right session after the spawning turn has ended. A cancelled child emits
// no completion event at all; a failed child is `ok: false` with `error`.
// ---------------------------------------------------------------------------

/// The child's structural tool policy (19.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentPolicyDto {
    ReadOnly,
    Full,
}

/// One artifact reference a child produced.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SubagentArtifactDto {
    pub kind: String,
    /// Workspace-relative path, or an engine-owned virtual artifact id.
    pub path: String,
}

/// A background subagent's one terminal result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentCompletedDto {
    pub child_id: String,
    #[serde(alias = "parent_session")]
    pub parent_session_id: SessionId,
    pub policy: SubagentPolicyDto,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub final_message: String,
    #[serde(default, alias = "artifact_refs")]
    pub artifacts: Vec<SubagentArtifactDto>,
}

// ---------------------------------------------------------------------------
// Phase 8 — workflow wire contract (ROADMAP Phase 8, ARCHITECTURE §18).
//
// Additive to protocol_version 1: the four commands and seven progress events
// are new arms on the existing tagged enums, so a pre-8 payload keeps loading.
// The engine owns orchestration; these shapes are the whole surface the shell
// reducer and the Workflows panel consume. Every run/phase/step/report/
// artifact id is an engine-minted opaque string (script-generated names are
// common), not a stable-id type.
// ---------------------------------------------------------------------------

/// The run's lifecycle — also the per-phase and per-step state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    #[default]
    Pending,
    Running,
    AwaitingApproval,
    Completed,
    Failed,
    Stopped,
}

/// One phase node in the run graph (the facade's `phase()` marker).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowPhaseDto {
    pub name: String,
    #[serde(default)]
    pub status: WorkflowStatus,
}

/// One unit of work inside a phase.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStepDto {
    pub id: String,
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub status: WorkflowStatus,
    /// The named agent (`agent()` facade) that ran the step, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// A short human-readable progress/diagnostic line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One entry in the run's report list; `artifact_id` links it to an artifact
/// when the report carries more than its item line.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowReportDto {
    pub id: String,
    #[serde(default)]
    pub item: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    /// The phase marker in effect when the item was reported, when known.
    /// Additive: old payloads omit it and loaders may leave it absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

/// One artifact produced by a run (`kind` is the producer's spelling, e.g.
/// `markdown` or `file`; the panel decides how to render it).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowArtifactDto {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub body: String,
    /// The phase marker in effect when the artifact was produced, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

/// The ARCHITECTURE §18.1 workflow-level cost cap default ($5).
fn default_workflow_max_cost_usd() -> f64 {
    5.0
}

/// A full run snapshot — the panel's starting state and resume answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunDto {
    pub run_id: String,
    pub name: String,
    #[serde(default)]
    pub status: WorkflowStatus,
    #[serde(default)]
    pub phases: Vec<WorkflowPhaseDto>,
    #[serde(default)]
    pub steps: Vec<WorkflowStepDto>,
    #[serde(default)]
    pub reports: Vec<WorkflowReportDto>,
    #[serde(default)]
    pub artifacts: Vec<WorkflowArtifactDto>,
    #[serde(default = "default_tokens")]
    pub tokens: u64,
    #[serde(default = "default_cost_usd")]
    pub cost_usd: f64,
    #[serde(default = "default_workflow_max_cost_usd")]
    pub max_cost_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The confirmation card's payload: the script body, the extracted command set
/// and the run's cost cap. `WorkflowRun` starts nothing until this is approved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSpecDto {
    pub name: String,
    pub script: String,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default = "default_workflow_max_cost_usd")]
    pub max_cost_usd: f64,
}

/// The `run_workflow` confirmation payload as it rides the agent's
/// `approval_needed` event (`ApprovalNeededDto::workflow`). Same four facts as
/// the `WorkflowSpecDto` the panel's Approve sends, but carried on the tool
/// card so the chat renders the grant where the model requested it. Every
/// field is defaulted: a partial payload still renders (an absent cap falls to
/// the §18.1 default rather than inventing zero).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowApprovalDto {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub script: String,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default = "default_workflow_max_cost_usd")]
    pub max_cost_usd: f64,
}

/// `WorkflowStarted` — the run was accepted; the panel can open its graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStartedDto {
    pub run_id: String,
    pub request_id: RequestId,
    /// Captured run origin, not the workspace selected when the event arrives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<WorkspaceId>,
    pub name: String,
    #[serde(default = "default_workflow_max_cost_usd")]
    pub max_cost_usd: f64,
}

/// `WorkflowPhase` — a graph node appeared or changed state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowPhaseEventDto {
    pub run_id: String,
    pub phase: WorkflowPhaseDto,
}

/// `WorkflowStep` — a live step's status/label/detail changed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStepEventDto {
    pub run_id: String,
    pub step: WorkflowStepDto,
}

/// `WorkflowReport` — one confirmed finding landed in the run log.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowReportEventDto {
    pub run_id: String,
    pub report: WorkflowReportDto,
}

/// `WorkflowArtifact` — a markdown/file artifact became available.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowArtifactEventDto {
    pub run_id: String,
    pub artifact: WorkflowArtifactDto,
}

/// `WorkflowApprovalNeeded` — a subagent's write/bash parked on the same card
/// shape as chat (§17.4 semantics unchanged). `WorkflowDecision` answers it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowApprovalNeededDto {
    pub run_id: String,
    pub request_id: RequestId,
    pub approval_id: ApprovalId,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub path_or_command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_or_input: Option<String>,
    #[serde(default)]
    pub can_approve_all_this_turn: bool,
}

/// `WorkflowFinished` — the terminal summary (completed, failed or stopped).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowFinishedDto {
    pub run_id: String,
    pub request_id: RequestId,
    #[serde(default)]
    pub status: WorkflowStatus,
    #[serde(default = "default_tokens")]
    pub tokens: u64,
    #[serde(default = "default_cost_usd")]
    pub cost_usd: f64,
    #[serde(default = "default_workflow_max_cost_usd")]
    pub max_cost_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BufferChangedDto {
    pub buffer_id: BufferId,
    pub version: u64,
    pub changes: Vec<TextChange>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchHitDto {
    pub path: WorkspacePath,
    pub line: u32,
    pub column: u32,
    pub preview: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchResultsDto {
    pub request_id: Option<RequestId>,
    pub query: SearchQueryDto,
    pub hits: Vec<SearchHitDto>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitDiffDto {
    pub path: WorkspacePath,
    pub unified_diff: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelsDto {
    pub models: Vec<ModelDto>,
    pub active_model_id: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SeloraStatusDto {
    pub status: ConnectionStatus,
    pub active_model_id: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToastDto {
    pub severity: ToastSeverity,
    pub message: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToastSeverity {
    Info,
    Success,
    Warning,
    Error,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProtocolErrorDto {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectResultDto {
    pub connected: bool,
    pub models_count: usize,
    pub active_model_id: Option<String>,
    pub base_url: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiKeyStatusDto {
    pub connected: bool,
    pub has_key: bool,
    pub source: String,
    pub base_url: String,
    pub key_hint: Option<String>,
    pub active_model_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum FrontendEvent {
    BootstrapState(Box<BootstrapState>),
    ChatDelta(ChatDeltaDto),
    ChatCompleted(ChatCompletedDto),
    ChatFailed(ChatFailedDto),
    AgentEvent(AgentEventDto),
    // Phase Z3 — the agent dock's stream. `AgentEvent` above is the Z1-era
    // generic carrier; it never had a producer and is superseded by the
    // typed payloads below (kept additively — protocol_version stays 1).
    ToolCallStarted(ToolCallStartedDto),
    ToolResult(ToolResultDto),
    ApprovalNeeded(ApprovalNeededDto),
    AgentStateChanged(AgentStateDto),
    AgentTurnDone(AgentTurnDoneDto),
    /// Phase 9d: the summarize round failed. The turn ends with a typed
    /// `CompactionFailed` end too; this event carries the partial summary.
    AgentCompactionFailed(AgentCompactionFailedDto),
    /// Phase 9e: one background child reached a terminal result. Keyed by
    /// `child_id`; never emitted for a cancelled child.
    SubagentCompleted(SubagentCompletedDto),
    DiagnosticsChanged(Vec<DiagnosticDto>),
    /// P1: one language server changed lifecycle (starting/ready/failed).
    /// `failed` carries a truthful, typed reason for the UI.
    LspServerStateChanged(LspServerStateDto),
    // Phase Z4b — the LSP request answers (each echoes the buffer so the
    // editor can drop a result for a tab that closed mid-flight).
    HoverResult(HoverResultDto),
    CompletionResult(CompletionResultDto),
    DefinitionResult(DefinitionResultDto),
    FileTreeChanged(FileTreeDto),
    BufferSnapshot(BufferSnapshotDto),
    BufferChanged(BufferChangedDto),
    /// P2 gap 3: a dirty tab's file changed on disk; local text is untouched.
    BufferConflict(BufferConflictDto),
    /// P2 gap 3: the tab's external-change marker is cleared.
    BufferConflictCleared(BufferConflictClearedDto),
    SearchResults(SearchResultsDto),
    GitStatusChanged(GitStatusDto),
    GitDiffReady(GitDiffDto),
    ModelsChanged(ModelsDto),
    SeloraStatusChanged(SeloraStatusDto),
    WorkspaceLayoutChanged(LayoutDto),
    Toast(ToastDto),
    // Phase 8 — the workflow engine's progress stream (§18.4). Each payload
    // carries `run_id`; the three command-scoped ones also echo `request_id`.
    WorkflowStarted(WorkflowStartedDto),
    WorkflowPhase(WorkflowPhaseEventDto),
    WorkflowStep(WorkflowStepEventDto),
    WorkflowReport(WorkflowReportEventDto),
    WorkflowArtifact(WorkflowArtifactEventDto),
    WorkflowApprovalNeeded(WorkflowApprovalNeededDto),
    WorkflowFinished(WorkflowFinishedDto),
    // Phase 9b — plan mode and live todos. Additive; a pre-9b consumer
    // ignores the arms it does not know.
    PlanStateChanged(PlanStateDto),
    TodosChanged(TodosDto),
    Error(ProtocolErrorDto),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope<T> {
    pub protocol_version: u32,
    pub workspace_id: Option<WorkspaceId>,
    pub core_instance_id: Option<CoreInstanceId>,
    pub request_id: RequestId,
    pub session_id: Option<SessionId>,
    pub payload: T,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    pub protocol_version: u32,
    pub workspace_id: Option<WorkspaceId>,
    pub core_instance_id: CoreInstanceId,
    pub sequence: u64,
    pub request_id: Option<RequestId>,
    pub session_id: Option<SessionId>,
    pub payload: T,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventValidationError {
    ProtocolVersionMismatch {
        expected: u32,
        received: u32,
    },
    CoreInstanceMismatch {
        expected: CoreInstanceId,
        received: CoreInstanceId,
    },
    SequenceGap {
        expected: u64,
        received: u64,
    },
    StaleSequence {
        last_seen: u64,
        received: u64,
    },
}
impl fmt::Display for EventValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProtocolVersionMismatch { expected, received } => write!(
                f,
                "protocol version mismatch: expected {expected}, received {received}"
            ),
            Self::CoreInstanceMismatch { expected, received } => write!(
                f,
                "core instance mismatch: expected {}, received {}",
                expected.as_str(),
                received.as_str()
            ),
            Self::SequenceGap { expected, received } => write!(
                f,
                "event sequence gap: expected {expected}, received {received}"
            ),
            Self::StaleSequence {
                last_seen,
                received,
            } => write!(
                f,
                "stale event sequence: last seen {last_seen}, received {received}"
            ),
        }
    }
}
impl std::error::Error for EventValidationError {}
pub fn validate_event_order<T>(
    event: &EventEnvelope<T>,
    expected_version: u32,
    expected_core: &CoreInstanceId,
    last_seen: Option<u64>,
) -> Result<(), EventValidationError> {
    if event.protocol_version != expected_version {
        return Err(EventValidationError::ProtocolVersionMismatch {
            expected: expected_version,
            received: event.protocol_version,
        });
    }
    if &event.core_instance_id != expected_core {
        return Err(EventValidationError::CoreInstanceMismatch {
            expected: expected_core.clone(),
            received: event.core_instance_id.clone(),
        });
    }
    if let Some(last) = last_seen {
        if event.sequence <= last {
            return Err(EventValidationError::StaleSequence {
                last_seen: last,
                received: event.sequence,
            });
        }
        if event.sequence != last.saturating_add(1) {
            return Err(EventValidationError::SequenceGap {
                expected: last.saturating_add(1),
                received: event.sequence,
            });
        }
    }
    Ok(())
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalValidationError {
    NoPendingApproval,
    RequestMismatch,
    SessionMismatch,
    TurnMismatch,
    ApprovalMismatch,
}
pub fn validate_and_consume_approval(
    pending: &mut Option<ApprovalDto>,
    request_id: &RequestId,
    session_id: &SessionId,
    turn_id: &TurnId,
    approval_id: &ApprovalId,
    decision: DecisionDto,
) -> Result<DecisionDto, ApprovalValidationError> {
    let approval = pending
        .take()
        .ok_or(ApprovalValidationError::NoPendingApproval)?;
    if &approval.request_id != request_id {
        *pending = Some(approval);
        return Err(ApprovalValidationError::RequestMismatch);
    }
    if &approval.session_id != session_id {
        *pending = Some(approval);
        return Err(ApprovalValidationError::SessionMismatch);
    }
    if &approval.turn_id != turn_id {
        *pending = Some(approval);
        return Err(ApprovalValidationError::TurnMismatch);
    }
    if &approval.approval_id != approval_id {
        *pending = Some(approval);
        return Err(ApprovalValidationError::ApprovalMismatch);
    }
    Ok(decision)
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BufferVersions(pub BTreeMap<BufferId, u64>);

#[cfg(test)]
mod tests {
    use super::*;
    fn wid(v: &str) -> WorkspaceId {
        WorkspaceId::new(v).unwrap()
    }
    fn cid(v: &str) -> CoreInstanceId {
        CoreInstanceId::new(v).unwrap()
    }
    fn sid(v: &str) -> SessionId {
        SessionId::new(v).unwrap()
    }
    fn bid(v: &str) -> BufferId {
        BufferId::new(v).unwrap()
    }
    fn rid(v: &str) -> RequestId {
        RequestId::new(v).unwrap()
    }
    fn tid(v: &str) -> TurnId {
        TurnId::new(v).unwrap()
    }
    fn aid(v: &str) -> ApprovalId {
        ApprovalId::new(v).unwrap()
    }
    fn sample() -> BootstrapState {
        BootstrapState {
            protocol_version: PROTOCOL_VERSION,
            core_instance_id: cid("core-1"),
            snapshot_sequence: 8,
            workspace: WorkspaceDto {
                id: wid("workspace-1"),
                core_instance_id: cid("core-1"),
                root: Some("C:/workspace".into()),
                display_name: "workspace".into(),
            },
            buffers: vec![BufferSnapshotDto {
                buffer_id: bid("buffer-1"),
                path: WorkspacePath::new("src/main.rs").unwrap(),
                language: "rust".into(),
                text: "fn main() {}".into(),
                version: 3,
                dirty: false,
                selection: Some(Selection {
                    anchor: TextPosition::default(),
                    active: TextPosition {
                        line: 0,
                        utf16_column: 2,
                    },
                    version: 3,
                }),
            }],
            active_buffer_id: Some(bid("buffer-1")),
            file_tree: FileTreeDto { root: None },
            models: vec![],
            active_model_id: None,
            connection: ConnectionStatus::Disconnected,
            chat: ChatStateDto {
                pending_approvals: Vec::new(),
                sessions: vec![SessionDto {
                    session_id: sid("session-1"),
                    title: "Welcome".into(),
                    message_ids: vec![],
                    active: true,
                    plan_phase: PlanPhaseDto::Executing,
                    plan: None,
                    todos: vec![],
                }],
                messages: vec![],
                active_session_id: None,
                active_request_id: None,
                active_turn_id: None,
                pending_approval: None,
                todos: vec![],
            },
            settings: SettingsViewDto {
                reduced_motion: false,
                theme: "selora-dark".into(),
                keybind_command_palette: "Ctrl+Shift+P".into(),
                keybind_toggle_chat: "Ctrl+L".into(),
                keybind_fuzzy_find: "Ctrl+P".into(),
                keybind_save: "Ctrl+S".into(),
                editor_font_size: 14,
                editor_tab_size: 4,
                editor_word_wrap: false,
            },
            // §1.1: the Z1 stub values (280/220, agent dock open) were
            // diagnostic placeholders — Z2's fixture carries the table's
            // values (240 / 360-closed / 200-closed).
            layout: LayoutDto {
                primary_sidebar_width_px: PRIMARY_SIDEBAR_DEFAULT_PX,
                agent_dock_width_px: AGENT_DOCK_DEFAULT_PX,
                agent_dock_open: false,
                bottom_dock_height_px: BOTTOM_DOCK_DEFAULT_PX,
                bottom_dock_open: false,
                bottom_view: SurfaceId::Output,
                editor_splits: 1,
                activity_view: SurfaceId::Files,
                primary_sidebar_open: true,
            },
            layout_constraints: LayoutConstraintsDto::from_constants(),
            git: GitStatusDto {
                branch: None,
                changed_paths: vec![],
                staged_paths: vec![],
                files: vec![],
            },
            diagnostics: vec![],
            lsp_servers: vec![],
            feature_flags: FeatureFlagsDto {
                agent: false,
                git: false,
                search: false,
                editor: true,
                later_slices: vec!["agent".into(), "lsp".into()],
            },
            workflows: vec![],
            agent_sessions: vec![],
        }
    }
    #[test]
    fn bootstrap_round_trip() {
        let state = sample();
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(
            serde_json::from_str::<BootstrapState>(&json).unwrap(),
            state
        );
    }

    /// P1 settings slice: a pre-P1 wire view (only the original four fields)
    /// deserializes with the documented editor/keybind defaults.
    #[test]
    fn settings_view_old_wire_uses_editor_and_keybind_defaults() {
        let view: SettingsViewDto = serde_json::from_str(
            r#"{"reduced_motion":true,"theme":"light","keybind_command_palette":"F2","keybind_toggle_chat":"Alt+L"}"#,
        )
        .expect("pre-P1 view must load");
        assert!(view.reduced_motion);
        assert_eq!(view.theme, "light");
        assert_eq!(view.keybind_command_palette, "F2");
        assert_eq!(view.keybind_toggle_chat, "Alt+L");
        assert_eq!(view.editor_font_size, 14);
        assert_eq!(view.editor_tab_size, 4);
        assert!(!view.editor_word_wrap);
        assert_eq!(view.keybind_fuzzy_find, "Ctrl+P");
        assert_eq!(view.keybind_save, "Ctrl+S");
    }

    /// The patch is additive: missing fields are `None`, so an old frontend
    /// sending only a theme still parses.
    #[test]
    fn settings_patch_old_wire_is_additive() {
        let patch: SettingsPatchDto =
            serde_json::from_str(r#"{"theme":"light"}"#).expect("old patch parses");
        assert_eq!(patch.theme.as_deref(), Some("light"));
        assert!(patch.reduced_motion.is_none());
        assert!(patch.editor_font_size.is_none());
        assert!(patch.editor_tab_size.is_none());
        assert!(patch.editor_word_wrap.is_none());
        assert!(patch.keybind_command_palette.is_none());
        assert!(patch.keybind_fuzzy_find.is_none());
        assert!(patch.keybind_toggle_chat.is_none());
        assert!(patch.keybind_save.is_none());
    }
    #[test]
    fn phase9d_resume_picker_and_compaction_failure_round_trip() {
        let summary = AgentSessionSummaryDto {
            session_id: sid("sess-disk"),
            title: "Yesterday".into(),
            updated_at: 42,
            model: Some("test-model".into()),
            mode: "execute".into(),
            pending: Some(AgentParkSummaryDto {
                kind: "approval".into(),
                summary: "bash cargo test".into(),
            }),
            compacted: true,
            message_count: 12,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert_eq!(
            serde_json::from_str::<AgentSessionSummaryDto>(&json).unwrap(),
            summary
        );

        // A pre-9d bootstrap (no agent_sessions key) still loads the picker as
        // empty; the command/event spellings are stable.
        let mut state = sample();
        state.agent_sessions = vec![summary];
        let mut value = serde_json::to_value(&state).unwrap();
        value.as_object_mut().unwrap().remove("agent_sessions");
        let back: BootstrapState = serde_json::from_value(value).unwrap();
        assert!(back.agent_sessions.is_empty());

        assert_eq!(
            serde_json::to_string(&FrontendCommand::ListAgentSessions).unwrap(),
            r#"{"type":"list_agent_sessions"}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::SetAgentAutoApprove {
                session_id: sid("sess-disk"),
                enabled: true,
            })
            .unwrap(),
            r#"{"type":"set_agent_auto_approve","payload":{"session_id":"sess-disk","enabled":true}}"#
        );
        let failure = AgentCompactionFailedDto {
            request_id: rid("req-1"),
            session_id: sid("sess-disk"),
            turn_id: tid("turn-1"),
            message: "gateway down".into(),
            partial_summary: Some("half".into()),
        };
        let event = FrontendEvent::AgentCompactionFailed(failure.clone());
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "agent_compaction_failed");
        assert_eq!(json["payload"]["partial_summary"], "half");
    }
    #[test]
    fn serde_rejects_invalid_ids_paths() {
        assert!(serde_json::from_str::<RequestId>(r#"""#).is_err());
        assert!(serde_json::from_str::<RequestId>(&format!(
            r#""{}""#,
            "x".repeat(MAX_ID_LENGTH + 1)
        ))
        .is_err());
        assert!(serde_json::from_str::<WorkspacePath>(r#""../secret""#).is_err());
    }
    #[test]
    fn url_policy_is_https_host_only() {
        assert!(validate_external_url("https://selora.lol/docs").is_ok());
        for value in [
            "javascript:alert(1)",
            "https:///missing-host",
            "https://user:pass@example.com",
            "http://example.com",
        ] {
            assert!(validate_external_url(value).is_err(), "{value}");
        }
    }
    #[test]
    fn command_wire_shape_is_stable() {
        assert_eq!(
            serde_json::to_string(&FrontendCommand::OpenFolder).unwrap(),
            r#"{"type":"open_folder"}"#
        );
    }
    #[test]
    fn z4_command_wire_shapes_are_stable() {
        assert_eq!(
            serde_json::to_string(&FrontendCommand::OpenFolderAt {
                root: "C:/work".into()
            })
            .unwrap(),
            r#"{"type":"open_folder_at","payload":{"root":"C:/work"}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::RefreshTree).unwrap(),
            r#"{"type":"refresh_tree"}"#
        );

        assert_eq!(
            serde_json::to_string(&FrontendCommand::CloseFolder).unwrap(),
            r#"{"type":"close_folder"}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::LspRestart {
                language: "rust".into()
            })
            .unwrap(),
            r#"{"type":"lsp_restart","payload":{"language":"rust"}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::OpenFile {
                path: WorkspacePath::new("src/main.rs").unwrap()
            })
            .unwrap(),
            r#"{"type":"open_file","payload":{"path":"src/main.rs"}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::ApplyEditorChange {
                buffer_id: BufferId::new("buf_1").unwrap(),
                version: 3,
                changes: vec![TextChange {
                    start: TextPosition {
                        line: 0,
                        utf16_column: 0
                    },
                    end: TextPosition {
                        line: 0,
                        utf16_column: 2
                    },
                    replacement: "fn".into(),
                }],
            })
            .unwrap(),
            r#"{"type":"apply_editor_change","payload":{"buffer_id":"buf_1","version":3,"changes":[{"start":{"line":0,"utf16_column":0},"end":{"line":0,"utf16_column":2},"replacement":"fn"}]}}"#
        );
        // The empty-tree refresh round-trips from the frontend's exact shape.
        let parsed: FrontendCommand =
            serde_json::from_str(r#"{"type":"search","payload":{"query":{"query":"fn main","case_sensitive":false,"whole_word":false,"use_regex":false}}}"#)
                .unwrap();
        assert!(matches!(parsed, FrontendCommand::Search { .. }));
    }

    #[test]
    fn reveal_in_explorer_wire_shape_round_trips() {
        let reveal = serde_json::to_string(&FrontendCommand::RevealInExplorer {
            path: WorkspacePath::new("src/main.rs").unwrap(),
        })
        .unwrap();
        assert_eq!(
            reveal,
            r#"{"type":"reveal_in_explorer","payload":{"path":"src/main.rs"}}"#
        );
        let parsed: FrontendCommand = serde_json::from_str(&reveal).unwrap();
        assert!(matches!(
            parsed,
            FrontendCommand::RevealInExplorer { path } if path.as_str() == "src/main.rs"
        ));
    }

    #[test]
    fn z2_command_wire_shapes_are_stable() {
        // §2.1's exact spellings, verified against the serde envelope.
        assert_eq!(
            serde_json::to_string(&FrontendCommand::ToggleSurface {
                surface: SurfaceId::Agent
            })
            .unwrap(),
            r#"{"type":"toggle_surface","payload":{"surface":"agent"}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::ResizeRegion {
                region: RegionId::PrimarySidebar,
                size_px: 240.0
            })
            .unwrap(),
            r#"{"type":"resize_region","payload":{"region":"primary_sidebar","size_px":240.0}}"#
        );
        // A unit variant: no `payload` key (same shape as `open_folder`).
        assert_eq!(
            serde_json::to_string(&FrontendCommand::SaveLayout).unwrap(),
            r#"{"type":"save_layout"}"#
        );
    }
    #[test]
    fn z2_event_wire_shapes_are_stable() {
        let event = FrontendEvent::WorkspaceLayoutChanged(LayoutDto {
            primary_sidebar_width_px: 240.0,
            agent_dock_width_px: 360.0,
            agent_dock_open: false,
            bottom_dock_height_px: 200.0,
            bottom_dock_open: false,
            bottom_view: SurfaceId::Output,
            editor_splits: 1,
            activity_view: SurfaceId::Files,
            primary_sidebar_open: true,
        });
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"workspace_layout_changed","payload":{"primary_sidebar_width_px":240.0,"agent_dock_width_px":360.0,"agent_dock_open":false,"bottom_dock_height_px":200.0,"bottom_dock_open":false,"bottom_view":"output","editor_splits":1,"activity_view":"files","primary_sidebar_open":true}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendEvent::Toast(ToastDto {
                severity: ToastSeverity::Info,
                message: "saved".into(),
            }))
            .unwrap(),
            r#"{"type":"toast","payload":{"severity":"info","message":"saved"}}"#
        );
    }
    #[test]
    fn z2_command_serde_boundary_rejects_malformed_payloads() {
        // Unknown surface/region spellings fail enum deserialization (§2.3
        // item 4) — the rejection happens at the serde boundary, before any
        // Rust arm runs.
        assert!(serde_json::from_str::<FrontendCommand>(
            r#"{"type":"toggle_surface","payload":{"surface":"nonexistent"}}"#
        )
        .is_err());
        assert!(serde_json::from_str::<FrontendCommand>(
            r#"{"type":"resize_region","payload":{"region":"sidebar"}}"#
        )
        .is_err());
        // `size_px: null` is a type error at the same boundary.
        assert!(serde_json::from_str::<FrontendCommand>(
            r#"{"type":"resize_region","payload":{"region":"primary_sidebar","size_px":null}}"#
        )
        .is_err());
    }
    #[test]
    fn layout_dto_round_trip_includes_new_fields() {
        let layout = LayoutDto {
            primary_sidebar_width_px: 320.5,
            agent_dock_width_px: 411.25,
            agent_dock_open: true,
            bottom_dock_height_px: 333.75,
            bottom_dock_open: true,
            bottom_view: SurfaceId::Problems,
            editor_splits: 3,
            activity_view: SurfaceId::Git,
            primary_sidebar_open: false,
        };
        let json = serde_json::to_string(&layout).unwrap();
        assert_eq!(serde_json::from_str::<LayoutDto>(&json).unwrap(), layout);
        // The two new fields really ride the wire.
        assert!(json.contains(r#""activity_view":"git""#));
        assert!(json.contains(r#""primary_sidebar_open":false"#));
    }
    #[test]
    fn pre_z2_and_partial_layout_json_load_with_defaults() {
        // A pre-Z2 persisted layout (the seven old fields) deserializes with
        // the §4.2 defaults for the two new fields.
        let pre_z2 = serde_json::from_str::<LayoutDto>(
            r#"{"primary_sidebar_width_px":240.0,"agent_dock_width_px":360.0,"agent_dock_open":false,"bottom_dock_height_px":200.0,"bottom_dock_open":false,"bottom_view":"output","editor_splits":1}"#,
        )
        .unwrap();
        assert_eq!(pre_z2.activity_view, SurfaceId::Files);
        assert!(pre_z2.primary_sidebar_open);
        // A partial (or empty) layout JSON fills every missing field with the
        // §4.2 defaults (§5.1: "every existing LayoutDto field gains a serde
        // default").
        let partial =
            serde_json::from_str::<LayoutDto>(r#"{"primary_sidebar_width_px":300.0}"#).unwrap();
        assert_eq!(
            partial,
            LayoutDto {
                primary_sidebar_width_px: 300.0,
                ..default_layout()
            }
        );
        assert_eq!(
            serde_json::from_str::<LayoutDto>(r#"{}"#).unwrap(),
            default_layout()
        );
    }
    #[test]
    fn clamp_region_size_bounds_table_and_non_finite() {
        use RegionId::*;
        // (region, input, expected) — finite out-of-range values are clamped,
        // never rejected; in-range values pass through exactly.
        for (region, input, expected) in [
            (PrimarySidebar, 100.0, PRIMARY_SIDEBAR_MIN_PX),
            (PrimarySidebar, 240.0, 240.0),
            (PrimarySidebar, 9999.0, PRIMARY_SIDEBAR_MAX_PX),
            (AgentDock, 100.0, AGENT_DOCK_MIN_PX),
            (AgentDock, 550.5, 550.5),
            (AgentDock, 600.0, AGENT_DOCK_MAX_PX),
            (BottomDock, 0.0, BOTTOM_DOCK_MIN_PX),
            (BottomDock, 200.0, 200.0),
            (BottomDock, 80_000.0, BOTTOM_DOCK_MAX_PX),
        ] {
            assert_eq!(clamp_region_size(region, input).unwrap(), expected);
        }
        // Non-finite values are rejected (the Rust-internal invariant guard).
        for region in [PrimarySidebar, AgentDock, BottomDock] {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(
                    clamp_region_size(region, value),
                    Err(ProtocolValidationError::InvalidRegionSize)
                );
            }
        }
    }
    #[test]
    fn sanitized_fixes_out_of_range_and_out_of_set_values() {
        let wedged = LayoutDto {
            primary_sidebar_width_px: f32::NAN,
            agent_dock_width_px: 9000.0,
            agent_dock_open: true,
            bottom_dock_height_px: 1.0,
            bottom_dock_open: true,
            // `agent` is not an activity view; `files` is not a bottom view.
            bottom_view: SurfaceId::Files,
            editor_splits: 0,
            activity_view: SurfaceId::Agent,
            primary_sidebar_open: true,
        };
        let sanitized = wedged.clone().sanitized();
        assert_eq!(sanitized.primary_sidebar_width_px, 240.0, "NaN → default");
        assert_eq!(sanitized.agent_dock_width_px, 600.0, "clamped to max");
        assert_eq!(sanitized.bottom_dock_height_px, 120.0, "clamped to min");
        assert_eq!(sanitized.bottom_view, SurfaceId::Output, "reset into set");
        assert_eq!(sanitized.editor_splits, EDITOR_SPLITS_MIN);
        assert_eq!(sanitized.activity_view, SurfaceId::Files, "reset into set");
        // Open flags are semantics, not sizes — untouched.
        assert!(sanitized.agent_dock_open);
        assert!(sanitized.bottom_dock_open);
        // A clean layout is a fixed point.
        assert_eq!(default_layout().clone().sanitized(), default_layout());
        // editor_splits clamps from above too.
        let splits = LayoutDto {
            editor_splits: 9,
            ..default_layout()
        };
        assert_eq!(splits.sanitized().editor_splits, EDITOR_SPLITS_MAX);
    }
    #[test]
    fn layout_constraints_dto_matches_constants() {
        let constraints = LayoutConstraintsDto::from_constants();
        assert_eq!(constraints.primary_sidebar_min_px, PRIMARY_SIDEBAR_MIN_PX);
        assert_eq!(constraints.primary_sidebar_max_px, PRIMARY_SIDEBAR_MAX_PX);
        assert_eq!(constraints.agent_dock_min_px, AGENT_DOCK_MIN_PX);
        assert_eq!(constraints.agent_dock_max_px, AGENT_DOCK_MAX_PX);
        assert_eq!(constraints.bottom_dock_min_px, BOTTOM_DOCK_MIN_PX);
        assert_eq!(constraints.bottom_dock_max_px, BOTTOM_DOCK_MAX_PX);
        assert_eq!(
            constraints.bottom_dock_max_workbench_ratio,
            BOTTOM_DOCK_MAX_WORKBENCH_RATIO
        );
        assert_eq!(constraints.editor_min_width_px, EDITOR_MIN_WIDTH_PX);
        assert_eq!(constraints.resize_hit_zone_min_px, RESIZE_HIT_ZONE_MIN_PX);
    }
    #[test]
    fn protocol_constants_are_the_documented_table() {
        // §1.1's table, verbatim — the single source of truth everything else
        // (settings defaults, the Z2 state bootstrap) delegates to.
        assert_eq!(PRIMARY_SIDEBAR_MIN_PX, 160.0);
        assert_eq!(PRIMARY_SIDEBAR_MAX_PX, 480.0);
        assert_eq!(PRIMARY_SIDEBAR_DEFAULT_PX, 240.0);
        assert_eq!(AGENT_DOCK_MIN_PX, 240.0);
        assert_eq!(AGENT_DOCK_MAX_PX, 600.0);
        assert_eq!(AGENT_DOCK_DEFAULT_PX, 360.0);
        assert_eq!(BOTTOM_DOCK_MIN_PX, 120.0);
        assert_eq!(BOTTOM_DOCK_MAX_PX, 800.0);
        assert_eq!(BOTTOM_DOCK_DEFAULT_PX, 200.0);
        assert_eq!(BOTTOM_DOCK_MAX_WORKBENCH_RATIO, 0.5);
        assert_eq!(EDITOR_MIN_WIDTH_PX, 400.0);
        assert_eq!(RESIZE_HIT_ZONE_MIN_PX, 6.0);
        assert_eq!(EDITOR_SPLITS_MIN, 1);
        assert_eq!(EDITOR_SPLITS_MAX, 4);
        // §4.3's valid sets.
        assert!(ACTIVITY_VIEWS.contains(&SurfaceId::Files));
        assert!(ACTIVITY_VIEWS.contains(&SurfaceId::Workflows));
        assert!(!ACTIVITY_VIEWS.contains(&SurfaceId::Problems));
        assert!(BOTTOM_VIEWS.contains(&SurfaceId::Problems));
        assert!(!BOTTOM_VIEWS.contains(&SurfaceId::Files));
    }

    /// The §1.1-table default layout (§4.2's "wire shape of a full section").
    fn default_layout() -> LayoutDto {
        LayoutDto {
            primary_sidebar_width_px: 240.0,
            agent_dock_width_px: 360.0,
            agent_dock_open: false,
            bottom_dock_height_px: 200.0,
            bottom_dock_open: false,
            bottom_view: SurfaceId::Output,
            editor_splits: 1,
            activity_view: SurfaceId::Files,
            primary_sidebar_open: true,
        }
    }
    #[test]
    fn event_order_rejects_stale_gap_epoch() {
        let event = EventEnvelope {
            protocol_version: PROTOCOL_VERSION,
            workspace_id: None,
            core_instance_id: cid("core-1"),
            sequence: 4,
            request_id: None,
            session_id: None,
            payload: FrontendEvent::Error(ProtocolErrorDto {
                code: "x".into(),
                message: "x".into(),
                retryable: false,
            }),
        };
        assert!(validate_event_order(&event, PROTOCOL_VERSION, &cid("core-1"), Some(3)).is_ok());
        assert!(matches!(
            validate_event_order(&event, PROTOCOL_VERSION, &cid("core-1"), Some(2)),
            Err(EventValidationError::SequenceGap { .. })
        ));
        assert!(matches!(
            validate_event_order(&event, PROTOCOL_VERSION, &cid("core-2"), None),
            Err(EventValidationError::CoreInstanceMismatch { .. })
        ));
    }
    #[test]
    fn z3_command_wire_shapes_are_stable() {
        // The four agent-dock commands, exact spellings (the Z3 contract
        // Antigravity's bridge types are built against).
        assert_eq!(
            serde_json::to_string(&FrontendCommand::SendChat {
                request_id: rid("request-1"),
                request: ChatRequestDto {
                    session_id: sid("session-1"),
                    message: "hello".into(),
                    model_id: Some("glm-5.3".into()),
                },
            })
            .unwrap(),
            r#"{"type":"send_chat","payload":{"request_id":"request-1","request":{"session_id":"session-1","message":"hello","model_id":"glm-5.3"}}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::StopChat {
                request_id: rid("request-1"),
            })
            .unwrap(),
            r#"{"type":"stop_chat","payload":{"request_id":"request-1"}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::SendAgent {
                request_id: rid("request-1"),
                request: AgentRequestDto {
                    session_id: sid("session-1"),
                    prompt: "fix it".into(),
                    model_id: None,
                    mode: AgentModeDto::Execute,
                    reasoning_effort: None,
                    system_instructions: None,
                    temperature: None,
                    editor_context: None,
                },
            })
            .unwrap(),
            r#"{"type":"send_agent","payload":{"request_id":"request-1","request":{"session_id":"session-1","prompt":"fix it","model_id":null,"mode":"execute"}}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::AgentDecision {
                resolution: None,
                request_id: rid("request-1"),
                approval_id: aid("approval-1"),
                decision: DecisionDto::ApproveAllThisTurn,
            })
            .unwrap(),
            r#"{"type":"agent_decision","payload":{"request_id":"request-1","approval_id":"approval-1","decision":"approve_all_this_turn"}}"#
        );
    }
    #[test]
    fn z3_event_wire_shapes_are_stable() {
        assert_eq!(
            serde_json::to_string(&FrontendEvent::ToolCallStarted(ToolCallStartedDto {
                request_id: rid("request-1"),
                session_id: sid("session-1"),
                turn_id: tid("turn-1"),
                call_id: "call_1".into(),
                tool_name: "read_file".into(),
                args_json: r#"{"path":"src/main.rs"}"#.into(),
            }))
            .unwrap(),
            r#"{"type":"tool_call_started","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","call_id":"call_1","tool_name":"read_file","args_json":"{\"path\":\"src/main.rs\"}"}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendEvent::ToolResult(ToolResultDto {
                request_id: rid("request-1"),
                session_id: sid("session-1"),
                turn_id: tid("turn-1"),
                call_id: "call_1".into(),
                result_summary: "fn main() {}".into(),
                is_err: false,
            }))
            .unwrap(),
            r#"{"type":"tool_result","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","call_id":"call_1","result_summary":"fn main() {}","is_err":false}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendEvent::ApprovalNeeded(ApprovalNeededDto {
                request_id: rid("request-1"),
                session_id: sid("session-1"),
                turn_id: tid("turn-1"),
                approval_id: aid("approval-1"),
                tool_name: "write_file".into(),
                path_or_command: "src/main.rs".into(),
                diff_or_input: Some("+fn main() {}".into()),
                workflow: None,
                plan: None,
                prompt: None,
                subagent_id: None,
            }))
            .unwrap(),
            r#"{"type":"approval_needed","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","approval_id":"approval-1","tool_name":"write_file","path_or_command":"src/main.rs","diff_or_input":"+fn main() {}"}}"#
        );
        // A bash card: no diff (the `diff_or_input` key is absent, not null).
        assert_eq!(
            serde_json::to_string(&FrontendEvent::ApprovalNeeded(ApprovalNeededDto {
                request_id: rid("request-1"),
                session_id: sid("session-1"),
                turn_id: tid("turn-1"),
                approval_id: aid("approval-1"),
                tool_name: "bash".into(),
                path_or_command: "cargo test".into(),
                diff_or_input: None,
                workflow: None,
                plan: None,
                prompt: None,
                subagent_id: None,
            }))
            .unwrap(),
            r#"{"type":"approval_needed","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","approval_id":"approval-1","tool_name":"bash","path_or_command":"cargo test"}}"#
        );
        // Phase 8: a `run_workflow` card carries the confirmation payload —
        // script, confirmed command set and cap — additively (`tool_name`
        // stays `run_workflow` so the card routes to the WorkflowCard).
        let workflow_card = ApprovalNeededDto {
            request_id: rid("request-1"),
            session_id: sid("session-1"),
            turn_id: tid("turn-1"),
            approval_id: aid("approval-1"),
            tool_name: "run_workflow".into(),
            path_or_command: "repo-audit".into(),
            diff_or_input: None,
            workflow: Some(WorkflowApprovalDto {
                name: "repo-audit".into(),
                script: "phase('scan');".into(),
                commands: vec!["cargo test".into(), "git status".into()],
                max_cost_usd: 5.0,
            }),
            plan: None,
            prompt: None,
            subagent_id: None,
        };
        let workflow_json =
            serde_json::to_string(&FrontendEvent::ApprovalNeeded(workflow_card.clone())).unwrap();
        assert_eq!(
            workflow_json,
            r#"{"type":"approval_needed","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","approval_id":"approval-1","tool_name":"run_workflow","path_or_command":"repo-audit","workflow":{"name":"repo-audit","script":"phase('scan');","commands":["cargo test","git status"],"max_cost_usd":5.0}}}"#
        );
        // Round-trip is identity.
        assert_eq!(
            serde_json::from_str::<FrontendEvent>(&workflow_json).unwrap(),
            FrontendEvent::ApprovalNeeded(workflow_card)
        );
        // A pre-8 (or non-workflow) card without the key still loads.
        let legacy: FrontendEvent = serde_json::from_str(
            r#"{"type":"approval_needed","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","approval_id":"approval-1","tool_name":"bash","path_or_command":"cargo test"}}"#,
        )
        .unwrap();
        match legacy {
            FrontendEvent::ApprovalNeeded(dto) => assert!(dto.workflow.is_none()),
            other => panic!("expected a legacy approval card, got {other:?}"),
        }
        // A partial payload fills the §18.1 cap default and an empty command set.
        let partial: WorkflowApprovalDto =
            serde_json::from_str(r#"{"name":"repo-audit","script":"report({})"}"#).unwrap();
        assert!(partial.commands.is_empty());
        assert_eq!(partial.max_cost_usd, default_workflow_max_cost_usd());
        assert_eq!(
            serde_json::to_string(&FrontendEvent::AgentStateChanged(AgentStateDto {
                request_id: rid("request-1"),
                session_id: sid("session-1"),
                turn_id: tid("turn-1"),
                state: AgentPhase::RunningTool,
                tool_name: Some("bash".into()),
            }))
            .unwrap(),
            r#"{"type":"agent_state_changed","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","state":"running_tool","tool_name":"bash"}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendEvent::AgentStateChanged(AgentStateDto {
                request_id: rid("request-1"),
                session_id: sid("session-1"),
                turn_id: tid("turn-1"),
                state: AgentPhase::Thinking,
                tool_name: None,
            }))
            .unwrap(),
            r#"{"type":"agent_state_changed","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","state":"thinking"}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendEvent::AgentTurnDone(AgentTurnDoneDto {
                request_id: rid("request-1"),
                session_id: sid("session-1"),
                turn_id: tid("turn-1"),
                total_tokens: 12400,
                cost_usd: 0.041,
                num_tools: 6,
                approved: 3,
                denied: 1,
                completed: true,
                end_message: "agent turn complete".into(),
            }))
            .unwrap(),
            r#"{"type":"agent_turn_done","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","total_tokens":12400,"cost_usd":0.041,"num_tools":6,"approved":3,"denied":1,"completed":true,"end_message":"agent turn complete"}}"#
        );
    }
    #[test]
    fn phase_9e_subagent_completion_wire_shape_is_stable() {
        // One event carries the child, the parent, the policy, the honest
        // ok/error pair, the final message and artifact refs.
        let event = FrontendEvent::SubagentCompleted(SubagentCompletedDto {
            child_id: "child-1".into(),
            parent_session_id: sid("session-1"),
            policy: SubagentPolicyDto::ReadOnly,
            ok: false,
            error: Some("gateway said no".into()),
            final_message: String::new(),
            artifacts: vec![SubagentArtifactDto {
                kind: "diff".into(),
                path: "src/lib.rs".into(),
            }],
        });
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"subagent_completed","payload":{"child_id":"child-1","parent_session_id":"session-1","policy":"read_only","ok":false,"error":"gateway said no","final_message":"","artifacts":[{"kind":"diff","path":"src/lib.rs"}]}}"#
        );
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<FrontendEvent>(&json).unwrap(), event);
    }
    #[test]
    fn z3_serde_boundaries_reject_malformed_payloads() {
        // An unknown mode spelling fails at the boundary, before any Rust arm.
        assert!(serde_json::from_str::<FrontendCommand>(
            r#"{"type":"send_agent","payload":{"request_id":"r","request":{"session_id":"s","prompt":"p","mode":"plan"}}}"#
        )
        .is_err());
        // An unknown decision spelling likewise.
        assert!(serde_json::from_str::<FrontendCommand>(
            r#"{"type":"agent_decision","payload":{"request_id":"r","approval_id":"a","decision":"maybe"}}"#
        )
        .is_err());
        // The new event spellings round-trip through the full enum.
        let event = FrontendEvent::AgentStateChanged(AgentStateDto {
            request_id: rid("request-1"),
            session_id: sid("session-1"),
            turn_id: tid("turn-1"),
            state: AgentPhase::AwaitingApproval,
            tool_name: None,
        });
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<FrontendEvent>(&json).unwrap(), event);
    }
    #[test]
    fn p9_todos_and_command_results_are_additive() {
        // A pre-Phase-9 chat snapshot (no todos) still loads: the additive
        // fields default to empty lists.
        let chat = serde_json::from_str::<ChatStateDto>(
            r#"{"sessions":[{"session_id":"s","title":"t","message_ids":[],"active":true}],
                "messages":[],"active_session_id":null,"active_request_id":null,
                "active_turn_id":null,"pending_approval":null}"#,
        )
        .unwrap();
        assert!(chat.todos.is_empty());
        assert!(chat.sessions[0].todos.is_empty());

        // The full list round-trips, including the status spelling.
        let with_todos = ChatStateDto {
            todos: vec![TodoDto {
                id: "t1".into(),
                title: "build".into(),
                status: TodoStatusDto::InProgress,
            }],
            ..chat.clone()
        };
        let json = serde_json::to_string(&with_todos).unwrap();
        assert!(json.contains(r#""status":"in_progress""#), "{json}");
        assert_eq!(
            serde_json::from_str::<ChatStateDto>(&json).unwrap(),
            with_todos
        );

        // A full snapshot still parses as the snapshot arm; an ack parses as
        // the small arm. The reply is always one JSON object.
        let snapshot = sample();
        let reply = CommandResultDto::Snapshot(Box::new(snapshot.clone()));
        let json = serde_json::to_string(&reply).unwrap();
        assert_eq!(
            serde_json::from_str::<CommandResultDto>(&json).unwrap(),
            reply
        );
        let ack = CommandResultDto::Ack(CommandAckDto {
            protocol_version: PROTOCOL_VERSION,
            core_instance_id: cid("core-1"),
            snapshot_sequence: 42,
            acknowledged: true,
            session_id: Some(sid("session-1")),
        });
        let json = serde_json::to_string(&ack).unwrap();
        assert!(json.len() < serde_json::to_string(&snapshot).unwrap().len());
        assert_eq!(
            serde_json::from_str::<CommandResultDto>(&json).unwrap(),
            ack
        );

        // The todo event rides the additive enum spelling.
        let event = FrontendEvent::TodosChanged(TodosDto {
            session_id: sid("session-1"),
            todos: with_todos.todos.clone(),
        });
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"todos_changed""#), "{json}");
        assert_eq!(serde_json::from_str::<FrontendEvent>(&json).unwrap(), event);
    }

    #[test]
    fn pre_z3_chat_completed_and_agent_request_load_with_defaults() {
        // A pre-Z3 ChatCompletedDto (no metering) deserializes with zeros.
        let completed = serde_json::from_str::<ChatCompletedDto>(
            r#"{"request_id":"r","session_id":"s","turn_id":"t","message_id":"m","text":"hi"}"#,
        )
        .unwrap();
        assert_eq!(completed.total_tokens, 0);
        assert_eq!(completed.cost_usd, 0.0);
        // A pre-Z3 AgentRequestDto (no mode) defaults to execute.
        let request = serde_json::from_str::<AgentRequestDto>(
            r#"{"session_id":"s","prompt":"p","model_id":null}"#,
        )
        .unwrap();
        assert!(matches!(request.mode, AgentModeDto::Execute));
    }
    #[test]
    fn editor_context_round_trips_and_old_payloads_default() {
        let request = AgentRequestDto {
            session_id: sid("session-1"),
            prompt: "fix it".into(),
            model_id: None,
            mode: AgentModeDto::Execute,
            reasoning_effort: None,
            system_instructions: None,
            temperature: None,
            editor_context: Some(EditorContextDto {
                path: "src/main.rs".into(),
                language: "rust".into(),
                selection: Some(Selection {
                    anchor: TextPosition {
                        line: 6,
                        utf16_column: 0,
                    },
                    active: TextPosition {
                        line: 6,
                        utf16_column: 13,
                    },
                    version: 3,
                }),
                dirty: true,
            }),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""editor_context""#), "{json}");
        assert!(json.contains(r#""utf16_column""#), "{json}");
        let back = serde_json::from_str::<AgentRequestDto>(&json).unwrap();
        assert_eq!(back, request);
        // An old payload (no editor context) deserializes with none.
        let old = serde_json::from_str::<AgentRequestDto>(
            r#"{"session_id":"s","prompt":"p","model_id":null}"#,
        )
        .unwrap();
        assert!(old.editor_context.is_none());
    }
    fn git_path(value: &str) -> WorkspacePath {
        WorkspacePath::new(value).unwrap()
    }

    #[test]
    fn git_status_round_trip_includes_per_slot_files() {
        let status = GitStatusDto {
            branch: Some("main".into()),
            changed_paths: vec![git_path("src/both.rs")],
            staged_paths: vec![git_path("src/both.rs")],
            files: vec![
                GitFileDto {
                    path: git_path("src/both.rs"),
                    staged: true,
                    worktree: true,
                },
                GitFileDto {
                    path: git_path("src/new.rs"),
                    staged: false,
                    worktree: true,
                },
            ],
        };
        let json = serde_json::to_string(&status).unwrap();
        // The new field really rides the wire, carrying both slots per file.
        assert!(
            json.contains(
                r#""files":[{"path":"src/both.rs","staged":true,"worktree":true},{"path":"src/new.rs","staged":false,"worktree":true}]"#
            ),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<GitStatusDto>(&json).unwrap(), status);
    }

    #[test]
    fn pre_p2_git_status_json_loads_with_no_files() {
        // A pre-P2 payload (the two legacy arrays only) deserializes with an
        // empty `files`, so readers keep using the arrays as the fallback.
        let status = serde_json::from_str::<GitStatusDto>(
            r#"{"branch":"main","changed_paths":["src/main.rs"],"staged_paths":[]}"#,
        )
        .unwrap();
        assert_eq!(status.changed_paths.len(), 1);
        assert!(status.staged_paths.is_empty());
        assert!(status.files.is_empty());
    }
    #[test]
    fn approval_is_bound_and_consumed_once() {
        let mut pending = Some(ApprovalDto {
            approval_id: aid("approval"),
            request_id: rid("request"),
            turn_id: tid("turn"),
            session_id: sid("session"),
            summary: "write".into(),
            can_approve_all_this_turn: false,
            plan: None,
            decision_prompt: None,
        });
        assert_eq!(
            validate_and_consume_approval(
                &mut pending,
                &rid("request"),
                &sid("session"),
                &tid("turn"),
                &aid("approval"),
                DecisionDto::Approve
            ),
            Ok(DecisionDto::Approve)
        );
        assert!(pending.is_none());
        assert_eq!(
            validate_and_consume_approval(
                &mut pending,
                &rid("request"),
                &sid("session"),
                &tid("turn"),
                &aid("approval"),
                DecisionDto::Approve
            ),
            Err(ApprovalValidationError::NoPendingApproval)
        );
    }

    // -----------------------------------------------------------------------
    // Phase 8 — workflows.
    // -----------------------------------------------------------------------

    fn workflow_spec() -> WorkflowSpecDto {
        WorkflowSpecDto {
            name: "review".into(),
            script: "phase('scan')".into(),
            commands: vec!["cargo test".into()],
            max_cost_usd: 5.0,
        }
    }

    fn workflow_step() -> WorkflowStepDto {
        WorkflowStepDto {
            id: "step-1".into(),
            phase: "Scan".into(),
            label: "read src/main.rs".into(),
            status: WorkflowStatus::Running,
            agent: Some("reviewer".into()),
            detail: Some("reading".into()),
        }
    }

    fn workflow_finished(status: WorkflowStatus, error: Option<&str>) -> WorkflowFinishedDto {
        WorkflowFinishedDto {
            run_id: "run-1".into(),
            request_id: rid("request-1"),
            status,
            tokens: 1200,
            cost_usd: 0.12,
            max_cost_usd: 5.0,
            error: error.map(str::to_owned),
        }
    }

    fn assert_command_wire(command: FrontendCommand, expected: &str) {
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(json, expected);
        assert_eq!(
            serde_json::from_str::<FrontendCommand>(&json).unwrap(),
            command
        );
    }

    fn assert_event_wire(event: FrontendEvent, expected: &str) {
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, expected);
        assert_eq!(serde_json::from_str::<FrontendEvent>(&json).unwrap(), event);
    }

    #[test]
    fn p8_workflow_command_wire_shapes_are_stable() {
        assert_command_wire(
            FrontendCommand::WorkflowRun {
                request_id: rid("request-1"),
                spec: workflow_spec(),
            },
            r#"{"type":"workflow_run","payload":{"request_id":"request-1","spec":{"name":"review","script":"phase('scan')","commands":["cargo test"],"max_cost_usd":5.0}}}"#,
        );
        assert_command_wire(
            FrontendCommand::WorkflowResume {
                request_id: rid("request-2"),
                run_id: "run-1".into(),
                allow_incomplete_asks: false,
                acknowledge_partial_effects: false,
            },
            r#"{"type":"workflow_resume","payload":{"request_id":"request-2","run_id":"run-1","allow_incomplete_asks":false,"acknowledge_partial_effects":false}}"#,
        );
        assert_command_wire(
            FrontendCommand::WorkflowResume {
                request_id: rid("request-5"),
                run_id: "run-1".into(),
                allow_incomplete_asks: true,
                acknowledge_partial_effects: true,
            },
            r#"{"type":"workflow_resume","payload":{"request_id":"request-5","run_id":"run-1","allow_incomplete_asks":true,"acknowledge_partial_effects":true}}"#,
        );
        assert_command_wire(
            FrontendCommand::WorkflowStop {
                request_id: rid("request-3"),
            },
            r#"{"type":"workflow_stop","payload":{"request_id":"request-3"}}"#,
        );
        assert_command_wire(
            FrontendCommand::WorkflowDecision {
                request_id: rid("request-4"),
                approval_id: aid("approval-1"),
                decision: DecisionDto::Approve,
            },
            r#"{"type":"workflow_decision","payload":{"request_id":"request-4","approval_id":"approval-1","decision":"approve"}}"#,
        );
    }

    #[test]
    fn p8_workflow_started_preserves_origin_and_accepts_legacy_json() {
        let legacy =
            r#"{"run_id":"run-1","request_id":"request-1","name":"review","max_cost_usd":5.0}"#;
        let mut started: WorkflowStartedDto = serde_json::from_str(legacy).unwrap();
        assert!(started.workspace_id.is_none());
        started.workspace_id = Some(WorkspaceId::new("workspace-a").unwrap());
        let encoded = serde_json::to_value(&started).unwrap();
        assert_eq!(encoded["workspace_id"], "workspace-a");
        assert_eq!(
            serde_json::from_value::<WorkflowStartedDto>(encoded).unwrap(),
            started
        );
    }

    #[test]
    fn p8_workflow_event_wire_shapes_are_stable() {
        assert_event_wire(
            FrontendEvent::WorkflowStarted(WorkflowStartedDto {
                run_id: "run-1".into(),
                request_id: rid("request-1"),
                workspace_id: None,
                name: "review".into(),
                max_cost_usd: 5.0,
            }),
            r#"{"type":"workflow_started","payload":{"run_id":"run-1","request_id":"request-1","name":"review","max_cost_usd":5.0}}"#,
        );
        assert_event_wire(
            FrontendEvent::WorkflowPhase(WorkflowPhaseEventDto {
                run_id: "run-1".into(),
                phase: WorkflowPhaseDto {
                    name: "Scan".into(),
                    status: WorkflowStatus::Running,
                },
            }),
            r#"{"type":"workflow_phase","payload":{"run_id":"run-1","phase":{"name":"Scan","status":"running"}}}"#,
        );
        assert_event_wire(
            FrontendEvent::WorkflowStep(WorkflowStepEventDto {
                run_id: "run-1".into(),
                step: workflow_step(),
            }),
            r#"{"type":"workflow_step","payload":{"run_id":"run-1","step":{"id":"step-1","phase":"Scan","label":"read src/main.rs","status":"running","agent":"reviewer","detail":"reading"}}}"#,
        );
        assert_event_wire(
            FrontendEvent::WorkflowReport(WorkflowReportEventDto {
                run_id: "run-1".into(),
                report: WorkflowReportDto {
                    id: "report-1".into(),
                    item: "confirmed finding".into(),
                    artifact_id: Some("artifact-1".into()),
                    phase: None,
                },
            }),
            r#"{"type":"workflow_report","payload":{"run_id":"run-1","report":{"id":"report-1","item":"confirmed finding","artifact_id":"artifact-1"}}}"#,
        );
        assert_event_wire(
            FrontendEvent::WorkflowArtifact(WorkflowArtifactEventDto {
                run_id: "run-1".into(),
                artifact: WorkflowArtifactDto {
                    id: "artifact-1".into(),
                    title: "Review".into(),
                    kind: "markdown".into(),
                    body: "# Review".into(),
                    phase: None,
                },
            }),
            r##"{"type":"workflow_artifact","payload":{"run_id":"run-1","artifact":{"id":"artifact-1","title":"Review","kind":"markdown","body":"# Review"}}}"##,
        );
        assert_event_wire(
            FrontendEvent::WorkflowApprovalNeeded(WorkflowApprovalNeededDto {
                run_id: "run-1".into(),
                request_id: rid("request-1"),
                approval_id: aid("approval-1"),
                summary: "write src/main.rs".into(),
                path_or_command: "src/main.rs".into(),
                diff_or_input: Some("+fn main() {}".into()),
                can_approve_all_this_turn: false,
            }),
            r#"{"type":"workflow_approval_needed","payload":{"run_id":"run-1","request_id":"request-1","approval_id":"approval-1","summary":"write src/main.rs","path_or_command":"src/main.rs","diff_or_input":"+fn main() {}","can_approve_all_this_turn":false}}"#,
        );
        // Completed: `error` is absent, not null.
        assert_event_wire(
            FrontendEvent::WorkflowFinished(workflow_finished(WorkflowStatus::Completed, None)),
            r#"{"type":"workflow_finished","payload":{"run_id":"run-1","request_id":"request-1","status":"completed","tokens":1200,"cost_usd":0.12,"max_cost_usd":5.0}}"#,
        );
        assert_event_wire(
            FrontendEvent::WorkflowFinished(workflow_finished(
                WorkflowStatus::Failed,
                Some("budget exceeded"),
            )),
            r#"{"type":"workflow_finished","payload":{"run_id":"run-1","request_id":"request-1","status":"failed","tokens":1200,"cost_usd":0.12,"max_cost_usd":5.0,"error":"budget exceeded"}}"#,
        );
    }

    #[test]
    fn p8_old_json_without_workflow_fields_still_loads() {
        // Existing command/event spellings are untouched by the additive arms.
        let command: FrontendCommand =
            serde_json::from_str(r#"{"type":"stop_chat","payload":{"request_id":"request-1"}}"#)
                .unwrap();
        assert!(matches!(command, FrontendCommand::StopChat { .. }));
        let event: FrontendEvent = serde_json::from_str(
            r#"{"type":"toast","payload":{"severity":"info","message":"saved"}}"#,
        )
        .unwrap();
        assert!(matches!(event, FrontendEvent::Toast(_)));

        // A minimal run snapshot (an earlier writer, or a hand-crafted test
        // fixture) fills every additive field with its default.
        let run: WorkflowRunDto =
            serde_json::from_str(r#"{"run_id":"run-1","name":"review","status":"running"}"#)
                .unwrap();
        assert_eq!(run.status, WorkflowStatus::Running);
        assert!(run.phases.is_empty());
        assert!(run.steps.is_empty());
        assert!(run.reports.is_empty());
        assert!(run.artifacts.is_empty());
        assert_eq!(run.tokens, 0);
        assert_eq!(run.cost_usd, 0.0);
        assert_eq!(run.max_cost_usd, 5.0);
        assert!(run.error.is_none());

        // The confirmation spec's command set and cap are defaulted too.
        let spec: WorkflowSpecDto =
            serde_json::from_str(r#"{"name":"review","script":"phase('scan')"}"#).unwrap();
        assert!(spec.commands.is_empty());
        assert_eq!(spec.max_cost_usd, 5.0);
    }

    #[test]
    fn p8_workflow_resume_flag_defaults_to_false_on_old_json() {
        let legacy: FrontendCommand = serde_json::from_str(
            r#"{"type":"workflow_resume","payload":{"request_id":"request-1","run_id":"run-1"}}"#,
        )
        .unwrap();
        let FrontendCommand::WorkflowResume {
            allow_incomplete_asks,
            request_id,
            run_id,
            acknowledge_partial_effects,
            ..
        } = legacy
        else {
            panic!("expected a resume command");
        };
        assert!(!allow_incomplete_asks);
        assert!(!acknowledge_partial_effects);

        // Resume carries no budget field: the confirmed grant is replayed, and
        // a budget-exhausted run needs a new run, never a silent raise.
        let encoded = serde_json::to_value(FrontendCommand::WorkflowResume {
            request_id,
            run_id,
            allow_incomplete_asks,
            acknowledge_partial_effects,
        })
        .unwrap();
        assert!(
            encoded["payload"].get("max_cost_usd").is_none(),
            "resume must not accept a replacement budget: {encoded}"
        );

        // A pre-history persisted run still loads: every additive field falls
        // to its empty/default value rather than failing the bootstrap.
        let run: WorkflowPersistedRunDto = serde_json::from_str(
            r#"{"run_id":"run-1","name":"review","status":"failed","resumable":true}"#,
        )
        .unwrap();
        assert!(run.phases.is_empty());
        assert!(run.steps.is_empty());
        assert!(run.reports.is_empty());
        assert!(run.artifacts.is_empty());
        assert_eq!(run.tokens, 0);
        assert_eq!(run.cost_usd, 0.0);
        assert!(run.detail.is_none());
        assert!(!run.partial_effects);
    }

    #[test]
    fn p9b_plan_and_todo_wire_is_additive_and_round_trips() {
        // A plan approval card carries the steps where the user decides.
        let card = ApprovalNeededDto {
            request_id: rid("request-1"),
            session_id: sid("session-1"),
            turn_id: tid("turn-1"),
            approval_id: ApprovalId::new("approval-1").unwrap(),
            tool_name: "exit_plan_mode".into(),
            path_or_command: "Make the change".into(),
            diff_or_input: None,
            workflow: None,
            plan: Some(PlanApprovalDto {
                summary: Some("Make the change".into()),
                steps: vec!["Inspect".into(), "Write".into()],
            }),
            prompt: None,
            subagent_id: None,
        };
        let json = serde_json::to_string(&FrontendEvent::ApprovalNeeded(card.clone())).unwrap();
        assert!(json.contains("\"plan\""));
        assert!(!json.contains("\"workflow\""));
        assert_eq!(
            serde_json::from_str::<FrontendEvent>(&json).unwrap(),
            FrontendEvent::ApprovalNeeded(card)
        );

        // A question prompt is additive too: it rides the same card and the
        // same single-use decision route, and its answer is a resolution.
        let question_card = ApprovalNeededDto {
            request_id: rid("request-1"),
            session_id: sid("session-1"),
            turn_id: tid("turn-1"),
            approval_id: ApprovalId::new("approval-1").unwrap(),
            tool_name: "ask_user_question".into(),
            path_or_command: "Which file?".into(),
            diff_or_input: None,
            workflow: None,
            plan: None,
            prompt: Some(DecisionPromptDto::Question {
                header: None,
                question: "Which file?".into(),
                options: vec![QuestionPromptOptionDto {
                    id: "src/a.rs".into(),
                    label: "src/a.rs".into(),
                    description: Some("left".into()),
                }],
                allow_free_text: true,
                free_text_placeholder: None,
            }),
            subagent_id: None,
        };
        let question_json =
            serde_json::to_string(&FrontendEvent::ApprovalNeeded(question_card.clone())).unwrap();
        assert!(question_json.contains("\"prompt\""));
        assert!(!question_json.contains("\"workflow\""));
        assert_eq!(
            serde_json::from_str::<FrontendEvent>(&question_json).unwrap(),
            FrontendEvent::ApprovalNeeded(question_card)
        );

        // Pre-9b approval_needed JSON still loads with no plan.
        let legacy: FrontendEvent = serde_json::from_str(
            r#"{"type":"approval_needed","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","approval_id":"approval-1","tool_name":"write_file","path_or_command":"a.rs"}}"#,
        )
        .unwrap();
        match legacy {
            FrontendEvent::ApprovalNeeded(dto) => assert!(dto.plan.is_none()),
            other => panic!("expected an approval card, got {other:?}"),
        }

        // The two live events and the revise decision round-trip.
        let plan_state = FrontendEvent::PlanStateChanged(PlanStateDto {
            session_id: sid("session-1"),
            phase: PlanPhaseDto::Planning,
            plan: None,
        });
        assert_eq!(
            serde_json::from_str::<FrontendEvent>(&serde_json::to_string(&plan_state).unwrap())
                .unwrap(),
            plan_state
        );
        let todos = FrontendEvent::TodosChanged(TodosDto {
            session_id: sid("session-1"),
            todos: vec![TodoDto {
                id: "todo-1".into(),
                title: "Inspect".into(),
                status: TodoStatusDto::InProgress,
            }],
        });
        assert_eq!(
            serde_json::from_str::<FrontendEvent>(&serde_json::to_string(&todos).unwrap()).unwrap(),
            todos
        );
        assert_eq!(
            serde_json::to_string(&DecisionDto::Revise).unwrap(),
            "\"revise\""
        );

        // A pre-9b session snapshot hydrates as executing with no plan and
        // no todos.
        let session: SessionDto = serde_json::from_str(
            r#"{"session_id":"session-1","title":"Old","message_ids":[],"active":false}"#,
        )
        .unwrap();
        assert_eq!(session.plan_phase, PlanPhaseDto::Executing);
        assert!(session.plan.is_none());
        assert!(session.todos.is_empty());
    }

    #[test]
    fn p8_workflow_event_order_validation_still_passes() {
        let event = EventEnvelope {
            protocol_version: PROTOCOL_VERSION,
            workspace_id: None,
            core_instance_id: cid("core-1"),
            sequence: 9,
            request_id: Some(rid("request-1")),
            session_id: None,
            payload: FrontendEvent::WorkflowFinished(workflow_finished(
                WorkflowStatus::Stopped,
                None,
            )),
        };
        assert!(validate_event_order(&event, PROTOCOL_VERSION, &cid("core-1"), Some(8)).is_ok());
        assert!(matches!(
            validate_event_order(&event, PROTOCOL_VERSION, &cid("core-1"), Some(7)),
            Err(EventValidationError::SequenceGap { .. })
        ));
        assert!(matches!(
            validate_event_order(&event, PROTOCOL_VERSION, &cid("core-1"), Some(9)),
            Err(EventValidationError::StaleSequence { .. })
        ));
    }

    #[test]
    fn p2_buffer_conflict_events_and_commands_keep_their_wire_names() {
        let conflict = FrontendEvent::BufferConflict(BufferConflictDto {
            buffer_id: bid("buffer-1"),
            path: WorkspacePath::new("src/main.rs").unwrap(),
        });
        let json = serde_json::to_value(&conflict).unwrap();
        assert_eq!(json["type"], "buffer_conflict");
        assert_eq!(json["payload"]["buffer_id"], "buffer-1");
        assert_eq!(json["payload"]["path"], "src/main.rs");

        let cleared = FrontendEvent::BufferConflictCleared(BufferConflictClearedDto {
            buffer_id: bid("buffer-1"),
            path: WorkspacePath::new("src/main.rs").unwrap(),
        });
        let json = serde_json::to_value(&cleared).unwrap();
        assert_eq!(json["type"], "buffer_conflict_cleared");

        for (command, name) in [
            (
                FrontendCommand::ReloadBufferFromDisk {
                    buffer_id: bid("buffer-1"),
                },
                "reload_buffer_from_disk",
            ),
            (
                FrontendCommand::KeepBufferEdits {
                    buffer_id: bid("buffer-1"),
                },
                "keep_buffer_edits",
            ),
        ] {
            let json = serde_json::to_value(&command).unwrap();
            assert_eq!(json["type"], name);
            assert_eq!(json["payload"]["buffer_id"], "buffer-1");
        }
    }
}
