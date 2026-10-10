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
}
/// The `SendAgent` permission mode (Phase Z3). Serde-rejected spellings
/// never reach a Rust arm — the boundary is the validator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModeDto {
    #[default]
    Execute,
    Explore,
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
pub const ACTIVITY_VIEWS: [SurfaceId; 4] = [
    SurfaceId::Files,
    SurfaceId::Search,
    SurfaceId::Git,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsViewDto {
    pub reduced_motion: bool,
    pub theme: String,
    /// §7.1's rebindable pair, populated from `settings.keybinds` (§5.1) so a
    /// user's egui-side rebind carries into the web shell. The other two
    /// settings keybinds (`fuzzy_find`, `save`) are not exposed until their
    /// slices use them.
    #[serde(default = "default_keybind_command_palette")]
    pub keybind_command_palette: String,
    #[serde(default = "default_keybind_toggle_chat")]
    pub keybind_toggle_chat: String,
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
    /// §5.1: the §1.1 bounds as delivered to the frontend — no bound is ever
    /// a JS literal. No serde default: the bootstrap is always generated
    /// whole.
    pub layout_constraints: LayoutConstraintsDto,
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
/// decision reaches the turn.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
                keybind_command_palette: "Ctrl+Shift+P".into(),
                keybind_toggle_chat: "Ctrl+L".into(),
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
                },
            })
            .unwrap(),
            r#"{"type":"send_agent","payload":{"request_id":"request-1","request":{"session_id":"session-1","prompt":"fix it","model_id":null,"mode":"execute"}}}"#
        );
        assert_eq!(
            serde_json::to_string(&FrontendCommand::AgentDecision {
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
            }))
            .unwrap(),
            r#"{"type":"approval_needed","payload":{"request_id":"request-1","session_id":"session-1","turn_id":"turn-1","approval_id":"approval-1","tool_name":"bash","path_or_command":"cargo test"}}"#
        );
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
