# Status — 2026-10-05 (Phase 4 complete)

> Restart point. Read this, then AGENT_ORCHESTRATION.md §10. RESUME_PROMPT.md
> at the repo root is the copy-paste prompt for a fresh orchestrator session.

## Last completed

**Phase 4 (Selora AI) — all six subagents merged AND orchestrator-wired.**
Full gate green on main (`9017cca`): build, 457 tests, fmt,
`clippy --workspace --all-targets -- -D warnings`, cargo deny.

## Phase 4 record

| Item | State | Notes |
|---|---|---|
| 4a settings Selora section | merged `cfd1fea`, wired | Connect (validate via list_models → keyring set), Disconnect, live status line (4 states), key Debug-redacted; boot auto-connect from keyring when settings arrive; Connect/Disconnect effects |
| 4b model picker | merged `32237dc` | Standalone `picker::ModelPicker` widget (filter, refresh, offline). NOTE: the chat header currently uses 4c's inline dropdown (same event vocabulary, tested with the panel); 4b's widget is available if a second picker consumer appears |
| 4c chat sidebar | merged `b569520`, wired `9017cca` | Region ③ right panel (Ctrl+L / Chat activity / View command, opening focuses composer); Send → Effect::SeloraChat keyed by Uuid→ChatId map; ChatDelta/Done/Error route into the panel (stale/late ids dropped); SelectModel persists settings.selora.active_model; workspace.json gains chat_open + chat_turns (serde-defaulted; last 20 turns restore at boot) |
| 4d ghost text | merged `6caa5bb`, wired | Stage-2 350ms idle debounce → Effect::SeloraGhostText{path, prefix, model, request_caret} → ghost_complete → GhostTextReady → set_ghost on matching editors (stale-dropped); request-key guard prevents steady-state re-requesting; comet fade + Tab/other-key/Esc gestures + egui focus-lock for Tab (all in selora-editor, 72 tests) |
| 4e cost meter | merged `df492e3`, wired `d8dcd15` | ChatDone feeds CostMeterState (receipt cost verbatim, else compute_cost_usd of the resolved model — never guessed); `Σ today $X.XX` + per-model tooltip left of the connection run; day rollover |
| 4f connection status | merged `ba43f52`, wired `d8dcd15` | 30s ping loop (first ping when settings land, then every 30s; probe = list_models so Connected = gateway up AND key valid, never a bare HEAD); disconnected run clicks through to Settings; Connecting/Error forms |

Keyring semantics (ARCHITECTURE.md §7.2): the OS keyring is the key's
only home — Connect validates first, stores second; boot restore reads
it (validate again); Disconnect deletes. Tests never touch the real
Credential Manager (persist_key flag / throwaway service names).

## Next

**Owner smoke test of Phase 4** (ROADMAP.md Phase 4 acceptance list) —
the one gate that needs a human and a real sk-gw- key:

1. `cargo run -p selora-app` — status bar right shows `○ selora
   disconnected · connect`
2. Click it → Settings → Selora → paste key → Connect → status flips to
   `● selora connected` within 2s; reload the app — still connected
   (keyring restore)
3. Ctrl+L → chat sidebar → "list 3 prime numbers" → Enter → streams in;
   code blocks have hover Copy; ⋯ menu clear/copy-transcript
4. Model picker in the chat header: filter, select, refresh; next chat
   uses the new model (settings.selora.active_model persisted)
5. Open a Rust file, stop typing 500ms → ghost text in --fg-faint;
   Tab accepts; cost meter reads nonzero
6. Reload — last 20 chat turns + sidebar visibility restored

Then **Phase 5 (Agent Mode)** — five subagents (5a–5e), interface-first
(5a lands `ToolSpec`/`ToolCallDelta`/tool-result types first), briefs in
ROADMAP.md Phase 5, ownership in AGENT_ORCHESTRATION.md §10. The
Effect/AppMessage enums grow additively through the orchestrator
(§10 shared-file rule). Heaviest review phase: every guard
(path rooting, SSRF, env stripping, output caps) gets a line-by-line
read at merge.

Pending from earlier phases (owner, not blocking):
- Phase 2 smoke: open folder, edit, fuzzy find, syntax, settings persist.
- Phase 3 smoke: LSP hover + go-to-def (needs `rustup component add
  rust-analyzer` — NOT verified on PATH), git commit, search.

## Blockers

None. Phase 4 owner smoke test pending (above) — recommended before
Phase 5 spawn (agent mode composes Phase 4's chat stream; a Phase 4 bug
found now is much cheaper).

## Environment (machine, not in any repo doc)

- Toolchain: rustup + stable 1.99.0 MSVC + VS 2022 Build Tools. cargo at
  C:\Users\Toxic\.cargo\bin — bash needs the PATH export.
- Remote `origin` = github.com/ToxicAngelX/Selora-IDE; main pushed
  through the Phase 4 commits.
- Known flake: `cargo test --workspace` occasionally prints "could not
  compile selora-… (lib test)" from MSVC/Defender file-locking during
  parallel compilation — rerun confirms exit 0.
- `rg` is NOT on the Windows PATH for the app (the bash `rg` is a
  Claude Code shim) — search uses the built-in fallback scanner.
- `rust-analyzer`: NOT verified on PATH; Phase 3 smoke needs it.
- The sk-gw- test key passed through chat in session 1 — rotate at the
  gateway when convenient.
- Gate: `cargo clippy --workspace --all-targets -- -D warnings` (the
  --all-targets matters; the 3b/3c-era gate missed test-target
  warnings, fixed in `75e18f5`).
- Subagent worktrees: spawned with the Agent tool's worktree isolation
  under .claude/worktrees/ (gitignored). NOTE for future phases: the
  worktree base was `origin/main` at spawn time — push the prep commit
  BEFORE spawning so agents branch from it (Phase 4's agents predated
  the prep and every merge needed conflict resolution). Two agents
  (4b, 4c) died on transient server errors mid-task and were resumed
  via SendMessage — cheaper than re-spawning.
