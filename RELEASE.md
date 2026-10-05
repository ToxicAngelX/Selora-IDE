# Selora IDE — Release

This document is the runbook for building, packaging, and publishing Selora IDE Lite Edition. It covers local builds, the Windows installer, the GitHub Actions CI/CD pipeline, code signing, and the open-source publication checklist.

The intended audience is the owner running the build, and the orchestrating AI executing Phase 6.

---

## 1. Local build (Windows)

### 1.1 Prerequisites

| Tool | Version | Why |
|---|---|---|
| Rust toolchain | stable (pin in `rust-toolchain.toml`) | Compiles the IDE |
| `rustup` | latest | Manages toolchain |
| `cargo` | bundled with rustup | Build tool |
| `cargo-deny` | latest | License + advisory check |
| `cargo-fmt` | bundled | Format check |
| `clippy` | bundled | Lint |
| Inno Setup 6 | latest | Windows installer compiler |
| Git | any | Repo |
| `rust-analyzer` | latest | LSP for Rust files (development convenience) |
| `typescript-language-server` | latest | LSP for TS/JS (development convenience) |

Install Rust:
```powershell
winget install Rustlang.Rustup
rustup default stable
```

Install Inno Setup:
```powershell
winget install JRSoftware.InnoSetup
# or download from https://jrsoftware.org/isdl.php
```

`cargo-deny`:
```bash
cargo install cargo-deny --locked
```

### 1.2 Build commands

From the repo root:

```bash
# Debug build (fast, for development)
cargo build --workspace

# Release build (optimized, what we ship)
cargo build --workspace --release

# Run the app
cargo run -p selora-app --release

# Run all tests
cargo test --workspace

# Lint and format
cargo fmt --check
cargo clippy --workspace -- -D warnings

# License check
cargo deny check licenses
```

The release binary lives at `target/release/selora-app.exe`.

### 1.3 Cross-platform notes

- **Windows is the only first-class target for Lite.** macOS and Linux compile-targets are documented as future work.
- The codebase uses `#[cfg(target_os = "windows")]` minimally. Most code is portable. A future Phase 1.x can add `cargo build --target x86_64-apple-darwin` and `--target x86_64-unknown-linux-gnu` once the owner wants to.
- `windows-sys` is used for Credential Manager (`keyring` crate handles it transparently) and dark title bar (`DwmSetWindowAttribute`).

---

## 2. Windows installer (Inno Setup)

### 2.1 Why Inno Setup

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| **Inno Setup** | Scriptable, well-known, AI agents know it, single .exe output, free | Older UI feel | **Pick this** |
| NSIS | More flexible scripting | Steeper learning curve, more agent friction | Skip for Lite |
| MSI (WiX) | Native Windows installer format, Group Policy support | Complex XML, no simple agent path | Skip for Lite |
| MSIX | Modern Windows packaging | Requires signing cert, sandbox restrictions | Skip for Lite |

Inno Setup produces a single `.exe` that runs as a normal user (no admin required for our install location: `%LOCALAPPDATA%\Programs\Selora IDE`).

### 2.2 The script: `installer/selora-ide.iss`

The script is a Phase 6c deliverable. It must:

1. Read the version from `Cargo.toml` of `selora-app` at build time (use Inno Setup's preprocessor with a small `prebuild.bat` that greps the version).
2. Install to `%LOCALAPPDATA%\Programs\Selora IDE\`.
3. Copy the release binary (`target/release/selora-app.exe`).
4. Copy the bundled fonts (`assets/fonts/*.ttf`) into the install dir so the app finds them.
5. Copy the icon (`assets/icons/selora-ide.ico`).
6. Create a Start Menu shortcut "Selora IDE" under `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Selora IDE\`.
7. Add an uninstaller entry in `Add or Remove Programs`.
8. Register `.txt` as a default open with `selora-app.exe` (Lite only; no source files).
9. Show the brand mark in the installer's header.
10. Use the modern "Wizard" style with no unnecessary pages (Welcome, Install, Finish).

A reference skeleton (the subagent will fill in):

```iss
[Setup]
AppName=Selora IDE
AppVersion={#MyAppVersion}
AppPublisher=Toxic
AppPublisherURL=https://github.com/ToxicAngelX/Selora-IDE
AppSupportURL=https://github.com/ToxicAngelX/Selora-IDE/issues
DefaultDirName={localappdata}\Programs\Selora IDE
DisableProgramGroupPage=yes
DisableDirPage=yes
OutputDir=installer\output
OutputBaseFilename=Selora-IDE-Setup-{#MyAppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
SetupIconFile=assets\icons\selora-ide.ico
UninstallDisplayIcon={app}\selora-ide.ico
UninstallDisplayName=Selora IDE
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0

[Files]
Source: "target\release\selora-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\fonts\*.ttf"; DestDir: "{app}\fonts"; Flags: ignoreversion
Source: "assets\icons\selora-ide.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userappdata}\Microsoft\Windows\Start Menu\Programs\Selora IDE\Selora IDE"; Filename: "{app}\selora-app.exe"; IconFilename: "{app}\selora-ide.ico"

[Run]
Filename: "{app}\selora-app.exe"; Description: "{cm:LaunchProgram,Selora IDE}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\selora-ide"

[Registry]
Root: HKCU; Subkey: "Software\Classes\.txt\OpenWithProgids"; ValueType: string; ValueName: "SeloraIDE.txt"; ValueData: ""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\SeloraIDE.txt\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\selora-app.exe"" ""%1"""
```

### 2.3 Build the installer

```powershell
cd "C:\Users\Toxic\Desktop\Selora IDE"
cargo build --workspace --release
& "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\selora-ide.iss
```

Output: `installer\output\Selora-IDE-Setup-X.Y.Z.exe`. The file is ~25 MB.

### 2.4 Install on a test machine

1. Copy the `.exe` to the test machine.
2. Run it. Accept the defaults.
3. Verify Start Menu shortcut works.
4. Launch the app.
5. Verify Settings → Apps lists "Selora IDE" and the uninstaller works cleanly.
6. Open a `.txt` file by double-clicking — should open in Selora IDE.

---

## 3. Code signing (optional, documented)

### 3.1 Why it's optional for Lite

Authenticode certificates cost $200-400/year. EV certificates (which give SmartScreen reputation immediately) cost $300+/year. Without signing, Windows SmartScreen shows "Windows protected your PC" on first run. The user clicks "More info" → "Run anyway". This is annoying but not blocking.

For a solo open-source maintainer, the cost may not be worth it for the Lite Edition. We document signing as a one-line CI change to enable when the owner is ready.

### 3.2 What we ship in CI

The release workflow (next section) includes a `Sign` step that:
- Reads `SIGNING_CERT_PATH`, `SIGNING_CERT_PASSWORD` from GitHub secrets.
- If the secrets exist, signs the binary and the installer with `signtool.exe`.
- If they don't exist, logs a warning and continues unsigned.

This means the owner can add the cert later without changing the workflow.

### 3.3 The signing command (when ready)

```powershell
# Requires Windows SDK with signtool.exe
signtool.exe sign /f $env:SIGNING_CERT_PATH /p $env:SIGNING_CERT_PASSWORD `
  /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
  /d "Selora IDE" /du "https://github.com/ToxicAngelX/Selora-IDE" `
  target\release\selora-app.exe
```

For the installer, the same command against the `.exe` output of Inno Setup.

---

## 4. GitHub Actions CI

### 4.1 Continuous integration (`.github/workflows/ci.yml`)

Runs on every push and every PR to `main`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: stable
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - name: Format check
        run: cargo fmt --all -- --check
      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: Build
        run: cargo build --workspace --locked
      - name: Test
        run: cargo test --workspace --locked
      - name: cargo-deny
        uses: EmbarkStudios/cargo-deny-action@v2
        with:
          command: check
          arguments: --hide-inclusion
```

The LSP integration tests that require `rust-analyzer` or `typescript-language-server` on PATH are skipped in CI (not failed) when the binaries are absent. See `ARCHITECTURE.md` §13.

### 4.2 Release workflow (`.github/workflows/release.yml`)

Runs on tag push matching `v*.*.*`:

```yaml
name: Release

on:
  push:
    tags:
      - 'v*.*.*'

permissions:
  contents: write   # needed to create the GitHub Release

jobs:
  build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      - name: Read version
        id: version
        shell: bash
        run: |
          echo "VERSION=${GITHUB_REF_NAME#v}" >> $GITHUB_OUTPUT

      - name: Build release binary
        run: cargo build --workspace --release --locked

      - name: Sign binary (optional)
        if: ${{ env.SIGNING_CERT_PATH != '' }}
        shell: pwsh
        run: |
          # signtool.exe comes with Windows SDK on the runner
          signtool.exe sign /f "${{ secrets.SIGNING_CERT_PATH }}" `
            /p "${{ secrets.SIGNING_CERT_PASSWORD }}" `
            /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
            /d "Selora IDE" /du "https://github.com/ToxicAngelX/Selora-IDE" `
            target\release\selora-app.exe

      - name: Install Inno Setup
        uses: Minionguyjpro/Inno-Setup-Action@v1
        with:
          path: installer/selora-ide.iss

      - name: Build installer
        shell: pwsh
        run: |
          & "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" /DMyAppVersion=${{ steps.version.outputs.VERSION }} installer\selora-ide.iss

      - name: Sign installer (optional)
        if: ${{ env.SIGNING_CERT_PATH != '' }}
        shell: pwsh
        run: |
          signtool.exe sign /f "${{ secrets.SIGNING_CERT_PATH }}" `
            /p "${{ secrets.SIGNING_CERT_PASSWORD }}" `
            /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
            /d "Selora IDE Installer" /du "https://github.com/ToxicAngelX/Selora-IDE" `
            "installer\output\Selora-IDE-Setup-${{ steps.version.outputs.VERSION }}.exe"

      - name: Upload installer to release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            installer/output/Selora-IDE-Setup-${{ steps.version.outputs.VERSION }}.exe
          generate_release_notes: true
          draft: true   # owner publishes manually after smoke-testing the installer
```

### 4.3 The release flow

1. Owner updates version in `crates/selora-app/Cargo.toml`.
2. Owner writes a CHANGELOG entry in `CHANGELOG.md`.
3. Owner commits and pushes to `main`. CI runs.
4. Owner creates a tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.
5. Release workflow runs. Produces a draft GitHub Release with the `.exe` attached.
6. Owner downloads the `.exe`, installs on a clean Windows VM, smoke-tests.
7. Owner edits the release notes, unchecks "Draft", publishes.

The draft step is critical. We do not auto-publish releases. The owner always smoke-tests first.

---

## 5. Versioning

We use **Semantic Versioning 2.0** with a twist: the Lite Edition uses `0.x.y` until the owner declares it 1.0.0.

- `0.1.0` — Phase 0 + Phase 1 (foundation + spike, the tech demo).
- `0.2.0` — Phase 2 (editor core; "it feels like an editor").
- `0.3.0` — Phase 3 (LSP + git).
- `0.4.0` — Phase 4 (Selora AI).
- `0.5.0` — Phase 5 (Agent Mode).
- `0.6.0` — Phase 6 (polish + ship).
- `1.0.0` — The owner declares 1.0 when they want to. Not in scope of this plan.

The version is in:
- `crates/selora-app/Cargo.toml` (`version = "0.X.0"`)
- Injected into the installer via Inno Setup's preprocessor.
- Injected into the app's About dialog via `env!("CARGO_PKG_VERSION")` in code.

---

## 6. CHANGELOG

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/). One section per version:

```markdown
# Changelog

## [0.5.0] - YYYY-MM-DD

### Added
- First-run experience with splash
- Windows installer (Inno Setup)
- GitHub Actions release pipeline
- About dialog with version, license, link to repo
- "Check for updates" via GitHub Releases API

### Changed
- Status bar shows version on hover
- Settings dialog uses new layout

### Fixed
- LSP server crash on long-running edit session (#123)
```

The owner writes the changelog. Subagents update the "Unreleased" section as they go.

---

## 7. Open-source publication checklist

When Phase 6f lands, the repo is ready to be public. The owner runs:

```bash
# 1. Create the repo on GitHub
gh repo create ToxicAngelX/Selora-IDE --public --description "The editor born connected to Selora." --source=. --remote=origin

# 2. Push
git push -u origin main

# 3. Configure the repo
gh repo edit --enable-issues --enable-projects=false --enable-wiki=false

# 4. Add topics
gh repo edit --add-topic editor,rust,ai,code-editor,selora,egui

# 5. Set the description and homepage
gh repo edit --description "The editor born connected to Selora." --homepage "https://selora.lol"

# 6. Add a license
# (MIT LICENSE is already at the repo root)

# 7. Enable Discussions (optional)
gh repo edit --enable-discussions

# 8. Set up branch protection on main
# (configure via gh CLI or web UI: require PR + 1 approval, require CI to pass)
```

### 7.1 Files that must exist before publication

| File | Status | Phase |
|---|---|---|
| `LICENSE` (MIT) | created in Phase 0 | 0 |
| `README.md` | polished in Phase 6e | 5e |
| `CONTRIBUTING.md` | created in Phase 0, polished in Phase 6f | 5f |
| `CODE_OF_CONDUCT.md` | created in Phase 6f | 5f |
| `.github/ISSUE_TEMPLATE/bug_report.md` | Phase 6f | 5f |
| `.github/ISSUE_TEMPLATE/feature_request.md` | Phase 6f | 5f |
| `.github/PULL_REQUEST_TEMPLATE.md` | Phase 6f | 5f |
| `CHANGELOG.md` | created in Phase 0, updated each release | 0+ |
| `assets/screenshots/*.png` | Phase 6g | 5g |
| `.github/workflows/ci.yml` | Phase 0 | 0 |
| `.github/workflows/release.yml` | Phase 6d | 5d |
| CI badge in README | Phase 6e | 5e |
| `deny.toml` | Phase 0 | 0 |

### 7.2 What we do NOT publish

- Telemetry. None. The IDE does not phone home.
- The owner's machine-specific configuration. Nothing in `%APPDATA%\selora-ide\` is committed.
- A `.selora/` folder in user repos. The IDE never writes to the project directory.

### 7.3 What we DO publish (eventually, when written)

- A docs site at `https://selora-ide.selora.lol` (MkDocs + GitHub Pages). Phase 6e writes the source; deploying is a v1.x step.
- Screenshots. Phase 6g produces 4-5 PNGs.
- A short demo GIF for the README.

---

## 8. The release checklist (per release)

```markdown
## Pre-release

- [ ] All `Unreleased` CHANGELOG entries moved to a new versioned section
- [ ] `Cargo.toml` version bumped
- [ ] `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --workspace`, `cargo deny check licenses` all green
- [ ] Smoke test: install the locally-built release, verify all features work
- [ ] Tag created: `git tag vX.Y.Z`

## Release

- [ ] `git push origin vX.Y.Z`
- [ ] Wait for the release workflow to finish
- [ ] Download the attached `.exe` from the draft release
- [ ] Install on a clean Windows 11 VM
- [ ] Smoke test:
  - [ ] App launches
  - [ ] First-run splash appears
  - [ ] Connect a real sk-gw- key
  - [ ] Open a folder
  - [ ] Open a Rust file, F12 works
  - [ ] Chat streams a real response
  - [ ] Ghost text appears and accepts
  - [ ] Git commit works
  - [ ] Settings persist
- [ ] Edit release notes
- [ ] Publish release

## Post-release

- [ ] Announce on https://selora.lol (optional)
- [ ] Post on relevant communities (Reddit r/rust, Hacker News, etc.) — optional, not Phase 6 scope
```

---

## 9. Open questions (for the owner, before/during Phase 6)

These are decisions the owner should make, not subagents:

1. **Repo name confirmation.** `ToxicAngelX/Selora-IDE` — final, or change?
2. **License.** MIT is recommended. Apache 2.0 if the owner wants a patent grant. Confirm.
3. **Display name in installer.** "Selora IDE" or "Selora IDE Lite"? The plan assumes "Selora IDE" with the Lite tag in version metadata.
4. **What the .txt file association should open.** A blank new buffer? The file's contents? The plan assumes the file's contents in the IDE.
5. **Whether to register a default for source files.** The plan does NOT register `.rs`/`.ts`/etc. as defaults. The owner can opt in.
6. **Auto-update behavior.** Lite does not auto-update. The user downloads a new installer from the GitHub Releases page. Owner can change this in v1.x.
7. **Code signing cert.** Owner decides whether to buy one. The CI supports it as a no-op until then.

---

## 10. After Lite ships

When the Lite Edition is on a public GitHub Release, the project is at a fork in the road. Three plausible next moves, all out of scope for this plan:

1. **Stop.** The owner uses the editor. The project is "done" in the open-source sense. Maintenance mode.
2. **Polish.** v1.0.0 with a real release announcement, code signing, a docs site, screenshots, and a short demo video.
3. **Extend.** Add a v1.x feature (built-in terminal, debugger, more languages, custom themes, Linux/macOS installers, an extension model).

The plan does not choose. The owner chooses after living with the Lite Edition for a few weeks.
