# Phase 8 — Workflows: the contract in this tree

Status: **wave 3 automated work complete, 2026-10-27; owner acceptance pending.** Wave 1 landed the engine
(`crates/selora-workflow`: sandbox, facade, schema, journal/resume, budget,
scheduler), the protocol (`SurfaceId::Workflows`, the `workflow_*` commands and
events) and the UI (activity route, panel, confirmation card). Wave 2 is the
**shell wiring**: a real `Host` over the agent loop/bash spawn/`selora-fs`/
`selora-git`, the `state.rs` dispatch arms, the event adapter into the panel
store, and mounting the card. Wave 2 is integrated. The tables below record
earlier implementation snapshots; `STATUS.md` is the current restart point.
This document is the Phase 8 contract *as realized in this tree*, and it is
deliberately blunt about the gap between the contract and what runs today. The
authoritative sources are:

- `ROADMAP.md` **§Phase 8 — Workflows (dynamic multi-agent orchestration)**
  (goal, ZCode→Selora mapping table, decisions/tradeoffs, tasks **8a–8d**,
  acceptance tests 1–9, "DONE means").
- `ARCHITECTURE.md` **§18** — §18.1 sandbox and caps, §18.2 the facade,
  §18.3 subagents/gates/journal, §18.4 process model and wiring.
- `ARCHITECTURE.md` **§17.3/§17.4/§17.5** — per-ask `TurnCaps`, the approval
  park, the bash spawn the gates reuse.
- `UI_SPEC.md` **§8/§9/§10** — the activity-driven panel ("region ④ when the
  activity is selected"). In the Z2 grid that region is the **primary sidebar**
  (`docs/ui/PHASE_Z2_DESIGN.md` §2 row 3, §3), not the bottom dock.
- `ROADMAP.md` **§Phase Z → Extension points for Phase 8 / Phase 9** — new work
  is additive `FrontendCommand`/`FrontendEvent` variants plus a `state.rs`
  dispatch arm, never a new Tauri IPC command.
- `ROADMAP.md` **§Phase 9** — the subagent runtime is shared; whichever phase
  lands first owns it.

Nothing below overrides those documents. Where this tree differs, the
difference is called out.

## 1. What actually runs in this tree

| Piece | Where | State |
|---|---|---|
| Activity item + route (`workflows`) | `src/layout/ActivityBar.tsx` | **Implemented this slice.** Native button, `aria-label="Workflows"`, `aria-pressed` selected state, `play` icon from `components/Icon.tsx` (no new icon library), no shortcut hint (no keybind exists). |
| Sidebar mount (lazy) | `src/layout/PrimarySidebar.tsx` | **Implemented this slice.** `workflows` renders `<WorkflowsSidebarView>`, which lazily resolves the panel module inside `Suspense`, with an error boundary and an explicit unavailable fallback. |
| Panel + its store + CSS | `src/features/workflows/WorkflowsPanel.tsx`, `workflows-store.ts`, `workflows.css` | **Live event wiring.** The bridge adapter feeds the store; persisted-run discovery/bootstrap hydration remains pending. |
| Engine core (8a) | `crates/selora-workflow` | **Landed.** Sandbox/facade/schema/limits and confirmation are implemented and tested. The crate is a root workspace member and the desktop Host consumes it. |
| Orchestration (8b) | `crates/selora-workflow` (`journal.rs`, `budget.rs`, `subagent.rs`) | **Landed in the crate, not on the run path.** Journal + `ResumePlan`, the budget, and the named-agent scheduler are implemented and tested; `run_workflow` does not accept or write a journal, and the facade calls `Host::ask` directly rather than the scheduler. The shell has to record/replay. |
| `run_workflow` tool, confirmation card, shell arms (8c) | `src/features/agent/components/WorkflowCard.tsx`, `src/bridge/bridge.ts` | **Run/stop wired.** The card is mounted in the transcript, approving dispatches the displayed spec, and all seven workflow event kinds feed the panel. Wave 3 extends resume/decision behavior. |
| Protocol `SurfaceId` | `src/bridge/bridge.ts:24`, `crates/selora-protocol/src/lib.rs:249` | **Done.** `Workflows` exists in both enums, `ACTIVITY_VIEWS` includes it, and the panel route round-trips `ToggleSurface("workflows")`. |

Verify rather than trust: `cd crates/selora-workflow && cargo test` and, in
`apps/selora-desktop/frontend`, `npm run typecheck && npx vitest run src/layout`.

## 2. The facade (ARCHITECTURE §18.2)

A workflow script sees exactly this and nothing else. There is no `fetch`, no
`fs`, no `process`, no timers, no `Date`, no ambient standard library — those
identifiers **do not exist** in the realm (ARCH §18.1). A script that reaches
for them fails at the facade; that is the security property, not a lint.

```js
agent(name?, persona?) → { ask(type?, instructions) → result }

phase(name)                       // marker; the panel's graph is built from these
world.run(cmd, args, {timeoutMs?}) → {exitCode, stdout, stderr}
files.glob(pattern) / files.grep(pattern, glob?) / files.read(path)
git.changedFiles(base?) / git.diff(base?, path?) / git.status() / git.log(count?)
report(item, artifactId?)         // progressive results, ≤256 items, ≤32 KB each
artifact.markdown(id, content, opts) / artifact.file(id, path, opts)
log(message)
```

The current Boa facade is blocking. `await` inside an async function can consume
these plain values, but `Promise.all` does not make these host calls concurrent.
Top-level `await` is unsupported. True script-level fan-out remains a gap even
when the host's scheduler supports concurrent callers.

Rules that make the facade trustworthy:

- **`ask` is validated, never coerced.** The script declares a JSON-schema-ish
  type; a result that does not match fails the run and names the offending
  field (ARCH §18.2).
- **`world.run` is gated on the confirmed command set.** The command must be a
  literal from the set extracted at confirmation time; a command assembled at
  runtime fails loudly instead of widening the grant.
- **`files`/`git` are read-only and cap-reject** rather than truncate.
- **Agent names are identities**, unique within a run and the key resume
  matches steps by.

## 3. Caps (ARCHITECTURE §18.1, §18.3; ROADMAP Phase 8 decisions)

| Cap | Value / owner | Notes |
|---|---|---|
| Script size | ≤ **64 KB** (ARCH §18.1) | Enforced by the engine before a realm exists. |
| Steps | ≤ **256** (ARCH §18.1) | Every JS-visible facade call is a `StepKind` and counts against the cap. |
| Per-ask turn caps | §17.3 `TurnCaps` | Unchanged: a subagent ask is an ordinary §17 loop turn. |
| Run cost | `workflow.max_cost_usd`, default **$5.00** (ARCH §18.1, ROADMAP Phase 8) | Checked **between every step**; the cap is inclusive, the first charge over it aborts the run before the next step. The engine exports this as `DEFAULT_MAX_COST_USD`; the UI must read the run's max, never hard-code `5`. |
| Concurrency | `workflow.max_concurrency`, default **4** (ARCH §18.3) | Separate agents run concurrently; asks on one agent queue FIFO. |
| Reports | ≤256 items, ≤32 KB each (ARCH §18.2) | Panel-side store bounds its log separately (`WORKFLOW_LOG_MAX`, 500). |

The shell must not restate these numbers as UI literals: the confirmation card
and the panel read the run's own limits. A budget bar rendered from a guessed
constant would be a lie the moment settings change.

## 4. The confirmation card

ROADMAP Phase 8 → 8c: a workflow run starts **only** from a user-approved
confirmation payload (a new §4.2 chat card type). It shows:

1. the **full script text** (what will execute),
2. the **extracted `world.run` command set** (what will touch the machine),
3. the **cost cap** for the run.

Approve starts the run; deny (or never approving) runs nothing. Approving a
script **never pre-approves a write**: a subagent's write/bash still parks on an
ordinary §17.4 approval card, and the Workflows panel renders the same card
inline. That separation is the whole point of the design (ROADMAP Phase 8
decisions table).

The transcript mounts the card and Approve dispatches `workflow_run`. The desktop
revalidates the script and extracted commands independently of model-supplied
arguments. The router's tool result acknowledges confirmation, not completion.

## 5. Journal and resume

**Path (contract):** `.selora/workflow-runs/{run_id}/journal.jsonl` — append-only
JSONL, inside the workspace, gitignored. The crate's `journal` module defines
`JOURNAL_DIR`, `JOURNAL_FILE`, `run_dir(workspace, run_id)` and
`journal_path(workspace, run_id)`.

- Every step, `ask` result, `world.run` outcome and world read is appended
  **before the run proceeds** (ARCH §18.3). The journal is the run's memory, the
  durable record behind the panel's live view.
- **Resume** fits the journal to a plan (`resume_plan()`): world reads replay
  **byte-identically**, finished asks are **not re-paid**, unfinished steps
  **re-dispatch**. Crash-resume equivalence is a roadmap test (8b), not a hope.
- A crash or a Stop leaves the journal on disk: the run stays inspectable and
  resumable.
- **Amend/cache** (re-running a revised script while importing unchanged named
  asks) is **v1.x**, explicitly not v1 (ROADMAP Phase 8 "v1.x (named, not v1)").

Open item in this tree: the repository `.gitignore` now carries `/.selora/`, and
`Journal::open` seeds `.selora/.gitignore` with `*`; the fixture in
`crates/selora-workflow/tests/phase8_acceptance.rs` asserts both the journal path
and that seed. The remaining half of acceptance step 8 is that **the app** must
write journals, and it does not yet — `run_workflow` has no journal writer on
its call path.

## 6. Panel regions (ROADMAP 8d, ARCHITECTURE §18.4)

The panel is the activity-driven sidebar. Its regions, all present in
`WorkflowsPanel.tsx`:

| Region | Content | Empty state |
|---|---|---|
| Run list | `<nav aria-label="Workflow runs">`, one button per run with status + USD | none (whole-panel empty state) |
| Run summary | run name, start time, tokens, USD spent / cap, budget `progressbar` | — |
| Phase graph | one node per `phase()`, `data-status`, current phase marked | "No phases reported yet." |
| Steps | one row per step with phase/agent chips, error text | "No steps reported yet." |
| Approvals | inline approve/deny cards (§17.4 semantics) | "No approvals requested." |
| Reports | progressive `report()` items | "No reports filed yet." |
| Artifacts | markdown + file cards | "No artifacts produced yet." |
| Run log | `role="log"`, `aria-live="polite"`, level-coloured lines | "No log entries yet." |
| Stop | header button; kills the run and its in-flight bash children (contract) | disabled until wired |

With an empty store the panel shows **"No workflow runs"** and invents nothing.
`Stop` is disabled while `stop_wired` is false and approve/deny while
`approval_wired` is false. Bootstrap installs both transports for live runs.

## 7. The shell route (this slice)

- **Route id:** `workflows`. It is declared once, as
  `WORKFLOWS_SURFACE` in `src/layout/ActivityBar.tsx`, and reused by the
  sidebar mount.
- **Selection:** identical to the Z2 views — the click issues `ToggleSurface`
  and the view changes only when the validating `workspace_layout_changed`
  layout arrives. Selected state is
  `primary_sidebar_open && activity_view === 'workflows'`.
- **Interaction:** a real `<button type="button">` (tab stop; Enter/Space
  activate), `aria-label="Workflows"`, `aria-pressed`, `is-selected` class,
  hover tooltip. No `aria-keyshortcuts` and no tooltip key hint, because no
  keybind ships with this slice.
- **Lazy mount:** `PrimarySidebar` resolves the panel module via a Vite glob
  (so an absent or renamed module cannot break the shell), wraps it in an error
  boundary, and shows either a loading status or a readable "not available in
  this build" fallback with the module path. The panel chunk is requested only
  while the route is active.
- **Existing behaviour untouched:** Activities, Agent, Files/Search/Git/Settings
  and the badge logic are unchanged; the sidebar's other routes render exactly
  as before.
- **Tests:** `src/layout/ChromeBars.test.tsx` (item, label, selected state,
  keyboard reachability, toggle call), `src/layout/ShellGrid.test.tsx` (list
  order, route switch, lazy panel mounted, other views still switch),
  `src/layout/PrimarySidebar.test.tsx` (mount, empty state, export contract).

## 8. Mismatches to close (explicit)

1. ~~**`SurfaceId` has no `workflows` member**~~ — **closed by the protocol
   slice.** Both enums carry it, `ACTIVITY_VIEWS` accepts it, and the
   `ToggleSurface("workflows")` round-trip is no longer an assertion.
2. **Panel props differ from the shell's assumption.** `WorkflowsPanel` takes
   `{ store?: WorkflowsStore }` and defaults to its module singleton; the shell
   passes `{ bootstrap }`, which React ignores. No props are *required* on
   either side today, so nothing breaks — but no data flows either.
3. **Two `WorkflowRun` shapes.** `bridge.ts` describes the wire DTO
   (`phases`/`steps`/`reports`/`artifacts`), while the panel's store describes
   its own `WorkflowRun` (cost, tokens, log, approvals, budget). There is no
   adapter between them; the event → store reducer belongs to the wiring slice.
4. **No store hydration.** The panel's store starts empty; nothing forwards
   `workflow_*` events (or a bootstrap snapshot) into it. Empty state is the
   honest current output.
5. **Actions are inert by design.** `stop_wired` / `approval_wired` are false
   until the shell handlers exist; the buttons are disabled rather than
   pretending.
6. **No keybind.** `commands.ts` has no workflow command, so the activity has no
   shortcut and the tooltip shows none.

## 9. Not implemented yet

Explicitly **not done** in this tree — do not read the panel or this document as
evidence of a working engine:

- Script execution end-to-end **in the app**: the engine runs standalone and is
  green, but there is no `Host` implementation in `src-tauri`, so no run starts.
  8b's scheduler, gates, budget and resume are implemented in the crate; only
  the facade path (`phase`/`ask`/`world.run`/reads/report/artifact) is on the
  engine's run path, and resume is a plan the shell must consult, not an engine
  parameter.
- Manifest v2's `run_workflow` tool and the Rust `state.rs` dispatch arms; the
  chat confirmation card component exists but is not mounted.
- Cost receipts / meters fed by real asks; the panel can render a budget bar,
  but no run has a real cost today.
- Journal writing from the app, the resume offer on relaunch, crash-resume
  equivalence (the journal/resume contracts are tested; nothing calls them from
  the shell).
- Chat-side approval cards for workflow subagents; the panel's inline cards are
  store-driven only.
- Amend/cache, dashboards, escalation questions, skills library — all v1.x or
  later (ROADMAP Phase 8).

## 10. Wave 2 wiring status (2026-10-27)

Wave 1 is the engine + protocol + UI. Wave 2 is everything between them. This
table is the current tree, verified by reading the call sites, not by intent:

| Wiring piece | Contract | State | Evidence in this tree |
|---|---|---|---|
| **Host** (`impl Host for …`) | Bridge `Host::ask`/`world_run`/`files`/`git`/`report`/`artifact` to the Phase 5 agent loop, the §17.5 spawn, `selora-fs` and `selora-git`; `cost_usd` reads the run's receipts. | **Pending** | `apps/selora-desktop/src-tauri/Cargo.toml` declares `selora-workflow`, but there is no `impl Host` under `src-tauri/src` and no call to `run_workflow`. |
| **Dispatch** (`FrontendCommand::Workflow*`) | `workflow_run` starts an approved spec; `_resume` re-opens a journal; `_stop` kills the run + children; `_decision` answers a parked approval. | **Pending** | `state.rs` handles only `SurfaceId::Workflows` in `apply_toggle` (layout); every workflow command falls into `other => Err(unsupported(...))`. |
| **Event adapter** (`FrontendEvent::Workflow*` → panel store) | Seven events map one-for-one onto `workflowsStore.applyEvent`; `run_id` routes; bootstrap/replay is idempotent. | **Pending** | `bridge.ts` has `isWorkflowEvent` + `workflowEventRunId` and the store has `applyEvent`, but `app-store.ts` never calls them and there is no bootstrap hydration. The panel test drives the store directly. |
| **Card payload** | `run_workflow` parks on a §4.2 card carrying the script text, the extracted command set and the cost cap; Approve is `workflow_run`, Deny resumes the loop. | **Done** (payload; mount pending) | `WorkflowSpecDto` (protocol), `WorkflowSpec` (bridge) and `WorkflowCard` props agree on name/script/commands/`maxCostUsd`; `WorkflowCard.test.tsx` pins all three. Nothing renders it in the transcript yet. |
| **Root workspace membership** | The crate builds with the app instead of as a stray standalone workspace. | **Done** | Root `Cargo.toml` lists `crates/selora-workflow` as a member and as a workspace dependency; `src-tauri/Cargo.toml` depends on it. Membership landed before consumption. |

Wave 2 test coverage added with this slice:
`crates/selora-workflow/tests/phase8_acceptance.rs` runs a realistic
`phase → files.glob → agent('reviewer').ask → world.run('git') → report →
artifact.markdown → phase` script against a fake `Host` and asserts the exact
event order, the report/artifact caps and the command-set gate. A second
fixture journals a real run, reloads the journal, and asserts the `ResumePlan`
replay contract (finished ask not re-asked, world read replayed
byte-identically, only the unfinished step re-dispatched).

Run it with:

```bash
cargo test --manifest-path crates/selora-workflow/Cargo.toml --test phase8_acceptance
```

### 10b. Wave 2 completion addendum (2026-10-27, later in the session)

The §10 table was written before the wave-2 agents landed. Current state:

| Piece | State | Evidence |
|---|---|---|
| Host | **Done (read-only ask)** | `apps/selora-desktop/src-tauri/src/workflows_host.rs` implements all 15 `Host` methods: root-contained files, read-only Git, env-stripped PowerShell `world.run` with timeout/cancel, non-streaming `ask`, reports/artifacts/logs/cost. Subagent tools/writes are wave 3. |
| Dispatch | **Done (run/stop)** | `state.rs` validates with `prepare`, runs the engine on `spawn_blocking`, maps every engine event to `FrontendEvent::Workflow*`, emits `WorkflowFinished` once, cancels via the host. `Resume` and workflow-card `Decision` are typed unavailable. |
| Event adapter | **Done** | `workflows-bridge.ts` maps seven events and installs Stop/Approve/Deny; `app-store.ts` routes workflow events through the gate; `workflow_started.request_id` is recorded so Stop addresses the run. |
| Card payload + mount | **Done** | `ApprovalNeededDto.workflow` carries name/script/commands/cap; `ToolCard` renders `WorkflowCard`; approving sends `workflow_run` with the displayed spec, then `agent_decision`. |
| Root membership | **Done** | `selora-workflow` is a root workspace member; the 1045-test root gate includes its 56 unit + 2 integration tests. |

### 10c. Wave 3 completion addendum

- Host asks are real §17 agent turns with read/write/bash/web tools; every bash
  command and every write parks on an explicit card. `run_workflow` cannot be
  re-entered, and session full-auto is never inherited.
- Ask receipts accumulate cost and tokens even on failed/stopped turns.
- Durable runs: confirmed spec + index-keyed journal + exclusive lock. Completed
  asks and world reads replay without host IO; interrupted commands fail closed;
  ask checkpoints restore full tool history before a follow-up.
- Bootstrap lists non-completed durable runs and the panel offers Resume only
  for safe ones. Completed runs are never re-offered.
### 10d. Wave 4 closure

- `world.run` now owns a Windows Job Object (`workflows_process.rs`) with
  `KILL_ON_JOB_CLOSE`; timeout, cancel and normal exit terminate the tree. The
  child is assigned immediately after `CreateProcess`, so descendants created in
  that sub-startup window can still escape.
- Resume preflights unfinished asks. Default resume returns
  `workflow_incomplete_asks` without starting a worker; the panel shows
  “Resume anyway”, and only that explicit click sets `allow_incomplete_asks`.
- Terminal workflow events refresh clean editor buffers and emit
  `BufferSnapshot`; dirty buffers are preserved with a toast. Persisted steps
  carry no phase, and LSP documents are not re-sent after a reload.
- Bootstrap loads completed-run history from the terminal journal report.
  Reports reconstruct only from successful parsable report steps; phase per step
  is unknown.
- Release EXE and an unsigned installer were built; see
  `docs/PHASE8_ACCEPTANCE.md`. Owner acceptance steps remain unrun.

### 10e. Partial tool effects (P2 gap 2)

- Every workflow ask's side-effecting tool (`write_file`, `edit_file`,
  `delete_file`, `bash`) writes a per-run ledger at
  `.selora/workflow-runs/{run_id}/tool-effects.jsonl`: `started` before the
  tool executes, `finished` after it returns. Both records carry the same
  `effect_id`; only the redacted argument summary (paths and byte counts) is
  stored, never file bodies, command text, or secrets.
- A crash leaves `started` without `finished`. Bootstrap then marks the run
  `resumable=false` with `partial_effects=true`, and Resume returns
  `workflow_partial_effects` before starting a worker. The panel shows a
  separate acknowledgement action ("Acknowledge and resume anyway"); only that
  click sets `acknowledge_partial_effects`.
- Completed asks' paired effects do not block. A read-only unfinished ask keeps
  ordinary Resume. A corrupt ledger is a typed refusal
  (`workflow_effect_ledger_corrupt`), never treated as empty.

## 11. Acceptance checklist (ROADMAP Phase 8, "Acceptance tests")

These are the roadmap's acceptance tests, verbatim. They are **owner-run**:
they need a real gateway key, a real workspace, and a human approving, denying,
killing and relaunching the app.

```bash
# 1. Chat: "review every Rust file in src/ for unwrap() calls; report the worst five"
#    → the agent proposes a run_workflow script; the confirmation card shows it + its commands
# 2. Approve → the Workflows panel shows the phase graph; subagents fan out; cards appear
# 3. A subagent proposes an edit → an approval card parks the run; Deny → the run continues
# 4. Kill the app mid-run → relaunch → the run offers Resume and finishes correctly
# 5. A gate (`world.run cargo test`) fails → the script's fix loop round is visible in the panel
# 6. Stop mid-run → run ends within seconds; in-flight bash children are dead
# 7. The cost meter reflects every ask; the run summary shows tokens + USD per phase
# 8. The journal lands under .selora/workflow-runs/ (never in git; .gitignore updated)
# 9. A script that tries `fetch`/`fs`/`process` fails at the facade — structurally absent
```

**Owner-only steps: 1–7.** Each needs a real key/workspace and a human in the
loop (approve, deny, kill, relaunch, Stop, cost check). **Step 8** is also an
owner run, but its repo half is automated (`.gitignore` + `Journal::open`
seeding + the fixture's path assertion). **Step 9 is automated now**: the
sandbox tests prove `fetch`/`fs`/`process` are structurally absent; the owner
only confirms it in a packaged build.

| # | Acceptance step | Today |
|---|---|---|
| 1 | Chat proposes a script; the confirmation card shows it + commands | card payload done; not mounted, no dispatch arm — **blocked** |
| 2 | Approve → panel shows the phase graph; subagents fan out | panel renders a graph once a store has runs; no Host/event adapter — **blocked** |
| 3 | A subagent edit parks the run; Deny continues | no Host/approval channel on the run path — **blocked** |
| 4 | Kill mid-run → relaunch → Resume finishes | `ResumePlan` tested; engine/API has no resume parameter and the app writes no journal — **blocked** |
| 5 | Failing gate visible as a fix round | engine gate + exit code tested; no live panel feed — **blocked** |
| 6 | Stop ends the run and its bash children | button exists, disabled (`stop_wired: false`); no run handle to kill — **blocked** |
| 7 | Cost meter reflects every ask | store renders cost/tokens from events; no receipts feed them — **contract only** |
| 8 | Journal under `.selora/workflow-runs/`, never in git | `.gitignore` + path/seed tested; app does not write journals — **half done** |
| 9 | `fetch`/`fs`/`process` fail structurally | engine tests green (`ambient_globals_are_structurally_absent`) — **done engine-side** |

The wave 2 wiring (Host, dispatch, adapter, card mount) is what moves rows 1–7
and the app half of row 8. Until then, Phase 8's "DONE means" is **not**
satisfied even though the engine suite is green.
