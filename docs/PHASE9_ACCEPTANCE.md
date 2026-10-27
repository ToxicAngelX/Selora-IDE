# Phase 9 acceptance checklist - Agent Workbench

> **STATUS: FINALIZATION SKELETON (2026-10-27).**
> Owner acceptance: **UNRUN**.
> Waves 9b-9e: commits exist on their branches but are **not merged** into this
> release lane; acceptance is **NOT RUN**.
> Wave 9a: implemented on `selora-p3-window`; not merged in this lane.
> Phase 9 is **not accepted**.
>
> Fill `TODO-VERIFY` only from the final merged tree. Do not mark a wave passed
> because a headless test exists. Do not invent test counts.

Phase: `ROADMAP.md` Phase 9 - Agent Workbench.
Companion status: `STATUS.md`, `docs/ui/PHASE9_AGENT_WORKBENCH.md`.

## 1. Wave gate

A step can run only when its owning wave is merged and its automated gate is
green. Otherwise record **BLOCKED - NOT IMPLEMENTED**.

| Wave | Scope | Merge state | Automated gate | Acceptance |
|---|---|---|---|---|
| 9a | Agent window, tabs, todo spine, question/plan cards, mode row | `TODO-VERIFY` | `TODO-VERIFY` | `TODO-VERIFY` |
| 9b | Plan mode, `todo_read`/`todo_write`, writes-during-planning rejection (**effects wave**) | branch commits; not merged in this lane | `NOT RUN` | **NOT RUN** |
| 9c | Subagent runtime, `spawn_subagent`, `ask_user_question`, steering | branch commits; not merged in this lane | `NOT RUN` | **NOT RUN** |
| 9d | Durable sessions, resume picker, compaction, session auto-approve | branch commits on `selora-wt-session` / `selora-p3-sessions`; not merged in this lane | `TODO-VERIFY` | **NOT RUN** |
| 9e | Background subagents, completion toast/badge, blocking wait | branch commits; not merged in this lane | `NOT RUN` | **NOT RUN** |

## 2. Owner prerequisites

- Real gateway key (`sk-gw-` or a signed-in account) and a real workspace.
- `rust-analyzer` and `typescript-language-server` on `PATH` for LSP steps.
- A visible window. Headless paint tests are necessary, not sufficient.
- A scratch repository for the write/approve steps.
- A clean profile if first-run onboarding is part of the check.

## 3. Acceptance steps (ROADMAP Phase 9)

### Step 1 - plan first, read-only exploration

Action: open the Agent activity, create a session, and ask:
"plan a fix for the TODO in src/main.rs".

Expected:

- the agent explores with read tools only;
- no write card appears before plan approval;
- a plan card lists the proposed steps.

Result: **UNRUN** `TODO-VERIFY`
Evidence / notes: `TODO-VERIFY`

Blocked by: 9b (plan gating). 9a can render a plan card from a parked approval,
but it does not enforce read-only planning.

### Step 2 - approve plan, approve diff, undo

Action: approve the plan, then approve the diff card.

Expected:

- the file changes only after the diff approval;
- `Ctrl+Z` reverts the edit;
- no second unrelated write occurs.

Result: **UNRUN** `TODO-VERIFY`
Evidence / notes: `TODO-VERIFY`

Blocked by: 9b (effects wave).

### Step 3 - live todos

Action: run the approved plan.

Expected: the todo list updates live as steps complete, and the state survives a
tab switch.

Result: **UNRUN** `TODO-VERIFY`
Evidence / notes: `TODO-VERIFY`

Partial: 9a renders the read-only spine; `selora-wt-session` stores todos from
`manage_todo`. `todo_read`/`todo_write` and user editing are 9b work.

### Step 4 - structured question

Action: trigger an escalation question.

Expected: a question card offers options and optional free text; answering
continues the same turn.

Result: **UNRUN** `TODO-VERIFY`
Evidence / notes: `TODO-VERIFY`

Blocked by: 9c. 9a only renders a question payload on a parked approval.

### Step 5 - read-only subagents

Action: "Spawn two read-only subagents to review the diff".

Expected: both run, their cards group in the transcript, and neither has write
tools.

Result: **UNRUN** `TODO-VERIFY`
Evidence / notes: `TODO-VERIFY`

Blocked by: 9c. No subagent runtime exists.

### Step 6 - crash and resume

Action: close the app mid-run, relaunch, choose Resume.

Expected: the session continues with its todos and transcript.

Result: **UNRUN** `TODO-VERIFY`
Evidence / notes: `TODO-VERIFY`

Blocked by: 9d. Sessions are currently in memory; no durable JSONL or resume
picker exists.

### Step 7 - compaction

Action: run a session past ~80% of the context budget.

Expected: one summarize round keeps the mission, open todos, and last N messages
coherent.

Result: **UNRUN** `TODO-VERIFY`
Evidence / notes: `TODO-VERIFY`

Blocked by: 9d. No compaction path exists.

### Step 8 - session auto-approve is not persisted

Action: enable the session auto-approve flag, make a write, kill the app, and
relaunch.

Expected: writes skip cards during that session; the flag is gone after
relaunch.

Result: **UNRUN** `TODO-VERIFY`
Evidence / notes: `TODO-VERIFY`

Blocked by: 9d. The flag is not implemented. The existing Full auto-apply is a
composer mode for file tools only and dies with the session; confirm its default
separately.

### Step 9 - background completion notification

Action: start a background subagent, edit a file while it runs, then wait for
completion.

Expected: a toast and panel badge appear; clicking the result opens the owning
session.

Result: **UNRUN** `TODO-VERIFY`
Evidence / notes: `TODO-VERIFY`

Blocked by: 9e. No background runtime or notification exists.

## 4. Automated checks to run after merge

Record exact commands and outcomes. Do not copy counts from another tree.

```powershell
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm.cmd --prefix apps/selora-desktop/frontend run typecheck
npm.cmd --prefix apps/selora-desktop/frontend test
npm.cmd --prefix apps/selora-desktop/frontend run build
```

Targeted evidence to identify (fill after merge):

| Area | Expected file(s) | Result |
|---|---|---|
| Workbench UI contract | `features/agent/AgentWorkbench.test.tsx` | `TODO-VERIFY` |
| Session tabs / keyboard | `components/AgentSessionTabs.tsx`, `AgentSessionRail.test.tsx` | `TODO-VERIFY` |
| Todo spine | `components/AgentTodoSpine.tsx` and backend todo tests | `TODO-VERIFY` |
| Question / plan cards | `AgentQuestionCard.tsx`, `AgentPlanCard.tsx` | `TODO-VERIFY` |
| Session commands | `sessions.rs`, `state.rs`, protocol tests | `TODO-VERIFY` |
| Event ordering | `EventLog` multi-producer regression test | `TODO-VERIFY` |
| Plan/effects gating | not implemented | **NOT RUN** |
| Subagent runtime | not implemented | **NOT RUN** |
| Compaction | not implemented | **NOT RUN** |
| Background runs | not implemented | **NOT RUN** |

## 5. Exit criteria

Phase 9 can be marked accepted only when:

- [ ] 9a merge state and UI gate recorded
- [ ] 9b plan mode gates writes; effects wave verified by deny + undo
- [ ] 9c subagents run with typed policies; no write tools in read-only mode
- [ ] 9d sessions survive relaunch; compaction observed; auto-approve dies
- [ ] 9e background completion notifies and opens the owning session
- [ ] Steps 1-9 above each have a result and evidence
- [ ] `STATUS.md`, `ROADMAP.md`, `CHANGELOG.md` updated from the same tree
- [ ] No unrun step is described as passed
- [ ] Known limits remain listed; none silently dropped

Until then:

```
Phase 9: NOT ACCEPTED
Owner acceptance: UNRUN
Waves: 9a-9e branches not merged in the release lane; acceptance unrun
```