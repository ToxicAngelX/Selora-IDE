# Phase 9 crash / soak matrix

Deterministic crash and soak coverage for the agent workbench. Every PASS row
is a test on `integration` that uses only public APIs, a scripted gateway, and
process harnesses where the window is a real process death. Rows marked
PENDING refer to APIs that do not exist on `integration` yet; they are listed,
not stubbed or invented, so the test suite compiles and runs unchanged after
each owning branch merges.

Run everything:

```powershell
pwsh -File scripts/phase9-soak.ps1
```

The script prints the same matrix and exits non-zero when a PASS row regresses.
The frontend row is skipped (PENDING) when `node_modules` is absent.

## Matrix

| # | Crash / soak window | Status | Test | Notes |
|---|---------------------|--------|------|-------|
| 1 | Kill mid-agent-turn while streaming | **PASS** | `crates/selora-ai/tests/phase9_agent_crash.rs::kill_mid_stream_keeps_partial_answer_and_runs_nothing` | Partial answer kept; no tool routed; gateway future dropped; `StoppedByUser`; one terminal event. |
| 2 | Kill while parked on a write approval card | **PASS** | `...::kill_while_parked_on_a_write_card_executes_nothing` | Card emitted, then Stop: zero executions, no tool result. |
| 3 | Kill while parked on a bash approval card | **PASS** | `...::kill_while_parked_on_a_bash_card_executes_nothing` | Same contract for `ApprovalCard::Command`. |
| 4 | Kill while an approved write/bash tool is executing | **PASS** | `...::kill_during_approved_execution_aborts_and_emits_no_result` | Runs for both card kinds; abort reaches the runner; no `ToolResult`; `tools_used` stays 0. |
| 5 | Kill while a real child process is parked on a write card | **PASS** | `...::process_kill_while_parked_leaves_the_workspace_untouched` | Parent kills the child after `ApprovalNeeded`; `src/main.rs` is byte-identical, no temp/backup siblings. |
| 6 | Stale approval id races the real decision | **PASS** | `...::a_stale_approval_id_never_unparks_the_parked_card` | Wrong id ignored; real `Deny` wins; write never executes. |
| 7 | Repeated event ordering under load | **PASS** | `...::repeated_turns_keep_the_event_contract` | 40 turns; `State(Done)` before the single `TurnDone`; no result without a start. |
| 8 | Concurrent turns do not cross events | **PASS** | `...::concurrent_turns_never_cross_events` | 12 concurrent turns; each stream has exactly its own tool result. |
| 9 | Auto-approve is turn-local | **PASS** | `...::auto_approve_applies_only_inside_the_turn_that_enabled_it` | No card while enabled; a fresh `TurnInput`/`AgentTurn` starts off. |
| 10 | Auto-approve never survives a restart / new session | **PASS** | `apps/selora-desktop/frontend/src/features/agent/agent-store.test.ts::never carries auto-approve across a restart or into a new session` | Module restart, `reset()`, and `createSession()` all return to `ask_before_change`. Backend mode is an `AgentRequestDto` field per turn, never persisted. |
| 11 | Same-agent FIFO under contention | **PASS** | `crates/selora-workflow/tests/phase9_subagent_soak.rs::same_agent_fifo_holds_under_load_and_token_usage_is_exact` | 24 asks, one executor in flight, exact order, exact receipt token sum. |
| 12 | Distinct subagents respect the concurrency cap | **PASS** | `...::distinct_agents_respect_the_concurrency_cap_under_load` | 24 agents, cap 3 reached but never exceeded, no same-name overlap. |
| 13 | Failed ask rolls back without reordering | **PASS** | `...::failed_asks_release_the_queue_and_leave_no_duplicate_instruction` | Failed instructions are removed; only successful pairs remain. |
| 14 | Panicking ask never wedges the queue | **PASS** | `...::a_panicking_ask_never_wedges_later_concurrent_asks` | Panic propagates to its caller; eight later asks complete. |
| 15 | Journal torn tail at every byte offset | **PASS** | `...::every_torn_tail_is_repaired_and_the_prefix_is_stable_across_restarts` | Every cut inside the final record repairs to the prefix; two restarts are stable; the completed record survives exactly once. |
| 16 | Real process killed mid-ask, then restart | **PASS** | `crates/selora-workflow/tests/phase9_workflow_crash.rs::process_kill_mid_ask_then_restart_dispatches_once_and_accounts_once` | Child killed inside `Host::ask`; `incomplete_asks` = 1; resume re-dispatches once; tokens `7` and cost `0.25` counted once; a second resume of the completed run is refused before any host call. |
| 17 | Kill mid-subagent **background** run | **PENDING** | owner: Phase 9c/9e (`selora-p3-background`) | `crates/selora-ai/src/agent/subagent.rs` (background runtime) is not on `integration`. Required assertions after merge: background task detached from the parent turn; completion notification delivered exactly once; killing the app loses only in-flight work, never a completed result. |
| 18 | Kill a **parked subagent child** | **PENDING** | owner: Phase 9c (`selora-p3-subagent`) | No park/cancel API on `integration`. Parent cancellation of a parked child must release its queue ticket and leave no duplicate user message. |
| 19 | **Parent cancellation** of a running subagent | **PENDING** | owner: Phase 9c | Engine-level parent death is covered by row 16; the Phase 9 runtime's cooperative cancel (not process death) has no API yet. |
| 20 | **Late result** after cancellation | **PENDING** | owner: Phase 9c | Requires the runtime's result-ownership API. Must prove a late ask result is dropped, not appended to a cancelled parent or a reused agent name. |
| 21 | Question-card park / kill | **PENDING** | owner: Phase 9c (`ask_user_question`) | No question card on `integration`. Must prove a killed question park is not auto-answered and is not resurrected after restart. |
| 22 | Plan-card park / kill | **PENDING** | owner: Phase 9b (`enter_plan_mode` / `exit_plan_mode`) | No plan card on `integration`. Must prove writes stay structurally rejected while a plan is unapproved, including across restart. |
| 23 | Session JSONL torn tail / restart | **PENDING** | owner: Phase 9d | `integration` has no `.selora/agent-sessions/` JSONL store. The existing `sessions.json` is a whole-file atomic replace; a crash keeps the previous file and ignores `.tmp` (loss of at most the last 5s snapshot), which is not the Phase 9 durable transcript. |
| 24 | Protocol/API-imposed test survival across merge | — | — | The PASS tests use only public `selora-ai` / `selora-workflow` APIs. PENDING rows deliberately have no placeholder test: adding one that references a missing symbol would fail to compile after merge in the opposite direction. |
| 25 | Compaction round repeated twice | **PENDING** | owner: Phase 9d | No summarize round on `integration`. Required: two consecutive compactions are idempotent for the carried summary/todos; compaction is journaled before continuing. |
| 26 | Compaction summary failure | **PENDING** | owner: Phase 9d | No compaction API. Required: a failed summary aborts visibly with the pre-compaction transcript intact; tokens spent on the failed summary are still accounted once. |
| 27 | Compaction token accounting | **PENDING** | owner: Phase 9d | Only the non-compaction half is proven now (rows 16 and the agent summary token totals). Compaction must not double-count carried messages or the summary request. |

## Remaining crash windows (known, not hidden by the matrix)

1. **Agent transcript and pending cards are in memory.** The loop's conversation
   and the parked `ApprovalCard` live only in the turn task. A process kill
   leaves the workspace safe (row 5) but loses the transcript and the pending
   question/plan/approval. Phase 9d's session JSONL is the fix; acceptance #6
   cannot pass before it lands.
2. **A kill during an approved atomic write can leave a temp sibling.** The
   target is replaced atomically, so the user file is never torn, but
   `.<name>.selora-tmp-<pid>-<seq>` can survive a hard kill. It is inert and
   uniquely named; no recovery sweep exists yet.
3. **A kill can orphan a bash child.** The agent's bash tool kills its child on
   Stop, but a hard process kill relies on the OS closing the job/handles.
   Phase 8's `world.run` uses a kill-on-close Job Object; the agent bash path
   does not have an equivalent process-harness assertion yet.
4. **In-flight subagent results are lost on process death.** The Phase 8 engine
   journals the ask intent and re-dispatches (row 16). The Phase 9 background
   runtime must not claim a result before it is durable; rows 17-20 own that
   proof.
5. **Compaction does not exist**, so a long session still ends at
   `TurnEnd::ContextOverflow` with a partial answer. No silent truncation is
   possible today, but the session also cannot continue.
6. **Frontend `sessions.json` (pre-Phase 9) is not a transcript log.** It is
   rewritten atomically every 5s; a crash loses at most the newest snapshot.
   This is a different guarantee from the Phase 9 JSONL tail rule.

## PENDING API inventory (do not invent)

| Feature | Expected API / file | Owner branch |
|---|---|---|
| Plan mode + plan card | `enter_plan_mode`, `exit_plan_mode`, plan approval card | `selora-p3-plan` / Phase 9b |
| Todos | `todo_read` / `todo_write`, session todo state | Phase 9b |
| Questions | `ask_user_question`, question card on the §17.4 park channel | Phase 9c |
| Subagent runtime | `spawn_subagent`, scoped policies, `send_message` steering | `selora-p3-subagent` / Phase 9c |
| Background subagents | detached execution, completion events, `wait` tool | `selora-p3-background` / Phase 9e |
| Session JSONL | `.selora/agent-sessions/{id}/` transcript + todos + open parks | Phase 9d |
| Compaction | summarize round at ~80% of the context budget | Phase 9d |
| Session auto-approve flag | session-scoped, off by default, never persisted | Phase 9d (frontend default already enforced here) |