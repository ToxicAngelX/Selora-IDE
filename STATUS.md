# Status - 2026-10-27 (post-merge documentation skeleton)

> **NOT FINAL. FINALIZE AFTER MERGE.**
> This restart point is a skeleton for the tree that exists after the Phase 8
> follow-up, Phase 9a, and session branches are merged. Every value marked
> `TODO-VERIFY` must be replaced from the final gate output. Do not copy an
> earlier number or hash into a final record.
>
> Restart point. Read this, then `AGENT_ORCHESTRATION.md` section 10 and
> `docs/POST_MERGE_FINALIZATION.md`. The pre-Tauri STATUS is archived as
> `STATUS_ARCHIVE_2026-10-09.md`; the per-slice audit and its hardening
> addendum live in `docs/ui/PHASE_Z_PARITY_REPORT.md`.

## Merge state (fill at finalization)

This worktree is the documentation branch `selora-post-docs`. It does **not**
contain the feature branches yet. Merge state below is a plan, not evidence.

| Branch | Scope | Merge state |
|---|---|---|
| `main` | Phase 8 waves 2-4 on the base tree | base |
| `integration` | Phase 8 follow-ups: journal history, dirty-buffer conflict, budget resume preflight, LSP lifecycle, search parity | `TODO-VERIFY` |
| `selora-p3-window` | Phase 9a Agent Workbench frontend | `TODO-VERIFY` |
| `selora-wt-session` | Backend-authoritative sessions, todos, retry, event ordering | `TODO-VERIFY` |
| `selora-wt-settings` | Real editor settings and dirty-close protection | `TODO-VERIFY` |
| `selora-wt-lsp` | LSP lifecycle isolation and `rg`/fallback parity | `TODO-VERIFY` |
| `selora-p3-plan` | Phase 9b plan mode + todos + effects gating | **NOT STARTED** (branch has no commits) |
| `selora-p3-subagent` | Phase 9c subagent runtime + escalations | **NOT STARTED** (branch has no commits) |
| `selora-p3-sessions` | Phase 9d persistence, resume, compaction | **NOT STARTED** (branch has no commits) |
| `selora-p3-background` | Phase 9e background runs + notifications | **NOT STARTED** (branch has no commits) |

## Last completed

Phase Z7 hardening and **Phase 8 waves 2-4**: execution, approvals, durable
replay, receipt accounting, process-tree containment, explicit unfinished-ask
resume consent, live buffer refresh, and a release build plus installer.

Post-merge additions expected from the branches above: completed-run history,
explicit journal phases/reports/artifacts, budget preflight on direct resume,
dirty-buffer conflict handling, LSP lifecycle fixes, real editor settings,
backend-authoritative sessions/todos/retry, and the Phase 9a Agent Workbench
frontend.

Automated work was recorded as green **before** the post-merge tree existed.
That result does not describe the merged tree. Re-run the full gate and record
the result here.

## Last completed gate (POST-MERGE: NOT RUN)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | `TODO-VERIFY` |
| `cargo clippy --workspace --all-targets -- -D warnings` | `TODO-VERIFY` |
| `cargo test --workspace --no-fail-fast` | `TODO-VERIFY` (passed / failed / ignored / suites) |
| `npm run typecheck` | `TODO-VERIFY` |
| `npm test` | `TODO-VERIFY` (passed / failed / files) |
| `npm run build` | `TODO-VERIFY` |
| `cargo deny check` | `TODO-VERIFY` |
| `scripts/phase8-acceptance.ps1` | `TODO-VERIFY` (passed / failed / skipped, exit code) |

Gate commands run from the repository root unless noted:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo deny check
npm.cmd --prefix apps/selora-desktop/frontend run typecheck
npm.cmd --prefix apps/selora-desktop/frontend test
npm.cmd --prefix apps/selora-desktop/frontend run build
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1
```

## Packaging artifacts (POST-MERGE: NOT BUILT)

Rebuild both artifacts from the final merged commit. Do not reuse a pre-merge
binary.

| Artifact | Path | Size | SHA-256 | Signature |
|---|---|---|---|---|
| Release app | `target\release\selora-desktop.exe` | `TODO-VERIFY` | `TODO-VERIFY` | `NotSigned` / `TODO-VERIFY` |
| Installer | `installer\output\Selora-IDE-Setup-<version>.exe` or owner-chosen path | `TODO-VERIFY` | `TODO-VERIFY` | `NotSigned` / `TODO-VERIFY` |

Current known state: **unsigned artifacts, no clean-VM install, no owner
acceptance, no visual pass.** The update check still targets a repository that
returns 404 until the owner publishes it.

## Phase 8 status (Workflows)

| Slice | State |
|---|---|
| 8a engine core | Implemented on `main`: boa engine, hardened realm, caps, schema-validated `ask`, command extraction, `prepare`/`run_workflow` |
| 8b journal/budget/subagents | Implemented on `main`; follow-ups for history, budget preflight, and buffer conflicts are on `integration` (`TODO-VERIFY`) |
| 8c protocol/bridge | Implemented on `main`; `TODO-VERIFY` after merge |
| 8c Host | Implemented on `main`: root-contained files/git, Job Object tree kill, agent turns, approvals, receipts |
| 8c dispatch | Implemented on `main`: start/stop/resume, live approvals, unfinished-ask consent |
| 8c tool/card | Implemented on `main`: manifest v2, strict validation, workflow card |
| 8d panel | Implemented on `main`; completed-run history is on `integration` (`TODO-VERIFY`) |
| 8d buffers | Implemented on `main`; conflict handling and LSP reload sync are on `integration` (`TODO-VERIFY`) |
| 8d shell/docs/package | Automation script and release EXE exist pre-merge; rebuild and re-verify after merge |
| Owner acceptance | **UNRUN** - ROADMAP Phase 8 steps 1-9 |

## Phase 9 status (Agent Workbench)

| Wave | State |
|---|---|
| 9a Agent window | Implemented on `selora-p3-window`; merge `TODO-VERIFY`; frontend contract in `docs/ui/PHASE9A_AGENT_WORKBENCH.md` |
| 9b Plan mode + todos + **effects gating** | **NOT STARTED** - no plan card execution, no `todo_read`/`todo_write`, no writes-during-planning rejection; see `docs/ui/PHASE9B_PLAN_TODOS.md` |
| 9c Subagent runtime + escalations | **NOT STARTED** - no `spawn_subagent`, no `ask_user_question`, no shared runtime |
| 9d Sessions + compaction | **PARTIAL** - backend-authoritative sessions/todos/retry/event ordering on `selora-wt-session`; durable JSONL, resume picker, compaction, and session auto-approve remain **NOT STARTED** |
| 9e Background runs + notifications | **NOT STARTED** |
| Owner acceptance | **UNRUN** - see `docs/PHASE9_ACCEPTANCE.md` |

Phase 9 is **not** accepted. Plan/effects waves are unimplemented. Do not
describe the Agent Workbench as complete.

## What is real now

| Slice | State |
|---|---|
| Shell / layout / persistence | Real: seven regions, drag + keyboard resize, layout restore |
| Workspace / files / editor | Real: folder picker, tree, 2 MiB UTF-8 open, tabs, save, find/replace, minimap |
| Search / Git / Problems | Real: literal + regex search, stage/unstage/commit, diffs, live diagnostics |
| LSP | Real for Rust and TS/JS from `PATH`; branch fixes pending merge |
| Agent Mode | Real tools, streaming, cards, approvals, stop; Full auto-apply for file tools in the session |
| Workflows (Phase 8) | Engine, Host, approvals, durable resume, buffer refresh, live panel, confirmation card wired; owner acceptance pending |
| Agent Workbench (Phase 9) | 9a frontend only, pending merge; 9b-9e not implemented |
| Settings | Theme + reduced motion persist; branch adds real editor settings (`TODO-VERIFY`) |
| Terminal / Output | Intentionally inert placeholders |
| Packaging | Pre-merge EXE and unsigned installer existed; rebuild from the final commit |

## Next (finalization order)

1. Merge the feature branches in dependency order. Run the full gate after each
   merge. Resolve conflicts in `STATUS.md`, `ROADMAP.md`, `CHANGELOG.md`, and
   `docs/` deliberately; this skeleton is the intended shape.
2. Fill every `TODO-VERIFY` in this file, `docs/PHASE8_ACCEPTANCE.md`,
   `docs/PHASE9_ACCEPTANCE.md`, `docs/ui/PHASE9_*.md`, `CHANGELOG.md`, and
   `RELEASE.md`.
3. Run Phase 8 owner acceptance steps 1-9 with a real key and workspace.
4. Run the Phase 9 acceptance checklist that applies to the merged waves. Mark
   unbuilt waves **NOT RUN / NOT IMPLEMENTED**, never passed.
5. Build the release EXE and installer from the final commit. Record sizes,
   SHA-256 values, and Authenticode state. Sign only if the owner supplied the
   certificate secrets.
6. Install on a clean Windows 11 VM. Repeat without WebView2 on Windows 10 if
   the runtime branch changed. Record the result.
7. Update `ROADMAP.md` phase status and this restart point. Commit only after
   the documentation matches the merged tree.

## Blockers (owner decisions or human-only)

1. **No signing certificate.** `SIGNING_CERT_BASE64` / `SIGNING_CERT_PASSWORD`
   are owner-only. Artifacts stay `NotSigned` until then.
2. **No clean-VM run.** WebView2-missing install, per-user install, file
   association, launch, and uninstall remain unverified.
3. **Update check returns 404.** The About dialog queries
   `api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest`; publish the
   repository or change the slug.
4. **Five of seven Libraries.dev packages are not installed**
   (`border-beam`, `thinking-orbs`, `bot-avatars`, `liquid-gooey`,
   `voice-glow`). Decide install-and-verify or accept the local ports.
5. **Session lifecycle is only partially delivered.** Backend
   new/switch/retry exists on a branch; durable resume, compaction, and
   cross-session trust rules are Phase 9d work.
6. **Full auto-apply is the default composer mode.** File writes apply without
   a card for the session; bash still cards. Confirm the default, or switch
   back to Ask before changes.
7. **Inno Setup may live under `%TEMP%`.** Install it durably before the owner
   packaging run; `installer\prebuild.bat` does not find a temp-only copy.

## Working agreements that still apply

- Real working features only, each with a verification step.
- No features outside `PLAN.md` section 2's non-goals.
- Gate after every change: build + test + fmt + clippy (and the npm trio).
- Token-only colours, reduced-motion equivalents, and the Z2 disabled-affordance
  rule (transparent fill + hairline + `--fg-disabled` + explanatory `title`).
- Shell is PowerShell here; no bash assumptions in the repo scripts.

## Housekeeping

- Gate logs (`*.log`) and the old `service.json` / empty text file are ignored
  by `.gitignore`; `target/`, `node_modules/`, `dist/`, and `installer/output/`
  were already ignored. `/.selora/` is ignored for workflow journals.
- `bundle.active=false`, so the Inno script remains the packaging path.
- Pre-merge evidence lives in git history; it is not evidence for the merged
  tree.