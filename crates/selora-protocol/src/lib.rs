//! Versioned DTOs for the Selora desktop shell boundary.
//!
//! Rust owns files, buffers, gateway credentials and approvals. A frontend
//! receives a safe snapshot and ordered incremental events through this crate.

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
    IdTooLong { max: usize },
    IdContainsControlCharacter,
    InvalidExternalUrl,
    InvalidWorkspacePath,
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentRequestDto {
    pub session_id: SessionId,
    pub prompt: String,
    pub model_id: Option<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionDto {
    Approve,
    Deny,
    ApproveAllThisTurn,
    Stop,
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsPatchDto {
    pub reduced_motion: Option<bool>,
    pub theme: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum FrontendCommand {
    GetBootstrapState,
    OpenFolder,
    OpenFile {
        path: WorkspacePath,
    },
    CloseBuffer {
        buffer_id: BufferId,
    },
    SaveBuffer {
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
    },
    RetryMessage {
        session_id: SessionId,
        message_id: MessageId,
    },
    SwitchSession {
        session_id: SessionId,
    },
    NewSession,
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
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Connected,
    Connecting,
    Disconnected,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsViewDto {
    pub reduced_motion: bool,
    pub theme: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutDto {
    pub primary_sidebar_width_px: f32,
    pub agent_dock_width_px: f32,
    pub agent_dock_open: bool,
    pub bottom_dock_height_px: f32,
    pub bottom_dock_open: bool,
    pub bottom_view: SurfaceId,
    pub editor_splits: u8,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitStatusDto {
    pub branch: Option<String>,
    pub changed_paths: Vec<WorkspacePath>,
    pub staged_paths: Vec<WorkspacePath>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticDto {
    pub path: WorkspacePath,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
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
    pub git: GitStatusDto,
    pub diagnostics: Vec<DiagnosticDto>,
    pub feature_flags: FeatureFlagsDto,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatDeltaDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub message_id: MessageId,
    pub sequence: u64,
    pub text: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletedDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub message_id: MessageId,
    pub text: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatFailedDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub error: ProtocolErrorDto,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentEventDto {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub approval: Option<ApprovalDto>,
    pub message: String,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum FrontendEvent {
    BootstrapState(Box<BootstrapState>),
    ChatDelta(ChatDeltaDto),
    ChatCompleted(ChatCompletedDto),
    ChatFailed(ChatFailedDto),
    AgentEvent(AgentEventDto),
    DiagnosticsChanged(Vec<DiagnosticDto>),
    FileTreeChanged(FileTreeDto),
    BufferSnapshot(BufferSnapshotDto),
    BufferChanged(BufferChangedDto),
    SearchResults(SearchResultsDto),
    GitStatusChanged(GitStatusDto),
    GitDiffReady(GitDiffDto),
    ModelsChanged(ModelsDto),
    SeloraStatusChanged(SeloraStatusDto),
    WorkspaceLayoutChanged(LayoutDto),
    Toast(ToastDto),
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
                sessions: vec![SessionDto {
                    session_id: sid("session-1"),
                    title: "Welcome".into(),
                    message_ids: vec![],
                    active: true,
                }],
                messages: vec![],
                active_session_id: None,
                active_request_id: None,
                active_turn_id: None,
                pending_approval: None,
            },
            settings: SettingsViewDto {
                reduced_motion: false,
                theme: "selora-dark".into(),
            },
            layout: LayoutDto {
                primary_sidebar_width_px: 280.0,
                agent_dock_width_px: 360.0,
                agent_dock_open: true,
                bottom_dock_height_px: 220.0,
                bottom_dock_open: false,
                bottom_view: SurfaceId::Output,
                editor_splits: 1,
            },
            git: GitStatusDto {
                branch: None,
                changed_paths: vec![],
                staged_paths: vec![],
            },
            diagnostics: vec![],
            feature_flags: FeatureFlagsDto {
                agent: false,
                git: false,
                search: false,
                editor: true,
                later_slices: vec!["agent".into(), "lsp".into()],
            },
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
    fn approval_is_bound_and_consumed_once() {
        let mut pending = Some(ApprovalDto {
            approval_id: aid("approval"),
            request_id: rid("request"),
            turn_id: tid("turn"),
            session_id: sid("session"),
            summary: "write".into(),
            can_approve_all_this_turn: false,
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
}
