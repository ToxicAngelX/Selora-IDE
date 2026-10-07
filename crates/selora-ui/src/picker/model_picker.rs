//! The model picker dropdown (UI_SPEC.md §4.1) — Phase 4b's file.
//!
//! Contract (ROADMAP.md Phase 4b): live model list on app boot, refresh on
//! demand (a button in the picker header), cached in
//! `settings.selora.active_model` for offline display, search/filter
//! inside the dropdown. The list comes in as `&[Model]` from
//! `AppMessage::ModelsLoaded` (the shell owns the fetch); the picker is a
//! pure widget — it emits a `ModelPickerEvent` (select / refresh / close)
//! and never touches the network or settings files itself.
//!
//! Layout (UI_SPEC.md §4.1):
//!
//! - closed: the chat-header button — the active model's display name plus
//!   a `▾` chevron in `--fg-faint` (the cached id when no model list is
//!   loaded, so the offline state still shows the user's last choice),
//! - open: a 320px dropdown (`--ink-mid`, 1px `--ink-hi`, `--radius-2`,
//!   no shadow — the dropdown/palette surface) anchored below the button:
//!   a 24px header ("MODEL" + the refresh button), the filter input, 32px
//!   rows (display name `--type-md` left, muted id `--type-xs` mono right,
//!   an `--ok` dot on the active row, scroll after 8 rows), and a 24px
//!   footer with the model count or the offline hint.
//!
//! Data flow (the Phase 2 effect/message pattern): the shell fetches
//! `GET /v1/models` on boot and on [`ModelPickerEvent::Refresh`], pushes
//! the list in with [`ModelPicker::set_models`], and applies
//! [`ModelPickerEvent::Select`] by writing `settings.selora.active_model`
//! (an echo back through [`ModelPicker::set_active_model`] is welcome but
//! idempotent — the widget flips its own active row immediately so the UI
//! never lags the settings write).
//!
//! Keyboard (UI_SPEC.md §17: no mouse-only actions): while open, the
//! filter owns the keyboard (the command-palette pattern — typing never
//! leaks into the chat composer behind the dropdown); narrowing the filter
//! and pressing Enter selects the first match; Esc or a click outside
//! dismisses.
//!
//! Every color, font, and spacing value comes from `crate::theme` (the
//! typed DESIGN_TOKENS.md contract); nothing is hardcoded here
//! (DESIGN_TOKENS.md §12).

use egui::{pos2, vec2, Align2, Key, Rect, Sense, Stroke, Ui, Vec2};

use selora_ai::Model;

use crate::theme::{color, font, radius, space, type_scale};

/// Height of the closed-state button in the chat sidebar header
/// (UI_SPEC.md §4 center slot).
pub const CLOSED_BUTTON_HEIGHT: f32 = 24.0;
/// The dropdown's width (UI_SPEC.md §4.1: "320px wide").
pub const DROPDOWN_WIDTH: f32 = 320.0;
/// Model rows visible before the list scrolls (UI_SPEC.md §4.1: "max 8").
pub const MAX_VISIBLE_ROWS: usize = 8;
/// One model row's height (UI_SPEC.md §4.1: "Each row: 32px tall").
pub const ROW_HEIGHT: f32 = 32.0;

/// The dropdown header row (the "MODEL" label + the refresh button).
const HEADER_ROW_HEIGHT: f32 = 24.0;
/// The filter input's row (an estimate — only used to clamp the popup on
/// short screens; the painted height follows the edit's natural height).
const FILTER_ROW_HEIGHT: f32 = 28.0;
/// The footer row (model count / offline hint).
const FOOTER_ROW_HEIGHT: f32 = 24.0;
/// The refresh button's tap slot, right-aligned in the header row.
const REFRESH_SLOT_WIDTH: f32 = 24.0;
/// The `▾` chevron's slot in the closed-state button.
const CHEVRON_WIDTH: f32 = 12.0;
/// Radius of the `--ok` dot on the active row.
const DOT_RADIUS: f32 = 3.0;
/// The closed-state label when no active model is set.
const NO_MODEL_LABEL: &str = "no model";
/// The dropdown header label (UI_SPEC.md §4.1 wireframe).
const HEADER_LABEL: &str = "MODEL";
/// The filter input's placeholder.
const FILTER_HINT: &str = "Filter models";
/// Row-area hint when the model list is empty (the offline state — the
/// shell hands the picker whatever it has cached, possibly nothing).
const OFFLINE_HINT: &str = "offline · no cached model list";
/// Row-area hint when the filter matches nothing.
const NO_MATCHES_HINT: &str = "no matches";
/// Footer text for the offline state.
const OFFLINE_FOOTER: &str = "offline";
/// The popup `Area`'s stable layer id (one picker per app — it lives in
/// the chat sidebar header).
const POPUP_ID: &str = "selora_model_picker_popup";
/// The filter input's stable widget id (focus survives re-layout).
const FILTER_ID: &str = "selora_model_picker_filter";
/// The row list's scroll id.
const ROWS_SCROLL_ID: &str = "selora_model_picker_rows";

// -----------------------------------------------------------------------------
// Events
// -----------------------------------------------------------------------------

/// What the picker tells the shell each frame.
///
/// - [`ModelPickerEvent::Select`] — the shell writes
///   `settings.selora.active_model` (and uses it as the chat request model).
/// - [`ModelPickerEvent::Refresh`] — the shell re-runs `GET /v1/models`;
///   the dropdown stays open and installs the reply via
///   [`ModelPicker::set_models`].
/// - [`ModelPickerEvent::Close`] — the dropdown dismissed itself (Esc,
///   outside click, toggle re-click); informational.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelPickerEvent {
    /// A model row was selected — apply it (write
    /// `settings.selora.active_model`).
    Select(String),
    /// The refresh button was pressed — refetch the model list.
    Refresh,
    /// The dropdown closed.
    Close,
}

// -----------------------------------------------------------------------------
// Geometry (the last painted frame)
// -----------------------------------------------------------------------------

/// The picker's last painted geometry. egui offers no rect-query API, so
/// the widget reports its own layout — this is the seam the headless
/// tests use to inject pointer events at real widget positions; the shell
/// never reads it.
#[derive(Clone, Debug)]
pub struct PickerGeometry {
    /// The closed-state button's rect (always painted).
    pub toggle: Rect,
    /// The dropdown's rect (`None` while closed).
    pub popup: Option<Rect>,
    /// The refresh button's rect (`None` while closed).
    pub refresh: Option<Rect>,
    /// `(model id, row rect)` for every painted row, in list order
    /// (empty while closed).
    pub rows: Vec<(String, Rect)>,
}

// -----------------------------------------------------------------------------
// ModelPicker
// -----------------------------------------------------------------------------

/// The chat sidebar's model picker (UI_SPEC.md §4 / §4.1). Create one per
/// app and call [`ModelPicker::show`] every frame from the chat header.
///
/// Pure widget: no network, no file IO, no settings writes — everything
/// flows in through [`ModelPicker::set_models`] /
/// [`ModelPicker::set_active_model`] and out through
/// [`ModelPickerEvent`].
pub struct ModelPicker {
    /// The active model id (`settings.selora.active_model`; the shell owns
    /// the persisted copy, the picker mirrors it).
    active_model: Option<String>,
    /// The model list, as last pushed by the shell (live or cached).
    models: Vec<Model>,
    /// Whether the dropdown is open.
    open: bool,
    /// The filter input's text.
    filter: String,
    // -- last painted geometry (see [`ModelPicker::last_geometry`]) ------
    toggle_rect: Rect,
    popup_rect: Option<Rect>,
    refresh_rect: Option<Rect>,
    row_rects: Vec<(String, Rect)>,
}

impl ModelPicker {
    /// A picker over `active_model` (the cached `settings.selora.active_model`,
    /// if any) with an empty model list — the shell pushes the list in when
    /// the boot fetch (or the offline cache) lands.
    pub fn new(active_model: Option<&str>) -> Self {
        Self {
            active_model: active_model.map(str::to_owned),
            models: Vec::new(),
            open: false,
            filter: String::new(),
            toggle_rect: Rect::NOTHING,
            popup_rect: None,
            refresh_rect: None,
            row_rects: Vec::new(),
        }
    }

    // -- shell-facing API ----------------------------------------------------

    /// Install the model list (from `AppMessage::ModelsLoaded` or the
    /// offline cache). Order is the gateway's; the picker keeps it.
    pub fn set_models(&mut self, models: &[Model]) {
        self.models = models.to_vec();
    }

    /// The models the shell last pushed in.
    pub fn models(&self) -> &[Model] {
        &self.models
    }

    /// Set the active model id (the shell's echo of
    /// `settings.selora.active_model` — on boot, on a settings change, or
    /// after applying a [`ModelPickerEvent::Select`]; idempotent, the
    /// picker has already flipped its own row).
    pub fn set_active_model(&mut self, id: Option<&str>) {
        self.active_model = id.map(str::to_owned);
    }

    /// The active model id.
    pub fn active_model(&self) -> Option<&str> {
        self.active_model.as_deref()
    }

    /// Whether the dropdown is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open the dropdown programmatically (the command palette's
    /// "Selora: Switch Model…" path).
    pub fn open(&mut self) {
        self.open = true;
    }

    /// Close the dropdown without emitting an event (the shell's own
    /// dismissal path; user-initiated closes emit
    /// [`ModelPickerEvent::Close`] from `show`).
    pub fn close(&mut self) {
        self.open = false;
        self.filter.clear();
    }

    /// The last painted geometry (the headless-test seam).
    pub fn last_geometry(&self) -> PickerGeometry {
        PickerGeometry {
            toggle: self.toggle_rect,
            popup: self.popup_rect,
            refresh: self.refresh_rect,
            rows: self.row_rects.clone(),
        }
    }

    // -- painting ------------------------------------------------------------

    /// Paint the picker (closed button, plus the dropdown while open) and
    /// return the frame's events.
    pub fn show(&mut self, ui: &mut Ui) -> Vec<ModelPickerEvent> {
        let toggle = self.paint_closed_button(ui);
        self.show_from_toggle(ui, toggle)
    }

    /// Paint the picker with the closed-state button pinned to `zone`
    /// instead of flowing at the ui cursor — the chat header's
    /// absolutely-positioned center slot (UI_SPEC.md §4) hosts the picker
    /// this way. The dropdown anchors below the button either way.
    pub fn show_at(&mut self, ui: &mut Ui, zone: Rect) -> Vec<ModelPickerEvent> {
        let toggle = self.paint_toggle_in(ui, zone);
        self.show_from_toggle(ui, toggle)
    }

    /// The shared body of [`Self::show`] / [`Self::show_at`]: keyboard
    /// handling (before the filter's own `TextEdit` reacts), the toggle's
    /// click, the dropdown, and the click-outside dismissal.
    fn show_from_toggle(&mut self, ui: &mut Ui, toggle: egui::Response) -> Vec<ModelPickerEvent> {
        let mut events = Vec::new();

        // 1. Keyboard (read before painting — the command-palette pattern,
        //    and the only way to see the frame's keys before the filter's
        //    own `TextEdit` reacts to them):
        //    - Esc dismisses (emitting `Close`);
        //    - Enter selects the first match, but only when the filter
        //      owned the keyboard at the start of the frame — an Enter in
        //      the chat composer or the editor behind the dropdown must
        //      not pick a model.
        if self.open {
            let escape = ui.ctx().input(|i| i.key_pressed(Key::Escape));
            let enter = ui.ctx().input(|i| i.key_pressed(Key::Enter))
                && ui
                    .ctx()
                    .memory(|mem| mem.focused() == Some(egui::Id::new(FILTER_ID)));
            if escape {
                self.close_emitting(&mut events);
            } else if enter {
                // An Enter with no match leaves the dropdown open (Esc or
                // a click still dismisses it).
                if let Some(id) = self.first_filtered_id() {
                    self.select_and_close(id, &mut events);
                }
            }
        }

        // 2. The closed-state button (also the dropdown's anchor).
        let toggle_clicked = toggle.clicked();
        if toggle_clicked {
            if self.open {
                self.close_emitting(&mut events);
            } else {
                self.open = true;
            }
        }

        if !self.open {
            self.popup_rect = None;
            self.refresh_rect = None;
            self.row_rects.clear();
            return events;
        }

        // 3. The dropdown.
        let painted = self.paint_dropdown(ui.ctx(), self.toggle_rect);
        self.filter = painted.filter;
        self.row_rects = painted.rows;
        self.popup_rect = Some(painted.rect);
        self.refresh_rect = Some(painted.refresh_rect);

        if painted.refresh {
            events.push(ModelPickerEvent::Refresh);
        }

        // 4. A row click selects (the widget flips its row immediately; the
        //    shell persists the id and may echo it back).
        if let Some(id) = painted.select {
            self.select_and_close(id, &mut events);
            return events;
        }

        // 5. A click that landed outside the dropdown (and off the toggle,
        //    which step 2 handled) dismisses it.
        let clicked_outside = ui.ctx().input(|i| {
            i.pointer.any_click()
                && i.pointer.interact_pos().is_some_and(|pos| {
                    !painted.rect.contains(pos) && !self.toggle_rect.contains(pos)
                })
        });
        if clicked_outside && !toggle_clicked {
            self.close_emitting(&mut events);
        }

        events
    }

    /// The closed-state header button's width — the same measurement
    /// [`Self::show`] allocates (callers that pin the trigger with
    /// [`Self::show_at`] size their zone with this so the button never
    /// clips its own label).
    pub fn closed_button_width(&self, painter: &egui::Painter) -> f32 {
        let (label, _) = self.closed_label();
        let font = egui::FontId::new(type_scale::MD, font::FAMILY_SANS);
        let galley = painter.layout_no_wrap(label, font, color::FG_DEFAULT);
        (galley.size().x + 2.0 * space::S1 + space::S1 + CHEVRON_WIDTH).max(64.0)
    }

    /// The closed-state header button: display name + `▾`
    /// (UI_SPEC.md §4 center slot), flowing at the ui cursor. Returns its
    /// response (the toggle).
    fn paint_closed_button(&mut self, ui: &mut Ui) -> egui::Response {
        let (label, text_color) = self.closed_label();
        let font = egui::FontId::new(type_scale::MD, font::FAMILY_SANS);
        let galley = ui.painter().layout_no_wrap(label, font, text_color);
        let width = galley.size().x + 2.0 * space::S1 + space::S1 + CHEVRON_WIDTH;
        let (rect, response) =
            ui.allocate_exact_size(vec2(width.max(64.0), CLOSED_BUTTON_HEIGHT), Sense::click());
        self.paint_toggle(ui, rect, response, galley, text_color)
    }

    /// The closed-state button pinned to `zone` (see [`Self::show_at`]).
    fn paint_toggle_in(&mut self, ui: &mut Ui, zone: Rect) -> egui::Response {
        let (label, text_color) = self.closed_label();
        let font = egui::FontId::new(type_scale::MD, font::FAMILY_SANS);
        let galley = ui.painter().layout_no_wrap(label, font, text_color);
        let rect = zone;
        let response = ui.allocate_rect(rect, Sense::click());
        self.paint_toggle(ui, rect, response, galley, text_color)
    }

    /// Paint the closed-state trigger's visuals into `rect` (both
    /// placements share this).
    fn paint_toggle(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        response: egui::Response,
        galley: std::sync::Arc<egui::Galley>,
        text_color: egui::Color32,
    ) -> egui::Response {
        // Open = --ink-low (the open state of the token state matrix),
        // hover = --ink-faint; --radius-1, no border.
        let background = if self.open {
            Some(color::INK_LOW)
        } else if response.hovered() {
            Some(color::INK_FAINT)
        } else {
            None
        };
        if let Some(fill) = background {
            ui.painter().rect_filled(rect, radius::R1, fill);
        }

        let text_pos = pos2(
            rect.left() + space::S1,
            rect.center().y - galley.size().y * 0.5,
        );
        ui.painter().galley(text_pos, galley, text_color);

        // The chevron: --fg-faint closed, --fg-default open (UI_SPEC.md §4).
        paint_chevron_down(
            ui.painter(),
            Rect::from_min_size(
                pos2(
                    rect.right() - space::S1 - CHEVRON_WIDTH,
                    rect.center().y - CHEVRON_WIDTH,
                ),
                Vec2::splat(CHEVRON_WIDTH * 2.0),
            ),
            if self.open {
                color::FG_DEFAULT
            } else {
                color::FG_FAINT
            },
        );

        self.toggle_rect = rect;
        response
    }

    /// The dropdown: header + filter + rows + footer in a foreground
    /// `Area` anchored below `anchor` (the closed button's rect). Pure
    /// with respect to `self` — everything it changes comes back in the
    /// returned [`PaintedDropdown`].
    fn paint_dropdown(&self, ctx: &egui::Context, anchor: Rect) -> PaintedDropdown {
        // All rows, unfiltered — the visible subset is computed *inside*
        // the frame closure, after the filter input has applied this
        // frame's keystrokes to `filter`, so the list narrows on the very
        // frame the user types (no one-frame lag).
        let all_rows = self.row_views();
        let mut filter = self.filter.clone();
        let models_empty = self.models.is_empty();
        let total = self.models.len();

        // Clamp onto the screen: never past the right edge, open upwards
        // when the list would run past the bottom.
        let screen = ctx.input(egui::InputState::viewport_rect);
        let mut pos = pos2(anchor.left(), anchor.bottom() + space::S1);
        pos.x = pos
            .x
            .min(screen.right() - DROPDOWN_WIDTH - space::S2)
            .max(screen.left());
        let estimate = estimated_dropdown_height(total);
        if pos.y + estimate > screen.bottom() {
            pos.y = (anchor.top() - estimate - space::S1).max(screen.top());
        }

        let area = egui::Area::new(egui::Id::new(POPUP_ID))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .interactable(true)
            .movable(false)
            .show(ctx, |ui| {
                ui.set_min_width(DROPDOWN_WIDTH);
                ui.set_max_width(DROPDOWN_WIDTH);

                let mut painted = PaintedDropdown {
                    filter: String::new(),
                    select: None,
                    refresh: false,
                    refresh_rect: Rect::NOTHING,
                    rows: Vec::new(),
                    rect: Rect::NOTHING,
                };

                // --ink-mid fill, 1px --ink-hi hairline, --radius-2, no
                // shadow (the dropdown surface, DESIGN_TOKENS.md §6).
                egui::Frame::default()
                    .fill(color::INK_MID)
                    .stroke(Stroke::new(1.0, color::INK_HI))
                    .corner_radius(radius::R2)
                    .inner_margin(egui::Margin::same(space::S2 as i8))
                    .show(ui, |ui| {
                        // -- header: "MODEL" left, refresh right ----------
                        ui.horizontal(|ui| {
                            ui.set_min_height(HEADER_ROW_HEIGHT);
                            ui.label(
                                egui::RichText::new(HEADER_LABEL)
                                    .color(color::FG_FAINT)
                                    .font(egui::FontId::new(type_scale::XS, font::FAMILY_SANS)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let (rect, response) = ui.allocate_exact_size(
                                        vec2(REFRESH_SLOT_WIDTH, HEADER_ROW_HEIGHT),
                                        Sense::click(),
                                    );
                                    paint_refresh(ui.painter(), rect, response.hovered());
                                    painted.refresh_rect = rect;
                                    painted.refresh |= response.clicked();
                                },
                            );
                        });

                        // -- filter input -------------------------------
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut filter)
                                .id(egui::Id::new(FILTER_ID))
                                .hint_text(FILTER_HINT)
                                .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS))
                                .desired_width(f32::INFINITY),
                        );
                        // The dropdown owns the keyboard while open (the
                        // command-palette pattern): the filter keeps focus
                        // so typing never leaks into the chat behind it.
                        if !response.has_focus() {
                            response.request_focus();
                        }

                        // The visible rows, from the filter *as of this
                        // frame's keystrokes* (see the comment on
                        // `all_rows` above).
                        let visible: Vec<&RowView> = all_rows
                            .iter()
                            .filter(|row| filter_matches(&row.display, &row.id, &filter))
                            .collect();

                        // -- rows (scroll after MAX_VISIBLE_ROWS) --------
                        egui::ScrollArea::vertical()
                            .id_salt(ROWS_SCROLL_ID)
                            .max_height(ROW_HEIGHT * MAX_VISIBLE_ROWS as f32)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                let width = ui.available_width();
                                if visible.is_empty() {
                                    let (rect, _) = ui.allocate_exact_size(
                                        vec2(width, ROW_HEIGHT),
                                        Sense::hover(),
                                    );
                                    let hint = if models_empty {
                                        OFFLINE_HINT
                                    } else {
                                        NO_MATCHES_HINT
                                    };
                                    ui.painter().text(
                                        pos2(rect.left() + space::S1, rect.center().y),
                                        Align2::LEFT_CENTER,
                                        hint,
                                        egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                                        color::FG_FAINT,
                                    );
                                }
                                for row in &visible {
                                    let (rect, response) = ui.allocate_exact_size(
                                        vec2(width, ROW_HEIGHT),
                                        Sense::click(),
                                    );
                                    paint_row(ui.painter(), rect, row, response.hovered());
                                    painted.rows.push((row.id.clone(), rect));
                                    if response.clicked() {
                                        painted.select = Some(row.id.clone());
                                    }
                                }
                            });

                        // -- footer --------------------------------------
                        let (rect, _) = ui.allocate_exact_size(
                            vec2(ui.available_width(), FOOTER_ROW_HEIGHT),
                            Sense::hover(),
                        );
                        ui.painter().text(
                            pos2(rect.left() + space::S1, rect.center().y),
                            Align2::LEFT_CENTER,
                            footer_text(models_empty, total, visible.len(), &filter),
                            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                            color::FG_FAINT,
                        );
                    });

                painted.filter = filter;
                painted
            });

        let mut painted = area.inner;
        painted.rect = area.response.rect;
        painted
    }

    // -- helpers ---------------------------------------------------------------

    /// The closed-state label: the active model's display name, the cached
    /// id when the model list has no entry for it (offline), and
    /// "no model" when nothing is active.
    fn closed_label(&self) -> (String, egui::Color32) {
        match self.active_model.as_deref() {
            None => (NO_MODEL_LABEL.to_owned(), color::FG_MUTED),
            Some(id) => {
                let display = self
                    .models
                    .iter()
                    .find(|model| model.id == id)
                    .map(|model| model.display_name.as_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(id);
                (display.to_owned(), color::FG_DEFAULT)
            }
        }
    }

    /// Every model as a row view (display name, id, active flag) — the
    /// filter is applied at paint time, see [`Self::paint_dropdown`].
    fn row_views(&self) -> Vec<RowView> {
        self.models
            .iter()
            .map(|model| RowView {
                display: if model.display_name.is_empty() {
                    model.id.clone()
                } else {
                    model.display_name.clone()
                },
                id: model.id.clone(),
                is_active: self.active_model.as_deref() == Some(model.id.as_str()),
            })
            .collect()
    }

    /// The first model id the current filter admits (Enter's target).
    fn first_filtered_id(&self) -> Option<String> {
        self.models
            .iter()
            .find(|model| filter_matches(&model.display_name, &model.id, &self.filter))
            .map(|model| model.id.clone())
    }

    /// Apply a selection: the widget flips its active row immediately (the
    /// shell's settings write is the persisted copy), close, emit.
    fn select_and_close(&mut self, id: String, events: &mut Vec<ModelPickerEvent>) {
        self.active_model = Some(id.clone());
        self.close();
        events.push(ModelPickerEvent::Select(id));
    }

    /// Close and report it.
    fn close_emitting(&mut self, events: &mut Vec<ModelPickerEvent>) {
        self.close();
        events.push(ModelPickerEvent::Close);
    }
}

/// What one dropdown paint pass produced (the Area closure's return value
/// — the closure is `FnOnce`, so the filter and the click targets ride
/// back out in this struct).
struct PaintedDropdown {
    filter: String,
    select: Option<String>,
    refresh: bool,
    refresh_rect: Rect,
    rows: Vec<(String, Rect)>,
    rect: Rect,
}

/// One row's paint data.
struct RowView {
    display: String,
    id: String,
    is_active: bool,
}

/// The dropdown's estimated height (screen clamping only).
fn estimated_dropdown_height(visible_rows: usize) -> f32 {
    let rows = visible_rows.clamp(1, MAX_VISIBLE_ROWS) as f32 * ROW_HEIGHT;
    2.0 * space::S2 + HEADER_ROW_HEIGHT + FILTER_ROW_HEIGHT + rows + FOOTER_ROW_HEIGHT + 2.0
}

/// The footer line: the offline hint, or "n models" / "n of N models"
/// while a filter is narrowing.
fn footer_text(models_empty: bool, total: usize, visible: usize, filter: &str) -> String {
    if models_empty {
        return OFFLINE_FOOTER.to_owned();
    }
    if filter.trim().is_empty() {
        format!("{total} model{}", if total == 1 { "" } else { "s" })
    } else {
        format!("{visible} of {total} models")
    }
}

/// Substring, case-insensitive, over `display_name` + `id` (the Phase 4b
/// brief). An empty (or whitespace-only) filter matches everything.
fn filter_matches(display_name: &str, id: &str, filter: &str) -> bool {
    let needle = filter.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    display_name.to_lowercase().contains(&needle) || id.to_lowercase().contains(&needle)
}

// -----------------------------------------------------------------------------
// Row / glyph painting (free functions — the paint loop holds only shared
// borrows, click targets ride out through PaintedDropdown)
// -----------------------------------------------------------------------------

/// One model row: display name left, muted id right, the `--ok` dot on the
/// active row (UI_SPEC.md §4.1). Active = `--ink-low`, hover =
/// `--ink-faint` (DESIGN_TOKENS.md §10).
fn paint_row(painter: &egui::Painter, rect: Rect, row: &RowView, hovered: bool) {
    let background = if row.is_active {
        Some(color::INK_LOW)
    } else if hovered {
        Some(color::INK_FAINT)
    } else {
        None
    };
    if let Some(fill) = background {
        painter.rect_filled(rect, radius::R0, fill);
    }

    painter.text(
        pos2(rect.left() + space::S1, rect.center().y),
        Align2::LEFT_CENTER,
        row.display.as_str(),
        egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
        color::FG_DEFAULT,
    );

    // Right side: the id in muted mono, the --ok dot outside it when active.
    let mut id_right = rect.right() - space::S1;
    if row.is_active {
        painter.circle_filled(
            pos2(id_right - DOT_RADIUS, rect.center().y),
            DOT_RADIUS,
            color::OK,
        );
        id_right -= 2.0 * DOT_RADIUS + space::S1;
    }
    painter.text(
        pos2(id_right, rect.center().y),
        Align2::RIGHT_CENTER,
        row.id.as_str(),
        egui::FontId::new(type_scale::XS, font::FAMILY_MONO),
        color::FG_FAINT,
    );
}

/// The `▾` chevron of the closed-state button: a two-segment v in a 1.5px
/// stroke (the icons module is a Phase 2 module this phase does not own —
/// same style as the file tree's disclosure chevron).
fn paint_chevron_down(painter: &egui::Painter, slot: Rect, color: egui::Color32) {
    let center = slot.center();
    let arm = 3.5;
    let stroke = egui::Stroke::new(1.5, color);
    let (left, mid, right) = (
        pos2(center.x - arm, center.y - arm / 2.0),
        pos2(center.x, center.y + arm / 2.0),
        pos2(center.x + arm, center.y - arm / 2.0),
    );
    painter.line_segment([left, mid], stroke);
    painter.line_segment([mid, right], stroke);
}

/// The refresh glyph: a 1.5px circle with a right-pointing arrowhead at
/// its top (the "refetch" affordance). `--fg-faint`, `--fg-default` on
/// hover.
fn paint_refresh(painter: &egui::Painter, slot: Rect, hovered: bool) {
    let color = if hovered {
        color::FG_DEFAULT
    } else {
        color::FG_FAINT
    };
    let stroke = egui::Stroke::new(1.5, color);
    let center = slot.center();
    let radius = 4.5;
    painter.circle_stroke(center, radius, stroke);

    // Arrowhead: a chevron at the circle's 1-o'clock tangent, pointing
    // clockwise (the arc's direction of travel).
    let tip = pos2(center.x + radius * 0.5, center.y - radius * 0.87);
    for delta in [vec2(-3.5, -1.5), vec2(-1.5, 3.5)] {
        painter.line_segment([tip, tip + delta], stroke);
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Pos2, RawInput};

    // -- fixtures -------------------------------------------------------------

    fn model(id: &str, display: &str) -> Model {
        Model {
            id: id.to_owned(),
            display_name: display.to_owned(),
            context_window: 200_000,
            price_per_1k_input: 0.5,
            price_per_1k_output: 1.5,
            supports_streaming: true,
            supports_tools: true,
            family: id.split('-').next().unwrap_or_default().to_owned(),
        }
    }

    fn fixture_models() -> Vec<Model> {
        vec![
            model("claude-opus-5-5", "Claude Opus 5.5"),
            model("claude-sonnet-5-5", "Claude Sonnet 5.5"),
            model("gpt-5.1", "GPT 5.1"),
            model("kimi-k3", "Kimi K3"),
            model("glm-5.3", "GLM 5.3"),
        ]
    }

    /// A test context with the bundled fonts registered (the picker lays
    /// text out with the named families).
    fn test_ctx() -> Context {
        let ctx = Context::default();
        crate::theme::set_fonts(&ctx);
        ctx
    }

    /// One picker frame over a chat-sidebar-sized surface; returns the
    /// frame's events.
    fn run_frame(
        ctx: &Context,
        picker: &mut ModelPicker,
        events: Vec<Event>,
    ) -> Vec<ModelPickerEvent> {
        let emitted = std::cell::RefCell::new(Vec::new());
        let output = ctx.run_ui(
            RawInput {
                events,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                ui.set_min_size(vec2(360.0, 600.0));
                *emitted.borrow_mut() = picker.show(ui);
            },
        );
        output.drop_without_applying_deltas();
        emitted.into_inner()
    }

    /// A primary click at `pos` (move, press, release).
    fn click_at(pos: Pos2) -> Vec<Event> {
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]
    }

    fn key_event(key: egui::Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    /// One idle frame so the popup is fully interactive: an `Area`'s first
    /// paint is a sizing pass during which egui marks it non-interactable,
    /// and the hit test of the *next* frame uses that saved state — clicks
    /// and typed text land from the second idle frame on.
    fn warm_up(ctx: &Context, picker: &mut ModelPicker) {
        run_frame(ctx, picker, vec![]);
        run_frame(ctx, picker, vec![]);
    }

    /// Row ids painted in the last frame.
    fn painted_row_ids(picker: &ModelPicker) -> Vec<String> {
        picker
            .last_geometry()
            .rows
            .iter()
            .map(|(id, _)| id.clone())
            .collect()
    }

    // -- open / close ---------------------------------------------------------

    #[test]
    fn toggle_click_opens_and_closes_the_dropdown() {
        let ctx = test_ctx();
        let mut picker = ModelPicker::new(Some("claude-opus-5-5"));
        picker.set_models(&fixture_models());

        // Idle closed frame paints the button and nothing else.
        assert!(run_frame(&ctx, &mut picker, vec![]).is_empty());
        assert!(!picker.is_open());
        let geometry = picker.last_geometry();
        assert!(geometry.popup.is_none());
        assert!(geometry.rows.is_empty());
        let toggle = geometry.toggle;

        // Click the toggle: opens, no event, rows + refresh visible, popup
        // anchored below the button.
        assert!(run_frame(&ctx, &mut picker, click_at(toggle.center())).is_empty());
        assert!(picker.is_open());
        warm_up(&ctx, &mut picker);
        let geometry = picker.last_geometry();
        let popup = geometry.popup.expect("popup");
        assert!(popup.top() >= toggle.bottom());
        assert_eq!(painted_row_ids(&picker).len(), 5);
        assert!(geometry.refresh.is_some());

        // Click the toggle again: closes and reports it.
        let events = run_frame(&ctx, &mut picker, click_at(toggle.center()));
        assert_eq!(events, vec![ModelPickerEvent::Close]);
        assert!(!picker.is_open());
        assert!(picker.last_geometry().popup.is_none());
    }

    #[test]
    fn escape_closes_and_emits_close() {
        let ctx = test_ctx();
        let mut picker = ModelPicker::new(Some("glm-5.3"));
        picker.set_models(&fixture_models());
        picker.open();

        assert!(run_frame(&ctx, &mut picker, vec![]).is_empty());
        let events = run_frame(&ctx, &mut picker, vec![key_event(Key::Escape)]);
        assert_eq!(events, vec![ModelPickerEvent::Close]);
        assert!(!picker.is_open());
    }

    #[test]
    fn click_outside_closes_and_emits_close() {
        let ctx = test_ctx();
        let mut picker = ModelPicker::new(Some("glm-5.3"));
        picker.set_models(&fixture_models());
        picker.open();

        assert!(run_frame(&ctx, &mut picker, vec![]).is_empty());
        // A click far from the popup (into the chat area behind it).
        let events = run_frame(&ctx, &mut picker, click_at(pos2(900.0, 500.0)));
        assert_eq!(events, vec![ModelPickerEvent::Close]);
        assert!(!picker.is_open());
    }

    // -- selection ------------------------------------------------------------

    #[test]
    fn row_click_emits_select_with_the_right_id() {
        let ctx = test_ctx();
        let mut picker = ModelPicker::new(Some("claude-opus-5-5"));
        picker.set_models(&fixture_models());
        picker.open();
        warm_up(&ctx, &mut picker);
        assert!(picker.is_open());

        let rect = picker
            .last_geometry()
            .rows
            .iter()
            .find(|(id, _)| id == "gpt-5.1")
            .map(|(_, rect)| *rect)
            .expect("gpt-5.1 row");

        let events = run_frame(&ctx, &mut picker, click_at(rect.center()));
        assert_eq!(events, vec![ModelPickerEvent::Select("gpt-5.1".to_owned())]);
        assert!(!picker.is_open());
        // The widget flipped its active row immediately; the shell persists
        // the id (settings.selora.active_model) and may echo it back.
        assert_eq!(picker.active_model(), Some("gpt-5.1"));
    }

    #[test]
    fn enter_in_the_filter_selects_the_first_match() {
        let ctx = test_ctx();
        let mut picker = ModelPicker::new(None);
        picker.set_models(&fixture_models());
        picker.open();
        warm_up(&ctx, &mut picker);

        // Warm up (sizing pass + focus), type to narrow, Enter submits.
        // The Enter frame reads the keys before painting, gated on the
        // filter owning the keyboard at the frame's start.
        assert!(run_frame(&ctx, &mut picker, vec![Event::Text("kimi".into())]).is_empty());
        let events = run_frame(&ctx, &mut picker, vec![key_event(Key::Enter)]);
        assert_eq!(events, vec![ModelPickerEvent::Select("kimi-k3".to_owned())]);
        assert!(!picker.is_open());
        assert_eq!(picker.active_model(), Some("kimi-k3"));
    }

    #[test]
    fn enter_elsewhere_does_not_pick_a_model() {
        // The chat composer owns the keyboard: an Enter there submits the
        // chat, not the model picker behind it. The picker reads Enter
        // only when its filter held focus at the frame's start.
        let ctx = test_ctx();
        let mut picker = ModelPicker::new(None);
        picker.set_models(&fixture_models());
        picker.open();

        // A composer painted after the picker takes the focus for the
        // next frame (the last `request_focus` in a frame wins).
        let mut composer = String::new();
        let mut events = Vec::new();
        let output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                ui.set_min_size(vec2(360.0, 600.0));
                events = picker.show(ui);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut composer)
                        .id(egui::Id::new("selora_test_composer"))
                        .hint_text("Ask anything…"),
                );
                if !response.has_focus() {
                    response.request_focus();
                }
            },
        );
        output.drop_without_applying_deltas();
        assert!(events.is_empty());

        // The composer submits on Enter (the search-panel submit signal);
        // the picker stays open and silent.
        let mut events = Vec::new();
        let mut composer_focus = false;
        let output = ctx.run_ui(
            RawInput {
                events: vec![key_event(Key::Enter)],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                ui.set_min_size(vec2(360.0, 600.0));
                events = picker.show(ui);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut composer)
                        .id(egui::Id::new("selora_test_composer"))
                        .hint_text("Ask anything…"),
                );
                if !response.has_focus() {
                    response.request_focus();
                }
                composer_focus = response.has_focus();
            },
        );
        output.drop_without_applying_deltas();
        assert!(composer_focus, "the composer held the keyboard");
        assert!(events.is_empty(), "events: {events:?}");
        assert!(picker.is_open());
    }

    // -- refresh --------------------------------------------------------------

    #[test]
    fn refresh_button_emits_refresh_and_keeps_the_dropdown_open() {
        let ctx = test_ctx();
        let mut picker = ModelPicker::new(Some("glm-5.3"));
        picker.set_models(&fixture_models());
        picker.open();
        warm_up(&ctx, &mut picker);
        let refresh = picker.last_geometry().refresh.expect("refresh slot");
        let events = run_frame(&ctx, &mut picker, click_at(refresh.center()));
        assert_eq!(events, vec![ModelPickerEvent::Refresh]);
        assert!(picker.is_open(), "refresh keeps the dropdown open");
    }

    // -- filter ---------------------------------------------------------------

    #[test]
    fn filter_narrows_the_rows_case_insensitively() {
        let ctx = test_ctx();
        let mut picker = ModelPicker::new(None);
        picker.set_models(&fixture_models());
        picker.open();
        warm_up(&ctx, &mut picker);

        // Empty filter: the whole list.
        assert_eq!(painted_row_ids(&picker).len(), 5);

        // Type into the focused filter: one row left.
        assert!(run_frame(&ctx, &mut picker, vec![Event::Text("kimi".into())]).is_empty());
        assert_eq!(painted_row_ids(&picker), vec!["kimi-k3".to_owned()]);

        // A no-match filter paints the hint row and no model rows.
        assert!(run_frame(&ctx, &mut picker, vec![Event::Text("zzz".into())]).is_empty());
        assert!(painted_row_ids(&picker).is_empty());
        assert!(picker.last_geometry().popup.is_some());
    }

    #[test]
    fn filter_matches_over_display_name_and_id() {
        let (display, id) = ("GLM 5.3", "glm-5.3");
        assert!(filter_matches(display, id, ""));
        assert!(filter_matches(display, id, "   "));
        assert!(filter_matches(display, id, "glm"));
        assert!(filter_matches(display, id, "GLM"));
        assert!(filter_matches(display, id, "5.3"));
        assert!(!filter_matches(display, id, "kimi"));
        assert!(!filter_matches(display, id, "zzz"));
    }

    // -- idle / offline -------------------------------------------------------

    #[test]
    fn idle_frames_emit_no_events() {
        let ctx = test_ctx();
        let mut picker = ModelPicker::new(Some("glm-5.3"));
        picker.set_models(&fixture_models());

        // Closed and idle.
        assert!(run_frame(&ctx, &mut picker, vec![]).is_empty());

        // Open and idle (the focus request is not an event).
        picker.open();
        assert!(run_frame(&ctx, &mut picker, vec![]).is_empty());
        assert!(run_frame(&ctx, &mut picker, vec![]).is_empty());
    }

    #[test]
    fn paints_with_an_empty_model_list_and_shows_the_offline_state() {
        let ctx = test_ctx();
        // The cached active model with no list — the boot state before the
        // fetch lands (or offline with nothing cached).
        let mut picker = ModelPicker::new(Some("claude-opus-5-5"));

        // Closed: the cached id is the label (no display name to resolve).
        assert!(run_frame(&ctx, &mut picker, vec![]).is_empty());
        assert_eq!(picker.closed_label().0, "claude-opus-5-5");

        // Open: no rows, no panic, popup painted.
        picker.open();
        assert!(run_frame(&ctx, &mut picker, vec![]).is_empty());
        let geometry = picker.last_geometry();
        assert!(geometry.popup.is_some());
        assert!(geometry.rows.is_empty());

        // A model list arriving later installs without disturbing state.
        picker.set_models(&fixture_models());
        assert!(run_frame(&ctx, &mut picker, vec![]).is_empty());
        assert_eq!(painted_row_ids(&picker).len(), 5);
    }

    // -- labels ---------------------------------------------------------------

    #[test]
    fn closed_label_uses_the_display_name_and_falls_back_to_the_id() {
        let mut picker = ModelPicker::new(None);
        assert_eq!(picker.closed_label().0, "no model");

        picker.set_active_model(Some("claude-opus-5-5"));
        assert_eq!(
            picker.closed_label().0,
            "claude-opus-5-5",
            "no model list: the cached id stands in"
        );

        picker.set_models(&fixture_models());
        assert_eq!(picker.closed_label().0, "Claude Opus 5.5");
    }

    #[test]
    fn set_active_model_echo_is_idempotent_after_a_select() {
        // The shell's settings write echoed back must not fight the
        // widget's optimistic flip.
        let mut picker = ModelPicker::new(None);
        picker.set_models(&fixture_models());
        picker.set_active_model(Some("glm-5.3"));
        assert_eq!(picker.active_model(), Some("glm-5.3"));
        picker.set_active_model(Some("kimi-k3"));
        assert_eq!(picker.active_model(), Some("kimi-k3"));
        picker.set_active_model(None);
        assert_eq!(picker.active_model(), None);
    }
}
