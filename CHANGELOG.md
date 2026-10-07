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

### Fixed

- Settings dialog painted near-fullscreen with collapsed sections: the modal
  now allocates an exact 720×520 pane (egui's placer pin never bounded the
  horizontal body's children).
- The status bar's connected dot (●) rendered as tofu — Space Grotesk lacks
  it; JetBrains Mono is now the sans symbol fallback.
- The chat header read "no model" and sends were blocked until a manual
  picker pick, even with a persisted `active_model`; the panel now adopts it
  at boot.
