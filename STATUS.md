# Status - 2026-10-28 (0.1.0 alpha.2)

## Release

Branch: `release/0.1.0-alpha.2`
Tag: `v0.1.0-alpha.2`
Code base: `825d312` + `ac5a1e9` + docs commit.

This alpha adds the missing daily-driver building blocks on top of
`v0.1.0-alpha.1`:

- integrated ConPTY terminal with xterm.js
- full Git backend and panel: branches, history, stash, discard, merge
  conflicts, fetch/pull/push
- structured agent questions and `spawn_subagent`/`send_message`/`wait`
- per-phase workflow cost
- durable effect acknowledgement/replay hardening
- broker-launch refusal and Job Object membership audit
- DeleteSession
- performance merge: no full bootstrap per keystroke
- Agent Workbench, plan mode, todos, sessions, resume, compaction, background

## Gate

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean, fresh target |
| `cargo test --workspace --locked --no-fail-fast` | **1373 passed, 0 failed, 7 ignored**, 36 suites |
| `npm run typecheck` | clean |
| `npm test` | **616 passed, 0 failed**, 57 files |
| `npm run build` | clean |
| `cargo deny check` | advisories/bans/licenses/sources ok |
| `scripts/phase8-acceptance.ps1` | **46 passed, 0 failed, 3 pending, 1 skipped** |
| `scripts/phase9-acceptance.ps1` | **20 passed, 0 failed, 1 pending**, 9 roadmap PASS |

Phase 8 PENDING:

- exactly-once unfinished asks
- Job assignment startup race
- per-phase cost evidence still reported PENDING by the harness even though the
  phase-cost feature is merged; re-run the harness after a fresh run with
  `engine_phase_usage` records.

Phase 9 PENDING:

- `sessions.workspace`: no `.selora/agent-sessions` directory existed at harness
  time. Runtime session persistence is merged; the probe needs a real session.

All owner rows remain **UNRUN**.

## Artifacts (unsigned)

| Artifact | Size | SHA-256 |
|---|---:|---|
| `target\release\selora-desktop.exe` | 20,421,120 | `dae00fa76184311891d3e3fb9e30bf51dc89b645b3b4710f692f9bf07d632ed4` |
| `installer\output\Selora-IDE-Setup-0.1.0.exe` | 9,894,346 | `2072f473229a2f6d78445a8180093073fcab8470111918b5d1c0e611ab96056c` |

Authenticode: `NotSigned`.

## Alpha limitations

- no code signing; SmartScreen warns
- no clean-VM test
- no owner real-key acceptance
- exactly-once side effects still have crash windows
- broker/obfuscated interpreter escape remains possible
- merge conflicts: ours/theirs only; no interactive 3-way editor
- Git has no rebase/cherry-pick/tags/remote management
- stash excludes untracked files
- push does not set upstream automatically
- network calls have no explicit timeout
- background children are in-memory and die on crash
- egui shell not updated
- no macOS/Linux, debugger, marketplace, collaboration

## Main checkout

`main` is being edited by another active session and was not used for this
release. Its changes are preserved in stash entries. The release branch is the
source of truth for this alpha.

## Next

1. Publish this branch/tag as a pre-release.
2. Run owner acceptance with a real key.
3. Clean Windows 11 / Windows 10-without-WebView2 install.
4. Sign artifacts.
5. Merge the other session's `main` work only after it commits and gates.