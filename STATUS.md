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

This worktree is the release lane `selora-lane-release`. It contains the
release-prep, walkthrough, and documentation lanes merged on top of the
feature tree. The feature branches in the table below are merged through
`integration`; the remaining Phase 9 lanes are not merged here yet.

| Branch | Scope | Merge state |
|---|---|---|
| `main` | Phase 8 waves 2-4 on the base tree | base |
| `integration` | Phase 8 follow-ups: journal history, dirty-buffer conflict, budget resume preflight, LSP lifecycle, search parity | merged (ancestor of this lane); gate `TODO-VERIFY` |
| `selora-p3-window` | Phase 9a Agent Workbench frontend | branch has commits; not merged in this lane |
| `selora-wt-session` | Backend-authoritative sessions, todos, retry, event ordering | branch has commits; not merged in this lane |
| `selora-wt-settings` | Real editor settings and dirty-close protection | branch has commits; not merged in this lane |
| `selora-wt-lsp` | LSP lifecycle isolation and `rg`/fallback parity | merged (part of `integration`) |
| `selora-p3-plan` | Phase 9b plan mode + todos + effects gating | branch has commits; not merged in this lane |
| `selora-p3-subagent` | Phase 9c subagent runtime + escalations | branch has commits; not merged in this lane |
| `selora-p3-sessions` | Phase 9d persistence, resume, compaction | branch has commits; not merged in this lane |
| `selora-p3-background` | Phase 9e background runs + notifications | branch has commits; not merged in this lane |

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

### Packaging artifacts (Phase 8 build, unsigned - historical)

| Artifact | Path | Size | SHA-256 | Signature |
|---|---|---|---|---|
| Release app | `target\release\selora-desktop.exe` | `TODO-VERIFY` | `TODO-VERIFY` | `NotSigned` / `TODO-VERIFY` |
| Installer | `installer\output\Selora-IDE-Setup-<version>.exe` or owner-chosen path | `TODO-VERIFY` | `TODO-VERIFY` | `NotSigned` / `TODO-VERIFY` |

Current known state: **unsigned artifacts, no clean-VM install, no owner
acceptance, no visual pass.** The update check still targets a repository that
returns 404 until the owner publishes it.

The earlier Phase 8 smoke artifacts landed outside `installer/output/` because
`prebuild.bat` could not find the `%TEMP%` ISCC copy. Both are `NotSigned`.
Release build process smoke: window created and shown; no visual/manual UI
acceptance was performed.

### 0.1.0 release dry run (2026-10-27, pre-merge)

| Artifact | Path | Size | SHA-256 |
|---|---|---|---|
| Release app | `target\release\selora-desktop.exe` | 18,875,904 | `acab5f75eb34a53525918c8b8e770f902adf8aa5c622419e3e13c4c1cc13cfd5` |
| Installer | `installer\output\Selora-IDE-Setup-0.1.0.exe` | 9,408,027 | `95b2f4deb29fa851215fd08fd6f6a88918ec46757884b19866d4a829882360b0` |

`installer\prebuild.bat` now resolves ISCC through `installer\find-iscc.ps1`
(the local installer is registered under `%TEMP%`) and passes an absolute
`/O`, so the setup lands in `installer\output`. The artifact is `NotSigned`,
ProductVersion 0.1.0. Release metadata, tag gates, draft-asset checks, and
owner-only steps are in `RELEASE_0.1.0_CHECKLIST.md`. These values describe the
pre-merge tree; rebuild from the final commit before recording them again.

Wave 3/4 review fixes include argument-safe `world.run`, approval retries,
hydration, late events, concurrent folder switches, configured git diff helpers,
and PowerShell arguments that cannot become script text.

Earlier denial gate required lockfile updates: `rustls 0.23.46`,
`rustls-webpki 0.103.16`, `tokio 1.54.1`, `webpki-roots 1.0.11`, and
`clipboard-win 5.4.3` (the previous versions hit 2026 RUSTSEC advisories or
were yanked). No source API changed.

## What changed in the hardening pass

| Area | Delivered |
|---|---|
| Editor | The edit version contract now sends the backend's current version (the old code sent `version + 1`, so every keystroke was rejected and saves wrote stale text); multi-range changes apply in pre-edit coordinates; per-buffer local state survives tab switches; Problems click uses the correct 1-based line; CRLF positions clamp |
| Agent tools | The Rust loop already runs `read_file` / `list_dir` / `fuzzy_find` / `search_files` / `write_file` / `edit_file` / `delete_file` / `bash` / web tools. The system prompt now explicitly instructs the model to use them, and the end-to-end tests prove a write applies only after the decision path |
| Session full auto-apply | `AgentModeDto::Auto` maps the composer's **Full auto-apply** mode to a session-scoped flag: file writes/edits/deletes apply without a card; `bash` still cards unless allowlisted. The flag is never persisted and dies with the session. Tests cover both the loop and the desktop dispatch path |
| Agent safety/UX | Approval Enter/Escape only work on the focused card; non-session commands no longer carry `session_id`; replay/live ordering, bootstrap chat hydration, stale-event routing, approval request ids, and stop-before-steer are fixed; fabricated model/attachment/context values removed; send is gated on a real catalog model and an open workspace |
| LSP | Windows verbatim `\\?\` roots are normalized (diagnostics/definitions were being dropped); npm `.cmd` servers resolve and spawn through `cmd.exe`; `kill_on_drop`; pending `didChange` flushes before a position request; gateway key env is stripped from server children |
| Security | Bash allowlist rejects shell control operators; `read_file` slices files over 4 MiB; atomic-write temp names are unique; keyring fallback is written only when the keyring fails (never on read) and delete reports failures; no `SELORA_API_KEY`/`OPENAI_API_KEY` env propagation |
| Performance | IPC commands run on `spawn_blocking` instead of the UI thread; sidebar and bottom dock are lazy chunks (entry JS `480.27 -> 435.69 kB`, entry CSS `177.64 -> 137.03 kB`) |
| Git/workspace | Staged rows show the HEAD-to-index diff; `GitStatusDto.files` carries both slots so an `MM` file appears in both sections; literal/regex whole-word parity for `@foo`, `C++`, `->`; activity-bar badge counts unique paths |
| UI/release | Settings panel shows truthful values and real commands; dirty tab close has Save / Discard / Cancel; remaining a11y fixes (file-tree roving focus, plan-spine buttons, progressbar, notification inert background, dialog Escape/restore); `CHANGELOG [0.1.0]`; release workflow gates tags on `main` and runs fmt/clippy/deny; release docs reconciled |

## Phase 8 status (Workflows, waves 2-4)

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
| 9a Agent window | Commits on `selora-p3-window`; not merged in this lane; frontend contract in `docs/ui/PHASE9A_AGENT_WORKBENCH.md` |
| 9b Plan mode + todos + **effects gating** | Commits on `selora-p3-plan`; not merged in this lane; acceptance **UNRUN**; see `docs/ui/PHASE9B_PLAN_TODOS.md` |
| 9c Subagent runtime + escalations | Commits on `selora-p3-subagent`; not merged in this lane; acceptance **UNRUN** |
| 9d Sessions + compaction | Commits on `selora-wt-session` and `selora-p3-sessions`; not merged in this lane; durable resume, compaction, and session auto-approve unverified here |
| 9e Background runs + notifications | Commits on `selora-p3-background`; not merged in this lane; acceptance **UNRUN** |
| Owner acceptance | **UNRUN** - see `docs/PHASE9_ACCEPTANCE.md` |

Phase 9 is **not** accepted, and no wave beyond the unmerged 9a frontend exists
in this tree. Do not describe the Agent Workbench as complete.

## What is real now

| Slice | State |
|---|---|
| Shell / layout / persistence | Real: seven regions, drag + keyboard resize, layout restore |
| Workspace / files / editor | Real: folder picker, tree, 2 MiB UTF-8 open, tabs, save, find/replace, minimap |
| Search / Git / Problems | Real: literal + regex search, stage/unstage/commit, diffs, live diagnostics |
| LSP | Real for Rust and TS/JS from `PATH`; branch fixes pending merge |
| Agent Mode | Real tools, streaming, cards, approvals, stop; Full auto-apply for file tools in the session |
| Workflows (Phase 8) | Engine, Host, approvals, durable resume, buffer refresh, live panel, confirmation card wired; owner acceptance pending |
| Agent Workbench (Phase 9) | 9a frontend and 9b-9e work exist on unmerged branches; only the 9a shell pieces present on `main` are real here |
| Settings | Theme + reduced motion persist; branch adds real editor settings (`TODO-VERIFY`) |
| Terminal / Output | Intentionally inert placeholders |
| Packaging | Unsigned 0.1.0 dry-run EXE and installer were built from the pre-merge tree into `target\release` and `installer\output`; rebuild from the final commit |

## Next (finalization order)

1. Merge remaining feature branches in dependency order. Run the full gate after
   each merge. Resolve conflicts in `STATUS.md`, `ROADMAP.md`, `CHANGELOG.md`,
   and `docs/` deliberately; this skeleton is the intended shape. The release
   prep, walkthrough, and docs lanes are merged into this release branch.
2. Fill every `TODO-VERIFY` in this file, `docs/PHASE8_ACCEPTANCE.md`,
   `docs/PHASE9_ACCEPTANCE.md`, `docs/ui/PHASE9_*.md`, `CHANGELOG.md`, and
   `RELEASE.md` from the final gate output.
3. Run Phase 8 owner acceptance steps 1-9 with a real key and workspace
   (`docs/PHASE8_ACCEPTANCE.md`). The release EXE was only smoke-launched
   pre-merge; no human visual or interaction check ran.
4. Run the owner manual walkthrough (`PHASE_Z_TAURI_PROMPT.md` lines 707-726)
   through `scripts/walkthrough.ps1` and `docs/WALKTHROUGH.md`: Open Folder,
   dirty-tab save/discard, LSP hover/definition with `rust-analyzer` on PATH,
   agent read/write + full auto-apply, agent approval for bash, Settings
   persistence, resize/restart.
5. Run the Phase 9 acceptance checklist that applies to the merged waves. Mark
   unbuilt waves **NOT RUN / NOT IMPLEMENTED**, never passed.
6. Build the release EXE and installer from the final commit. Record sizes,
   SHA-256 values, and Authenticode state. Sign only if the owner supplied the
   certificate secrets.
7. Install on a clean Windows 11 VM. Repeat without WebView2 on Windows 10 if
   the runtime branch changed. Record the result.
8. Update `ROADMAP.md` phase status and this restart point. Commit only after
   the documentation matches the merged tree.

## Blockers (owner decisions or human-only)

1. **No signing certificate.** `SIGNING_CERT_BASE64` / `SIGNING_CERT_PASSWORD`
   are owner-only. Artifacts stay `NotSigned` until then.
2. **Upstream repository not published.** Local git history exists (no remote
   configured), but `github.com/ToxicAngelX/Selora-IDE` returns HTTP 404. The
   tag merge gate needs `origin/main`, and the About dialog and Rust update
   check need a published release for
   `api.github.com/repos/<owner>/<repo>/releases/latest`. Create/push the
   repository, or change the slug in the four sources that
   `scripts/check-release-metadata.ps1` enforces. Publishing v0.1.0 resolves
   the 404.
3. **No clean-VM run.** WebView2-missing install, per-user install, file
   association, launch, and uninstall remain unverified.
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
- `target/release/selora-desktop.exe` and
  `installer/output/Selora-IDE-Setup-0.1.0.exe` exist as unsigned dry-run
  artifacts (gitignored); rebuild both from the final commit.
  `bundle.active=false`, so the Inno script remains the packaging path.
- Pre-merge evidence lives in git history; it is not evidence for the merged
  tree.
