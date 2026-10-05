//! The cost meter (UI_SPEC.md §1.2 right zone) — Phase 4e.
//!
//! `Σ today $X.XX`, summed from the [`ChatReceipt`] of every completed
//! chat / ghost-text request (ROADMAP.md Phase 4e): the receipt's own
//! `cost_usd` when the gateway reported usage, else
//! [`selora_ai::compute_cost_usd`] against the model's price table
//! (RISKS.md §3 — never a guessed price). Hovering the run lists the
//! per-model totals ("claude-opus-5-5 $0.42").
//!
//! The receipts are owned by the shell; the aggregation lives here in
//! [`CostMeterState`], which the shell steps:
//!
//! - [`CostMeterState::add_receipt`] — once per completed request (the
//!   `AppMessage::ChatDone` handler), with the model resolved from
//!   `state.models` by `receipt.model` so uncosted estimates can be priced.
//!   A model of `None` (unknown id) bills nothing — we never guess.
//! - [`CostMeterState::sync_day`] — once per frame with today's day number,
//!   so the total clears at the day boundary even with no new traffic.
//!
//! "Today" is a plain `u32` day ordinal the shell computes.
//! [`day_number`] is the default helper (UTC days since the Unix epoch,
//! from [`std::time::SystemTime`]); any local-date ordinal works — the
//! rollover triggers on *any* change of the value, so the shell may
//! substitute a local-timezone day count without touching this module.
//!
//! [`CostMeter`] is the paint half: a pure widget that renders the total
//! right-anchored in status-bar typography (`--type-xs` sans,
//! `--fg-muted`, `Σ` glyph, two decimals) and paints nothing at all while
//! the total is zero — a fresh session shows no clutter.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use selora_ai::{compute_cost_usd, ChatReceipt, Model};

use crate::theme::{color, font, space, type_scale};

/// Seconds in one day (the [`day_number`] unit step).
const SECS_PER_DAY: u64 = 86_400;

/// Days since the Unix epoch of `at` (UTC) — the shell's default source
/// for the "today" day ordinal. See the module docs: any day ordinal the
/// shell can produce works; rollover fires when the value changes.
pub fn day_number(at: SystemTime) -> u32 {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    (secs / SECS_PER_DAY) as u32
}

/// The running `Σ today` total — the aggregation half of the cost meter.
///
/// Owned by the shell (alongside the receipts) and stepped via
/// [`add_receipt`](Self::add_receipt) / [`sync_day`](Self::sync_day);
/// rendered by [`CostMeter`]. Per-model totals are kept in a `BTreeMap` so
/// the tooltip order is stable.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CostMeterState {
    /// The day ordinal the current totals belong to; `None` until the first
    /// receipt or sync lands.
    day: Option<u32>,
    /// `Σ today` in USD.
    total_usd: f64,
    /// Per-model totals, keyed by model id.
    per_model: BTreeMap<String, f64>,
}

impl CostMeterState {
    /// An empty meter (fresh session: paints nothing).
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one completed request's receipt to the running total.
    ///
    /// `model` is the shell's resolution of `receipt.model` from
    /// `state.models`. It is only consulted when the receipt is an estimate
    /// with no gateway cost (`cost_usd == 0.0 && estimated`, RISKS.md §3):
    /// then the cost comes from [`selora_ai::compute_cost_usd`] against the
    /// model's price table. A receipt that cannot be priced (unknown model)
    /// contributes nothing rather than a guessed value.
    ///
    /// `today` is the current day ordinal; when it differs from the day the
    /// accumulated totals belong to, they reset first (the "today" window).
    pub fn add_receipt(&mut self, receipt: &ChatReceipt, model: Option<&Model>, today: u32) {
        self.roll_if_new_day(today);
        let cost = cost_of(receipt, model);
        if cost <= 0.0 {
            // Free model, or an estimate we cannot price (no Model row):
            // no tooltip row, no pixels. Never a guessed price.
            return;
        }
        self.total_usd += cost;
        *self.per_model.entry(receipt.model.clone()).or_insert(0.0) += cost;
    }

    /// Refresh the "today" boundary without a receipt, so the meter clears
    /// at the day change even when no request completes after it. The shell
    /// calls this once per frame.
    pub fn sync_day(&mut self, today: u32) {
        self.roll_if_new_day(today);
    }

    /// `Σ today` in USD.
    pub fn total_usd(&self) -> f64 {
        self.total_usd
    }

    /// Per-model totals in ascending model-id order (tooltip + tests).
    pub fn per_model_totals(&self) -> Vec<(&str, f64)> {
        self.per_model
            .iter()
            .map(|(model, cost)| (model.as_str(), *cost))
            .collect()
    }

    /// Whether there is anything to paint (zero total = fresh session).
    pub fn is_empty(&self) -> bool {
        self.total_usd <= 0.0
    }

    /// The status-bar label: `Σ today $X.XX` (two decimals, `Σ`, `$`).
    fn label(&self) -> String {
        format!("Σ today ${:.2}", self.total_usd)
    }

    /// One tooltip row per model: `claude-opus-5-5 $0.42`.
    fn tooltip_lines(&self) -> Vec<String> {
        self.per_model
            .iter()
            .map(|(model, cost)| format!("{model} ${cost:.2}"))
            .collect()
    }

    /// Reset the totals when the day ordinal moved (or was never set).
    fn roll_if_new_day(&mut self, today: u32) {
        if self.day != Some(today) {
            self.day = Some(today);
            self.total_usd = 0.0;
            self.per_model.clear();
        }
    }
}

/// The effective USD cost of one receipt: the gateway's own figure when it
/// reported one, else the price-table computation for estimates
/// (RISKS.md §3). Unknown model + estimate = 0.0, never a guess.
fn cost_of(receipt: &ChatReceipt, model: Option<&Model>) -> f64 {
    if receipt.estimated && receipt.cost_usd <= 0.0 {
        model
            .map(|model| compute_cost_usd(model, receipt.prompt_tokens, receipt.completion_tokens))
            .unwrap_or(0.0)
    } else {
        receipt.cost_usd
    }
}

/// The cost-meter widget — the paint half (UI_SPEC.md §1.2 right zone).
///
/// Renders `Σ today $X.XX` right-anchored in a status-bar strip rect, in
/// the status-bar typography (`--type-xs` sans, `--fg-muted`); hovering
/// shows the per-model breakdown. Zero total paints nothing.
pub struct CostMeter<'a> {
    state: &'a CostMeterState,
}

impl<'a> CostMeter<'a> {
    /// Wrap the state the shell owns.
    pub fn new(state: &'a CostMeterState) -> Self {
        Self { state }
    }

    /// Paint the meter right-anchored inside `rect` (the 24px status-bar
    /// strip), vertically centered, with the `--space-3` right inset the
    /// shell's other right-zone runs use.
    ///
    /// Hovering the painted run shows the per-model breakdown
    /// (`Sense::hover` — the meter is not a click control).
    ///
    /// Returns the horizontal width consumed *including* the right inset
    /// (`--space-3` + text), so the shell can anchor the next right-zone
    /// run at `rect.right() - consumed - gap`. Returns `0.0` and paints
    /// nothing while the total is zero (no clutter on a fresh session).
    pub fn show(self, ui: &egui::Ui, rect: egui::Rect) -> f32 {
        if self.state.is_empty() {
            return 0.0;
        }

        let font = egui::FontId::new(type_scale::XS, font::FAMILY_SANS);
        let galley = ui
            .painter()
            .layout_no_wrap(self.state.label(), font, color::FG_MUTED);
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - space::S3 - galley.size().x, rect.top()),
            egui::vec2(galley.size().x, rect.height()),
        );
        ui.painter().galley(
            egui::pos2(text_rect.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            color::FG_MUTED,
        );

        // Hover target: the text run plus the right inset, full bar height.
        let hit_rect = egui::Rect::from_min_max(
            egui::pos2(text_rect.left() - space::S1, rect.top()),
            rect.right_bottom(),
        );
        ui.interact(
            hit_rect,
            egui::Id::new("status_bar.cost_meter"),
            egui::Sense::hover(),
        )
        .on_hover_text(self.state.tooltip_lines().join("\n"));

        space::S3 + text_rect.width()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 1000 prompt / 2000 completion tokens: with the 0.5/1.5 prices below,
    /// `compute_cost_usd` yields exactly $3.50.
    fn receipt(model: &str, cost_usd: f64, estimated: bool) -> ChatReceipt {
        ChatReceipt {
            request_id: Default::default(),
            model: model.to_owned(),
            prompt_tokens: 1000,
            completion_tokens: 2000,
            cost_usd,
            duration_ms: 0,
            estimated,
        }
    }

    fn model(id: &str, price_in: f64, price_out: f64) -> Model {
        Model {
            id: id.to_owned(),
            display_name: id.to_owned(),
            context_window: 0,
            price_per_1k_input: price_in,
            price_per_1k_output: price_out,
            supports_streaming: true,
            supports_tools: false,
            family: String::new(),
        }
    }

    fn bar_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 24.0))
    }

    #[test]
    fn aggregation_sums_receipts_and_groups_per_model() {
        let mut meter = CostMeterState::new();
        meter.add_receipt(&receipt("claude-opus-5-5", 0.42, false), None, 100);
        meter.add_receipt(&receipt("claude-opus-5-5", 0.10, false), None, 100);
        meter.add_receipt(&receipt("glm-5.3", 0.05, false), None, 100);
        assert!((meter.total_usd() - 0.57).abs() < 1e-12);

        let per_model = meter.per_model_totals();
        assert_eq!(per_model.len(), 2);
        assert_eq!(per_model[0].0, "claude-opus-5-5");
        assert!((per_model[0].1 - 0.52).abs() < 1e-12);
        assert_eq!(per_model[1].0, "glm-5.3");
        assert!((per_model[1].1 - 0.05).abs() < 1e-12);
    }

    #[test]
    fn estimated_receipts_without_gateway_cost_use_the_model_table() {
        let model = model("glm-5.3", 0.5, 1.5);

        // Gateway sent no usage: cost from compute_cost_usd (RISKS.md §3).
        let mut meter = CostMeterState::new();
        meter.add_receipt(&receipt("glm-5.3", 0.0, true), Some(&model), 0);
        assert!((meter.total_usd() - 3.50).abs() < 1e-12);

        // Unknown model id (None): billed nothing, never a guessed price.
        let mut meter = CostMeterState::new();
        meter.add_receipt(&receipt("claude-opus-5-5", 0.0, true), None, 0);
        assert!(meter.is_empty());
        assert!(meter.per_model_totals().is_empty());
    }

    #[test]
    fn gateway_reported_cost_is_used_unchanged() {
        let model = model("claude-opus-5-5", 9.9, 9.9); // prices must be ignored
        let mut meter = CostMeterState::new();
        meter.add_receipt(&receipt("claude-opus-5-5", 1.23, false), Some(&model), 0);
        assert_eq!(meter.total_usd(), 1.23);

        // An estimate that already carries a cost was priced upstream: trust it.
        meter.add_receipt(&receipt("claude-opus-5-5", 0.10, true), Some(&model), 0);
        assert!((meter.total_usd() - 1.33).abs() < 1e-12);
    }

    #[test]
    fn day_rollover_resets_the_total() {
        let mut meter = CostMeterState::new();
        meter.add_receipt(&receipt("glm-5.3", 0.40, false), None, 100);
        meter.sync_day(100); // same day: totals survive
        assert!((meter.total_usd() - 0.40).abs() < 1e-12);

        // First receipt of the next day resets before accumulating.
        meter.add_receipt(&receipt("glm-5.3", 0.02, false), None, 101);
        assert!((meter.total_usd() - 0.02).abs() < 1e-12);
        assert_eq!(meter.per_model_totals().len(), 1);
    }

    #[test]
    fn sync_day_clears_at_midnight_without_new_receipts() {
        let mut meter = CostMeterState::new();
        meter.add_receipt(&receipt("glm-5.3", 0.40, false), None, 100);
        meter.sync_day(101);
        assert!(meter.is_empty());
        assert!(meter.per_model_totals().is_empty());
    }

    #[test]
    fn day_number_counts_epoch_days() {
        assert_eq!(day_number(UNIX_EPOCH), 0);
        let mid_day_3 = UNIX_EPOCH + Duration::from_secs(3 * SECS_PER_DAY + 7);
        assert_eq!(day_number(mid_day_3), 3);
    }

    #[test]
    fn label_and_tooltip_format() {
        let mut meter = CostMeterState::new();
        assert!(meter.is_empty());
        meter.add_receipt(&receipt("claude-opus-5-5", 0.42, false), None, 0);
        meter.add_receipt(&receipt("glm-5.3", 0.05, false), None, 0);
        assert_eq!(meter.label(), "Σ today $0.47");
        assert_eq!(
            meter.tooltip_lines(),
            vec![
                "claude-opus-5-5 $0.42".to_owned(),
                "glm-5.3 $0.05".to_owned(),
            ]
        );
    }

    #[test]
    fn zero_total_paints_nothing() {
        egui::__run_test_ui(|ui| {
            // Fresh session: nothing painted, nothing consumed.
            let empty = CostMeterState::new();
            assert_eq!(CostMeter::new(&empty).show(ui, bar_rect()), 0.0);

            // Unpriceable estimate (no Model): still nothing painted.
            let mut meter = CostMeterState::new();
            meter.add_receipt(&receipt("glm-5.3", 0.0, true), None, 0);
            assert!(meter.is_empty());
            assert_eq!(CostMeter::new(&meter).show(ui, bar_rect()), 0.0);
        });
    }

    #[test]
    fn paints_nonzero_total_headlessly() {
        let mut meter = CostMeterState::new();
        meter.add_receipt(&receipt("claude-opus-5-5", 0.42, false), None, 0);
        meter.add_receipt(&receipt("glm-5.3", 0.05, false), None, 0);
        assert!(!meter.is_empty());

        egui::__run_test_ui(|ui| {
            let consumed = CostMeter::new(&meter).show(ui, bar_rect());
            assert!(consumed > 0.0, "the meter run should consume width");
        });
    }
}
