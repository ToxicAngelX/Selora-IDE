//! Workflow-level budget (ARCHITECTURE.md §18.1, ROADMAP.md Phase 8b).
//!
//! Per-ask `TurnCaps` are already enforced by the §17 loop, but a 50-subagent
//! fan-out of cheap turns still lands a bill. A workflow therefore carries its
//! own ceiling (`workflow.max_cost_usd`, default $5) checked **between every
//! step**.
//!
//! Two shapes share the type, for two callers:
//!
//! * 8a's engine asks the pure question — [`Budget::allows`] against the
//!   spend the [`crate::Host`] reports — and never mutates anything.
//! * 8b's run accounting *charges* each receipt and gates each step:
//!   [`Budget::charge`] records (a receipt is money already spent — hiding it
//!   would under-report the bill), then [`Budget::check`] returns a typed
//!   [`BudgetError::Exceeded`] that aborts before the next step.
//!
//! The cap is inclusive: spend exactly on it is allowed, the next charge over
//! it aborts. The comparison carries a nanodollar of tolerance so accumulated
//! float error cannot abort a run that is, to the cent, under budget.
//!
//! [`Budget::new`] keeps 8a's infallible signature and fails **closed** on an
//! unusable ceiling (a $0 budget); [`Budget::try_new`] is the strict form for
//! validated settings input.

use crate::sandbox::DEFAULT_MAX_COST_USD;

use serde::{Deserialize, Serialize};

/// Float slack for the cap comparison, in USD. A cent is 1e-2; this is seven
/// orders of magnitude smaller, so it only absorbs accumulation error.
const CAP_TOLERANCE_USD: f64 = 1e-9;

/// A run's cost ceiling and (once charging starts) its spend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    max_cost_usd: f64,
    spent_usd: f64,
    tokens: u64,
}

/// A typed budget failure: an invalid cap/charge, or the cap firing.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BudgetError {
    /// The configured ceiling is not a usable USD amount.
    #[error("invalid workflow cost cap ${value:.4}: must be finite and non-negative")]
    InvalidCap {
        /// The rejected value.
        value: f64,
    },
    /// A receipt reported an unusable charge.
    #[error("invalid workflow cost charge ${cost_usd:.4}: must be finite and non-negative")]
    InvalidCharge {
        /// The rejected charge.
        cost_usd: f64,
    },
    /// Spend is past the ceiling; the run must abort before its next step.
    ///
    /// The engine maps this onto
    /// [`crate::WorkflowError::BudgetExceeded`] for the run report.
    #[error("workflow cost cap exceeded: spent ${spent_usd:.4} of ${max_cost_usd:.2}")]
    Exceeded {
        /// Spend recorded so far (`> max_cost_usd`).
        spent_usd: f64,
        /// The configured ceiling.
        max_cost_usd: f64,
    },
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_cost_usd: DEFAULT_MAX_COST_USD,
            spent_usd: 0.0,
            tokens: 0,
        }
    }
}

impl Budget {
    /// A budget with the given ceiling, in USD.
    ///
    /// Infallible for 8a's engine path: a non-finite or negative ceiling
    /// fails **closed** (a $0 budget) rather than open. Settings validation
    /// goes through [`Budget::try_new`] instead.
    #[must_use]
    pub fn new(max_usd: f64) -> Self {
        Self {
            max_cost_usd: sanitize_cap(max_usd),
            spent_usd: 0.0,
            tokens: 0,
        }
    }

    /// A budget with `max_usd`, rejecting an unusable ceiling.
    ///
    /// # Errors
    /// [`BudgetError::InvalidCap`] when `max_usd` is negative or non-finite.
    pub fn try_new(max_usd: f64) -> Result<Self, BudgetError> {
        validate_cap(max_usd)?;
        Ok(Self {
            max_cost_usd: max_usd,
            spent_usd: 0.0,
            tokens: 0,
        })
    }

    /// A budget restored from a journal, so a resumed run keeps accounting
    /// for what it already spent.
    ///
    /// Spend may already be at/over the ceiling; [`Budget::check`] then
    /// refuses the next step. The ceiling itself must be valid.
    ///
    /// # Errors
    /// [`BudgetError::InvalidCap`] for an unusable ceiling,
    /// [`BudgetError::InvalidCharge`] for a negative/non-finite spend.
    pub fn with_spent(max_usd: f64, spent_usd: f64, tokens: u64) -> Result<Self, BudgetError> {
        validate_cap(max_usd)?;
        if !spent_usd.is_finite() || spent_usd < 0.0 {
            return Err(BudgetError::InvalidCharge {
                cost_usd: spent_usd,
            });
        }
        Ok(Self {
            max_cost_usd: max_usd,
            spent_usd,
            tokens,
        })
    }

    /// The ceiling, in USD.
    #[must_use]
    pub fn limit(&self) -> f64 {
        self.max_cost_usd
    }

    /// The ceiling, in USD (alias of [`Budget::limit`] for 8b callers).
    #[must_use]
    pub fn max_cost_usd(&self) -> f64 {
        self.max_cost_usd
    }

    /// Spend recorded by [`Budget::charge`], in USD.
    #[must_use]
    pub fn spent_usd(&self) -> f64 {
        self.spent_usd
    }

    /// Receipt tokens recorded by [`Budget::charge`].
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.tokens
    }

    /// Whether `spent_usd` is still within the ceiling (8a's pure check).
    ///
    /// Inclusive at the cap, and tolerant of float accumulation. `NaN`
    /// spend is never allowed.
    #[must_use]
    pub fn allows(&self, spent_usd: f64) -> bool {
        spent_usd <= self.max_cost_usd + CAP_TOLERANCE_USD
    }

    /// The between-steps gate over the spend this budget has charged.
    ///
    /// # Errors
    /// [`BudgetError::Exceeded`] once spend is past the ceiling.
    pub fn check(&self) -> Result<(), BudgetError> {
        if self.allows(self.spent_usd) {
            Ok(())
        } else {
            Err(BudgetError::Exceeded {
                spent_usd: self.spent_usd,
                max_cost_usd: self.max_cost_usd,
            })
        }
    }

    /// Record one receipt's usage, then report whether the workflow may
    /// continue.
    ///
    /// The charge is always recorded first: a receipt is money already
    /// spent. `Err(Exceeded)` means the engine must abort before the next
    /// step; the spend that caused it stays visible in
    /// [`Budget::spent_usd`].
    ///
    /// # Errors
    /// [`BudgetError::InvalidCharge`] for a negative/non-finite charge, or
    /// [`BudgetError::Exceeded`] after recording a charge that crosses the
    /// ceiling.
    pub fn charge(&mut self, tokens: u64, cost_usd: f64) -> Result<(), BudgetError> {
        if !cost_usd.is_finite() || cost_usd < 0.0 {
            return Err(BudgetError::InvalidCharge { cost_usd });
        }
        self.tokens = self.tokens.saturating_add(tokens);
        self.spent_usd += cost_usd;
        self.check()
    }

    /// How much room is left, given `spent_usd` (never negative).
    #[must_use]
    pub fn remaining(&self, spent_usd: f64) -> f64 {
        (self.max_cost_usd - spent_usd).max(0.0)
    }

    /// How much room is left against this budget's own spend.
    #[must_use]
    pub fn remaining_usd(&self) -> f64 {
        self.remaining(self.spent_usd)
    }

    /// Spend as a fraction of the ceiling, clamped to `0.0..=1.0` for the
    /// cost meter.
    #[must_use]
    pub fn used_fraction(&self) -> f64 {
        if self.max_cost_usd <= 0.0 {
            return 1.0;
        }
        (self.spent_usd / self.max_cost_usd).clamp(0.0, 1.0)
    }

    /// A serializable view for the run summary / cost meter.
    #[must_use]
    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            max_cost_usd: self.max_cost_usd,
            spent_usd: self.spent_usd,
            tokens: self.tokens,
        }
    }
}

/// The cost meter's wire shape (§18.4 progress messages, run summary).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    /// The configured workflow ceiling in USD.
    pub max_cost_usd: f64,
    /// Spend so far in USD.
    pub spent_usd: f64,
    /// Receipt tokens so far.
    pub tokens: u64,
}

fn validate_cap(value: f64) -> Result<(), BudgetError> {
    if !value.is_finite() || value < 0.0 {
        return Err(BudgetError::InvalidCap { value });
    }
    Ok(())
}

fn sanitize_cap(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_is_inclusive_and_never_negative() {
        let budget = Budget::new(5.0);
        assert!(budget.allows(5.0));
        assert!(!budget.allows(5.01));
        assert_eq!(budget.remaining(6.0), 0.0);
        assert_eq!(budget.remaining(1.5), 3.5);
    }

    #[test]
    fn default_is_five_dollars() {
        let budget = Budget::default();
        assert_eq!(budget.limit(), 5.0);
        assert_eq!(budget.spent_usd(), 0.0);
        assert_eq!(budget.tokens(), 0);
        assert_eq!(budget.remaining_usd(), 5.0);
        assert_eq!(budget.used_fraction(), 0.0);
        assert!(budget.check().is_ok());
    }

    #[test]
    fn stays_under_the_cap() {
        let mut budget = Budget::try_new(5.0).expect("cap");
        budget.charge(1_000, 0.25).expect("under cap");
        budget.charge(2_000, 1.50).expect("under cap");
        assert_eq!(budget.tokens(), 3_000);
        assert!((budget.spent_usd() - 1.75).abs() < 1e-12);
        assert!((budget.remaining_usd() - 3.25).abs() < 1e-12);
        assert!(budget.check().is_ok());
        let snapshot = budget.snapshot();
        assert_eq!(snapshot.max_cost_usd, 5.0);
        assert_eq!(snapshot.tokens, 3_000);
        assert!((snapshot.spent_usd - 1.75).abs() < 1e-12);
    }

    #[test]
    fn the_exact_cap_is_allowed() {
        let mut budget = Budget::try_new(5.0).expect("cap");
        budget.charge(10, 2.5).expect("first half");
        // Exactly on the cap: inclusive, so the run continues.
        budget.charge(10, 2.5).expect("exact cap");
        assert_eq!(budget.spent_usd(), 5.0);
        assert_eq!(budget.remaining_usd(), 0.0);
        assert_eq!(budget.used_fraction(), 1.0);
        assert!(budget.check().is_ok());
    }

    #[test]
    fn over_cap_aborts_before_the_next_step() {
        let mut budget = Budget::try_new(5.0).expect("cap");
        budget.charge(10, 4.99).expect("under cap");
        let error = budget.charge(10, 0.02).expect_err("over cap");
        match error {
            BudgetError::Exceeded {
                spent_usd,
                max_cost_usd,
            } => {
                assert!((spent_usd - 5.01).abs() < 1e-9);
                assert_eq!(max_cost_usd, 5.0);
            }
            other => panic!("expected Exceeded, got {other:?}"),
        }
        // The charge is recorded (it happened) and every subsequent step
        // gate refuses until the run aborts.
        assert!((budget.spent_usd() - 5.01).abs() < 1e-9);
        assert!(budget.check().is_err());
    }

    #[test]
    fn a_resumed_budget_keeps_prior_spend() {
        let mut budget = Budget::with_spent(5.0, 4.5, 900).expect("resume");
        assert_eq!(budget.tokens(), 900);
        assert!(budget.check().is_ok());
        budget.charge(100, 0.6).expect_err("already over");
        assert!((budget.spent_usd() - 5.1).abs() < 1e-9);

        let over = Budget::with_spent(5.0, 9.0, 0).expect("resume over cap");
        assert!(over.check().is_err());
    }

    #[test]
    fn invalid_caps_and_charges_are_typed_errors() {
        for bad in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(matches!(
                Budget::try_new(bad),
                Err(BudgetError::InvalidCap { .. })
            ));
            // 8a's infallible entry point fails closed.
            let budget = Budget::new(bad);
            assert_eq!(budget.limit(), 0.0, "{bad}");
            assert!(!budget.allows(0.01));
        }
        let mut budget = Budget::try_new(5.0).expect("cap");
        for bad in [-0.1, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                budget.charge(0, bad),
                Err(BudgetError::InvalidCharge { .. })
            ));
        }
        assert_eq!(budget.spent_usd(), 0.0);
        assert!(matches!(
            Budget::with_spent(5.0, -1.0, 0),
            Err(BudgetError::InvalidCharge { .. })
        ));
    }

    #[test]
    fn a_zero_cap_allows_only_free_steps() {
        let mut budget = Budget::new(0.0);
        budget.charge(10, 0.0).expect("free step");
        assert!(budget.check().is_ok());
        assert!(budget.charge(10, 0.000_001).is_err());
    }

    #[test]
    fn float_accumulation_does_not_false_abort() {
        // 0.1 * 10 is 0.9999999999999999 in f64: under the cap with the
        // tolerance, never a spurious abort.
        let mut budget = Budget::try_new(1.0).expect("cap");
        for _ in 0..10 {
            budget.charge(1, 0.1).expect("under cap");
        }
        assert!(budget.check().is_ok());
    }
}
