//! Design tokens for the editor pane.
//!
//! A re-export of the leaf token crate `selora-tokens` (the typed
//! DESIGN_TOKENS.md §11 contract). This module existed as a value-exact
//! *mirror* of `selora-ui`'s theme in Phase 1, because PLAN.md §5's
//! dependency direction (`selora-ui` → `selora-editor`) made importing it a
//! cargo cycle. Phase 2 hoisted the constants into `selora-tokens`, which
//! both crates import — one source of truth, no mirror to keep in sync.

pub use selora_tokens::{color, font, motion, radius, space, type_scale};
