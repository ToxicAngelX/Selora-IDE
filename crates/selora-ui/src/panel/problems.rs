//! The problems panel (Phase R6) — region ④'s diagnostics list: the
//! latest LSP diagnostics grouped by file. A row click opens the buffer
//! at the diagnostic's line (the shell's open-and-jump queue).
//!
//! Severity glyphs: `✕` in `--err` (error), `!` in `--warn` (warning),
//! `i` in `--info`, `·` faint (hint) — the diagnostics' own semantic
//! palette, nothing invented (§12). Empty state: a muted note.

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{pos2, vec2, Align2, Sense, Ui};
use selora_editor::lsp_overlay::{LspDiagnostic, LspSeverity};

use crate::theme::{color, font, radius, space, type_scale};

/// One row's height.
const ROW_HEIGHT: f32 = 24.0;
/// A file group's header height.
const GROUP_HEIGHT: f32 = 24.0;
/// The glyph slot.
const GLYPH_WIDTH: f32 = 18.0;

/// What the panel asks the shell for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProblemsEvent {
    /// A row was clicked — open the file at the 1-based line.
    OpenAtLine { path: PathBuf, line: usize },
}

/// Paint the problems list. `diagnostics` is the aggregated per-path
/// store (path order; insertion order within).
pub fn show(
    ui: &mut Ui,
    diagnostics: &BTreeMap<PathBuf, Vec<LspDiagnostic>>,
) -> Vec<ProblemsEvent> {
    let mut events = Vec::new();
    let width = ui.available_width();

    if diagnostics.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
        ui.painter().text(
            pos2(rect.left() + space::S3, rect.center().y),
            Align2::LEFT_CENTER,
            "No problems detected.",
            egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
            color::FG_MUTED,
        );
        return events;
    }

    for (path, items) in diagnostics {
        // The file group header: the path (relative to the workspace
        // root when possible), the count right.
        let (header, _) = ui.allocate_exact_size(vec2(width, GROUP_HEIGHT), Sense::hover());
        let shown = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        ui.painter().text(
            pos2(header.left() + space::S3, header.center().y),
            Align2::LEFT_CENTER,
            format!(
                "{shown} — {} problem{}",
                items.len(),
                crate::sidebar::chat::plural(items.len())
            ),
            egui::FontId::new(type_scale::XS, font::family_sans_semibold()),
            color::FG_MUTED,
        );

        for diagnostic in items {
            let (row, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(row, radius::R0, color::INK_FAINT);
            }
            let line = diagnostic.range.start.line as usize + 1;
            let (glyph, glyph_color) = severity_glyph(diagnostic.severity);
            ui.painter().text(
                pos2(row.left() + space::S3, row.center().y),
                Align2::LEFT_CENTER,
                glyph,
                egui::FontId::new(type_scale::XS, font::FAMILY_MONO),
                glyph_color,
            );
            // The message, elided to the row.
            let message = elide(ui, &diagnostic.message, width - GLYPH_WIDTH - 64.0);
            ui.painter().text(
                pos2(row.left() + space::S3 + GLYPH_WIDTH, row.center().y),
                Align2::LEFT_CENTER,
                message,
                egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                if response.hovered() {
                    color::FG_DEFAULT
                } else {
                    color::FG_MUTED
                },
            );
            ui.painter().text(
                pos2(row.right() - space::S3, row.center().y),
                Align2::RIGHT_CENTER,
                format!("line {line}"),
                egui::FontId::new(type_scale::XS, font::FAMILY_MONO),
                color::FG_FAINT,
            );
            if response.clicked() {
                events.push(ProblemsEvent::OpenAtLine {
                    path: path.clone(),
                    line,
                });
            }
        }
    }
    events
}

/// The severity glyph + color (the diagnostics' semantic palette).
fn severity_glyph(severity: LspSeverity) -> (&'static str, egui::Color32) {
    match severity {
        LspSeverity::Error => ("✕", color::ERR),
        LspSeverity::Warning => ("!", color::WARN),
        LspSeverity::Information => ("i", color::INFO),
        LspSeverity::Hint => ("·", color::FG_FAINT),
    }
}

/// Elide `text` to `max_width` at the row's font (a char boundary, `…`).
fn elide(ui: &Ui, text: &str, max_width: f32) -> String {
    let font = egui::FontId::new(type_scale::SM, font::FAMILY_SANS);
    if ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), color::FG_MUTED)
        .size()
        .x
        <= max_width
    {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(8).collect();
    let mut chars = text.chars().peekable();
    while let Some(next) = chars.next() {
        let candidate = format!("{cut}{next}…");
        if ui
            .painter()
            .layout_no_wrap(candidate.clone(), font.clone(), color::FG_MUTED)
            .size()
            .x
            > max_width
        {
            break;
        }
        cut.push(next);
        let _ = chars.peek();
    }
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(severity: LspSeverity, message: &str, line: u32) -> LspDiagnostic {
        LspDiagnostic {
            range: selora_editor::lsp_overlay::LspRange {
                start: selora_editor::lsp_overlay::LspPosition { line, character: 0 },
                end: selora_editor::lsp_overlay::LspPosition { line, character: 3 },
            },
            severity,
            source: Some("rust-analyzer".into()),
            message: message.into(),
        }
    }

    fn diagnostics() -> BTreeMap<PathBuf, Vec<LspDiagnostic>> {
        let mut map = BTreeMap::new();
        map.insert(
            PathBuf::from("/repo/src/main.rs"),
            vec![
                diagnostic(LspSeverity::Error, "cannot find value `x`", 12),
                diagnostic(LspSeverity::Warning, "unused variable: `y`", 30),
            ],
        );
        map
    }

    /// The panel paints grouped rows; the empty state paints its note.
    #[test]
    fn problems_panel_paints_groups_and_empty_state() {
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 200.0));
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw.clone(), |ui| {
            let events = show(ui, &diagnostics());
            assert!(events.is_empty(), "no click, no events");
        });
        output.textures_delta.clear();

        let mut output = ctx.run_ui(raw, |ui| {
            let events = show(ui, &BTreeMap::new());
            assert!(events.is_empty());
        });
        output.textures_delta.clear();
    }

    /// A row click emits OpenAtLine with the file and the 1-based line.
    #[test]
    fn a_row_click_opens_the_file_at_line() {
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 200.0));
        let raw = |events: Vec<egui::Event>| egui::RawInput {
            focused: true,
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        // Two settle frames, then click the first diagnostic row (the
        // group header + row 1: y ≈ 24..48).
        let diags = diagnostics();
        for _ in 0..2 {
            let mut output = ctx.run_ui(raw(vec![]), |ui| {
                let _ = show(ui, &diags);
            });
            output.textures_delta.clear();
        }
        let pos = egui::pos2(200.0, 36.0);
        let mut got = None;
        for pressed in [true, false] {
            let mut output = ctx.run_ui(
                raw(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]),
                |ui| {
                    for event in show(ui, &diags) {
                        got = Some(event);
                    }
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(
            got,
            Some(ProblemsEvent::OpenAtLine {
                path: PathBuf::from("/repo/src/main.rs"),
                line: 13,
            })
        );
    }
}
