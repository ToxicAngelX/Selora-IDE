//! The Selora connection status (UI_SPEC.md §1.2 right zone) — Phase 4f's
//! file.
//!
//! "● selora connected · <model>" or "○ selora disconnected · connect".
//! The state is [`selora_ai::SeloraStatus`] (the shell pings every 30s via
//! `SeloraClient::status` and hands the result here); the disconnected
//! form is a click target for opening the Settings dialog's Selora
//! section. On a disconnect mid-chat, in-flight chats complete — new ones
//! get the toast (the shell's call, via `ChatError`).
//!
//! Pure widget: paints the state it is handed, emits a click event.
