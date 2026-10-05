# Status — 2026-10-05 (mid-Phase 4)

> Restart point. Read this, then AGENT_ORCHESTRATION.md §10. RESUME_PROMPT.md
> at the repo root is the copy-paste prompt for a fresh orchestrator session.

## Where we are

**Phase 3 complete** (3a/3b/3c merged AND wired; gate green on `75e18f5`,
pushed). **Phase 4 (Selora AI) in flight**: 4e and 4f are merged AND
wired (through `d8dcd15`); 4a, 4b, 4c, 4d are still running in subagent
worktrees. Merge each on its notification, then wire (checklist below).

## IMPORTANT for merging the remaining branches

All six subagent worktrees were cut from `origin/main` at `0a09a99` —
BEFORE prep commit `4f334d4` (they lack the stubs; each recreated its
own registrations). Merging against current main yields add/add
conflicts; the resolution pattern that worked for 4e/4f:
- `git checkout --theirs <their-owned file>` (their implementation
  replaces the stub),
- `git checkout --ours` for registration files (`mod.rs`, `lib.rs`) where
  main's version is the superset,
- then full gate + `git commit --no-edit` on the `--no-ff` merge.

## Phase 4 ownership (AGENT_ORCHESTRATION.md §10)

| Agent | Owns | State |
|---|---|---|
| 4a | selora-ui/src/dialogs/settings/ (selora_section.rs — Connect flow, Disconnect) | in flight |
| 4b | selora-ui/src/picker/ (model_picker.rs) | in flight |
| 4c | selora-ui/src/sidebar/chat.rs | in flight |
| 4d | selora-editor/src/ghost_text.rs (+ minimal editor.rs integration) | in flight |
| 4e | selora-ui/src/status_bar/cost_meter.rs | merged `df492e3`, wired `d8dcd15` |
| 4f | selora-ui/src/status_bar/selora_status.rs | merged `ba43f52`, wired `d8dcd15` |

Shared-file rule in force: subagents propose Effect/AppMessage/AppState
and persistence diffs in their reports; the orchestrator applies them.
All six were told: no selora-app edits, no Cargo.toml edits, gate their
worktree (build/fmt/clippy --all-targets -D warnings/test) before commit.

## Phase 4 prep (`4f334d4`), already on main

- workspace + selora-settings: `keyring = 3` dep;
  `selora_settings::keyring::{api_key_get,api_key_set,api_key_delete}`
  (Windows Credential Manager; get-missing = Ok(None); delete idempotent).
- selora-ai: `ghost_complete(model, prompt)` (non-streaming completion,
  blank → Ok(None)) and `status()` (HEAD ping →
  Connected{model:None,latency}/Disconnected) — fills ARCHITECTURE §10.6
  gaps. NOTE: ghost_complete gained a model param vs the §10.6 sketch;
  the shell passes settings.selora.active_model.
- selora-ui stubs pre-registered: dialogs/settings split into
  settings/{mod.rs,selora_section.rs} (pane_selora moved), picker/,
  status_bar/{cost_meter.rs, selora_status.rs}, sidebar/chat.rs.
- selora-editor: ghost_text.rs stub.

## Phase 4 wiring checklist (orchestrator, after merges)

DONE (in `d8dcd15`):
- Cost meter: AppState.cost_meter + ChatDone feed + day sync + the
  status-bar right-zone paint (left of the connection run).
- Status bar right: 4f's widget + Effect::SeloraPing + AppMessage::
  SeloraStatus + the stage-2 30s ping loop (first ping when settings
  arrive); Connected = key present + gateway answered (no-key pings
  reply Disconnected without network); OpenConnect click → open
  settings. NOTE: the dialog currently opens on its default section —
  preselecting the Selora section waits for 4a's dialog API.

REMAINING:
1. Keyring at boot: Effect::SeloraConnect already stores the key
   in-session; swap storage to selora_settings::keyring (set on Connect,
   get at boot, delete on Disconnect). 4a's SettingsEvent additions tell
   the exact flow (validate via list_models → keyring set → status).
2. Chat: replace AppState's raw `chat_stream`/`chat_receipt` fields with
   4c's ChatPanel state; drive it from ChatDelta/ChatDone/ChatError;
   Ctrl+L toggle (settings.keybinds.toggle_chat); persist last 20 turns
   + visibility in persistence.rs (WorkspaceState).
3. Model picker: fetch on boot (Effect::SeloraListModels exists), hand
   4b's picker the list, write settings.selora.active_model on Select.
4. Ghost text: frame stage 2 gets the 350ms idle debounce →
   Effect::SeloraGhostText(path, prefix) → wiring calls
   client.ghost_complete(model, prompt) → AppMessage::GhostTextReady(path,
   Option<String>) → Editor::set_ghost. 4d's report has the exact seams.
5. Cost meter: on ChatDone, feed 4e's state; paint in the status bar
   right zone.
6. Status bar right: replace the hardcoded "○ selora disconnected"
   (shell.rs status_bar fn) with 4f's widget; 30s ping loop (frame stage 2
   clock) calling client.status(); click → open settings on Selora
   section. Mid-chat disconnect policy: in-flight complete, new chats
   toast.
7. Gate, commit, update STATUS.md, owner smoke test (ROADMAP Phase 4
   acceptance list), then Phase 5.

## Next

Phase 4 merges + wiring (above). Then Phase 5 (Agent Mode).

### Owner smoke tests still pending (§11)

- Phase 2: open folder, edit, fuzzy find, syntax highlight, settings
  persist, split panes.
- Phase 3: LSP hover + go-to-def (needs `rustup component add
  rust-analyzer` — NOT yet verified on PATH), git commit from panel,
  search in files.
- Phase 4 (after wiring): the ROADMAP Phase 4 acceptance list (connect
  real key, stream chat, ghost text accept, nonzero cost meter, model
  switch).

## Blockers

None. Owner smoke tests pending (not blocking Phase 4).

## Environment (machine, not in any repo doc)

- Toolchain: rustup + stable 1.99.0 MSVC + VS 2022 Build Tools. cargo at
  C:\Users\Toxic\.cargo\bin — bash needs the PATH export.
- Remote `origin` = github.com/ToxicAngelX/Selora-IDE; main pushed
  through `4f334d4`.
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
- Gate note: clippy gate must run `--all-targets` (`cargo clippy
  --workspace --all-targets -- -D warnings`) — the 3b/3c-era gate missed
  test-target warnings; fixed in `75e18f5`.
- Subagent worktrees live under .claude/worktrees/ (gitignored); each
  agent commits to its own branch (4a/4b/4c/4d/4e/4f reports name them).
  Check `git worktree list` + `git branch -a` when resuming mid-phase;
