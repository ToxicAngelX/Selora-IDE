# Automated walkthrough and human checklist

This document covers the functional walkthrough for the Selora IDE desktop
shell. It has two halves:

1. `scripts/walkthrough.ps1` drives a real build over the WebView2 DevTools
   protocol (CDP) and writes evidence.
2. The human checklist below covers the visual, judgement, real-gateway, and
   OS-level steps that automation cannot honestly decide.

**The script never claims human acceptance.** A green run means the listed
behaviours worked on this machine at that time. It does not mean the UI looks
right, that a real model answered, or that the installer is trustworthy.

## Quick start

From the repository root, in Windows PowerShell 5.1:

```powershell
# Use the existing release binary (fails with build instructions if missing)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\walkthrough.ps1

# Build the frontend plus the release binary, then run
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\walkthrough.ps1 -Build

# Dev loop: Vite on 1420 + the debug binary (starts and stops its own Vite)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\walkthrough.ps1 -Mode dev -Build

# Gate CI on recorded product findings as well as failed steps
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\walkthrough.ps1 -FailOnFindings
```

The app opens on screen and is driven with real renderer mouse/key events. Do
not touch the keyboard or mouse while it runs.

Artifacts go to `%TEMP%\selora-walkthrough-<stamp>` unless `-ArtifactsDir` is
given. The console prints the exact path.

## What the automation covers

| Step id | Behaviour | Evidence |
|---|---|---|
| `boot` | Window boots, bootstrap answers, shell renders | screenshot `00-launch.png`, `cdp-result.json` |
| `onboarding` | First-run overlay is dismissed with its Skip control | step detail (present/absent) |
| `workspace-boot` | `SELORA_WORKSPACE_ROOT` is attached and the explorer lists its files | screenshot `01-workspace.png` |
| `workspace-open` | Another folder opens through the same protocol command the folder picker uses, then the fixture is restored | screenshots `02-second-workspace.png` |
| `file-open` | A file opens from the explorer into a tab with CodeMirror content | screenshot `03-file-open.png` |
| `edit-save` | CDP typing reaches the buffer; Ctrl+S clears dirty and the marker is on disk | screenshot `04-edited-saved.png` |
| `dirty-close-guard` | Closing a dirty tab shows Save / Discard / Cancel; Cancel keeps the tab | step detail |
| `tabs` | Second tab opens, tab switch works, a clean tab closes | screenshot `05-tabs.png` |
| `settings` | Settings UI writes `reduced_motion`; bootstrap and `settings.json` agree; value restored | screenshot `06-settings.png` |
| `workflows-smoke` | Workflows route mounts the real panel (not the failure fallback) | screenshot `07-workflows.png` |
| `agent-activity-button` | The Agent activity button opens the dock; a no-op is recorded as a finding | `finding-agent-activity-button.png` |
| `agent-smoke` | Agent dock (opened with the documented Ctrl+L shortcut) mounts its composer; send gate is reported; draft cleared, nothing sent | screenshot `08-agent.png` |
| `runtime-errors` | No `Runtime.exceptionThrown` happened during the run | result JSON |

The driver attaches with:

```text
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port> --remote-allow-origins=*
```

It does not import frontend modules and does not call test helpers. Selectors
come from the shipped accessibility surface (`aria-label`, roles,
`data-testid`) and from the Tauri IPC seam (`get_bootstrap_state`,
`dispatch_command`). The folder-open step deliberately uses
`open_folder_at` because the native folder dialog cannot be driven safely
without an OS-level automation dependency; the command is the same one the
dialog path calls after the user picks a folder.

## Findings versus failures

A **FAIL** means a step's own post-condition did not hold; the run stops there,
exits `1`, and writes a failure screenshot plus page diagnostics.

A **FINDING** is a product behaviour the walkthrough observed, reproduced with
the UI, and worked around so the remaining steps can still run. Findings are
printed and listed at the top of `summary.md`; they do not change the exit code
unless `-FailOnFindings` is passed. A finding is never an acceptance.

Recorded on 2026-10-27 (release build, artifact
`%TEMP%\selora-walkthrough-20261027-094314`):

- `agent-activity-button`: clicking the **Agent** button in the activity bar
  leaves the agent dock closed. `ActivityBar` calls
  `onSelectViewMode('split')`, which already opens the dock, and then calls
  `layoutStore.toggle('agent')` again, closing it. Clicking again repeats the
  double toggle, so the button never opens the dock from editor mode. The
  status-bar Agent pill and **Ctrl+L** both open it (the walkthrough uses
  Ctrl+L). Reproduced with a fresh profile and a warm one. This is an open
  product defect, not accepted as correct.

## Isolation and safety

- `SELORA_SETTINGS_DIR` points inside the artifact directory. The walkthrough
  cannot read or write `%APPDATA%\selora-ide\settings.json`, and layout writes
  stay in the artifact tree.
- The workspace is a generated fixture under the artifact directory unless
  `-Workspace` is supplied. A supplied workspace is never seeded or cleaned.
- No API key is read, set, or sent by the scripts. The app's own boot
  connection probe still runs exactly as in a normal launch: if the OS keyring
  already holds a key, `connect_probe` calls `list_models` on the configured
  gateway and the screenshots show `selora connected`. The walkthrough itself
  never sends chat, agent, or workflow requests. Use a machine/profile without
  a key if the run must be fully offline, but expect the agent send gate to
  report `no_model` instead of a live catalog. The agent draft is typed and
  cleared without pressing Enter; workflows are checked only at panel level.
- The script refuses to start if another `selora-desktop` process is running:
  the Tauri single-instance plugin would forward the launch to that process,
  which has no CDP port.
- On exit it kills only the process tree it started (`taskkill /PID <pid> /T
  /F`) and, in dev mode, its own Vite process. `-KeepRunning` skips this.
- WebView2's own profile (`%LOCALAPPDATA%\lol.selora.ide`) is shared with the
  normal app. The onboarding Skip writes the persistent "seen" flag. The
  walkthrough does not delete or back up that profile.

## Artifacts

```
<TEMP>\selora-walkthrough-<stamp>\
  summary.md              human-readable run report (start here)
  summary.json            machine-readable summary
  cdp-result.json         per-step results, exceptions, bootstrap snapshot
  cdp-driver.log          driver console (also in cdp-result.json)
  build.log               build steps when -Build was used
  app-boot.log            selora_boot.log lines for this pid
  app-stdout.log          app stdout (usually empty)
  app-stderr.log          app stderr
  app-panic.log           present only when a panic was written during the run
  00-launch.png ... 08-agent.png
  failure-<step>.png      written only for a failing step
  finding-<step>.png      written for a recorded product finding
  failure-final.png       page state when the driver aborts
  app-launch-attempt-N.log  exit/stderr when WebView2 refused a profile
  fixture-main\           generated workspace
  fixture-second\         generated switch target
  settings\settings.json  isolated settings and layout
```

Read `summary.md` first. It lists each step with its duration and detail, the
renderer exception count, and links the screenshots. `cdp-result.json`
contains the final bootstrap snapshot (workspace root, buffers, settings,
layout) for cross-checking.

## Last recorded automated run

2026-10-27, release mode on this machine:

- 12 steps passed, 0 failed, 1 finding, 0 renderer exceptions, exit 0.
- The finding is the Agent activity button no-op described above.
- Artifact directory: `%TEMP%\selora-walkthrough-20261027-094314`
  (`summary.md`, `cdp-result.json`, screenshots `00`-`08`).
- `-FailOnFindings` was verified against that same run shape: exit 1, with
  the finding text as the failure line (`%TEMP%\selora-walkthrough-20261027-094337`).
- Human acceptance: **not claimed**; the checklist below was not executed as a
  formal acceptance pass.

Dev mode: the debug binary built successfully on 2026-10-27
(`target\debug\selora-desktop.exe`), but the dev walkthrough could not run
because port 1420 was already served by an unrelated checkout's Vite process.
The launcher refuses to reuse or kill that server. Dev mode remains
**unverified end-to-end** until it is run with port 1420 free; the run criteria
are identical to release.

## Human checklist

Run these after a green automated run. Tick each item only after looking at
the app; the script cannot tick any of them. Suggested order matches the
automated flow.

### 1. Visual shell

- [ ] Launch the release build from `launch-selora.bat` (or the dev build).
- [ ] Window chrome: the custom title bar shows the Selora mark, File / Edit /
      View / Help, the workspace pill, and working window controls.
- [ ] Seven regions are visible and aligned: title bar, activity bar, primary
      sidebar, editor, agent dock when opened, bottom dock when opened, status
      bar.
- [ ] Hover a resize handle: the pointer changes, dragging feels smooth, and
      the editor never collapses below its floor.
- [ ] No clipped text, overlapping controls, or scrollbars in the wrong place
      at 1440x900 and at the 960x640 minimum size.
- [ ] Colours, hairlines, and typography match `DESIGN_TOKENS.md`; only
      functional states use green.
- [ ] Screenshots from the automated run (`00`-`08`) look like the same app
      state you see by hand.

### 2. Motion and presence

- [ ] Onboarding (fresh profile) plays calmly and has no frozen/blank hero.
- [ ] The activity-bar motion toggle switches Calm Mode and the change is
      obvious and immediate.
- [ ] Agent presence (idle / thinking / running) animates without stutter or
      layout shift.
- [ ] Toasts enter and leave without covering the active control for long.
- [ ] With "Reduce motion" enabled in Settings, looping motion stops but the
      UI stays usable.

### 3. Editing and workspace judgement

- [ ] Open the release app, open a folder with your own project, and confirm
      the explorer, breadcrumbs, minimap, and status bar agree.
- [ ] Type for a few seconds: no dropped characters, no scroll jumps, cursor
      and selection stay where expected.
- [ ] Undo/redo and find-in-file behave normally.
- [ ] Save a file and confirm the on-disk bytes with an external editor.
- [ ] Trigger an external change while the buffer is clean and confirm the
      refresh is not surprising; then make the buffer dirty and confirm it is
      preserved.
- [ ] Reopen the app and confirm layout, theme, and reduced-motion persisted.

### 4. Tabs, panels, and dialogs

- [ ] Open at least three files; switch with the mouse and with Ctrl+Tab or
      the tab arrow keys.
- [ ] Right-click a tab: every context-menu action does something sensible.
- [ ] Close a dirty tab: Save, Discard, and Cancel each do what they say.
- [ ] Close the last tab and confirm the empty state points at the Files view.
- [ ] Open Search, Git, Problems, Output, and Terminal placeholder; each empty
      state explains itself and no panel crashes.
- [ ] Click the **Agent** activity button from editor mode: the dock must open.
      On the 2026-10-27 build it is a no-op (recorded finding); tick only after
      the defect is fixed and the walkthrough step passes without a finding.
- [ ] Ctrl+L opens the agent dock and Ctrl+L again closes it.
- [ ] Command palette (Ctrl+Shift+P) filters, runs, and closes with Escape.
- [ ] Keyboard shortcuts (Ctrl+/) opens the cheat sheet; every chord listed
      works.

### 5. Settings

- [ ] Open Settings and read every tab (AI Gateway, Editor, Theme, Keymap).
- [ ] Controls marked read-only say why, and no control silently no-ops.
- [ ] Change the theme and reduced motion, close and reopen the app, and
      confirm both persisted.
- [ ] With the app closed, inspect `settings.json`; it must contain no API key.

### 6. Real gateway: chat and agent (requires a key)

- [ ] Connect a real key through the Connect dialog; confirm the status bar
      turns connected and a model catalog appears.
- [ ] Send a short chat message; streaming text arrives, Stop stops it, and
      the cost meter updates plausibly.
- [ ] Send an agent prompt that reads a file. Confirm the tool card shows the
      real path, arguments, and result.
- [ ] Approve a write. Confirm the diff matches what lands on disk.
- [ ] Deny a write. Confirm nothing is written and the model is told.
- [ ] Start a command that needs approval, press Stop, and confirm no orphan
      child process survives (`Get-Process` before/after).
- [ ] Disconnect the key; confirm the UI degrades to a clear disconnected
      state and nothing crashes.

### 7. Workflows (requires a key)

- [ ] Run a workflow that asks a question and returns a report; the card,
      approval, phase graph, steps, artifacts, and cost all agree.
- [ ] Deny a workflow step; confirm the run ends in the documented state.
- [ ] Stop a running workflow; confirm the panel shows stopped and the script
      never resumes on its own.
- [ ] Kill the app mid-run, relaunch, and confirm the completed-run history
      and the resume consent gate (including "Resume anyway").
- [ ] Confirm no `SELORA_API_KEY` / `OPENAI_API_KEY` appears in the child
      environment of a workflow command.

### 8. OS integration and packaging

- [ ] Double-click a second shortcut while the app runs: the existing window
      focuses and no second window opens.
- [ ] Open the About dialog links; only the allowlisted URL opens in the
      default browser.
- [ ] Open a folder through the native picker (not the protocol seam) and
      confirm the same tree appears.
- [ ] Minimise, restore, maximise, and move between monitors with different
      DPI scaling; no blurry or misplaced regions.
- [ ] Install from the compiled installer on a clean profile/VM, launch it
      from the Start menu, and uninstall cleanly.
- [ ] Signing/update status is checked against `RELEASE.md`; do not claim a
      signed or auto-updating build without evidence.

### 9. Accessibility

- [ ] Tab through the whole shell: focus is always visible and never trapped
      except inside a modal.
- [ ] Escape closes every modal and returns focus to the invoking control.
- [ ] Screen reader announces the activity bar, tab list, tree, dialogs, and
      live status/toast changes.
- [ ] At 200% text scaling, labels remain readable and controls remain
      reachable.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Every automated step passed and no renderer exception was recorded (findings may still be listed) |
| 1 | At least one step failed, or the driver aborted after attaching, or `-FailOnFindings` saw a finding |
| 2 | Preflight/build/launch/attach failed; no UI step was driven |

Owner/human rows never affect the exit code because the script does not run
them.

## Troubleshooting

**`selora-desktop is already running`** - close it and retry. The
single-instance plugin forwards a second launch to the first process, which
has no debugging port. There is no override; killing someone else's editor is
not a walkthrough action.

**`CDP endpoint did not expose a page`** - WebView2 ignored
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, or the window never created. Check
`app-boot.log` for `setup() started` / `Window built successfully!`, and try
`-Port` with another free port. On locked-down machines, verify the WebView2
runtime version (`Get-ChildItem 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients'`).

**Startup exits with `0x800700AA` / "requested resource is in use"** - WebView2
still holds the previous run's profile for a few seconds. The launcher retries
three times with backoff; each attempt writes `app-launch-attempt-N.log`.
Close other Selora windows and wait before retrying manually.

**`cargo` says "Blocking waiting for file lock"** - another cargo build, test,
or clippy run owns the target directory. The walkthrough waits for the lock; it
never kills another build. If the other run is long, build once yourself and
re-run without `-Build`.

**A toast covers the control being clicked** - the driver retries the hit test
for up to 10 seconds before failing. Toasts last 4-8 seconds. A real modal or a
stuck toast still fails the step with the intercepted element named.

**`release binary missing`** - run with `-Build`, or build manually:

```powershell
npm.cmd --prefix apps\selora-desktop\frontend run build
cargo build --release -p selora-desktop --locked
```

The release binary embeds `frontend/dist`, so rebuild the frontend before the
binary when changing UI code.

**Dev mode fails at Vite** - port 1420 is taken or `npm ci` was never run. See
`vite-dev.log` in the artifact directory. `npm.cmd` (not `npm.ps1`) avoids the
PowerShell execution-policy block on this machine. The launcher checks port
1420 before building when `-Mode dev` is used and refuses to reuse an
unrelated dev server; stop that server first. The devUrl is compiled as
`http://localhost:1420`, so a different port cannot be substituted without
changing `tauri.conf.json` and rebuilding.

**First step times out** - the app may be showing the boot error screen,
onboarding, or a modal. `failure-final.png` and the `diagnostics.bodyText`
field in `cdp-result.json` show what the page actually displayed.

**Screenshots are black or missing** - `Page.captureScreenshot` needs a
visible, composited window. Unlock the session, do not run over a disconnected
RDP session, and keep the window unminimised.

**A selector broke after a UI rename** - update the specific finder in
`scripts/walkthrough/cdp-walkthrough.mjs`; prefer `aria-label`, `role`, or
`data-testid` over class names. Then re-run. The failure screenshot plus the
listed attempted finder is the starting point.

## Known gaps

The walkthrough does not, and should not pretend to, cover:

1. Visual taste, spacing, motion feel, or colour accuracy.
2. Real gateway chat/agent/workflow runs, cost, approvals, or streaming.
3. Native folder-dialog interaction, OS clipboard, drag and drop, or file
   associations.
4. Installer, signing, auto-update, or clean-VM behaviour.
5. Crash/relaunch recovery, OS suspend/resume, multi-monitor/DPI changes.
6. Screen-reader output and full keyboard-only traversal.
7. LSP servers that are absent on the machine (completion/hover/definition
   only run when a server resolves).
8. WebView2 profile isolation: onboarding state is shared with the real app.
   The Skip branch runs only on a fresh profile; the recorded 2026-10-27 run
   used an already-onboarded profile, so that branch is covered by the human
   checklist rather than by the recorded run.
9. Dev mode end-to-end; the debug binary built, but the port-1420 conflict
   described above prevented a driven dev run.

Extend the driver only with steps that produce observable evidence. Never add
a step that reports PASS from a mock, a fixture, or a unit test: those belong
in the normal test suites, not in a functional walkthrough.