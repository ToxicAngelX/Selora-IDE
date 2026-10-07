# Troubleshooting

## The status bar never leaves "connecting…"

The 30-second ping is failing. Check that the base URL in Settings →
Selora is `https://api.selora.lol` (the default) and that your network
reaches it. A revoked key shows the gateway's own error words in the
status run.

## Chat says "no API key connected"

The key never persisted. Connect from Settings → Selora and watch for the
status flip to `● connected`. If Windows Credential Manager is unavailable
(rare; some Server SKUs), the app reports the keyring error as a toast —
the key cannot be stored, and the session key lasts until exit.

## Ghost text never appears

Ghost text needs a connected key, a supported model selected, and
`editor.ghost_text` on. It requests only after ~350ms of idle typing.

## rust-analyzer features are dead

The status bar shows the LSP state. If it never reaches `ready`, install
rust-analyzer (`rustup component add rust-analyzer`) or set its path in
Settings → LSP, then reopen the folder.

## The agent "did nothing"

Look at the cards: a denied card means the model was told "user denied"
and continued without the change. A cap toast (25 iterations / 10 minutes
/ $1) means the turn was aborted with its partial answer — raise the caps
in Settings (`agent.*`) if the task legitimately needs more.

## Installer fails / the app will not start from the installer

Install logs: rerun the setup exe from a terminal. The app writes no
crash log of its own; start it from `cmd` (`selora-app.exe`) to see a
panic message, and file an issue with the output.
