# Selora IDE — Release

This document is the runbook for building, packaging, and publishing Selora IDE Lite Edition. It covers local builds, the Windows installer, the GitHub Actions CI/CD pipeline, code signing, and the open-source publication checklist.

The intended audience is the owner running the build, and the orchestrating AI executing Phase 6.

---

## 1. Local build (Windows)

### 1.1 Prerequisites

| Tool | Version | Why |
|---|---|---|
| Rust toolchain | stable (pin in `rust-toolchain.toml`) | Compiles the Tauri shell and workspace |
| `rustup` / `cargo` | latest | Toolchain management, build tool |
| MSVC Build Tools 2022 | v143 x64 + Windows 10 SDK | Linker and the CRT redist DLLs the installer ships |
| Node.js | 22 LTS | Builds the React/CodeMirror frontend |
| npm | bundled with Node | `npm ci`, typecheck, tests, `npm run build` |
| Inno Setup | 6.3 or newer (6.x or 7.x; CI pins 6.7.1) | Windows installer compiler (`x64compatible`) |
| `cargo-deny` | latest | License + advisory check |
| Git | any | Repo |
| `rust-analyzer` | latest | LSP for Rust files (development convenience) |
| `typescript-language-server` | latest | LSP for TS/JS (development convenience) |

Install Rust:
```powershell
winget install Rustlang.Rustup
rustup default stable
```

Install Node.js:
```powershell
winget install OpenJS.NodeJS.LTS
```

Install Visual Studio Build Tools 2022 with the **Desktop development with
C++** workload (MSVC v143 x64/x86 build tools + Windows SDK). The standalone
Rust MSVC target needs the linker, and `installer\prebuild.bat` reads the CRT
DLLs from this installation's `VC\Redist\MSVC` directory.

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

```powershell
# One-time install / after package-lock.json changes
npm --prefix apps/selora-desktop/frontend ci

# Frontend checks
npm --prefix apps/selora-desktop/frontend run typecheck
npm --prefix apps/selora-desktop/frontend test

# Rust checks
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked

# Release build - frontend first, then the Tauri shell
npm --prefix apps/selora-desktop/frontend run build
cargo build --release -p selora-desktop --locked

# License check
cargo deny check
```

The release binary lives at `target/release/selora-desktop.exe`. The
frontend bundle is written to `apps/selora-desktop/frontend/dist/`, which is
gitignored but embedded into the binary by `tauri-build` at compile time -
so the frontend build must run before any `cargo build`/`cargo test` that
compiles `selora-desktop`.

For day-to-day development, run the Vite dev server and a debug build in two
terminals (the Tauri config's `devUrl` points at port 1420):

```powershell
npm --prefix apps/selora-desktop/frontend run dev
cargo run -p selora-desktop
```

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

`installer/selora-ide.iss` is the source of truth. It does exactly this:

1. Receives the version through `/DMyAppVersion` (from `prebuild.bat`
   locally, from the tag in CI). It refuses to compile without it.
2. Installs to `%LOCALAPPDATA%\Programs\Selora IDE\` with
   `PrivilegesRequired=lowest` - no admin prompt.
3. Copies the Tauri shell `target\release\selora-desktop.exe`.
4. Copies `assets\icons\selora-ide.ico` into the install directory.
5. Copies the MSVC runtime DLLs `vcruntime140.dll` and `vcruntime140_1.dll`
   beside the exe. `msvcp140.dll` is copied only when it was staged (the
   current x64 binary does not import it; see 2.2.1).
6. Bundles Microsoft's WebView2 Evergreen Bootstrapper in `{tmp}` and runs
   it only when the `[Code]` registry check finds no WebView2 runtime. The
   check reads the `pv` value from the EdgeUpdate clients key for Microsoft's
   WebView2 GUID `{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` in `HKCU`, the
   `HKLM32` (WOW6432Node) view, and the native `HKLM64` view. The
   bootstrapper is signed by Microsoft in CI, but it is optional at local
   compile time so an offline build still produces an installer.
7. Creates the Start Menu shortcut under
   `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Selora IDE.lnk`.
8. Adds the uninstaller entry in Settings -> Apps.
9. Adds Selora IDE to the `.txt` "Open with" list through HKCU
   `OpenWithProgids`; it does not take over the user's default app.
10. Uses the modern wizard with Welcome, Install, and Finish only.

Fonts are **not** copied separately: Space Grotesk and JetBrains Mono are
`include_bytes!`-embedded in `selora-ui`, so there is no runtime font
directory to populate.

**WebView2 policy: Evergreen.** No fixed-version runtime is bundled or
shipped. The app uses the machine's Evergreen WebView2 runtime, and Setup
runs Microsoft's bootstrapper only when the registry check finds none.

#### 2.2.1 Why the CRT DLLs are required

`apps/selora-desktop/src-tauri/build.rs` calls `static_vc_runtime(false)`.
The resulting x64 binary imports `VCRUNTIME140.dll` and
`VCRUNTIME140_1.dll` (verified with `dumpbin /dependents`); a machine
without the VC++ Redistributable would fail at process start. Shipping the
DLLs beside the exe keeps the per-user install admin-free and works without
installing the system-wide redistributable.

Setup checks after files are copied that both DLLs exist in `{app}` and
warns with the VC++ 2015-2022 x64 Redistributable remedy if a hand-edited
script omitted them.

### 2.3 Build the installer

```powershell
installer\prebuild.bat
```

Run it from anywhere; the script enters the repository root itself.
`prebuild.bat` is quote-safe for checkout paths with spaces. It:

1. Runs `installer\read-version.ps1`, which reads
   `[workspace.package].version` from the root `Cargo.toml` and requires a
   numeric `x.y.z` (Inno Setup's `AppVersion`).
2. Resolves `ISCC.exe` through `installer\find-iscc.ps1` **before** it stages
   anything, so a missing compiler fails fast:
   - `%SELORA_ISCC%` if set (absolute path to `ISCC.exe`),
   - `ISCC.exe` on `PATH`,
   - `Inno Setup 6` / `Inno Setup 7` under `%ProgramFiles%`,
     `%ProgramFiles(x86)%`, and the per-user `%LOCALAPPDATA%\Programs`,
   - the uninstall registry (`HKLM` 64-bit, `HKLM\WOW6432Node`, `HKCU`):
     `InstallLocation`, `DisplayIcon`, and `Inno Setup: App Path`.

   The registry pass is what finds a portable or extracted Inno Setup whose
   only trace is an uninstall entry. The Phase 8 acceptance machine had one
   under `%TEMP%`, which the old Program Files-only probe could not see.
3. Requires `target\release\selora-desktop.exe` to exist (it does not build
   the app for you).
4. Locates the newest MSVC x64 CRT redist under the Visual Studio
   installation found by `vswhere.exe` and copies `vcruntime140.dll` /
   `vcruntime140_1.dll` into `target\release\`. It runs
   `dumpbin /dependents` and adds `msvcp140.dll` only when the built exe
   imports it.
5. Downloads Microsoft's Evergreen Bootstrapper to
   `target\release\MicrosoftEdgeWebview2Setup.exe` when it is not cached and
   verifies its Authenticode signature is valid and signed by Microsoft. The
   download retries up to three times, deletes a partial file on failure, and
   pins Windows PowerShell's module path so an inherited PowerShell 7
   `PSModulePath` cannot break the signature check. Offline (or on a failed
   signature), it warns and continues; the installer then tells the user to
   install WebView2 manually if the runtime is missing.
6. Creates `installer\output\`, deletes a previous setup for the same
   version, and runs
   `ISCC.exe /DMyAppVersion=<version> /DMyTargetDir=<abs> /DMyCrtDlls=<list> /O<repo>\installer\output installer\selora-ide.iss`.
   The absolute `/O` is deliberate: the compiler's install directory and the
   current directory cannot move the artifact.
7. Fails unless `installer\output\Selora-IDE-Setup-X.Y.Z.exe` exists, then
   prints its absolute path and size.

Output: `installer\output\Selora-IDE-Setup-X.Y.Z.exe`. Size depends on the
binary and on whether the WebView2 bootstrapper was staged.

`installer\prebuild.bat --help` prints usage and writes nothing. A hand-run
`ISCC.exe /DMyAppVersion=X.Y.Z installer\selora-ide.iss` also writes to
`installer\output\` (the `.iss` default), because `{#SourcePath}` resolves
against the script's own directory.

### 2.4 Install on a test machine

1. Copy the `.exe` to the test machine.
2. Run it. Accept the defaults; there should be no UAC prompt.
3. Verify `%LOCALAPPDATA%\Programs\Selora IDE\` contains
   `selora-desktop.exe`, `selora-ide.ico`, `vcruntime140.dll`, and
   `vcruntime140_1.dll`.
4. Verify the Start Menu shortcut works.
5. Launch the app. On a Windows 10 VM without WebView2, the installer should
   install the Evergreen runtime first (network required); with a runtime
   already present, it must not run the bootstrapper.
6. Verify Settings -> Apps lists "Selora IDE" and the uninstaller works
   cleanly (including `%APPDATA%\selora-ide`).
7. Use "Open with" on a `.txt` file - Selora IDE should be listed without
   changing the existing default.

---

## 3. Code signing (optional, documented)

### 3.1 Why it's optional for Lite

Authenticode certificates cost $200-400/year. EV certificates (which give SmartScreen reputation immediately) cost $300+/year. Without signing, Windows SmartScreen shows "Windows protected your PC" on first run. The user clicks "More info" → "Run anyway". This is annoying but not blocking.

For a solo open-source maintainer, the cost may not be worth it for the Lite Edition. We document signing as a one-line CI change to enable when the owner is ready.

### 3.2 What we ship in CI

The release workflow has two optional signing steps, gated on a job-level
environment flag derived from the certificate secret:

- Secrets: `SIGNING_CERT_BASE64` (the PFX, base64-encoded) and
  `SIGNING_CERT_PASSWORD`.
- When `SIGNING_CERT_BASE64` is empty, both steps are skipped and the
  artifacts are unsigned. The workflow still builds and drafts the release.
- When it is set, the workflow writes the PFX to `RUNNER_TEMP`, locates
  `signtool.exe` in the Windows SDK, signs `target\release\selora-desktop.exe`
  **before** ISCC compiles the installer, signs the installer **before**
  the SHA256 checksum is generated, and deletes the PFX.

This means the owner can add the certificate later without changing the
workflow. Signing does not change the per-user install or the WebView2/CRT
behavior.

### 3.3 The signing command (when ready)

```powershell
# Requires Windows SDK with signtool.exe
# $env:SIGNING_PFX = "C:\path\to\selora.pfx"
# $env:SIGNING_CERT_PASSWORD = "the PFX password"
signtool.exe sign /f $env:SIGNING_PFX /p $env:SIGNING_CERT_PASSWORD `
  /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
  /d "Selora IDE" /du "https://github.com/ToxicAngelX/Selora-IDE" `
  target\release\selora-desktop.exe
```

For the installer, the same command against the `.exe` output of Inno Setup.

---

## 4. GitHub Actions CI

### 4.1 Continuous integration (`.github/workflows/ci.yml`)

Runs on every push to `main` and every pull request. The job is
`windows-latest`, `contents: read`, with a concurrency group that cancels
superseded runs. Steps, in order:

1. `actions/checkout` (pinned release tag).
2. `scripts\check-release-metadata.ps1`: the three manifests, the tag (on tag
   pushes), the CHANGELOG section, one repository slug, and the update-check
   derivation/CSP. No build needed.
3. `actions/setup-node` with Node 22 and npm caching keyed on
   `apps/selora-desktop/frontend/package-lock.json`.
4. `npm ci` in `apps/selora-desktop/frontend`.
5. `npm run typecheck`.
6. `npm test` (Vitest).
7. `npm run build` (tsc + Vite) and a check that `dist/index.html` exists.
8. Rust stable with `rustfmt` + `clippy`, then `Swatinem/rust-cache`.
9. `cargo fmt --all -- --check`.
10. `cargo clippy --workspace --all-targets --locked -- -D warnings`.
11. `cargo build --workspace --locked`.
12. `cargo test --workspace --locked`.
13. `cargo install cargo-deny --version 0.20.2 --locked`, then
    `cargo deny check` (licenses + advisories). The upstream
    `cargo-deny-action` is a Docker
    container action, which GitHub Actions cannot run on `windows-latest`,
    so CI installs the tool natively instead.

The frontend build must come first: `apps/selora-desktop/frontend/dist/` is
gitignored (see `apps/selora-desktop/.gitignore`) and `tauri-build` embeds
it into `selora-desktop` while compiling. A checkout without the frontend
build cannot compile the Tauri crate.

The LSP integration tests that require `rust-analyzer` or
`typescript-language-server` on PATH are skipped in CI (not failed) when the
binaries are absent. See `ARCHITECTURE.md` §13.

### 4.2 Release workflow (`.github/workflows/release.yml`)

Runs on a `v*.*.*` tag push, `windows-latest`, with `contents: write` and
serialized releases. Third-party actions are pinned to release tags; bump
them deliberately. Steps, in order:

1. Checkout with full history (`fetch-depth: 0`), then verify the tagged
   commit is an ancestor of `origin/main`. A tag bypasses branch protection,
   so this is the check that proves the commit was already merged and passed
   CI; the workflow fails when `origin/main` is absent or the tag is not
   contained in it.
2. Resolve the version from the tag through
   `scripts\check-release-metadata.ps1 -Tag <tag>`. That script requires a
   numeric `X.Y.Z` tag and equality across **all three** manifests:
   `Cargo.toml` `[workspace.package].version` (through
   `installer\read-version.ps1`), `tauri.conf.json` `version`, and
   `frontend/package.json` `version`. It also requires a `CHANGELOG.md`
   `## [<version>]` heading and one consistent repository slug across
   `Cargo.toml`, `frontend/package.json`, `installer/selora-ide.iss`, and
   `docs/mkdocs.yml`. Any mismatch fails before anything is built.
3. Node 22 + npm cache; `npm ci`, `npm run typecheck`, `npm test`,
   `npm run build`, then verify `dist/index.html`.
4. Rust stable + `Swatinem/rust-cache` (shared `release-windows` key).
5. `cargo fmt --all -- --check`.
6. `cargo clippy --workspace --all-targets --locked -- -D warnings`.
7. `cargo install cargo-deny --version 0.20.2 --locked`, then
   `cargo deny check` (licenses + advisories). A tag must not be able to skip
   the merge gate that `ci.yml` runs.
8. `cargo test --workspace --locked`.
9. `cargo build --release -p selora-desktop --locked`.
10. Verify `target\release\selora-desktop.exe` exists.
11. Stage the MSVC runtime DLLs from `VC\Redist\MSVC\<newest>\x64\Microsoft.VC*.CRT`.
    The step runs `dumpbin /dependents` on the built exe and fails if it no
    longer imports `VCRUNTIME140.dll` (a static-CRT change must update the
    installer), then copies `vcruntime140.dll` + `vcruntime140_1.dll` (and
    `msvcp140.dll` when imported) beside the exe.
12. Download Microsoft's x64 Evergreen Bootstrapper
    (`https://go.microsoft.com/fwlink/p/?LinkId=2124703`) to
    `target\release\MicrosoftEdgeWebview2Setup.exe`, verify its Authenticode
    signature is valid and signed by Microsoft, and fail otherwise.
13. `choco install innosetup --version=6.7.1` (pinned so a future 7.x
    Chocolatey package cannot silently change the compiler under a release),
    then resolve the compiler with `installer\find-iscc.ps1`. The workflow
    does not assume Chocolatey's install path.
14. Optional: sign `target\release\selora-desktop.exe` (see §3).
15. Create `installer\output\`, delete a stale setup for the tag version, and
    run ISCC with `/DMyAppVersion=<tag version> /DMyCrtDlls=<dumpbin list>
    /O<runner>\installer\output`. The absolute `/O` makes the artifact path
    independent of the compiler's install directory and the current
    directory. The step fails if the setup is missing or smaller than 5 MiB.
16. Optional: sign the installer.
17. Verify the installer signature matches the configuration: `Valid` when
    `SIGNING_CERT_BASE64` is set, `NotSigned` when it is not. A half-signed
    artifact cannot reach the draft.
18. Generate `Selora-IDE-Setup-X.Y.Z.exe.sha256` from the final (possibly
    signed) installer, assert the exact `<64 lowercase hex><two spaces><name>`
    shape, then hash the installer again to prove it did not change.
19. Upload the installer and checksum as a workflow artifact (14 days).
20. Refuse to continue if a release for the tag already exists and is not a
    draft; publishing is the owner's manual step.
21. Open a **draft** GitHub Release and attach the installer + `.sha256`,
    with generated release notes. The workflow never publishes.
22. Verify the draft: `isDraft` is true, both assets exist, the uploaded
    installer size equals the local size, and a `gh release download`
    round-trip re-hashes to the local SHA256.

### 4.3 The release flow

1. Bump the version in the root `Cargo.toml` (`[workspace.package]`),
   `apps/selora-desktop/src-tauri/tauri.conf.json`, and
   `apps/selora-desktop/frontend/package.json`. All three must equal the tag.
   The release workflow enforces all three (the frontend package version is
   user-visible: Vite injects it into the About dialog), not two of them.
2. Move the `CHANGELOG.md` entries out of `Unreleased` into a new versioned
   section with a `## [X.Y.Z] - YYYY-MM-DD` heading, and leave `Unreleased`
   empty. The workflow requires a `## [<version>]` heading and fails the tag
   without one.
3. Run the checks from §1.2 locally (frontend typecheck/test/build first), plus
   `scripts\check-release-metadata.ps1 -Tag vX.Y.Z` to prove the tag will pass
   the release gate.
4. Commit and push `main`; wait for CI. The tag must point at a commit that
   is already on `main`: the release workflow verifies the tagged commit is an
   ancestor of `origin/main` and re-runs `cargo fmt`, `cargo clippy`, and
   `cargo deny` itself rather than trusting the branch run.
5. Create and push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.
6. The release workflow produces a draft GitHub Release with the installer
   and its SHA256 file. The binary and installer are **unsigned** unless the
   `SIGNING_CERT_BASE64` and `SIGNING_CERT_PASSWORD` secrets are set;
   `SIGNING_REQUIRED=true` turns a missing certificate into a hard failure.
7. Download both. Verify the hash on Windows:

   ```powershell
   Get-FileHash .\Selora-IDE-Setup-X.Y.Z.exe -Algorithm SHA256
   Get-Content .\Selora-IDE-Setup-X.Y.Z.exe.sha256
   ```

   The hash must match the first field of the `.sha256` file.
8. Install on a clean Windows VM and run the §2.4 + §8 smoke tests.
9. Edit the release notes, uncheck "Draft", publish.

The draft step is critical. We do not auto-publish releases. The owner always smoke-tests first.

---

## 5. Versioning

We use **Semantic Versioning 2.0**: the Lite Edition stays on `0.x.y` until the
owner declares 1.0.0. All three manifests currently read **0.1.0**.

**Decision (2026-10-27): the first published release is `v0.1.0`.** The
project did not ship the per-phase `0.1.0` -> `0.6.0` ladder from the original
egui plan, so the first public tag is `0.1.0`, not `0.6.0` and not `1.0.0`.
No version bump is needed for this tag: `CHANGELOG.md` already carries a
`## [0.1.0] - 2026-10-26` section, and all three manifests agree. The release
stays a draft until the owner runs the clean-VM smoke test in section 8 and
publishes it manually.

A release tag `vX.Y.Z` must equal all three version fields, and the release
workflow enforces all three plus the changelog:

- Root `Cargo.toml` `[workspace.package].version` - the Rust source of truth.
  `installer\read-version.ps1` reads exactly this table, and the release
  workflow validates the tag against it.
- `apps/selora-desktop/src-tauri/tauri.conf.json` `version` - Tauri's
  metadata; the release workflow validates it too.
- `apps/selora-desktop/frontend/package.json` `version` - Vite injects it as
  `__APP_VERSION__`, which drives the About dialog and its update check. The
  release workflow validates it against the tag as well; `ci.yml` does not
  compare versions, so an ordinary push is not gated on it.
- `CHANGELOG.md` - the workflow requires a `## [<version>]` heading. A tag
  whose version has no changelog section fails before any build starts.

The version is also injected into the installer via `/DMyAppVersion` (Inno
preprocessor). The About dialog does **not** read `env!("CARGO_PKG_VERSION")`.

Release tags are not merge shortcuts. The tag must point at a commit that is
already on `main`, and the release workflow re-runs `cargo fmt`, `cargo
clippy`, and `cargo deny` against that commit before it builds anything.
Artifacts stay unsigned unless the signing secrets from §3.2 are present.

> The earlier `0.1.0` → `0.6.0` per-phase table was a planning artifact from
> the original egui plan. It is historical and does not describe the shipped
> manifests, which are all 0.1.0.

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
| `scripts/check-release-metadata.ps1` | 2026-10-27: version + slug gate | release |
| `installer/find-iscc.ps1` | 2026-10-27: ISCC resolution | release |
| `RELEASE_0.1.0_CHECKLIST.md` | 2026-10-27: draft-asset checklist | 0.1.0 |
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

> **Current tree (2026-10-27): NOT FINALIZED.** The checklist below has not been
> run against the merged tree. Artifacts are unsigned, no clean-VM install was
> performed, and owner acceptance is unrun. Record the actual release in §11
> after the final gate; do not check boxes from pre-merge evidence.

```markdown
## Pre-release

- [ ] All `Unreleased` CHANGELOG entries moved to a new versioned section
- [ ] Root `Cargo.toml` (`[workspace.package]`), `tauri.conf.json`, **and**
      `frontend/package.json` versions set to the same value as the tag
- [ ] `scripts\check-release-metadata.ps1 -Tag vX.Y.Z` green (version,
      CHANGELOG heading, repository slug, update-check derivation, CSP)
- [ ] `npm ci`, `npm run typecheck`, `npm test`, `npm run build` green
- [ ] `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
      `cargo test --workspace --locked`, `cargo deny check` all green
- [ ] `installer\prebuild.bat` builds `installer\output\Selora-IDE-Setup-X.Y.Z.exe`
      (not `%TEMP%`; see section 2.3), and the install dir has
      `selora-desktop.exe`, `vcruntime140.dll`, and `vcruntime140_1.dll`
- [ ] Smoke test: install the locally-built release, verify the app launches
- [ ] Tag created: `git tag vX.Y.Z`

## Release

- [ ] `git push origin vX.Y.Z`
- [ ] Wait for the release workflow to finish
- [ ] Confirm the workflow's "Verify draft release assets" step passed
      (draft=true, two assets, round-trip SHA256 match)
- [ ] Download the attached `.exe` and `.exe.sha256` from the draft release
- [ ] Verify the SHA256 (`Get-FileHash -Algorithm SHA256` or `certutil -hashfile`)
- [ ] Install on a clean Windows 11 VM (and, when WebView2 changes, a Windows
      10 VM without the WebView2 runtime)
- [ ] Confirm the installer does not prompt for admin and does not run the
      WebView2 bootstrapper when a runtime is already registered
- [ ] Smoke test:
  - [ ] App launches
  - [ ] First-run splash appears
  - [ ] Connect a real sk-gw- key
  - [ ] Open a folder
  - [ ] Open a Rust file, edit, save, and use find/replace
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
4. **What the .txt file association should open.** The installer currently
   only registers the **Open with** entry; the Tauri shell ignores `%1`, so
   the selected file is not loaded yet. The owner decides the follow-up:
   open the file's contents, or drop the association until argv handling
   exists.
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

---

## 11. Post-merge release record (skeleton)

> **STATUS: SKELETON (2026-10-27).** Do not publish from this section until
> every `TODO-VERIFY` is replaced from the final commit. This section records
> one release; the checklist in §8 is the procedure.

### 11.1 Release identity

| Field | Value |
|---|---|
| Version | `TODO-VERIFY` (all three manifests must agree) |
| Tag | `TODO-VERIFY` (`vX.Y.Z`) |
| Commit | `TODO-VERIFY` (full hash) |
| Branch | `TODO-VERIFY` (must be `main` for a release tag) |
| Date | `TODO-VERIFY` |
| Built by | `TODO-VERIFY` |
| Tree clean before build | `TODO-VERIFY` (`git status --short` empty) |

### 11.2 Gate on the release commit

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | `TODO-VERIFY` |
| `cargo clippy --workspace --all-targets -- -D warnings` | `TODO-VERIFY` |
| `cargo test --workspace --no-fail-fast` | `TODO-VERIFY` |
| `cargo deny check` | `TODO-VERIFY` |
| `npm run typecheck` | `TODO-VERIFY` |
| `npm test` | `TODO-VERIFY` |
| `npm run build` | `TODO-VERIFY` |
| `scripts/phase8-acceptance.ps1` | `TODO-VERIFY` |
| Phase 8 owner acceptance | **UNRUN** |
| Phase 9 owner acceptance | **UNRUN** (partial waves) |

Do not fill this table from an earlier commit. Re-run the gate on the tagged
commit.

### 11.3 Artifacts

| Artifact | Path | Size | SHA-256 | Authenticode |
|---|---|---|---|---|
| Release app | `target\release\selora-desktop.exe` | `TODO-VERIFY` | `TODO-VERIFY` | `TODO-VERIFY` |
| Installer | `TODO-VERIFY` | `TODO-VERIFY` | `TODO-VERIFY` | `TODO-VERIFY` |
| Checksum file | `TODO-VERIFY` | - | - | - |

Current state: **UNSIGNED**. `SIGNING_CERT_BASE64` and
`SIGNING_CERT_PASSWORD` are not configured. Both artifacts are expected to
report `NotSigned`; SmartScreen will warn.

### 11.4 Clean-VM install

| Test | Result |
|---|---|
| Windows 11 clean VM, installer, launch | **NOT RUN** |
| Windows 10 without WebView2 | **NOT RUN** |
| Per-user install (no admin prompt) | **NOT RUN** |
| WebView2 bootstrapper skipped when runtime present | **NOT RUN** |
| `.txt` Open-with entry | **NOT RUN** |
| Uninstall | **NOT RUN** |
| Launch after install | **NOT RUN** |

### 11.5 Owner-only release steps

- [ ] Confirm the version and tag
- [ ] Provide signing certificate secrets, or accept unsigned artifacts
- [ ] Run Phase 8 owner steps 1-9 (real key, real workspace, visible window)
- [ ] Run the Phase 9 steps supported by merged waves; mark the rest blocked
- [ ] Install on a clean Windows 11 VM
- [ ] Optionally test Windows 10 without WebView2
- [ ] Publish the GitHub repository/release, or change the update-check slug
- [ ] Edit release notes from `CHANGELOG.md`
- [ ] Publish the draft release

### 11.6 Known release limits

- Artifacts are unsigned and will trigger SmartScreen.
- No clean-VM install has been performed.
- The update check returns 404 until the public repository/release exists.
- Phase 8 owner acceptance is unrun.
- Phase 9 is partial: 9a branch-only, 9b-9e not implemented, 9d partial.
- No test counts or hashes are valid until the final gate writes them here.
- Full auto-apply remains the default composer mode pending owner confirmation.
