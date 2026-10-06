# Status — 2026-10-06 (Phase 4 complete; smoke-test fixes landed; Phase 5 prep landed)

> Restart point. Read this, then AGENT_ORCHESTRATION.md §10. RESUME_PROMPT.md
> at the repo root is the copy-paste prompt for a fresh orchestrator session.

## Owner decisions (2026-10-06)

- **Phase 4 smoke test deferred** by owner decision: Phase 5 spawn prep
  started instead. The six-step list below stays the standing gate
  before "DONE" can be claimed on Phase 4 — run it before Phase 5's
  final acceptance (it exercises the same chat path the agent loop
  builds on).

## Phase 5 merge log

| Subagent | Branch/commit | State | Gate |
|---|---|---|---|
| 5e web tools | `phase5e-web-tools` @ `a44b1f2` | **merged** (ff) | 20 targets, clippy, fmt — green |
| 5a tool plumbing | `phase5a-tool-plumbing` @ `3493677` | **merged** (`51de58b`) | 20 targets, clippy, fmt — green |
| 5b tool router | `phase5b-tool-router` @ `461cf98` | **merged** (ff) | 20 targets, clippy, fmt — green |
| 5c agent loop | `phase5c-agent-loop` @ `ec746e4` | **merged** (ff) | 20 targets, clippy, fmt — green |

5c review notes (orchestrator, decision core read in full): the loop is
trait-injected (`ChatStream`/`ToolRunner` — no HTTP, no execution inside
turn.rs; production backends are thin adapters over 5a/5b); Deny feeds
`router::denied_result` back to the model; ApproveAllThisTurn is a
turn-local flag with a no-carryover test; Stop is honored at every phase
(gated select wrapper), abort kills the sticky BashAbortHandle + 500ms
grace before dropping the future; caps (25/600s/$1/context−20%) each
abort with a partial answer + `TurnEnd::toast()`. 31 tests, including a
Stop mid-tool killing a real `ping -n 60` child.

**5c deviations accepted:**
- `AgentEvent`/`TurnDecision`/`TurnCaps`/`TurnEnd` vocabulary in turn.rs
  (the shell reduces AgentEvent onto the §17.6 AppMessages).
- Context reserve fixed at 0.20 (no settings field — add
  `agent.context_reserve` only if the owner wants it tunable).
- tokio `test-util` + a dev-dep on selora-settings in selora-ai
  (test-only; the settings→TurnCaps mapping is proven in tests).

**Wiring owed when 5d lands (orchestrator's final Phase 5 pass):**
- `Effect::AgentSend` → `spawn_agent_turn` with `TurnCaps` from
  settings + the active model's context window; `Effect::AgentDecision`
  → the turn's decision channel; `AgentEvent` → AppMessage reduction +
  the per-event notify() (the repaint seam).
- `agent.brave_search_api_key` → `AgentToolSettings` (5b's
  skip_serializing field).
- §17.7 open-buffer routing: approved edits to open editors go to the
  in-memory buffer (the card is the save moment).
- 5d's cards into `sidebar/chat/mod.rs`.

5b review notes (orchestrator, guard core read line-by-line — §10's
heaviest-review rule): path rooting double-checks (lexical `..` fold +
canonicalized-ancestor symlink/junction containment, case-insensitive
on Windows); env is a fixed ALLOWLIST (`env_clear` + explicit set — the
Selora key is structurally absent, verified in a real child); bash
allowlist matches on word boundaries (`cargo testx` ≠ `cargo test`);
plan/apply separation — `execute_tool` never writes, `execute_approved`
re-parses and re-validates against current file contents (stale-approval
replay tested); bash children: `-NoProfile -NonInteractive`,
`CREATE_NO_WINDOW`, stdin null, capped pipes, `start_kill` + bounded
reap on Stop/timeout (tokio's async `Child::kill` never-fires bug found
and fixed). 33 tests.

**5b deviations accepted:**
- `execute_tool(ctx: &ToolContext, call)` reworked; `ToolContext { root,
  settings: AgentToolSettings, abort: BashAbortHandle }` in router.rs.
- selora-fs added as a selora-ai dependency (§17.6 composition).
- `AgentToolSettings` mirrors the §17.5 settings slice + a
  `skip_serializing` brave_api_key (the app maps its settings in — the
  router never reads settings/keyring).

**5b open items for the 5c merge + final wiring:**
- §17.7 open-buffer routing is the app seam: `apply_write`/`apply_edit`
  are the on-disk half; an approved edit to an open editor must also go
  to the in-memory buffer (undoable, the card is the save moment).
- `execute_approved("bash")` runs unconditionally — 5c must only hand
  it calls that produced a card (an allowlisted call re-passed would
  run twice; documented in router.rs).
- ENV_ALLOWLIST may need app-level additions (CARGO_HOME, proxies) —
  one-line changes in tools/bash.rs.
- 5c loop caps (25/10min/$1) read from settings `agent.*` —
  `AgentSettings` already has them (max_iterations/max_turn_secs/
  max_cost_usd).

5a review notes (orchestrator): OpenAI wire shapes are custom serde in
chat_types.rs (tools `{"type":"function","function":{…}}`, calls with
tolerant `arguments` folding — string/object/null); stream parsing is
defensive (missing index → slot 0, empty fragments skipped); the
scripted 3-turn mock test asserts exact request bodies per turn.
Deliverables verified: 61 tests in selora-ai.

**5a deviations accepted:**
- `finish_reason` is NOT surfaced as data — the loop distinguishes
  tool_calls-vs-stop via `accumulator.finish()` (non-empty ⇔ tools);
  5c was told (see below).
- `ToolCallAccumulator` re-exported from lib.rs (one line, additive).
- New public `SeloraClient::complete_chat` (non-streaming half).


5e review notes (orchestrator, guard read line-by-line): the §17.2 SSRF
guard covers every listed range plus 0/8, CGNAT, TEST-NETs,
benchmarking, IPv6 ULA/link-local/multicast/documentation, and
IPv4-mapped IPv6; `resolve_and_check` checks EVERY resolved address;
redirects are manual (policy: none) so each hop re-enters the guard;
fetch output capped 256 KiB (raw buffer 1 MiB), search key never in
results/logs. Known accepted limitation: DNS is resolved at
guard-time and again at connect-time, so a fast DNS rebind can race the
check (check-then-fetch, no IP pinning) — read-only tool, output-capped,
approval-gated writes; fine for Lite.

**5e wiring owed by later merges:**
- 5b's router: call `web_search(query, brave_api_key)` (two-arg) and
  `web_fetch(url)`; route URLs through `resolve_and_check` (not just
  the pure `is_url_allowed`); stamp `result.id = call.id` (web results
  carry an empty id).
- Orchestrator (post-merge): add `agent.brave_search_api_key` to
  selora-settings and pass it through; empty/absent → DuckDuckGo.

## Phase 5 prep (landed this session, gate green)

The interface skeletons all five subagents build against
(AGENT_ORCHESTRATION.md §10 file ownership; the Phase 4 lesson — push
prep BEFORE spawning so worktrees branch from it):

| Piece | File | Owner |
|---|---|---|
| `ToolSpec`/`ToolCall`/`ToolResult` + round-trip tests | `crates/selora-ai/src/chat_types.rs` (new) | 5a |
| `ChatRequest.tools`, `ChatMessage.tool_calls/tool_call_id` + `Role::Tool`, `ChatEvent::ToolCallDelta` (additive; old JSON unchanged — tested) | `crates/selora-ai/src/types.rs` | orchestrator (applied) |
| `agent` module: `ApprovalId`/`AgentDecision`/`ApprovalCard`/`AgentSummary`; `router.rs` (`ToolOutcome`, `tool_manifest`, `execute_tool` stubs); `turn.rs` (`AgentTurnState`/`AgentTurn`); `tools/web.rs` stubs | `crates/selora-ai/src/agent/` (new) | 5b/5c/5e |
| `Effect::AgentSend`/`AgentDecision` + `AppMessage::ToolCallStarted`/`ToolResult`/`ApprovalNeeded`/`AgentTurnDone` (§17.6 shapes, Uuid-keyed like `SeloraChat`; apply()/dispatcher arms are no-op stubs) | `crates/selora-app/src/app_state.rs`, `wiring.rs` | orchestrator (shared file) |
| `agent.*` settings (§17.3 caps + §17.5 shell/allowlist, serde-defaulted; pre-Phase-5 files load — tested). **`Settings.agent` is `Box<AgentSettings>`** — the unboxed section pushed `Settings` past clippy's large-enum-variant threshold; field access auto-derefs, JSON identical | `crates/selora-settings/src/settings.rs` | orchestrator |
| chat panel moved `sidebar/chat.rs` → `sidebar/chat/mod.rs`; `agent_cards.rs` stub | `crates/selora-ui/src/sidebar/chat/` | 5d |

Deviations from ARCHITECTURE.md §17.6 (deliberate, both noted in code):
worker messages key by the per-request `Uuid` + `chat_ids` map (the
existing `SeloraChat` pattern), not a bare `ChatId`; `AppMessage::ToolResult`
carries the call id inside `ToolResult` instead of repeating it in the tuple.

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

**Phase 5 subagents 5a–5e are SPAWNED and running** (parallel
worktree-isolated branches from the pushed prep `57ffcab`). The
orchestrator's duties while they run:
- Merge in arrival order, gate after each: build + test + fmt + clippy
  (the shared-file rule — apply Effect/AppMessage/chat-panel changes
  centrally, never let two branches collide).
- Line-by-line review of every 5b guard (path rooting, SSRF, env
  stripping, output caps).
- 5d's integration into `sidebar/chat/mod.rs` is the orchestrator's
  (5d delivers standalone widgets + a call surface).
- Deviations any agent reports (settings fields to wire, shape changes)
  land as orchestrator commits.
- If an agent dies on a transient error, resume it via SendMessage
  before re-spawning (the Phase 4 lesson: cheaper than restarting).

After all five merge: the Phase 5 acceptance list in ROADMAP.md (real
key, real workspace), then the deferred Phase 4 smoke test list below,
then Phase 6.

**Owner smoke test of Phase 4** (ROADMAP.md Phase 4 acceptance list) —
the one gate that needs a human and a real sk-gw- key. The owner started
it 2026-10-06 and found bugs; the fixes below are merged — **re-run from
step 1 on a fresh build** (`cargo run -p selora-app`):

1. `cargo run -p selora-app` — status bar right shows `○ selora
   disconnected · connect`
2. Click it → Settings opens ON the Selora section (Connect form) → paste
   key → Connect (or press Enter in the field) → status flips to
   `● selora connected` within 2s AND STAYS connected (the 30s ping no
   longer flips it back); reload the app — still connected (keyring
   restore)
3. Ctrl+L → chat sidebar → "list 3 prime numbers" → Enter → streams in;
   code blocks have hover Copy; ⋯ menu clear/copy-transcript
4. Model picker in the chat header: filter, select, refresh; next chat
   uses the new model (settings.selora.active_model persisted)
5. Open a Rust file, stop typing 500ms → ghost text in --fg-faint;
   Tab accepts; cost meter reads nonzero
6. Reload — last 20 chat turns + sidebar visibility restored

## Smoke-test bugfix round 1 (2026-10-06, all tested + gate green)

Seven fixes, all found by the owner smoke-testing the connect flow:

| Bug | Fix | Where |
|---|---|---|
| Manual Connect never set the dispatcher's session key — 30s later the ping flipped the status back to Disconnected; chat/ghost said "no API key" | `Effect::SeloraConnect` adopts the key immediately (AutoConnect idiom); regression test against a wiremock 401 gateway (never writes the real keyring) | `wiring.rs` |
| Editing Base URL in Settings never reached the dispatcher — Connect/chat/ping kept the boot URL until restart | new `Effect::SettingsSynced { base_url }` (no disk write, no reply; empty → DEFAULT_BASE_URL) sent when the shell adopts dialog settings | `app_state.rs`, `wiring.rs`, `shell.rs` |
| Esc while typing in a field closed the dialog and threw away the pasted key | first Esc leaves the focused field (dialog tracks `field_focused` per frame — egui's begin_pass surrenders focus on Esc before `show` can see it), second Esc closes | `dialogs/settings/mod.rs` |
| Enter in the masked key field did nothing | Enter submits Connect (inert while Connecting); the input got an explicit id for focus-driving tests | `dialogs/settings/selora_section.rs` |
| The "modal" was a plain egui::Window — clicks and keystrokes around it fell through to the workbench (file tree opened files, editor took typing; the dialog seemed "fully bugged") | real `egui::Modal` (0.36): input-blocking backdrop, keyboard focus confinement, backdrop-click dismisses; geometry pinned min==max (Modal sizes to content, and the body's available-size feedback loop made layout drift frame-to-frame — clicks missed buttons by 20px until pinned) | `dialogs/settings/mod.rs` |
| Status-bar "○ selora disconnected · connect" opened Settings on the EDITOR section instead of the Connect form | `SettingsDialog::open_selora_section()` + shell `open_settings_to_selora()` for `StatusEvent::OpenConnect` | `dialogs/settings/mod.rs`, `shell.rs` |
| `Effect::SeloraKeyringSet` was dead code (nothing sent it — the session-key path that never ran) | left in place (tests' seam); its doc comment is accurate now | `app_state.rs` |

Gate after the fixes: build, 20 test targets (~230 tests incl. 8 new
settings/connect tests), fmt, `clippy --workspace --all-targets --
-D warnings`, app boots and stays alive. The one cosmetic gap knowingly
left: after Connect the status line says "no model" until the picker runs
(`spawn_connect` reports `model: None`); also the Modal's backdrop scrim
has no design token (egui default black-alpha — token gap to note in a
DESIGN_TOKENS follow-up).

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
