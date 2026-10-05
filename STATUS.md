# Status — 2026-10-05 (Phase 3 complete)

> Restart point. Read this, then AGENT_ORCHESTRATION.md §10. RESUME_PROMPT.md
> at the repo root is the copy-paste prompt for a fresh orchestrator session.

## Last completed

**Phase 3 (LSP + Git + Search) — all three subagents merged AND
orchestrator-wired.** Full gate green on main (`75e18f5`): build, 355
tests, fmt, `clippy --workspace --all-targets -- -D warnings`, cargo
deny. Main is pushed through `75e18f5`.

## Phase 3 record

| Item | State | Notes |
|---|---|---|
| 3b selora-git + panel/git.rs + tree badges | merged `c270fc2`, wired `4fc574b` | GitRepo open/status/stage/unstage/commit/diff_workdir; region ④ bottom panel; status-bar branch + warn dirty dot |
| 3c selora-fs search + panel/search.rs | merged `e2136f0`, wired `d6c92b1` | rg-if-on-PATH else fallback walker; Ctrl+J panel, Ctrl+Shift+F search; result click opens file at line |
| 3a selora-lsp + lsp_overlay.rs | merged `590b76b`/`0d915f5`, wired `75e18f5` | client actor + manager (debounced didChange, crash-restart, doc replay), registry, UTF-16 position translation; wiring: per-path didOpen guard + shape-tracked didChange in the paint loop, didSave, didClose (last tab close + before re-root), F12 open-then-jump, Ctrl+Space popup (owns keyboard: arrows/Enter/Tab/Esc), Ctrl+K Ctrl+I chord (500ms window), diagnostics → editor overlay, status-bar server status + progress + dead-server restart click. Tests: apply_lsp reducer coverage + headless shell document-sync stream (Open→Change→Save→Close) |

Wiring-test pattern (reusable in Phase 4/5): with settings never
"arrived", the shell keeps `lsp_effect_rx` and a test can observe the
dispatcher's forwarded `LspEffect` stream after `yield_now().await` —
see `shell::tests::lsp_document_sync_flows_open_change_save_close_from_the_shell`.

## Next

**Phase 4 — Selora AI** (six parallel subagents 4a–4f, briefs in
ROADMAP.md Phase 4; ownership table in AGENT_ORCHESTRATION.md §10):
4a settings Selora section, 4b model picker, 4c chat sidebar,
4d ghost text, 4e cost meter, 4f Selora status. All consume
`selora-ai` from Phase 1. `Effect`/`AppMessage` additions go through
the orchestrator (same shared-file rule as before).

Before spawning: the §11 owner smoke tests for Phase 2 and Phase 3
are still pending (below) — recommend the owner runs them first; they
are not a blocker for starting Phase 4 subagents that only touch
`selora-ui`/`selora-ai`, but 4c (chat sidebar) wiring will want a
verified Phase 2 chat stream.

### Owner smoke tests pending (§11, after Phase 2 / Phase 3)

- Phase 2: open folder, edit, fuzzy find, syntax highlight, settings
  persist, split panes.
- Phase 3: LSP hover + go-to-def on a real Rust file (needs
  `rustup component add rust-analyzer` — NOT yet verified on PATH),
  git stage/commit from the panel, search in files.

## Blockers

None. Owner smoke tests pending (not blocking Phase 4 spawn).

## Environment (machine, not in any repo doc)

- Toolchain: rustup + stable 1.99.0 MSVC + VS 2022 Build Tools. cargo at
  C:\Users\Toxic\.cargo\bin — bash needs the PATH export.
- Remote `origin` = github.com/ToxicAngelX/Selora-IDE; main pushed
  through `75e18f5`.
- Known flake: `cargo test --workspace` occasionally prints "could not
  compile selora-… (lib test)" from MSVC/Defender file-locking during
  parallel compilation — rerun confirms exit 0.
- `rg` is NOT on the Windows PATH for the app (the bash `rg` is a
  Claude Code shim) — search uses the built-in fallback scanner. Works,
  just slower on big workspaces.
- `rust-analyzer`: NOT verified on PATH; Phase 3 LSP acceptance needs
  it (`rustup component add rust-analyzer`).
- The sk-gw- test key passed through chat in session 1 — rotate at the
  gateway when convenient.
- Gate note: the earlier "clippy -D warnings green" on `d6c92b1` had
  not used `--all-targets`; 7 test-target warnings were fixed in
  `75e18f5`. Future gates: `cargo clippy --workspace --all-targets --
  -D warnings`.
