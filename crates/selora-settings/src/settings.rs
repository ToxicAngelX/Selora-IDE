//! The `settings.json` schema and its persistence (ARCHITECTURE.md §8.1,
//! §10.7).
//!
//! The schema is versioned with a `_v` field. On read, any `_v != 1` file is
//! wiped (deleted and rewritten with defaults) and the caller gets a
//! [`SeloraError::Config`] it can toast. Every field carries a serde default
//! so old or partial files load with the documented defaults filled in.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use selora_core::SeloraError;
use serde::{Deserialize, Serialize};

/// The schema version of `settings.json` (ARCHITECTURE.md §8.1).
pub const SCHEMA_VERSION: u32 = 1;

/// Directory under `%APPDATA%` (PLAN.md §7).
pub const CONFIG_DIR: &str = "selora-ide";

/// File name inside the settings directory.
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// Environment variable that overrides the settings directory. Intended for
/// tests and sandboxed shells, so neither has to touch the real `%APPDATA%`.
/// The parallel-safe API for tests is [`Settings::load_from`] /
/// [`Settings::save_to`], which take explicit paths.
pub const SETTINGS_DIR_ENV: &str = "SELORA_SETTINGS_DIR";

// ---------------------------------------------------------------------------
// Defaults (ARCHITECTURE.md §8.1 — every value in the schema's example JSON).
// ---------------------------------------------------------------------------

fn default_version() -> u32 {
    SCHEMA_VERSION
}

fn default_font_family() -> String {
    "JetBrains Mono".into()
}

fn default_font_size() -> u32 {
    14
}

fn default_tab_size() -> u32 {
    4
}

fn default_line_numbers() -> bool {
    true
}

fn default_theme() -> String {
    "selora-dark".into()
}

fn default_sidebar_width() -> u32 {
    240
}

fn default_chat_sidebar_width() -> u32 {
    360
}

fn default_base_url() -> String {
    "https://api.selora.lol".into()
}

fn default_command_palette() -> String {
    "Ctrl+Shift+P".into()
}

fn default_fuzzy_find() -> String {
    "Ctrl+P".into()
}

fn default_toggle_chat() -> String {
    "Ctrl+L".into()
}

fn default_save() -> String {
    "Ctrl+S".into()
}

/// Inclusive ranges the desktop validates a settings patch against. Kept here
/// so the schema's documented range and the shell's check share one source.
pub const EDITOR_FONT_SIZE_MIN: u32 = 8;
pub const EDITOR_FONT_SIZE_MAX: u32 = 32;
pub const EDITOR_TAB_SIZE_MIN: u32 = 1;
pub const EDITOR_TAB_SIZE_MAX: u32 = 8;

// ---------------------------------------------------------------------------
// The schema.
// ---------------------------------------------------------------------------

/// The root of `settings.json` (ARCHITECTURE.md §8.1). `Send + Sync` (plain
/// data, no interior mutability — ARCHITECTURE.md §12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The `_v` schema-version field. Anything other than
    /// [`SCHEMA_VERSION`] is wiped on load.
    #[serde(rename = "_v", default = "default_version")]
    pub version: u32,

    #[serde(default)]
    pub editor: EditorSettings,

    #[serde(default)]
    pub ui: UiSettings,

    #[serde(default)]
    pub lsp: LspSettings,

    #[serde(default)]
    pub selora: SeloraSettings,

    #[serde(default)]
    pub keybinds: KeybindsSettings,

    /// `layout.*` (Phase Z2, docs/ui/PHASE_Z2_ARCHITECTURE.md §4.2) — the
    /// Tauri shell's workbench layout. Additive and serde-defaulted: a
    /// pre-Z2 settings.json (no `layout` key) loads unchanged with the
    /// §1.1 defaults, so `_v` stays 1. The egui app's own layout state
    /// (`ui.sidebar_width` / `ui.chat_sidebar_width`) is untouched —
    /// reconciling the two at cutover is a Z6 decision.
    #[serde(default)]
    pub layout: LayoutSettings,

    /// `agent.*` (ARCHITECTURE.md §17.3, §17.5) — the loop caps and the
    /// bash tool's shell/allowlist. Defaults are the spec's; every field
    /// is serde-defaulted so pre-Phase-5 settings.json loads unchanged.
    ///
    /// Boxed: `Settings` is cloned per effect-send and per dialog frame,
    /// and the unboxed section pushed `Settings` past clippy's
    /// large-enum-variant threshold for `Effect::SettingsSave` /
    /// `AppMessage::SettingsLoaded` (serde treats the box as
    /// transparent — the JSON is identical).
    #[serde(default)]
    pub agent: Box<AgentSettings>,
}

/// `editor.*` (ARCHITECTURE.md §8.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorSettings {
    #[serde(default = "default_font_family")]
    pub font_family: String,

    #[serde(default = "default_font_size")]
    pub font_size: u32,

    #[serde(default = "default_tab_size")]
    pub tab_size: u32,

    #[serde(default = "default_line_numbers")]
    pub line_numbers: bool,

    #[serde(default)]
    pub minimap: bool,

    #[serde(default)]
    pub word_wrap: bool,
}

/// `ui.*` (ARCHITECTURE.md §8.1). Only `"selora-dark"` is honored in Lite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default = "default_theme")]
    pub theme: String,

    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: u32,

    #[serde(default = "default_chat_sidebar_width")]
    pub chat_sidebar_width: u32,

    /// DESIGN_TOKENS.md §7.4: `orbit`/`pulse`/`wave`/`spin`/`comet` become
    /// static under reduced motion (`fade`/`bloom` stay). Default off.
    #[serde(default)]
    pub reduce_motion: bool,
}

/// `lsp.*` (ARCHITECTURE.md §8.1) — `None` means "resolve from PATH".
///
/// `*_args` replace the registry's built-in arguments only when non-empty
/// (an empty list keeps the default, e.g. TypeScript's `--stdio`). `enabled`
/// is the master switch: false starts no language server and makes an LSP
/// restart answer the typed `lsp_disabled` error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LspSettings {
    #[serde(default)]
    pub rust_analyzer_path: Option<String>,

    #[serde(default)]
    pub typescript_server_path: Option<String>,

    #[serde(default)]
    pub rust_analyzer_args: Vec<String>,

    #[serde(default)]
    pub typescript_server_args: Vec<String>,

    #[serde(default = "default_lsp_enabled")]
    pub enabled: bool,
}

fn default_lsp_enabled() -> bool {
    true
}

impl Default for LspSettings {
    fn default() -> Self {
        Self {
            rust_analyzer_path: None,
            typescript_server_path: None,
            rust_analyzer_args: Vec::new(),
            typescript_server_args: Vec::new(),
            enabled: default_lsp_enabled(),
        }
    }
}

/// `selora.*` (ARCHITECTURE.md §8.1). The API key is NOT here — it lives in
/// the OS keyring (Phase 4) and never in a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeloraSettings {
    #[serde(default = "default_base_url")]
    pub base_url: String,

    #[serde(default)]
    pub active_model: Option<String>,
}

/// `keybinds.*` (ARCHITECTURE.md §8.1) — string values like
/// `"Ctrl+Shift+P"`, parsed by [`crate::Keybind::parse`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeybindsSettings {
    #[serde(default = "default_command_palette")]
    pub command_palette: String,

    #[serde(default = "default_fuzzy_find")]
    pub fuzzy_find: String,

    #[serde(default = "default_toggle_chat")]
    pub toggle_chat: String,

    #[serde(default = "default_save")]
    pub save: String,
}

/// `agent.*` (ARCHITECTURE.md §17.3, §17.5) — the agent-loop caps and
/// the bash tool's shell + auto-run allowlist. 5c reads the caps, 5b
/// reads the shell/allowlist; both land in Phase 5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSettings {
    /// Max tool iterations per turn (§17.3: default 25).
    #[serde(default = "default_agent_max_iterations")]
    pub max_iterations: u32,

    /// Max wall-clock seconds per turn (§17.3: default 600 = 10 min).
    #[serde(default = "default_agent_max_turn_secs")]
    pub max_turn_secs: u64,

    /// Max estimated USD cost per turn (§17.3: default 1.00).
    #[serde(default = "default_agent_max_cost_usd")]
    pub max_cost_usd: f64,

    /// The bash shell (§17.5: `pwsh` if installed, else `powershell` —
    /// the default here is resolved to `pwsh` when present).
    #[serde(default = "default_agent_shell")]
    pub shell: String,

    /// Read-only commands that auto-run without a card (§17.5's list;
    /// prefix match, user-editable).
    #[serde(default = "default_agent_auto_approve_commands")]
    pub auto_approve_commands: Vec<String>,

    /// §17.1 `web_search`: the optional Brave Search API key (DuckDuckGo
    /// Lite is the no-key default).
    ///
    /// Posture (owner-signed-off, Phase 5): this is plaintext at rest in
    /// settings.json — the user's chosen configuration file (the VS Code
    /// extension-key posture). A `skip_serializing` field would be
    /// silently deleted by the next unrelated `SettingsSave` (the shell's
    /// SelectModel route clones and saves the whole `Settings`), and
    /// keyring-only storage would make the feature unconfigurable today
    /// (no settings-dialog field exists for it). The no-echo half of
    /// §17.2 is the invariant to protect: the key must never land in
    /// workspace.json, logs, tool results, or the bash environment, and
    /// the router-side copy (`AgentToolSettings::brave_api_key`) stays
    /// `skip_serializing` (router.rs, frozen) — its only sink is the
    /// `X-Subscription-Token` header.
    ///
    /// Follow-up (post-Phase-5, owner decision): OS keyring storage like
    /// the Selora key, plus a settings-dialog field so it stays
    /// configurable.
    #[serde(default)]
    pub brave_search_api_key: Option<String>,
}

/// `layout.*` — the Tauri shell's workbench layout (Phase Z2, docs/ui/
/// PHASE_Z2_ARCHITECTURE.md §4.2). Global in Z2; the workspace-picker slice
/// adds per-workspace overrides (§4.5 of that contract). Every field is
/// serde-defaulted so a pre-Z2 settings.json loads unchanged, and the
/// defaults delegate to the protocol's §1.1 constants, which are the single
/// source of truth. The JSON spellings are the protocol's snake_case wire
/// names (`"bottom_view": "output"`), so the file and the wire share one
/// vocabulary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutSettings {
    #[serde(default = "default_primary_sidebar_width_px")]
    pub primary_sidebar_width_px: f32,

    #[serde(default = "default_agent_dock_width_px")]
    pub agent_dock_width_px: f32,

    #[serde(default = "default_agent_dock_open")]
    pub agent_dock_open: bool,

    #[serde(default = "default_bottom_dock_height_px")]
    pub bottom_dock_height_px: f32,

    #[serde(default)]
    pub bottom_dock_open: bool,

    #[serde(default = "default_bottom_view")]
    pub bottom_view: selora_protocol::SurfaceId,

    #[serde(default = "default_editor_splits")]
    pub editor_splits: u8,

    #[serde(default = "default_activity_view")]
    pub activity_view: selora_protocol::SurfaceId,

    #[serde(default = "default_primary_sidebar_open")]
    pub primary_sidebar_open: bool,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            primary_sidebar_width_px: default_primary_sidebar_width_px(),
            agent_dock_width_px: default_agent_dock_width_px(),
            agent_dock_open: default_agent_dock_open(),
            bottom_dock_height_px: default_bottom_dock_height_px(),
            bottom_dock_open: false,
            bottom_view: default_bottom_view(),
            editor_splits: default_editor_splits(),
            activity_view: default_activity_view(),
            primary_sidebar_open: default_primary_sidebar_open(),
        }
    }
}

/// The load direction: the persisted section as a protocol DTO. The desktop
/// state sanitizes it (protocol §4.3) before use.
impl From<LayoutSettings> for selora_protocol::LayoutDto {
    fn from(settings: LayoutSettings) -> Self {
        Self {
            primary_sidebar_width_px: settings.primary_sidebar_width_px,
            agent_dock_width_px: settings.agent_dock_width_px,
            agent_dock_open: settings.agent_dock_open,
            bottom_dock_height_px: settings.bottom_dock_height_px,
            bottom_dock_open: settings.bottom_dock_open,
            bottom_view: settings.bottom_view,
            editor_splits: settings.editor_splits,
            activity_view: settings.activity_view,
            primary_sidebar_open: settings.primary_sidebar_open,
        }
    }
}

/// The save direction: the authoritative in-memory DTO as the persisted
/// section.
impl From<selora_protocol::LayoutDto> for LayoutSettings {
    fn from(layout: selora_protocol::LayoutDto) -> Self {
        Self {
            primary_sidebar_width_px: layout.primary_sidebar_width_px,
            agent_dock_width_px: layout.agent_dock_width_px,
            agent_dock_open: layout.agent_dock_open,
            bottom_dock_height_px: layout.bottom_dock_height_px,
            bottom_dock_open: layout.bottom_dock_open,
            bottom_view: layout.bottom_view,
            editor_splits: layout.editor_splits,
            activity_view: layout.activity_view,
            primary_sidebar_open: layout.primary_sidebar_open,
        }
    }
}

// --- layout defaults: delegates to the protocol's §1.1 constants (§4.2), so
// the serde defaults, `LayoutSettings::default()`, and the constants can
// never diverge. The open-state literals are §1.1's first-run column. ---

fn default_primary_sidebar_width_px() -> f32 {
    selora_protocol::PRIMARY_SIDEBAR_DEFAULT_PX
}

fn default_agent_dock_width_px() -> f32 {
    selora_protocol::AGENT_DOCK_DEFAULT_PX
}

fn default_agent_dock_open() -> bool {
    // §1.1's first-run state: closed.
    false
}

fn default_bottom_dock_height_px() -> f32 {
    selora_protocol::BOTTOM_DOCK_DEFAULT_PX
}

fn default_bottom_view() -> selora_protocol::SurfaceId {
    selora_protocol::SurfaceId::Output
}

fn default_editor_splits() -> u8 {
    selora_protocol::EDITOR_SPLITS_MIN
}

fn default_activity_view() -> selora_protocol::SurfaceId {
    selora_protocol::SurfaceId::Files
}

fn default_primary_sidebar_open() -> bool {
    // §1.1's first-run state: open.
    true
}

fn default_agent_max_iterations() -> u32 {
    25
}

fn default_agent_max_turn_secs() -> u64 {
    600
}

fn default_agent_max_cost_usd() -> f64 {
    1.0
}

fn default_agent_shell() -> String {
    "powershell".into()
}

fn default_agent_auto_approve_commands() -> Vec<String> {
    [
        "git status",
        "git diff",
        "git log",
        "git show",
        "ls",
        "dir",
        "cat",
        "type",
        "Get-Content",
        "Get-ChildItem",
        "cargo check",
        "cargo build",
        "cargo test",
        "cargo clippy",
        "cargo tree",
        "node --version",
        "rustc --version",
        "where",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            editor: EditorSettings::default(),
            ui: UiSettings::default(),
            lsp: LspSettings::default(),
            selora: SeloraSettings::default(),
            keybinds: KeybindsSettings::default(),
            layout: LayoutSettings::default(),
            agent: Box::new(AgentSettings::default()),
        }
    }
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size: default_font_size(),
            tab_size: default_tab_size(),
            line_numbers: default_line_numbers(),
            minimap: false,
            word_wrap: false,
        }
    }
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            sidebar_width: default_sidebar_width(),
            chat_sidebar_width: default_chat_sidebar_width(),
            reduce_motion: false,
        }
    }
}

impl Default for SeloraSettings {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            active_model: None,
        }
    }
}

impl Default for KeybindsSettings {
    fn default() -> Self {
        Self {
            command_palette: default_command_palette(),
            fuzzy_find: default_fuzzy_find(),
            toggle_chat: default_toggle_chat(),
            save: default_save(),
        }
    }
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            max_iterations: default_agent_max_iterations(),
            max_turn_secs: default_agent_max_turn_secs(),
            max_cost_usd: default_agent_max_cost_usd(),
            shell: default_agent_shell(),
            auto_approve_commands: default_agent_auto_approve_commands(),
            brave_search_api_key: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence (ARCHITECTURE.md §10.7).
// ---------------------------------------------------------------------------

impl Settings {
    /// Load from `%APPDATA%\selora-ide\settings.json`.
    ///
    /// A missing file yields the defaults (first run). A `_v` mismatch wipes
    /// the file (see [`Settings::load_from`]) and returns the `Config` error
    /// the caller should toast.
    pub fn load() -> Result<Settings, SeloraError> {
        Settings::load_from(&Settings::default_path())
    }

    /// Load from an explicit path. Hermetic — this is the variant tests (and
    /// any sandboxed shell) use so they never touch the real `%APPDATA%`.
    pub fn load_from(path: &Path) -> Result<Settings, SeloraError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(e) => return Err(SeloraError::Io(e)),
        };

        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| SeloraError::Config(format!("settings file is not valid JSON: {e}")))?;

        let version = value
            .get("_v")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if version != u64::from(SCHEMA_VERSION) {
            // ARCHITECTURE.md §8.1: "run a migration (or wipe with a toast)".
            // We wipe: delete the unusable file and rewrite the defaults so
            // the next load is clean, then surface the toast-able error.
            let _ = std::fs::remove_file(path);
            let _ = Settings::default().save_to(path);
            return Err(SeloraError::Config(format!(
                "settings schema version {version} is unsupported; settings were wiped and reset to defaults"
            )));
        }

        serde_json::from_value(value).map_err(|e| {
            SeloraError::Config(format!("settings file does not match the schema: {e}"))
        })
    }

    /// Load, falling back to defaults on any error while keeping the error
    /// for the caller to toast. The one-call version of
    /// `match Settings::load() { Ok(s) => (s, None), Err(e) => (Settings::default(), Some(e)) }`.
    pub fn load_or_default() -> (Settings, Option<SeloraError>) {
        match Settings::load() {
            Ok(settings) => (settings, None),
            Err(err) => (Settings::default(), Some(err)),
        }
    }

    /// Save to `%APPDATA%\selora-ide\settings.json` (atomic).
    pub fn save(&self) -> Result<(), SeloraError> {
        self.save_to(&Settings::default_path())
    }

    /// Save to an explicit path, atomically: write `settings.json.tmp` next
    /// to the target, then rename over it (same directory, so the rename
    /// never crosses a volume boundary). The directory is created if
    /// missing. No `.tmp` file is left behind on failure.
    pub fn save_to(&self, path: &Path) -> Result<(), SeloraError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| SeloraError::Config(format!("failed to serialize settings: {e}")))?;

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let mut tmp = OsString::from(path.as_os_str());
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);

        std::fs::write(&tmp, json.as_bytes())?;
        match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Never leave the temp file behind on a failed rename.
                let _ = std::fs::remove_file(&tmp);
                Err(SeloraError::Io(e))
            }
        }
    }

    /// `%APPDATA%\selora-ide\settings.json` (PLAN.md §7). Honors
    /// [`SETTINGS_DIR_ENV`] as an override. When neither is set this returns
    /// the relative fallback `selora-ide/settings.json` rather than panic;
    /// [`Settings::load`] / [`Settings::save`] route through
    /// [`settings_dir`] and error cleanly instead of ever writing there.
    pub fn default_path() -> PathBuf {
        match settings_dir() {
            Ok(dir) => dir.join(SETTINGS_FILE_NAME),
            Err(_) => PathBuf::from(CONFIG_DIR).join(SETTINGS_FILE_NAME),
        }
    }

    /// The directory holding `settings.json`:
    /// `$SELORA_SETTINGS_DIR` if set, else `%APPDATA%\selora-ide`. Errors
    /// cleanly when APPDATA is not set and no override is configured.
    pub fn settings_dir() -> Result<PathBuf, SeloraError> {
        settings_dir()
    }
}

/// Pure resolution of the settings directory, parameterized for tests (env
/// vars are process-global and tests run in parallel threads, so the pure
/// form takes both inputs explicitly).
fn settings_dir_from(
    override_dir: Option<String>,
    appdata: Option<String>,
) -> Result<PathBuf, SeloraError> {
    // Test / embedding override wins over %APPDATA%.
    if let Some(dir) = override_dir {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    appdata
        .map(|base| Path::new(&base).join(CONFIG_DIR))
        .ok_or_else(|| {
            SeloraError::Config(format!(
                "the APPDATA environment variable is not set; cannot locate the {CONFIG_DIR} settings directory"
            ))
        })
}

fn settings_dir() -> Result<PathBuf, SeloraError> {
    settings_dir_from(
        std::env::var(SETTINGS_DIR_ENV).ok(),
        std::env::var("APPDATA").ok(),
    )
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

    /// A unique temp directory per test (tests run in parallel threads; env
    /// vars are process-global, so the path-parameterized API is used
    /// instead of mutating APPDATA).
    pub(crate) fn temp_settings_dir(label: &str) -> PathBuf {
        let n = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "selora-settings-test-{label}-{}-{}",
            std::process::id(),
            n
        ))
    }

    #[test]
    fn defaults_match_the_schema_doc() {
        let s = Settings::default();
        assert_eq!(s.version, 1);
        assert_eq!(s.editor.font_family, "JetBrains Mono");
        assert_eq!(s.editor.font_size, 14);
        assert_eq!(s.editor.tab_size, 4);
        assert!(s.editor.line_numbers);
        assert!(!s.editor.minimap);
        assert!(!s.editor.word_wrap);
        assert_eq!(s.ui.theme, "selora-dark");
        assert_eq!(s.ui.sidebar_width, 240);
        assert_eq!(s.ui.chat_sidebar_width, 360);
        assert_eq!(s.lsp.rust_analyzer_path, None);
        assert_eq!(s.lsp.typescript_server_path, None);
        assert!(s.lsp.rust_analyzer_args.is_empty());
        assert!(s.lsp.typescript_server_args.is_empty());
        assert!(s.lsp.enabled, "language servers default on");
        assert_eq!(s.selora.base_url, "https://api.selora.lol");
        assert_eq!(s.selora.active_model, None);
        assert_eq!(s.keybinds.command_palette, "Ctrl+Shift+P");
        assert_eq!(s.keybinds.fuzzy_find, "Ctrl+P");
        assert_eq!(s.keybinds.toggle_chat, "Ctrl+L");
        assert_eq!(s.keybinds.save, "Ctrl+S");
        assert!(!s.ui.reduce_motion);
        assert_eq!(s.agent.brave_search_api_key, None);
    }

    #[test]
    fn round_trip_save_then_load_is_equal() {
        let dir = temp_settings_dir("round-trip");
        let path = dir.join(SETTINGS_FILE_NAME);
        let mut settings = Settings::default();
        settings.editor.font_size = 18;
        settings.lsp.rust_analyzer_path = Some("C:/tools/rust-analyzer.exe".into());
        settings.lsp.rust_analyzer_args = vec!["--log-file".into(), "ra.log".into()];
        settings.lsp.typescript_server_args = vec!["--stdio".into()];
        settings.lsp.enabled = false;
        settings.selora.active_model = Some("glm-5.3".into());
        // Phase 5d: the agent Brave key and the reduced-motion flag.
        settings.agent.brave_search_api_key = Some("brave-test-key".into());
        settings.ui.reduce_motion = true;

        settings.save_to(&path).expect("save");
        assert!(path.is_file(), "settings file must exist after save");
        // Atomic write: no temp file left behind.
        assert!(!dir.join("settings.json.tmp").is_file());

        let loaded = Settings::load_from(&path).expect("load");
        assert_eq!(loaded, settings);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// P1 settings slice: the editor preferences and all four keybinds
    /// round-trip through the real file, preserving every other section.
    #[test]
    fn editor_preferences_and_keybinds_round_trip() {
        let dir = temp_settings_dir("editor-keybinds");
        let path = dir.join(SETTINGS_FILE_NAME);
        let mut settings = Settings::default();
        settings.editor.font_size = 20;
        settings.editor.tab_size = 2;
        settings.editor.word_wrap = true;
        settings.keybinds.save = "Ctrl+Alt+S".into();
        settings.keybinds.fuzzy_find = "Ctrl+Shift+F".into();
        settings.keybinds.command_palette = "F2".into();
        settings.keybinds.toggle_chat = "Alt+L".into();
        settings.ui.reduce_motion = true;
        settings.ui.theme = "light".into();

        settings.save_to(&path).expect("save");
        let loaded = Settings::load_from(&path).expect("load");
        assert_eq!(loaded, settings);
        assert_eq!(loaded.editor.font_size, 20);
        assert_eq!(loaded.editor.tab_size, 2);
        assert!(loaded.editor.word_wrap);
        assert_eq!(loaded.keybinds.save, "Ctrl+Alt+S");
        assert_eq!(loaded.keybinds.fuzzy_find, "Ctrl+Shift+F");
        assert_eq!(loaded.keybinds.command_palette, "F2");
        assert_eq!(loaded.keybinds.toggle_chat, "Alt+L");
        // Unrelated sections survive the whole-file rewrite.
        assert_eq!(loaded.agent.max_iterations, 25);
        assert_eq!(loaded.layout, LayoutSettings::default());

        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains(r#""font_size": 20"#));
        assert!(text.contains(r#""word_wrap": true"#));
        assert!(text.contains(r#""save": "Ctrl+Alt+S""#));

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The published validation ranges are the documented ones and do not
    /// overlap inverted (the desktop uses them for every patch).
    #[test]
    fn editor_range_constants_are_sane() {
        assert_eq!((EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX), (8, 32));
        assert_eq!((EDITOR_TAB_SIZE_MIN, EDITOR_TAB_SIZE_MAX), (1, 8));
        assert!((EDITOR_FONT_SIZE_MIN..=EDITOR_FONT_SIZE_MAX).contains(&14));
        assert!((EDITOR_TAB_SIZE_MIN..=EDITOR_TAB_SIZE_MAX).contains(&4));
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = temp_settings_dir("atomic-replace");
        let path = dir.join(SETTINGS_FILE_NAME);

        let mut first = Settings::default();
        first.editor.font_size = 20;
        first.save_to(&path).expect("first save");

        let mut second = Settings::default();
        second.editor.font_size = 12;
        second
            .save_to(&path)
            .expect("second save (rename over existing)");

        assert!(
            !dir.join("settings.json.tmp").is_file(),
            "no temp file left behind"
        );
        assert_eq!(
            Settings::load_from(&path).expect("load").editor.font_size,
            12
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_creates_missing_directory() {
        let dir = temp_settings_dir("create-dir");
        let path = dir.join("nested").join("deeper").join(SETTINGS_FILE_NAME);
        Settings::default()
            .save_to(&path)
            .expect("save must create the directory");
        assert!(path.is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn partial_file_loads_with_defaults() {
        let dir = temp_settings_dir("partial");
        let path = dir.join(SETTINGS_FILE_NAME);
        std::fs::create_dir_all(&dir).expect("mkdir");
        // Old/partial file: `_v` present, most fields missing.
        std::fs::write(
            &path,
            br#"{"_v":1,"editor":{"font_size":18},"ui":{"theme":"selora-dark"}}"#,
        )
        .expect("write partial file");

        let s = Settings::load_from(&path).expect("partial file must load");
        assert_eq!(s.version, 1);
        assert_eq!(s.editor.font_size, 18);
        // Everything else falls back to the documented defaults.
        assert_eq!(s.editor.font_family, "JetBrains Mono");
        assert_eq!(s.editor.tab_size, 4);
        assert!(s.editor.line_numbers);
        assert_eq!(s.ui.sidebar_width, 240);
        assert_eq!(s.selora.base_url, "https://api.selora.lol");
        assert_eq!(s.keybinds.save, "Ctrl+S");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Phase 5d: a pre-5d settings.json (no `agent.brave_search_api_key`,
    /// no `ui.reduce_motion`) deserializes with `None` / `false`.
    #[test]
    fn pre_5d_file_loads_with_agent_and_motion_defaults() {
        let dir = temp_settings_dir("pre-5d");
        let path = dir.join(SETTINGS_FILE_NAME);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            &path,
            br#"{"_v":1,"agent":{"max_iterations":3},"ui":{"theme":"selora-dark"}}"#,
        )
        .expect("write pre-5d file");

        let s = Settings::load_from(&path).expect("pre-5d file must load");
        assert_eq!(s.agent.max_iterations, 3, "the present field survives");
        assert_eq!(s.agent.brave_search_api_key, None);
        assert!(!s.ui.reduce_motion);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Phase Z2 (§10): `LayoutSettings::default()` equals the protocol §1.1
    /// values exactly (240 / 360-closed / 200-closed) — the divergence guard
    /// between the serde defaults, the `Default` impl, and the §1.1 constants
    /// (tokens.css intentionally carries no region values).
    #[test]
    fn layout_settings_default_equals_the_protocol_constants() {
        let layout = LayoutSettings::default();
        assert_eq!(
            layout.primary_sidebar_width_px,
            selora_protocol::PRIMARY_SIDEBAR_DEFAULT_PX
        );
        assert_eq!(
            layout.agent_dock_width_px,
            selora_protocol::AGENT_DOCK_DEFAULT_PX
        );
        assert!(!layout.agent_dock_open, "agent dock closed first-run");
        assert_eq!(
            layout.bottom_dock_height_px,
            selora_protocol::BOTTOM_DOCK_DEFAULT_PX
        );
        assert!(!layout.bottom_dock_open, "bottom dock closed first-run");
        assert_eq!(layout.bottom_view, selora_protocol::SurfaceId::Output);
        assert_eq!(layout.editor_splits, selora_protocol::EDITOR_SPLITS_MIN);
        assert_eq!(layout.activity_view, selora_protocol::SurfaceId::Files);
        assert!(layout.primary_sidebar_open, "sidebar open first-run");
        // The table, verbatim, so a constant drift fails here too.
        assert_eq!(layout.primary_sidebar_width_px, 240.0);
        assert_eq!(layout.agent_dock_width_px, 360.0);
        assert_eq!(layout.bottom_dock_height_px, 200.0);
    }

    /// Phase Z2 (§10): the `layout` section round-trips through the file, and
    /// a save preserves all other sections.
    #[test]
    fn layout_section_round_trips_and_preserves_other_sections() {
        let dir = temp_settings_dir("z2-round-trip");
        let path = dir.join(SETTINGS_FILE_NAME);

        let mut settings = Settings::default();
        settings.editor.font_size = 18;
        settings.agent.max_iterations = 7;
        settings.ui.reduce_motion = true;
        settings.layout = LayoutSettings {
            primary_sidebar_width_px: 411.5,
            agent_dock_width_px: 500.25,
            agent_dock_open: true,
            bottom_dock_height_px: 640.0,
            bottom_dock_open: true,
            bottom_view: selora_protocol::SurfaceId::Problems,
            editor_splits: 2,
            activity_view: selora_protocol::SurfaceId::Git,
            primary_sidebar_open: false,
        };

        settings.save_to(&path).expect("save");
        let loaded = Settings::load_from(&path).expect("load");
        assert_eq!(loaded.layout, settings.layout, "layout round-trips");
        assert_eq!(loaded.editor.font_size, 18, "editor section survives");
        assert_eq!(loaded.agent.max_iterations, 7, "agent section survives");
        assert!(loaded.ui.reduce_motion, "ui section survives");
        assert_eq!(loaded.keybinds.toggle_chat, "Ctrl+L", "keybinds survive");

        // The on-disk spellings are the protocol's wire names.
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains(r#""bottom_view": "problems""#));
        assert!(text.contains(r#""activity_view": "git""#));

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Phase Z2 (§10): the ARCHITECTURE.md §8.1 example file (no `layout`
    /// key) loads unchanged with the §1.1 layout defaults.
    #[test]
    fn pre_z2_architect_example_file_loads_with_layout_defaults() {
        let dir = temp_settings_dir("z2-pre-z2-file");
        let path = dir.join(SETTINGS_FILE_NAME);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            &path,
            r#"{
  "_v": 1,
  "editor": {
    "font_family": "JetBrains Mono",
    "font_size": 14,
    "tab_size": 4,
    "line_numbers": true,
    "minimap": false,
    "word_wrap": false
  },
  "ui": {
    "theme": "selora-dark",
    "sidebar_width": 240,
    "chat_sidebar_width": 360
  },
  "lsp": {
    "rust_analyzer_path": null,
    "typescript_server_path": null
  },
  "selora": {
    "base_url": "https://api.selora.lol",
    "active_model": null
  },
  "keybinds": {
    "command_palette": "Ctrl+Shift+P",
    "fuzzy_find": "Ctrl+P",
    "toggle_chat": "Ctrl+L",
    "save": "Ctrl+S"
  }
}"#,
        )
        .expect("write pre-Z2 file");

        let s = Settings::load_from(&path).expect("pre-Z2 file must load");
        assert_eq!(s.layout, LayoutSettings::default());
        assert_eq!(s.ui.sidebar_width, 240, "legacy egui keys untouched");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Phase Z2: a partial `layout` section fills the missing fields with the
    /// serde defaults (the additive-sections compatibility mechanism §4.2
    /// cites, settings.rs' own §6 header note).
    #[test]
    fn partial_layout_section_loads_with_defaults() {
        let dir = temp_settings_dir("z2-partial-layout");
        let path = dir.join(SETTINGS_FILE_NAME);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            &path,
            br#"{"_v":1,"layout":{"primary_sidebar_width_px":300.0,"agent_dock_open":true}}"#,
        )
        .expect("write partial layout file");

        let s = Settings::load_from(&path).expect("partial layout must load");
        assert_eq!(s.layout.primary_sidebar_width_px, 300.0);
        assert!(s.layout.agent_dock_open, "the present field survives");
        assert_eq!(s.layout.agent_dock_width_px, 360.0);
        assert_eq!(s.layout.bottom_dock_height_px, 200.0);
        assert!(!s.layout.bottom_dock_open);
        assert_eq!(s.layout.activity_view, selora_protocol::SurfaceId::Files);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Phase Z2: the protocol conversions are field-for-field in both
    /// directions (the §4.5 `LayoutStore` seam's building blocks).
    #[test]
    fn layout_settings_and_layout_dto_convert_losslessly() {
        let settings = LayoutSettings {
            primary_sidebar_width_px: 411.5,
            agent_dock_width_px: 500.25,
            agent_dock_open: true,
            bottom_dock_height_px: 640.0,
            bottom_dock_open: true,
            bottom_view: selora_protocol::SurfaceId::TerminalPlaceholder,
            editor_splits: 4,
            activity_view: selora_protocol::SurfaceId::Search,
            primary_sidebar_open: false,
        };
        let dto: selora_protocol::LayoutDto = settings.clone().into();
        let back: LayoutSettings = dto.into();
        assert_eq!(settings, back);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let dir = temp_settings_dir("missing");
        let path = dir.join(SETTINGS_FILE_NAME);
        let s = Settings::load_from(&path).expect("missing file = first run = defaults");
        assert_eq!(s, Settings::default());
        assert!(!path.exists(), "load must not create the file");
    }

    #[test]
    fn version_mismatch_wipes_and_errors() {
        let dir = temp_settings_dir("version-wipe");
        let path = dir.join(SETTINGS_FILE_NAME);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(&path, br#"{"_v":2,"editor":{"font_size":99}}"#).expect("write v2 file");

        let err = Settings::load_from(&path).expect_err("v2 must be rejected");
        assert!(
            matches!(&err, SeloraError::Config(m) if m.contains("wiped")),
            "expected a toast-able Config error, got: {err:?}"
        );

        // Wiped: the file on disk is now the defaults, with `_v: 1`.
        let reloaded = Settings::load_from(&path).expect("reload after wipe");
        assert_eq!(reloaded, Settings::default());
        let text = std::fs::read_to_string(&path).expect("file rewritten");
        assert!(text.contains("\"_v\": 1"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn file_without_version_is_wiped() {
        let dir = temp_settings_dir("no-version");
        let path = dir.join(SETTINGS_FILE_NAME);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(&path, br#"{"editor":{"font_size":99}}"#).expect("write versionless file");

        assert!(Settings::load_from(&path).is_err());
        assert_eq!(
            Settings::load_from(&path).expect("reload"),
            Settings::default()
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_json_errors_cleanly() {
        let dir = temp_settings_dir("bad-json");
        let path = dir.join(SETTINGS_FILE_NAME);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(&path, b"{ not json").expect("write garbage");

        let err = Settings::load_from(&path).expect_err("garbage must error");
        assert!(matches!(err, SeloraError::Config(_)), "got: {err:?}");
        // The garbage file is left in place (it is not a version mismatch).
        assert!(path.is_file());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pre_phase5_file_loads_with_agent_defaults() {
        // A Phase 4 settings.json has no `agent` key — it must load with
        // the §17.3/§17.5 defaults, not error.
        let dir = temp_settings_dir("pre-phase5");
        let path = dir.join(SETTINGS_FILE_NAME);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let json = serde_json::json!({
            "_v": SCHEMA_VERSION,
            "editor": { "font_size": 15 },
            "selora": { "base_url": "https://api.selora.lol" }
        });
        std::fs::write(&path, serde_json::to_string(&json).unwrap()).expect("write");

        let settings = Settings::load_from(&path).expect("load");
        assert_eq!(settings.editor.font_size, 15);
        assert_eq!(settings.agent.max_iterations, 25);
        assert_eq!(settings.agent.max_turn_secs, 600);
        assert!((settings.agent.max_cost_usd - 1.0).abs() < f64::EPSILON);
        assert_eq!(settings.agent.shell, "powershell");
        assert!(settings
            .agent
            .auto_approve_commands
            .contains(&"cargo check".to_owned()));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_appdata_errors_cleanly() {
        // The pure resolver: no override, no APPDATA → a Config error, not a
        // panic and not a silently-wrong path.
        let err = settings_dir_from(None, None).expect_err("no APPDATA must error");
        assert!(
            matches!(err, SeloraError::Config(ref m) if m.contains("APPDATA")),
            "got: {err:?}"
        );
        // ...and with APPDATA set, the documented directory resolves.
        let dir =
            settings_dir_from(None, Some(r"C:\Users\x\AppData\Roaming".into())).expect("resolves");
        assert_eq!(
            dir,
            PathBuf::from(r"C:\Users\x\AppData\Roaming").join(CONFIG_DIR)
        );
        // The override wins over APPDATA.
        let overridden =
            settings_dir_from(Some(r"C:\tmp\sandbox".into()), Some(r"C:\Users\x".into()))
                .expect("override resolves");
        assert_eq!(overridden, PathBuf::from(r"C:\tmp\sandbox"));
    }

    #[test]
    fn load_or_default_keeps_the_toastable_error() {
        let dir = temp_settings_dir("load-or-default");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join(SETTINGS_FILE_NAME), br#"{"_v":7}"#).expect("write v7 file");

        // load_or_default routes through load()/default_path(); point it at
        // the temp dir via the documented env override, then restore. No
        // other test in this crate reads the env var (they use load_from /
        // save_to), so the window is race-free.
        std::env::set_var(SETTINGS_DIR_ENV, &dir);
        let out = Settings::load_or_default();
        std::env::remove_var(SETTINGS_DIR_ENV);

        assert_eq!(out.0, Settings::default());
        assert!(
            out.1.is_some(),
            "the wipe error must be surfaced for toasting"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
