//! The command palette (UI_SPEC.md §5).
//!
//! Two pieces live here:
//!
//! - the [`CommandPalette`] state machine: hidden/shown, the query, the ranked
//!   result list, and the selection. It emits a [`CommandId`] on Enter — it
//!   never runs commands itself (the shell routes ids; see
//!   [`super::registry`]).
//! - the fuzzy ranking ([`rank`]): `nucleo-matcher` over each command's
//!   name + tags, no regex, empty query = registry order.
//!
//! Geometry and color come from UI_SPEC.md §5 via the typed tokens — 560px
//! wide, at most 320px tall (scrolls after 8 rows of 28px), `INK_MID` fill,
//! 1px `INK_HI` border, `radius-2` (the only non-zero-radius surface with a
//! hairline-only border per DESIGN_TOKENS.md §6 — no shadow). The open
//! animation is the `bloom` glyph (DESIGN_TOKENS.md §7.3): scale 0.96 → 1
//! with a fade, `dur-base` on `ease-precise`.

use std::time::Instant;

use egui::{Align2, Context, FontId, Id, Key, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

use super::registry::{Command, CommandId, COMMANDS};
use crate::theme::{color, font, motion, radius, space, type_scale};

// -----------------------------------------------------------------------------
// Geometry (UI_SPEC.md §5, every measurement)
// -----------------------------------------------------------------------------

/// Palette width: 560px (clamped to the workbench if it is narrower).
const WIDTH: f32 = 560.0;
/// Palette maximum total height: 320px.
const MAX_HEIGHT: f32 = 320.0;
/// Row height: 28px.
const ROW_HEIGHT: f32 = 28.0;
/// Rows visible before the list scrolls: 8.
const MAX_VISIBLE_ROWS: usize = 8;
/// Input top padding: 12px (`--space-3`).
const INPUT_TOP_PADDING: f32 = space::S3;
/// Input bottom padding before the hairline: `--space-2`.
const INPUT_BOTTOM_PADDING: f32 = space::S2;
/// Input horizontal inset: 12px each side ("full width minus 24px").
const INPUT_INSET: f32 = space::S3;
/// Row horizontal inset for label and keybind hint.
const ROW_INSET: f32 = space::S3;

/// The input prompt glyph (UI_SPEC.md §5: "`>` prefix glyph in --fg-faint").
const PROMPT_GLYPH: &str = ">";
/// Placeholder text for the input.
const PLACEHOLDER: &str = "Type a command…";
/// Right-aligned hint shown on the *selected* row when it has no keybind
/// (the wireframe's "⏎ Enter" affordance).
const ENTER_HINT: &str = "⏎ Enter";

/// Bloom keyframes: scale-in from 0.96 (DESIGN_TOKENS.md §7.3).
const BLOOM_FROM_SCALE: f32 = 0.96;

// -----------------------------------------------------------------------------
// Fuzzy ranking (nucleo-matcher, no regex)
// -----------------------------------------------------------------------------

/// Rank the registry against `query`.
///
/// - Empty (or whitespace-only) query: the full registry in registry order
///   (the palette's resting state, matching the UI_SPEC.md §5 wireframe).
/// - Otherwise: `nucleo-matcher` score over each command's **name** and each
///   of its **tags**, keeping the best of those scores per command. A command
///   matches only if every whitespace-separated query atom matches the same
///   haystack (nucleo's `Pattern` semantics).
/// - Sort is stable, so equal scores keep the registry order.
pub fn rank(query: &str) -> Vec<&'static Command> {
    if query.trim().is_empty() {
        return COMMANDS.iter().collect();
    }

    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);

    let mut scored: Vec<(u32, usize)> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            command_score(&pattern, &mut matcher, command).map(|score| (score, index))
        })
        .collect();
    scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
    scored
        .into_iter()
        .map(|(_, index)| &COMMANDS[index])
        .collect()
}

/// Best fuzzy score for one command across its name and tags. Each haystack
/// is scored independently (a query must match every atom within one
/// haystack); the best matching haystack wins.
fn command_score(pattern: &Pattern, matcher: &mut Matcher, command: &Command) -> Option<u32> {
    let haystacks = std::iter::once(command.name).chain(command.tags.iter().copied());
    let mut best: Option<u32> = None;
    for haystack in haystacks {
        if let Some(score) = pattern.score(Utf32String::from(haystack).slice(..), matcher) {
            best = Some(best.map_or(score, |best: u32| best.max(score)));
        }
    }
    best
}

// -----------------------------------------------------------------------------
// The palette state machine
// -----------------------------------------------------------------------------

/// The command palette: hidden ⇄ shown, with the query, the ranked results,
/// and the selected row.
///
/// The shell owns one of these, toggles it on `Ctrl+Shift+P`, and calls
/// [`CommandPalette::show_ui`] every frame. Enter (or a row click) returns the
/// selected [`CommandId`] for that frame only; the palette hides itself.
#[derive(Debug, Default)]
pub struct CommandPalette {
    /// Visibility: the whole state machine (hidden = `false`).
    open: bool,
    /// The live query text.
    query: String,
    /// The query the results were ranked for; results are recomputed only
    /// when this differs (typing is the only trigger).
    ranked_query: String,
    /// Ranked commands for the current query.
    results: Vec<&'static Command>,
    /// Index into `results`; resets to 0 whenever the query changes.
    selected: usize,
    /// Set by arrow-key navigation so the selected row scrolls into view.
    follow_selection: bool,
    /// When the palette was last opened — drives the bloom animation.
    open_at: Option<Instant>,
}

impl CommandPalette {
    /// A hidden palette with empty results (call [`Self::show`] before
    /// [`Self::show_ui`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the palette with a fresh query, top result selected, and the
    /// bloom animation armed.
    pub fn show(&mut self) {
        self.open = true;
        self.query.clear();
        self.rerank();
        self.open_at = Some(Instant::now());
    }

    /// Close the palette. Query and selection are reset on the next
    /// [`Self::show`].
    pub fn hide(&mut self) {
        self.open = false;
    }

    /// Flip visibility.
    pub fn toggle(&mut self) {
        if self.open {
            self.hide();
        } else {
            self.show();
        }
    }

    /// Is the palette visible this frame?
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The live query text.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Set the query programmatically (the shell or a test driving the state
    /// machine directly instead of through the input field). Reranks and
    /// resets the selection to the top result.
    pub fn set_query(&mut self, query: &str) {
        if self.query != query {
            self.query = query.to_owned();
            self.refresh();
        }
    }

    /// The currently selected command.
    pub fn selected_command(&self) -> Option<&'static Command> {
        self.results.get(self.selected).copied()
    }

    /// Run the selected command: emit its id and hide. Returns `None` when
    /// the palette is hidden or there is nothing to run.
    pub fn confirm(&mut self) -> Option<CommandId> {
        if !self.open {
            return None;
        }
        let id = self.selected_command().map(|command| command.id);
        self.hide();
        id
    }

    /// Paint the palette for this frame over `workbench` (the editor area
    /// between the title bar and the status bar, supplied by the shell).
    ///
    /// Returns the selected [`CommandId`] on the frame the user runs it —
    /// Enter, or a row click — and `None` every other frame. Handles the
    /// keyboard model from UI_SPEC.md §5: Esc closes, ↑/↓ move the selection
    /// (the list scrolls to follow), Enter runs.
    pub fn show_ui(&mut self, ctx: &Context, workbench: Rect) -> Option<CommandId> {
        if !self.open {
            return None;
        }

        // --- Keyboard (checked before painting so a run/close frame skips
        // --- rendering the list entirely).
        let (escape, up, down, enter) = ctx.input(|input| {
            (
                input.key_pressed(Key::Escape),
                input.key_pressed(Key::ArrowUp),
                input.key_pressed(Key::ArrowDown),
                input.key_pressed(Key::Enter),
            )
        });
        if escape {
            self.hide();
            return None;
        }
        if enter {
            return self.confirm();
        }
        if up || down {
            let last = self.results.len().saturating_sub(1);
            if down {
                self.selected = (self.selected + 1).min(last);
            } else {
                self.selected = self.selected.saturating_sub(1);
            }
            self.follow_selection = true;
        }

        // --- Bloom progress (0 → 1 over dur-base on ease-precise).
        let bloom = self.bloom_progress();
        if bloom < 1.0 {
            ctx.request_repaint();
        }
        let scale = BLOOM_FROM_SCALE + (1.0 - BLOOM_FROM_SCALE) * bloom;

        // --- Geometry: centered horizontally, top of the palette at the top
        // --- 25% of the workbench. Clamped so a narrow window still fits.
        let width = WIDTH.min(workbench.width());
        let pos = Pos2::new(
            workbench.left() + (workbench.width() - width) * 0.5,
            workbench.top() + workbench.height() * 0.25,
        );

        let mut emitted = None;
        let area = egui::Area::new(Id::new("selora_command_palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .interactable(true)
            .movable(false)
            .show(ctx, |ui| {
                // Width: fixed at `width` (Area otherwise sizes to content).
                ui.set_max_width(width);
                ui.set_min_width(width);
                ui.set_opacity(bloom);

                // INK_MID fill, 1px INK_HI hairline, radius-2, no shadow
                // (DESIGN_TOKENS.md §6 — the palette's one-layer border).
                egui::Frame::NONE
                    .fill(color::INK_MID)
                    .stroke(Stroke::new(1.0, color::INK_HI))
                    .corner_radius(radius::R2)
                    .inner_margin(0.0)
                    .show(ui, |ui| {
                        let input_height = self.input_row(ui);
                        self.rows(ui, &mut emitted, input_height);
                    });
            });

        // --- Bloom transform: scale the whole palette layer around its
        // --- center (identity once the animation settles, which also clears
        // --- the layer transform). `TSTransform` is not re-exported at the
        // --- `egui` root, so a value is obtained from the layer itself and
        // --- built up through its public fields.
        let layer = area.response.layer_id;
        let mut transform = ctx.layer_transform_to_global(layer).unwrap_or_default();
        if scale < 1.0 {
            let center = area.response.rect.center();
            transform.scaling = scale;
            transform.translation = center.to_vec2() * (1.0 - scale);
        } else {
            transform.scaling = 1.0;
            transform.translation = Vec2::ZERO;
        }
        ctx.set_transform_layer(layer, transform);

        emitted
    }

    /// Paint the input row. Returns its height (input + paddings + hairline),
    /// which caps the row list so the palette never exceeds 320px.
    fn input_row(&mut self, ui: &mut Ui) -> f32 {
        ui.add_space(INPUT_TOP_PADDING);
        ui.horizontal(|ui| {
            ui.add_space(INPUT_INSET);
            ui.label(
                egui::RichText::new(PROMPT_GLYPH)
                    .color(color::FG_FAINT)
                    .font(FontId::new(type_scale::LG, font::family_sans_medium())),
            );
            ui.add_space(space::S1);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(PLACEHOLDER)
                    .font(FontId::new(type_scale::LG, font::family_sans_medium()))
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
            );
            // The palette is modal-ish: the input owns the keyboard while it
            // is open, so typing never leaks into the editor behind it.
            if !response.has_focus() {
                response.request_focus();
            }
        });
        ui.add_space(INPUT_BOTTOM_PADDING);

        // 1px hairline between the input and the rows (the wireframe's
        // divider; DESIGN_TOKENS.md §5).
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, color::INK_HI);

        ui.min_rect().height()
    }

    /// Paint the result rows: 28px each, label `--type-md` left, keybind hint
    /// `--type-xs --fg-faint` right, selected row `INK_LOW`, hovered
    /// `INK_FAINT` (DESIGN_TOKENS.md §10). Scrolls after 8 rows and follows
    /// keyboard selection.
    fn rows(&mut self, ui: &mut Ui, emitted: &mut Option<CommandId>, input_height: f32) {
        // Recompute results once per query change; a change resets the
        // selection to the top result.
        self.refresh();

        let rows_height =
            (ROW_HEIGHT * MAX_VISIBLE_ROWS as f32).min((MAX_HEIGHT - input_height).max(ROW_HEIGHT));
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(rows_height)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let width = ui.available_width();
                for (index, command) in self.results.iter().enumerate() {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click());
                    let selected = index == self.selected;
                    let hovered = response.hovered();

                    let background = if selected {
                        color::INK_LOW
                    } else if hovered {
                        color::INK_FAINT
                    } else {
                        color::INK_MID
                    };
                    if background != color::INK_MID {
                        ui.painter().rect_filled(rect, 0.0, background);
                    }
                    // Hover moves the selection (rows are one-shot targets;
                    // the row under the cursor is the row Enter will run).
                    if hovered {
                        self.selected = index;
                    }
                    if response.clicked() {
                        *emitted = Some(command.id);
                    }

                    ui.painter().text(
                        Pos2::new(rect.left() + ROW_INSET, rect.center().y),
                        Align2::LEFT_CENTER,
                        command.name,
                        FontId::new(type_scale::MD, font::FAMILY_SANS),
                        color::FG_DEFAULT,
                    );
                    // Keybind hint — or the ⏎ affordance on the selected row
                    // when the command has no keybind (the wireframe's top
                    // result).
                    let hint =
                        command
                            .keybind_hint
                            .or(if selected { Some(ENTER_HINT) } else { None });
                    if let Some(hint) = hint {
                        ui.painter().text(
                            Pos2::new(rect.right() - ROW_INSET, rect.center().y),
                            Align2::RIGHT_CENTER,
                            hint,
                            FontId::new(type_scale::XS, font::FAMILY_MONO),
                            color::FG_FAINT,
                        );
                    }

                    // Keep the keyboard selection in view.
                    if selected && self.follow_selection {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                        self.follow_selection = false;
                    }
                }
            });

        if emitted.is_some() {
            self.hide();
        }
    }

    /// Recompute `results` when the query changed; reset the selection to the
    /// top result.
    fn refresh(&mut self) {
        if self.ranked_query != self.query {
            self.rerank();
        }
    }

    /// Recompute `results` unconditionally (used by [`Self::show`], where the
    /// query may be unchanged-but-empty and the previous results were for the
    /// last session's query).
    fn rerank(&mut self) {
        self.ranked_query.clone_from(&self.query);
        self.results = rank(&self.query);
        self.selected = 0;
        self.follow_selection = false;
    }

    /// Bloom progress: 0 → 1 over `--dur-base` on `--ease-precise`
    /// (DESIGN_TOKENS.md §7.2, §7.3). `0.0`-at-open and clamped at 1.
    fn bloom_progress(&self) -> f32 {
        match self.open_at {
            None => 1.0,
            Some(opened) => {
                let elapsed = opened.elapsed().as_millis() as f32 / motion::DUR_BASE_MS as f32;
                ease_precise(elapsed.min(1.0))
            }
        }
    }
}

/// The `bloom` easing (DESIGN_TOKENS.md §7.3: `--ease-precise`,
/// cubic-bezier(0.2, 0, 0, 1)) evaluated at linear progress `t`.
fn ease_precise(t: f32) -> f32 {
    let (x1, y1, x2, y2) = motion::EASE_PRECISE;
    cubic_bezier_y(x1, y1, x2, y2, t)
}

/// Evaluate the y of a cubic bezier from (0, 0) to (1, 1) with control points
/// `(x1, y1)` and `(x2, y2)`, at x = `t`. Newton-Raphson on x(u) = t, then
/// y(u); robust enough for a 240ms one-shot animation.
fn cubic_bezier_y(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    // Polynomial coefficients for x(u) and y(u) after the (0,0)→(1,1) fixed
    // endpoints: x(u) = ((ax + bx) * u + cx) * u * u, same shape for y.
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;

    let sample_x = |u: f32| ((ax * u + bx) * u + cx) * u;
    let sample_y = |u: f32| ((ay * u + by) * u + cy) * u;
    let sample_dx = |u: f32| (3.0 * ax * u + 2.0 * bx) * u + cx;

    // Newton-Raphson for x(u) = t.
    let mut u = t;
    for _ in 0..8 {
        let error = sample_x(u) - t;
        if error.abs() < 1e-5 {
            break;
        }
        let derivative = sample_dx(u);
        if derivative.abs() < 1e-6 {
            break;
        }
        u -= error / derivative;
    }
    sample_y(u.clamp(0.0, 1.0))
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::registry::CommandId;
    use super::*;
    use egui::{Event, RawInput};

    /// The reference workbench (UI_SPEC.md: 1280x800 window minus the 32px
    /// title bar and 24px status bar).
    fn test_workbench() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 32.0), Vec2::new(1280.0, 800.0 - 32.0 - 24.0))
    }

    /// A test context with the bundled fonts registered (the palette paints
    /// with the named Space Grotesk Medium family, which must be bound or
    /// text layout panics).
    fn test_ctx() -> Context {
        let ctx = Context::default();
        crate::theme::set_fonts(&ctx);
        ctx
    }

    /// Run one palette frame with the given raw input events, returning what
    /// `show_ui` emitted.
    fn run_frame(
        ctx: &Context,
        palette: &mut CommandPalette,
        events: Vec<Event>,
    ) -> Option<CommandId> {
        let workbench = test_workbench();
        let emitted = std::cell::RefCell::new(None);
        let output = ctx.run_ui(
            RawInput {
                events,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                *emitted.borrow_mut() = palette.show_ui(ui.ctx(), workbench);
            },
        );
        output.drop_without_applying_deltas();
        emitted.into_inner()
    }

    fn key_event(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    // --- Ranking (unit tests over the registry).

    #[test]
    fn empty_query_returns_the_whole_registry_in_order() {
        let ranked = rank("");
        assert_eq!(ranked.len(), COMMANDS.len());
        assert_eq!(ranked[0].name, "File: Open File…");
        assert_eq!(ranked[1].name, "File: Open Folder…");
    }

    #[test]
    fn whitespace_only_query_returns_the_whole_registry() {
        assert_eq!(rank("   ").len(), COMMANDS.len());
    }

    #[test]
    fn toggle_chat_ranks_the_chat_toggle_first() {
        let ranked = rank("toggle chat");
        assert_eq!(
            ranked.first().map(|command| command.name),
            Some("View: Toggle Chat Sidebar")
        );
    }

    #[test]
    fn every_atom_must_match_the_same_haystack() {
        // "chat tree" has no single command whose name or tag matches both
        // atoms — nucleo requires each atom to hit the haystack it scores.
        assert!(rank("chat tree").is_empty());
    }

    #[test]
    fn save_ranks_the_shortest_save_command_first() {
        let ranked = rank("save");
        assert_eq!(ranked[0].name, "File: Save");
        let names: Vec<&str> = ranked.iter().map(|command| command.name).collect();
        assert!(names.contains(&"File: Save All"));
    }

    #[test]
    fn tags_match_where_names_do_not() {
        // "explorer" only exists as a tag on the file tree toggle.
        let ranked = rank("explorer");
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].name, "View: Toggle File Tree");
    }

    #[test]
    fn non_matching_query_returns_nothing() {
        assert!(rank("qqqzzzz").is_empty());
    }

    // --- State machine.

    #[test]
    fn selection_resets_to_the_top_result_when_the_query_changes() {
        let mut palette = CommandPalette::new();
        palette.show();
        assert_eq!(
            palette.selected_command().map(|c| c.name),
            Some("File: Open File…")
        );

        palette.set_query("toggle");
        assert_eq!(palette.selected, 0);

        // Change the query: the selection is back at the top result.
        palette.set_query("save");
        assert_eq!(palette.selected, 0);
        assert_eq!(
            palette.selected_command().map(|c| c.name),
            Some("File: Save")
        );
    }

    #[test]
    fn confirm_emits_the_selected_id_and_hides() {
        let mut palette = CommandPalette::new();
        palette.show();
        palette.set_query("toggle chat");
        assert_eq!(palette.confirm(), Some(CommandId::ViewToggleChatSidebar));
        assert!(!palette.is_open());
        assert_eq!(palette.confirm(), None);
    }

    // --- Headless UI test (the UI_SPEC.md §5 acceptance path): open → type
    // --- "toggle chat" → Enter emits the command id.

    #[test]
    fn open_type_and_enter_emits_the_command() {
        let ctx = test_ctx();
        let mut palette = CommandPalette::new();
        assert!(!palette.is_open());

        palette.show();
        // Frame 1: renders and requests input focus.
        assert_eq!(run_frame(&ctx, &mut palette, vec![]), None);
        assert!(palette.is_open());

        // Frame 2: type the query into the focused input.
        assert_eq!(
            run_frame(&ctx, &mut palette, vec![Event::Text("toggle chat".into())]),
            None
        );
        assert_eq!(
            palette.selected_command().map(|command| command.name),
            Some("View: Toggle Chat Sidebar")
        );

        // Frame 3: Enter runs the selection.
        assert_eq!(
            run_frame(&ctx, &mut palette, vec![key_event(Key::Enter)]),
            Some(CommandId::ViewToggleChatSidebar)
        );
        assert!(!palette.is_open());
    }

    #[test]
    fn escape_closes_without_emitting() {
        let ctx = test_ctx();
        let mut palette = CommandPalette::new();
        palette.show();
        run_frame(&ctx, &mut palette, vec![]);
        assert_eq!(
            run_frame(&ctx, &mut palette, vec![key_event(Key::Escape)]),
            None
        );
        assert!(!palette.is_open());
    }

    #[test]
    fn arrow_keys_move_the_selection_without_emitting() {
        let ctx = test_ctx();
        let mut palette = CommandPalette::new();
        palette.show();
        run_frame(&ctx, &mut palette, vec![]);
        let first = palette.selected;

        assert_eq!(
            run_frame(&ctx, &mut palette, vec![key_event(Key::ArrowDown)]),
            None
        );
        assert_eq!(palette.selected, first + 1);

        assert_eq!(
            run_frame(&ctx, &mut palette, vec![key_event(Key::ArrowUp)]),
            None
        );
        assert_eq!(palette.selected, first);
    }

    // --- The bloom easing.

    #[test]
    fn ease_precise_hits_its_endpoints() {
        assert!((ease_precise(0.0)).abs() < 1e-6);
        assert!((ease_precise(1.0) - 1.0).abs() < 1e-6);
        // Monotonic in between.
        let mut previous = ease_precise(0.0);
        for step in 1..=10 {
            let t = step as f32 / 10.0;
            let current = ease_precise(t);
            assert!(current >= previous);
            previous = current;
        }
    }
}
