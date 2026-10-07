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
