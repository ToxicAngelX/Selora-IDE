# Status - 2026-10-27

> Restart point. Read this, then `AGENT_ORCHESTRATION.md` section 10.
> The pre-Tauri STATUS is archived as `STATUS_ARCHIVE_2026-10-09.md`; the
> per-slice audit and its hardening addendum live in
> `docs/ui/PHASE_Z_PARITY_REPORT.md`.

## Last completed

Phase Z7 hardening plus **Phase 8 waves 2–4**: execution, approvals, durable
replay, receipt accounting, process-tree containment, explicit unfinished-ask
resume consent, live buffer refresh, completed-run history, and a release build
plus installer. A first-run freeze was diagnosed and fixed: the onboarding
WebGL effect hung WebView2, and duplicate launches raced for one WebView2
profile. Brand surfaces are CSS-only and the app is single-instance. Automated
work is green; Phase 8 acceptance waits on the owner’s real-key steps.

Branch: **none** - this checkout has no `.git` directory, so nothing could be
committed, branched, or diffed. All work is in the working tree only.
Integrated: 2026-10-27 (working tree, no VCS)
Automated baseline: **green** (numbers below). Manual acceptance is unrun.

## Last completed gate (wave 4, end of session)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean, exit 0 |
| `cargo test --workspace --no-fail-fast` | **1121 passed, 0 failed** (4 ignored), 29 suites |
| `npm run typecheck` | clean, exit 0 |
| `npm test` | **466 passed, 0 failed**, 51 files |
| `npm run build` | green |
| `cargo deny check` | **advisories ok, bans ok, licenses ok, sources ok** |
| `scripts/phase8-acceptance.ps1` | **13 passed, 0 failed, 2 skipped**; exit 0 |

### First-run freeze fix (verified)

1. With a fresh WebView2 profile the old `img-fx` hero failed shader creation,
   lost its context, and stopped the renderer thread (CDP evaluations timed out).
2. `BrandFx` now always renders the CSS orbital mark; `OnboardingHero` and
   `MetalBrand` load no WebGL module. Fresh-profile verification: overlay
   visible, zero canvases, evaluations answered in 1–2 ms.
3. `tauri-plugin-single-instance` focuses the running window and exits the
   second process. Verified: second launch exited, process count stayed one.
   This prevents the WebView2 setup failure (`0x8007139F`) seen with duplicate
   launches.

A backup of the replaced WebView2 profile remains at
`%LOCALAPPDATA%\lol.selora.ide\EBWebView.bak`; the new profile carries first-run
state.

Wave 4 closed the five recorded gaps at the automated level: Windows Job Object
containment for `world.run`; detected unfinished asks with default-refused resume
and explicit `Resume anyway`; clean-buffer refresh with `BufferSnapshot` events;
completed-run history from the terminal journal report; and a release binary plus
a compiled installer. Every acceptance step still carries an `OWNER` row.

### Packaging artifacts (built this tree, unsigned)

| Artifact | Path | Size | SHA-256 |
|---|---|---|---|
| Release app | `target\release\selora-desktop.exe` | 18,823,168 | `167fcf8b4271bc558b413535dd2757d1d73168b48a1e134376b20ab21a4de50f` |
| Installer | `%TEMP%\selora-phase8-installer\Selora-IDE-Setup-0.1.0.exe` | 9,388,643 | `7a20d776e9d31e9293036f9fb1314d3ec3008bde7fd841ffcdfc616abb806fa1` |

The installer is outside `installer/output/` because ISCC lives in `%TEMP%`.
Both artifacts are `NotSigned`. Release build process smoke: window created and
shown; no visual/manual UI acceptance was performed.

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

## Phase 8 status (Workflows, waves 2–4)

| Slice | State |
|---|---|
| 8a engine core | **done**: `crates/selora-workflow` is a root-workspace member with boa 0.22 (`default-features=false`), hardened realm (no fetch/fs/process/timers/Date/eval/random), 64 KiB script + 256 step caps, literal-only `world.run` command extraction, schema-validated `ask`, `prepare`/`run_workflow`; 56 unit + 2 integration + 1 doctest |
| 8b journal/budget/subagents | **done on the run path**: durable spec + JSONL journal, index-keyed replay, strict damaged-journal refusal, torn-tail recovery, exclusive run lock, restored spend/tokens, named-agent FIFO/concurrency |
| 8c protocol/bridge | **done**: run/resume/stop/decision commands, seven events, origin workspace id, bootstrap listing with completed history, old JSON defaults |
| 8c Host | **done**: root-contained files/git, Job Object tree kill for `world.run`, real §17 agent turns with tools, lossless history, explicit approvals, receipts |
| 8c dispatch | **done**: starts/stops/resumes, answers live approvals, unfinished-ask consent gate, single terminal event |
| 8c tool/card | **done**: manifest v2, strict validation, full workflow payload, mounted card, exactly-once launch |
| 8d panel | **done**: live events, exact approval cards, retry-safe actions, safe Resume, `Resume anyway` confirmation, completed-run history |
| 8d buffers | **done**: terminal event reloads clean buffers, emits `BufferSnapshot`, preserves dirty tabs with a toast, refuses cross-workspace refresh |
| 8d shell/docs/package | **done**: lazy route, release EXE, installer compiled under `%TEMP%`, `docs/PHASE8_ACCEPTANCE.md`, automation script |

**Still pending (honest gaps):**

1. Job assignment happens just after `CreateProcess`, so a descendant spawned in
   that sub-startup window could escape; everything after assignment is killed.
2. “Resume anyway” is explicit, but tools in a previously unfinished ask may
   already have applied effects. Exactly-once subagent side effects are not
   claimed.
3. Dirty editor buffers are intentionally not overwritten; LSP documents are not
   re-sent after a clean-buffer reload. Budget-tripped runs are only pre-refused
   by the listing, not the direct resume handler.
4. Persisted history comes from the terminal journal report: per-step phases are
   unknown and reports reconstruct only from successful, parsable report steps.
5. Owner acceptance steps 1–9 remain unrun; no clean-VM install, no signing, no
   update check (404), and no `.git` history.

## What is real now

| Slice | State |
|---|---|
| Shell / layout / persistence | Real: seven regions, drag + keyboard resize, layout restore |
| Workspace / files / editor | Real: folder picker, tree, 2 MiB UTF-8 open, tabs, save, find/replace, minimap |
| Search / Git / Problems | Real: literal + regex search, stage/unstage/commit, staged/unstaged diffs, live diagnostics |
| LSP | Real for Rust and TS/JS from `PATH`; `.cmd` shims handled; no settings path overrides in the desktop bridge |
| Agent Mode | Real tools, streaming, cards, approvals, stop; **Full auto-apply** for file tools in the session; plan-only arms read tools only |
| Workflows (Phase 8) | Engine, Host, approvals, durable resume/history, buffer refresh, live panel, and confirmation card are wired; owner acceptance pending |
| Settings | Theme + reduced motion persist through Rust; every other control is disabled/read-only |
| Terminal / Output | Intentionally inert placeholders |
| Packaging | Release EXE and an unsigned installer were built from this tree; artifacts live in `target\release` and `%TEMP%`, not `installer/output` |

## Next

1. **Phase 8 owner acceptance:** run roadmap steps 1–9 with a real key and
   workspace. The release EXE was smoke-launched (window created/shown), but no
   human visual or interaction check ran. Use `docs/PHASE8_ACCEPTANCE.md`.
2. **Owner-run manual walkthrough** (`PHASE_Z_TAURI_PROMPT.md` lines 707-726):
   Open Folder, dirty-tab save/discard, LSP hover/definition with
   `rust-analyzer` on PATH, agent read/write + full auto-apply, agent approval
   for bash, Settings persistence, resize/restart.
3. **Clean-VM installer test** — Windows 11, plus Windows 10 without WebView2.
   The compiled installer is unsigned and currently under `%TEMP%`.
4. **Session lifecycle** remains the main functional gap:
   `SwitchSession` / `NewSession` / `RetryMessage` are still local-only or
   `unsupported_phase_slice`. The design is recorded in the audit; implement it
   before/with Phase 9, or scope the rail explicitly as local.
5. **Phase 9 (Agent Workbench)** follows Phase 8; do not start it until
   Phase 8 is wired and accepted.

## Blockers (owner decisions or human-only)

1. **No `.git` in this checkout.** Nothing was committed. If history/rollback
   matter, initialise a repository (or point the folder at
   `github.com/ToxicAngelX/Selora-IDE`) before more parallel work.
2. **Update check returns 404.** The About dialog queries
   `api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest`; publish the
   repo or change the slug.
3. **Five of seven Libraries.dev packages are not installed**
   (`border-beam`, `thinking-orbs`, `bot-avatars`, `liquid-gooey`,
   `voice-glow`). The tree uses local ports; decide install-and-verify or
   accept the ports as the shipped implementation.
4. **Signing certificate** for `SIGNING_CERT_BASE64` / `SIGNING_CERT_PASSWORD`;
   artifacts are unsigned until then.
5. **Session lifecycle and the single-theme selector** remain documented gaps.
6. **Full auto-apply is the default composer mode.** File writes apply without
   a card for the session; bash still cards. Confirm this is the intended
   default, or switch the default back to Ask before changes.

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
  were already ignored.
- No `target/release/selora-desktop.exe` and no `installer/output/` exist in
  this checkout; `bundle.active=false`, so the Inno script is the packaging
  path.
