# Status - 2026-10-27 (0.1.0 alpha)

## Last completed

Phase 8 plus Phase 9 workbench preview merged and gated on branch
`release/0.1.0-alpha`.

- Phase 8: workflow engine, approvals, durable journal/resume, effect ledger,
  Job Object containment, budget preflight, history, buffer conflicts, LSP
  reload sync, receipt cost/tokens.
- Phase 9 preview: Agent Workbench, plan mode/todos, durable sessions, resume,
  compaction, session-scoped auto-approve, background children and `wait`.
- Settings: editor font/tabs/wrap, keybinds, theme/light theme.
- Sessions/retries/todos are backend-authoritative; event order, active buffer,
  per-session transcripts and parks are reconciled.

Not included in this alpha:

- **Structured question flow from `selora-p3-subagent` is not merged.**
  Phase 9 acceptance reports `questions.structured: PENDING`; the branch commit
  is `bb28da8`, not an ancestor of the release branch.
- Integrated Terminal.
- Git branch/history/push/merge UI.
- Per-phase workflow cost.
- Exactly-once subagent side effects.

## Gate (final release tree)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `cargo test --workspace --locked --no-fail-fast` | **1292 passed, 0 failed, 4 ignored**, 35 suites |
| `npm run typecheck` | clean |
| `npm test` | **556 passed, 0 failed**, 55 files |
| `npm run build` | clean |
| `cargo deny check` | advisories/bans/licenses/sources ok |
| `scripts/phase8-acceptance.ps1` | **46 passed, 0 failed, 3 pending, 1 skipped**, exit 0 |
| `scripts/phase9-acceptance.ps1` | **19 passed, 0 failed, 2 pending**, exit 0; Phase 9 not accepted |

Phase 8 PENDING evidence:

1. exactly-once unfinished asks are not claimed
2. broker processes created outside the Job Object can escape; known launch
   forms are refused and membership is audited, but a confirmed interpreter or
   obfuscated command can still reach a broker
3. no per-phase USD in the protocol

Phase 9 PENDING:

1. structured questions (9c unmerged)
2. remaining owner/real-runtime observations

All nine Phase 8 and Phase 9 owner rows remain **UNRUN**.

## Artifacts (rebuilt from this tree, unsigned)

| Artifact | Size | SHA-256 | Signature |
|---|---:|---|---|
| `target\release\selora-desktop.exe` | 19,591,168 | `2e9962edf7706b85bdd00fb137fece58d2f2341f2571ef0470b30fad7c0fed28` | NotSigned |
| `installer\output\Selora-IDE-Setup-0.1.0.exe` | 9,611,230 | `d145b76b8f5c8e9b1a996794f2f04863c32109cfdf51c7e73c7a6220f88d3239` | NotSigned |

## Branches

- release branch: `release/0.1.0-alpha`
- code commit: `f4fceec` (documentation commit follows)
- seven integration merges are ancestors
- historical backup: `stash@{0}` contains pre-integration working-tree changes
- `selora-post-perf` is **not merged**; it conflicts with the final ack/session
  path. Its measured report is in `docs/PERFORMANCE.md`.
- `selora-p3-subagent` remains unmerged.

## Known limitations

- unsigned installer and EXE; SmartScreen warns
- no clean-VM install; Windows 11/10-without-WebView2 untested
- no integrated terminal
- Git limited to status/stage/unstage/commit/diff
- no code signing, auto-update release, or published repository yet
- background children are in-memory; process crash loses in-flight children
- workflow effect evidence fails closed but cannot prove exactly-once side
  effects across every crash window
- legacy egui shell is not updated with Phase 9 features
- frontend lazy-chunk tests are timing-sensitive under heavy parallel load;
  final full run passed

## Next

1. Publish alpha pre-release from `release/0.1.0-alpha`.
2. Owner acceptance with a real key/workspace.
3. Clean-VM install.
4. Merge 9c structured questions and finish owner acceptance.
5. Terminal + full Git.
6. Signing and published auto-update release.