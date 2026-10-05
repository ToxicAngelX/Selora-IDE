//! The cost meter (UI_SPEC.md §1.2 right zone) — Phase 4e's file.
//!
//! `Σ today $X.XX`, summed from the [`ChatReceipt`] of every completed
//! chat / ghost-text request (`selora_ai::ChatReceipt::cost_usd`, exact
//! when the gateway reports usage, else `compute_cost_usd` from the
//! model's price table — RISKS.md §3). Per-model breakdown in a tooltip.
//!
//! Pure widget: the running total arrives from the shell (which owns the
//! receipts); this file only renders it.
