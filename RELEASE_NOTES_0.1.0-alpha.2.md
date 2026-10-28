# Selora IDE 0.1.0-alpha.2

Windows coding IDE with a real editor, LSP, Git, integrated terminal, AI chat,
approval-gated agents, workflows, and a durable Agent Workbench.

**Alpha preview.** Unsigned, not clean-VM tested, not a production replacement
for VS Code or JetBrains IDEs.

## New since alpha.1

- Integrated ConPTY terminal with xterm.js, tabs, resize, themes, env
  allowlist, session caps, and process-tree kill.
- Full Git panel and backend: branches, history, commit details, discard,
  stage/unstage all, stash apply/pop/drop, merge conflict resolution,
  fetch/pull/push.
- Structured agent questions and real subagent tools:
  `spawn_subagent`, `send_message`, `wait`, `ask_user_question`.
- Delete agent sessions, including disk-only durable sessions.
- Per-phase workflow tokens/USD.
- Durable tool-effect acknowledgement and replay hardening.
- Broker-launch refusal and Job Object membership audit for `world.run`.
- Performance fixes: no full bootstrap per keystroke; no double document
  stringify.
- Agent Workbench: plan mode, live todos, durable sessions, resume, compaction,
  auto-approve reset, background children.

## Verification

- Rust: 1373 passed, 0 failed, 7 ignored, 36 suites
- Frontend: 616 passed, 57 files
- Phase 8: 46 passed, 0 failed, 3 pending, 1 skipped
- Phase 9: 20 passed, 0 failed, 1 pending; 9 roadmap steps PASS
- Owner acceptance: UNRUN

## Known limits

- Unsigned; SmartScreen warns.
- No clean-VM install.
- Exactly-once subagent side effects are not guaranteed across every crash
  window.
- A broker-launched process hidden in an obfuscated interpreter argument can
  escape the Job Object.
- Git has no rebase/cherry-pick/tags/remote management; stash excludes
  untracked files; push does not set upstream.
- Background children die if the process crashes.
- No macOS/Linux, debugger, marketplace, or collaboration.

## Installer

`Selora-IDE-Setup-0.1.0.exe`

SHA-256:
`2072F473229A2F6D78445A8180093073FCAB8470111918B5D1C0E611AB96056C`

Back up or commit your repository before using agent writes/workflows.