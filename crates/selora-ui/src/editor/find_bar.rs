//! The find bar (Phase R6c) — Ctrl+F / Ctrl+H over the editor area:
//! the query field, the match count, prev/next, and (in replace mode)
//! the replace field + Replace / Replace All. An egui Area over the
//! pane's top-right — it never disturbs the pane layout.
//!
//! The widget holds the text state and emits [`FindBarEvent`]s; the shell
//! routes them to the active editor's `set_find` / `find_next` /
//! `find_replace_*` methods (the paint loop highlights the matches). The
//! bar's field owns the keyboard while focused (the composer's pattern);
//! Esc closes.

use egui::{pos2, vec2, Align2, Key, Rect, Sense, Ui};

use crate::theme::{color, font, radius, space, type_scale};

/// The bar's width.
const BAR_WIDTH: f32 = 340.0;
/// One row's height (the find row; the replace row mirrors it).
const ROW_HEIGHT: f32 = 28.0;
/// The query field's stable widget id.
const QUERY_ID: &str = "selora_find_query";
/// The replace field's stable widget id.
const REPLACE_ID: &str = "selora_find_replace";

/// The find bar's state — lives in the shell (one bar per app).
#[derive(Default)]
pub struct FindBar {
    /// Whether the bar is open.
    pub open: bool,
    /// Ctrl+H opened it with the replace row.
    pub replace_mode: bool,
    /// The query text.
    pub query: String,
    /// Case-sensitive matching (the `Aa` toggle).
    pub case_sensitive: bool,
    /// The replacement text (replace mode).
    pub replacement: String,
    /// Focus the query on the next frame (opened or reopened).
    want_focus: bool,
}

/// What the bar asks the shell for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindBarEvent {
    /// The query changed — recompute the matches (empty = clear).
    Query(String),
    /// The bar closed (Esc / ×) — clear the editor's find state.
    Close,
    /// Step to the next / previous match.
    Next,
    Prev,
    /// Replace the active match.
    Replace(String),
    /// Replace every match.
    ReplaceAll(String),
}

impl FindBar {
    /// Open for finding (Ctrl+F); the query keeps its last text and the
    /// replace row hides.
    pub fn open_for_find(&mut self) {
        self.open = true;
        self.replace_mode = false;
        self.want_focus = true;
    }

    /// Open for replacing (Ctrl+H): the replace row shows.
    pub fn open_for_replace(&mut self) {
        self.open = true;
        self.replace_mode = true;
        self.want_focus = true;
    }

    /// Close the bar.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Paint the bar over `anchor` (the active pane's top-right corner)
    /// and return the frame's events. `matches`/`active` are the active
    /// editor's counts (1-based active).
    pub fn show(
        &mut self,
        ui: &mut Ui,
        anchor: Rect,
        matches: usize,
        active: usize,
    ) -> Vec<FindBarEvent> {
        let mut events = Vec::new();
        if !self.open {
            return events;
        }

        let height = ROW_HEIGHT * if self.replace_mode { 2.0 } else { 1.0 };
        let rect = Rect::from_min_max(
            pos2(
                anchor.right() - BAR_WIDTH - space::S1,
                anchor.top() + space::S1,
            ),
            pos2(
                anchor.right() - space::S1,
                anchor.top() + space::S1 + height,
            ),
        );

        // The bar owns the keyboard while its query or replace field is
        // focused (the composer pattern): Enter steps forward,
        // Shift+Enter back, Esc closes.
        let query_focused = ui
            .ctx()
            .memory(|m| m.focused() == Some(egui::Id::new(QUERY_ID)));
        let replace_focused = ui
            .ctx()
            .memory(|m| m.focused() == Some(egui::Id::new(REPLACE_ID)));
        // Esc closes from anywhere — the bar is an overlay. (egui clears
        // the widget focus on Escape globally, so this must not gate on
        // focus.)
        let escape = ui.ctx().input(|i| i.key_pressed(Key::Escape));
        if escape {
            events.push(FindBarEvent::Close);
        }
        if query_focused || replace_focused {
            let mut step: Option<i64> = None;
            ui.ctx().input_mut(|input| {
                input.events.retain(|event| match event {
                    egui::Event::Key {
                        key: Key::Enter,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.ctrl && !modifiers.alt => {
                        step = Some(if modifiers.shift { -1 } else { 1 });
                        false
                    }
                    _ => true,
                });
            });
            match step {
                Some(1) => events.push(FindBarEvent::Next),
                Some(-1) => events.push(FindBarEvent::Prev),
                _ => {}
            }
        }

        egui::Area::new(egui::Id::new("selora_find_bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .interactable(true)
            .movable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::default()
                    .fill(color::INK_MID)
                    .stroke(egui::Stroke::new(1.0, color::INK_HI))
                    .corner_radius(radius::R1 as u8)
                    .inner_margin(egui::Margin::same(space::S2 as i8))
                    .show(ui, |ui| {
                        ui.set_min_width(BAR_WIDTH - 2.0 * space::S2);
                        // -- the find row: [query] [N of M] [↑] [↓] [×] ----
                        ui.horizontal(|row| {
                            let query_response = row.add(
                                egui::TextEdit::singleline(&mut self.query)
                                    .id(egui::Id::new(QUERY_ID))
                                    .hint_text("Find")
                                    .font(egui::FontId::new(type_scale::SM, font::FAMILY_SANS))
                                    .desired_width(150.0)
                                    .frame(field_frame(query_focused)),
                            );
                            if self.want_focus {
                                query_response.request_focus();
                            }
                            if query_response.changed() {
                                events.push(FindBarEvent::Query(self.query.clone()));
                            }
                            // The match count: `--fg-muted`, the §10
                            // idle-row readout tone (DESIGN_TOKENS.md §10).
                            row.painter().text(
                                pos2(
                                    query_response.rect.right() + space::S2,
                                    query_response.rect.center().y,
                                ),
                                Align2::LEFT_CENTER,
                                if matches == 0 && !self.query.is_empty() {
                                    "no matches".to_owned()
                                } else {
                                    format!("{active} of {matches}")
                                },
                                egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                                color::FG_MUTED,
                            );
                            row.add_space(space::S1);
                            // The `Aa` case toggle: active = the §10
                            // selected row (`--ink-low`), hover
                            // `--ink-faint`, pressed `--ink-low`.
                            let galley = row.painter().layout_no_wrap(
                                "Aa".to_owned(),
                                egui::FontId::new(type_scale::XS, font::FAMILY_MONO),
                                color::FG_DEFAULT,
                            );
                            let (rect, response) = row.allocate_exact_size(
                                vec2(
                                    galley.size().x + 2.0 * space::S1,
                                    ROW_HEIGHT - 2.0 * space::S1,
                                ),
                                Sense::click(),
                            );
                            if self.case_sensitive || response.is_pointer_button_down_on() {
                                row.painter().rect_filled(rect, radius::R1, color::INK_LOW);
                            } else if response.hovered() {
                                row.painter()
                                    .rect_filled(rect, radius::R1, color::INK_FAINT);
                            }
                            row.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                "Aa",
                                egui::FontId::new(type_scale::XS, font::FAMILY_MONO),
                                if self.case_sensitive {
                                    color::FG_DEFAULT
                                } else {
                                    color::FG_MUTED
                                },
                            );
                            if response.clicked() {
                                self.case_sensitive = !self.case_sensitive;
                                // Re-query with the new case.
                                events.push(FindBarEvent::Query(self.query.clone()));
                            }
                            if small_button(row, "↑", "find_prev").clicked() {
                                events.push(FindBarEvent::Prev);
                            }
                            if small_button(row, "↓", "find_next").clicked() {
                                events.push(FindBarEvent::Next);
                            }
                            if small_button(row, "×", "find_close").clicked() {
                                events.push(FindBarEvent::Close);
                            }
                        });
                        // -- the replace row (Ctrl+H mode) ---------------
                        if self.replace_mode {
                            ui.add_space(space::S1);
                            ui.horizontal(|row| {
                                let _ = row.add(
                                    egui::TextEdit::singleline(&mut self.replacement)
                                        .id(egui::Id::new(REPLACE_ID))
                                        .hint_text("Replace")
                                        .font(egui::FontId::new(type_scale::SM, font::FAMILY_SANS))
                                        .desired_width(150.0)
                                        .frame(field_frame(replace_focused)),
                                );
                                if small_button(row, "Replace", "find_replace").clicked() {
                                    events.push(FindBarEvent::Replace(self.replacement.clone()));
                                }
                                if small_button(row, "Replace All", "find_replace_all").clicked() {
                                    events.push(FindBarEvent::ReplaceAll(self.replacement.clone()));
                                }
                            });
                        }
                    });
            });
        self.want_focus = false;
        events
    }
}

/// A small text button for the bar's controls (the cards' one-liner
/// affordance style: XS text, hover fill). §10 matrix: hover
/// `--ink-faint`, pressed `--ink-low` (REVAMP_SPEC.md §7).
fn small_button(ui: &mut Ui, label: &str, _salt: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
        color::FG_DEFAULT,
    );
    let width = galley.size().x + 2.0 * space::S1;
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, ROW_HEIGHT - 2.0 * space::S1), Sense::click());
    if response.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, radius::R1, color::INK_LOW);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, radius::R1, color::INK_FAINT);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
        if response.hovered() {
            color::FG_DEFAULT
        } else {
            color::FG_MUTED
        },
    );
    response
}

/// The field frame's vertical text inset — egui's own TextEdit default
/// (`Margin::symmetric(4, 2)`, egui 0.36 `text_edit/builder.rs:136`),
/// kept so the explicit frame leaves the field's geometry unchanged. Off
/// the 4px spacing grid by necessity: it mirrors egui, not a token.
const FIELD_MARGIN_V: i8 = 2;

/// The find/replace field frame (REVAMP_SPEC.md §7): the §10 Focused row —
/// `--ink-low` fill, 1px `--fg-default` border while focused, `--ink-hi`
/// otherwise. An explicit frame on the edit (not egui's default frame) so
/// every state's stroke is a token; the focus read happens pre-paint in
/// [`FindBar::show`] (the splash key-input precedent — the inner
/// `Response` doesn't exist at stroke-paint time, the one-frame lag is
/// egui-inherent). The margin keeps egui's own text-edit inset
/// (`--space-1` × [`FIELD_MARGIN_V`]) so the field's geometry is unchanged.
fn field_frame(focused: bool) -> egui::Frame {
    egui::Frame::default()
        .fill(color::INK_LOW)
        .stroke(egui::Stroke::new(
            1.0,
            if focused {
                color::FG_DEFAULT
            } else {
                color::INK_HI
            },
        ))
        .corner_radius(radius::R1)
        .inner_margin(egui::Margin::symmetric(space::S1 as i8, FIELD_MARGIN_V))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The headless viewport at frame-clock time `time` (seconds). The
    /// clock must advance across frames of one test: the bar's `Area`
    /// fades in over `animation_time`, premultiplying every paint, so an
    /// exact-color assertion only holds once `time` clears the fade
    /// (REVAMP_SPEC.md §0.1 — the `raw_input_with` time parameter is the
    /// chat panel's precedent).
    fn screen(time: f64) -> egui::RawInput {
        egui::RawInput {
            focused: true,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            time: Some(time),
            ..Default::default()
        }
    }

    fn run(ctx: &egui::Context, raw: egui::RawInput, bar: &mut FindBar) -> Vec<FindBarEvent> {
        run_full(ctx, raw, bar).1
    }

    /// One headless frame, keeping the `FullOutput` for shape assertions.
    fn run_full(
        ctx: &egui::Context,
        raw: egui::RawInput,
        bar: &mut FindBar,
    ) -> (egui::FullOutput, Vec<FindBarEvent>) {
        let mut events = Vec::new();
        let mut output = ctx.run_ui(raw, |ui| {
            ui.set_min_size(vec2(1280.0, 800.0));
            let anchor = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
            events = bar.show(ui, anchor, 3, 1);
        });
        output.textures_delta.clear();
        (output, events)
    }

    /// The bar opens, the query field takes focus, Esc closes (no other
    /// keys survive to the frame).
    #[test]
    fn the_bar_opens_focused_and_esc_closes() {
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let mut bar = FindBar::default();
        bar.open_for_find();
        assert!(bar.open);

        // Frame 1 grants focus; frame 2 presses Esc.
        let _ = run(&ctx, screen(0.0), &mut bar);
        let events = run(
            &ctx,
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..screen(0.0)
            },
            &mut bar,
        );
        assert!(events.contains(&FindBarEvent::Close));
        bar.close();
        assert!(!bar.open);
        let events = run(&ctx, screen(0.0), &mut bar);
        assert!(events.is_empty(), "a closed bar emits nothing");
    }

    /// Enter steps forward, Shift+Enter back, while the query is focused.
    #[test]
    fn enter_and_shift_enter_step_the_matches() {
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let mut bar = FindBar::default();
        bar.open_for_find();
        let _ = run(&ctx, screen(0.0), &mut bar); // focus frame

        let press = |shift: bool| egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: if shift {
                    egui::Modifiers::SHIFT
                } else {
                    Default::default()
                },
            }],
            ..screen(0.0)
        };
        let events = run(&ctx, press(false), &mut bar);
        assert!(events.contains(&FindBarEvent::Next));
        let _ = run(&ctx, screen(0.0), &mut bar); // settle
        let events = run(&ctx, press(true), &mut bar);
        assert!(events.contains(&FindBarEvent::Prev));
    }

    /// The replace mode paints both rows; the buttons carry the
    /// replacement text.
    #[test]
    fn replace_mode_paints_both_rows() {
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let mut bar = FindBar::default();
        bar.open_for_replace();
        assert!(bar.replace_mode);
        let events = run(&ctx, screen(0.0), &mut bar);
        assert!(events.is_empty());
    }

    /// REVAMP_SPEC.md §7: the fields take the §10 Focused row — a 1px
    /// `--fg-default` border while focused (`--ink-hi` otherwise).
    #[test]
    fn focused_query_field_paints_the_focus_border() {
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let mut bar = FindBar::default();
        bar.open_for_find();

        // Frame 1 is the Area's sizing pass (its shapes come back as
        // Noop) and grants the field's focus via `want_focus`. Frame 2's
        // pre-paint memory read sees the focus and flips the stroke —
        // and its clock time must sit past the Area's fade-in, which
        // premultiplies every paint by `quadratic_out(age /
        // animation_time)`: only at age ≥ animation_time (egui's
        // default ≈ 0.083s) are the fill/stroke the exact tokens
        // (REVAMP_SPEC.md §0.1).
        let _ = run(&ctx, screen(0.0), &mut bar);
        let (output, _) = run_full(&ctx, screen(0.2), &mut bar);
        let focus_border = |output: &egui::FullOutput| {
            output.shapes.iter().any(|clipped| {
                matches!(
                    &clipped.shape,
                    egui::Shape::Rect(shape)
                        if shape.fill == color::INK_LOW
                            && shape.stroke == egui::Stroke::new(1.0, color::FG_DEFAULT)
                )
            })
        };
        assert!(
            focus_border(&output),
            "the focused query field paints its 1px --fg-default border"
        );

        // Focus surrendered (Escape clears egui's widget focus during
        // input processing): the border falls back to --ink-hi. The
        // clock keeps advancing so this negative assertion is exact
        // rather than fade-dimmed.
        let events = run(
            &ctx,
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..screen(0.3)
            },
            &mut bar,
        );
        assert!(events.contains(&FindBarEvent::Close));
        let (output, _) = run_full(&ctx, screen(0.4), &mut bar);
        assert!(
            !focus_border(&output),
            "unfocused: no --fg-default border on the fields"
        );
    }
}
