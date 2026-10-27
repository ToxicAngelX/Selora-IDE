//! Selora LSP — the Language Server Protocol client (ARCHITECTURE.md §5).
//!
//! One actor per server (rust-analyzer, typescript-language-server) over
//! stdio JSON-RPC, built on `lsp-types` + `tokio` — no `tower-lsp` (that is
//! server-side; the client is thin enough to hand-roll, PLAN.md §4.5).
//! Depends only on `selora-core`.
//!
//! # Layout
//!
//! - [`LspClient`] — the per-server actor: owns the child process, frames
//!   stdin/stdout, routes responses by request id, resolves hover /
//!   definition / completion requests, dies loudly when the server does
//!   (`client` module).
//! - [`LspRegistry`] — language → command resolution from the settings-fed
//!   paths with PATH-lookup defaults (`registry` module).
//! - [`LspManager`] — the app-facing task: spawns sessions on first open,
//!   debounces `didChange` (250 ms), translates wire types into the plain
//!   [`LspEvent`] data, restarts dead servers on request (`manager`
//!   module).
//! - `framing` / `jsonrpc` / `position` / `uri` — the pure helpers: message
//!   framing, message classification, UTF-16 ↔ code-point positions, and
//!   `file://` conversion.
//!
//! # App wiring (the contract the orchestrator applies)
//!
//! The app (`selora-app`) owns the `Effect` / `AppMessage` enums; this crate
//! mirrors them as [`LspEffect`] / [`LspEvent`] so the runtime dispatcher can
//! route LSP effects into [`spawn_manager`] and the frame loop can reduce
//! `LspEvent`s into `AppState`. Concretely, in `app_state.rs`:
//!
//! ```rust,ignore
//! // Effect: add one variant that wraps ours (or the eight variants 1:1).
//! #[derive(Debug)]
//! pub enum Effect {
//!     // … existing variants …
//!     Lsp(selora_lsp::LspEffect),
//! }
//!
//! // AppMessage: same shape.
//! #[derive(Debug)]
//! pub enum AppMessage {
//!     // … existing variants …
//!     Lsp(selora_lsp::LspEvent),
//! }
//! ```
//!
//! In `wiring.rs`, the dispatcher creates the manager once (before the
//! loop) and forwards matching effects:
//!
//! ```rust,ignore
//! let (lsp_effect_tx, lsp_effect_rx) = mpsc::unbounded_channel::<selora_lsp::LspEffect>();
//! let lsp_config = selora_lsp::LspManagerConfig::new(selora_lsp::LspRegistry::new(
//!     selora_lsp::LspServerPaths {
//!         rust_analyzer: settings.lsp.rust_analyzer_path.clone(),
//!         typescript_language_server: settings.lsp.typescript_server_path.clone(),
//!     },
//! ))
//! .with_root(workspace_root.clone()); // root = the open folder, if any
//! selora_lsp::spawn_manager(&handle, notify.clone(), lsp_effect_rx, msg_tx.clone(), lsp_config);
//!
//! // in the dispatcher match:
//! Effect::Lsp(effect) => { let _ = lsp_effect_tx.send(effect); }
//! ```
//!
//! The shell's frame loop emits effects from editor signals:
//! first paint of a tab → `LspEffect::Open`, an edit commit (the same
//! shape/edit signal `note_editor_frame` already derives) →
//! `LspEffect::Change` with the buffer text, Ctrl+S (besides
//! `SaveBuffer`) → `LspEffect::Save`, tab close (where `prune_closed`
//! runs) → `LspEffect::Close`, and F12 / Ctrl+K Ctrl+I / Ctrl+Space /
//! the 600 ms hover hold → the position requests (the `selora_editor`
//! side of those hooks is in `selora_editor::lsp_overlay`).

#![forbid(unsafe_code)]

mod client;
#[cfg(test)]
mod fake;
mod framing;
mod jsonrpc;
mod manager;
pub mod position;
mod registry;
mod uri;

pub use client::{spawn_process, ClientEvent, LaunchedServer, LspClient, ServerStatus};
pub use manager::{
    spawn_manager, Launcher, LspCompletionItem, LspDiagnostic, LspEffect, LspEvent, LspLocation,
    LspManagerConfig, LspPosition, LspRange, LspSeverity, Notify, DEBOUNCE,
};
pub use registry::{
    find_in_dirs, find_on_path, LanguageId, LspRegistry, LspServerPaths, ServerSpec,
};
pub use uri::{path_to_uri, uri_to_path};
