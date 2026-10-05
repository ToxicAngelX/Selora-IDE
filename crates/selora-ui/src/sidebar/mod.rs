//! Left-region chrome: activity bar and the file tree sidebar
//! (UI_SPEC.md §2.1, §2.2). Phase 2b.

pub mod activity_bar;
pub mod file_tree;
mod icons;

pub use activity_bar::{Activity, ActivityBar, STRIP_WIDTH};
pub use file_tree::{
    FileTreePanel, ShowOutcome, TreeEvent, FOOTER_HEIGHT, HEADER_HEIGHT, HEADER_LABEL,
    INDENT_PER_LEVEL, ROW_HEIGHT,
};
