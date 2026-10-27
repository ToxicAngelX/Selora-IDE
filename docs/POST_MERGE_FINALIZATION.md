# Post-merge finalization

> **STATUS: SKELETON (2026-10-27).** This file is the index for finalizing the
> documentation after the feature branches merge. It is not a release record.
> Every `TODO-VERIFY` must be replaced from the final gate output. Never copy a
> count, size, date, or hash from an earlier tree or from memory.

## 1. Why this exists

The documentation branch `selora-post-docs` was authored on `main` before these
branches merge. It is now merged into the release lane `selora-lane-release`
together with the release-prep and walkthrough lanes. Branch merge states stay
`TODO-VERIFY` until the finalization run records them. The branches covered are:

- `integration` - Phase 8 follow-ups (history, buffers, budget, LSP, search)
- `selora-p3-window` - Phase 9a Agent Workbench frontend
- `selora-wt-session` - backend sessions, todos, retry, event ordering
- `selora-wt-settings` - real editor settings, dirty close
- `selora-wt-lsp` - LSP lifecycle isolation, search parity

The `selora-p3-plan`, `selora-p3-subagent`, `selora-p3-sessions`, and
`selora-p3-background` branches each carry commits but are not merged into the
release lane yet. The docs must record their merge state and acceptance result
plainly after the merge, and must not describe planned work as delivered.

## 2. Wave status at skeleton time

| Wave | Status | Evidence |
|---|---|---|
| Phase 8 waves 2-4 | Implemented on `main`; automated gate recorded pre-merge; owner **UNRUN** | `STATUS.md`, `docs/PHASE8_ACCEPTANCE.md` |
| Phase 8 follow-ups | Merged via `integration` into this lane; gate `TODO-VERIFY` | branch log |
| Phase 9a Agent window | Frontend on `selora-p3-window`; not merged in this lane | `docs/ui/PHASE9A_AGENT_WORKBENCH.md` |
| Phase 9b plan mode | Commits on `selora-p3-plan`; not merged in this lane | branch log |
| **Effects wave** (approved writes) | No plan gate in this tree's router; branch not merged | router |
| Phase 9c subagents | Commits on `selora-p3-subagent`; not merged in this lane | branch log |
| Phase 9d sessions | Commits on `selora-wt-session` and `selora-p3-sessions`; not merged in this lane | `docs/ui/SESSION_STATE_SLICE.md` after merge |
| Phase 9e background | Commits on `selora-p3-background`; not merged in this lane | branch log |
| Owner acceptance (8 and 9) | **UNRUN** | acceptance checklists |
| Artifacts | **UNSIGNED**; not rebuilt from the final merge | `RELEASE.md` section 11 |
| Clean VM | **NOT RUN** | no clean-VM evidence |

## 3. Fields to fill after the final gate

### `STATUS.md`

- Merge state for every branch: `TODO-VERIFY`
- Full gate results: fmt, clippy, cargo test counts, npm typecheck/test counts,
  build, cargo deny: `TODO-VERIFY`
- Phase 8 acceptance script counts and exit code: `TODO-VERIFY`
- Phase 8 status table merge notes: `TODO-VERIFY`
- Phase 9 wave table merge notes: `TODO-VERIFY`
- What-is-real table branch notes: `TODO-VERIFY`
- Packaging artifact table: paths, sizes, SHA-256, signature: `TODO-VERIFY`
- Final merge commit: `TODO-VERIFY`

### `ROADMAP.md`

- Phase 8 post-merge gate result: `TODO-VERIFY`
- Phase 8 follow-up merge state: `TODO-VERIFY`
- Phase 9 wave table merge state: `TODO-VERIFY`
- Phase 9 owner acceptance result: `TODO-VERIFY`
- Phase 9 effects wave status: `TODO-VERIFY` (currently **NOT STARTED**)

### `docs/PHASE8_ACCEPTANCE.md`

- Final commit, branch/tag, date, version: `TODO-VERIFY`
- Toolchain table: `TODO-VERIFY`
- Frontend build counts, `dist` bytes, `index.html` hash: `TODO-VERIFY`
- Release EXE size, SHA-256, build time, signature: `TODO-VERIFY`
- Installer path, size, SHA-256, signature: `TODO-VERIFY`
- Acceptance script Summary block: `TODO-VERIFY`
- Roadmap steps 1-9 automated column: `TODO-VERIFY`
- Owner observations: **UNRUN** until performed
- Clean-VM result: **NOT RUN** until performed

### `docs/PHASE9_ACCEPTANCE.md`

- 9a merge state and gate: `TODO-VERIFY`
- 9b result: **NOT RUN / NOT IMPLEMENTED**
- 9c result: **NOT RUN / NOT IMPLEMENTED**
- 9d result: **NOT RUN / PARTIAL**
- 9e result: **NOT RUN / NOT IMPLEMENTED**
- Steps 1-9 evidence: `TODO-VERIFY` where the owning wave exists
- Exit criteria checkboxes: only after owner observation

### `docs/ui/PHASE9_AGENT_WORKBENCH.md`

- 9a merge commit and gate counts: `TODO-VERIFY`
- Session branch merge commit: `TODO-VERIFY`
- NewSession / SwitchSession reconciliation: `TODO-VERIFY`
- Typed `decision_prompt` / `todos` promotion: `TODO-VERIFY`
- Plan/effects status: `TODO-VERIFY`
- Subagent, durable session, background status: `TODO-VERIFY`

### `docs/ui/PHASE9A_AGENT_WORKBENCH.md`

- Branch merge commit: `TODO-VERIFY`
- Frontend typecheck / test / build: `TODO-VERIFY`
- Session success path and fallback text: `TODO-VERIFY`
- Owner visual acceptance: **UNRUN**

### `docs/ui/PHASE9B_PLAN_TODOS.md`

- Implementation commit: `TODO-VERIFY`
- Manifest version: `TODO-VERIFY`
- Plan gate / todo test results: `TODO-VERIFY`
- Bash-in-plan decision: `TODO-VERIFY`
- Owner steps 1-3: **UNRUN**
- Effects wave: **NOT STARTED** until observed

### `CHANGELOG.md`

- Move `Unreleased` entries to a versioned section only after the gate passes:
  `TODO-VERIFY` version and date
- Remove pending 9a entries if the branch is discarded: `TODO-VERIFY`
- Do not add test counts or hashes; link to `STATUS.md` / `RELEASE.md`

### `RELEASE.md` section 11

- Final version, commit, tag, date: `TODO-VERIFY`
- Gate result table: `TODO-VERIFY`
- Release EXE size / SHA-256 / signature: `TODO-VERIFY`
- Installer size / SHA-256 / signature: `TODO-VERIFY`
- Clean Windows 11 VM result: **NOT RUN**
- Windows 10 without WebView2 result: **NOT RUN**
- Signing state: **UNSIGNED** until owner supplies secrets

## 4. Final gate

Run from the repository root on the final commit:

```powershell
git status --short
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo deny check
npm.cmd --prefix apps/selora-desktop/frontend ci
npm.cmd --prefix apps/selora-desktop/frontend run typecheck
npm.cmd --prefix apps/selora-desktop/frontend test
npm.cmd --prefix apps/selora-desktop/frontend run build
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1
```

Then build both artifacts from that same commit:

```powershell
cargo clean -p selora-desktop --release
cargo build --release -p selora-desktop --locked
& "<ISCC path>" /Qp /DMyAppVersion=<version> `
  "/DMyTargetDir=$((Resolve-Path target\release).Path)" `
  "/DMyCrtDlls=vcruntime140.dll,vcruntime140_1.dll" `
  "/O<output directory>" installer\selora-ide.iss
Get-FileHash target\release\selora-desktop.exe -Algorithm SHA256
Get-FileHash "<installer path>" -Algorithm SHA256
Get-AuthenticodeSignature target\release\selora-desktop.exe
Get-AuthenticodeSignature "<installer path>"
```

Record exit codes and full Summary blocks. A SKIP is not a pass.

## 5. Known limits (must survive finalization)

1. **Unsigned artifacts.** No signing certificate is configured. Both the EXE
   and installer report `NotSigned`; SmartScreen will warn.
2. **No clean-VM install.** The WebView2-missing branch, per-user install, file
   association, launch, and uninstall are unverified.
3. **Update check returns 404.** The About dialog targets
   `api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest`.
4. **Phase 8 resume is not exactly-once.** An unfinished ask may already have
   applied tool effects before **Resume anyway**.
5. **Dirty buffers are preserved, not merged.** Workflow reloads clean buffers;
   dirty tabs stay dirty with a toast.
6. **Journal history depends on `integration`.** Without that merge, completed
   history and explicit phases/reports/artifacts are absent.
7. **Phase 9 plan/effects waves are not implemented.** No write is structurally
   blocked during planning; no plan approval flips execution.
8. **Phase 9 subagents are not implemented.** No runtime, policies, steering,
   or question cards from the backend.
9. **Phase 9 sessions are only in memory.** No durable JSONL, resume picker,
   compaction, or session auto-approve.
10. **Phase 9 background runs are not implemented.** No toast, badge, or wait.
11. **Five Libraries.dev packages are not installed** (`border-beam`,
    `thinking-orbs`, `bot-avatars`, `liquid-gooey`, `voice-glow`). The ports
    ship unless the owner decides otherwise.
12. **Full auto-apply is the default composer mode.** File writes skip cards
    for the session; bash still cards. The owner must confirm the default.
13. **Inno Setup may be temp-only.** Install it durably before packaging.
14. **Headless green is not runtime evidence.** No visible-window acceptance is
    implied by tests.

## 6. Owner-only steps

These cannot be completed by an agent and must not be checked off in advance:

1. Provide a real gateway key or sign in with a real account.
2. Run Phase 8 acceptance steps 1-9 in a visible window and record what was
   observed.
3. Run the Phase 9 steps that the merged waves support; mark the rest
   **BLOCKED - NOT IMPLEMENTED**.
4. Decide the Full auto-apply default.
5. Decide the five Libraries.dev packages: install and verify, or accept ports.
6. Supply `SIGNING_CERT_BASE64` and `SIGNING_CERT_PASSWORD`, or accept unsigned
   artifacts.
7. Publish the GitHub repository/release so the update check stops returning
   404, or change the slug.
8. Install on a clean Windows 11 VM; optionally test Windows 10 without
   WebView2.
9. Install Inno Setup durably if the only copy is under `%TEMP%`.
10. Approve the final version number and tag.

## 7. Acceptance rule

A phase is accepted only when:

- the final commit is recorded and the tree is clean;
- the full gate is green on that commit;
- the artifacts are rebuilt from that commit and their hashes recorded;
- every owner step has an observation, not an inference;
- unrun items are labelled **UNRUN** and unimplemented items **NOT
  IMPLEMENTED**;
- no known limit is silently removed.

Until then:

```
Phase 8: automated implementation complete - acceptance UNRUN
Phase 9: 9a branch-only; 9b/9c/9e NOT STARTED; 9d PARTIAL - acceptance UNRUN
Artifacts: UNSIGNED - not rebuilt from final merge
Clean VM: NOT RUN
```