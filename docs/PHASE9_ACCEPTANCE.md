# Phase 9 acceptance record (headless)

Date: 2026-10-27.
Phase: ROADMAP.md Phase 9 - Agent Workbench.
Worktree: `C:\Users\Toxic\Desktop\selora-p3-accept`, branch `selora-p3-accept`,
based on `integration` at `9135090`.
Result: **12 automated checks PASS, 0 FAIL, 9 PENDING. Owner acceptance UNRUN.
Phase 9 is NOT accepted.**

This record covers only what this worktree executed headlessly: deterministic
agent-loop fixtures, existing desktop agent tests, the protocol slice, and the
frontend agent slice. It does not claim a live-key run, a visible window, a
crash/relaunch observation, or any human judgement.

## 1. Status vocabulary

| Status | Meaning in this record |
|---|---|
| **PASS** | An automated check ran on this machine and passed. It is not owner acceptance. |
| **FAIL** | An automated check ran and failed, or a merged branch carries an API that the worktree lost. |
| **PENDING** | The runtime API is not in this worktree. The owning branch is unmerged, stale, or carries no implementation. Never counted as a pass. |
| **OWNER** | A human observation that needs a live key, a real workspace, a visible window, or a crash/relaunch. Always shown as `OWNER`, never auto-filled. |

`PENDING` and `OWNER` never change the harness exit code. A `FAIL` does.

## 2. Method

No product behavior was changed to make a check pass. The additions are:

- `crates/selora-ai/tests/phase9_acceptance.rs` - five deterministic fixtures over
  the public `ChatStream`/`ToolRunner` seams and a throwaway temp workspace. No
  network, no gateway, no key, no clock dependence.
- `scripts/phase9-acceptance.ps1` - the headless runner. It probes the runtime
  branches, runs the available suites, and prints every roadmap step with
  `AUTOMATED: PASS|FAIL|PENDING` and an explicit `OWNER: OWNER` column.
- This record.

The existing desktop tests use a loopback `wiremock` mock gateway and a fixed
test key (`sessions.rs`'s Z3 harness). The frontend slice is vitest. None of the
runs touched a real credential or a live gateway.

### 2.1 Deterministic fixtures added

| Fixture | What it proves | Step |
|---|---|---|
| `step1_write_is_planned_as_a_diff_and_applied_only_after_approval` | `write_file` returns a diff card; the file is untouched until `execute_approved`; the approved bytes land and the recovery sidecar exists. | 1 substrate |
| `step1_edit_diff_revalidates_against_the_current_file` | An `edit_file` card built against one revision is refused after a concurrent change; the newer content is not clobbered. | 1 substrate |
| `step2_manage_todo_is_ready_and_never_builds_an_approval_card` | `manage_todo` is a non-side-effecting result, not a write card. It does not prove authoritative session todos. | 2 substrate |
| `step6_context_budget_is_eighty_percent_of_the_window` | `TurnCaps::context_budget_tokens` is window minus the 20% reserve (80%). It does not prove a summarize round. | 6 substrate |
| `step7_auto_approve_skips_cards_for_one_turn_and_resets_on_restart` | Auto mode writes without a file card; a fresh `TurnInput` (the engine-restart analogue) defaults back to cards and a Deny leaves the file untouched. | 7 |
| Desktop `agent_turn_streams_tools_approvals_and_completes` | Full desktop vertical: read round, diff card, decision, write, turn done. | 1 substrate |
| Desktop `auto_mode_applies_file_writes_without_an_approval_card` | The desktop Auto session mode skips the file card and applies the write. | 7 |
| Desktop `explore_mode_arms_the_read_only_manifest` | Explore mode arms only read tools; this is capability advertisement, not a structural plan-mode write rejection. | 9 substrate |

## 3. Automated run

Command (from the worktree root):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase9-acceptance.ps1
```

Result on this machine: **exit 0** (available automated checks passed).

| Check | Status | Result |
|---|---|---|
| `test.phase9-suite` | PASS | 5 passed, 0 failed |
| `test.step1-plan-diff` | PASS | 2 passed, 0 failed |
| `test.step2-todo` | PASS | 1 passed, 0 failed |
| `test.step6-context-budget` | PASS | 1 passed, 0 failed |
| `test.step7-auto-reset` | PASS | 1 passed, 0 failed |
| `test.protocol-agent` | PASS | 1 passed, 0 failed |
| `test.desktop-write-gate` | PASS | 1 passed, 0 failed |
| `test.desktop-auto` | PASS | 1 passed, 0 failed |
| `test.desktop-explore` | PASS | 1 passed, 0 failed |
| `test.frontend-agent` | PASS | 106 passed, 0 failed |
| `sessions.gitignore` | PASS | `/.selora/` is ignored |
| `auto.session-flag` | PASS | `auto_approve_writes`/`AgentModeDto::Auto` present |
| `plan.api` | PENDING | unmerged `selora-p3-plan` |
| `todos.authoritative` | PENDING | unmerged `selora-p3-plan` |
| `questions.structured` | PENDING | unmerged `selora-p3-subagent` |
| `subagents.tool` | PENDING | unmerged `selora-p3-subagent` |
| `sessions.resume` | PENDING | unmerged `selora-p3-sessions` |
| `compaction.at80` | PENDING | unmerged `selora-p3-sessions` |
| `background.completion` | PENDING | unmerged `selora-p3-background` |
| `plan.write-rejection` | PENDING | unmerged `selora-p3-plan` |
| `sessions.workspace` | PENDING | no `.selora/agent-sessions/` in this checkout |

The desktop/frontend slices need `apps/selora-desktop/frontend/dist` and
`node_modules`. This run built them locally:

```powershell
npm.cmd --prefix apps\selora-desktop\frontend ci --no-audit --no-fund
npm.cmd --prefix apps\selora-desktop\frontend run build
```

Both directories are gitignored, so a clean checkout should use:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase9-acceptance.ps1 -BuildFrontend
```

Without them the harness reports `SKIP` for the desktop and frontend checks and
still runs the Rust acceptance suite.

## 4. Roadmap steps

| Step | Automated | Owner | Blocked on |
|---|---|---|---|
| 1. Plan -> approve plan -> diff -> write | **PENDING** (write/diff substrate PASS) | OWNER | `plan.api` (`selora-p3-plan`) |
| 2. Live todos | **PENDING** (`manage_todo` substrate PASS) | OWNER | `todos.authoritative` |
| 3. Structured question resumes | **PENDING** | OWNER | `questions.structured` |
| 4. Two read-only subagents grouped | **PENDING** | OWNER | `subagents.tool` |
| 5. Close/relaunch resume with todos | **PENDING** | OWNER | `sessions.resume`, `todos.authoritative` |
| 6. Long session compaction at 80% | **PENDING** (80% budget substrate PASS) | OWNER | `compaction.at80` |
| 7. Auto-approve resets on restart | **PASS** | OWNER | none headlessly; kill/relaunch is owner-only |
| 8. Background completion + result | **PENDING** | OWNER | `background.completion` |
| 9. Plan mode rejects writes structurally | **PENDING** (Explore manifest substrate PASS) | OWNER | `plan.write-rejection` |

Step 7 is the only automated PASS. It is still **not** owner acceptance: the
headless test proves a fresh turn has `auto_approve_writes = false`; only a real
kill/relaunch on a visible window proves the session flag is gone.

Step 9's Explore-mode manifest test is deliberately not counted as a step pass.
Explore filters the advertised tools; it is not the 9b router/session gate that
returns the plan-first error for a write.

### 4.1 Owner observations (all UNRUN)

1. New Agent session, mission, plan card, approve, diff card, approve, file
   changes, and Ctrl+Z reverts it.
2. The todo spine updates live as steps complete and survives a session switch.
3. A structured question parks the turn; answering it (option and free text)
   continues the same run.
4. Two read-only subagents both run, group under the parent, and cannot write.
5. Kill the app mid-run, relaunch, Resume, and the session continues with its
   todos.
6. A session past 80% performs one compaction round and continues coherently.
7. Auto-approve skips cards; kill/relaunch clears the flag and restores cards.
8. A background child finishes while editing; a toast appears and its result is
   in the parent session.
9. Plan mode makes a write request return a plan-first error; no diff/write card
   appears.

## 5. Runtime branch state at run time

These are the heads observed by the harness, not claims about their contents:

| Branch | Head | State in the worktree | Owning slice |
|---|---|---|---|
| `selora-p3-plan` | `9d9b87e` (`feat(agent): plan mode, real todos, and plan approval card`) | UNMERGED+WORK | 9b |
| `selora-p3-subagent` | `bb28da8` (`feat(agent): shared subagent runtime and structured escalations`) | UNMERGED+WORK | 9c |
| `selora-p3-sessions` | `ed44c1e` (`feat(agent): durable sessions with resume, retry, compaction...`) | UNMERGED+WORK | 9d |
| `selora-p3-background` | `0be91ee` (`feat(agent): background subagent completion events and wait tool`) | UNMERGED+WORK | 9e |
| `integration` | `9135090` | base | - |

`selora-p3-window` (`97f9499`, 9a frontend workbench) is also unmerged. The
worktree already has an earlier agent dock and plan-spine UI, but the backend
contracts those components need (`PlanStateChanged`, `TodosChanged`,
`decision_prompt`, `ChatStateDto.todos`) are absent.

## 6. Blocked items in detail

- **Plan mode / todos (9b, `selora-p3-plan` 9d9b87e).** The branch adds
  `crates/selora-ai/src/agent/plan.rs`, `enter_plan_mode`, `exit_plan_mode`,
  `todo_read`, `todo_write`, `PlanStateChanged`, `TodosChanged`, and a
  plan-first rejection in the router. None is in this worktree. `manage_todo`
  here only echoes the supplied list back; it stores no session state.
- **Structured questions and tool subagents (9c, `selora-p3-subagent`
  bb28da8).** The branch adds `ask_user_question`, `spawn_subagent`, the
  structural `SubagentPolicy` (`read-only` has no write tools), the
  escalation payload, and `QuestionCard.tsx`. None of it is in this worktree;
  the 9a `AgentQuestionCard` on `selora-p3-window` also has no backend payload
  here.
- **Sessions, resume, compaction (9d, `selora-p3-sessions` ed44c1e).** The branch
  adds `crates/selora-ai/src/agent/session.rs` with
  `.selora/agent-sessions/{id}/`, `transcript.jsonl`, `todos.json`,
  `state.json`, `COMPACTION_THRESHOLD = 0.80`, and
  `compact_conversation`. This worktree keeps chat sessions in memory only and
  never writes `.selora/agent-sessions/`.
- **Background completion (9e, `selora-p3-background` 0be91ee).** The branch adds
  `SubagentRuntime`, `SubagentCompleted`, and the `wait` tool. This worktree has
  neither the runtime file nor the protocol event, so a background child cannot
  notify the parent.
- **Frontend dist for desktop tests.** A clean checkout lacks
  `apps/selora-desktop/frontend/dist`; the Tauri test binary embeds it. The
  harness reports the desktop checks as `SKIP` unless `-BuildFrontend` is used.
  This is an environment prerequisite, not a Phase 9 failure.
- **Owner acceptance.** Every row in section 4.1 needs a human, a live key, a
  real workspace, and a visible window. Nothing here is a substitute.

## 7. What happens when the branches merge

`scripts/phase9-acceptance.ps1` needs no edit for the merges:

- It probes the worktree first, then the branch tree. A probe that exists only
  on an unmerged branch is `PENDING` with the branch name.
- When the API reaches the worktree, the same script runs the registered
  headless filter (`plan_mode`, `todo`, `question`, `subagent`, `resume`,
  `compact`) across `selora-ai` and, when `frontend/dist` exists,
  `selora-desktop`.
- A filter with a passing test flips the capability to `PASS`. A filter with no
  matching test stays `PENDING` with "API present; no headless test matched
  yet" - it does not fake a pass.
- If a branch is merged and its tree carries the API but the worktree does not,
  the capability is `FAIL`: a merged slice was lost.
- `PENDING` never becomes `PASS` from a probe alone. Every roadmap step still
  carries `OWNER: OWNER` and the owner observation remains unrun.

## 8. Reproduce

```powershell
# Available checks only (desktop/frontend slices may SKIP on a clean checkout)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase9-acceptance.ps1

# Build the frontend first so the Tauri test binary can compile
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase9-acceptance.ps1 -BuildFrontend

# The new suite directly
cargo test -p selora-ai --test phase9_acceptance --locked
```

Logs land under `%TEMP%\selora-phase9-acceptance-<timestamp>\` and are never
added to the repository.