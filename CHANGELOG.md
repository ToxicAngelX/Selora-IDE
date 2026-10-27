# Changelog

All notable changes to Selora IDE are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/) (RELEASE.md §6).

## [Unreleased]

<!-- FINALIZATION: keep these entries under Unreleased until the post-merge gate
passes and a version/tag is chosen. Do not add test counts, artifact hashes, or
dates from memory; record gate evidence in STATUS.md and RELEASE.md. Remove any
Phase 9a bullet if the `selora-p3-window` branch is discarded instead of merged. -->

### Added

- Workflows (Phase 8): a scripted multi-agent engine with a hardened facade,
  schema-validated `ask`, deterministic `world.run` gates, named-agent FIFO
  scheduling, a per-run budget, and a durable JSONL journal under
  `.selora/workflow-runs/`.
- Workflows (Phase 8): the `run_workflow` tool, manifest v2, a confirmation
  card showing the script body, command set, and cost cap, an approval route
  that reuses Agent Mode's cards, run/stop/resume dispatch, and receipt/token
  accounting.
- Workflows (Phase 8): a live panel with the phase graph, step status, run log,
  inline approvals, Stop, and completed-run history. Completed history,
  explicit journal phases/reports/artifacts, and budget preflight on direct
  resume are on the `integration` branch pending merge.
- Workflows (Phase 8): `world.run` creates the child suspended and assigns it
  to a Windows Job Object before execution, so the process tree is contained
  on Stop, timeout, and cancel. `Resume anyway` is explicit and default-refused
  for an unfinished ask.
- Agent Workbench (Phase 9a, on `selora-p3-window` pending merge): a dedicated
  Agent surface with truthful session tabs, a read-only todo spine, structured
  question and plan cards, Chat/Plan views, and a composer mode row mapped to
  session permission policies. Backend session routing lands with the session
  slice, not in 9a alone.

### Changed

- Agent sessions (branch `selora-wt-session`, pending merge): new/switch/retry
  become backend-authoritative, todos are stored per session and published as
  `session_todos_changed`, the active buffer is recorded, and event sequence
  assignment is serialized under one order guard.
- Editor settings (branch `selora-wt-settings`, pending merge): editor
  settings become real controls and dirty-tab close is protected.
- LSP (branch `selora-wt-lsp`, pending merge): lifecycle isolation, `.cmd`
  server resolution, and `rg`/fallback search parity.

### Fixed

- LSP documents are re-sent after a clean-buffer workflow reload; dirty buffers
  are preserved with a toast instead of being overwritten.
- A budget-exhausted workflow run is pre-refused on direct resume, not only by
  the run listing.

### Pending verification

- Post-merge gate: **NOT RUN** - `TODO-VERIFY` in `STATUS.md`.
- Phase 8 owner acceptance: **UNRUN** - `docs/PHASE8_ACCEPTANCE.md`.
- Phase 9 acceptance: **UNRUN / partial** - `docs/PHASE9_ACCEPTANCE.md`.
- Artifacts: unsigned, not built from the merged tree, no clean-VM install -
  `RELEASE.md` section 11.

## [0.1.0] - 2026-10-26

### Added

- First-run splash: the orbiting brand mark, masked key input, Connect/Skip,
  and a fade-out on successful connect (Phase 6a).
- About dialog ("Help: About Selora IDE" in the command palette) with the
  build version, license line, repo link, and a "Check for updates" button
  that calls the GitHub Releases API and toasts the result with a download
  link (Phase 6b).
- Windows installer (Inno Setup 6): per-user install to
  `%LOCALAPPDATA%\Programs\Selora IDE`, Start Menu shortcut, uninstaller,
  an "Open with Selora IDE" entry for `.txt`, the MSVC runtime DLLs beside
  the exe, and a WebView2 detection guard that runs Microsoft's Evergreen
  bootstrapper only when the runtime is missing (Phase 6c).
- Release pipeline: pushing a `v*.*.*` tag typechecks and tests the
  frontend, builds the Tauri binary, stages the MSVC runtime DLLs, compiles
  the installer, and attaches the installer plus a SHA256 checksum to a
  draft GitHub Release; code signing activates when the owner adds the
  certificate secrets (Phase 6d).
- Docs site sources (`docs/`, MkDocs) and a README with the feature tour
  (Phase 6e); publication-checklist files (issue/PR templates, Code of
  Conduct) (Phase 6f).
- AI presence & motion (Phase 6h): the ThinkingOrb thought indicator (nine
  agent-loop states) replaces the streaming wave and rides the agent status
  line; the AgentAvatar identity glyph (idle/working/sleeping); the
  StreamGlow composer edge-glow driven by streaming activity; the LiquidPill
  melt on the agent-mode toggle; and the flag-gated MetalMark splash sheen
  (off by default — owner decision).
- The `--scrim` overlay token (true black @ 60%): the one sanctioned modal
  backdrop value (Phase R0; DESIGN_TOKENS.md §1.7).
- The ZCode-grade composer (Phase R1): a multiline auto-growing input (6-row
  cap) where Enter submits and Shift+Enter inserts the newline at the caret;
  ↑/↓ recall the last 10 submitted prompts on a one-line buffer; and while a
  plain stream runs the send button becomes a Stop square whose click
  cancels the request for real — the new `Effect::SeloraChatAbort` drops the
  in-flight SSE request (the connection closes) and the relay ends silently,
  never reporting a user stop as an error. The partial reply stays in the
  transcript and re-enters the next request's context.

- Markdown-lite rendering for assistant messages (Phase R2): `###`/`####`
  headings, bullet and ordered lists, block quotes with a left rule, `---`
  rules, `**strong**` (SemiBold — 700-weight bold stays banned), `*emphasis*`
  (Medium — italics stay banned), `` `inline code` `` chips, and
  `[text](url)` links that open in the browser on click. Fenced code blocks
  are now tree-sitter-highlighted in the §1.5 mono palette with a header row
  (language label + Copy); the whole message carries a Copy action.
- Message footers (Phase R2): elapsed time + a token estimate while and
  after streaming; a failed stream offers Retry, which drops the failed
  bubble and re-sends its user message.
- The empty transcript renders a welcome state (Phase R2): the brand mark,
  a prompt, and three suggestion chips that prefill the composer.
- Multi-session chat with restore (Phase R3): the header's label is now
  the session's auto-titled name with a dropdown (the live session marked,
  stored ones newest-first, "+ new session" on top). Sessions persist to
  `sessions.json` beside `workspace.json` — the full timeline including
  agent tool cards (serde-mirrored) survives restart; the legacy
  `chat_turns` remain the fallback for pre-R3 files. Switching or starting
  new mid-turn refuses with a toast; 20 sessions / 100 entries each.
- The chat's rendered layout is cached per message (Phase R2): re-laid out
  only when the content or the panel width changes — a settled message
  costs no per-frame re-layout.

- The shared DiffView widget (Phase R4): the diff renderer extracted from
  the chat's approval cards into `diff_view` (Phase R6's git tabs reuse it),
  now with tree-sitter highlighting of the new-side text (§1.5 palette, the
  parse cached by content hash so steady frames cost nothing) and word-level
  intra-line emphasis — a paired removed/added run's changed words carry a
  12% alpha tint of the same `--err`/`--ok` (a §12 bend: alpha of existing
  semantic colors, no new values — flagged for sign-off).
- Card affordances (Phase R4): every file-touching card body (diff, read,
  write/edit results) carries an "open path · line N" link — one click
  opens the buffer at the diff's first changed line; bash output strips
  ANSI escape sequences; web_search rows open in the browser on click (the
  hover `copy` glyph keeps the clipboard path).
- The status bar's Selora zone gains the ThinkingOrb's reserved arms
  (Phase R4): `Shaping` while the file tree rebuilds, `Composing` while a
  workspace search runs, `Weaving` while a ghost completion is in flight —
  the Phase 6h states' documented surfaces, wired.

- The Settings dialog's Agent section (Phase R5): the turn caps (max
  iterations / seconds / USD), the bash shell, the auto-approve allowlist
  (editable rows + add field), and the Brave Search API key — which now
  lives in the OS keyring (`brave-key` entry), never in settings.json; the
  agent turn injects it from the keyring at send time (a legacy plaintext
  field still wins if the keyring is empty, so nothing breaks).
- General is a real section (Phase R5): the reduce-motion toggle
  (live-wired through the existing settings adoption). The Git placeholder
  section and the no-op Minimap checkbox were removed — a control that
  does nothing is a lie (the schema fields stay for the future features).

- Title-bar menus (Phase R6, UI_SPEC §1.1 wired at last): File / Edit /
  View / Help dropdowns routing to the palette commands, plus File → Exit
  (the viewport close; the graceful-close hook persists).
- The problems panel (Phase R6): region ④'s third view — the latest LSP
  diagnostics grouped by file with severity glyphs (`✕` err / `!` warn /
  `i` info / `·` hint), a row click opening the buffer at the diagnostic's
  line, and a problems count badge on the activity bar's warning icon.
- The git diff view (Phase R6b): clicking a changed file in the git panel
  opens a read-only overlay over the editor area — "path (working tree)"
  header, the working-tree-vs-index unified diff through the shared
  DiffView (syntax highlighted), Esc/× to close. `FileDiff::to_unified` is
  new in selora-git.
- Final polish (Phase R7): the MetalMark liquid-metal sheen on the splash
  (owner-enabled — the one permitted specular highlight); the `--scrim`
  token applied to both modal backdrops; toasts fade in over 240ms and out
  over their last 400ms; and the keyboard cheat sheet ("Help: Show Keyboard
  Shortcuts" — a modal of the live keybind table, Esc/backdrop to close).
- Find/replace in the editor (Phase R6c): Ctrl+F opens the find bar over
  the active pane (query, match count, `Aa` case toggle, ↑/↓, ×); Ctrl+H
  adds the replace row (Replace / Replace All). Every match gets a hairline
  box, the active one an ink-mid fill; Enter/Shift+Enter step, Esc closes.
  The palette's Find/Replace commands and the Edit menu route to it.
- Workspace opening (Phase Z4): the native folder picker (or `OpenFolderAt`)
  attaches a real workspace, publishes the file tree, refreshes it from
  disk, and runs workspace-wide search through `selora-fs`.
- The CodeMirror 6 editor (Phase Z4): multi-buffer tabs with dirty state,
  breadcrumbs and a minimap, versioned edit application, save, and the
  in-buffer find/replace bar.
- The Git panel (Phase Z4): stage, unstage, and commit through `selora-git`,
  plus a read-only working-tree diff overlay for a changed file.
- LSP hover, go-to-definition, and completion reach the editor through the
  `selora-lsp` bridge, and published diagnostics feed the workspace's
  diagnostics event stream (Phase Z4b).

- First-run onboarding (Phase Z5b): a three-step overlay with a shader hero
  (img-fx) that falls back to a token-only brand mark when WebGL is
  unavailable, reduced motion is requested, or the app runs under a test DOM;
  completion is persisted and can be skipped on any step.
- A notification center (Phase Z5a): a title-bar bell with an unread badge, a
  50-entry persisted history, mark-all-read, clear-history, and per-item
  dismissal, while transient toasts still auto-expire on their own timers.
- The keyboard cheat sheet (Phase Z5c): \Ctrl+/\ and the palette's
  "Keyboard Shortcuts" open a searchable, grouped chord reference with the
  same focus-trap behaviour as the other modals.
- "About Selora" (Phase Z5b) now opens a Tauri dialog with the Vite-injected
  version, license and repository links, a \metal-fx\ brand surface with a
  fallback, and a real GitHub Releases update check that reports failures
  instead of pretending an update exists.

- Regex workspace search: the Search panel's Regex toggle sets `use_regex`,
  Rust compiles the pattern once (honouring case sensitivity and whole-word),
  and a bad pattern reports `invalid_query` inline while the previous results
  stay on screen.
- Settings persistence: reduced motion and theme are written through
  `selora-settings` and republished in the bootstrap, so they survive a
  restart; controls with no backend field render disabled with an explanation
  instead of pretending to save.

### Changed

- Architecture seams ahead of the UI revamp (Phase R0): the title strip, the
  status bar's workbench zones (git/LSP left, cursor center), the toast
  stack, and both workbench empty states moved from the app shell into
  `selora-ui` widgets (`title_bar`, `status_bar::workbench_zone`, `toasts`,
  `empty_state`); `Toast`/`Severity` moved with them.
- The chat header now hosts the canonical model picker (Phase 4b's
  `picker::ModelPicker`, previously dead code): display names, a filter
  input, and Enter-to-select while the filter owns the keyboard — the
  header's parallel inline dropdown was deleted (~400 lines).
- The frontend is code-split (Phase Z6): the CodeMirror editor, the agent
  dock, the connect modal, and the WebGL brand surfaces each load as their own
  chunk on first use, taking the eager entry chunk from roughly 1,230 kB to
  roughly 490 kB.
- External links (About's repository, Connect's provider docs) now open in the
  OS browser through a Rust `open_external` command with an eight-host
  allowlist; the webview no longer renders dead `target="_blank"` anchors.
- Release packaging pins Inno Setup 6.7.1 in CI so a future 7.x Chocolatey
  package cannot silently change the compiler under a release; the local
  script accepts any Inno Setup 6.3+ (or 7.x) install.

### Fixed

- Closing a buffer left its last published diagnostics in Problems because the
  LSP manager emits no close event; the bridge now retracts them and pushes the
  pruned list.
- An accessibility pass (Phase Z6) added dialog focus traps and focus
  restoration, roving tab indices for tab strips, labelled icon-only controls,
  live-region announcements, and reduced-motion coverage for every looping
  effect; the findings are recorded in \docs/ui/Z6_A11Y_AUDIT.md\.
- The installer's WebView2 guard probed a non-existent registry GUID, so an
  installed runtime was never detected and Setup always offered the
  bootstrapper; it now reads Microsoft's EdgeUpdate client key.
- \installer/prebuild.bat\ failed at the Authenticode check whenever it
  inherited a PowerShell 7 \PSModulePath\, and a truncated bootstrapper
  download could be embedded; both are fixed (retry, timeout, cleanup).
- The installer packaged `target\release\selora-app.exe` (the egui binary)
  instead of the shipped Tauri shell, `selora-desktop.exe`; the Start Menu
  shortcut, file association, and launch entry now point at the app that is
  actually built and tested.
- CI and the release workflow built `selora-desktop` without generating
  `apps/selora-desktop/frontend/dist` (which is gitignored but embedded at
  compile time); both now install, typecheck, test, and build the frontend
  first.
- The release binary links the dynamic CRT, but no installer shipped
  `vcruntime140.dll` / `vcruntime140_1.dll`; prebuild and the release
  workflow now stage them from the MSVC redist, the installer ships them,
  and Setup verifies they landed beside the exe.
- WebView2 was an undocumented implicit dependency. The installer now reads
  Microsoft's EdgeUpdate client registration and runs the bundled Evergreen
  bootstrapper only when no runtime is present (per-user, matching the
  no-admin install).
- The release workflow's optional signing steps compared a secret in an
  `if:` expression where the `secrets` context is unavailable; signing now
  keys off a job-level environment flag, uses the correct `selora-desktop.exe`
  path, and signs the installer before hashing it.
- Release packaging now fails fast when the tag, `Cargo.toml`
  `[workspace.package]` version, or `tauri.conf.json` version disagree,
  instead of publishing an installer with a mismatched version.
- Settings dialog painted near-fullscreen with collapsed sections: the modal
  now allocates an exact 720×520 pane (egui's placer pin never bounded the
  horizontal body's children).
- The status bar's connected dot (●) rendered as tofu — Space Grotesk lacks
  it; JetBrains Mono is now the sans symbol fallback.
- The chat header read "no model" and sends were blocked until a manual
  picker pick, even with a persisted `active_model`; the panel now adopts it
  at boot.
- The "no folder open" empty state's button pair painted at the panel's
  left edge instead of centered (UI_SPEC §12) — a `ui.horizontal` inside
  `ui.vertical_centered` resets to left-aligned layout; the row is now
  measured and centered.
- WebView2 detection queried a non-existent EdgeUpdate client GUID, so
  Setup never saw an already-installed runtime and always offered the
  bootstrapper. The check now reads Microsoft's documented
  `{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` registration in `HKCU`,
  `HKLM32` (WOW6432Node), and `HKLM64`, and the `[Code]` helper is
  prefixed so it cannot collide with a future Inno Setup support function.
  The bootstrapper download retries three times and deletes a partial file
  on failure, and the signature check pins Windows PowerShell's module path
  so a PowerShell 7 environment cannot break it.
- CI's `cargo-deny` step used the upstream container action, which GitHub
  Actions cannot run on `windows-latest`; CI now installs `cargo-deny`
  natively and runs `cargo deny check` against the existing `deny.toml`.
