# Phase 9 - Agent Workbench (status and contract)

> **STATUS: FINALIZATION SKELETON (2026-10-27).**
> Phase 9 is **NOT ACCEPTED**. Only the 9a frontend contract exists, on branch
> `selora-p3-window`; merge `TODO-VERIFY`. Waves 9b-9e are **NOT STARTED**,
> except the in-memory session/todo/retry slice on `selora-wt-session`.
>
> This document describes the intended post-merge shape. Do not cite it as
> proof that a wave landed. Fill `TODO-VERIFY` after the final gate.

Phase 9 upgrades Agent Mode from chat-with-tool-cards to a dedicated workbench:
plan mode, a live todo spine, structured escalation questions, subagents,
resumable sessions with compaction, and session-scoped permission modes.

## 1. Wave status

| Wave | Scope | State |
|---|---|---|
| 9a | Agent window, session tabs, read-only todo spine, question/plan card rendering, mode row | Frontend implemented on `selora-p3-window`; merge `TODO-VERIFY` |
| 9b | `enter_plan_mode` / `exit_plan_mode`, `todo_read` / `todo_write`, writes-during-planning rejection, plan approval flips execution (**effects wave**) | **NOT STARTED** |
| 9c | Shared subagent runtime, `spawn_subagent`, `ask_user_question`, steering, read-only/full policies | **NOT STARTED** |
| 9d | Durable session JSONL, resume picker, compaction, session auto-approve | **PARTIAL** - in-memory authoritative sessions/todos/retry on `selora-wt-session`; persistence, resume, compaction, auto-approve **NOT STARTED** |
| 9e | Background subagents, completion toast/badge, blocking wait | **NOT STARTED** |

Plan/effects status is explicit: **the plan wave is not started, and the
effects wave (approved writes after a plan) is not started.** The 9a plan card
can display a plan delivered on a parked approval, but it does not gate tools.

## 2. Surfaces

| Surface | Path | Responsibility |
|---|---|---|
| Workbench shell | `frontend/src/layout/AgentDock.tsx` | Session tabs, transcript, todo spine, Chat/Plan views, composer, header Stop, Editor/Split exits |
| Session tabs | `features/agent/components/AgentSessionTabs.tsx` | WAI-ARIA tabs over backend sessions; New / select dispatch real commands |
| Todo spine | `features/agent/components/AgentTodoSpine.tsx` | Read-only authoritative todos + progress |
| Question card | `features/agent/components/AgentQuestionCard.tsx` | Options + optional free text on a parked approval |
| Plan card | `features/agent/components/AgentPlanCard.tsx` | Plan steps + Approve / Request changes on a parked approval |
| Transcript card | `features/agent/components/ToolCard.tsx` | Reuses the existing card; renders the decision cards when a prompt payload is present |
| Composer | `features/agent/components/AgentComposer.tsx` | Ask / Agent / Plan row mapped to session permission policies |

Transcript rendering is not forked. `AgentExecutionFeed`, `ReasoningAccordion`,
`MarkdownMessage`, prompt queue, and Rewind/Fork keep their existing behavior.

The workbench is mounted by `ShellGrid` in `data-view-mode="agent"` (full height
beside the activity bar) and in split mode. The title-bar Agent pill and the
activity-bar Agent button enter it; the dock header exposes **Editor** and
**Split** exits.

## 3. Session contract

Sessions must never be synthesized by the frontend.

- Hydrate reads `BootstrapState.chat.sessions` into `agentStore.sessions`, and
  keeps each session's transcript and todos in `transcripts` /
  `todosBySession`.
- New -> `FrontendCommand::NewSession`.
- Select -> `FrontendCommand::SwitchSession { session_id }`.
- On success the authoritative `bootstrap_state` event rehydrates the rail.
- 9a against the base tree received `unsupported_phase_slice` and showed the
  exact message without creating a placeholder. The `selora-wt-session` branch
  implements the commands; **re-verify this fallback path after merging both**.
- The backend session slice also adds `retry_message` and `set_active_buffer`,
  returns a small ack for event-backed mutations, and serializes event sequence
  assignment under one order guard.

Durable session storage (`.selora/agent-sessions/`), resume, compaction, and the
session auto-approve flag are 9d work and do not exist yet.

## 4. Additive protocol shapes

The 9a branch declares these frontend shapes. Promote them to typed Rust
commands/events in 9b/9c; unknown fields are ignored by the pre-9b backend.

```ts
type AgentDecisionPrompt =
  | {
      kind: 'question';
      header?: string;
      question: string;
      options: { id: string; label: string; description?: string }[];
      allow_free_text?: boolean;
      free_text_placeholder?: string;
    }
  | {
      kind: 'plan';
      title?: string;
      summary?: string;
      steps: { title: string; detail?: string }[];
    };
```

```ts
type AgentDecisionResolution =
  | { kind: 'question'; option_id?: string; text?: string }
  | {
      kind: 'plan';
      decision: 'approve' | 'request_changes';
      feedback?: string;
    };
```

Todos ride an additive `ChatStateDto.todos` field:

```ts
chat.todos?: {
  id: string;
  session_id?: string;
  title: string;
  status: 'pending' | 'in_progress' | 'completed';
  assigned_subagent?: string;
}[];
```

The session branch publishes `SessionDto.todos` and
`session_todos_changed { session_id, todos }`; `ChatStateDto.todos` carries the
active session. Markdown checklists in model prose and prompt-derived steps are
deliberately **not** converted into todos.

## 5. Plan mode and the effects wave (9b, not started)

The design rides the Phase 5 plan/apply split:

- `enter_plan_mode` / `exit_plan_mode(plan)`; plan approval is a parked
  approval with a plan card.
- While planning, write tools return a structural "plan approved first" tool
  error. The rejection is enforced by the router, not by prompt discipline.
- Approving the plan flips the session to execution; the file edit still gets a
  diff card under the normal §17.4 rules.
- `todo_read` / `todo_write` let the agent read and update the session todo list.
  The user-facing spine stays read-only unless a separate command is designed.
- Additive loop events only; no new trust primitive.

Acceptance for the effects wave: a write attempted before approval produces a
tool error and no file change; after approval the diff card appears; deny leaves
the file unchanged; undo reverts an approved edit.

## 6. Subagent runtime and escalations (9c, not started)

One runtime, two consumers: the interactive `spawn_subagent` tool and the Phase
8 workflow engine. Whichever phase lands first owns
`crates/selora-ai/src/agent/subagent.rs`; the other reuses it additively.

- Typed policies: `read-only` (no write tools) and `full` (writes still card).
- Background execution and `send_message` steering.
- `ask_user_question(questions)`: text, 2-4 options, optional free text; the
  answer rides the existing decision route and continues the same turn.
- Completion notifications and grouped cards.

## 7. Sessions, compaction, background (9d/9e, not started)

- Session JSONL + todos + open parks under `.selora/agent-sessions/`
  (gitignored, matching the workflow journal).
- Resume picker; compaction at ~80% of the context budget (summary + open todos
  + last N messages).
- Session auto-approve: session-scoped, off by default, never persisted.
- Background subagents survive editing; toast + panel badge on completion; a
  blocking `wait` tool for the main agent.

## 8. Accessibility and motion

- Session tabs: `tablist` / `tab` / `tabpanel`, roving tabindex,
  Left/Right/Home/End activate the backend session.
- Composer mode row: labelled group of `aria-pressed` buttons; the permission
  dropdown still owns the detailed policy copy.
- Question options are native radios; free text is a labelled textarea; empty
  answers cannot be sent. Plan changes require a revision note.
- Stop is a real header button with an accessible name and a disabled idle state.
- Focus rings use `--border-focus`; colors, spacing, and radii read tokens only.
- Reduced motion is inherited from `base.css`; 9a adds no new loops.

## 9. Gates

```powershell
npm.cmd --prefix apps/selora-desktop/frontend run typecheck
npm.cmd --prefix apps/selora-desktop/frontend test
npm.cmd --prefix apps/selora-desktop/frontend run build
cargo test --workspace --no-fail-fast
```

Headless coverage on the 9a branch covers new/select, unsupported fallback,
keyboard tabs, todo empty/live, question option/free text, plan
approve/request changes, Stop, and the mode row. Record the actual counts after
merge: `TODO-VERIFY`. Do not copy a count from this paragraph.

## 10. Gaps to resolve at finalization

1. `NewSession` / `SwitchSession` routing: 9a fallback vs session-branch
   implementation. Reconcile the docs and tests.
2. `ChatStateDto.todos` / `ApprovalDto.decision_prompt`: declared in 9a; promote
   to typed Rust and emit on bootstrap so a parked card survives reload.
3. Plan execution gating (9b) and effects (approved writes) are absent.
4. Subagent runtime (9c) is absent.
5. Durable sessions, resume, compaction, auto-approve (9d) are absent except
   in-memory session/todo/retry.
6. Background runs and notifications (9e) are absent.
7. Token/cost badges on sessions are omitted until `SessionDto` reports them.
8. The pre-existing rail may still contain fabricated data unless the session
   branch replaces it. Verify no seeded roster, fake branch, or local todo edit
   survives the merge.

## 11. Finalization fields

- 9a merge commit: `TODO-VERIFY`
- 9a gate counts: `TODO-VERIFY`
- Session branch merge commit: `TODO-VERIFY`
- Plan/effects wave status: **NOT STARTED** (re-check after merge)
- Subagent wave status: **NOT STARTED**
- Durable sessions status: **PARTIAL** (in-memory only)
- Background wave status: **NOT STARTED**
- Owner acceptance: **UNRUN** - `docs/PHASE9_ACCEPTANCE.md`