# Installation

## Installer (Windows)

1. Download `Selora-IDE-Setup-X.Y.Z.exe` from the latest
   [GitHub Release](https://github.com/ToxicAngelX/Selora-IDE/releases).
2. Run it. The install is per-user (`%LOCALAPPDATA%\Programs\Selora IDE`) —
   no admin prompt. The installer adds a Start Menu shortcut and an
   "Open with Selora IDE" entry for `.txt` files (your default `.txt` app is
   untouched).
3. Launch **Selora IDE** from the Start Menu.

!!! note "SmartScreen"
    Builds are not code-signed yet (signing is wired in CI but off until a
    certificate is provided — see `RELEASE.md` §3). Windows SmartScreen may
    show "Windows protected your PC" on first run: **More info → Run
    anyway**.

## Uninstall

Settings → Apps → Selora IDE → Uninstall. The uninstaller also removes the
per-user data folder (`%APPDATA%\selora-ide`: settings and workspace state).
Your OS keyring entry (the API key) is removed on Disconnect in the app's
settings; remove it manually from Windows Credential Manager if you skipped
that.

## From source

Requires Rust stable (MSVC toolchain) on Windows 10+:

```sh
git clone https://github.com/ToxicAngelX/Selora-IDE
cd Selora-IDE
cargo run -p selora-app --release
```

To build the installer yourself: install
[Inno Setup 6](https://jrsoftware.org/isdl.php), then
`installer\prebuild.bat` from the repo root.
