//! The chat sidebar (UI_SPEC.md §4, region ③) — Phase 4c's file.
//!
//! Full chat panel: bubbles (user right-aligned `--ink-mid`, assistant
//! left-aligned `--ink-low`), streaming with the comet fade-in, code
//! blocks with hover Copy buttons, conversation history (last 20 turns
//! persisted to workspace state), "clear conversation" action, and the
//! composer (Enter sends, Shift+Enter newline). Toggled by `Ctrl+L`.
//!
//! The panel is a pure widget over state the shell owns: it emits
//! `ChatPanelEvent`s (send / clear / pick model / toggle) and renders the
//! message list it is handed. The model picker header is 4b's
//! `crate::picker::model_picker`.
