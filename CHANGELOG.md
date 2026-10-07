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
