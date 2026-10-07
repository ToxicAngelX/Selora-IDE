//! The status bar's workbench zones (UI_SPEC.md §1.2): the left zone's git
//! branch + change count + per-server LSP status, and the center zone's
//! cursor position + language + LSP progress. Phase R0: extracted from
//! `selora-app`'s `shell.rs` — the right zone (`cost_meter`,
//! `selora_status`) was already a widget pair; these two complete the bar.
//!
//! The widget is state-in/events-out like its siblings: the shell builds a
//! [`WorkbenchInput`] snapshot from `AppState` each frame and routes
//! [`WorkbenchEvent::RestartServer`] to `Effect::Lsp(Restart)` (a dead
//! server's status run is a click target, §5.6).

use selora_lsp::{LanguageId, ServerStatus};

use crate::theme::{color, font, space, type_scale};
use selora_editor::Language;

/// The left zone's git summary (`None` outside a repository).
#[derive(Clone, Debug, Default)]
pub struct GitSummary {
    /// The branch name (empty when HEAD is detached — the bar then shows
    /// no branch run).
    pub branch: String,
    /// How many files changed (worktree + index).
    pub changed_count: usize,
}

/// The center zone's cursor snapshot for the active editor.
#[derive(Clone, Copy, Debug)]
pub struct CursorSummary {
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub col: usize,
    /// The highlighting language's display name.
    pub language: &'static str,
}

/// One frame's snapshot of everything the workbench zones paint.
#[derive(Default)]
pub struct WorkbenchInput {
    /// The git summary, or `None` outside a repository (the workspace
    /// root's name paints instead).
    pub git: Option<GitSummary>,
    /// The workspace root's file name (the no-repo left run; "no folder"
    /// when no workspace is open).
    pub workspace_name: String,
    /// Per-server LSP statuses, in the registry's order.
    pub lsp_servers: Vec<(LanguageId, ServerStatus)>,
    /// The active editor's cursor summary (`None` when no file is open).
    pub cursor: Option<CursorSummary>,
    /// The LSP work-done progress line, while a server indexes.
    pub lsp_progress: Option<String>,
}

/// What the workbench zones ask the shell for.
#[derive(Clone, Debug)]
pub enum WorkbenchEvent {
    /// A dead server's status run was clicked — restart it.
    RestartServer(LanguageId),
}

/// Paint the left and center zones into the status bar's `ui` (the bar's
/// full-width rect; the right zone's widgets paint after and measure
/// against the same rect).
pub fn show(ui: &egui::Ui, input: &WorkbenchInput) -> Vec<WorkbenchEvent> {
    let mut events = Vec::new();
    let rect = ui.max_rect();
    let painter = ui.painter_at(rect);
    let sans = || egui::FontId::new(type_scale::XS, font::FAMILY_SANS);
    let mono = || egui::FontId::new(type_scale::XS, font::FAMILY_MONO);

    // Left: the git branch + "●" + change count (UI_SPEC.md §1.2, the
    // Phase 3b subset — ahead/behind need remotes, out of Lite). Outside a
    // repository: the workspace root's name.
    let left = match input.git.as_ref() {
        Some(summary) => format!("{} ● {}", summary.branch, summary.changed_count),
        None => input.workspace_name.clone(),
    };
    let left_font = sans();
    // In a repo with changes, the dot runs hot (--warn); the rest of the
    // run stays --fg-muted. Painted as one text run in --fg-muted plus a
    // warn-colored "●" glyph over it at the branch-name offset (measured
    // with the same font).
    let dirty = input
        .git
        .as_ref()
        .is_some_and(|summary| summary.changed_count > 0);
    let dot_pos = if dirty {
        let branch = input
            .git
            .as_ref()
            .map(|summary| summary.branch.clone())
            .unwrap_or_default();
        let galley =
            painter.layout_no_wrap(format!("{branch} "), left_font.clone(), color::FG_MUTED);
        Some(egui::pos2(
            rect.left() + space::S3 + galley.size().x,
            rect.center().y,
        ))
    } else {
        None
    };
    painter.text(
        egui::pos2(rect.left() + space::S3, rect.center().y),
        egui::Align2::LEFT_CENTER,
        left.clone(),
        left_font.clone(),
        color::FG_MUTED,
    );
    if let Some(pos) = dot_pos {
        // Same font as the run it overlays, so the x offset measured above
        // lands exactly on the dot.
        painter.text(pos, egui::Align2::LEFT_CENTER, "●", left_font, color::WARN);
    }

    // Left, after the git run: per-server LSP status
    // ("rust-analyzer: ready", Phase 3a). A dead server is a click target
    // for `Effect::Lsp(Restart)` (§5.6).
    let mut status_x = painter
        .layout_no_wrap(
            format!("{left}  "),
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_MUTED,
        )
        .size()
        .x
        + rect.left()
        + space::S3;
    for (language, status) in &input.lsp_servers {
        let label = match &status {
            ServerStatus::Starting => "starting".to_owned(),
            ServerStatus::Ready => "ready".to_owned(),
            ServerStatus::Dead(_) => "dead — click to restart".to_owned(),
        };
        let text = format!("{}: {label}", language.server_name());
        let status_color = match &status {
            ServerStatus::Dead(_) => color::ERR,
            _ => color::FG_MUTED,
        };
        let font = sans();
        let galley = painter.layout_no_wrap(text.clone(), font.clone(), status_color);
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(status_x, rect.top()),
            egui::vec2(galley.size().x + space::S3, rect.height()),
        );
        painter.galley(
            egui::pos2(status_x, rect.center().y - galley.size().y / 2.0),
            galley,
            status_color,
        );
        if matches!(status, ServerStatus::Dead(_)) {
            let response = ui.interact(
                text_rect,
                egui::Id::new(("lsp_restart", language)),
                egui::Sense::click(),
            );
            if response.clicked() {
                events.push(WorkbenchEvent::RestartServer(*language));
            }
        }
        status_x = text_rect.right() + space::S3;
    }

    // Center: cursor line:col + language of the active tab, and the LSP
    // work-done progress line while a server indexes
    // ("rust-analyzer: Indexing 42%", Phase 3a).
    let mut center = input
        .cursor
        .map(|cursor| {
            format!(
                "Ln {}, Col {}   {}",
                cursor.line, cursor.col, cursor.language
            )
        })
        .unwrap_or_default();
    if let Some(progress) = input.lsp_progress.clone() {
        if !center.is_empty() {
            center.push_str("   ");
        }
        center.push_str(&progress);
    }
    if !center.is_empty() {
        let center_font = mono();
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            center,
            center_font,
            color::FG_MUTED,
        );
    }

    events
}

/// The display name of a highlighting language.
pub fn language_name(language: Option<Language>) -> &'static str {
    match language {
        None => "Plain Text",
        Some(Language::Rust) => "Rust",
        Some(Language::TypeScript) => "TypeScript",
        Some(Language::Tsx) => "TSX",
        Some(Language::JavaScript) => "JavaScript",
        Some(Language::Python) => "Python",
        Some(Language::Json) => "JSON",
        Some(Language::Toml) => "TOML",
        Some(Language::Markdown) => "Markdown",
        Some(Language::Html) => "HTML",
        Some(Language::Css) => "CSS",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        }
    }

    fn input() -> WorkbenchInput {
        WorkbenchInput {
            git: Some(GitSummary {
                branch: "main".into(),
                changed_count: 3,
            }),
            workspace_name: "repo".into(),
            lsp_servers: vec![(LanguageId::Rust, ServerStatus::Dead("exit 1".to_owned()))],
            cursor: Some(CursorSummary {
                line: 12,
                col: 4,
                language: "Rust",
            }),
            lsp_progress: None,
        }
    }

    /// One headless frame with its texture deltas handled.
    fn run(ctx: &egui::Context, raw: egui::RawInput, f: impl FnMut(&mut egui::Ui)) {
        let mut output = ctx.run_ui(raw, f);
        output.textures_delta.clear();
    }

    /// The zones paint headlessly with a dirty repo + dead server (the
    /// shell's smoke test previously covered this inline).
    #[test]
    fn zones_paint_headlessly() {
        let ctx = egui::Context::default();
        run(&ctx, screen(), |ui| {
            let events = show(ui, &input());
            assert!(events.is_empty(), "no click, no events");
        });
    }

    /// A dead server's run is the restart click target: the four-frame
    /// click pattern on its measured rect emits `RestartServer`.
    #[test]
    fn clicking_a_dead_server_run_emits_restart() {
        let ctx = egui::Context::default();
        // The dead-server run starts after the git run: 12px inset + the
        // "main ● 3  " run. Sweep the zone right of it; the first
        // successful click must carry the language id.
        let mut got: Option<LanguageId> = None;
        'probe: for dx in (60..=340).step_by(8) {
            for dy in [4.0, 8.0, 12.0] {
                let pos = egui::pos2(dx as f32, dy);
                let frames = [
                    vec![egui::Event::PointerMoved(pos)],
                    vec![egui::Event::PointerMoved(pos)],
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ],
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Default::default(),
                        },
                    ],
                ];
                for events in frames {
                    let mut raw = screen();
                    raw.events = events;
                    run(&ctx, raw, |ui| {
                        for event in show(ui, &input()) {
                            got = match event {
                                WorkbenchEvent::RestartServer(language) => Some(language),
                            };
                        }
                    });
                }
                if got.is_some() {
                    break 'probe;
                }
            }
        }
        assert_eq!(
            got,
            Some(LanguageId::Rust),
            "the restart event carries the language"
        );
    }

    #[test]
    fn language_names_are_stable() {
        assert_eq!(language_name(None), "Plain Text");
        assert_eq!(language_name(Some(Language::Rust)), "Rust");
        assert_eq!(language_name(Some(Language::Markdown)), "Markdown");
    }
}
