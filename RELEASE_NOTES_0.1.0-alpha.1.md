# Selora IDE 0.1.0-alpha.1

Windows coding IDE with real files, editor, LSP, Git basics, AI chat, an
approval-gated agent, and a workflows engine.

**This is an alpha preview.** It is not signed, not clean-VM tested, and not a
production replacement for VS Code or JetBrains IDEs.

## Included

- Windows Tauri 2 shell, seven-region layout, layout persistence
- File tree, tabs, save, search, command palette
- Rust/TypeScript LSP through servers on `PATH`
- Git status, stage/unstage, commit, staged/unstaged diffs
- AI chat and Agent Mode with streaming, tools, approval cards, Stop
- Workflows: script confirmation, subagent turns, approvals, journal, resume,
  budget, buffer refresh, completed-run history
- Agent Workbench preview: plan mode, live todos, durable sessions, resume,
  compaction, session-scoped auto-approve, background children
- Dark and light themes
- Editor font/tab/wrap and keybind settings

## Not included

- Integrated terminal
- Git branch/history/push/merge UI
- Debugger
- Structured question flow (9c) is not merged in this build
- Extensions, collaboration, macOS/Linux
- Code signing
- Auto-update release channel

## Install

1. Download `Selora-IDE-Setup-0.1.0.exe`.
2. Verify SHA-256:
   `D145B76B8F5C8E9B1A996794F2F04863C32109CFDF51C7E73C7A6220F88D3239`
3. Run the installer.
4. Start Selora IDE and open a folder.
5. Connect a Selora gateway API key in Settings.

The installer is unsigned. Windows SmartScreen will show an unknown publisher.

## Before using agents on a repository

- Commit or back up your work.
- Prefer Ask/Plan mode over Full auto-apply.
- Read every approval card.
- Work on a branch.
- The effect ledger and journal are in `.selora/` and are gitignored.

## Verification

- Rust: 1292 passed, 0 failed, 4 ignored, 35 suites
- Frontend: 556 passed, 55 files
- Phase 8 automation: 46 passed, 0 failed, 3 pending, 1 skipped
- Phase 9 automation: 19 passed, 0 failed, 2 pending
- Owner acceptance steps: UNRUN

## Known limits

- Workflow resume cannot guarantee exactly-once subagent side effects.
- Broker/shell-service child processes can escape the Job Object. Known broker
  launch forms in the confirmed `world.run` command are refused before spawn,
  and the job's membership is audited after every exit; a broker process
  created outside the job (for example WMI code carried in an argument, or an
  obfuscated command) still escapes.
- Background children die on process crash.
- Per-phase workflow cost is not implemented.
- No clean-VM install has been performed.
- No signing certificate.

Report issues with the version, Windows build, steps, and logs. Do not include
API keys.