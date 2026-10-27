# Kimi K3 brief — Phase 5d + final Phase 5 wiring

Run this BEFORE KIMI_UI_PROMPT.md (Phase 6h, the UI-effects brief). 6h hard-requires this phase merged.
Order for the whole chain: this brief → the owner's real-key smoke test (see bottom) → 6h.

Copy everything below the line into the subagent.

---

You are Phase 5d of the Selora IDE build (Agent Mode cards UI + the final Phase 5 wiring).
5a (tool types), 5b (tool router + guards), 5c (agent loop), 5e (web tools) are merged and green on main. You are the last Phase 5 subagent — and this brief also includes the orchestrator's final wiring pass, so Agent Mode works end-to-end when you are done. One agent, no parallel worktrees, no shared-file conflicts.

First action: verify the tree is green before touching anything:

    cargo build --workspace && cargo test --workspace

If the gate is RED: stop and diagnose. Do not build on a broken tree.

READ FIRST, in full, before writing any code:
- ARCHITECTURE.md §17 — all of it; §17.4 (approval protocol), §17.6 (plumbing), §17.7 (open-buffer interaction) are your spec
- UI_SPEC.md §4 and §4.2 — the card wireframes; render exactly this
- DESIGN_TOKENS.md §1, §7, §12 — ink ramp, motion glyphs, the no-decoration rule
- ROADMAP.md Phase 5 — the acceptance list
- crates/selora-ui/src/sidebar/chat/mod.rs — the chat panel you are extending (end to end)
- crates/selora-ai/src/agent/turn.rs — the loop you are driving
- crates/selora-ai/src/agent/router.rs — ToolContext, AgentToolSettings, execute_tool / execute_approved
- crates/selora-app/src/app_state.rs and wiring.rs — the skeletons you are bringing alive

THE DEPENDENCIES YOU CONSUME (all merged, all tested):
- selora-ai::agent — `spawn_agent_turn(handle, TurnInput, chat, tools, events) -> (decision_tx, JoinHandle<AgentTurnReport>)`; production adapters: `ClientChatStream::new(SeloraClient)`, `RouterToolRunner::new(ToolContext::new(root, AgentToolSettings))`. `TurnInput::new(model, conversation)` arms the fixed manifest + spec-default caps — build TurnCaps yourself from settings + the model's context window.
- The loop's event stream: `AgentEvent::State(AgentTurnState)` / `Delta(String)` / `ToolCallStarted(ToolCall)` / `ToolResult(ToolResult)` / `ApprovalNeeds { approval_id, card }` → (shape is `ApprovalNeeded { approval_id, card }`) / `TurnDone(AgentTurnReport)`. `AgentTurnState`: Thinking | RunningTool(name) | AwaitingApproval(id) | Done. Decisions in: `AgentDecision::Approve | ApproveAllThisTurn | Deny | Stop` on the turn's decision channel.
- Existing skeletons, all `#[allow(dead_code)]`, waiting for you:
  - `Effect::AgentSend(Uuid, String)` and `Effect::AgentDecision(Uuid, ApprovalId, AgentDecision)` — app_state.rs ~L128
  - `AppMessage::ToolCallStarted / ToolResult / ApprovalNeeded / AgentTurnDone(Uuid, ChatReceipt, AgentSummary)` — app_state.rs ~L190, with no-op `apply()` arms ~L505
  - the dispatcher's no-op arms — wiring.rs ~L831
  - the `agent_cards.rs` stub — selora-ui/src/sidebar/chat/agent_cards.rs

DELIVERABLES:

A. The cards (agent_cards.rs; UI_SPEC §4.2; every color/duration/curve from theme.rs; headless paint tests, the existing selora-ui pattern):
1. Read-type tool card — collapsed one-line 24px card (tool name --type-xs --fg-faint, argument summary --fg-muted, ✓ in --ok / ✕ in --err on the right); click expands to the tool result (monospace, max 200px, scrolls).
2. Diff approval card (write_file / edit_file) — unified diff, line numbers --fg-faint, removed lines in --err, added lines in --ok (the chat's ONLY saturated uses, semantic); body max 240px, scrolls; buttons right-aligned: Deny (ghost) / Approve all (appears from the second approval in a turn) / Approve (--ink-mid). While parked: the agent status line reads `awaiting your approval` with the pulse animation on the dot.
3. Bash command card — `$` command line (--fg-faint prefix, --type-md monospace); live streaming output (--ink-canvas, monospace, max 200px, "output truncated" notice at the 256KB cap); while running, spin glyph in the header and Stop replaces Approve; `✓ exit 0` / `✕ exit N` in the header. Auto-approved (allowlisted) commands render with no buttons.
4. Web cards — same collapsed one-line style: `web_search "query"` expands to a numbered result list (title + URL, click = copy URL); `web_fetch https://…` expands to the fetched text (truncated) with the source URL in the header.
5. Agent status line — the 24px line under the last bubble while a turn runs: phase text (`thinking` / `reading src/main.rs` / `awaiting your approval` / `running cargo test`) + the Stop button on the right (emits the Stop decision).
6. Turn summary footer — `6 tools · 3 approved · 1 denied · 12.4k tokens · $0.041` in --type-xs --fg-faint, left-aligned under the final bubble (from AgentSummary).

B. The panel integration (sidebar/chat/mod.rs):
7. Agent-mode toggle pill in the chat header (--type-xs, --fg-muted; on = --ok dot). Persisted in workspace.json like chat_open (serde-defaulted so pre-5d files load).
8. With the toggle on, submit starts an agent turn instead of a plain chat; card events render inline between bubbles in arrival order; decisions go back out. Extend the ChatPanelEvent vocabulary additively — existing events keep their shapes.

C. The final wiring (selora-app; make the skeletons real):
9. `Effect::AgentSend(request_id, text)` in wiring.rs: build the turn — conversation from the panel's turns (the same source the plain-chat path feeds into ChatRequest); model = the active model; TurnCaps from `settings.agent` (max_iterations / max_turn_secs / max_cost_usd, all three exist) + the model's context_window with the 0.20 reserve; `ClientChatStream` over a `SeloraClient` with the session key + base URL (exactly like spawn_chat); `RouterToolRunner` over `ToolContext::new(workspace_root, AgentToolSettings)` mapped from Settings.agent (+ the brave key, deliverable 12). The dispatcher must track the workspace root (it already routes OpenFolder) since ToolContext needs it. Call `spawn_agent_turn`, keep the turn's decision_tx keyed by request_id, and relay `AgentEvent` → the four AppMessage variants with `notify()` after every send (the spawn_chat relay pattern; stale/late ids dropped, never a panic).
10. `Effect::AgentDecision(request_id, approval_id, decision)`: forward onto that turn's decision channel; unknown or ended turn → no-op.
11. `apply()` arms in app_state.rs: reduce the four messages into the chat panel; AgentTurnDone also feeds the cost meter the way ChatDone does.
12. Settings: add `agent.brave_search_api_key` (serde-defaulted, so old files load). It maps into AgentToolSettings' existing `skip_serializing` field — the key never lands in a persisted file or a log. Empty/absent → DuckDuckGo.
13. §17.7 open-buffer routing: an approved write/edit to a file open in `AppState.buffers` must also update the in-memory buffer — the edit stays undoable (Ctrl+Z restores the pre-edit text), and since approving IS the disk write, the buffer ends up matching disk (clean). Test: approve an edit to an open file → buffer shows the new text, Ctrl+Z reverts the buffer, disk keeps the approved text.

One known seam to resolve (report whichever you pick): `AppMessage::AgentTurnDone` expects a `ChatReceipt`, but `AgentTurnReport` carries summary / end / final_message / conversation — no receipt. Either synthesize the receipt from the summary's tokens + cost_usd (compute_cost_usd as fallback), or adjust the message shape additively. Deviation is allowed; silence is not.

DO NOT:
- Touch selora-ai internals: turn.rs's loop, the router's guards, the tool manifest. You wire and render; they are frozen.
- Persist approvals across turns or sessions (ApproveAllThisTurn is turn-local — the loop already enforces it; keep it that way).
- Violate DESIGN_TOKENS §12 — the diff card's -/--err and +/--ok are the only saturated colors in the chat; everything else stays ink.
- Hardcode colors, durations, or curves — everything flows from theme.rs.
- Reshape the existing Effect/AppMessage variants — fill their arms; extend only additively.
- Persist agent transcripts across sessions (chat_turns keeps plain turns) — list it as an open item in your report.
- Run the loop on the UI thread or share UI state with the runtime — the §1/§2 process model (main-thread state, channels, request_repaint) is the contract.

ACCEPTANCE:
- `cargo build --workspace && cargo test --workspace`; `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings` — all green.
- Headless app test (the existing wiremock pattern, no network): a scripted agent turn drives the whole path — AgentSend → events arrive → cards render headlessly → a Deny continues the loop with the denied tool result → an Approve writes and updates the open buffer → a Stop ends the turn; the cost meter sees the turn's receipt; late/stale request ids are dropped.
- Headless paint tests for every card, the status line, and the summary footer.
- Manual, with a real key + real workspace: the ROADMAP.md Phase 5 acceptance list (11 steps) passes. That real-key run is the owner's post-merge gate — flag it, do not claim it.

WHEN DONE, report:
- The exact commands you ran and their outputs.
- Every deviation from UI_SPEC §4.2 / ARCHITECTURE §17 and why (including the ChatReceipt seam).
- Open items for the orchestrator/owner: the real-key smoke test, transcript persistence, ENV_ALLOWLIST additions (CARGO_HOME, proxies) if you hit them.
- Whether Phase 6h (KIMI_UI_PROMPT.md at the repo root) can now start.
