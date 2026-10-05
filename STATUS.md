# Status — 2026-10-05 (mid-Phase 3)

> Restart point. Read this, then AGENT_ORCHESTRATION.md §10. RESUME_PROMPT.md
> at the repo root is the copy-paste prompt for a fresh orchestrator session.

## Where we are

**Phase 3 (LSP + Git + Search) is 2/3 merged.** 3b (git) and 3c (search)
are merged AND orchestrator-wired into `selora-app`; the full gate is
green on main (`d6c92b1`): build, 286 tests, fmt, clippy -D warnings,
cargo deny. **3a (LSP) is still in flight** in a subagent worktree —
nothing of it has landed; when its notification arrives, follow the
merge/wiring pattern below.

## Phase 3 progress record

| Item | State | Notes |
|---|---|---|
| 3b selora-git + panel/git.rs + tree badges | merged `c270fc2`, wired `4fc574b` | GitRepo open/status/stage/unstage/commit/diff_workdir; 12 temp-repo tests; region ④ bottom panel; status-bar branch + warn dirty dot; GitStatus re-snapshot on tree update + save |
| 3c selora-fs search + panel/search.rs | merged `e2136f0`, wired `d6c92b1` | rg-if-on-PATH else ignore-walker fallback (20 tests); Ctrl+J toggles region ④, Ctrl+Shift+F opens search; result click opens file at line via new `Editor::goto_line` |
| 3a selora-lsp + lsp_overlay.rs | **in flight** | subagent in worktree, brief = verbatim ROADMAP 3a + addendum (no selora-app edits; report proposes Effect/AppMessage diffs) |

Orchestrator prep for Phase 3 (commit `0468f2a`): workspace deps
lsp-types 0.97 + git2 0.21 (vendored) pre-added, module stubs
(panel/{git,search}.rs, lsp_overlay.rs) pre-registered, so 3a/3b/3c had
strictly disjoint ownership. `.claude/worktrees/` is gitignored.

## When 3a lands (the exact plan)

1. Review its branch against the ROADMAP 3a brief + ARCHITECTURE §5
   (actor per server, framing, registry, crash recovery; ≥3 tests:
   framing, request-id routing, crash recovery).
2. `git merge <branch> --no-ff`; gate (build/test/fmt/clippy/deny).
   Beware: 3a may add additive hooks in selora-editor files — main's
   editor.rs has since gained `goto_line`/`pending_reveal` (from 3c
   wiring); resolve any conflict keeping both.
3. Apply its proposed Effect/AppMessage/AppState additions (pattern is
   in `app_state.rs`/`wiring.rs`/`shell.rs` — same as the 3b/3c wiring
   commits). Wire: didOpen/didChange/didSave/didClose lifecycle in the
   drain/paint loop, F12 definition, Ctrl+K Ctrl+I hover, Ctrl+Space
   completion, diagnostics into the editor overlay, LSP status in the
   status-bar center. UI hooks that need `state.editors` reuse
   `AppState::goto_line` for jumps.
4. Gate again, commit, update STATUS.md, close Phase 3.
5. Then: owner smoke test (§11 after Phase 3: LSP hover + go-to-def,
   git commit, search), then Phase 4.

## Environment (machine, not in any repo doc)

- Toolchain: rustup + stable 1.99.0 MSVC + VS 2022 Build Tools. cargo at
  C:\Users\Toxic\.cargo\bin — bash needs the PATH export.
- A git remote `origin` now exists (github.com/ToxicAngelX/Selora-IDE);
  main is pushed through `d6c92b1`'s predecessor. Push after gates.
- Known flake: `cargo test --workspace` occasionally prints "could not
  compile selora-… (lib test)" from MSVC/Defender file-locking during
  parallel compilation — rerun confirms exit 0.
- `rg` is NOT on the Windows PATH for the app (the bash `rg` is a
  Claude Code shim) — search uses the built-in fallback scanner. Works,
  just slower on big workspaces.
- `rust-analyzer`: NOT verified on PATH — 3a's report will say; Phase 3
  LSP acceptance needs it (rustup component add rust-analyzer).
- The sk-gw- test key passed through chat in session 1 — rotate at the
  gateway when convenient.

## Blockers

None. 3a pending; owner smoke tests for Phase 2 and Phase 3 pending.
