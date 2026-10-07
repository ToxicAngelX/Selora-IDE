# Changelog

All notable changes to Selora IDE are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/) (RELEASE.md §6).

## [Unreleased]

### Added

- First-run splash: the orbiting brand mark, masked key input, Connect/Skip,
  and a fade-out on successful connect (Phase 6a).
- About dialog ("Help: About Selora IDE" in the command palette) with the
  build version, license line, repo link, and a "Check for updates" button
  that calls the GitHub Releases API and toasts the result with a download
  link (Phase 6b).
- Windows installer (Inno Setup 6): per-user install to
  `%LOCALAPPDATA%\Programs\Selora IDE`, Start Menu shortcut, uninstaller,
  and an "Open with Selora IDE" entry for `.txt` (Phase 6c).
- Release pipeline: pushing a `v*.*.*` tag builds the release binary, compiles
  the installer, and attaches it to a draft GitHub Release; code signing
  activates when the owner adds the certificate secrets (Phase 6d).
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

### Fixed

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
