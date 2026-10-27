# Phase 9a - Agent window (frontend contract)

> **STATUS: FINALIZATION SKELETON (2026-10-27).**
> Implementation lives on branch `selora-p3-window`. Merge state:
> `TODO-VERIFY`. This worktree does not contain the branch.
> 9a is a frontend contract only. It does **not** gate writes, spawn
> subagents, persist sessions, or compact context.
>
> Re-verify every fallback against the merged backend before finalizing. The
> branch was written before the backend session slice merged, so its
> `unsupported_phase_slice` note may be stale. The branch also carries a
> `docs/ui/PHASE9A_AGENT_WORKBENCH.md`; resolve that merge conflict in favor of
> this skeleton, then fill the fields below.

## 1. Surface

The dedicated Agent surface is `AgentDock`
(`frontend/src/layout/AgentDock.tsx`), mounted by `ShellGrid` in
`data-view-mode="agent"` (full height beside the activity bar) and in split
mode. The same component is used in both; only the shell grid changes. The
title-bar Agent pill and the activity-bar Agent button enter the view, and the
dock header exposes **Editor** / **Split** exits.

## 2. Components

| Component | Responsibility |
|---|---|
| `layout/AgentDock.tsx` | Workbench shell: session tabs, transcript, live todo spine, Chat/Plan views, composer, header Stop, exit switches |
| `features/agent/components/AgentSessionTabs.tsx` | WAI-ARIA tabs over `bootstrap.chat.sessions`; New and select dispatch the real commands |
| `features/agent/components/AgentTodoSpine.tsx` | Read-only authoritative todo list + progress; says `No todos yet` when absent |
| `features/agent/components/AgentQuestionCard.tsx` | Structured question (options + optional free text) on a parked approval |
| `features/agent/components/AgentPlanCard.tsx` | Plan steps + Approve / Request changes on a parked approval |
| `features/agent/components/ToolCard.tsx` | Reuses the existing transcript card; renders the two decision cards when the payload is present |
| `features/agent/components/AgentComposer.tsx` | Ask / Agent / Plan row mapped to real session permission policies |

Transcript rendering is not forked: `AgentExecutionFeed`, `ToolCard`,
`ReasoningAccordion`, `MarkdownMessage`, prompt queue, and Rewind/Fork keep
their existing behavior.

## 3. Session contract

Sessions are never synthesized on the frontend.

- Hydrate reads `BootstrapState.chat.sessions` into `agentStore.sessions`, and
  keeps each session's transcript and todos in `transcripts` /
  `todosBySession`, so a tab switch shows that session's state.
- New -> `FrontendCommand::NewSession` (`agentStore.requestNewSession`).
- Select -> `FrontendCommand::SwitchSession { session_id }`
  (`agentStore.requestSwitchSession`).
- Against the base tree the backend answers `unsupported_phase_slice`; the UI
  shows the exact message in the tab strip and keeps the session list
  unchanged. No placeholder session is created.
- The `selora-wt-session` branch implements both commands and emits an
  authoritative `bootstrap_state`. **Finalization:** verify the success path and
  delete or rewrite the fallback paragraph if it no longer applies.
- On success the authoritative `bootstrap_state` event rehydrates the rail
  (`app-store` calls `agentStore.hydrate`).

## 4. Additive protocol shapes (seam for 9b/9c)

`approval_needed` keeps the existing park/decision route. A question or plan
adds one optional field to `ApprovalNeededPayload`:

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

The answer rides the existing `AgentDecision` command as an optional
`resolution`:

```ts
type AgentDecisionResolution =
  | { kind: 'question'; option_id?: string; text?: string }
  | {
      kind: 'plan';
      decision: 'approve' | 'request_changes';
      feedback?: string;
    };
```

The pre-9b Rust command ignores unknown fields, so it records the permit/deny
and the turn continues. 9b/9c must promote `resolution` into the typed command
and emit `pending_approval` (`ApprovalDto.decision_prompt`) on bootstrap so a
parked question survives a reload with its card.

Todos use a second additive field on `ChatStateDto`:

```ts
chat.todos?: {
  id: string;
  session_id?: string;
  title: string;
  status: 'pending' | 'in_progress' | 'completed';
  assigned_subagent?: string;
}[];
```

Until 9b writes it, the only live source is the real `manage_todo` /
`update_plan` tool event. Markdown checklists in model prose and
prompt-derived steps are deliberately **not** turned into todos.

## 5. What 9a does not do

- It does **not** enforce plan mode. The plan card records an answer; the
  router does not reject writes during planning.
- It does **not** spawn subagents or ask structured questions from the runtime.
- It does **not** persist sessions, resume after restart, or compact context.
- It does **not** add a persisted auto-approve flag.
- It does **not** own `todo_read` / `todo_write`; the spine is read-only.

Plan gating and the effects wave are 9b. Subagents and escalations are 9c.
Durable sessions and compaction are 9d. Background runs are 9e.

## 6. Accessibility / motion

- Session tabs: `tablist` / `tab` / `tabpanel`, roving tabindex,
  Left/Right/Home/End activate the backend session.
- The composer mode row is a labelled group of `aria-pressed` buttons; the
  permission dropdown still owns the detailed policy copy.
- Question options are native radios; free text is a labelled textarea; empty
  answers cannot be sent. Plan changes require a revision note.
- Stop is a real header button with an accessible name and a disabled idle
  state.
- Focus rings use `--border-focus`; colors, spacing, and radii read tokens only.
- Reduced motion is inherited from `base.css` (`prefers-reduced-motion` and the
  shell's `data-reduced-motion`); this slice adds no new loops.

## 7. Gates

```powershell
cd apps/selora-desktop/frontend
npm ci
npm run typecheck
npm test
npm run build
```

Headless coverage on the branch: `AgentWorkbench.test.tsx` (new/select,
unsupported fallback, keyboard tabs, todo empty/live, question option/free text,
plan approve/request changes, Stop, mode row), plus `AgentDock.test.tsx`,
`features/agent/AgentDock.test.tsx`, and
`features/agent/components/AgentSessionRail.test.tsx`.

Final counts: `TODO-VERIFY` after merge. Do not copy a count into a release
record without the gate log.

## 8. Gaps to reconcile at finalization

- NewSession / SwitchSession backend routing (session branch) vs the 9a
  fallback.
- `ChatStateDto.todos` and `ApprovalDto.decision_prompt` typed in Rust
  (9b/9c); hydrate must consume them when present.
- Plan execution gating (9b) and approved writes (effects wave).
- Subagent runtime and question cards (9c).
- Durable sessions, resume, compaction, auto-approve (9d).
- Background runs and notifications (9e).
- Token/cost badges on sessions until `SessionDto` reports them.
- Any fabricated roster, fake branch, or local todo editing removed by the
  session slice must stay removed after the merge.

## 9. Finalization fields

- Branch merge commit: `TODO-VERIFY`
- Frontend typecheck / test / build: `TODO-VERIFY`
- NewSession / SwitchSession success path: `TODO-VERIFY`
- Fallback text still accurate: `TODO-VERIFY`
- Owner visual acceptance: **UNRUN**
