# Selora IDE

Selora IDE is a fast Windows code editor with AI assistance, workspace search,
Git tools, language intelligence, and a focused desktop workbench.

## Download

Download the newest Windows installer from:

<https://github.com/ToxicAngelX/Selora-IDE/releases/latest>

Run `selora-installer-v1.exe`. The installer is per-user and does not
need administrator access. It installs WebView2 when Windows does not have it.

## Start quickly

1. Open Selora IDE.
2. Choose **Open Folder**.
3. Select your project folder.
4. Open a file from **Files**.
5. Press `Ctrl+S` to save.
6. Press `Ctrl+L` to open the AI dock.

AI features need a provider key. Open **Settings** or **Connect**, choose a
provider, enter the gateway URL and key, then test the connection.

## Useful shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+Shift+P` | Command palette |
| `Ctrl+B` | Toggle file sidebar |
| `Ctrl+J` | Toggle bottom dock |
| `Ctrl+Shift+E` | Files |
| `Ctrl+Shift+F` | Search |
| `Ctrl+Shift+G` | Git |
| `Ctrl+Shift+M` | Problems |
| `Ctrl+,` | Settings |
| `Ctrl+L` | AI dock |
| `Ctrl+S` | Save file |
| `F12` | Go to definition |

Right-click files and folders for **New File**, **New Folder**, **Rename**, and
**Delete**.

## Build from source

Requirements:

- Windows 10 or 11
- WebView2 Runtime
- Rust stable with the MSVC toolchain
- Node.js 22

Install dependencies:

```powershell
npm --prefix apps/selora-desktop/frontend ci
```

Build frontend and desktop app:

```powershell
npm --prefix apps/selora-desktop/frontend run build
cargo build --release -p selora-desktop --locked
```

Run the built app:

```powershell
start.bat
```

Build the Windows installer:

```powershell
installer\prebuild.bat
```

The installer is written to `installer\output\`.

## Project layout

- `apps/selora-desktop/` ΓÇö Tauri desktop shell and frontend.
- `crates/` ΓÇö Rust core, editor, Git, AI, settings, and protocol modules.
- `assets/` ΓÇö application icons and bundled fonts.
- `installer/` ΓÇö Windows installer scripts.
- `.github/workflows/` ΓÇö CI and release automation.

## Privacy and credentials

Do not commit API keys or `.env` files. Selora stores provider credentials in
Windows Credential Manager when available.

## License

MIT. See [LICENSE](LICENSE).
