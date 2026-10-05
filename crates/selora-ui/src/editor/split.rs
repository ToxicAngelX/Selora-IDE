//! Split containers (UI_SPEC.md §3.3). Phase 2a.
//!
//! The editor area of the workbench is a recursive split tree:
//!
//! ```text
//! SplitNode::Leaf(pane_id)            // one editor pane with its own TabsState
//! SplitNode::Split { axis, ratio, children: [a, b] }
//! ```
//!
//! `Ctrl+\` splits the active leaf into two (a split of a split is a 2x2
//! grid, per UI_SPEC.md §3.3); closing the last tab of a leaf removes the
//! leaf and prunes empty parents. Each leaf owns its own
//! [`TabsState`][crate::editor::tabs::TabsState]. The active leaf is marked
//! with a 1px `--ink-hi` border.
//!
//! The model is pure (all structure operations are unit-tested headlessly);
//! [`show`] is the paint function that draws each pane's tab bar plus the
//! split dividers and reports where the editor content of every pane should
//! be painted. The app shell (not this module) wires `Ctrl+\` / `Ctrl+W` —
//! it calls [`EditorArea::request_split`] / [`EditorArea::request_close`] —
//! and paints the actual [`selora_editor::Editor`] widgets into the
//! reported content rects.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::{pos2, vec2, Rect, Sense, Stroke, StrokeKind, Ui};

use crate::editor::tabs::{self, TabsState};
use crate::theme::{color, radius};

/// Identity of one editor pane (leaf of the split tree). Never reused within
/// a session.
pub type PaneId = u64;

/// Divider between split children: 1px hairline (DESIGN_TOKENS.md §5).
const DIVIDER: f32 = 1.0;
/// All borders are 1px (DESIGN_TOKENS.md §5).
const HAIRLINE: f32 = 1.0;
/// A split's first child never takes less than 10% (nor more than 90%) of
/// the available space — ratios clamp rather than collapse a pane.
pub const RATIO_MIN: f32 = 0.1;
pub const RATIO_MAX: f32 = 0.9;
/// The default split ratio: 50/50 (UI_SPEC.md §3.3 "splits the active
/// editor into two").
pub const RATIO_DEFAULT: f32 = 0.5;

// ---------------------------------------------------------------------------
// Split tree
// ---------------------------------------------------------------------------

/// Which way a split's children are laid out.
///
/// `Horizontal` puts the children side by side (the divider is vertical) —
/// the `Ctrl+\` default. `Vertical` stacks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

/// One node of the split tree (UI_SPEC.md §3.3).
///
/// A `Split` always has exactly two children; when pruning leaves a single
/// child behind, the split collapses into that child, so the invariant is
/// preserved by construction.
#[derive(Debug, Clone, PartialEq)]
pub enum SplitNode {
    /// One editor pane, identified by its [`PaneId`]. The pane's tabs live
    /// in the [`EditorArea`]'s pane map, not in the node.
    Leaf(PaneId),
    /// Two children separated by a 1px divider. `ratio` is the fraction of
    /// the available space (minus the divider) given to the *first* child,
    /// clamped to [`RATIO_MIN`]..[`RATIO_MAX`].
    Split {
        axis: SplitAxis,
        ratio: f32,
        children: Vec<SplitNode>,
    },
}

impl SplitNode {
    /// Clamp a ratio into [`RATIO_MIN`]..[`RATIO_MAX`].
    pub fn clamp_ratio(ratio: f32) -> f32 {
        ratio.clamp(RATIO_MIN, RATIO_MAX)
    }

    /// The leaves of this subtree in paint order (left-to-right, top-to-bottom).
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut leaves = Vec::new();
        self.collect_leaves(&mut leaves);
        leaves
    }

    fn collect_leaves(&self, leaves: &mut Vec<PaneId>) {
        match self {
            SplitNode::Leaf(pane) => leaves.push(*pane),
            SplitNode::Split { children, .. } => {
                for child in children {
                    child.collect_leaves(leaves);
                }
            }
        }
    }

    /// Set this split's ratio, clamped. A no-op on a leaf.
    pub fn set_ratio(&mut self, ratio: f32) {
        if let SplitNode::Split {
            ratio: node_ratio, ..
        } = self
        {
            *node_ratio = Self::clamp_ratio(ratio);
        }
    }

    /// Whether `pane` lives in this subtree.
    fn contains_pane(&self, pane: PaneId) -> bool {
        match self {
            SplitNode::Leaf(id) => *id == pane,
            SplitNode::Split { children, .. } => {
                children.iter().any(|child| child.contains_pane(pane))
            }
        }
    }
}

/// Replace the `Leaf(pane)` inside `node` with `replacement`. Panes are
/// unique and the pane is known to be present, so the replacement is
/// consumed by exactly one leaf; the search is exhaustive for safety.
fn replace_leaf(node: &mut SplitNode, pane: PaneId, replacement: SplitNode) {
    match node {
        SplitNode::Leaf(id) => {
            if *id == pane {
                *node = replacement;
            }
        }
        SplitNode::Split { children, .. } => {
            for child in children {
                if child.contains_pane(pane) {
                    replace_leaf(child, pane, replacement);
                    return; // ownership consumed by the recursion
                }
            }
        }
    }
}

/// Prune one split subtree: drop leaves whose pane has no tabs, collapse
/// single-child splits, and report whether anything worth keeping remains.
fn prune_node(node: &mut SplitNode, panes: &mut BTreeMap<PaneId, TabsState>) -> bool {
    match node {
        SplitNode::Leaf(pane) => panes.get(pane).is_some_and(|tabs| !tabs.is_empty()),
        SplitNode::Split { children, .. } => {
            children.retain_mut(|child| prune_node(child, panes));
            match children.as_slice() {
                [] => false,
                [only] => {
                    let collapsed = only.clone();
                    *node = collapsed;
                    true
                }
                _ => true,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Editor area (the split tree + the panes)
// ---------------------------------------------------------------------------

/// The whole editor region of the workbench: the split tree plus the pane
/// map that gives every leaf its own tabs (UI_SPEC.md §3.3) and the active
/// pane.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorArea {
    root: SplitNode,
    panes: BTreeMap<PaneId, TabsState>,
    active: PaneId,
    next_id: PaneId,
}

impl Default for EditorArea {
    /// One empty pane (pane 0), nothing split — the "no file open" state.
    fn default() -> Self {
        Self {
            root: SplitNode::Leaf(0),
            panes: BTreeMap::from([(0, TabsState::new())]),
            active: 0,
            next_id: 1,
        }
    }
}

impl EditorArea {
    /// One empty pane, nothing split. The `Default` of the editor area.
    pub fn new() -> Self {
        Self::default()
    }

    // -- structure ---------------------------------------------------------

    /// The split tree (read-only view; structure changes go through
    /// [`Self::request_split`] and [`Self::request_close`]).
    pub fn root(&self) -> &SplitNode {
        &self.root
    }

    /// The split tree, mutable — for ratio adjustments via
    /// [`SplitNode::set_ratio`].
    pub fn root_mut(&mut self) -> &mut SplitNode {
        &mut self.root
    }

    /// All pane ids in paint order.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.root.leaves()
    }

    /// The tabs of one pane.
    pub fn pane(&self, pane: PaneId) -> Option<&TabsState> {
        self.panes.get(&pane)
    }

    /// The tabs of one pane, mutable.
    pub fn pane_mut(&mut self, pane: PaneId) -> Option<&mut TabsState> {
        self.panes.get_mut(&pane)
    }

    /// The active pane's tabs, mutable — where a newly opened file lands.
    pub fn active_pane_mut(&mut self) -> Option<&mut TabsState> {
        self.panes.get_mut(&self.active)
    }

    // -- active pane -------------------------------------------------------

    /// The active pane id. The active pane carries the 1px `--ink-hi`
    /// border and receives `Ctrl+\` / `Ctrl+W` / newly opened files.
    pub fn active_pane_id(&self) -> PaneId {
        self.active
    }

    /// Focus a pane (clicking its tab bar or its content). Unknown ids are
    /// ignored.
    pub fn set_active_pane(&mut self, pane: PaneId) {
        if self.panes.contains_key(&pane) {
            self.active = pane;
        }
    }

    // -- operations the app shell wires to keys/commands --------------------

    /// Open (or focus) `path` as a tab in the active pane.
    pub fn open_in_active(&mut self, path: impl Into<PathBuf>) {
        if let Some(tabs) = self.active_pane_mut() {
            tabs.open(path);
        }
    }

    /// Mark the tab for `path` dirty (or clean) in every pane that shows it
    /// — the same file open in two panes shows the same dirty state.
    pub fn set_dirty(&mut self, path: &Path, dirty: bool) {
        for tabs in self.panes.values_mut() {
            tabs.set_dirty(path, dirty);
        }
    }

    /// `Ctrl+\`: split the active leaf into two (UI_SPEC.md §3.3). The new
    /// pane receives a copy of the active pane's tabs — every split has its
    /// own tabs, and the buffer itself is shared by path in the app shell —
    /// and becomes the active pane. Splitting an empty pane is a no-op
    /// (there is nothing to show in the new leaf; it would prune itself
    /// immediately).
    pub fn request_split(&mut self) -> Option<PaneId> {
        self.request_split_axis(SplitAxis::Horizontal)
    }

    /// [`Self::request_split`] with an explicit axis.
    pub fn request_split_axis(&mut self, axis: SplitAxis) -> Option<PaneId> {
        let active = self.active;
        // Snapshot the tabs first: the new pane is a copy, and cloning up
        // front keeps the pane-map borrows sequential.
        let tabs = self.panes.get(&active)?.clone();
        if tabs.is_empty() {
            return None;
        }
        let pane = self.fresh_pane();
        self.panes.insert(pane, tabs);
        replace_leaf(
            &mut self.root,
            active,
            SplitNode::Split {
                axis,
                ratio: RATIO_DEFAULT,
                children: vec![SplitNode::Leaf(active), SplitNode::Leaf(pane)],
            },
        );
        self.active = pane;
        Some(pane)
    }

    /// `Ctrl+W`: close the active tab of the active pane. If that was the
    /// pane's last tab, the pane is removed and empty parents are pruned
    /// (UI_SPEC.md §3.3). Returns the closed path (the shell closes the
    /// buffer); `None` when there was no tab to close.
    pub fn request_close(&mut self) -> Option<PathBuf> {
        let closed = self
            .active_pane_mut()
            .and_then(|tabs| tabs.close_active())
            .map(|tab| tab.path);
        if closed.is_some() {
            self.prune();
        }
        closed
    }

    /// Drop every split leaf whose pane has no tabs left, collapse
    /// single-child splits, forget orphaned panes, and keep the active pane
    /// valid. The root leaf itself is never removed — an editor area with
    /// no tabs is *one* empty pane, not zero.
    pub fn prune(&mut self) {
        let mut root = std::mem::replace(&mut self.root, SplitNode::Leaf(0));
        if let SplitNode::Split { children, .. } = &mut root {
            children.retain_mut(|child| prune_node(child, &mut self.panes));
            match children.as_slice() {
                [] => {
                    // Every pane emptied: keep exactly one (empty) pane.
                    let pane = self.fresh_pane();
                    self.panes.insert(pane, TabsState::new());
                    root = SplitNode::Leaf(pane);
                }
                [only] => {
                    let collapsed = only.clone();
                    root = collapsed;
                }
                _ => {}
            }
        }
        self.root = root;

        // Forget panes that are no longer in the tree.
        let live = self.root.leaves();
        self.panes.retain(|pane, _| live.contains(pane));

        // Keep the active pane valid: focus the first pane in paint order.
        if !self.panes.contains_key(&self.active) {
            self.active = live.first().copied().unwrap_or(0);
        }
    }

    fn fresh_pane(&mut self) -> PaneId {
        let pane = self.next_id;
        self.next_id += 1;
        pane
    }
}

// ---------------------------------------------------------------------------
// Paint
// ---------------------------------------------------------------------------

/// Where one pane lives this frame: the rect the shell should paint the
/// [`selora_editor::Editor`] into, plus the pane's tab bar rect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneView {
    pub pane: PaneId,
    /// The pane's tab bar (already painted by [`show`]).
    pub tabs: Rect,
    /// The editor canvas below the tab bar. The shell paints the editor
    /// widget here and calls [`EditorArea::set_active_pane`] when the user
    /// interacts with it (the editor widget, painted later, wins the
    /// hit-test over the pane's background).
    pub content: Rect,
}

/// What the user did to the editor area this frame. The model has already
/// been updated in place; the shell turns these into effects.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SplitResponse {
    /// Every pane in paint order, with its content rect.
    pub panes: Vec<PaneView>,
    /// Tabs closed this frame (via a pane's × button). Already gone from
    /// the model; the shell should close the corresponding buffers.
    pub closed: Vec<PathBuf>,
    /// Panes whose "+" (quick-open) button was clicked.
    pub new_tab: Vec<PaneId>,
    /// The pane that became active by a click this frame, if any.
    pub activated: Option<PaneId>,
}

/// Paint the whole editor area into the next available position in `ui`
/// (UI_SPEC.md §3.3): every pane's tab bar, the 1px dividers between split
/// children, and the 1px `--ink-hi` border around the active pane. Panes
/// whose last tab was closed this frame are pruned before returning.
pub fn show(ui: &mut Ui, area: &mut EditorArea) -> SplitResponse {
    let avail = ui.available_rect_before_wrap();
    let size = vec2(ui.available_width().max(0.0), avail.height().max(0.0));
    let rect = Rect::from_min_size(avail.min, size);
    ui.allocate_rect(rect, Sense::hover());

    let mut out = SplitResponse::default();
    // The tree is only mutated by the model operations, never mid-paint;
    // walking a snapshot keeps the pane-map borrows disjoint.
    let root = area.root.clone();
    show_node(ui, &root, area, rect, &mut out);
    area.prune();
    out
}

fn show_node(
    ui: &Ui,
    node: &SplitNode,
    area: &mut EditorArea,
    rect: Rect,
    out: &mut SplitResponse,
) {
    match node {
        SplitNode::Leaf(pane) => show_leaf(ui, area, *pane, rect, out),
        SplitNode::Split {
            axis,
            ratio,
            children,
        } => {
            let (first, second, divider) = split_rects(rect, *axis, *ratio);
            // 1px hairline between split children (DESIGN_TOKENS.md §5).
            ui.painter_at(rect)
                .rect_filled(divider, radius::R0, color::INK_HI);
            if let Some(child) = children.first() {
                show_node(ui, child, area, first, out);
            }
            if let Some(child) = children.get(1) {
                show_node(ui, child, area, second, out);
            }
        }
    }
}

fn show_leaf(ui: &Ui, area: &mut EditorArea, pane: PaneId, rect: Rect, out: &mut SplitResponse) {
    let was_active = area.active_pane_id() == pane;
    let tabs_rect = Rect::from_min_size(rect.min, vec2(rect.width(), tabs::TAB_HEIGHT));
    let content = Rect::from_min_max(pos2(rect.left(), tabs_rect.bottom()), rect.max);
    let id = ui.id().with(pane);

    // The pane's own tab bar (UI_SPEC.md §3.1). A pane missing from the map
    // cannot happen while the tree and map are in sync; paint a bare canvas
    // anyway so the frame never has a hole.
    let mut clicked_tab = None;
    let mut closed = None;
    let mut new_tab = false;
    if let Some(tabs) = area.pane_mut(pane) {
        let bar = tabs::tab_bar_at(ui, tabs, tabs_rect, id);
        clicked_tab = bar.activated;
        closed = bar.closed;
        new_tab = bar.new_tab;
    } else {
        ui.painter_at(tabs_rect)
            .rect_filled(tabs_rect, radius::R0, color::INK_CANVAS);
    }

    // Clicking a tab or the empty canvas of a pane focuses it. (Clicks on a
    // live editor widget are won by the widget itself — painted later, on
    // top — which is why the shell also calls `set_active_pane` on editor
    // interaction.)
    let content_resp = ui.interact(content, id.with("content"), Sense::click());
    if clicked_tab.is_some() || content_resp.clicked() {
        area.set_active_pane(pane);
        if !was_active {
            out.activated = Some(pane);
        }
    }
    if let Some(path) = closed {
        out.closed.push(path);
    }
    if new_tab {
        out.new_tab.push(pane);
    }

    // Editor canvas: --ink-canvas, no padding (UI_SPEC.md §3.2).
    let painter = ui.painter_at(rect);
    painter.rect_filled(content, radius::R0, color::INK_CANVAS);
    // The active pane's 1px --ink-hi border (UI_SPEC.md §3.3).
    if area.active_pane_id() == pane {
        painter.rect_stroke(
            rect,
            radius::R0,
            Stroke::new(HAIRLINE, color::INK_HI),
            StrokeKind::Inside,
        );
    }

    out.panes.push(PaneView {
        pane,
        tabs: tabs_rect,
        content,
    });
}

/// Split `rect` along `axis` at `ratio`, reserving the 1px divider.
fn split_rects(rect: Rect, axis: SplitAxis, ratio: f32) -> (Rect, Rect, Rect) {
    let ratio = SplitNode::clamp_ratio(ratio);
    match axis {
        SplitAxis::Horizontal => {
            let first_width = (rect.width() - DIVIDER).max(0.0) * ratio;
            let split_x = rect.left() + first_width;
            (
                Rect::from_min_max(rect.min, pos2(split_x, rect.bottom())),
                Rect::from_min_max(pos2(split_x + DIVIDER, rect.top()), rect.max),
                Rect::from_min_max(
                    pos2(split_x, rect.top()),
                    pos2(split_x + DIVIDER, rect.bottom()),
                ),
            )
        }
        SplitAxis::Vertical => {
            let first_height = (rect.height() - DIVIDER).max(0.0) * ratio;
            let split_y = rect.top() + first_height;
            (
                Rect::from_min_max(rect.min, pos2(rect.right(), split_y)),
                Rect::from_min_max(pos2(rect.left(), split_y + DIVIDER), rect.max),
                Rect::from_min_max(
                    pos2(rect.left(), split_y),
                    pos2(rect.right(), split_y + DIVIDER),
                ),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    // --- Model: split / prune ---

    #[test]
    fn split_creates_two_leaves_with_own_tabs() {
        let mut area = EditorArea::new();
        area.open_in_active("a.rs");
        let new_pane = area.request_split().expect("split of a live pane");

        // Structure: the root is a 50/50 horizontal split of two leaves.
        match area.root() {
            SplitNode::Split {
                axis,
                ratio,
                children,
            } => {
                assert_eq!(*axis, SplitAxis::Horizontal);
                assert!((*ratio - RATIO_DEFAULT).abs() < f32::EPSILON);
                assert_eq!(children.len(), 2);
                assert_eq!(area.root().leaves(), vec![0, new_pane]);
            }
            other => panic!("expected a split, got {other:?}"),
        }
        // Every split has its own tabs (UI_SPEC.md §3.3): the new pane got
        // a copy, and the new pane is active (focus follows the split).
        assert_eq!(
            area.pane(new_pane).expect("pane exists").paths(),
            vec![path("a.rs")]
        );
        assert_eq!(area.active_pane_id(), new_pane);
        assert_eq!(area.pane_ids().len(), 2);
    }

    #[test]
    fn split_of_split_makes_a_grid() {
        let mut area = EditorArea::new();
        area.open_in_active("a.rs");
        area.request_split(); // panes 0 | 1
        area.set_active_pane(0);
        area.request_split(); // (0 | 2) | 1
        area.set_active_pane(1);
        area.request_split(); // (0 | 2) | (1 | 3)
        assert_eq!(area.root().leaves(), vec![0, 2, 1, 3]);
        assert_eq!(area.pane_ids().len(), 4);
    }

    #[test]
    fn split_of_empty_pane_is_a_noop() {
        let mut area = EditorArea::new();
        assert_eq!(area.request_split(), None);
        assert!(matches!(area.root(), SplitNode::Leaf(_)));
        assert_eq!(area.pane_ids(), vec![0]);
    }

    #[test]
    fn closing_all_tabs_of_a_nested_split_collapses_it() {
        let mut area = EditorArea::new();
        area.open_in_active("a.rs");
        area.open_in_active("b.rs");
        area.request_split(); // panes 0 | 1, both showing a.rs + b.rs
        assert!(matches!(area.root(), SplitNode::Split { .. }));

        // Close every tab of the (active) new pane.
        assert_eq!(area.request_close(), Some(path("b.rs")));
        assert_eq!(area.request_close(), Some(path("a.rs")));

        // The split collapsed: back to a single leaf, the empty pane is gone
        // from the tree and the map, and focus fell back to the survivor.
        assert!(matches!(area.root(), SplitNode::Leaf(pane) if *pane == 0));
        assert_eq!(area.pane_ids(), vec![0]);
        assert_eq!(area.panes_len_for_test(), 1);
        assert_eq!(area.active_pane_id(), 0);
        // The surviving pane kept its tabs.
        assert_eq!(area.pane(0).expect("pane").len(), 2);
    }

    #[test]
    fn closing_the_last_tab_of_a_nested_split_collapses_parents() {
        let mut area = EditorArea::new();
        area.open_in_active("a.rs");
        area.request_split(); // (0 | 1)
        area.set_active_pane(0);
        area.request_split(); // ((0 | 2) | 1)
                              // Empty out the deeply nested pane 2: the inner split collapses and
                              // the outer split's remaining child takes its place.
        while area.active_pane_id() == 2 {
            area.request_close();
        }
        let leaves = area.root().leaves();
        assert_eq!(leaves.len(), 2);
        assert!(leaves.contains(&0) && leaves.contains(&1));
        // No split ever keeps one child.
        assert_split_invariant(area.root());
    }

    #[test]
    fn closing_the_last_tab_everywhere_keeps_one_empty_pane() {
        let mut area = EditorArea::new();
        area.open_in_active("a.rs");
        assert_eq!(area.request_close(), Some(path("a.rs")));
        // An editor area with no tabs is one empty pane, not zero.
        assert!(matches!(area.root(), SplitNode::Leaf(_)));
        assert_eq!(area.pane_ids().len(), 1);
        assert_eq!(area.active_pane_id(), area.pane_ids()[0]);
        assert!(area.pane(area.active_pane_id()).expect("pane").is_empty());
    }

    #[test]
    fn ratios_clamp() {
        let mut node = SplitNode::Split {
            axis: SplitAxis::Horizontal,
            ratio: RATIO_DEFAULT,
            children: vec![SplitNode::Leaf(0), SplitNode::Leaf(1)],
        };
        node.set_ratio(2.0);
        assert!((node_ratio(&node) - RATIO_MAX).abs() < f32::EPSILON);
        node.set_ratio(-1.0);
        assert!((node_ratio(&node) - RATIO_MIN).abs() < f32::EPSILON);
        node.set_ratio(0.42);
        assert!((node_ratio(&node) - 0.42).abs() < f32::EPSILON);
        // set_ratio on a leaf is a no-op.
        let mut leaf = SplitNode::Leaf(7);
        leaf.set_ratio(0.5);
        assert_eq!(leaf, SplitNode::Leaf(7));
    }

    #[test]
    fn request_close_falls_back_within_the_pane() {
        let mut area = EditorArea::new();
        area.open_in_active("a.rs");
        area.open_in_active("b.rs");
        // Ctrl+W closes the active (most recent) tab and falls back to the
        // previous one — the tab model's focus-order semantics.
        assert_eq!(area.request_close(), Some(path("b.rs")));
        assert_eq!(
            area.pane(0).expect("pane").active_path(),
            Some(path("a.rs").as_path())
        );
    }

    #[test]
    fn dirty_state_is_shared_across_panes_by_path() {
        let mut area = EditorArea::new();
        area.open_in_active("a.rs");
        area.request_split();
        area.set_dirty(&path("a.rs"), true);
        for pane in area.pane_ids() {
            assert!(area.pane(pane).expect("pane").is_dirty(&path("a.rs")));
        }
    }

    /// A `Split` never has zero or one child (the collapse invariant).
    fn assert_split_invariant(node: &SplitNode) {
        if let SplitNode::Split { children, .. } = node {
            assert_eq!(children.len(), 2, "split with {} children", children.len());
            for child in children {
                assert_split_invariant(child);
            }
        }
    }

    fn node_ratio(node: &SplitNode) -> f32 {
        match node {
            SplitNode::Split { ratio, .. } => *ratio,
            SplitNode::Leaf(_) => panic!("expected a split"),
        }
    }

    impl EditorArea {
        /// Test-only view of the pane map size (pruning must forget panes).
        fn panes_len_for_test(&self) -> usize {
            self.panes.len()
        }
    }

    // --- Paint: headless, the pattern from selora-editor's editor.rs ---

    #[test]
    fn split_paints_two_panes_side_by_side() {
        let mut area = EditorArea::new();
        area.open_in_active("a.rs");
        area.request_split();

        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            let response = show(ui, &mut area);
            assert_eq!(response.panes.len(), 2);
            assert!(response.closed.is_empty());
            assert!(response.new_tab.is_empty());

            let (first, second) = (response.panes[0], response.panes[1]);
            // Side by side, split by a 1px divider at the default ratio.
            assert!((first.content.width() - second.content.width()).abs() < 1.0);
            assert!(second.content.left() >= first.content.right() - 0.5);
            // Each pane is a tab bar on top of the content canvas.
            assert_eq!(first.tabs.height(), tabs::TAB_HEIGHT);
            assert_eq!(first.content.top(), first.tabs.bottom());
            assert!(first.content.height() > 0.0);
            // Both panes kept their tabs through the paint.
            assert_eq!(area.pane(first.pane).expect("pane").len(), 1);
            assert_eq!(area.pane(second.pane).expect("pane").len(), 1);
        });
    }

    #[test]
    fn single_pane_paints_headlessly() {
        let mut area = EditorArea::new();
        egui::__run_test_ui(|ui| {
            ui.set_max_size(vec2(640.0, 480.0));
            let response = show(ui, &mut area);
            assert_eq!(response.panes.len(), 1);
            // An empty pane is still a pane with a tab bar and a canvas.
            assert_eq!(response.panes[0].tabs.height(), tabs::TAB_HEIGHT);
            assert!(response.panes[0].content.height() > 0.0);
        });
    }
}
