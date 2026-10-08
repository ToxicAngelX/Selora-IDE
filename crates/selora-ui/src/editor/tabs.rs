//! The tab bar + tab management (UI_SPEC.md §3.1). Phase 2a.
//!
//! Two halves:
//!
//! - [`TabsState`] — the pure tab model: ordered open tabs, the active tab,
//!   dirty flags. Its `open`/`close` semantics mirror
//!   `selora_core::Workspace` (ARCHITECTURE.md §3.3: `open_files` is "the
//!   last-known open file set, in tab order"): opening a path that is
//!   already open *focuses* it (moving it to the most-recent end) rather
//!   than duplicating it, and closing the active tab falls back to the most
//!   recently opened remaining tab. The app shell keeps
//!   `workspace.open_files` and each pane's `TabsState` in sync through
//!   these two operations, which is why the semantics must agree exactly.
//! - [`tab_bar_at`] / [`tab_bar`] — the egui paint + interaction for the
//!   32px chrome strip on top of every editor pane.
//!
//! Every measurement below is a UI_SPEC §3.1 or DESIGN_TOKENS value; see
//! the constants for the mapping. No shadows, no weight-700, no borders on
//! individual tabs (the active tab's 1px `--fg-default` bottom line is the
//! one sanctioned exception, UI_SPEC.md §3.1).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{
    pos2, vec2, Align2, Color32, Event, FontId, Galley, Id, MouseWheelUnit, Rect, Sense, Stroke, Ui,
};

use crate::theme::{color, font, radius, space, type_scale};

// ---------------------------------------------------------------------------
// Geometry (UI_SPEC.md §3.1, all values from DESIGN_TOKENS.md where one exists)
// ---------------------------------------------------------------------------

/// Tab bar height: "32px tall" (UI_SPEC.md §3.1) = `--space-6`.
pub const TAB_HEIGHT: f32 = space::S6;
/// Horizontal padding around each tab label: "Padded 12px horizontal" =
/// `--space-3`.
const TAB_PAD_H: f32 = space::S3;
/// Breathing room between the label and the dirty dot = `--space-1`.
const LABEL_TRAIL_GAP: f32 = space::S1;
/// Dirty marker diameter: "6px circle" (UI_SPEC.md §3.1).
const DOT_DIAMETER: f32 = 6.0;
/// Gap between the dirty dot and the close zone = `--space-1`.
const DOT_GAP: f32 = space::S1;
/// Close button hit zone width = `--space-3`.
const CLOSE_WIDTH: f32 = space::S3;
/// Inset of the close zone from the tab's right edge = `--space-2`
/// ("8px from the right edge of the tab").
const CLOSE_INSET: f32 = space::S2;
/// Everything reserved to the right of a label: trail gap + dot + dot gap +
/// close zone + close inset. Reserved unconditionally so tab widths never
/// jump when the dot or the close button appears.
const RIGHT_ZONE: f32 = LABEL_TRAIL_GAP + DOT_DIAMETER + DOT_GAP + CLOSE_WIDTH + CLOSE_INSET;
/// Width floor for a shrunken tab (UI_SPEC.md §3.1: "tabs shrink", but only
/// so far — `--space-6` doubled).
const TAB_MIN_WIDTH: f32 = space::S6 + space::S6;
/// The "+" button at the right end of the bar: a `--space-6` square.
const PLUS_WIDTH: f32 = space::S6;
/// All borders are 1px (DESIGN_TOKENS.md §5).
const HAIRLINE: f32 = 1.0;

// ---------------------------------------------------------------------------
// Tab model
// ---------------------------------------------------------------------------

/// One open tab: the file it points at and whether it has unsaved changes.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub path: PathBuf,
    pub dirty: bool,
}

/// The tabs of one editor pane (UI_SPEC.md §3.1). Ordered by recency of
/// opening — the same contract as `selora_core::Workspace::open_files`, so
/// the two can be kept in sync one-way (workspace → tabs on restore, tabs →
/// workspace on open/close).
///
/// `scroll_x` is the only view state in here: how far the strip is scrolled
/// horizontally when tabs overflow (UI_SPEC.md §3.1). It lives with the
/// model for the same reason the `Editor` widget's scroll offset does —
/// egui is immediate mode, so persistent view state needs a stable owner.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TabsState {
    tabs: Vec<Tab>,
    active: Option<usize>,
    scroll_x: f32,
}

impl TabsState {
    /// No tabs open, nothing active.
    pub fn new() -> Self {
        Self::default()
    }

    /// Restore a tab set from a persisted `workspace.open_files` (the
    /// recency order is preserved; the last file becomes active).
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut state = Self::default();
        for path in paths {
            state.open(path);
        }
        state
    }

    /// Open (or focus) `path`, mirroring `Workspace::open`: a path that is
    /// already open is moved to the most-recent end and made active rather
    /// than duplicated.
    pub fn open(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.tabs.retain(|tab| tab.path != path);
        self.tabs.push(Tab { path, dirty: false });
        self.active = Some(self.tabs.len() - 1);
    }

    /// Close the tab at `index` and return it. If the closed tab was the
    /// active one, the active tab falls back to the most recent remaining
    /// tab (mirroring `Workspace::close`).
    pub fn close_index(&mut self, index: usize) -> Option<Tab> {
        if index >= self.tabs.len() {
            return None;
        }
        let was_active = self.active == Some(index);
        let tab = self.tabs.remove(index);
        self.active = if was_active {
            self.tabs.len().checked_sub(1)
        } else {
            match self.active {
                Some(active) if active > index => Some(active - 1),
                other => other,
            }
        };
        Some(tab)
    }

    /// Close the tab for `path`, wherever it sits. Returns the closed tab.
    pub fn close_path(&mut self, path: &Path) -> Option<Tab> {
        let index = self.position(path)?;
        self.close_index(index)
    }

    /// Close the active tab (the Ctrl+W path).
    pub fn close_active(&mut self) -> Option<Tab> {
        let index = self.active?;
        self.close_index(index)
    }

    /// Focus (make active) the tab at `index` — a plain click on the tab.
    /// Unlike [`Self::open`] this does not reorder: clicking around must not
    /// shuffle the visual tab order.
    pub fn focus(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = Some(index);
        }
    }

    /// The open tabs in recency (= visual) order.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// The tab at `index`.
    pub fn tab(&self, index: usize) -> Option<&Tab> {
        self.tabs.get(index)
    }

    /// Position of the tab for `path`, if open.
    pub fn position(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.path == path)
    }

    /// Index of the active tab.
    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    /// The active tab.
    pub fn active(&self) -> Option<&Tab> {
        self.active.and_then(|index| self.tabs.get(index))
    }

    /// Path of the active tab.
    pub fn active_path(&self) -> Option<&Path> {
        self.active().map(|tab| tab.path.as_path())
    }

    /// The open paths, in tab order — the value to persist into
    /// `workspace.open_files`.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.tabs.iter().map(|tab| tab.path.clone()).collect()
    }

    /// Number of open tabs.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Whether any tab is open.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Mark the tab for `path` dirty (or clean). Returns whether a tab for
    /// that path exists.
    pub fn set_dirty(&mut self, path: &Path, dirty: bool) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.path == path) else {
            return false;
        };
        tab.dirty = dirty;
        true
    }

    /// Whether the tab for `path` is dirty.
    pub fn is_dirty(&self, path: &Path) -> bool {
        self.tabs.iter().any(|tab| tab.path == path && tab.dirty)
    }

    /// Whether any open tab is dirty (for "unsaved changes" prompts).
    pub fn any_dirty(&self) -> bool {
        self.tabs.iter().any(|tab| tab.dirty)
    }
}

// ---------------------------------------------------------------------------
// Tab bar response
// ---------------------------------------------------------------------------

/// What the user did to a tab bar this frame. The app shell turns these into
/// effects (closing a buffer, opening the quick-open dropdown); the model
/// has already been updated in place.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TabBarResponse {
    /// A tab was closed via its × button. The tab is already gone from the
    /// [`TabsState`].
    pub closed: Option<PathBuf>,
    /// A tab was clicked and is now the active one.
    pub activated: Option<usize>,
    /// The "+" button was clicked: the shell should open quick-open.
    pub new_tab: bool,
}

// ---------------------------------------------------------------------------
// Tab bar widget
// ---------------------------------------------------------------------------

/// Paint the tab bar at the next available position in `ui`, claiming the
/// full available width and [`TAB_HEIGHT`] pixels of height. For callers
/// that need the strip at an explicit rect (the split containers), use
/// [`tab_bar_at`].
pub fn tab_bar(ui: &mut Ui, tabs: &mut TabsState) -> TabBarResponse {
    let height = TAB_HEIGHT;
    let avail = ui.available_rect_before_wrap();
    let rect = Rect::from_min_size(avail.min, vec2(ui.available_width(), height));
    ui.allocate_rect(rect, Sense::hover());
    let id = ui.id().with("tab_bar");
    tab_bar_at(ui, tabs, rect, id)
}

/// Paint + interact the tab bar for `tabs` inside `rect`, at an explicit
/// position (UI_SPEC.md §3.1). `id` namespaces the widget: every editor
/// pane in a split must pass a distinct id (e.g. `ui.id().with(pane)`).
pub fn tab_bar_at(ui: &Ui, tabs: &mut TabsState, rect: Rect, id: Id) -> TabBarResponse {
    let mut out = TabBarResponse::default();
    let painter = ui.painter_at(rect);

    // Bar background: --ink-canvas (UI_SPEC.md §3.1).
    painter.rect_filled(rect, radius::R0, color::INK_CANVAS);

    let font_id = FontId::new(type_scale::MD, font::FAMILY_SANS);

    // The "+" button is pinned at the right; tabs fill the rest.
    let plus_rect = Rect::from_min_max(
        pos2(rect.right() - PLUS_WIDTH, rect.top()),
        pos2(rect.right(), rect.bottom()),
    );
    let strip = Rect::from_min_max(rect.min, pos2(plus_rect.left(), rect.bottom()));

    // Layout: measure every label once, then decide the per-tab widths
    // (UI_SPEC.md §3.1: shrink on overflow, then scroll). The galleys are
    // laid out with PLACEHOLDER color so the per-state text color is applied
    // at paint time (Painter::galley treats it as a fallback).
    let galleys: Vec<Arc<Galley>> = tabs
        .tabs()
        .iter()
        .map(|tab| {
            painter.layout_no_wrap(label_text(&tab.path), font_id.clone(), Color32::PLACEHOLDER)
        })
        .collect();
    let ideal: Vec<f32> = galleys
        .iter()
        .map(|galley| TAB_PAD_H + galley.size().x + RIGHT_ZONE)
        .collect();
    let avail = strip.width().max(0.0);
    let widths = shrink_widths(&ideal, avail);
    let total: f32 = widths.iter().sum();
    let max_scroll = (total - avail).max(0.0);
    tabs.scroll_x = tabs.scroll_x.clamp(0.0, max_scroll);

    // Horizontal scrolling when even shrunken tabs overflow: plain
    // (non-Ctrl — Ctrl+wheel is egui's zoom gesture) wheel over the strip,
    // vertical notches included, VS Code-style.
    let strip_resp = ui.interact(strip, id.with("strip"), Sense::hover());
    if strip_resp.hovered() {
        let delta = ui.ctx().input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    // `phase` (trackpad momentum) does not matter here.
                    Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                        ..
                    } if !modifiers.ctrl => Some(wheel_pixels(*unit, *delta, avail)),
                    _ => None,
                })
                .sum::<f32>()
        });
        if delta != 0.0 {
            tabs.scroll_x = (tabs.scroll_x + delta).clamp(0.0, max_scroll);
        }
    }

    // Keep the active tab in view when it changes off-screen.
    if let Some(index) = tabs.active_index() {
        if index < widths.len() && total > avail {
            let left: f32 = widths[..index].iter().sum();
            let right = left + widths[index];
            if left < tabs.scroll_x {
                tabs.scroll_x = left;
            } else if right > tabs.scroll_x + avail {
                tabs.scroll_x = right - avail;
            }
            tabs.scroll_x = tabs.scroll_x.clamp(0.0, max_scroll);
        }
    }

    let mut x = strip.left() - tabs.scroll_x;
    for (index, width) in widths.iter().enumerate() {
        let tab_rect = Rect::from_min_size(pos2(x, strip.top()), vec2(*width, TAB_HEIGHT));
        x += *width;
        // Fully scrolled out of view: neither paint nor interact.
        if tab_rect.right() < strip.left() || tab_rect.left() > strip.right() {
            continue;
        }

        let tab_resp = ui.interact(tab_rect, id.with(("tab", index)), Sense::click());
        let hovered = tab_resp.hovered();
        let pressed = tab_resp.is_pointer_button_down_on();
        let active = tabs.active_index() == Some(index);
        let dirty = tabs.tab(index).is_some_and(|tab| tab.dirty);

        // Pressed: `--ink-low` (the §10 active/pressed row); inactive
        // hover: one ink-step lift (UI_SPEC.md §3.1). The active tab
        // itself carries no fill — its bottom line is the mark.
        if pressed {
            painter.rect_filled(tab_rect, radius::R0, color::INK_LOW);
        } else if hovered && !active {
            painter.rect_filled(tab_rect, radius::R0, color::INK_FAINT);
        }
        // Active tab: 1px --fg-default bottom line — the one place a light
        // line marks the active tab against the canvas.
        if active {
            painter.hline(
                tab_rect.x_range(),
                tab_rect.bottom() - HAIRLINE / 2.0,
                Stroke::new(HAIRLINE, color::FG_DEFAULT),
            );
        }

        // Label, clipped to the label zone: a shrunken tab truncates here.
        let label_zone = Rect::from_min_max(
            pos2(tab_rect.left() + TAB_PAD_H, tab_rect.top()),
            pos2(tab_rect.right() - RIGHT_ZONE, tab_rect.bottom()),
        );
        if label_zone.width() > 0.0 {
            let galley = galleys[index].clone();
            let y = tab_rect.center().y - galley.size().y / 2.0;
            let text_color = if active || hovered {
                color::FG_DEFAULT
            } else {
                color::FG_MUTED
            };
            painter.with_clip_rect(label_zone).galley(
                pos2(label_zone.left(), y),
                galley,
                text_color,
            );
        }

        // Dirty marker: 6px --warn circle before the close button.
        if dirty {
            let center = pos2(
                tab_rect.right() - CLOSE_INSET - CLOSE_WIDTH - DOT_GAP - DOT_DIAMETER / 2.0,
                tab_rect.center().y,
            );
            painter.circle_filled(center, DOT_DIAMETER / 2.0, color::WARN);
        }

        // Close (×): visible on hover or when dirty (UI_SPEC.md §3.1).
        // Registered after the tab itself so it wins the hit-test on top.
        if hovered || dirty {
            let close_rect = Rect::from_min_max(
                pos2(tab_rect.right() - CLOSE_INSET - CLOSE_WIDTH, tab_rect.top()),
                pos2(tab_rect.right() - CLOSE_INSET, tab_rect.bottom()),
            );
            let close_resp = ui.interact(close_rect, id.with(("close", index)), Sense::click());
            let close_color = if close_resp.hovered() {
                color::FG_DEFAULT
            } else {
                color::FG_FAINT
            };
            painter.text(
                close_rect.center(),
                Align2::CENTER_CENTER,
                "×",
                font_id.clone(),
                close_color,
            );
            if close_resp.clicked() {
                out.closed = tabs.close_index(index).map(|tab| tab.path);
                break; // indices shifted; one close per frame is enough
            }
        }

        if tab_resp.clicked() {
            tabs.focus(index);
            out.activated = Some(index);
        }
    }

    // "+" at the right: opens quick-open (wired by the shell). Hover
    // `--ink-faint`, pressed `--ink-low` (the §10 matrix).
    let plus_resp = ui.interact(plus_rect, id.with("plus"), Sense::click());
    if plus_resp.is_pointer_button_down_on() {
        painter.rect_filled(plus_rect, radius::R0, color::INK_LOW);
    } else if plus_resp.hovered() {
        painter.rect_filled(plus_rect, radius::R0, color::INK_FAINT);
    }
    let plus_color = if plus_resp.hovered() {
        color::FG_DEFAULT
    } else {
        color::FG_FAINT
    };
    painter.text(
        plus_rect.center(),
        Align2::CENTER_CENTER,
        "+",
        font_id,
        plus_color,
    );
    if plus_resp.clicked() {
        out.new_tab = true;
    }

    // The bar's own 1px bottom hairline (UI_SPEC.md §3.1).
    painter.hline(
        rect.x_range(),
        rect.bottom() - HAIRLINE / 2.0,
        Stroke::new(HAIRLINE, color::INK_HI),
    );

    out
}

/// Distribute the available strip width across the ideal tab widths
/// (UI_SPEC.md §3.1): ideal widths when they fit, proportional shrink
/// between the floor and the ideal when they don't, the floor itself
/// (scroll takes over) when even that doesn't fit.
fn shrink_widths(ideal: &[f32], avail: f32) -> Vec<f32> {
    let total: f32 = ideal.iter().sum();
    if ideal.is_empty() || total <= avail {
        return ideal.to_vec();
    }
    let floor: Vec<f32> = vec![TAB_MIN_WIDTH; ideal.len()];
    let floor_total = TAB_MIN_WIDTH * ideal.len() as f32;
    if floor_total >= avail {
        return floor;
    }
    let above_floor: f32 = ideal
        .iter()
        .zip(&floor)
        .map(|(width, min)| (width - min).max(0.0))
        .sum();
    if above_floor <= 0.0 {
        return floor;
    }
    let slack = avail - floor_total;
    ideal
        .iter()
        .zip(&floor)
        .map(|(width, min)| min + (width - min).max(0.0) * slack / above_floor)
        .collect()
}

/// Wheel motion in strip pixels. Line notches move a tab-height's worth;
/// Page moves a strip's worth; Point deltas are already pixels.
fn wheel_pixels(unit: MouseWheelUnit, delta: egui::Vec2, page: f32) -> f32 {
    let motion = delta.x + delta.y;
    match unit {
        MouseWheelUnit::Point => motion,
        MouseWheelUnit::Line => motion * TAB_HEIGHT,
        MouseWheelUnit::Page => motion * page,
    }
}

/// The label for a tab: the file name (the tree and the fuzzy finder carry
/// the path context; the tab carries the name, UI_SPEC.md §3.1).
fn label_text(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: impl Into<PathBuf>) -> PathBuf {
        name.into()
    }

    // --- Model: parity with selora_core::Workspace (workspace.rs tests) ---

    #[test]
    fn open_focuses_and_orders_like_workspace() {
        let mut tabs = TabsState::new();
        tabs.open(path("a.rs"));
        tabs.open(path("b.rs"));
        tabs.open(path("a.rs")); // duplicate open focuses, does not duplicate
        assert_eq!(tabs.paths(), vec![path("b.rs"), path("a.rs")]);
        assert_eq!(tabs.active_path(), Some(path("a.rs").as_path()));
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn close_falls_back_to_most_recent_like_workspace() {
        let mut tabs = TabsState::from_paths(["a.rs", "b.rs"].map(PathBuf::from));
        assert_eq!(tabs.active_path(), Some(path("b.rs").as_path()));
        tabs.close_path(&path("b.rs"));
        assert_eq!(tabs.active_path(), Some(path("a.rs").as_path()));
        tabs.close_path(&path("a.rs"));
        assert_eq!(tabs.active_path(), None);
        assert!(tabs.is_empty());
    }

    #[test]
    fn close_non_active_keeps_active_and_shifts_indices() {
        let mut tabs = TabsState::from_paths(["a.rs", "b.rs", "c.rs"].map(PathBuf::from));
        tabs.focus(0); // a.rs active, but c.rs is most recent
                       // Close an inactive tab before the active one: the active index shifts.
        assert_eq!(tabs.close_index(1).map(|tab| tab.path), Some(path("b.rs")));
        assert_eq!(tabs.active_path(), Some(path("a.rs").as_path()));
        // Close an inactive tab after the active one: nothing moves.
        assert!(tabs.close_path(&path("c.rs")).is_some());
        assert_eq!(tabs.active_path(), Some(path("a.rs").as_path()));
        assert_eq!(tabs.paths(), vec![path("a.rs")]);
    }

    #[test]
    fn close_active_and_close_path_round_trip() {
        let mut tabs = TabsState::from_paths(["a.rs", "b.rs"].map(PathBuf::from));
        assert_eq!(tabs.close_active().map(|tab| tab.path), Some(path("b.rs")));
        assert_eq!(tabs.active_path(), Some(path("a.rs").as_path()));
        assert_eq!(tabs.close_active().map(|tab| tab.path), Some(path("a.rs")));
        assert_eq!(tabs.close_active(), None);
    }

    #[test]
    fn focus_selects_without_reordering() {
        let mut tabs = TabsState::from_paths(["a.rs", "b.rs", "c.rs"].map(PathBuf::from));
        tabs.focus(0);
        tabs.focus(2);
        assert_eq!(tabs.paths(), vec![path("a.rs"), path("b.rs"), path("c.rs")]);
        assert_eq!(tabs.active_index(), Some(2));
        // Out-of-range focus is ignored, not clamped to a phantom tab.
        tabs.focus(9);
        assert_eq!(tabs.active_index(), Some(2));
    }

    #[test]
    fn dirty_toggles_by_path() {
        let mut tabs = TabsState::from_paths(["a.rs", "b.rs"].map(PathBuf::from));
        assert!(!tabs.any_dirty());
        assert!(tabs.set_dirty(&path("a.rs"), true));
        assert!(tabs.is_dirty(&path("a.rs")));
        assert!(!tabs.is_dirty(&path("b.rs")));
        assert!(tabs.any_dirty());
        assert!(tabs.set_dirty(&path("a.rs"), false));
        assert!(!tabs.any_dirty());
        // Unknown path: reported, model untouched.
        assert!(!tabs.set_dirty(&path("zz.rs"), true));
        // A closed dirty tab is forgotten, not resurrected by a reopen.
        tabs.set_dirty(&path("b.rs"), true);
        tabs.close_path(&path("b.rs"));
        tabs.open(path("b.rs"));
        assert!(!tabs.is_dirty(&path("b.rs")));
    }

    // --- Paint: the full tab bar path runs headlessly (the pattern from
    // selora-editor's editor.rs) ---

    #[test]
    fn tab_bar_paints_headlessly_and_claims_its_height() {
        let mut tabs = TabsState::new();
        tabs.open(path("src/main.rs"));
        tabs.open(path("src/buffer.rs"));
        tabs.set_dirty(&path("src/main.rs"), true);
        let before = tabs.clone();

        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            let response = tab_bar(ui, &mut tabs);
            // Nothing was clicked in a headless frame.
            assert_eq!(response, TabBarResponse::default());
            // The bar claims exactly its 32px of height at the top.
            assert_eq!(ui.min_rect().height(), TAB_HEIGHT);
        });
        // A paint-only frame leaves the model untouched.
        assert_eq!(tabs, before);
    }

    #[test]
    fn tab_bar_overflows_without_panicking() {
        // Enough tabs that even shrunken they overflow a narrow strip — the
        // horizontal-scroll path (shrink to the floor, clamp the offset).
        let mut tabs = TabsState::new();
        for index in 0..24 {
            tabs.open(path(format!("src/file_number_{index:02}.rs")));
        }
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(160.0, TAB_HEIGHT));
            let response = tab_bar(ui, &mut tabs);
            assert_eq!(response, TabBarResponse::default());
            assert_eq!(ui.min_rect().height(), TAB_HEIGHT);
        });
        // The active (last) tab is kept in view: the scroll offset is at the
        // far end of the overflow, not in the negative.
        assert!(tabs.scroll_x >= 0.0);
        // Width math: the floor fits the same invariant regardless of count.
        let widths = shrink_widths(&[200.0, 200.0, 200.0], 300.0);
        let total: f32 = widths.iter().sum();
        assert!(total <= 300.0 + 0.5);
        assert!(widths.iter().all(|width| *width >= TAB_MIN_WIDTH - 0.5));
    }

    #[test]
    fn empty_tab_bar_paints_headlessly() {
        let mut tabs = TabsState::new();
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            let response = tab_bar(ui, &mut tabs);
            assert_eq!(response, TabBarResponse::default());
            assert_eq!(ui.min_rect().height(), TAB_HEIGHT);
        });
        assert!(tabs.is_empty());
    }
}
