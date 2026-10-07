# Selora AI

The IDE is a first-class client of the Selora gateway
(`api.selora.lol`): one key unlocks every model the gateway fronts.

## Connecting

Paste your `sk-gw-…` key on the first-run splash or in Settings → Selora
→ **Connect**. The key is validated (`GET /v1/models`) and stored in the
Windows Credential Manager — never in a file, never in settings.json.

The status bar's right side shows the live state: `● selora connected ·
<model>` or `○ selora disconnected · connect`. A 30-second ping keeps it
honest.

## Chat

`Ctrl+L` toggles the chat sidebar. Enter sends, Shift+Enter inserts a
newline. Replies stream token-by-token; code blocks render in JetBrains
Mono with a hover **Copy** button. The last 20 turns persist across
restarts (per workspace).

The header holds the model picker (search, refresh) and the ⋯ menu
(clear conversation, copy transcript).

## Ghost text

Stop typing for ~350ms and the editor requests an inline completion,
shown in faint text with a hairline underline. `Tab` accepts; any other
key dismisses. Set `editor.ghost_text` off in Settings → Editor to
disable.

## Cost meter

The status bar shows `Σ today $X.XX` — the running total of every
receipt the gateway returned today. Hover for the per-model breakdown.
The meter resets at midnight.

## Agent Mode

See [Agent Mode](agent-mode.md).
