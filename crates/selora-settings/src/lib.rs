//! Selora Settings — persistence for user configuration.
//!
//! Responsibility (PLAN.md §5): JSON config read/write at
//! `%APPDATA%\selora-ide\settings.json`, and the keyring reference (Windows
//! Credential Manager via the `keyring` crate). The API key is never written
//! to a file.
//!
//! Phase 0: crate skeleton only. Implementation lands in Phase 2.

#![forbid(unsafe_code)]
