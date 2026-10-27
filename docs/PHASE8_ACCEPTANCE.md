# Phase 8 acceptance and packaging record

Date: 2026-10-27 (host local time).
Phase: ROADMAP.md Phase 8 - Workflows (dynamic multi-agent orchestration).
Result: **automated checks PASS; owner acceptance UNRUN.** Phase 8 is not accepted.

This record covers only what this machine actually executed: the release build, a
headless acceptance script, the targeted test suites, the `.selora` journal path
contract, and an installer smoke compile. It does not claim a live-key run, a
visible UI pass, a clean-VM install, or a crash/relaunch observation.

## 1. Tree state and method

- Repository: `C:\Users\Toxic\Desktop\Selora IDE`
- Working tree only. The checkout has **no `.git` directory**, so nothing was
  committed, diffed, or tagged, and `git status` cannot confirm `.gitignore`.
- Another session was editing `crates/selora-workflow`, `apps/selora-desktop`
  and the frontend while this acceptance started (file mtimes 05:41-06:09).
  The first release build below failed on that in-flight state. The final
  artifact was built at **06:19:16** after the tree had been quiet for about
  nine minutes; no Rust, TypeScript, protocol, STATUS or ROADMAP file was
  changed by this acceptance work.
- No application behavior was modified to make a check pass.

## 2. Machine and toolchain

| Item | Value |
|---|---|
| OS | Microsoft Windows 11 Pro 10.0.26200, build 26200, AMD64 |
| PowerShell | 5.1.26100.9168 (Windows PowerShell) |
| Node / npm | v24.19.0 / 11.17.0 |
| Rust / Cargo | rustc 1.99.0 (b940084d7 2026-09-28) / cargo 1.99.0 |
| MSVC | Visual Studio Build Tools 2022, MSVC 14.44.35207, redist 14.44.35112 |
| WebView2 runtime | 151.0.4129.78 (HKLM WOW6432Node EdgeUpdate client) |
| Workspace version | 0.1.0 (root `Cargo.toml`) |

## 3. Release build

### 3.1 Frontend

Command (from the repository root):

```powershell
npm.cmd --prefix apps/selora-desktop/frontend run build
```

| Attempt | When | Exit | Elapsed | Result |
|---|---|---|---|---|
| initial | 2026-10-27 05:51 | 0 | 14.9 s | `tsc -b && vite build`, 1142 modules; one >500 kB chunk warning |
| final (after the frontend changed during the session) | 2026-10-27 06:16:45 | 0 | 15.1 s | `dist` rewritten; vite reported `built in 9.99s` |

Final `dist`: 29 files, 2,971,714 bytes, `index.html` SHA256
`033175cbd2b640f5e480c9404b85d6acbf714ebcabafdfd9416a0487728f9a6d`.
The chunk-size warning is pre-existing and non-fatal.

### 3.2 Release executable

Command (from the repository root):

```powershell
cargo build --release -p selora-desktop --locked
```

| Attempt | When | Exit | Elapsed | Result |
|---|---|---|---|---|
| 1 | 2026-10-27 05:38-05:41 | **101** | 183.9 s | `selora-workflow` did not compile; see the blocker below |
| 2 | 2026-10-27 06:14-06:16 | 0 | 137.8 s | release binary produced |
| 3 (final) | 2026-10-27 06:17-06:19 | 0 | 131.2 s | `cargo clean -p selora-desktop --release`, then build, so the new frontend `dist` is embedded |

Attempt 1 failed while another session was mid-edit in
`crates/selora-workflow/src/persisted.rs` and `src/lib.rs`:

```text
error[E0425]: cannot find value `spent_usd` in this scope      (persisted.rs:413, 420)
error[E0425]: cannot find value `tokens` in this scope         (persisted.rs:421)
error[E0599]: no variant named `IncompleteAsks` found for enum `WorkflowError`
```

The same files were modified at 05:41:38 and 05:41:49, during the build. A retry
after the edits settled compiled without any intervention here. This is recorded
as a transient concurrent-tree blocker, not as a defect in the final artifact.

Final artifact:

| Field | Value |
|---|---|
| Path | `C:\Users\Toxic\Desktop\Selora IDE\target\release\selora-desktop.exe` |
| Size | 18,978,304 bytes (18.10 MiB) |
| SHA256 | `e89b8c6ddb818e7328013ebb19784a87066aa73a864bd87aef3eb95ee9cc5a06` |
| Built | 2026-10-27 06:19:16 |
| Version | 0.1.0 |
| Authenticode | `NotSigned` |
| Staged beside it | `vcruntime140.dll` (124,544 B), `vcruntime140_1.dll` (49,792 B), `MicrosoftEdgeWebview2Setup.exe` (1,888,472 B) |

The exe's import table lists only `vcruntime140.dll` and `vcruntime140_1.dll`
from the MSVC runtime, so those are the only CRT DLLs staged and passed to Inno
Setup. `dumpbin /dependents` reported no `MSVCP140*`, `CONCRT140`, or `VCOMP140`.

### 3.3 Shell smoke test (not a UI pass)

`tauri.conf.json` has `devUrl: http://localhost:1420`, but that URL is used only
in debug builds. `main.rs` selects `WebviewUrl::App("index.html")` under
`#[cfg(not(debug_assertions))]`, and the crate enables Tauri's
`custom-protocol` feature, so the release exe serves the embedded `dist` and
does not need the Vite dev server. No devUrl blocker was found.

A bounded process smoke was run:

```powershell
$p = Start-Process target\release\selora-desktop.exe -PassThru
Start-Sleep -Seconds 8
# $p.HasExited => False
Stop-Process -Id $p.Id -Force
```

The boot log (`%TEMP%\selora_boot.log`) recorded for that PID:

```text
[pid 24236] main() started
[pid 24236] DesktopState created
[pid 24236] setup() started
[pid 24236] Window built successfully!
[pid 24236] Window shown & focused!
```

The process stayed alive for 8 s and was killed by the test. **No human saw the
window, no IPC round-trip was exercised, and no UI state was verified.** The
stale `%TEMP%\selora_last_panic.log` (2026-10-26 09:47, WebView2
`0x800700AA`) predates this run and is not from this binary.

## 4. Installer

### 4.1 Prerequisites found on this machine

| Prerequisite | Result |
|---|---|
| `installer\selora-ide.iss` | present |
| Inno Setup compiler | **not on PATH** and not in the standard install directories; found via the uninstall registry at `C:\Users\Toxic\AppData\Local\Temp\innosetup-6.7.3\ISCC.exe`, version 6.7.3 |
| `installer\prebuild.bat` | present but cannot locate that ISCC copy: it probes only `%ProgramFiles%`, `%ProgramFiles(x86)%`, and `%LOCALAPPDATA%\Programs`. The manual equivalent below was used. |
| Visual Studio / `dumpbin` | present: `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools` |
| WebView2 runtime | installed: 151.0.4129.78 |
| WebView2 bootstrapper cache | **absent** in `target\release`; downloaded for this build (1,888,472 bytes, Authenticode `Valid`, signer `CN=Microsoft Corporation`, SHA256 `ca4c9016c70e2184c5e231662e0e98abf4826647180ef3faab32fcb1ca2ec15e`) |

Important: the only registered Inno Setup install lives under `%TEMP%`. Windows
or a cleanup tool can remove it. A durable owner/CI install is
`winget install JRSoftware.InnoSetup` (6.3+ or 7.x).

> **Resolved 2026-10-27:** `installer/prebuild.bat` now resolves ISCC through
> `installer/find-iscc.ps1` (SELORA_ISCC, PATH, standard directories, then the
> uninstall registry) and passes an absolute `/O`, so this `%TEMP%` compiler is
> found and the setup lands in `installer/output/`. The manual `/O` override
> below is kept as a record of the original run.

### 4.2 Staging and compile

`prebuild.bat` was not run because it would have exited at the ISCC lookup after
staging. Its documented steps were reproduced: dumpbin-derived CRT staging, then
ISCC with `/DMyAppVersion`, `/DMyTargetDir`, `/DMyCrtDlls`.

```powershell
ISCC.exe /Qp /DMyAppVersion=0.1.0 `
  "/DMyTargetDir=C:\Users\Toxic\Desktop\Selora IDE\target\release" `
  "/DMyCrtDlls=vcruntime140.dll,vcruntime140_1.dll" `
  "/OC:\Users\Toxic\AppData\Local\Temp\selora-phase8-installer" `
  installer\selora-ide.iss
```

| Field | Value |
|---|---|
| Exit | 0 |
| Elapsed | 4.7 s |
| Output | `C:\Users\Toxic\AppData\Local\Temp\selora-phase8-installer\Selora-IDE-Setup-0.1.0.exe` |
| Size | 9,550,461 bytes |
| SHA256 | `4ab21c662be0ce4576d4e5194eb8b20845908ab8ed1aa83f34718bc0fefab3c8` |
| Product version | 0.1.0 |
| Authenticode | `NotSigned` |
| `installer\output` | absent; the `/O` override kept the repository clean |

This is a compile-only smoke artifact. It was **not installed** and **not run on
a clean VM**. The WebView2-missing branch, per-user install, `.txt`
association, and uninstall behavior remain unverified.

## 5. Automated acceptance helper

New file: `scripts/phase8-acceptance.ps1` (PowerShell 5.1 compatible, no PS7
syntax). It:

1. checks the release exe exists and prints its path, size and SHA256;
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

Exit code is 0 when every automated check passes; 1 when any automated check
fails. SKIP and OWNER rows never change the exit code, and no live-key or UI row
is ever marked as an automated pass.

Usage:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1 -Build
```

Test logs are written to `%TEMP%\selora-phase8-acceptance-<timestamp>\` and are
not added to the repository.

### 5.1 Run on this machine

Command and result:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1
# SCRIPT_EXIT=0
```

Started 2026-10-27 06:19:22. Logs:
`C:\Users\Toxic\AppData\Local\Temp\selora-phase8-acceptance-20261027-061922`.

```text
[PASS] release-exe - ...\target\release\selora-desktop.exe (18.1 MB, SHA256 e89b8c6d...)
[PASS] test.engine - cargo test -p selora-workflow: 85 passed, 0 failed, 0 ignored, 1.5s
[PASS] test.protocol - cargo test -p selora-protocol workflow: 6 passed, 0 failed, 0.3s
[PASS] test.desktop - cargo test -p selora-desktop workflow: 64 passed, 0 failed, 13s
[PASS] test.sandbox-absence - ambient_globals_are_structurally_absent: 1 passed, 0.5s
[PASS] test.frontend - vitest src/features/workflows: 35 passed, 0 failed, 2.7s
[PASS] journal.path-constants - journal.rs declares .selora/workflow-runs + journal.jsonl
[PASS] journal.layout-test - persisted_execution covers .selora/workflow-runs/<id>/journal.jsonl
[PASS] journal.gitignore - .gitignore ignores /.selora/
[SKIP] journal.workspace-runs - no local .selora/workflow-runs yet (expected until an owner test run)
[PASS] installer.script - ...\installer\selora-ide.iss
[PASS] installer.iscc - C:\Users\Toxic\AppData\Local\Temp\innosetup-6.7.3\ISCC.exe (version 6.7.3)
[SKIP] installer.webview2-bootstrapper - not cached at ...\target\release\... (downloaded manually afterwards)
[SKIP] installer.artifact - installer/output absent (no installer compiled in this checkout)
```

Summary printed by the script:

```text
Summary
  automated checks : 12 passed, 0 failed, 3 skipped
  roadmap steps    : 0 with an automated failure, 9 owner observations pending
RESULT: automated checks passed. Phase 8 acceptance still requires the OWNER steps.
```

Notes on the SKIP rows:

- `journal.workspace-runs` is expected to be empty until a real workflow runs.
  The path contract itself is covered by the constants, the layout test, and the
  85-test engine suite.
- `installer.webview2-bootstrapper` was SKIP during the script run, then the
  bootstrapper was downloaded and staged for the manual ISCC compile in §4.
- `installer.artifact` stays SKIP for `installer/output`; the smoke setup was
  written to `%TEMP%` by design. The script intentionally does not treat a temp
  artifact as a repository release artifact.

The suites above are headless and run without any live key. They do not prove
that a real workflow run reaches the gateway, that a window paints correctly,
that approvals are clickable, or that a crash/relaunch resumes.

A second run after the WebView2 bootstrapper was staged (2026-10-27 06:23:06,
`SCRIPT_EXIT=0`) reported **13 passed, 0 failed, 2 skipped**:
`installer.webview2-bootstrapper` became PASS, while
`journal.workspace-runs` and `installer.artifact` remained SKIP for the reasons
above. The ROADMAP step statuses were unchanged.

## 6. ROADMAP Phase 8 steps 1-9

Automated column = what this machine proved headlessly. Owner column = the
observation still required with a real key, a real workspace and a visible
window. Every owner row is **UNRUN** as of this record.

| Step | Automated evidence (this run) | Owner must observe (UNRUN) |
|---|---|---|
| 1. Agent proposes `run_workflow`; card shows script + commands | **PASS** - engine `prepare`/command-extraction tests, desktop `workflows_agent` tool/card tests, frontend 35 tests incl. confirmation card | Send "review every Rust file in src/ for unwrap() calls; report the worst five"; read the card's script body, command set and cost cap |
| 2. Approve -> phase graph, fan-out, live cards | **PASS** - engine event-sequence fixture, desktop host/dispatch tests, panel/bridge tests | Approve; watch phases appear, subagents fan out, step status update live |
| 3. Edit parks on approval card; Deny continues | **PASS** - `proposed_write_parks_deny_does_not_write...`, approval retry/stale-id tests | Deny a write card; confirm the run continues and the file is unchanged |
| 4. Kill mid-run; relaunch; Resume finishes | **PASS** - journal replay/resume tests, resume preflight, bootstrap resume | Kill `selora-desktop.exe` mid-run; relaunch; Resume; confirm no finished ask is re-billed |
| 5. Failing `world.run` gate shows the fix round | **PASS** - engine gate/exit-code branch tests, desktop `world.run` timeout/cancel tests | Run a gate that fails; watch the retry round in the panel |
| 6. Stop ends the run and kills in-flight children | **PASS** - desktop cancel tests incl. Job Object grandchild-kill tests | Click Stop during bash work; confirm quick exit and no surviving child processes |
| 7. Cost meter/summary reflects every ask | **PASS** - receipt/cost/token tests, budget-abort tests | Compare summary tokens/USD with receipts; check the workflow budget cap |
| 8. Journal under `.selora/workflow-runs/`, never in git | **PASS** - constants + layout test + `.gitignore` `/.selora/`; live dir absent so far | After a real run check `.selora/workflow-runs/<run_id>/journal.jsonl`; with a real repository check `git status` stays clean |
| 9. `fetch`/`fs`/`process` absent at the facade | **PASS** - `ambient_globals_are_structurally_absent` + realm-hardening tests | Paste a script that calls them; confirm a facade error, not an effect |

## 7. Known limits

1. **Unsigned artifacts.** Both `selora-desktop.exe` and the setup report
   Authenticode `NotSigned`. `SIGNING_CERT_BASE64` / `SIGNING_CERT_PASSWORD` are
   not configured here. SmartScreen and enterprise policy will treat these as
   unknown publisher.
2. **No clean-VM install.** This machine already has WebView2 151.0.4129.78 and
   the MSVC runtime. The installer's missing-WebView2 path, per-user install,
   file association, launch checkbox and uninstall were not exercised.
3. **Update check returns 404.** The About dialog queries
   `api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest`; the repository
   or release does not exist publicly.
4. **No `.git`.** Nothing was committed. `.gitignore` is checked textually only.
   The Phase 8 journal is verified not to be tracked by that text, not by an
   actual `git status`.
5. **Inno Setup is installed under `%TEMP%`.** `C:\Users\Toxic\AppData\Local\Temp\innosetup-6.7.3\`
   is cleanup-prone and is invisible to `installer\prebuild.bat`. Install Inno
   Setup properly before the owner packaging run.
6. **Headless coverage is not runtime evidence.** All 85 engine, 6 protocol, 64
   desktop and 35 frontend tests run without a live key. Steps 1-9's owner
   observations are still required by the ROADMAP "DONE means" rule.
7. **Concurrent-tree caveat.** The artifact hashes describe the tree at
   2026-10-27 06:19:16. Another session was editing this repository during the
   session; with no VCS there is no diff to bound those edits.
8. **Grandchild kill is test-covered, not owner-verified.** `workflows_process.rs`
   creates the child suspended, assigns it to the job, and resumes it only after
   the assignment; its tests make the child spawn a grandchild as its first
   statement and check job membership plus death on success/timeout/cancel. Live
   process-tree behavior after Stop or timeout remains an owner check (step 6).
9. **Completed-run history** is not part of this record; STATUS.md's Phase 8 gap
   list remains authoritative and was not edited here.

## 8. Reproduce

```powershell
# 1. Frontend + release shell (final artifact)
npm.cmd --prefix apps/selora-desktop/frontend run build
cargo clean -p selora-desktop --release   # only needed after a dist change
cargo build --release -p selora-desktop --locked

# 2. Automated acceptance (0 = automated checks green)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1

# 3. Installer smoke (temporary output; ISCC path from this machine)
& "C:\Users\Toxic\AppData\Local\Temp\innosetup-6.7.3\ISCC.exe" /Qp `
  /DMyAppVersion=0.1.0 `
  "/DMyTargetDir=$((Resolve-Path target\release).Path)" `
  "/DMyCrtDlls=vcruntime140.dll,vcruntime140_1.dll" `
  "/O$env:TEMP\selora-phase8-installer" installer\selora-ide.iss
```

Owner acceptance order (after a real key and workspace are available): run the
script once for a baseline, then perform ROADMAP steps 1-9 in order and record
the observations next to this file. Do not mark Phase 8 accepted from the
headless result alone.

## 9. Post-build freeze fix (2026-10-27, later)

The first runtime launch exposed two startup defects that the headless gate did
not cover:

1. **Onboarding WebGL hang.** `img-fx` created a context successfully but its
   shader failed under this WebView2/GPU combination; the renderer thread then
   stopped answering (CDP evaluations timed out). `BrandFx` now ships the CSS
   orbital mark only and loads no WebGL module. Verified with a fresh WebView2
   profile: overlay visible, `canvas` count 0, evaluations answered in 1–2 ms.
2. **Duplicate-launch WebView2 conflict.** Two processes raced for one profile;
   one died during setup with `0x8007139F`. `tauri-plugin-single-instance` now
   focuses the running window and exits the second process. Verified: second
   launch exited and the process count stayed at one.

Rebuilt artifacts supersede §2/§4:

| Artifact | Size | SHA-256 |
|---|---|---|
| `target\release\selora-desktop.exe` | 18,823,168 | `167fcf8b4271bc558b413535dd2757d1d73168b48a1e134376b20ab21a4de50f` |
| `%TEMP%\selora-phase8-installer\Selora-IDE-Setup-0.1.0.exe` | 9,388,643 | `7a20d776e9d31e9293036f9fb1314d3ec3008bde7fd841ffcdfc616abb806fa1` |

The release was rebuilt again after the File/Edit/Help menus were implemented;
the installer was recompiled against it.

The installer was recompiled against the rebuilt binary. It remains unsigned and
uninstalled. Automated acceptance was rerun after the fix: 13 passed, 0 failed,
2 skipped. Owner UI steps remain unrun.