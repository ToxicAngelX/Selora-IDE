# Phase 8 acceptance and packaging record

> **STATUS: FINALIZATION SKELETON (2026-10-27).**
> Automated checks on the merged tree: **NOT RUN** - `TODO-VERIFY`.
> Owner acceptance: **UNRUN**.
> Artifacts: **UNSIGNED**, not rebuilt from the final merge - `TODO-VERIFY`.
> Clean-VM install: **NOT RUN**.
> Phase 8 is **not accepted**.
>
> Replace every `TODO-VERIFY` from the final gate output after the merge. Do
> not copy counts, sizes, dates, or hashes from an earlier tree. The pre-merge
> record that this skeleton replaces remains in git history at commit
> `6634f8a`; its numbers are **superseded** and must not be cited as final.

Phase: `ROADMAP.md` Phase 8 - Workflows (dynamic multi-agent orchestration).

This record covers only what the final machine executed: the release build,
the headless acceptance script, the targeted suites, the `.selora` journal path
contract, and an installer compile. It does not claim a live-key run, a visible
UI pass, a clean-VM install, or a crash/relaunch observation.

## 1. Finalization fields

| Field | Value |
|---|---|
| Final commit | `TODO-VERIFY` |
| Branch / tag | `TODO-VERIFY` |
| Documentation date | `TODO-VERIFY` |
| Workspace version | `TODO-VERIFY` (current manifest says `0.1.0`; confirm all three manifests agree) |
| Gate log directory | `TODO-VERIFY` (`%TEMP%\selora-phase8-acceptance-<timestamp>` expected) |
| Merged follow-up branches | `TODO-VERIFY` (`integration`, LSP, settings, session, 9a) |

## 2. Tree state and method

- Repository: `TODO-VERIFY` (record the final worktree path).
- Tree must be **clean** before the gate. Record `git status --short` output or
  state "clean".
- Record whether any other session edited the tree during the gate. If yes,
  bound the edits by commit and re-run.
- No application behavior may be modified to make a check pass.
- Pre-merge evidence is not valid for this record. Re-run every command.

## 3. Machine and toolchain

| Item | Value |
|---|---|
| OS | `TODO-VERIFY` |
| PowerShell | `TODO-VERIFY` |
| Node / npm | `TODO-VERIFY` |
| Rust / Cargo | `TODO-VERIFY` |
| MSVC | `TODO-VERIFY` |
| WebView2 runtime | `TODO-VERIFY` |
| Inno Setup | `TODO-VERIFY` (path and version) |

## 4. Release build

### 4.1 Frontend

```powershell
npm.cmd --prefix apps/selora-desktop/frontend run build
```

| Field | Value |
|---|---|
| Exit | `TODO-VERIFY` |
| Elapsed | `TODO-VERIFY` |
| `dist` file count / bytes | `TODO-VERIFY` |
| `index.html` SHA-256 | `TODO-VERIFY` |
| Warnings | `TODO-VERIFY` (record any chunk-size warning verbatim) |

### 4.2 Release executable

```powershell
cargo clean -p selora-desktop --release
cargo build --release -p selora-desktop --locked
```

The clean is required after a `dist` change so the final frontend is embedded.

| Field | Value |
|---|---|
| Exit | `TODO-VERIFY` |
| Elapsed | `TODO-VERIFY` |
| Path | `target\release\selora-desktop.exe` |
| Size | `TODO-VERIFY` |
| SHA-256 | `TODO-VERIFY` |
| Built | `TODO-VERIFY` |
| Version | `TODO-VERIFY` |
| Authenticode | `TODO-VERIFY` (expected `NotSigned` until the owner supplies a certificate) |
| Staged DLLs | `TODO-VERIFY` (`vcruntime140.dll`, `vcruntime140_1.dll` expected) |
| WebView2 bootstrapper | `TODO-VERIFY` (path and signature) |

### 4.3 Shell smoke test (not a UI pass)

```powershell
$p = Start-Process target\release\selora-desktop.exe -PassThru
Start-Sleep -Seconds 8
# $p.HasExited => TODO-VERIFY
Stop-Process -Id $p.Id -Force
```

Record the boot log lines for that PID. State plainly: **no human saw the
window, no IPC round-trip was exercised, and no UI state was verified.**

## 5. Installer

Compile only. Do not describe the output as a released artifact.

```powershell
& "<ISCC path>" /Qp /DMyAppVersion=<version> `
  "/DMyTargetDir=$((Resolve-Path target\release).Path)" `
  "/DMyCrtDlls=vcruntime140.dll,vcruntime140_1.dll" `
  "/O<output directory>" installer\selora-ide.iss
```

| Field | Value |
|---|---|
| ISCC path / version | `TODO-VERIFY` |
| Exit | `TODO-VERIFY` |
| Elapsed | `TODO-VERIFY` |
| Output path | `TODO-VERIFY` |
| Size | `TODO-VERIFY` |
| SHA-256 | `TODO-VERIFY` |
| Product version | `TODO-VERIFY` |
| Authenticode | `TODO-VERIFY` (expected `NotSigned`) |
| Installed | **NO** |
| Clean-VM run | **NO** |

The WebView2-missing branch, per-user install, `.txt` association, launch
checkbox, and uninstall behavior remain unverified.

## 6. Automated acceptance helper

`scripts/phase8-acceptance.ps1` is PowerShell 5.1 compatible. It:

1. checks the release exe exists and prints path, size, and SHA-256;
2. runs the targeted suites:

   ```powershell
   cargo test -p selora-workflow --locked
   cargo test -p selora-protocol --locked workflow
   cargo test -p selora-desktop --locked workflow
   cargo test -p selora-workflow --locked ambient_globals_are_structurally_absent
   npm.cmd --prefix apps/selora-desktop/frontend run test -- src/features/workflows
   ```

3. checks the journal contract: `JOURNAL_DIR` / `JOURNAL_FILE` constants, the
   persisted-execution layout assertion, `.gitignore`'s `/.selora/` entry, and
   whether local run journals exist yet;
4. reports installer prerequisites (iss, ISCC, bootstrapper cache, built setup);
5. prints ROADMAP Phase 8 steps 1-9 with `AUTOMATED: PASS|FAIL|SKIP` and an
   explicit `OWNER: OWNER` column plus the observation the owner must make.

Usage:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1 -Build
```

### 6.1 Result on the final tree

| Field | Value |
|---|---|
| Started | `TODO-VERIFY` |
| Log directory | `TODO-VERIFY` |
| Automated checks | `TODO-VERIFY` (passed / failed / skipped) |
| Roadmap owner rows pending | `TODO-VERIFY` (expected 9) |
| Exit code | `TODO-VERIFY` |

Paste the script's `Summary` block verbatim. Do not retype counts from memory.

SKIP rows never mean pass. Explain each SKIP (for example, no live journal
until an owner run, or no `installer/output` artifact).

## 7. ROADMAP Phase 8 steps 1-9

Automated column = what the final machine proved headlessly. Owner column =
the observation still required with a real key, a real workspace, and a visible
window. Every owner row is **UNRUN** until the owner records it.

| Step | Automated evidence (final run) | Owner must observe (UNRUN) |
|---|---|---|
| 1. Agent proposes `run_workflow`; card shows script + commands | `TODO-VERIFY` | Send "review every Rust file in src/ for unwrap() calls; report the worst five"; read the card's script body, command set, and cost cap |
| 2. Approve -> phase graph, fan-out, live cards | `TODO-VERIFY` | Approve; watch phases appear, subagents fan out, step status update live |
| 3. Edit parks on approval card; Deny continues | `TODO-VERIFY` | Deny a write card; confirm the run continues and the file is unchanged |
| 4. Kill mid-run; relaunch; Resume finishes | `TODO-VERIFY` | Kill `selora-desktop.exe` mid-run; relaunch; Resume; confirm no finished ask is re-billed |
| 5. Failing `world.run` gate shows the fix round | `TODO-VERIFY` | Run a gate that fails; watch the retry round in the panel |
| 6. Stop ends the run and kills in-flight children | `TODO-VERIFY` | Click Stop during bash work; confirm quick exit and no surviving child processes |
| 7. Cost meter/summary reflects every ask | `TODO-VERIFY` | Compare summary tokens/USD with receipts; check the workflow budget cap |
| 8. Journal under `.selora/workflow-runs/`, never in git | `TODO-VERIFY` | After a real run check `.selora/workflow-runs/<run_id>/journal.jsonl`; with a real repository check `git status` stays clean |
| 9. `fetch`/`fs`/`process` absent at the facade | `TODO-VERIFY` | Paste a script that calls them; confirm a facade error, not an effect |

Headless green does not prove that a real workflow reaches the gateway, that a
window paints, that approvals are clickable, or that a crash/relaunch resumes.

## 8. Known limits

1. **Unsigned artifacts.** Expect `NotSigned` until `SIGNING_CERT_BASE64` /
   `SIGNING_CERT_PASSWORD` are configured. SmartScreen and enterprise policy
   will treat both artifacts as unknown publisher.
2. **No clean-VM install.** The WebView2-missing path, per-user install, file
   association, launch checkbox, and uninstall are unverified.
3. **Update check returns 404.** The About dialog queries
   `api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest`; the public
   repository or release does not exist yet.
4. **No final VCS evidence yet.** Fill the final commit and confirm
   `git status --short` is clean. The journal must be verified with a real
   `git status`, not only by reading `.gitignore`.
5. **Inno Setup location.** If ISCC exists only under `%TEMP%`, a cleanup can
   remove it and `installer\prebuild.bat` will not find it. Install durably;
   prefer `winget install JRSoftware.InnoSetup`.
6. **Headless coverage is not runtime evidence.** The targeted suites run
   without a live key. Steps 1-9 remain required by the ROADMAP "DONE means"
   rule.
7. **Grandchild kill is test-covered, not owner-verified.** The child is created
   suspended and assigned to the Job Object before it runs; live behavior after
   Stop or timeout remains step 6.
8. **Resume effects are not exactly-once.** An unfinished ask may already have
   applied tool effects when the owner chooses **Resume anyway**.
9. **Dirty buffers are preserved, not merged.** A workflow reload reloads clean
   buffers only; dirty tabs stay dirty with a toast.
10. **Completed history depends on the `integration` branch.** If that branch is
    not merged, history and explicit phases/reports/artifacts are absent. Mark
    the merge state before accepting.

## 9. Reproduce

```powershell
# 1. Frontend + release shell (final artifact)
npm.cmd --prefix apps/selora-desktop/frontend run build
cargo clean -p selora-desktop --release   # only needed after a dist change
cargo build --release -p selora-desktop --locked

# 2. Automated acceptance (0 = automated checks green)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1

# 3. Installer smoke (temporary or owner-chosen output)
& "<ISCC path>" /Qp /DMyAppVersion=<version> `
  "/DMyTargetDir=$((Resolve-Path target\release).Path)" `
  "/DMyCrtDlls=vcruntime140.dll,vcruntime140_1.dll" `
  "/O<output directory>" installer\selora-ide.iss
```

Owner acceptance order: run the script once for a baseline, then perform
ROADMAP steps 1-9 in order and record the observations in this file. Do not
mark Phase 8 accepted from the headless result alone.

## 10. Finalization checklist

- [ ] Final commit recorded; tree clean
- [ ] Follow-up branch merge state recorded
- [ ] Full workspace gate recorded in `STATUS.md`
- [ ] Phase 8 acceptance script re-run; Summary pasted verbatim
- [ ] Release EXE rebuilt; size + SHA-256 recorded
- [ ] Installer rebuilt; size + SHA-256 recorded
- [ ] Authenticode state recorded for both artifacts
- [ ] Owner steps 1-9 performed and observed
- [ ] Clean-VM install performed (or explicitly left **NOT RUN**)
- [ ] Remaining limits reviewed; no limit silently dropped
- [ ] `ROADMAP.md` Phase 8 status updated
- [ ] `CHANGELOG.md` entries moved out of `Unreleased` only after the gate