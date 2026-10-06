//! Tool implementations (ARCHITECTURE.md §17.1) — Phase 5b's file.
//!
//! One file per tool family: the fs tools (`read_file`, `list_dir`,
//! `fuzzy_find`, `search_files`, `write_file`, `edit_file`,
//! `delete_file`), the bash tool (§17.5), and [`web`] (Phase 5e). The
//! guards live in [`super::router`] — a tool that cannot prove its
//! safety returns an error result instead.

pub mod web;
