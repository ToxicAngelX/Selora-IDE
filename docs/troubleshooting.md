# Troubleshooting

## The app exits immediately or shows no window

Start it from `cmd` to check the exit code and timing:

```bat
"%LOCALAPPDATA%\Programs\Selora IDE\selora-desktop.exe"
```

The shell also writes `%TEMP%\selora_last_panic.log` on a panic and
`%TEMP%\selora_boot.log` during startup. Include both in a bug report.

## "VCRUNTIME140.dll was not found" (or VCRUNTIME140_1.dll)

The release binary links the dynamic MSVC runtime. The installer ships
`vcruntime140.dll` and `vcruntime140_1.dll` beside `selora-desktop.exe`, so:

1. Confirm `%LOCALAPPDATA%\Programs\Selora IDE\` contains both DLLs.
2. Reinstall with the current installer if they are missing.
3. If you copied the exe somewhere else by hand, copy the DLLs too, or
   install the
   [Microsoft Visual C++ 2015-2022 x64 Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe).

## Blank window, or "WebView2 runtime not found"

Selora IDE renders through Microsoft Edge WebView2. The installer detects an
existing runtime and runs the bundled Evergreen bootstrapper only when none
is registered. If that step failed (offline install or a locked-down
network), install the runtime manually from
[Microsoft's WebView2 page](https://developer.microsoft.com/microsoft-edge/webview2/),
then start Selora IDE again.

To check by hand, the runtime registers itself under Microsoft EdgeUpdate
with the WebView2 client GUID `{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`; a
`pv` value should exist in one of these keys and contain a version:

```text
HKCU\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}\pv
HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}\pv
HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}\pv
```

```powershell
Get-ItemProperty 'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -Name pv
```

## The .txt "Open with" entry launches the app but not the file

The installer adds Selora IDE to the `.txt` **Open with** list, but the
current shell does not read the `%1` path argument. Open the containing
folder with **File -> Open Folder**, then open the file from the explorer.
Your existing default `.txt` app is untouched.

## Installer fails, or SmartScreen blocks it

- Rerun the setup exe from a terminal with a log:

  ```bat
  Selora-IDE-Setup-X.Y.Z.exe /LOG="%TEMP%\selora-install.log"
  ```

  The log names the failing file or action. A per-user install should not
  request elevation.
- Verify the download against the attached `.sha256` before treating an
  error as a bug:

  ```powershell
  Get-FileHash .\Selora-IDE-Setup-X.Y.Z.exe -Algorithm SHA256
  ```

- SmartScreen: **More info -> Run anyway**. Builds are unsigned until the
  owner adds the signing certificate (see `RELEASE.md` section 3).

## The status bar never leaves "connecting…"

The gateway connection has not been established. Open Connect (click the
connection indicator in the status bar) or **Settings -> AI Gateway**, confirm
the base URL is `https://api.selora.lol` (the default), and check that your
network reaches it. A revoked key shows the gateway's own error words in the
status run.

## Chat says "no API key connected"

The key never persisted. Connect from the Connect dialog (click the connection
indicator in the status bar, or use **Settings -> AI Gateway -> Configure API
Key**) and watch the status flip to connected. The key is written to Windows
Credential Manager and, best-effort, to the plaintext fallback
`%APPDATA%\selora-ide\.api_key`; if both writes fail, the session key lasts
until exit.

## Ghost text never appears

Ghost text needs a connected key, a supported model selected, and
`editor.ghost_text` on. It requests only after ~350ms of idle typing.

## rust-analyzer features are dead

The status bar shows the LSP state. If it never reaches `ready`, put
rust-analyzer on `PATH` (`rustup component add rust-analyzer`, then confirm
`rust-analyzer --version` works in the shell that launched the app) and reopen
the folder. This build has no Settings LSP path override: resolution is
PATH-only. TypeScript features likewise need `typescript-language-server` on
`PATH`.

## The agent "did nothing"

Look at the cards: a denied card means the model was told "user denied"
and continued without the change. A cap toast (25 iterations / 10 minutes
/ $1) means the turn was aborted with its partial answer - raise the caps
in Settings (`agent.*`) if the task legitimately needs more.
