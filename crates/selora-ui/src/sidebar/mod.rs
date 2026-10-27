//! Left-region chrome (activity bar, file tree) and the right-region
//! chat sidebar content (UI_SPEC.md §2.1, §2.2, §4). Phase 2b / 4c.

pub mod activity_bar;
pub mod chat;
pub mod file_tree;
mod icons;

pub use activity_bar::{Activity, ActivityBar, STRIP_WIDTH};
pub use chat::{
    ChatId, ChatPanel, ChatPanelEvent, ChatPanelOutcome, ChatTurn, MAX_PERSISTED_TURNS,
};
pub use file_tree::{
    FileTreePanel, ShowOutcome, TreeEvent, FOOTER_HEIGHT, HEADER_HEIGHT, HEADER_LABEL,
    INDENT_PER_LEVEL, ROW_HEIGHT,
};
