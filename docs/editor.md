# The editor

## Tabs and splits

Files open in tabs; middle-click or `Ctrl+W` closes. `Ctrl+\` splits the
active editor — splits nest (a split of a split is a 2×2 grid), and each
pane has its own tabs. The active pane carries a 1px border.

## Syntax highlighting

tree-sitter grammars: Rust, TypeScript, JavaScript, Python, JSON, TOML,
Markdown, HTML, CSS. Highlighting is incremental (only changed ranges
re-parse) and uses the Selora mono-tinted syntax palette — no saturated
colors.

## Language intelligence (LSP)

rust-analyzer (Rust) and typescript-language-server (TS/JS) are spawned as
child processes when a folder opens, if found on `PATH` (or configured in
Settings → LSP).

- Hover: hold the pointer, or `Ctrl+K Ctrl+I`
- Go to definition: `F12`
- Completion: `Ctrl+Space`
- Diagnostics: red squiggles in the buffer, plus the Problems section of
  the bottom panel

The status bar shows server state (e.g. `rust-analyzer: indexing 42%`).

## Git

The git activity (left bar) opens the Git panel: changed files with
status badges, stage/unstage per file, a commit message box
(`Ctrl+Enter` commits). The file tree decorates entries with M/A/?/D
badges. Push/pull/fetch are intentionally out of scope for the Lite
Edition.

## Search

`Ctrl+Shift+F` (or the search activity) searches the whole workspace —
via ripgrep when it is on `PATH`, otherwise a built-in scanner. Click a
result to open the file at that line.
