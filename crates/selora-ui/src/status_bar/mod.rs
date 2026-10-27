//! Status-bar widgets (UI_SPEC.md §1.2): the workbench zones (left: git +
//! LSP servers; center: cursor + progress — Phase R0), the cost meter
//! (right zone) and the Selora connection status (right zone).

pub mod cost_meter;
pub mod selora_status;
pub mod workbench_zone;

/// Status bar height (UI_SPEC.md §1.2).
pub const HEIGHT: f32 = 24.0;
