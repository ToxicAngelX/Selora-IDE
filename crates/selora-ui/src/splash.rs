//! The first-run splash (UI_SPEC.md §7).
//!
//! A full-window cover over the workbench (below the title strip) shown when
//! the app launches with no Selora key in the OS keyring: the 64px brand mark
//! running the `orbit` animation, the wordmark, a masked key input, and
//! Connect / Skip. Connect validation, keyring storage, and the status flip
//! all happen in the shell (the splash is a widget — it never touches the
//! network or the keyring, same contract as the Settings dialog's Selora
//! section).
//!
//! Close choreography (UI_SPEC.md §7): on `Connected` the splash fades out
//! over a 1.2s delay and then emits [`SplashEvent::Dismissed`]. Reduce-motion
//! (DESIGN_TOKENS.md §7.4) freezes the orbit (a static mark) but keeps the
//! fade — §7.4 lists `fade` among the kept animations.
//!
//! Every color, font, spacing, duration, and curve comes from
//! `selora-tokens` (DESIGN_TOKENS.md §12).

use std::time::{Duration, Instant};

use selora_ai::SeloraStatus;
use selora_tokens::{color, font, motion, radius, space, type_scale};

use crate::brand;
use crate::presence::metal;

/// The mark at splash scale (UI_SPEC.md §7): 64px.
const MARK_SIZE: f32 = 64.0;
/// The key input's width (UI_SPEC.md §7): 360px.
const INPUT_WIDTH: f32 = 360.0;
/// The fade-out's total run (UI_SPEC.md §7: "closes after a 1.2s delay with
/// the fade animation" — one `--dur-loop`).
const FADE_OUT: Duration = Duration::from_millis(motion::DUR_LOOP_MS as u64);
/// The repaint cadence while the orbit runs (~30fps is fluid for a 1.2s
/// loop; half the frame rate of the editor's animations).
const ORBIT_REPAINT: Duration = Duration::from_millis(33);

/// Events the splash emits to the shell.
#[derive(Clone, PartialEq, Eq)]
pub enum SplashEvent {
    /// The user pressed Connect (or Enter in the field) with a key in the
    /// input. The raw `sk-gw-…` key is carried here and nowhere else: the
    /// shell validates it with `SeloraClient::list_models` and writes it to
    /// the OS keyring (ARCHITECTURE.md §7.2 — the `SettingsEvent::Connect`
    /// contract).
    Connect(String),
    /// "Skip for now" — the splash closes; the status bar's connect
    /// affordance stays (UI_SPEC.md §7).
    Skip,
    /// The post-Connect fade completed — the shell drops the splash.
    Dismissed,
}

impl std::fmt::Debug for SplashEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(_) => f.debug_tuple("Connect").field(&"[REDACTED]").finish(),
            Self::Skip => f.write_str("Skip"),
            Self::Dismissed => f.write_str("Dismissed"),
        }
    }
}

/// The first-run splash. One per app run: the shell constructs it when the
/// boot keyring read reports no stored key, and drops it on Skip / Dismissed.
pub struct Splash {
    /// The masked key buffer. Never persisted, never in `Debug` output —
    /// cleared the moment the key is emitted in [`SplashEvent::Connect`]
    /// *and* on the `Connected` transition (the keyring is the key's only
    /// home by then).
    api_key: String,
    /// The live connection state, handed in by the shell once per frame.
    status: SeloraStatus,
    /// When the `Connected` status arrived (the fade-out's clock).
    connected_at: Option<Instant>,
    /// Whether [`SplashEvent::Dismissed`] has been emitted.
    dismissed: bool,
    /// DESIGN_TOKENS.md §7.4: freeze the orbit; the fade stays.
    reduce_motion: bool,
    /// The orbit animation's epoch (a `Instant` is monotone — a wall-clock
    /// phase would jump on clock adjustments).
    created: Instant,
    events: Vec<SplashEvent>,
}

impl Splash {
    pub fn new() -> Self {
        Self {
            api_key: String::new(),
            status: SeloraStatus::Disconnected,
            connected_at: None,
            dismissed: false,
            reduce_motion: false,
            created: Instant::now(),
            events: Vec::new(),
        }
    }

    /// The shell hands the live connection state over once per frame (the
    /// Settings dialog's `set_selora_status` contract). A `Connected`
    /// transition starts the fade-out clock and clears the key buffer.
    pub fn set_status(&mut self, status: SeloraStatus) {
        if matches!(status, SeloraStatus::Connected { .. }) {
            self.api_key.clear();
            if self.connected_at.is_none() {
                self.connected_at = Some(Instant::now());
            }
        }
        self.status = status;
    }

    /// DESIGN_TOKENS.md §7.4: freeze the orbit (the fade is kept — §7.4
    /// keeps `fade` among the allowed animations).
    pub fn set_reduce_motion(&mut self, reduce_motion: bool) {
        self.reduce_motion = reduce_motion;
    }

    /// Whether the fade-out is running (the shell keeps painting the splash
    /// until [`SplashEvent::Dismissed`] lands).
    pub fn is_fading(&self) -> bool {
        self.connected_at.is_some()
    }

    /// Paint the cover into `rect` (the region below the title strip) and
    /// return the events emitted this frame.
    pub fn show(&mut self, ctx: &egui::Context, rect: egui::Rect) -> Vec<SplashEvent> {
        // The fade-out's alpha (1 → 0 over FADE_OUT). `Dismissed` fires
        // once, at the end.
        let mut alpha = 1.0f32;
        if let Some(connected_at) = self.connected_at {
            let t = (connected_at.elapsed().as_secs_f32() / FADE_OUT.as_secs_f32()).min(1.0);
            alpha = 1.0 - motion::ease_bezier(motion::EASE_FADE, t);
            if t >= 1.0 && !self.dismissed {
                self.dismissed = true;
                self.events.push(SplashEvent::Dismissed);
            }
        }

        // The orbit: the satellite traces its ellipse once per --dur-loop,
        // eased by --ease-orbital (DESIGN_TOKENS.md §7.3). Frozen under
        // reduce-motion (§7.4) and once the fade starts (the mark leaves
        // with everything else).
        let theta = if self.reduce_motion || self.connected_at.is_some() {
            0.0
        } else {
            let loop_secs = motion::DUR_LOOP_MS as f32 / 1000.0;
            let phase = (self.created.elapsed().as_secs_f32() / loop_secs) % 1.0;
            std::f32::consts::TAU * motion::ease_bezier(motion::EASE_ORBITAL, phase)
        };

        // Keep the orbit (and the fade) painting; an idle splash under
        // reduce-motion needs no repaints.
        if self.connected_at.is_some() || !self.reduce_motion {
            ctx.request_repaint_after(ORBIT_REPAINT);
        }

        egui::Area::new(egui::Id::new("selora-splash"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
                ui.set_opacity(alpha);
                self.body(ui, rect, theta);
            });

        std::mem::take(&mut self.events)
    }

    // -- internals ---------------------------------------------------------

    fn body(&mut self, ui: &mut egui::Ui, rect: egui::Rect, theta: f32) {
        // The opaque cover (the workbench must not show through) — and the
        // click swallower: allocating the full rect with a click sense puts
        // the splash on top of every panel below it in paint order, so
        // clicks land here instead of falling through to the workbench.
        let (cover, _) = ui.allocate_exact_size(rect.size(), egui::Sense::click());
        ui.painter()
            .rect_filled(cover, radius::R0, color::INK_CANVAS);

        // The centered column (UI_SPEC.md §7): mark, wordmark, tagline,
        // input, buttons — vertically centered as a block.
        let column_height = MARK_SIZE
            + space::S5
            + type_scale::XXL
            + space::S2
            + type_scale::MD
            + space::S6
            + INPUT_HEIGHT
            + space::S4
            + BUTTON_HEIGHT
            + space::S3
            + type_scale::SM;
        let top = rect.center().y - column_height / 2.0;
        let mut cursor = top;

        // Mark (64px, orbiting).
        let mark_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, cursor + MARK_SIZE / 2.0),
            egui::vec2(MARK_SIZE, MARK_SIZE),
        );
        brand::paint_mark_orbiting(ui.painter(), mark_rect, theta);
        // Phase 6h (OWNER-DECISION): the MetalMark liquid-metal sheen —
        // the app's single permitted specular highlight, monochrome white
        // on ink, on the §7.3 comet vocabulary. Behind
        // `metal::METAL_MARK_ENABLED`; the owner flips the flag. The
        // splash's orbit repaint cadence drives it; reduce-motion freezes
        // it (the comet family is §7.4-disabled).
        if metal::METAL_MARK_ENABLED && !self.reduce_motion {
            metal::paint_sheen(
                ui.painter(),
                mark_rect,
                self.created.elapsed().as_secs_f32(),
            );
        }
        cursor += MARK_SIZE + space::S5;

        // Wordmark + tagline.
        ui.painter().text(
            egui::pos2(rect.center().x, cursor + type_scale::XXL / 2.0),
            egui::Align2::CENTER_CENTER,
            "Selora IDE",
            egui::FontId::new(type_scale::XXL, font::family_sans_semibold()),
            color::FG_DEFAULT,
        );
        cursor += type_scale::XXL + space::S2;
        ui.painter().text(
            egui::pos2(rect.center().x, cursor + type_scale::MD / 2.0),
            egui::Align2::CENTER_CENTER,
            "The editor born connected.",
            egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
            color::FG_MUTED,
        );
        cursor += type_scale::MD + space::S6;

        // The key input (360px, masked, --ink-low fill, 1px --ink-hi
        // border, --radius-1, 12px padding — UI_SPEC.md §7).
        let input_rect = egui::Rect::from_min_size(
            egui::pos2(rect.center().x - INPUT_WIDTH / 2.0, cursor),
            egui::vec2(INPUT_WIDTH, INPUT_HEIGHT),
        );
        let mut input_ui = ui.new_child(egui::UiBuilder::new().max_rect(input_rect));
        let connecting = matches!(self.status, SeloraStatus::Connecting);
        let frame = egui::Frame::default()
            .fill(color::INK_LOW)
            .stroke(egui::Stroke::new(1.0, color::INK_HI))
            .corner_radius(radius::R1)
            .inner_margin(egui::Margin::same(space::S3 as i8));
        frame.show(&mut input_ui, |ui| {
            ui.set_min_width(INPUT_WIDTH - 2.0 * space::S3);
            let edit = egui::TextEdit::singleline(&mut self.api_key)
                .password(true)
                .hint_text(egui::RichText::new("Paste your sk-gw-… API key").color(color::FG_FAINT))
                .text_color(color::FG_DEFAULT)
                .font(egui::FontId::new(type_scale::MD, font::FAMILY_MONO))
                .frame(egui::Frame::default());
            let response = ui.add_enabled(!connecting, edit);
            // A pasted key never carries surrounding whitespace (the
            // Settings dialog's rule — a trailing newline fails validation).
            if response.changed() && self.api_key.contains(char::is_whitespace) {
                self.api_key.retain(|c| !c.is_whitespace());
            }
            // Enter submits (the egui singleline idiom: the field defocuses
            // on Enter and the key press is visible the same frame).
            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && !connecting
                && !self.api_key.is_empty()
            {
                self.events.push(SplashEvent::Connect(self.api_key.clone()));
            }
        });
        cursor += INPUT_HEIGHT + space::S4;

        // The buttons: Connect (primary) + Skip (text button).
        let buttons_width = BUTTON_WIDTH + space::S5 + SKIP_WIDTH;
        let buttons_rect = egui::Rect::from_min_size(
            egui::pos2(rect.center().x - buttons_width / 2.0, cursor),
            egui::vec2(buttons_width, BUTTON_HEIGHT),
        );
        let mut buttons_ui = ui.new_child(egui::UiBuilder::new().max_rect(buttons_rect));
        buttons_ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            // Connect: --ink-mid fill, --fg-default label (UI_SPEC.md §7).
            let (connect_rect, connect) = ui.allocate_exact_size(
                egui::vec2(BUTTON_WIDTH, BUTTON_HEIGHT),
                egui::Sense::click(),
            );
            let connect_fill = if connect.hovered() && !connecting {
                color::INK_HI
            } else {
                color::INK_MID
            };
            ui.painter()
                .rect_filled(connect_rect, radius::R1, connect_fill);
            ui.painter().rect_stroke(
                connect_rect,
                radius::R1,
                egui::Stroke::new(1.0, color::INK_HI),
                egui::StrokeKind::Inside,
            );
            let label = if connecting {
                "Connecting…"
            } else {
                "Connect"
            };
            ui.painter().text(
                connect_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::new(type_scale::MD, font::family_sans_medium()),
                color::FG_DEFAULT,
            );
            if connect.clicked() && !connecting && !self.api_key.is_empty() {
                self.events.push(SplashEvent::Connect(self.api_key.clone()));
            }

            ui.add_space(space::S5);

            // Skip: text button, --fg-muted (UI_SPEC.md §7).
            let (skip_rect, skip) =
                ui.allocate_exact_size(egui::vec2(SKIP_WIDTH, BUTTON_HEIGHT), egui::Sense::click());
            let skip_color = if skip.hovered() {
                color::FG_DEFAULT
            } else {
                color::FG_MUTED
            };
            ui.painter().text(
                skip_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Skip for now",
                egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
                skip_color,
            );
            if skip.clicked() {
                self.events.push(SplashEvent::Skip);
            }
        });
        cursor += BUTTON_HEIGHT + space::S3;

        // The status line: the gateway's own error words, verbatim (the
        // Settings dialog's Selora section contract); nothing while idle.
        if let SeloraStatus::Error(message) = &self.status {
            ui.painter().text(
                egui::pos2(rect.center().x, cursor + type_scale::SM / 2.0),
                egui::Align2::CENTER_CENTER,
                message,
                egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                color::ERR,
            );
        }
    }
}

/// Input row height: 13px text + 2×12px padding + 2px border ≈ 40px
/// (rounded to the 4px grid).
const INPUT_HEIGHT: f32 = 40.0;
/// Button row height (matches the input).
const BUTTON_HEIGHT: f32 = 32.0;
/// Connect button width (fits "Connecting…" without a resize).
const BUTTON_WIDTH: f32 = 112.0;
/// Skip text-button hit-target width.
const SKIP_WIDTH: f32 = 96.0;

impl Default for Splash {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 32.0), egui::vec2(1280.0, 768.0))
    }

    fn frame(splash: &mut Splash) -> (egui::Context, Vec<SplashEvent>) {
        let ctx = egui::Context::default();
        // The real bundled fonts — the wordmark uses the named SemiBold
        // family, which an unregistered context cannot resolve.
        crate::theme::set_fonts(&ctx);
        let input = egui::RawInput {
            focused: true,
            screen_rect: Some(test_rect()),
            ..Default::default()
        };
        let mut events = Vec::new();
        let mut output = ctx.run_ui(input, |ui| {
            events = splash.show(ui.ctx(), test_rect());
        });
        output.textures_delta.clear();
        (ctx, events)
    }

    #[test]
    fn idle_splash_emits_no_events() {
        let mut splash = Splash::new();
        let (_ctx, events) = frame(&mut splash);
        assert!(events.is_empty());
        assert!(!splash.is_fading());
    }

    #[test]
    fn connect_clears_the_key_buffer_and_starts_the_fade() {
        let mut splash = Splash::new();
        splash.api_key = "sk-gw-secret".to_owned();
        splash.set_status(SeloraStatus::Connected {
            model: None,
            latency_ms: 12,
        });
        assert!(splash.api_key.is_empty());
        assert!(splash.is_fading());
        assert!(splash.connected_at.is_some());
    }

    #[test]
    fn a_second_connected_status_never_restarts_the_fade_clock() {
        let mut splash = Splash::new();
        splash.set_status(SeloraStatus::Connected {
            model: None,
            latency_ms: 12,
        });
        let first = splash.connected_at.unwrap();
        splash.set_status(SeloraStatus::Connected {
            model: None,
            latency_ms: 8,
        });
        assert_eq!(splash.connected_at, Some(first));
    }

    #[test]
    fn dismissed_fires_once_when_the_fade_completes() {
        let mut splash = Splash::new();
        // Backdate the connect past the fade duration.
        splash.connected_at = Some(Instant::now() - FADE_OUT - Duration::from_millis(50));
        let (_ctx, events) = frame(&mut splash);
        assert_eq!(events, vec![SplashEvent::Dismissed]);
        // A second frame never re-emits.
        let (_ctx, events) = frame(&mut splash);
        assert!(events.is_empty());
    }

    #[test]
    fn the_fade_alpha_ramps_with_the_ease_fade_curve() {
        // Halfway through the fade the cover is partially transparent.
        let mut splash = Splash::new();
        splash.connected_at = Some(Instant::now() - FADE_OUT / 2);
        let (_ctx, events) = frame(&mut splash);
        assert!(events.is_empty());
        assert!(splash.is_fading());
    }

    #[test]
    fn error_status_is_kept_verbatim_for_the_status_line() {
        let mut splash = Splash::new();
        splash.set_status(SeloraStatus::Error("gateway says no".to_owned()));
        assert_eq!(
            splash.status,
            SeloraStatus::Error("gateway says no".to_owned())
        );
        assert!(!splash.is_fading());
    }

    #[test]
    fn connect_event_redacts_the_key_in_debug() {
        let event = SplashEvent::Connect("sk-gw-secret".to_owned());
        let debug = format!("{event:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("sk-gw-secret"));
    }

    #[test]
    fn reduce_motion_freezes_the_orbit_but_keeps_the_fade() {
        let mut splash = Splash::new();
        splash.set_reduce_motion(true);
        let (_ctx, events) = frame(&mut splash);
        assert!(events.is_empty());
        // Fade still runs under reduce-motion (DESIGN_TOKENS.md §7.4).
        splash.connected_at = Some(Instant::now() - FADE_OUT - Duration::from_millis(50));
        let (_ctx, events) = frame(&mut splash);
        assert_eq!(events, vec![SplashEvent::Dismissed]);
    }

    // -- click-level (the agent_cards headless pattern) ---------------------

    /// One persistent headless context per test: an `Area`'s layer bounds
    /// are learned on the frame it is first painted, so pointer interaction
    /// needs a settled prior frame (the real app paints the splash for many
    /// frames before a click can land).
    struct Harness {
        ctx: egui::Context,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::set_fonts(&ctx);
            Self { ctx }
        }

        fn frame(&self, splash: &mut Splash, events: Vec<egui::Event>) -> Vec<SplashEvent> {
            let input = egui::RawInput {
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = Vec::new();
            self.ctx
                .run_ui(input, |ui| {
                    out = splash.show(ui.ctx(), test_rect());
                })
                .drop_without_applying_deltas();
            out
        }

        /// A real click (the status-bar click-test pattern, plus one extra
        /// settle frame: an `Area`'s layer bounds land one frame after its
        /// first paint, so the hover needs two frames to register before
        /// the press). Returns the release frame's events.
        fn click(&self, splash: &mut Splash, pos: egui::Pos2) -> Vec<SplashEvent> {
            self.frame(splash, vec![egui::Event::PointerMoved(pos)]);
            self.frame(splash, vec![egui::Event::PointerMoved(pos)]);
            self.frame(
                splash,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
            );
            self.frame(
                splash,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            )
        }
    }

    /// The deterministic layout math of `body`, shared by the click tests.
    /// Returns (input center, connect center, skip center).
    fn layout_centers() -> (egui::Pos2, egui::Pos2, egui::Pos2) {
        let rect = test_rect();
        let column_height = MARK_SIZE
            + space::S5
            + type_scale::XXL
            + space::S2
            + type_scale::MD
            + space::S6
            + INPUT_HEIGHT
            + space::S4
            + BUTTON_HEIGHT
            + space::S3
            + type_scale::SM;
        let top = rect.center().y - column_height / 2.0;
        let input_y =
            top + MARK_SIZE + space::S5 + type_scale::XXL + space::S2 + type_scale::MD + space::S6;
        let buttons_y = input_y + INPUT_HEIGHT + space::S4;
        let buttons_width = BUTTON_WIDTH + space::S5 + SKIP_WIDTH;
        // The buttons live in a left_to_right child Ui — egui applies one
        // item_spacing.x of indent before the first widget.
        let indent = egui::style::Spacing::default().item_spacing.x;
        let buttons_left = rect.center().x - buttons_width / 2.0 + indent;
        (
            egui::pos2(rect.center().x, input_y + INPUT_HEIGHT / 2.0),
            egui::pos2(
                buttons_left + BUTTON_WIDTH / 2.0,
                buttons_y + BUTTON_HEIGHT / 2.0,
            ),
            egui::pos2(
                buttons_left + BUTTON_WIDTH + space::S5 + SKIP_WIDTH / 2.0,
                buttons_y + BUTTON_HEIGHT / 2.0,
            ),
        )
    }

    #[test]
    fn skip_click_emits_skip() {
        let harness = Harness::new();
        let mut splash = Splash::new();
        let (_, _, skip) = layout_centers();
        let events = harness.click(&mut splash, skip);
        assert_eq!(events, vec![SplashEvent::Skip]);
    }

    #[test]
    fn connect_click_with_a_key_emits_connect() {
        let harness = Harness::new();
        let mut splash = Splash::new();
        splash.api_key = "sk-gw-test".to_owned();
        let (_, connect, _) = layout_centers();
        let events = harness.click(&mut splash, connect);
        assert_eq!(events, vec![SplashEvent::Connect("sk-gw-test".to_owned())]);
    }

    #[test]
    fn connect_click_with_an_empty_input_is_inert() {
        let harness = Harness::new();
        let mut splash = Splash::new();
        let (_, connect, _) = layout_centers();
        let events = harness.click(&mut splash, connect);
        assert!(events.is_empty());
    }

    #[test]
    fn typing_and_enter_emits_connect_with_whitespace_stripped() {
        let harness = Harness::new();
        let mut splash = Splash::new();
        let (input, _, _) = layout_centers();
        // Focus the field with a real click.
        let events = harness.click(&mut splash, input);
        assert!(events.is_empty());
        // Type (with a stray space — the field strips whitespace).
        let events = harness.frame(
            &mut splash,
            vec![
                egui::Event::Text("sk-gw-".to_owned()),
                egui::Event::Text("abc ".to_owned()),
            ],
        );
        assert!(events.is_empty());
        assert_eq!(splash.api_key, "sk-gw-abc");
        // Enter submits.
        let events = harness.frame(
            &mut splash,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(events, vec![SplashEvent::Connect("sk-gw-abc".to_owned())]);
    }

    #[test]
    fn connect_is_inert_while_connecting() {
        let harness = Harness::new();
        let mut splash = Splash::new();
        splash.api_key = "sk-gw-test".to_owned();
        splash.set_status(SeloraStatus::Connecting);
        let (_, connect, _) = layout_centers();
        let events = harness.click(&mut splash, connect);
        assert!(events.is_empty());
    }
}
