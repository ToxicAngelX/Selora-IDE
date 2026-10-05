//! Selora Settings — persistence for user configuration.
//!
//! Responsibility (PLAN.md §5, ARCHITECTURE.md §8.1 / §10.7): the
//! `settings.json` schema, read/write at `%APPDATA%\selora-ide\settings.json`
//! (atomic: temp file + rename), and the keybind parser the shell uses to
//! route shortcuts. The keyring reference (Windows Credential Manager) is
//! Phase 4 — the settings dialog only emits events for it; the API key is
//! never written to a file.
//!
//! Layout:
//! - [`settings`] — the [`Settings`] struct (ARCHITECTURE.md §8.1 schema,
//!   `_v: 1`) and `load` / `save`.
//! - [`keybind`] — [`Keybind::parse`] and the egui shortcut matcher.
//! - [`detect`] — the `which`-style PATH lookup used by the Settings dialog's
//!   "Auto-detect" buttons.
//!
//! Every fallible operation returns `Result<T, SeloraError>` (PLAN.md §8);
//! version-mismatch wipes surface as [`SeloraError::Config`] so the shell can
//! toast them.

#![forbid(unsafe_code)]

mod detect;
mod keybind;
mod settings;

pub use detect::{find_in_dirs, find_on_path};
pub use keybind::{keybind_matches, Keybind};
pub use settings::{
    EditorSettings, KeybindsSettings, LspSettings, SeloraSettings, Settings, UiSettings,
    CONFIG_DIR, SCHEMA_VERSION, SETTINGS_DIR_ENV, SETTINGS_FILE_NAME,
};
