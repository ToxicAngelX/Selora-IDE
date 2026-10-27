# Phase 9b - Plan mode, todos, and the effects wave

> **STATUS: FINALIZATION SKELETON (2026-10-27).**
> **NOT STARTED.** The `selora-p3-plan` branch has no commits.
> No plan gate, no `todo_read`, no `todo_write`, and no writes-during-planning
> rejection exist in the tree.
>
> Everything below is a contract for the wave, not evidence that it landed.
> Fill `TODO-VERIFY` only after implementation and the gate.

Depends on: Phase 5b plan/apply split, Phase 9a Agent window, and the
backend-authoritative session slice.
Blocks: the Phase 9 owner steps 1-3 and the effects acceptance.

## 1. Wave status

| Item | State |
|---|---|
| `enter_plan_mode` / `exit_plan_mode(plan)` | **NOT STARTED** |
| Plan card approve / request changes | Rendering exists in 9a; runtime does not |
| Writes-during-planning structural rejection | **NOT STARTED** |
| Plan approval flips execution | **NOT STARTED** |
| `todo_read` | **NOT STARTED** |
| `todo_write` | **NOT STARTED**; `manage_todo` exists on the session branch |
| Manifest v3 | **NOT STARTED** |
| Owner acceptance | **UNRUN** |

**Plan wave:** not started. **Effects wave:** not started. Do not label the
Agent Workbench "plan-gated" until both are implemented and observed.

## 2. Plan mode contract

Plan mode is a session mode, enforced at the router's plan/apply seam, not by
prompt discipline.

- While planning, `execute_tool` on `write_file`, `edit_file`, `delete_file`,
  and non-allowlisted `bash` returns a tool error: "plan approved first;
  propose the plan". The model sees the error and adapts, like a path-guard
  rejection.
- The plan is delivered by `exit_plan_mode(plan)`. It parks the turn on a plan
  card (the existing approval-card family).
- **Approve** flips the session to execution mode. **Request changes** returns
  the revision note as a tool result and keeps the session in plan mode.
- No write is possible before approval by construction: the write path is
  `execute_approved`, and nothing calls it while the session is planning.
- Approved writes still use the normal diff/approval card. Plan approval does
  not blanket-approve file content.

### Effects wave exit criteria

- A write attempted before plan approval produces a tool error and no file
  change.
- An approved plan produces a diff card; deny leaves the file unchanged;
  approve writes once; undo reverts.
- `bash` remains separately approved unless allowlisted.
- The plan card survives a reload if sessions are durable (9d); otherwise the
  limitation is explicit.

## 3. Todo contract

- `todo_write` validates and stores a list for the owning session. Proposed
  shape matches the existing `manage_todo` store: id, text, status; bounded
  count and lengths; unique ids; `pending` / `in_progress` / `completed`.
- `todo_read` returns the stored list; it must not synthesize steps from model
  prose.
- Malformed input is a tool error and stores nothing.
- A context without a session store (workflow subagent) returns a typed error.
- The spine remains read-only unless a separate user command is designed.
  Do not reintroduce local edits that the backend never sees.
- Live updates use `session_todos_changed`; `SessionDto.todos` and
  `ChatStateDto.todos` carry the authoritative state.

Finalize after implementation:

- Tool result shape: `TODO-VERIFY`
- Event name / payload: `TODO-VERIFY`
- Bounds (count, id length, text length): `TODO-VERIFY` against code, not this
  document
- Whether `manage_todo` stays as an alias or is replaced: `TODO-VERIFY`

## 4. Session mode row

The 9a composer row maps Ask / Agent / Plan to session policies. 9b must make
those policies real:

- Ask: chat only, no tools.
- Agent: normal tool manifest.
- Plan: read tools only plus `exit_plan_mode`; writes rejected.
- The selected mode is session state, visible in the header, and does not
  persist as a trust grant.

## 5. Automated checks to add

```powershell
cargo test -p selora-ai --locked plan
cargo test -p selora-desktop --locked plan
cargo test -p selora-protocol --locked plan
npm.cmd --prefix apps/selora-desktop/frontend run test -- src/features/agent
```

Required coverage:

- write/edit/delete/bash rejected during planning, with no filesystem effect;
- approval flips execution;
- request-changes returns feedback and stays in plan mode;
- todo read/write round-trip, bounds, malformed input, no-store error;
- todo event reaches the active session only;
- plan card survives the supported reload path, or the limitation is asserted.

Results: `TODO-VERIFY`.

## 6. Owner acceptance

Run Phase 9 steps 1-3 in `docs/PHASE9_ACCEPTANCE.md` on a visible window:

1. planning explores read-only and proposes a plan card;
2. approving the plan produces a diff card, then an approved write, and Ctrl+Z
   reverts;
3. the todo spine updates live during execution.

All three are **UNRUN**.

## 7. Risks and limits

- The plan gate must not be bypassable through a subagent or workflow tool.
  Audit every write-capable path, including Phase 8's `files` facade.
- `bash` is not a write tool in the manifest, but it can write. Decide whether
  plan mode rejects all non-allowlisted bash or only mutating commands. Record
  the decision; do not leave it implicit.
- Todo state must stay in the session store. A local copy recreates the
  fabricated-surface bug the session slice removed.
- If 9d is not merged, plan state is memory-only and cannot survive a restart.
  Mark that limitation in the acceptance record.

## 8. Finalization fields

- Implementation merge commit: `TODO-VERIFY`
- Manifest version: `TODO-VERIFY`
- Plan gate tests: `TODO-VERIFY`
- Todo tests: `TODO-VERIFY`
- Owner steps 1-3: **UNRUN**
- Bash-in-plan decision: `TODO-VERIFY`
- Effects wave status: **NOT STARTED** until the three exit criteria are
  observed