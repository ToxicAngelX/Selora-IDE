//! Search-in-files panel (UI_SPEC.md §9) — Phase 3c.
//!
//! The shell owns the surrounding panel surface (region ④ in UI_SPEC.md §2,
//! shown when the `search` activity is selected in the activity bar); this
//! widget paints the panel's content:
//!
//! - the 24px "SEARCH" header (`--type-xs`, `--fg-faint`),
//! - the 32px query bar: a full-width input with a `×` clear affordance
//!   and the `case` (`Aa`) / `word` (`\b`) toggles; the `⌄` regex expander
//!   is permanently disabled in Lite and reports "coming soon"
//!   (UI_SPEC.md §9),
//! - results grouped by file: a group header (chevron + path + hit count)
//!   over 20px line rows with the line number in `--fg-faint` and the
//!   matched substring highlighted on a `--syn-string` background,
//! - the footer summary ("3 results · 2 files").
//!
//! The panel owns only *view* state (query text, toggles, collapsed
//! groups, selection); the data flow is the Phase 2 effect/message
//! pattern: [`SearchEvent::RunSearch`] goes out, the worker runs
//! `selora_fs::search::search_in_files` off the frame path, and the
//! finished hits come back through `AppMessage::SearchResults` into
//! [`SearchPanel::set_results`] (stale replies are dropped by
//! [`SearchId`]).
//!
//! Every color, font, and spacing value comes from `crate::theme` (the
//! typed DESIGN_TOKENS.md contract); nothing is hardcoded here
//! (DESIGN_TOKENS.md §12).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use egui::{pos2, vec2, Align2, Key, Rect, Sense, Ui, Vec2};

use selora_fs::fuzzy::display_path;
use selora_fs::search::{MatchSpan, SearchHit, SearchId, SearchQuery};

use crate::theme::{color, font, radius, space, type_scale};

/// Header bar height (UI_SPEC.md §2.2 header treatment; §9 panel).
pub const HEADER_HEIGHT: f32 = 24.0;
/// The panel's sidebar label (UI_SPEC.md §9).
pub const HEADER_LABEL: &str = "SEARCH";
/// Query bar height (UI_SPEC.md §9: "Query bar: 32px tall").
pub const QUERY_BAR_HEIGHT: f32 = 32.0;
/// Toggle chip size (28×24 tap targets, `--radius-1`).
pub const TOGGLE_WIDTH: f32 = 28.0;
pub const TOGGLE_HEIGHT: f32 = 24.0;
/// File-group header row height.
pub const GROUP_HEADER_HEIGHT: f32 = 22.0;
/// Result line row height: `--type-md` line height (DESIGN_TOKENS.md §2.2).
pub const ROW_HEIGHT: f32 = 20.0;
/// Line-number column width inside a result row.
pub const LINE_NUMBER_WIDTH: f32 = 40.0;
/// Footer height (the results summary line).
pub const FOOTER_HEIGHT: f32 = 24.0;
/// The query input's placeholder.
const QUERY_HINT: &str = "Search";
/// The regex expander's coming-soon notice (UI_SPEC.md §9: regex is v1.x).
const REGEX_COMING_SOON: &str = "Regex search arrives in v1.x";

// -----------------------------------------------------------------------------
// Events
// -----------------------------------------------------------------------------

/// What the panel tells the shell each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchEvent {
    /// The user submitted a search (Enter, or a toggle flip with a live
    /// query). The shell resolves the workspace root and dispatches
    /// `Effect::Search { id, root, query }`; results return via
    /// `AppMessage::SearchResults(id, hits)`.
    RunSearch { id: SearchId, query: SearchQuery },
    /// A result row was clicked — open `path` at `line` (1-based).
    OpenResultAtLine { path: PathBuf, line: usize },
    /// An informational notice the shell should surface as a toast
    /// (e.g. the disabled regex expander).
    Info(String),
}

/// Result of one `show` call.
#[derive(Debug, Default)]
pub struct SearchOutcome {
    /// Events for the shell, in frame order.
    pub events: Vec<SearchEvent>,
    /// Rows painted (group headers + result rows; exposed for the
    /// headless paint test — the shell ignores it).
    pub rows_painted: usize,
}

// -----------------------------------------------------------------------------
// Result grouping
// -----------------------------------------------------------------------------

/// One file's hits, in arrival order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchGroup {
    /// The file's path, as the search engine reported it.
    pub path: PathBuf,
    /// Display form: workspace-relative with `/` separators when the
    /// workspace root is known, else the full display path.
    pub display: String,
    /// The file's matching lines.
    pub hits: Vec<SearchHit>,
}

// -----------------------------------------------------------------------------
// SearchPanel
// -----------------------------------------------------------------------------

/// The search-in-files panel. Create one per app and call
/// [`SearchPanel::show`] every frame from the shell's panel surface.
pub struct SearchPanel {
    /// The query input's text.
    query: String,
    /// The `case` (Aa) toggle.
    case_sensitive: bool,
    /// The `word` (`\b`) toggle.
    whole_word: bool,
    /// The workspace root, for relative result paths.
    root: Option<PathBuf>,
    /// The id of the in-flight (or last completed) search.
    last_id: Option<SearchId>,
    /// Whether a search is in flight (cleared by `set_results` /
    /// `search_failed`).
    running: bool,
    /// Results, grouped by file.
    groups: Vec<SearchGroup>,
    /// Collapsed file groups.
    collapsed: BTreeSet<PathBuf>,
    /// The selected result row (path, line).
    selected: Option<(PathBuf, usize)>,
    /// Focus the query input on the next frame (Ctrl+Shift+F wiring).
    want_focus: bool,
}

impl Default for SearchPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchPanel {
    /// An empty panel with no results.
    pub fn new() -> Self {
        Self {
            query: String::new(),
            case_sensitive: false,
            whole_word: false,
            root: None,
            last_id: None,
            running: false,
            groups: Vec::new(),
            collapsed: BTreeSet::new(),
            selected: None,
            want_focus: false,
        }
    }

    // -- shell-facing API ----------------------------------------------------

    /// The workspace root (result paths display relative to it). Called
    /// when a folder opens; recomputes the groups' display strings.
    pub fn set_root(&mut self, root: PathBuf) {
        self.root = Some(root);
        let root = self.root.as_deref();
        for group in &mut self.groups {
            group.display = display_for(root, &group.path);
        }
    }

    /// Install the results of search `id` (from `AppMessage::SearchResults`).
    /// A stale reply — one whose id is not the current
    /// [`SearchEvent::RunSearch`] — is dropped: a second `Enter` before the
    /// first worker replies must not paint the first search's results.
    pub fn set_results(&mut self, id: SearchId, hits: Vec<SearchHit>) {
        if self.last_id != Some(id) {
            return;
        }
        self.running = false;
        self.regroup(hits);
    }

    /// A search failed (from `AppMessage::SearchFailed`); the shell toasts
    /// the error, the panel just stops "searching…".
    pub fn search_failed(&mut self, id: SearchId) {
        if self.last_id == Some(id) {
            self.running = false;
        }
    }

    /// Whether a search is in flight.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Whether any search has been submitted this session (the footer's
    /// visibility condition).
    pub fn has_run(&self) -> bool {
        self.last_id.is_some()
    }

    /// The query input's current text.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Set the query text programmatically (tests, agent-mode integration).
    /// Does not run a search; the next Enter or toggle does.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_owned();
    }

    /// The `case` / `word` toggle states.
    pub fn toggles(&self) -> (bool, bool) {
        (self.case_sensitive, self.whole_word)
    }

    /// Set the `case` toggle programmatically (tests, keybinds).
    pub fn set_case_sensitive(&mut self, on: bool) {
        self.case_sensitive = on;
    }

    /// Submit the current query (the Enter path; also a shell-facing seam
    /// for keybinds and agent-mode integration). Returns the emitted
    /// [`SearchEvent::RunSearch`], or `None` when the query is empty
    /// (previous results are cleared either way).
    pub fn submit_query(&mut self) -> Option<SearchEvent> {
        let mut events = Vec::new();
        self.emit_run(&mut events);
        events.pop()
    }

    /// Focus the query input on the next frame (the `Ctrl+Shift+F`
    /// keybind / activity-bar switch).
    pub fn focus_query(&mut self) {
        self.want_focus = true;
    }

    /// The grouped results (groups in arrival order, hits in engine
    /// order within each).
    pub fn groups(&self) -> &[SearchGroup] {
        &self.groups
    }

    /// Number of file groups in the current results.
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Total hits across all groups.
    pub fn hit_count(&self) -> usize {
        self.groups.iter().map(|group| group.hits.len()).sum()
    }

    /// Collapse or expand a file group.
    pub fn toggle_group(&mut self, path: &Path) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_path_buf());
        }
    }

    // -- painting ------------------------------------------------------------

    /// Paint the panel content. Returns the frame's events plus a row
    /// count (used by the headless test).
    pub fn show(&mut self, ui: &mut Ui) -> SearchOutcome {
        let mut events = Vec::new();
        self.paint_header(ui);
        self.paint_query_bar(ui, &mut events);
        let rows_painted = self.paint_results(ui, &mut events);
        self.paint_footer(ui);
        SearchOutcome {
            events,
            rows_painted,
        }
    }

    /// The 24px "SEARCH" header (UI_SPEC.md §2.2 header treatment).
    fn paint_header(&mut self, ui: &mut Ui) {
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
        ui.painter().text(
            pos2(rect.left() + space::S3, rect.center().y),
            Align2::LEFT_CENTER,
            HEADER_LABEL,
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_FAINT,
        );
    }

    /// The 32px query bar: input, `×` clear, `Aa` / `\b` toggles, the
    /// disabled `⌄` regex expander (UI_SPEC.md §9).
    fn paint_query_bar(&mut self, ui: &mut Ui, events: &mut Vec<SearchEvent>) {
        let want_focus = self.want_focus;
        let mut submit = false;
        let mut clear_clicked = false;
        let mut case_clicked = false;
        let mut word_clicked = false;
        let mut regex_clicked = false;

        ui.horizontal(|row| {
            row.set_min_height(QUERY_BAR_HEIGHT);
            row.add_space(space::S3);

            // [input] — full width minus the trailing controls and the
            // 12px right padding (UI_SPEC.md §9: "Input is full-width
            // minus 24px padding").
            let reserved = TOGGLE_WIDTH + 3.0 * TOGGLE_WIDTH + 3.0 * space::S1 + space::S3;
            let response = row.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(egui::Id::new("selora_search_query"))
                    .hint_text(QUERY_HINT)
                    .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS))
                    .desired_width((row.available_width() - reserved).max(40.0)),
            );
            if want_focus {
                response.request_focus();
            }

            // [×] — clears the query and the results; visible only while
            // there is something to clear.
            let (rect, clear_response) =
                row.allocate_exact_size(vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT), Sense::click());
            if !self.query.is_empty() {
                let icon_color = if clear_response.hovered() {
                    color::FG_DEFAULT
                } else {
                    color::FG_FAINT
                };
                paint_x(row.painter(), rect, icon_color);
                clear_clicked |= clear_response.clicked();
            }

            // [Aa] case toggle.
            let case_response = chip(row, "Aa", self.case_sensitive, true);
            case_clicked |= case_response.clicked();

            // [\b] word toggle.
            let word_response = chip(row, "\\b", self.whole_word, true);
            word_clicked |= word_response.clicked();

            // [⌄] regex expander: disabled in Lite; a click reports
            // "coming soon" (UI_SPEC.md §9).
            let regex_response = chip(row, "⌄", false, false);
            regex_clicked |= regex_response.clicked();

            // Enter submits. The single-line edit surrenders focus on
            // Enter; `lost_focus` + the key still being pressed is the
            // documented egui submit signal (text_edit docs), and it
            // guarantees Enter only fires when the query input owned the
            // keyboard this frame.
            submit = response.lost_focus()
                && row.input(|i| i.key_pressed(Key::Enter) && i.modifiers.is_none());
        });
        self.want_focus = false;

        if clear_clicked {
            self.query.clear();
            self.groups.clear();
            self.running = false;
            self.last_id = None;
        }
        if case_clicked {
            self.case_sensitive = !self.case_sensitive;
        }
        if word_clicked {
            self.whole_word = !self.whole_word;
        }
        if regex_clicked {
            events.push(SearchEvent::Info(REGEX_COMING_SOON.to_owned()));
        }
        // Enter always runs; a toggle flip re-runs only with a live query
        // (matching the VS Code behavior of applying toggles to the
        // current search).
        let rerun = (case_clicked || word_clicked) && !self.query.is_empty();
        if submit || rerun {
            self.emit_run(events);
            if submit {
                self.want_focus = true; // keep typing after a search
            }
        }
    }

    /// The grouped results in a vertical scroll (group header + line rows).
    fn paint_results(&mut self, ui: &mut Ui, events: &mut Vec<SearchEvent>) -> usize {
        if self.groups.is_empty() {
            return 0;
        }

        // Click targets are applied after painting so the paint loop holds
        // only shared borrows of the result data.
        let collapsed = self.collapsed.clone();
        let selected = self.selected.clone();
        let mut open: Option<(PathBuf, usize)> = None;
        let mut toggle: Option<PathBuf> = None;
        let mut rows_painted = 0;

        egui::ScrollArea::vertical()
            .id_salt("selora_search_results")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for group in &self.groups {
                    // -- group header: chevron + path + hit count -------
                    let (rect, response) = ui.allocate_exact_size(
                        vec2(ui.available_width(), GROUP_HEADER_HEIGHT),
                        Sense::click(),
                    );
                    let is_collapsed = collapsed.contains(&group.path);
                    let hovered = response.hovered();

                    let text_color = if hovered {
                        color::FG_DEFAULT
                    } else {
                        color::FG_MUTED
                    };
                    let icon_y = rect.top() + (GROUP_HEADER_HEIGHT - 16.0) / 2.0;
                    paint_chevron(
                        ui.painter(),
                        Rect::from_min_size(
                            pos2(rect.left() + space::S1, icon_y),
                            Vec2::splat(16.0),
                        ),
                        color::FG_FAINT,
                        !is_collapsed,
                    );
                    ui.painter().text(
                        pos2(rect.left() + 2.0 * 16.0 + space::S1, rect.center().y),
                        Align2::LEFT_CENTER,
                        group.display.as_str(),
                        egui::FontId::new(type_scale::SM, font::FAMILY_MONO),
                        text_color,
                    );
                    let count = group.hits.len().to_string();
                    ui.painter().text(
                        pos2(rect.right() - space::S2, rect.center().y),
                        Align2::RIGHT_CENTER,
                        count,
                        egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                        color::FG_FAINT,
                    );
                    if response.clicked() {
                        toggle = Some(group.path.clone());
                    }
                    rows_painted += 1;

                    if is_collapsed {
                        continue;
                    }

                    // -- one line row per hit ----------------------------
                    for hit in &group.hits {
                        rows_painted += paint_result_row(ui, hit, &selected, &mut open);
                    }
                }
            });

        if let Some(path) = toggle {
            self.toggle_group(&path);
        }
        if let Some((path, line)) = open {
            self.selected = Some((path.clone(), line));
            events.push(SearchEvent::OpenResultAtLine { path, line });
        }
        rows_painted
    }

    /// The footer summary ("3 results · 2 files", or "searching…" while a
    /// search is in flight).
    fn paint_footer(&mut self, ui: &mut Ui) {
        let text = if self.running {
            "searching…".to_owned()
        } else if self.last_id.is_some() {
            let hits = self.hit_count();
            let files = self.group_count();
            format!(
                "{} result{} · {} file{}",
                hits,
                if hits == 1 { "" } else { "s" },
                files,
                if files == 1 { "" } else { "s" }
            )
        } else {
            return;
        };
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), FOOTER_HEIGHT), Sense::hover());
        ui.painter().text(
            pos2(rect.left() + space::S3, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_FAINT,
        );
    }

    // -- helpers ---------------------------------------------------------------

    /// Submit the current query: assign the id, flip to running, clear the
    /// previous results, and emit [`SearchEvent::RunSearch`].
    fn emit_run(&mut self, events: &mut Vec<SearchEvent>) {
        if self.query.is_empty() {
            // Nothing to search: an empty pattern matches nothing, so the
            // previous results are simply dropped.
            self.groups.clear();
            self.running = false;
            self.last_id = None;
            return;
        }
        let id = SearchId::next();
        let query = SearchQuery {
            pattern: self.query.clone(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
        };
        self.last_id = Some(id);
        self.running = true;
        self.groups.clear();
        self.selected = None;
        events.push(SearchEvent::RunSearch { id, query });
    }

    /// Group `hits` by file (arrival order) and compute display paths.
    fn regroup(&mut self, hits: Vec<SearchHit>) {
        let root = self.root.as_deref();
        let mut index: HashMap<PathBuf, usize> = HashMap::new();
        let mut groups: Vec<SearchGroup> = Vec::new();
        for hit in hits {
            let group = match index.get(&hit.path) {
                Some(&at) => &mut groups[at],
                None => {
                    let at = groups.len();
                    index.insert(hit.path.clone(), at);
                    let display = display_for(root, &hit.path);
                    groups.push(SearchGroup {
                        path: hit.path.clone(),
                        display,
                        hits: Vec::new(),
                    });
                    &mut groups[at]
                }
            };
            group.hits.push(hit);
        }
        self.groups = groups;
        self.collapsed.clear();
        self.selected = None;
    }
}

/// The display string for `path`: workspace-relative with `/`
/// separators when the root is known and the path is inside it, else the
/// full display path.
fn display_for(root: Option<&Path>, path: &Path) -> String {
    match root {
        Some(root) => match path.strip_prefix(root) {
            Ok(relative) => display_path(relative),
            Err(_) => display_path(path),
        },
        None => display_path(path),
    }
}

// -----------------------------------------------------------------------------
// Row painting helpers (free functions: the paint loop borrows the result
// data immutably while click targets are collected for later application)
// -----------------------------------------------------------------------------

/// Paint one result row: line number, the line text with matched
/// substrings highlighted, selection/hover surfaces. Clicks append to
/// `open`. Returns 1 (rows painted).
fn paint_result_row(
    ui: &mut Ui,
    hit: &SearchHit,
    selected: &Option<(PathBuf, usize)>,
    open: &mut Option<(PathBuf, usize)>,
) -> usize {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let is_selected = selected.as_ref() == Some(&(hit.path.clone(), hit.line_number));
    let hovered = response.hovered();

    // Row background: selected --ink-low, hover --ink-faint (DESIGN
    // TOKENS.md §10); no row borders (DESIGN_TOKENS.md §5.2).
    let bg = if is_selected {
        Some(color::INK_LOW)
    } else if hovered {
        Some(color::INK_FAINT)
    } else {
        None
    };
    if let Some(fill) = bg {
        ui.painter().rect_filled(rect, radius::R0, fill);
    }

    // Line number, right-aligned in its column (--type-sm mono, faint).
    let number_x = rect.left() + space::S2 + LINE_NUMBER_WIDTH;
    ui.painter().text(
        pos2(number_x, rect.center().y),
        Align2::RIGHT_CENTER,
        hit.line_number.to_string(),
        egui::FontId::new(type_scale::SM, font::FAMILY_MONO),
        color::FG_FAINT,
    );

    // The line: matched substrings on a --syn-string background (UI_SPEC
    // §9), everything else plain --fg-default.
    let font = egui::FontId::new(type_scale::MD, font::FAMILY_MONO);
    let mut x = number_x + space::S1;
    for (text, is_match) in segments(&hit.line_text, &hit.spans) {
        if text.is_empty() {
            continue;
        }
        let galley = ui
            .painter()
            .layout_no_wrap(text.to_owned(), font.clone(), color::FG_DEFAULT);
        let size = galley.size();
        let y = rect.top() + (rect.height() - size.y) / 2.0;
        if is_match {
            let highlight = Rect::from_min_size(pos2(x, y), size);
            ui.painter()
                .rect_filled(highlight, radius::R0, color::SYN_STRING);
        }
        ui.painter().galley(pos2(x, y), galley, color::FG_DEFAULT);
        x += size.x;
    }

    if response.clicked() {
        *open = Some((hit.path.clone(), hit.line_number));
    }
    1
}

/// Split `line` into `(text, is_match)` segments at `spans` (char spans
/// over `line`). Overlapping or out-of-range spans are clamped/dropped.
fn segments<'a>(line: &'a str, spans: &[MatchSpan]) -> Vec<(&'a str, bool)> {
    let char_count = line.chars().count();
    let mut out = Vec::new();
    let mut cursor = 0usize; // byte offset of the next un-emitted segment
    for span in spans {
        if span.end <= span.start || span.start >= char_count {
            continue;
        }
        let end = span.end.min(char_count);
        let start = span.start.min(end);
        let (Some(start_byte), Some(end_byte)) =
            (char_to_byte(line, start), char_to_byte(line, end))
        else {
            continue;
        };
        if start_byte < cursor {
            continue; // overlaps a segment already emitted
        }
        if start_byte > cursor {
            out.push((&line[cursor..start_byte], false));
        }
        out.push((&line[start_byte..end_byte], true));
        cursor = end_byte;
    }
    if cursor < line.len() {
        out.push((&line[cursor..], false));
    }
    out
}

/// Byte offset of char index `at` in `line`.
fn char_to_byte(line: &str, at: usize) -> Option<usize> {
    line.char_indices()
        .nth(at)
        .map(|(offset, _)| offset)
        .or_else(|| (at == line.chars().count()).then_some(line.len()))
}

/// A toggle chip: 28×24, `--radius-1`; active = `--ink-low` fill +
/// `--fg-default`, inactive = `--fg-muted`, disabled = `--fg-disabled`
/// (DESIGN_TOKENS.md §10 state matrix).
fn chip(row: &mut Ui, label: &str, active: bool, enabled: bool) -> egui::Response {
    let (rect, response) =
        row.allocate_exact_size(vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT), Sense::click());
    if active {
        row.painter().rect_filled(rect, radius::R1, color::INK_LOW);
    } else if enabled && response.hovered() {
        row.painter()
            .rect_filled(rect, radius::R1, color::INK_FAINT);
    }
    let text_color = if !enabled {
        color::FG_DISABLED
    } else if active || response.hovered() {
        color::FG_DEFAULT
    } else {
        color::FG_MUTED
    };
    row.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::new(type_scale::XS, font::FAMILY_MONO),
        text_color,
    );
    response
}

/// The group-header disclosure chevron: a two-segment `›` that becomes
/// `v` when the group is expanded (UI_SPEC.md §9 groups; the sidebar's
/// icons module is a Phase 2 module this phase does not own, so the
/// 1.5px-stroke glyph is drawn here in the same style).
fn paint_chevron(painter: &egui::Painter, slot: Rect, color: egui::Color32, down: bool) {
    let center = slot.center();
    let arm = 4.0;
    let stroke = egui::Stroke::new(1.5, color);
    if down {
        let (left, mid, right) = (
            pos2(center.x - arm, center.y - arm / 2.0),
            pos2(center.x, center.y + arm / 2.0),
            pos2(center.x + arm, center.y - arm / 2.0),
        );
        painter.line_segment([left, mid], stroke);
        painter.line_segment([mid, right], stroke);
    } else {
        let (top, mid, bottom) = (
            pos2(center.x - arm, center.y - arm / 2.0),
            pos2(center.x, center.y),
            pos2(center.x - arm, center.y + arm / 2.0),
        );
        painter.line_segment([top, mid], stroke);
        painter.line_segment([mid, bottom], stroke);
    }
}

/// The `×` clear glyph: a 1.5px-stroke cross (the icons module is a
/// Phase 2 module this phase does not own; the glyph is two segments).
fn paint_x(painter: &egui::Painter, slot: Rect, color: egui::Color32) {
    let center = slot.center();
    let arm = 3.5;
    let stroke = egui::Stroke::new(1.5, color);
    painter.line_segment(
        [
            pos2(center.x - arm, center.y - arm),
            pos2(center.x + arm, center.y + arm),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pos2(center.x - arm, center.y + arm),
            pos2(center.x + arm, center.y - arm),
        ],
        stroke,
    );
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str, line: usize, text: &str, spans: Vec<(usize, usize)>) -> SearchHit {
        SearchHit {
            path: PathBuf::from(path),
            line_number: line,
            line_text: text.to_owned(),
            spans: spans
                .into_iter()
                .map(|(start, end)| MatchSpan { start, end })
                .collect(),
        }
    }

    fn key_event(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn enter_runs_the_query_and_reports_the_search_id() {
        let mut panel = SearchPanel::new();
        panel.focus_query();
        panel.set_query("ropey");

        // Two frames over one context (`__run_test_ui` is a fresh context
        // per pass, but `lost_focus` needs focus history): frame 1 grants
        // the input focus, frame 2 delivers Enter — the single-line edit
        // surrenders focus, which is the submit signal.
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_min_size(vec2(360.0, 400.0));
            let _ = panel.show(ui);
        });
        // The harness has no renderer to feed the font-texture delta; the
        // test only reads widget state, so the delta is dropped cleanly.
        output.textures_delta.clear();

        let mut raw = egui::RawInput::default();
        raw.focused = true;
        raw.events.push(key_event(egui::Key::Enter));
        let mut events = Vec::new();
        let mut output = ctx.run_ui(raw, |ui| {
            ui.set_min_size(vec2(360.0, 400.0));
            events = panel.show(ui).events;
        });
        output.textures_delta.clear();

        assert_eq!(events.len(), 1);
        match &events[0] {
            SearchEvent::RunSearch { id, query } => {
                assert_eq!(query.pattern, "ropey");
                assert!(!query.case_sensitive);
                assert!(!query.whole_word);
                assert_eq!(panel.last_id, Some(*id));
                assert!(panel.is_running());
            }
            other => panic!("expected RunSearch, got {other:?}"),
        }
    }

    #[test]
    fn enter_without_query_focus_does_not_submit() {
        // Enter landing elsewhere in the app must not trigger a search.
        let mut panel = SearchPanel::new();
        panel.set_query("ropey");
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.focused = true;
        raw.events.push(key_event(egui::Key::Enter));
        let mut events = Vec::new();
        let mut output = ctx.run_ui(raw, |ui| {
            ui.set_min_size(vec2(360.0, 400.0));
            events = panel.show(ui).events;
        });
        output.textures_delta.clear();
        assert!(events.is_empty(), "events: {events:?}");
        assert!(!panel.is_running());
    }

    #[test]
    fn submit_query_runs_the_current_query_state() {
        // The programmatic submit seam (used by Enter and available to the
        // shell for keybinds): assigns the id, flips to running, clears
        // previous results.
        let mut panel = SearchPanel::new();
        panel.set_query("rope");
        panel.set_case_sensitive(true);

        let event = panel.submit_query().expect("event");
        match event {
            SearchEvent::RunSearch { id, query } => {
                assert_eq!(query.pattern, "rope");
                assert!(query.case_sensitive);
                assert!(!query.whole_word);
                assert_eq!(panel.last_id, Some(id));
                assert!(panel.is_running());
            }
            other => panic!("expected RunSearch, got {other:?}"),
        }

        // An empty query clears instead of searching.
        panel.set_query("");
        assert!(panel.submit_query().is_none());
        assert!(!panel.is_running());
        assert!(!panel.has_run());
    }

    #[test]
    fn results_install_by_id_and_stale_replies_are_dropped() {
        let mut panel = SearchPanel::new();
        panel.set_query("rope");
        let stale = SearchId::next();
        let fresh = SearchId::next();
        panel.set_results(stale, vec![hit("a.rs", 1, "rope", vec![(0, 4)])]);
        assert_eq!(panel.group_count(), 0, "stale id must be ignored");

        panel.last_id = Some(fresh);
        panel.set_results(
            fresh,
            vec![
                hit("/repo/src/a.rs", 2, "let rope = 1;", vec![(4, 8)]),
                hit("/repo/src/a.rs", 3, "rope again", vec![(0, 4)]),
                hit("/repo/src/b.rs", 7, "ropey", vec![(0, 4)]),
            ],
        );
        assert!(!panel.is_running());
        assert_eq!(panel.group_count(), 2);
        assert_eq!(panel.hit_count(), 3);
        // Grouped by file, arrival order.
        assert_eq!(panel.groups()[0].path, PathBuf::from("/repo/src/a.rs"));
        assert_eq!(panel.groups()[0].hits.len(), 2);
        assert_eq!(panel.groups()[1].hits[0].line_number, 7);
    }

    #[test]
    fn group_display_is_relative_to_the_workspace_root() {
        let mut panel = SearchPanel::new();
        let id = SearchId::next();
        panel.last_id = Some(id);
        panel.set_results(
            id,
            vec![
                hit("/repo/src/buffer.rs", 1, "rope", vec![(0, 4)]),
                hit("/elsewhere/other.rs", 1, "rope", vec![(0, 4)]),
            ],
        );
        assert_eq!(panel.groups()[0].display, "/repo/src/buffer.rs");
        assert_eq!(panel.groups()[1].display, "/elsewhere/other.rs");

        // A root arriving after the results recomputes the display.
        panel.set_root(PathBuf::from("/repo"));
        assert_eq!(panel.groups()[0].display, "src/buffer.rs");
        assert_eq!(panel.groups()[1].display, "/elsewhere/other.rs");
    }

    #[test]
    fn painting_results_groups_rows_headlessly() {
        let mut panel = SearchPanel::new();
        let id = SearchId::next();
        panel.last_id = Some(id);
        panel.set_results(
            id,
            vec![
                hit("/repo/a.rs", 42, "pub fn insert() {}", vec![(7, 13)]),
                hit("/repo/a.rs", 88, "insert again", vec![(0, 6)]),
                hit("/repo/b.rs", 12, "pub const X = insert;", vec![(16, 22)]),
            ],
        );

        // 2 group headers + 3 rows.
        let rows = paint_rows(&mut panel);
        assert_eq!(rows, 5);

        // Collapsing a group hides its rows but keeps its header.
        panel.toggle_group(Path::new("/repo/a.rs"));
        assert_eq!(paint_rows(&mut panel), 3);
        panel.toggle_group(Path::new("/repo/a.rs"));
        assert_eq!(paint_rows(&mut panel), 5);
    }

    /// One headless frame; returns the painted row count.
    fn paint_rows(panel: &mut SearchPanel) -> usize {
        let mut rows = 0;
        egui::__run_test_ui(|ui| {
            ui.set_min_size(vec2(360.0, 500.0));
            rows = panel.show(ui).rows_painted;
        });
        rows
    }

    #[test]
    fn empty_panel_paints_no_rows_and_emits_nothing() {
        let mut panel = SearchPanel::new();
        let (rows, events) = {
            let mut events = Vec::new();
            let mut rows = 0;
            egui::__run_test_ui(|ui| {
                ui.set_min_size(vec2(360.0, 400.0));
                let outcome = panel.show(ui);
                rows = outcome.rows_painted;
                events = outcome.events;
            });
            (rows, events)
        };
        assert_eq!(rows, 0);
        assert!(events.is_empty());
        assert!(!panel.has_run());
    }

    #[test]
    fn running_search_shows_then_clears_in_the_footer_state() {
        // The footer's text is derived state; exercise the flags directly.
        let mut panel = SearchPanel::new();
        assert!(!panel.has_run());
        let id = SearchId::next();
        panel.last_id = Some(id);
        panel.running = true;
        assert!(panel.is_running());
        panel.set_results(id, vec![hit("a.rs", 1, "x", vec![(0, 1)])]);
        assert!(!panel.is_running());
        assert!(panel.has_run());
    }

    // -- segments -------------------------------------------------------------

    #[test]
    fn segments_split_line_at_match_spans() {
        let spans = vec![MatchSpan { start: 4, end: 8 }];
        assert_eq!(
            segments("let rope = 1;", &spans),
            vec![("let ", false), ("rope", true), (" = 1;", false)]
        );

        // Multiple spans.
        let spans = vec![
            MatchSpan { start: 0, end: 2 },
            MatchSpan { start: 4, end: 6 },
        ];
        assert_eq!(
            segments("abcd ef", &spans),
            vec![("ab", true), ("cd", false), (" e", true), ("f", false)]
        );
    }

    #[test]
    fn segments_tolerate_bad_spans() {
        // Reversed, overlapping, and out-of-range spans are dropped.
        let spans = vec![
            MatchSpan { start: 9, end: 5 },
            MatchSpan {
                start: 100,
                end: 105,
            },
        ];
        assert_eq!(segments("abc", &spans), vec![("abc", false)]);

        // An overlapping span does not double-cut.
        let spans = vec![
            MatchSpan { start: 0, end: 3 },
            MatchSpan { start: 2, end: 5 },
        ];
        assert_eq!(
            segments("abcde", &spans),
            vec![("abc", true), ("de", false)]
        );
    }

    #[test]
    fn segments_handle_multibyte_lines() {
        // "föhn ropey": chars f(0) ö(1) h(2) n(3) ' '(4) r(5)….
        let spans = vec![MatchSpan { start: 5, end: 10 }];
        assert_eq!(
            segments("f\u{f6}hn ropey", &spans),
            vec![("f\u{f6}hn ", false), ("ropey", true)]
        );
    }
}
