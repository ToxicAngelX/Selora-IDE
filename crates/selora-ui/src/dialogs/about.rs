//! The About dialog (Phase 6b, UI_SPEC.md §11).
//!
//! A 400×280 centered modal: the 48px brand mark, the wordmark, the edition
//! and version line, a hairline, the license line, the repo link, and the
//! two buttons ("Check for updates" / "Close"). The dialog is a widget: the
//! update check itself is the shell's `Effect::CheckForUpdates` (the widget
//! never touches the network); the result arrives as a toast (UI_SPEC.md
//! §11).
//!
//! Every color, font, spacing, and radius comes from `selora-tokens`
//! (DESIGN_TOKENS.md §12). The version string is handed in at construction
//! (the app's `CARGO_PKG_VERSION` — RELEASE.md §5).

use crate::theme::{color, font, radius, space, type_scale};

use crate::brand;

/// Modal size (UI_SPEC.md §11): 400×280.
const WIDTH: f32 = 400.0;
const HEIGHT: f32 = 280.0;
/// The mark at about scale (UI_SPEC.md §11): 48px.
const MARK_SIZE: f32 = 48.0;
/// Button geometry (matches the splash's row).
const BUTTON_HEIGHT: f32 = 32.0;
const BUTTON_WIDTH: f32 = 152.0;

/// The repository's public home (RELEASE.md §7).
pub const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");
/// The repo's short display form (UI_SPEC.md §11 shows it without the
/// scheme).
fn repo_display() -> &'static str {
    REPO_URL
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_start_matches("https://")
        .trim_start_matches("http://")
}

/// Events the dialog emits to the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AboutEvent {
    /// Esc, the Close button, or a backdrop click.
    Close,
    /// "Check for updates" — the shell sends `Effect::CheckForUpdates`.
    CheckForUpdates,
}

/// The About dialog. Purely presentational; the shell hands the in-flight
/// update-check flag in via [`AboutDialog::set_checking`].
pub struct AboutDialog {
    /// The running build's version (e.g. `0.1.0`), shown as
    /// `Lite Edition · v0.1.0`.
    version: String,
    /// Whether an update check is in flight (the button goes display-only).
    checking: bool,
    events: Vec<AboutEvent>,
}

impl AboutDialog {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            checking: false,
            events: Vec::new(),
        }
    }

    /// The shell hands over the in-flight state once per frame.
    pub fn set_checking(&mut self, checking: bool) {
        self.checking = checking;
    }

    /// Paint the modal and return the events emitted this frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Vec<AboutEvent> {
        // Esc closes (no text fields here — the Settings dialog's two-step
        // Esc exists to protect half-typed input, which this dialog lacks).
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.events.push(AboutEvent::Close);
        }

        let frame = egui::Frame::default()
            .fill(color::INK_MID)
            .stroke(egui::Stroke::new(1.0, color::INK_HI))
            .corner_radius(radius::R2)
            .inner_margin(egui::Margin::same(space::S5 as i8));

        let modal = egui::Modal::new(egui::Id::new("selora-about-dialog"))
            .backdrop_color(color::SCRIM)
            .frame(frame)
            .show(ctx, |ui| {
                // Pinned both ends: the modal sizes to content, and the
                // available-size feedback loop made the Settings dialog's
                // layout drift until it was pinned — same rule here.
                ui.set_min_size(egui::vec2(WIDTH, HEIGHT));
                ui.set_max_size(egui::vec2(WIDTH, HEIGHT));
                self.body(ui);
            });
        if modal.backdrop_response.clicked() {
            self.events.push(AboutEvent::Close);
        }

        std::mem::take(&mut self.events)
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        let content = ui.max_rect();
        let center_x = content.center().x;
        let mut cursor = content.top() + space::S2;

        // The 48px mark (static — the About dialog is not a loading
        // surface; the orbit belongs to the splash, UI_SPEC.md §7).
        let mark_rect = egui::Rect::from_center_size(
            egui::pos2(center_x, cursor + MARK_SIZE / 2.0),
            egui::vec2(MARK_SIZE, MARK_SIZE),
        );
        brand::paint_mark(ui.painter(), mark_rect);
        cursor += MARK_SIZE + space::S3;

        ui.painter().text(
            egui::pos2(center_x, cursor + type_scale::LG / 2.0),
            egui::Align2::CENTER_CENTER,
            "Selora IDE",
            egui::FontId::new(type_scale::LG, font::family_sans_semibold()),
            color::FG_DEFAULT,
        );
        cursor += type_scale::LG + space::S1;

        ui.painter().text(
            egui::pos2(center_x, cursor + type_scale::SM / 2.0),
            egui::Align2::CENTER_CENTER,
            format!("Lite Edition · v{}", self.version),
            egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
            color::FG_MUTED,
        );
        cursor += type_scale::SM + space::S3;

        // The hairline (UI_SPEC.md §11), inset from the modal's edges.
        let rule = egui::Rect::from_min_size(
            egui::pos2(content.left() + space::S4, cursor),
            egui::vec2(content.width() - 2.0 * space::S4, 1.0),
        );
        ui.painter().rect_filled(rule, radius::R0, color::INK_HI);
        cursor += 1.0 + space::S3;

        ui.painter().text(
            egui::pos2(center_x, cursor + type_scale::SM / 2.0),
            egui::Align2::CENTER_CENTER,
            "Open source · MIT",
            egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
            color::FG_MUTED,
        );
        cursor += type_scale::SM + space::S3;

        // The repo link — a real hyperlink (eframe opens the system
        // browser; the widget stays offline).
        let link_rect = egui::Rect::from_center_size(
            egui::pos2(center_x, cursor + type_scale::SM / 2.0),
            egui::vec2(260.0, type_scale::SM + space::S2),
        );
        let mut link_ui = ui.new_child(egui::UiBuilder::new().max_rect(link_rect));
        link_ui.centered_and_justified(|ui| {
            ui.hyperlink_to(
                egui::RichText::new(repo_display())
                    .color(color::FG_MUTED)
                    .font(egui::FontId::new(type_scale::SM, font::FAMILY_SANS)),
                REPO_URL,
            );
        });

        // The buttons, bottom-centered.
        let buttons_width = BUTTON_WIDTH + space::S3 + BUTTON_WIDTH;
        let buttons_rect = egui::Rect::from_min_size(
            egui::pos2(
                center_x - buttons_width / 2.0,
                content.bottom() - BUTTON_HEIGHT - space::S1,
            ),
            egui::vec2(buttons_width, BUTTON_HEIGHT),
        );
        let mut buttons_ui = ui.new_child(egui::UiBuilder::new().max_rect(buttons_rect));
        buttons_ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            self.button(
                ui,
                "Check for updates",
                !self.checking,
                AboutEvent::CheckForUpdates,
            );
            ui.add_space(space::S3);
            self.button(ui, "Close", true, AboutEvent::Close);
        });
    }

    /// One boxed button (--ink-mid fill, 1px --ink-hi hairline,
    /// --radius-1); `enabled = false` paints it dimmed and inert (the
    /// in-flight update check).
    fn button(&mut self, ui: &mut egui::Ui, label: &str, enabled: bool, event: AboutEvent) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(BUTTON_WIDTH, BUTTON_HEIGHT),
            egui::Sense::click(),
        );
        let fill = if enabled && response.hovered() {
            color::INK_HI
        } else {
            color::INK_LOW
        };
        ui.painter().rect_filled(rect, radius::R1, fill);
        ui.painter().rect_stroke(
            rect,
            radius::R1,
            egui::Stroke::new(1.0, color::INK_HI),
            egui::StrokeKind::Inside,
        );
        let label_color = if enabled {
            color::FG_DEFAULT
        } else {
            color::FG_DISABLED
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
            label_color,
        );
        if enabled && response.clicked() {
            self.events.push(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_repository_url_comes_from_the_manifest() {
        assert!(REPO_URL.starts_with("https://github.com/"));
        let display = repo_display();
        assert!(!display.contains("://"));
        assert!(display.starts_with("github.com/"));
        assert_eq!(display, REPO_URL.trim_start_matches("https://"));
    }

    struct Harness {
        ctx: egui::Context,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::set_fonts(&ctx);
            Self { ctx }
        }

        fn frame(&self, dialog: &mut AboutDialog, events: Vec<egui::Event>) -> Vec<AboutEvent> {
            let mut out = Vec::new();
            self.ctx
                .run_ui(
                    egui::RawInput {
                        focused: true,
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        out = dialog.show(ui.ctx());
                    },
                )
                .drop_without_applying_deltas();
            out
        }

        /// Move + press + release over three frames (the click pattern an
        /// egui overlay needs: `clicked()` fires on the release frame).
        fn click(&self, dialog: &mut AboutDialog, pos: egui::Pos2) -> Vec<AboutEvent> {
            self.frame(dialog, vec![egui::Event::PointerMoved(pos)]);
            self.frame(dialog, vec![egui::Event::PointerMoved(pos)]);
            self.frame(
                dialog,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
            );
            self.frame(
                dialog,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            )
        }
    }

    /// The modal is centered on the 1280×800 headless screen.
    fn modal_rect() -> egui::Rect {
        egui::Rect::from_center_size(
            egui::pos2(640.0, 400.0),
            egui::vec2(WIDTH + 2.0 * space::S5, HEIGHT + 2.0 * space::S5),
        )
    }

    #[test]
    fn idle_dialog_emits_no_events() {
        let harness = Harness::new();
        let mut dialog = AboutDialog::new("0.1.0");
        let events = harness.frame(&mut dialog, vec![]);
        assert!(events.is_empty());
    }

    #[test]
    fn esc_closes() {
        let harness = Harness::new();
        let mut dialog = AboutDialog::new("0.1.0");
        let events = harness.frame(
            &mut dialog,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(events, vec![AboutEvent::Close]);
    }

    #[test]
    fn close_button_closes() {
        let harness = Harness::new();
        let mut dialog = AboutDialog::new("0.1.0");
        // The Close button is the right-hand of the two bottom buttons.
        let modal = modal_rect();
        let buttons_y = modal.bottom() - space::S5 - space::S1 - BUTTON_HEIGHT / 2.0;
        let close_x = modal.center().x + (BUTTON_WIDTH + space::S3) / 2.0;
        let events = harness.click(&mut dialog, egui::pos2(close_x, buttons_y));
        assert_eq!(events, vec![AboutEvent::Close]);
    }

    #[test]
    fn check_for_updates_emits_once_per_click() {
        let harness = Harness::new();
        let mut dialog = AboutDialog::new("0.1.0");
        let modal = modal_rect();
        let buttons_y = modal.bottom() - space::S5 - space::S1 - BUTTON_HEIGHT / 2.0;
        let check_x = modal.center().x - (BUTTON_WIDTH + space::S3) / 2.0;
        let events = harness.click(&mut dialog, egui::pos2(check_x, buttons_y));
        assert_eq!(events, vec![AboutEvent::CheckForUpdates]);
    }

    #[test]
    fn the_button_is_inert_while_a_check_is_in_flight() {
        let harness = Harness::new();
        let mut dialog = AboutDialog::new("0.1.0");
        dialog.set_checking(true);
        let modal = modal_rect();
        let buttons_y = modal.bottom() - space::S5 - space::S1 - BUTTON_HEIGHT / 2.0;
        let check_x = modal.center().x - (BUTTON_WIDTH + space::S3) / 2.0;
        let events = harness.click(&mut dialog, egui::pos2(check_x, buttons_y));
        assert!(events.is_empty());
    }
}
