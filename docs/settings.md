# Settings

`Ctrl+,` opens the Settings dialog. Saving is implicit — every field
writes `%APPDATA%\selora-ide\settings.json` on blur; there are no
Apply/Cancel buttons. Esc closes (once to leave a focused field, twice to
close).

| Section | Contents |
|---|---|
| General | (placeholder) |
| Editor | font size, tab width, ghost text on/off |
| UI | reduce-motion, tree width, theme (`selora-dark` / `dark` / `space-black` / `light`), automatic update check (`ui.check_for_updates`, default on) |
| Selora | connection (Connect/Disconnect), base URL, active model |
| LSP | rust-analyzer / typescript-language-server paths (auto-detect) |
| Git | (placeholder) |
| Keybinds | rebinding for the rebindable four (save, palettes, chat) |

The API key is **not** a settings field: it lives in the Windows Credential
Manager only. The Brave Search API key (optional, for `web_search` in Agent
Mode) is the one exception — it is stored in `settings.json` (plaintext at
rest, by design until a keyring-backed field lands); leave it empty to use
DuckDuckGo instead.

## Where things live

- `%APPDATA%\selora-ide\settings.json` — settings
- `%APPDATA%\selora-ide\workspace.json` — per-workspace UI state (open
  tabs, chat visibility, last 20 chat turns, agent pill)
- Windows Credential Manager — the API key
