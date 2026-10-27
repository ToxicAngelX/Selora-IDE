# Task: give the agent the editor's context (active file + selection), and stop the UI implying it already has it

You are working in the **Selora IDE** repo: a Rust workspace + Tauri v2 desktop app whose
React frontend lives in `apps/selora-desktop/frontend`. A TypeScript agent (`bash`,
`read_file`, `edit_file`, …) runs in a sidebar "agent dock". Read `AGENTS.md` and
`ARCHITECTURE.md` first; respect whatever invariants they state.

## The bug

The agent's system prompt is assembled in
`apps/selora-desktop/src-tauri/src/sessions.rs` (the `sys_prompt` `format!`, roughly line 474).
It interpolates exactly four things: the workspace root, an approval-mode sentence, a
reasoning-depth sentence, and the user's custom instructions. It contains **no editor state** —
no active buffer path, no selection.

But the UI tells the user otherwise:

- `apps/selora-desktop/frontend/src/layout/AgentDock.tsx:16` — the `SUGGESTIONS` array holds the
  strings `'Explain this file'`, `'Find a bug in the selection'`, `'Summarize the workspace'`.
- `apps/selora-desktop/frontend/src/features/agent/components/PromptSuggestions.tsx:23` —
  `PROMPT_SUGGESTIONS` holds `'Review the active changes for bugs'`,
  `'Generate unit tests with Vitest coverage'`, `'Refactor for clean architecture & modularity'`,
  `'Explain this workspace architecture'`.

Clicking "Explain this file" sends a prompt whose only referent is a file the model cannot see. It
either asks which file, or guesses wrong and writes a confident answer about the wrong file. That
silent wrongness is the actual defect; the missing context is its cause.

Note the state exists but never crosses the boundary: `active_buffer_id` is in the bootstrap
snapshot (`apps/selora-desktop/src-tauri/src/state.rs:1457-1514`,
`crates/selora-protocol/src/lib.rs:855`) and is already read by `StatusBar.tsx:86`,
`EditorRegion.tsx:123`, and `editor-store.ts:51-129`; the last known selection per buffer is
stored by `Workspace::set_selection` (`apps/selora-desktop/src-tauri/src/workspace.rs:322`) into
`BufferState.selection` (`workspace.rs:99`). None of it is passed into `AgentRequestDto`
(`crates/selora-protocol/src/lib.rs:192-211`).

## What to change

### 1. Protocol — carry the reference, not the file

Add one optional field to `AgentRequestDto` in `crates/selora-protocol/src/lib.rs`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub editor_context: Option<EditorContextDto>,
```

and a DTO alongside it:

```rust
/// Where the user is, at the moment the turn was sent. Never the file
/// contents: the agent reads those with `read_file`, so the prompt stays
/// small and the turn sees the same bytes the tools do.
pub struct EditorContextDto {
    /// Workspace-relative path of the active buffer.
    pub path: String,
    /// Language id of the active buffer (e.g. "rust", "typescript").
    pub language: String,
    /// Last known selection, if any. Empty selections are omitted by the sender.
    pub selection: Option<Selection>,
    /// Whether the buffer has unsaved edits at send time.
    pub dirty: bool,
}
```

Rules:
- `serde(default)` + `skip_serializing_if` so old clients and old turn records keep working, and
  so the protocol tests at `crates/selora-protocol/src/lib.rs:1573` and `:2001` can omit it.
- **Do not put file text in this DTO**, and do not read the file on the backend to fill it. Path,
  language, selection, dirty flag. That is the whole payload. If you find yourself wanting
  "just the first 200 lines", that is `read_file`'s job.
- Selection coordinates are already `Selection { anchor, active, version }`
  (`crates/selora-protocol/src/lib.rs:185-189`). Reuse it; do not invent a parallel shape.

### 2. Session prompt — append a delimited block

In `sessions.rs`, after the existing `approval_note` / `depth_instructions` / `custom_sys`
work and inside the same `format!`, append a block only when `payload.editor_context` is present.
Something like:

```
Editor context (as of when the user pressed send; it may have moved since):
Active file: crates/selora-ai/src/agent/turn.rs
Language: rust
Selection: 7:1-7:14
Status: has unsaved edits
Use `read_file` before concluding anything about this file. Do not assume the selection is still
accurate, and do not claim to have read a file you have not read in this turn.
```

Constraints:
- Use the **workspace-relative** path, and say so, so the model passes a path the tools accept.
- Omit the `Selection:` line entirely when there is no selection — never print `undefined`,
  `0:0-0:0`, or an empty range.
- Saying "has unsaved edits" matters: a dirty buffer's on-disk text differs from what the user is
  looking at, and that is precisely the case where the agent contradicts the user.
- The "don't claim to have read a file you haven't" sentence is not padding — with context in the
  prompt, models start answering from the path alone. Keep the instruction.
- Keep the `format!` reading order intact and make sure all placeholders still line up; the current
  one already ends with `{}{}` for depth/custom instructions.

### 3. Frontend — send it, or don't offer it

- `apps/selora-desktop/frontend/src/bridge/bridge.ts:630-670` (`sendAgent`) takes an `options`
  object. Extend it with `editorContext?: EditorContextDto` and forward it on the wire the same way
  `reasoningEffort` is forwarded. Keep the type in sync with `AgentMode`
  (`bridge.ts:160` is `'execute' | 'explore' | 'auto'`).
- Populate it where the prompt is actually dispatched — `AgentDock.tsx` and
  `AgentComposer.tsx` — from the live bootstrap (`active_buffer_id` → buffer → `path`,
  `language`, `selection`) plus the dirty flag, which you can derive by comparing the buffer's text
  against its saved text or from whatever the bootstrap already exposes. Read
  `editor-store.ts` first: it already owns the active-buffer resolution
  (`editor-store.ts:51`, `:88-91`, `:129`) and you should reuse it rather than re-deriving.
- **When there is no active buffer**, do not fabricate context and do not send an empty DTO.

### 4. Make the chips honest

This is the part people will actually notice, so do not skip it because it is "just copy".

- In `AgentDock.tsx`, the chip `'Explain this file'` and `'Find a bug in the selection'` must only
  render when the context they promise exists: an active buffer for the first, a non-empty
  selection for the second. Filter the array against the live snapshot; keep
  `'Summarize the workspace'` always available since it needs nothing.
- In `PromptSuggestions.tsx`, `'Review the active changes for bugs'` promises git state the agent
  has no tool for. Either reword it to what the agent can actually do (e.g. "review the workspace
  for bugs" — it has `search_files` and `read_file`), or drop it. Same test for
  `'Generate unit tests with Vitest coverage'`: the agent can write tests, but "coverage" implies
  running a coverage tool, which the manifest does not have. Prefer wording that describes the
  agent's real capability over wording that sounds impressive.
- If the composer is reachable with a prompt that references a file and no file is active, that is
  acceptable — the agent will ask. What is not acceptable is a button that guarantees the
  ambiguity.

### 5. Tests

Every existing suite must stay green, and these cases need coverage:

- **Protocol round-trip**: `AgentRequestDto` with `editor_context` present serializes/deserializes;
  and an old payload without the key still deserializes (`editor_context == None`).
- **Prompt assembly** (Rust, in `sessions.rs`'s test module — there are already turn tests around
  `:1625`, `:2353`, `:2529`): context present → the prompt contains the relative path and, when a
  selection exists, its range; context absent → the prompt is byte-identical to today's.
- **Empty selection** → no `Selection:` line, not an empty range.
- **Chips** (`AgentComposer.test.tsx`, `ThinkingOrb.test.tsx:80-84` already render
  `PromptSuggestions`): no active buffer → "Explain this file" is absent; selection → present.
- **Bridge** (`bridge.test.ts:130-156` already asserts `reasoning_effort` forwarding): assert
  `editor_context` is forwarded when supplied and omitted when not.

### 6. Verify before you report

Run and make green:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/selora-desktop/frontend; npm test
```

Frontend has 465 tests and Rust has 1121 as of the last recorded status in `STATUS.md`; a number
that drops is a test you deleted, not a test that got faster.

## Out of scope

Do not: change the model, the tool manifest, or the approval flow; add a whole-file "context
attachment" system; touch the egui shell's `spawn_agent` path in `crates/selora-ui`
(`crates/selora-editor` has its own agent spawn per `AGENT_ORCHESTRATION.md §17.6`) unless the
protocol change forces it — if it does, say so in the report rather than silently editing it.
Do not update `STATUS.md`/`ROADMAP.md` from "in progress" to "done" without the commands above
actually passing.

## Report back

State, in order: what you changed per file; the exact before/after of the prompt block; which
chips are now conditional and on what; test counts before and after; and anything you chose not
to do, with the reason. If you decided a suggestion chip should be deleted rather than reworded,
say which and why — that is a product decision worth surfacing, not hiding.
