//! The model picker dropdown (UI_SPEC.md §4.1) — Phase 4b's file.
//!
//! Contract (ROADMAP.md Phase 4b): live model list on app boot, refresh on
//! demand (a button in the picker header), cached in
//! `settings.selora.active_model` for offline display, search/filter
//! inside the dropdown. The list comes in as `&[Model]` from
//! `AppMessage::ModelsLoaded` (the shell owns the fetch); the picker is a
//! pure widget — it emits a `ModelPickerEvent` (select / refresh / close)
//! and never touches the network or settings files itself.
