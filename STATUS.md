# Status — 2026-10-04

## Last completed
Phase 0 — Foundation
Branch: main (commit 7c0f5f8 + orchestrator follow-up)
Merged: 2026-10-04
Acceptance: all green (build / test / fmt / clippy verified by orchestrator, not just reported)
Notes:
- Workspace: all 9 crates wired per PLAN.md §5 dependency graph.
- `crates/selora-ui/src/theme.rs` = DESIGN_TOKENS.md §11 contract, verified value-by-value.
- egui is **0.36.2**: `eframe::App::ui(&mut self, ui, frame)` replaces `update`;
  panels are `egui::Panel::top/left/right(...)`; theming via `set_theme` /
  `set_visuals_of` / `set_style_of`. ARCHITECTURE.md's 0.29-era snippets are
  design contracts, not copy-paste code. Phase 1+ briefs must assume 0.36 idioms.
- Fonts: 5 real OFL TTFs bundled (Space Grotesk R/M/SB, JetBrains Mono R/M) with
  license texts. Icons: `selora-ide-mono.svg` only; .ico/.png generation documented
  in `assets/icons/NOTE.md` (needed for the Phase 5 installer).
- Toolchain installed fresh this session: rustup + stable 1.99.0 MSVC + VS Build
  Tools (C++ workload). cargo at `C:\Users\Toxic\.cargo\bin` — export PATH in bash.
- cargo-deny.toml + CI deny step added by orchestrator (RELEASE.md §7.1 gap the
  Phase 0 brief missed). Local `cargo deny check licenses` still to be run once
  cargo-deny finishes installing (CI will enforce it regardless).
- `cargo run -p selora-app` smoke: boots, stays alive, dark canvas, title strip
  with brand mark. (Window verified by subagent smoke test; visual sign-off
  belongs to the owner.)

## Next
Phase 1 — Risk spike (1a editor pane, 1b selora-ai client, 1c spike app)
Briefs: 1c brief is verbatim in ROADMAP.md §Phase 1; 1a/1b briefs derived from
the Phase 1 task list + ARCHITECTURE.md §9/§10 (selora-editor, selora-ai).
Blockers: none. Owner action pending: a real `sk-gw-…` key for the Phase 1
acceptance test (the human-run SSE verification).
