# Installation

## Requirements

- Windows 10 (1809 or later) or Windows 11, x64.
- Microsoft Edge WebView2 Runtime. Windows 11 normally includes it; the
  installer checks for it and runs Microsoft's Evergreen bootstrapper only
  when it is missing (that step needs an internet connection).
- No administrator rights: the app installs per-user.

The installer ships the Visual C++ runtime DLLs it needs
(`vcruntime140.dll` and `vcruntime140_1.dll`) beside the executable, so a
separate VC++ Redistributable install is not required.

## Installer (Windows)

1. Download `Selora-IDE-Setup-X.Y.Z.exe` and
   `Selora-IDE-Setup-X.Y.Z.exe.sha256` from the latest
   [GitHub Release](https://github.com/ToxicAngelX/Selora-IDE/releases).
2. Optionally verify the download:

   ```powershell
   Get-FileHash .\Selora-IDE-Setup-X.Y.Z.exe -Algorithm SHA256
   Get-Content .\Selora-IDE-Setup-X.Y.Z.exe.sha256
   ```

   The hash must equal the first field in the `.sha256` file.
3. Run the installer. It installs to
   `%LOCALAPPDATA%\Programs\Selora IDE\` with no admin prompt, adds a Start
   Menu shortcut, registers an **Open with** entry for `.txt` files (without
   changing your default app), and adds an uninstaller entry.
4. If WebView2 is missing, Setup runs the bundled Evergreen bootstrapper
   before offering to launch the app.
5. Launch **Selora IDE** from the Start Menu.

!!! note "SmartScreen"
    Builds are not code-signed yet (signing is wired in CI but off until a
    certificate is provided - see `RELEASE.md` section 3). Windows SmartScreen
    may show "Windows protected your PC" on first run: **More info -> Run
    anyway**.

## What the installer puts on disk

Inside `%LOCALAPPDATA%\Programs\Selora IDE\`:

- `selora-desktop.exe` - the Tauri/WebView2 application.
- `selora-ide.ico` - the shortcut/uninstaller icon.
- `vcruntime140.dll` and `vcruntime140_1.dll` - the MSVC runtime the x64
  binary imports. `msvcp140.dll` is present only if a build needed it.

It also creates the Start Menu shortcut and the per-user uninstall entry.
Settings, workspace state, and chat sessions live in
`%APPDATA%\selora-ide\`. The API key is stored in Windows Credential Manager
(service `selora-ide`, entry `api-key`) when it is available; the app also
writes a best-effort plaintext copy to `%APPDATA%\selora-ide\.api_key`, so
the key is not keyring-only.

## Updates

Selora IDE checks GitHub Releases on launch and offers a newer version in a
centered prompt. Choosing **Yes, update** downloads the release's installer,
verifies it against the published SHA256, and runs it silently; the installer
upgrades the current install in place, so there is nothing to uninstall and
your settings, workspace, and sessions stay. **No, later** closes the prompt
until the next launch.

Turn the automatic check off under **Settings -> Theme -> Automatic Updates**.
The manual check in **Help -> About** keeps working either way.

## Uninstall

Settings -> Apps -> Selora IDE -> Uninstall. The uninstaller removes the
application directory, the Start Menu shortcut, the `.txt` Open-with entry,
and the per-user data folder (`%APPDATA%\selora-ide`: settings, workspace
state, sessions, and the plaintext `.api_key` fallback). It does not know the
keyring entry; use Disconnect in the app first, or remove the `selora-ide`
entry from Windows Credential Manager.

## From source

Requires Rust stable (MSVC toolchain) and Node.js 22 on Windows 10 (1809+)
or Windows 11.

```powershell
git clone https://github.com/ToxicAngelX/Selora-IDE
cd Selora-IDE
npm --prefix apps/selora-desktop/frontend ci
```

Development uses two terminals - Vite on :1420 and a debug shell:

```powershell
npm --prefix apps/selora-desktop/frontend run dev
cargo run -p selora-desktop
```

A release run embeds the built bundle, so build the frontend first:

```powershell
npm --prefix apps/selora-desktop/frontend run build
cargo build --release -p selora-desktop --locked
.\target\release\selora-desktop.exe
```

To build the installer yourself, install
[Inno Setup 6.3 or newer](https://jrsoftware.org/isdl.php) (the 6.x line, or
7.x) and the Visual Studio Build Tools C++ workload, then run
`installer\prebuild.bat` from the repo root:

```powershell
installer\prebuild.bat
```

The script reads the version from the root `Cargo.toml`, copies the MSVC
runtime DLLs beside `target\release\selora-desktop.exe`, downloads
Microsoft's WebView2 Evergreen bootstrapper when it is not already cached
(and verifies Microsoft's signature on it), and writes
`installer\output\Selora-IDE-Setup-<version>.exe`.

Running `installer\prebuild.bat --help` prints usage and writes nothing.
