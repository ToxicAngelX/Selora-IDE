# Getting started

## First launch

With no Selora key stored, the app opens on the splash: paste your
`sk-gw-…` key and **Connect**, or **Skip for now** and connect later from
the status bar (`○ selora disconnected · connect`, bottom right).

Connect validates the key against the gateway and stores it in the Windows
Credential Manager — never in a file. On the next launch the key is
restored and re-validated automatically.

## Open a folder

`Ctrl+K Ctrl+O` (or the empty state's **Open Folder**). The file tree
populates on the left; git status decorations appear when the folder is a
repository.

## The essentials

- `Ctrl+P` — fuzzy file finder
- `Ctrl+Shift+P` — command palette
- `Ctrl+L` — chat sidebar
- `Ctrl+B` — file tree, `Ctrl+J` — bottom panel, `Ctrl+\` — split editor
- `Ctrl+,` — settings

## Pick a model

The chat header's model dropdown lists the live catalog from the gateway
(refresh button in the picker). Your choice persists.

## Check it works

Open a Rust file and wait for `rust-analyzer: ready` in the status bar,
then hover a type or press `F12` on a call. Open the chat (`Ctrl+L`) and
send "list 3 prime numbers" — the reply streams in, and the cost meter in
the status bar moves off zero.
