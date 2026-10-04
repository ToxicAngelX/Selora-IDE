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
Phase 1 — Risk spike: 1a and 1b MERGED and gated (commits 3c2d5f7, 6d5cbb2).
1c (spike app) launching next. Then: full gate + human smoke test.
Blockers: none. Owner action pending: a real `sk-gw-…` key for the Phase 1
acceptance test (the human-run SSE verification).

## Phase 1 merge notes (for Phase 2 briefs)
- selora-editor ships `Editor::new(Arc<RwLock<Buffer>>)`, `set_text`, `text`,
  `cursor()`, added via `ui.add(&mut editor)` (Widget on `&mut Editor`).
  Editor font is 14px (ROADMAP 1a brief override of the 13px token); Phase 2
  settings make font_size user-driven.
- KNOWN DEBT (Phase 2 must resolve): selora-editor mirrors theme tokens in
  `crates/selora-editor/src/theme.rs` because PLAN §5's graph (selora-ui →
  selora-editor) makes importing selora-ui a cargo cycle. Options: dev-dep
  equality test, or hoist tokens to a leaf both import. Mirror is value-exact
  and documented.
- selora-ai: `SeloraClient::new(base_url, api_key)`, `list_models`,
  `stream_chat(req, tx)` → ChatReceipt; SSE parser is `sse::parse_sse_event`
  (12+ mandatory RISKS §3 fixtures covered). `selora-settings` dep was removed
  from selora-ai for the spike (constructor-arg config); Phase 4 re-adds it
  for the model-price cache. Mid-stream failures are NOT retried (avoids
  duplicated deltas) — only request establishment retries (ARCHITECTURE §6.5).
- Cost: `compute_cost_usd(model, prompt, completion)` is exported for Phase 4.
