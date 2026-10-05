# Selora IDE — Agent Orchestration

This document tells the executing AI (Kimi K3, GLM 5.3, Claude with subagents) **exactly** how to split the Lite Edition build into subagent runs, what to run in parallel, what to run sequentially, and how to verify each merge before the next phase begins.

The default mode is: **one human orchestrator, many parallel subagents, one merge gate between every phase.** Subagents are spawned, given a brief, allowed to work, and their output is reviewed against the contract before the next phase starts.

---

## 1. The orchestrator's job

The orchestrator is the human (or a single designated long-context agent) who:

1. Reads this plan pack in full before starting.
2. Spawns subagents with the verbatim briefs from `ROADMAP.md`.
3. Reviews each subagent's output against the brief and the architecture.
4. Runs the merge gate (`cargo build --workspace && cargo test --workspace`) before declaring a phase done.
5. Writes a `STATUS.md` after each phase.
6. Decides what to do when a subagent goes off-script.

**The orchestrator is not a subagent.** Subagents do not spawn subagents. There is one orchestrator per project.

---

## 2. The phase model

The build is 6 phases (0-5), matching `ROADMAP.md`. Each phase has:

- A **goal** (one sentence, from the roadmap).
- A **brief** (verbatim, from the roadmap — copy-paste, do not paraphrase).
- **Inputs** (what the subagent must read or have).
- **Outputs** (PR, branch, files modified).
- **Acceptance tests** (run by the orchestrator or the human).
- **DONE means** (the literal exit condition).

A phase is *not* done when the subagent says so. A phase is done when:

1. The orchestrator has merged the subagent's branch.
2. The merge gate (`cargo build --workspace && cargo test --workspace`) is green.
3. The acceptance tests in `ROADMAP.md` for that phase pass.
4. `STATUS.md` is updated.

---

## 3. Sequential vs parallel — the dependency graph

```
Phase 0 (Foundation)              — sequential, no deps
        │
        ▼
Phase 1a (Editor pane)   ─┐
Phase 1b (Selora client)  ─┼─ parallel — all depend on Phase 0
Phase 1c (Spike app)     ─┘  (1c depends on 1a + 1b's crates)
        │
        ▼ (after all of 1a/1b/1c merged)
Phase 2a (Tabs/splits)  ─┐
Phase 2b (File tree)     ─┤
Phase 2c (Fuzzy find)     ─┤
Phase 2d (Syntax HL)      ─┼─ parallel — all depend on Phase 1
Phase 2e (Cmd palette)    ─┤
Phase 2f (Settings)       ─┘
        │
        ▼
Phase 3a (LSP)          ─┐
Phase 3b (Git)           ─┼─ parallel — all depend on Phase 2
Phase 3c (Search)        ─┘
        │
        ▼
Phase 4a (Connect UI)    ─┐
Phase 4b (Model picker)   ─┤
Phase 4c (Chat sidebar)   ─┤
Phase 4d (Ghost text)     ─┼─ parallel — all depend on Phase 3
Phase 4e (Cost meter)     ─┤
Phase 4f (Conn status)    ─┘
        │
        ▼
Phase 6a (Splash)       ─┐
Phase 6b (About)         ─┤
Phase 6c (Installer)     ─┤
Phase 6d (CI release)    ─┼─ mostly sequential — depend on Phase 4
Phase 6e (Docs)          ─┤
Phase 6f (Repo hygiene)  ─┤
Phase 6g (Screenshots)   ─┘
```

**Critical rules:**

- A phase's parallel subagents **must not modify the same files**. If two subagents both need to edit `AppState`, they run sequentially. The orchestrator decides.
- A parallel subagent **must not depend on another parallel subagent's in-progress work**. If 2c fuzzy finder needs 2b's `FileTree` type, the orchestrator either runs 2b first or stubs the type in a shared header that both consume.
- A phase starts only when **all** of the previous phase's subagents have merged and the merge gate is green.

---

## 4. Per-phase subagent pattern

### 4.1 Spawn the subagent

The orchestrator pastes the verbatim brief from `ROADMAP.md` into a subagent. The brief includes:

- The READ FIRST list (the plan docs the subagent must read).
- The DEPENDENCIES YOU CONSUME list (the crate APIs the subagent will call).
- The DELIVERABLES list (the files the subagent will create/modify).
- The DO NOT list (negative constraints).
- The ACCEPTANCE list (the test commands).

The orchestrator does **not** add extra context. The brief is enough.

### 4.2 Subagent works autonomously

The subagent:

- Reads the inputs.
- Implements the deliverables.
- Runs the acceptance tests.
- Iterates until the tests pass.
- Reports back: the run instructions, the test output, any deviations.

The subagent does **not** ask the orchestrator questions. If the brief is unclear, the subagent makes the most reasonable interpretation and notes it in the report.

### 4.3 Orchestrator reviews

The orchestrator reviews the subagent's output:

1. **Spec check:** Does the implementation match the brief?
2. **Architecture check:** Does the implementation respect the crate boundaries in `PLAN.md` §5?
3. **Design check:** Does the implementation use only the tokens in `DESIGN_TOKENS.md`?
4. **Build check:** Does `cargo build --workspace` pass?
5. **Test check:** Does `cargo test --workspace` pass?
6. **Acceptance check:** Do the acceptance tests in the brief pass?

If any check fails, the orchestrator sends the subagent back with a specific list of what to fix. The subagent is not re-run from scratch.

### 4.4 Merge

The orchestrator merges the subagent's branch into the trunk. Updates `STATUS.md`. Decides the next phase.

---

## 5. The merge gate

Every phase ends with a **merge gate**. The gate is:

```bash
cd "C:\Users\Toxic\Desktop\Selora IDE"
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo deny check licenses
```

If the gate fails, the phase is not done. The orchestrator fixes or sends the subagent back.

The merge gate is the contract between phases. A subagent that produces code that compiles in isolation but breaks the workspace is a failed subagent, not a partial success.

---

## 6. The brief format

A good brief is **short, specific, and ends with acceptance**. Here is a template, with placeholders:

```
You are Phase {X}{Y} of the Selora IDE build ({short feature name}).
{Dependency note: e.g. "Phase {X} is merged. Phase {X}{Z} is running in parallel."}

READ FIRST, in full, before writing any code:
- C:\Users\Toxic\Desktop\Selora IDE\PLAN.md
- C:\Users\Toxic\Desktop\Selora IDE\ARCHITECTURE.md ({relevant sections})
- C:\Users\Toxic\Desktop\Selora IDE\DESIGN_TOKENS.md ({relevant sections})
- C:\Users\Toxic\Desktop\Selora IDE\UI_SPEC.md ({relevant sections})

THE DEPENDENCIES YOU CONSUME:
- crates/{name} — exposes {API summary}

DELIVERABLES:
1. {file/concrete thing}
2. {file/concrete thing}
3. {file/concrete thing}

DO NOT:
- {negative constraint 1}
- {negative constraint 2}
- {negative constraint 3}

ACCEPTANCE:
- {runnable command or observable behavior 1}
- {runnable command or observable behavior 2}
- {runnable command or observable behavior 3}

WHEN DONE, report:
- The exact commands you ran and their outputs.
- Any deviations from the spec and why.
- Whether you think the architecture is fit for the rest of the build.
```

The orchestrator writes each brief in this format and pastes it verbatim. Ad-libbing the brief is how subagents drift.

---

## 7. STATUS.md

After every phase (or subagent merge), the orchestrator writes `STATUS.md` at the repo root:

```markdown
# Status — YYYY-MM-DD

## Last completed
Phase {X} — {name}
Branch: {branch}
Merged: YYYY-MM-DD
Acceptance: all green
Notes: {any open issues}

## Next
Phase {X+1} — {name}
Briefs: ready in ROADMAP.md §{X+1}
Blockers: {any, or "none"}
```

This is the human's restart point. If they come back in two weeks, `STATUS.md` tells them where they are.

---

## 8. Common orchestrator mistakes

### 8.1 "Just give it more context"

Subagents perform worse with bloated briefs, not better. Concise briefs that point to the spec produce better code than briefs that re-summarize the spec.

### 8.2 "Re-run it with the same brief"

A subagent that failed with a given brief will fail again with the same brief. The orchestrator's job is to diagnose: was the brief unclear? Did the subagent hallucinate? Did the architecture need to change? Re-running blindly is a waste.

### 8.3 "Skip the merge gate"

The merge gate is the only thing keeping the project from rotting. A "I'll just check the diff" review is not a gate. The gate is the build + test + lint.

### 8.4 "Let it run for hours"

A subagent that runs for 4+ hours is almost certainly stuck. Schedule a wakeup after 90 minutes; if no progress, stop and inspect.

### 8.5 "Trust the report"

Subagents sometimes claim "all tests pass" when they don't. The orchestrator runs the gate themselves. Always.

### 8.6 "Just merge it, we'll fix it later"

Fix-it-later is a debt that compounds. The orchestrator sends subagents back until the brief is met. No exceptions.

---

## 9. Subagent capability tiers

Different subagents have different strengths. The orchestrator matches the tier to the task.

| Tier | Strengths | Use for |
|---|---|---|
| Tier 1 — Fast, single-file | Simple file edits, well-defined output | Tweak a design token, fix a typo, rename a symbol |
| Tier 2 — Standard implementation | One crate, clear interface, well-specified | Most subagent briefs in this plan |
| Tier 3 — Architect | Cross-crate changes, negotiation, ambiguity | Phase 1 spike (1c), Phase 2 merge integration, Phase 6d CI |

For Tier 3 tasks, the orchestrator may itself act as the subagent — the human is often the best Tier 3. For Tier 1 and 2, parallel subagents are fine.

---

## 10. Per-phase orchestration detail

### Phase 0 — Foundation

- **Pattern:** Single subagent, Tier 2.
- **Spawn:** Once.
- **Duration:** ~4-6 hours of agent time.
- **Review:** Heavy on the theme module (it will be the foundation of every visual decision for the rest of the project).
- **Gate:** `cargo run -p selora-app` opens the dark "Selora IDE" window.

### Phase 1 — Risk spike

- **Pattern:** Three parallel subagents (1a, 1b, 1c), all Tier 2-3.
- **Spawn:** All three in the same orchestrator turn, with parallel tool use.
- **Synchronization:** 1c cannot start until 1a and 1b have stubbed their crate APIs. The orchestrator's first move is to have 1a and 1b write the *crate skeleton* (Cargo.toml, lib.rs with type signatures). Then 1c can start. 1a and 1b then fill in the implementations. This is the "interface-first" pattern.
- **Merge order:** 1a, 1b in any order; 1c last. Each merge is gated.
- **Duration:** ~6-12 hours of agent time total.
- **Review:** Architectural. Is the crate split correct? Are the type signatures stable? Do the three subagents agree on the `AppMessage` / `Effect` enums?
- **Gate:** The spike window works end-to-end with a real Selora key.

### Phase 2 — Editor core

- **Pattern:** Six parallel subagents, all Tier 2.
- **Synchronization:** 2a-2f all depend on Phase 1's `AppState`, `Effect`, `AppMessage` types. The orchestrator confirms these are stable before launching 2a-2f.
- **File ownership:** Strict. Each subagent owns its crate:
  - 2a owns `crates/selora-ui/src/editor/tabs.rs` (new) and may modify `crates/selora-app/src/app_state.rs` only via the orchestrator.
  - 2b owns `crates/selora-fs/` and `crates/selora-ui/src/sidebar/file_tree.rs`.
  - 2c owns `crates/selora-fs/src/fuzzy.rs` and `crates/selora-ui/src/palette/fuzzy.rs`.
  - 2d owns `crates/selora-editor/src/syntax.rs`.
  - 2e owns `crates/selora-ui/src/palette/command.rs`.
  - 2f owns `crates/selora-settings/` and `crates/selora-ui/src/dialogs/settings.rs`.
- **Shared file rule:** `AppState` (in `crates/selora-app`) is the only file multiple subagents may need to touch. The orchestrator owns the merge of `AppState` changes — subagents send diffs, the orchestrator applies.
- **Merge order:** Any order. The orchestrator runs the gate after each merge.
- **Duration:** ~12-18 hours of agent time total.
- **Review:** Heavy on the AppState changes. The orchestrator's biggest job in Phase 2 is keeping `AppState` from becoming a mess.
- **Gate:** Open folder, edit a file, fuzzy find, syntax highlight, settings persist.

### Phase 3 — LSP & Git

- **Pattern:** Three parallel subagents, Tier 2-3.
- **Synchronization:** 3a is Tier 3 (LSP is hard). 3b and 3c are Tier 2.
- **File ownership:**
  - 3a owns `crates/selora-lsp/` and `crates/selora-editor/src/lsp_overlay.rs`.
  - 3b owns `crates/selora-git/` and `crates/selora-ui/src/panel/git.rs`.
  - 3c owns `crates/selora-fs/src/search.rs` and `crates/selora-ui/src/panel/search.rs`.
- **Duration:** ~8-12 hours of agent time total.
- **Review:** Heavy on 3a. The LSP test suite must be solid before integration.
- **Gate:** LSP works on a real Rust file, git commit works, search-in-files works.

### Phase 4 — Selora AI

- **Pattern:** Six parallel subagents, Tier 2.
- **Synchronization:** 4a-4f all depend on `selora-ai` from Phase 1. Most code is in `selora-ui`.
- **File ownership:**
  - 4a owns `crates/selora-ui/src/dialogs/settings/selora_section.rs`.
  - 4b owns `crates/selora-ui/src/picker/model_picker.rs`.
  - 4c owns `crates/selora-ui/src/sidebar/chat.rs`.
  - 4d owns `crates/selora-editor/src/ghost_text.rs`.
  - 4e owns `crates/selora-ui/src/status_bar/cost_meter.rs`.
  - 4f owns `crates/selora-ui/src/status_bar/selora_status.rs`.
- **Duration:** ~10-14 hours of agent time total.
- **Review:** Heavy on the chat UX. Streaming must feel smooth.
- **Gate:** Real Selora chat streams, ghost text works, cost meter shows real values.

### Phase 5 — Agent Mode

- **Pattern:** Five parallel subagents (5a tool types, 5b tool router, 5c loop controller, 5d cards UI, 5e web tools).
- **Synchronization:** Interface-first. 5a defines the tool plumbing types first (`ToolSpec`, `ToolCallDelta`, tool-result messages) — the other four consume that API. The orchestrator has 5a land the type skeletons within the first hour, then all five fill in implementations.
- **File ownership:**
  - 5a owns `crates/selora-ai/src/chat_types.rs` (tool spec/call/result types, request plumbing).
  - 5b owns `crates/selora-ai/src/agent/router.rs` + `crates/selora-ai/src/agent/tools/` (all tool implementations, guards).
  - 5c owns `crates/selora-ai/src/agent/turn.rs` (loop state machine, caps, decisions).
  - 5d owns `crates/selora-ui/src/sidebar/chat/agent_cards.rs` (all card rendering).
  - 5e owns `crates/selora-ai/src/agent/tools/web.rs` (search + fetch, only this file may add an HTTP dependency outside SeloraClient).
- **Shared file rule:** `Effect`/`AppMessage` enums in `ARCHITECTURE.md` §17.6 grow additively. The orchestrator applies those enum changes centrally (same pattern as `AppState` in Phase 2) so 5b/5c/5d never touch the same file.
- **Duration:** ~12-16 hours of agent time total.
- **Review:** Heaviest review in the project. Every guard (path rooting, SSRF, env stripping, output caps) gets a line-by-line read by the orchestrator. The security-relevant code is small (~500 LOC of guards); read all of it.
- **Gate:** All Phase 5 acceptance tests in ROADMAP.md pass against a real Selora key and a real workspace; the deny-path test proves no disk write happens without approval.

### Phase 6 — Polish & ship

- **Pattern:** Mostly sequential. 6c and 6d can run in parallel after 6a-6b. 6e-6g can run in parallel after 6c.
- **Synchronization:** 6c needs the binary from 6a-6b. 6d needs the installer from 6c. 6e-6g need a working app.
- **File ownership:**
  - 6a owns `crates/selora-ui/src/splash.rs`.
  - 6b owns `crates/selora-ui/src/dialogs/about.rs`.
  - 6c owns `installer/selora-ide.iss`.
  - 6d owns `.github/workflows/release.yml`.
  - 6e owns `README.md`, `docs/`.
  - 6f owns `.github/ISSUE_TEMPLATE/`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`.
  - 6g owns `assets/screenshots/`.
- **Duration:** ~6-10 hours of agent time total.
- **Review:** Light. This is polish.
- **Gate:** Installer downloads, installs, launches, all features work on a clean Windows VM.

---

## 11. Regression checks after each merge

After every merge, the orchestrator runs:

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo deny check licenses
```

Plus, after each phase, a **manual smoke test** on the owner's machine:

| After Phase | Smoke test |
|---|---|
| 0 | Window opens, looks right |
| 1 | Spike window: type, click, see real SSE |
| 2 | Open folder, edit, find, settings persist |
| 3 | LSP hover + go-to-def, git commit, search |
| 4 | Real chat streams, ghost text, cost meter |
| 5 | Agent reads a file, edits via approved diff, runs an allowlisted command, fetches the web; Deny produces no disk change |
| 6 | Installer runs, all features work on a fresh install |

If a smoke test fails, the orchestrator decides: re-run the subagent, fix it themselves, or move the feature to a Future list.

---

## 12. The verbatim handoff line

This is the literal contract between the planning AI and the executing AI. It appears in `PLAN.md` §11, in this document, and in the final reply:

> **Take my task → split it into 5 logical phases → identify dependencies → run all independent phases in parallel using subagents → merge and review everything → complete the final result.**

The orchestrator treats this line as the operating principle. The 6 phases in `ROADMAP.md` are an elaboration of "5 logical phases" — Phase 0 (foundation) is preparatory, Phases 1-4 are the four big feature groups, Phase 6 is the ship. The orchestrator can collapse or expand as needed, but the line is the *shape* of the work.
