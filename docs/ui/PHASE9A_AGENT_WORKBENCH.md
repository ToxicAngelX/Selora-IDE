# Phase 9a — Agent Workbench (frontend contract)

Status: **landed (frontend)**. Runtime tasks 9b–9e are still separate slices.

The dedicated Agent surface is `AgentDock` (`src/layout/AgentDock.tsx`) mounted
by `ShellGrid` in `data-view-mode="agent"` (full height beside the activity
bar) and in split mode. The same component is used in both; only the shell's
grid changes. The title-bar Agent pill and the activity-bar Agent button enter
the dedicated view, and the dock header exposes **Editor** / **Split** exits.

## Components

| Component | Responsibility |
|---|---|
| `layout/AgentDock.tsx` | Workbench shell: session tabs, transcript, live todo spine, Chat/Plan views, composer, header Stop, exit switches. |
| `features/agent/components/AgentSessionTabs.tsx` | WAI-ARIA tabs over `bootstrap.chat.sessions`; New and select dispatch the real commands. |
| `features/agent/components/AgentTodoSpine.tsx` | Read-only authoritative todo list + progress; says `No todos yet` when absent. |
| `features/agent/components/AgentQuestionCard.tsx` | Structured question (options + optional free text) on a parked approval. |
| `features/agent/components/AgentPlanCard.tsx` | Plan steps + Approve / Request changes on a parked approval. |
| `features/agent/components/ToolCard.tsx` | Reuses the existing transcript card; renders the two decision cards when the payload is present. |
| `features/agent/components/AgentComposer.tsx` | Ask / Agent / Plan row mapped to real session permission policies. |

Transcript rendering is **not** forked: `AgentExecutionFeed`, `ToolCard`,
`ReasoningAccordion`, `MarkdownMessage`, prompt queue, and Rewind/Fork all keep
their existing behavior.

## Session contract

Sessions are never synthesized on the frontend.

- Hydrate reads `BootstrapState.chat.sessions` into `agentStore.sessions`, and
  keeps each session's transcript and todos in `transcripts` / `todosBySession`
  so a tab switch shows that session's state.
- New → `FrontendCommand::NewSession` (`agentStore.requestNewSession`).
- Select → `FrontendCommand::SwitchSession { session_id }`
  (`agentStore.requestSwitchSession`).
- The current Rust build does not route these variants yet. It answers
  `unsupported_phase_slice`; the UI shows the exact message in the tab strip
  and keeps the session list unchanged. No placeholder session is created.
- On success the authoritative `bootstrap_state` event rehydrates the rail
  (`app-store` already calls `agentStore.hydrate` for it).

## Additive protocol shapes (seam for 9b/9c)

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

Rust's `FrontendCommand::AgentDecision` currently ignores unknown fields, so a
pre-9b build still records the permit/deny and the turn continues. 9b/9c should
promote `resolution` into the typed command and emit `pending_approval`
(`ApprovalDto.decision_prompt`) on bootstrap so a parked question survives a
reload with its card.

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
`update_plan` tool event. Markdown checklists in model prose and prompt-derived
steps are deliberately **not** turned into todos.

## Accessibility / motion

- Session tabs: `tablist`/`tab`/`tabpanel`, roving tabindex, Left/Right/Home/End
  activate the backend session.
- The composer mode row is a labelled group of `aria-pressed` buttons; the
  permission dropdown still owns the detailed policy copy.
- Question options are native radios; free text is a labelled textarea; empty
  answers cannot be sent. Plan changes require a revision note.
- Stop is a real header button with an accessible name and a disabled idle state.
- Focus rings use `--border-focus`; colors/spacing/radii read tokens only.
- Reduced motion is inherited from `base.css` (`prefers-reduced-motion` and the
  shell's `data-reduced-motion`); this slice adds no new loops.

## Gates

```bash
cd apps/selora-desktop/frontend
npm ci
npm run typecheck
npm test
npm run build
```

Headless coverage: `features/agent/AgentWorkbench.test.tsx` (new/select,
unsupported fallback, keyboard tabs, todo empty/live, question option/free text,
plan approve/request changes, Stop, mode row), plus
`layout/AgentDock.test.tsx`, `features/agent/AgentDock.test.tsx`, and
`features/agent/components/AgentSessionRail.test.tsx`.

## Gaps

- `NewSession` / `SwitchSession` backend routing (9d) — UI is ready and falls
  back with a truthful message.
- `ChatStateDto.todos` and `ApprovalDto.decision_prompt` (9b/9c) — additive
  types are declared; hydrate consumes them when present.
- Plan execution gating lives in 9b; this card only records the user's answer
  through the existing decision command.
- Token/cost badges on sessions are omitted until `SessionDto` reports them.