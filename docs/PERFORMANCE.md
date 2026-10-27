# Performance

Selora ships a reproducible harness that measures the six numbers that matter
for an editor shell: cold start, bootstrap readiness, idle memory, typing
latency, event-log throughput/ordering, and workflow-run overhead.

The harness never asserts that a machine is fast. It fails only when a stage
cannot run or a correctness invariant breaks. Numbers are recorded with the
machine and commit they came from, because absolute milliseconds move between
CPUs, Windows builds, and background load.

## Run it

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\perf.ps1
```

Useful switches:

| Switch | Effect |
|---|---|
| `-Runs 5` | Launches per configuration (default 3). |
| `-IdleSeconds 10` | Settle time before the RSS sample (default 5). |
| `-SkipBuild` | Reuse `target\debug`, `target\release`, and `frontend\dist`. |
| `-SkipApp` | Microbenchmarks only (no windows are opened). |
| `-SkipRust` | Skip the Rust benchmarks. |
| `-SkipFrontend` | Skip the CodeMirror typing spec. |
| `-ResultsDir <path>` | Write results somewhere other than `target\perf\<stamp>`. |

Every run writes:

- `results.json` — machine metadata, raw samples, medians, gaps;
- `summary.md` — the tables below for that run;
- the build/vitest/cargo logs that produced the numbers.

The app opens a real window, so run the harness in an interactive desktop
session. Debug builds load the Vite dev server (`WebviewUrl::default()`), so the
script starts it on port 1420 and stops it afterwards. Release builds serve the
bundled `frontend/dist`.

## What each number means

| Metric | Boundary |
|---|---|
| `window_ms` | Process spawn → Win32 `MainWindowHandle` is non-zero. The Tauri window exists, but WebView2 may still be loading. |
| `ready_ms` | Process spawn → `frontend_ready`, written by the frontend after bootstrap and the event replay reach `ready`. |
| Boot stages | Rust `main()` → state creation, Tauri `setup`, WebView build, window shown, frontend ready (from `%TEMP%\selora_boot.log`). |
| `main_rss_mb` | Working set of `selora-desktop.exe` after the idle settle. |
| `tree_rss_mb` | Working set of the app plus every descendant (WebView2 helpers included). |
| Typing (frontend) | One CodeMirror transaction plus the protocol-change computation and local buffer bookkeeping. No paint, no IPC. |
| Typing (Rust) | `ApplyEditorChange` through `DesktopState`: apply, record `BufferChanged`, hand text to the LSP, and (in the comparison arm) build the response body. |
| Event soak | 8 threads × 12,500 events through the in-process `EventLog`, then the sequence/retention invariants and a 1,024-event replay. |
| Workflow | `prepare`, a no-op `run_workflow`, and 50 `world.run` steps against a null host. The engine only: no journal fsync, no IPC, no real agent turn. |

## Current numbers

Measured 2026-10-27 on:

- AMD Ryzen 7 5700X (8C/16T), 15.9 GB RAM
- Windows 11 Pro 10.0.26200
- rustc 1.99.0, Node 24.19.0
- base commit `6634f8a`, branch `selora-post-perf`; the harness and fixes are
  in the same commit as this document
- 5 launches per configuration, 5 s idle settle
- release profile: thin LTO, codegen-units 1

Other test/build jobs were running on the same machine during parts of the
session; the numbers below are from the final run and the repeatable medians are
stable to roughly ±10% for the microbenchmarks.

### Cold start, bootstrap, and idle memory

| Config | Window ms (median) | Bootstrap ready ms (median) | Main RSS MB | Tree RSS MB | Main private MB | Tree private MB |
|---|---|---|---|---|---|---|
| debug | 562.3 | 1407.1 | 43.4 | 473.9 | 7.9 | 307.1 |
| release | 57.5 | 657.0 | 31.4 | 456.0 | 7.0 | 312.1 |

Per-run spread (5 runs):

| Config | `window_ms` | `ready_ms` |
|---|---|---|
| debug | 724.1, 515.1, 507.6, 562.3, 564.1 | 1750.9, 1330.0, 1290.5, 1466.1, 1407.1 |
| release | 67.3, 57.5, 32.5, 85.9, 45.9 | 671.2, 654.7, 657.0, 924.2, 644.4 |

Rust-side boot stages (milliseconds after `main()` starts, from the run-1
sample):

| Stage | Debug | Release |
|---|---|---|
| `DesktopState` created | 12 | 8 |
| Tauri `setup()` started | 26 | 16 |
| WebView window built | 514 | 352 |
| Window shown and focused | 520 | 361 |
| Frontend bootstrap ready | 1073 | 622 |

The debug/release gap is the Vite dev server and an unoptimized binary. The
release shell is interactive at a median 657 ms; most of that is WebView2
startup, not Rust state construction (8 ms) or Tauri setup (16 ms).

Idle memory is dominated by WebView2, not the Rust process. The release tree
holds 6 helper processes and about 456 MB working set; the main process is
31.4 MB. Private bytes show the same split (7.0 MB main, 312.1 MB tree).

### Typing latency

Frontend, 300 single-character transactions per document, no paint and mocked
IPC (`frontend/perf/typing-latency.perf.tsx`):

| Document | p50 | p95 | max | mean |
|---|---|---|---|---|
| 1,000 lines / 35,099 chars | 0.333 ms | 0.529 ms | 2.607 ms | 0.361 ms |
| 10,000 lines / 368,099 chars | 1.342 ms | 2.124 ms | 3.061 ms | 1.452 ms |

Rust `ApplyEditorChange`, 5,000-line buffer plus 200 workspace files, 100 edits
per arm:

| Arm | p50 | p95 | max | IPC response |
|---|---|---|---|---|
| Legacy full bootstrap response | 2.391 ms | 3.140 ms | 4.193 ms | 204,833 bytes |
| Ack path (production) | 0.220 ms | 0.389 ms | 0.439 ms | small ack |

The ack path is the one the shell uses: `dispatch_command` answers
`CommandResultDto::Ack` for event-backed mutations, and that includes
`ApplyEditorChange` and `SetSelection`. The legacy full-bootstrap arm stays in
the harness as the regression control: it shows what returning the bootstrap on
every keystroke costs.

### Event throughput and ordering

100,000 events from 8 concurrent producers:

| Measure | Result |
|---|---|
| Total elapsed | 840 ms |
| Throughput | 118,958 events/s |
| Retained after cap | 1,024 events |
| Sequence order | contiguous `1..100000`, strictly increasing in the ring |
| Replay of 1,024 events | p50 240 µs, p95 459 µs, max 520 µs |

The ordering assertion is the same contract the frontend gate relies on: a
single global sequence, no duplicates, no gaps, and a prefix-only retention
drop. It passes under contention, so no event-path change was needed.

### Workflow run overhead

50-step scripts, null host, release build:

| Measure | p50 | p95 | max |
|---|---|---|---|
| `prepare` (parse + command scan) | 0.319 ms | 1.130 ms | 1.450 ms |
| Empty run (realm + teardown) | 0.667 ms | 1.779 ms | 1.966 ms |
| 50 × `world.run` | 1.400 ms | 2.793 ms | 2.793 ms |
| 50 × `phase` | 1.076 ms | 3.267 ms | 3.267 ms |

Marginal cost of one facade step is about 14 µs. The fixed cost of a run is
sub-millisecond to about 2 ms on a quiet machine. Journal writes, approval cards,
IPC emits, and real agent turns are shell layers and are not in these numbers.

## Regressions found and fixed

The first measured pass exposed two scaling problems. Both fixes are covered by
the harness so they cannot silently return.

### 1. Every keystroke serialized a full bootstrap back over IPC

`dispatch_command` returned `BootstrapState` for every command, including
`ApplyEditorChange` and `SetSelection`. The frontend ignores that value. Building
it walked the file tree, read git status, scanned persisted workflow runs and
cloned every open buffer's text; Tauri then serialized the result to JSON.

Measured on the same 5,000-line buffer plus 200 files:

| | Before | After |
|---|---|---|
| Rust p50 | 7.659 ms | 0.220 ms |
| Rust p95 | 34.393 ms | 0.389 ms |
| Response body | 204,933 bytes | small ack |

Fix: `dispatch_command` answers the typed `CommandResultDto` instead of a
`BootstrapState` — reads and folder opens keep the snapshot shape, event-backed
mutations (the two fire-and-forget editor commands included) answer the small
ack. The state still validates the same envelope, applies the edit, records the
same events, and feeds the language server. The full-response arm remains in
the perf test as the control.

### 2. The editor stringified the whole document twice per keystroke

`computeTextChanges` called `docBefore.toString()` and then scanned from offset
0 for every change; the local buffer cache also stored a fresh
`doc.toString()` on every update. Both are O(file size) per keystroke even though
the cached text is only read when the user switches tabs.

Fix: derive line/column with `Text.lineAt` (O(log lines)) and cache the immutable
`Text` object, stringifying it only on a tab switch. A remote-snapshot replace
still compares strings, but that path is not per-keystroke.

Frontend typing cost on the 10,000-line document:

| | Before | After |
|---|---|---|
| p50 | 6.614 ms | 1.342 ms |
| p95 | 10.374 ms | 2.124 ms |

## Post-merge verification (`final-perf-merge`)

Re-measured on the merged final line: AMD Ryzen 7 5700X, 16 GB, Windows 11,
release builds, `scripts/perf.ps1 -Runs 3`. The frontend "before" column comes
from the same spec run against `git archive 88163e2` on this machine (three
runs, median of p50). The backend "before" column is the perf test's legacy
full-bootstrap arm, which is the pre-merge reply shape running on the merged
tree.

| Measure | Before (88163e2) | After (merge) |
|---|---|---|
| `ApplyEditorChange` p50 | 2.410 ms (full bootstrap reply) | 0.242 ms (ack) |
| `ApplyEditorChange` p95 | 3.460 ms | 0.389 ms |
| IPC response body | 205,062 B | 124 B |
| Frontend keystroke p50, 1,000 lines | 0.56 ms | 0.33 ms |
| Frontend keystroke p50, 10,000 lines | 5.12 ms | 1.30 ms |
| Event log, 8 producers | — | 96,222 events/s, 1,024 retained |
| Release cold start: window / ready | — | 148 ms / 1,466 ms |
| Release idle tree RSS | — | 464.9 MB |

All harness stages passed (`build`, `cold-start-debug`, `idle-rss-debug`,
`cold-start-release`, `idle-rss-release`, `benchmarks`). The debug cold start
(30.3 s) loads the Vite dev server and is not a meaningful number.

## Known gaps

- Cold start uses a warm OS file cache. Windows has no supported drop-caches
  command, so this is not a true first-boot measurement.
- `ready_ms` marks the end of bootstrap + replay, not the first painted frame.
  The paint can land a frame later. The frontend marker is the closest
  non-invasive boundary available.
- Frontend typing latency runs in happy-dom: it excludes paint, layout, and the
  real Tauri IPC round trip. The Rust arm covers the backend half. An
  end-to-end key-to-paint number would need WebView2 tracing and is not claimed
  here.
- The event soak measures the in-process log and the replay copy, not the Tauri
  emit serialization to the webview.
- Workflow numbers cover the engine only; the shell's journal fsync, IPC emits,
  and agent turns are out of scope.
- Absolute timings depend on the machine and background load. Compare runs from
  the same machine and commit, not from different workstations.