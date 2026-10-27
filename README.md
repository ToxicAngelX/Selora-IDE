# Selora IDE Lite

Selora IDE Lite is a Windows desktop code editor. This build is a Tauri 2
application: a Rust core in `apps/selora-desktop/src-tauri`, a React and
TypeScript shell in `apps/selora-desktop/frontend`, and CodeMirror 6 for text
editing. Workspace I/O, Git, language-server traffic, chat, and agent tool
calls are handled in Rust. The earlier egui shell remains in the workspace
under `crates/selora-app`, but it is not the build this README describes.

Current version: 0.1.0. License: MIT. Platform: Windows only.

## Run From Source

Requirements:

- Windows 10 or 11 with the WebView2 runtime
- Rust stable with the MSVC x64 toolchain
- Node.js 22 (the version used by the release workflow)

Install frontend dependencies from the repository root:

```powershell
npm --prefix apps/selora-desktop/frontend install
```

Development runs two processes. Start the Vite dev server:

```powershell
npm --prefix apps/selora-desktop/frontend run dev
```

Then start the desktop shell in a second terminal:

```powershell
cargo run -p selora-desktop
```

The debug shell loads the frontend from `http://localhost:1420`, the
`devUrl` in `apps/selora-desktop/src-tauri/tauri.conf.json`. At boot the Rust
core attaches the folder named by `SELORA_WORKSPACE_ROOT` when it is a
directory; otherwise it attaches the process working directory. Open Folder
in the editor or Files sidebar switches workspaces.

## Build The Installer

`installer\prebuild.bat` packages an existing release binary. Build the
frontend and the release binary first:

```powershell
npm --prefix apps/selora-desktop/frontend run build
cargo build --release -p selora-desktop
installer\prebuild.bat
```

The script reads the version from the root `Cargo.toml`, stages the MSVC
runtime DLLs beside `target\release\selora-desktop.exe`, downloads Microsoft's
WebView2 Evergreen bootstrapper when it is not cached, and invokes Inno Setup.
It requires Inno Setup 6.3 or newer (or 7.x) and the Visual Studio C++ x64
toolset. `installer/find-iscc.ps1` locates the compiler through
`%SELORA_ISCC%`, `PATH`, the standard install directories, or the uninstall
registry, and the script passes an absolute `/O`, so the setup always lands in
the repository even when Inno Setup is installed elsewhere (for example a
portable copy under `%TEMP%`). The output is:

```text
installer\output\Selora-IDE-Setup-<version>.exe
```

The installer is per-user (`PrivilegesRequired=lowest`), installs under
`%LOCALAPPDATA%\Programs\Selora IDE`, adds a Start Menu shortcut, registers
the app for `.txt` "Open with", and installs WebView2 first when the runtime
is missing.

Tagged releases use `.github/workflows/release.yml`. A `vX.Y.Z` tag must point
at a commit already on `main` and must equal the version in `Cargo.toml`,
`tauri.conf.json`, and `frontend/package.json`; `CHANGELOG.md` must contain a
`## [<version>]` section. The workflow re-runs fmt, clippy, cargo-deny, the
frontend and Rust tests, builds the release binary and Inno installer, writes
a SHA256 checksum, and creates a draft GitHub Release. It refuses to touch an
already-published release, verifies the installer signature (signed only with
the secrets below, otherwise `NotSigned`), and round-trip hashes the draft
assets. Signing runs only when
the `SIGNING_CERT_BASE64` and `SIGNING_CERT_PASSWORD` repository secrets are
set; otherwise the artifacts are unsigned.

## Feature Tour

The table describes the `apps/selora-desktop` build as of 2026-10-26.
**Terminal and Output are intentionally inert placeholders**: they do not
start a shell, run tasks, or collect output.

| Area | What works | Where it lives |
|---|---|---|
| Workbench shell | Seven regions: title bar, activity bar, primary sidebar, editor, bottom dock, agent dock, and status bar. The sidebar, agent dock, and bottom dock toggle from commands and resize by drag or keyboard; sizes and open state persist in `settings.json` and restore at launch. The title bar has Agent/Split/Editor view switching. The View menu is populated; File, Edit, and Help are disabled placeholders. | `apps/selora-desktop/frontend/src/layout/ShellGrid.tsx`, `layout/TitleBar.tsx`, `layout/ActivityBar.tsx`, `features/layout/layout-store.ts`, `apps/selora-desktop/src-tauri/src/state.rs` |
| Status bar | Shows workspace, Git branch, active language, agent status, diagnostic counts, UTF-8, the core instance prefix, and connection state. Workspace, language, agent, Problems, and connection items are clickable. | `apps/selora-desktop/frontend/src/layout/StatusBar.tsx` |
| Editor | CodeMirror 6 with tabs, dirty markers, breadcrumbs, minimap (pointer and keyboard), bracket matching, folding, multiple selections, undo/redo, in-buffer search, and `Ctrl+S` save. Opens UTF-8 text up to 2 MiB; binary and non-UTF-8 files are rejected. Highlighting modes: Rust, JavaScript, JSX, TypeScript, TSX, CSS, JSON, Markdown. No split editor panes. | `apps/selora-desktop/frontend/src/features/editor/CodeMirrorPane.tsx`, `features/editor/language-loaders.ts`, `components/editor/EditorTabBar.tsx`, `apps/selora-desktop/src-tauri/src/workspace.rs` |
| Language intelligence | rust-analyzer and typescript-language-server through the in-house LSP client: hover after 600 ms, go to definition with `F12` or Ctrl-click, a completion popup while typing or on `Ctrl+Space`, and diagnostics in the editor and Problems. Rust, TypeScript/TSX, and JavaScript/JSX only. | `apps/selora-desktop/src-tauri/src/lsp_bridge.rs`, `crates/selora-lsp/`, `apps/selora-desktop/frontend/src/features/editor/lsp.ts` |
| Files | Native Open Folder picker, tree refresh, expand/collapse, dirty-buffer dots, and open-into-tab. Hidden entries and `.gitignore`/`.ignore` matches are skipped; each directory is capped at 2,000 entries. | `apps/selora-desktop/frontend/src/features/sidebar/FileTreeView.tsx`, `crates/selora-fs/src/tree.rs`, `apps/selora-desktop/src-tauri/src/workspace.rs` |
| Search | Literal and regex workspace search with case-sensitive, whole-word, and regex toggles (a bad pattern reports `invalid_query` inline), a 250 ms debounce, a 500-hit backend cap, the first 200 hits rendered, results grouped by file with highlighted previews, and click-to-open at the matching line. | `apps/selora-desktop/frontend/src/features/sidebar/SearchPanelView.tsx`, `apps/selora-desktop/src-tauri/src/workspace.rs` |
| Git | Branch, staged and changed lists, stage/unstage, inline unified diff, and commit with a message (`Ctrl+Enter` or the Commit button). Requires a workspace inside a Git repository. | `apps/selora-desktop/frontend/src/features/sidebar/GitPanelView.tsx`, `apps/selora-desktop/src-tauri/src/workspace.rs`, `crates/selora-git/` |
| Problems | Live diagnostics grouped by file and severity, severity tabs, a text filter, and click-to-jump. Counts also appear on the Problems activity icon and the status bar. | `apps/selora-desktop/frontend/src/features/dock/ProblemsPanel.tsx`, `layout/BottomDock.tsx`, `layout/StatusBar.tsx` |
| Agent dock | Streaming transcript with tool cards, approval cards, a per-turn tool/token/cost summary, a session rail, a plan/progress card, and a composer. Enter sends, Shift+Enter adds a line, Up/Down recalls prompt history, and Stop cancels. The model picker uses the gateway catalog; reasoning effort, temperature, and custom system instructions are sent with the turn. Plan only arms the read-only tool set; Ask before changes arms the full tool set and parks every write or command on an approval card; Full access arms the full tool set and auto-applies file writes, edits, and deletes for the session, while bash commands still park a card unless the command matches the allowlist. Session switching, new session, prompt copy/edit/rewind/fork, todo/plan state, and the subagent roster are local UI state, not Rust-authoritative. | `apps/selora-desktop/frontend/src/layout/AgentDock.tsx`, `features/agent/agent-store.ts`, `features/agent/components/AgentComposer.tsx`, `apps/selora-desktop/src-tauri/src/sessions.rs` |
| Agent tools | `read_file`, `list_dir`, `fuzzy_find`, `search_files`, `write_file`, `edit_file`, `delete_file`, `bash`, `web_search`, `web_fetch`, and `manage_todo`. Writes and deletes park an Approve/Deny card in Ask before changes and apply directly in Full access; PowerShell commands still park a card in both modes unless the command matches the `auto_approve_commands` allowlist, and allowlisted read-only commands run automatically. Defaults are 25 tool iterations, 600 seconds, and $1.00 estimated cost per turn, read from the `agent` section of `settings.json`. | `crates/selora-ai/src/agent/`, `apps/selora-desktop/src-tauri/src/sessions.rs` |
| Terminal | **Placeholder - intentionally inert.** Shows "Terminal placeholder" and "Not wired yet". No shell bridge exists. | `apps/selora-desktop/frontend/src/features/dock/TerminalPanel.tsx`, `layout/BottomDock.tsx` |
| Output | **Placeholder - intentionally inert.** Shows "No output yet" and says tasks land with the terminal slice. No task or output stream exists. | `apps/selora-desktop/frontend/src/features/dock/OutputPanel.tsx`, `layout/BottomDock.tsx` |
| Settings | Four tabs: AI Gateway, Editor, Theme, and Keymap. Theme and reduced-motion changes go through the Rust `settings_update` command and persist in `settings.json`; reduced motion applies immediately. The stored theme value does not restyle the UI because the stylesheet has one visual theme. Every other settings field, including the Gateway provider/model fields and Editor font/tab/wrap fields, is disabled with a "no backend settings field" title. The Gateway tab's API-key button opens Connect. The Keymap tab is a read-only table. | `apps/selora-desktop/frontend/src/features/sidebar/SettingsPanelView.tsx`, `apps/selora-desktop/src-tauri/src/state.rs` |
| Connect | Provider presets, API key field, gateway URL, Test Connection, and Disconnect. The key is written to Windows Credential Manager and to a best-effort plaintext fallback file, `.api_key`, in the settings directory; reads prefer the keyring. The gateway is probed with `list_models`. Open it from the status bar or Settings. | `apps/selora-desktop/frontend/src/features/auth/ConnectModal.tsx`, `apps/selora-desktop/src-tauri/src/commands.rs`, `crates/selora-settings/src/keyring.rs` |
| Command surfaces | Command palette with fuzzy filtering, category tabs, and recent commands; keyboard cheat sheet; About dialog with version, MIT, repository, and Check for updates; notification center with a persisted 50-entry history; toast stack capped at three visible; three-step onboarding persisted in `localStorage`; calm-mode switch in the activity bar. | `components/CommandPalette.tsx`, `components/CheatSheet.tsx`, `components/AboutDialog.tsx`, `components/NotificationCenter.tsx`, `components/ToastViewport.tsx`, `features/onboarding/OnboardingOverlay.tsx`, `layout/ActivityBar.tsx` |

## Keyboard

These are the default shortcuts in the command registry
(`apps/selora-desktop/frontend/src/features/keymap/commands.ts`). The palette
and agent-dock chords are rebindable through `settings.json`
(`keybinds.command_palette` and `keybinds.toggle_chat`); the others are fixed.

| Chord | Command |
|---|---|
| `Ctrl+Shift+P` | Open Command Palette |
| `Ctrl+B` | Toggle Primary Sidebar |
| `Ctrl+J` | Toggle Bottom Dock |
| `Ctrl+Shift+E` | Show Files |
| `Ctrl+Shift+F` | Show Search |
| `Ctrl+Shift+G` | Show Git |
| `Ctrl+Shift+M` | Show Problems |
| `Ctrl+,` | Show Settings |
| `Ctrl+L` | Toggle Agent Dock |
| `Ctrl+/` | Keyboard Shortcuts |

`Esc` closes the command palette when it is open; otherwise it dismisses the
top toast. Show Output, Show Terminal, About Selora, and Save Layout have no
default chord and are reachable from the palette or View menu.

Except for the palette chord, the global listener ignores shortcuts while the
palette is open or focus is in a text input, select, or content-editable
element (`features/keymap/use-shortcuts.ts`).

## Known Gaps

- **Agent and chat need a Selora key.** Store a gateway key with Connect
  (Windows Credential Manager plus a plaintext `.api_key` fallback in the
  settings directory), or set `SELORA_API_KEY` or `OPENAI_API_KEY`. With no
  key, requests fail with `not_connected`.
- **Language servers resolve from `PATH`, including npm `.cmd` shims.** Install
  `rust-analyzer` and `typescript-language-server` on `PATH`. On Windows the
  resolver probes `.exe` first, then the `.cmd`/`.bat` shims npm installs, and
  launches a `.cmd`/`.bat` through `cmd.exe`, so npm's
  `typescript-language-server.cmd` works. A missing server
  produces a warning toast and the file still opens, but without hover,
  completion, definition, or diagnostics. The desktop bridge still does not
  apply the settings-file LSP path overrides.
- **No manual Windows walkthrough has been recorded.** Every gate is automated
  and headless. The real WebView2 runtime path, the packaged folder picker, and
  the installer have not been exercised on a clean machine for this build.
- **No installer artifact is present.** `installer/output/` does not exist in
  this checkout and `tauri.conf.json` sets `bundle.active=false`; the Inno
  script is compiled locally (see Build The Installer above) or by a release
  tag.
- **No Git history in this checkout.** The folder has no `.git`, so there is no
  commit, branch, or diff record; work is verified by building and reading the
  tree.
- **The update check needs the GitHub repository published.** The About
  dialog fetches
  `https://api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest`; an
  unpublished repository or a missing release makes the check fail. The
  endpoint is derived from `Cargo.toml` / `package.json` `repository`, not
  hard-coded; `scripts/check-release-metadata.ps1` keeps the two manifests and
  the docs on one slug.
- **Artifacts are unsigned until signing secrets exist.** The release
  workflow signs the executable and installer only when
  `SIGNING_CERT_BASE64` and `SIGNING_CERT_PASSWORD` are configured; otherwise
  Windows reports no verified publisher.

## Other Current Limits

- Theme and reduced-motion changes persist through `settings_update`; every
  other Settings field is disabled or read-only. The stored theme value does
  not restyle the UI because the stylesheet ships one theme, and the panel
  states that the remaining controls are read-only. The File, Edit, and Help
  menus are disabled.
- External links (the About repository and Connect's provider docs) open in the
  OS browser through Rust's `open_external` command, which rejects any URL whose
  host is not on its allowlist; a non-allowlisted provider link renders
  disabled with an explanatory title.
- The Search panel sends literal queries by default and sets `use_regex` when
  the Regex toggle is on; Rust compiles the pattern and reports `invalid_query`
  inline for a bad one. There are no split editor panes.
- Settings > Keymap is read-only and lists Go to file (`Ctrl+P`) and Format
  document (`Shift+Alt+F`), neither of which has a command handler in this
  build. The command registry in the Keyboard section is authoritative.
- PLAN.md section 2 keeps these out of the Lite scope: real-time
  collaboration, a built-in terminal, agent sandboxing, background or
  unattended agents, a debugger, an extension marketplace, mobile/web clients,
  macOS/Linux installers, code signing, a custom UI framework, and a profiler
  with flame charts and traces.

## Documentation

- `PLAN.md` - scope, Lite non-goals, and delivery phases.
- `ARCHITECTURE.md` - technical contract.
- `DESIGN_TOKENS.md` - visual tokens.
- `UI_SPEC.md` - screen layout contract.
- `ROADMAP.md` - phase breakdown and exit gates.
- `RELEASE.md` - build, installer, and signing runbook.
- `docs/ui/PHASE_Z_PARITY_REPORT.md` - protocol and UI parity audit.
- `docs/ui/Z6_A11Y_AUDIT.md` - accessibility and reduced-motion audit.
- `CONTRIBUTING.md` - contribution rules.

## License

MIT. See `LICENSE`. Bundled Space Grotesk and JetBrains Mono fonts are SIL OFL
1.1.
