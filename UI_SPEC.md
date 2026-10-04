# Selora IDE — UI Specification

This document defines the screen-by-screen layout of Selora IDE Lite. Every measurement comes from `DESIGN_TOKENS.md`. No screen in this document may deviate from those values. The build agent renders each surface against the rules here, with no "creative" interpretation.

All values are referenced to a 1280×800 default IDE window. Layouts must remain correct down to a 960×600 minimum.

---

## 1. Window chrome (outer shell)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ ⬤  Selora IDE   File   Edit   Selection   View   Run   Help      ─ □ ✕    │  ← 32px title bar
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │  ← workbench
│                                                                             │
│                                                                             │
├─────────────────────────────────────────────────────────────────────────────┤
│ main ●  0↑0↓  UTF-8  LF  Rust  ● selora connected · opus-5-5              │  ← 24px status bar
└─────────────────────────────────────────────────────────────────────────────┘
```

### 1.1 Title bar (32px tall)

- **Left:** brand mark (16px orbital planet) + 8px gap + "Selora IDE" in `--type-xl`, weight `--weight-semibold`, tracking `--tracking-tight`.
- **Center:** menu (File / Edit / Selection / View / Run / Help). Each menu is `--type-md`, `--fg-muted` by default, `--fg-default` on hover. Hover background is `--ink-faint`. No border, no shadow.
- **Right:** standard Windows window controls (minimize, maximize, close). We use the OS chrome; the IDE does not paint its own. On Windows, the OS draws the title bar background in dark mode automatically when the app's `DWMWA_USE_IMMERSIVE_DARK_MODE` is set. The build agent calls `DwmSetWindowAttribute` at window creation to request dark mode chrome.

### 1.2 Status bar (24px tall, 1px top hairline)

Three zones, left to right:

- **Left:** git branch name, then "●" + change count, then "↑" / "↓" for ahead/behind. Color: `--fg-muted` for branch, `--warn` for ahead count, `--fg-muted` for behind. A dirty state changes the dot to `--warn`.
- **Center:** editor metadata (encoding, line ending, language). Each is a clickable chip: `--type-xs`, `--fg-muted`, hover `--fg-default`.
- **Right:** Selora connection status — `● selora connected · <model>` (color `--ok`) or `○ selora disconnected · connect` (color `--fg-muted`, the word "connect" is `--fg-default` and clickable to open settings).

### 1.3 Window background

- Workbench background: `--ink-canvas` (`#000000`).
- 0px corner radius. The OS handles the window shape; the IDE does not draw a background behind the OS chrome.

---

## 2. Main workbench layout

The workbench is a 5-region grid:

```
┌────┬──────────────────────────────────────────────┬────────────┐
│    │                                              │            │
│ ①  │                  ② editor                    │     ③      │
│ Fi │                                              │   chat     │
│ le │                                              │  sidebar   │
│ Tr │                                              │  (toggle)  │
│ ee ├──────────────────────────────────────────────┤            │
│    │                  ④ panel                     │            │
│    │                  (git / problems)            │            │
└────┴──────────────────────────────────────────────┴────────────┘
```

| Region | Default width | Min | Max | Hide key | Default state |
|---|---|---|---|---|---|
| ① Activity + file tree | 240px | 160px | 480px | `Ctrl+B` | visible |
| ② Editor (central) | flex | 400px | — | — | always visible |
| ③ Chat sidebar | 360px | 240px | 600px | `Ctrl+L` | hidden |
| ④ Panel (bottom) | 200px tall | 0 | 50% of workbench height | `Ctrl+J` | hidden |

All four regions are drag-resizable. A 1px hairline border (`--ink-hi`) separates ① from ② and ② from ③. The ④ panel has a 1px top hairline.

### 2.1 Activity bar (the 48px wide leftmost strip)

A vertical strip with 4 icon buttons (16×16 each), centered vertically:

- `files` icon (default) — shows the file tree
- `search` icon — shows search-in-files
- `git` icon — shows the git panel
- `chat` icon — toggles the chat sidebar (3)

The selected activity has an `--ink-low` background pill behind the icon, 28×28, radius `--radius-1`. Hover: `--ink-faint`. No labels; the icons are well-known.

### 2.2 File tree (region ①, when "files" is active)

```
┌─ FILES ────────────────────┐
│ ▾ src                       │  ← folder, expanded (chevron-down)
│   ▸ editor                  │
│   ▸ lsp                     │
│   ▾ ui                      │
│     theme.rs                │  ← file
│     palette.rs              │
│   main.rs                   │
│ ▸ tests                     │
│ ▸ docs                      │
│   Cargo.toml                │
│   README.md                 │
│                              │
│                              │
│  + (new file)               │  ← bottom-right, --fg-faint, hover --fg-default
└──────────────────────────────┘
```

- 1px hairline right border, 1px hairline bottom border.
- Header bar: 24px tall, label "FILES" in `--type-xs`, `--fg-faint`, tracking `--tracking-wide`, 12px horizontal padding.
- Rows: 22px tall, 4px horizontal padding, 8px between icon and label.
- Disclosure chevron: `--fg-faint`, rotates 90° on open. 12px indent per level.
- Hover: row background `--ink-faint`, text `--fg-default`.
- Selected row: background `--ink-low`, text `--fg-default`, no left bar (we don't use the VS Code colored bar).
- New file `+` button: bottom-right of the tree, 24px tap target.
- A right-click context menu (New File, New Folder, Rename, Delete, Copy Path, Reveal in Explorer) appears at the cursor. Menu is `--ink-mid` background, 1px `--ink-hi` border, `--radius-2`.

---

## 3. Editor area (region ②)

### 3.1 Tabs

```
┌─ main.rs ──   buffer.rs ─   lsp/mod.rs ─   ──── ── +            ┐
│  theme.rs                                                          │  ← tab bar
└─────────────────────────────────────────────────────────────────────┘
```

- Tab bar: 32px tall, 1px bottom hairline. Background `--ink-canvas`.
- Each tab: 13px height + 8px vertical padding, label in `--type-md`. Padded 12px horizontal.
- Inactive tab: `--fg-muted`, background `--ink-canvas`. Hover: background `--ink-faint`, text `--fg-default`.
- Active tab: `--fg-default`, background `--ink-canvas`, 1px bottom border `--fg-default` (the only place a "light" line is used, to mark the active tab against the canvas).
- Dirty marker: 6px circle `--warn` 8px from the right edge of the tab, before the close button.
- Close button (×): only visible on hover or when dirty, `--fg-faint`, hover `--fg-default`.
- Tab overflow: tabs shrink; if even shrunken tabs don't fit, a horizontal scroll appears (no `…` button; we keep tabs readable).
- "+" at the right: opens a quick-open dropdown (recent files + fuzzy match against the workspace).

### 3.2 Editor canvas (the central work area)

```
┌─ 1 ──│  fn theme() -> Theme {                                 │
│  2 ──│      Theme::from_tokens(TOKENS)                        │
│  3 ──│  }                                                       │
└──────┴─────────────────────────────────────────────────────────┘
```

- Background `--ink-canvas`. No padding.
- **Gutter** (44px wide, right-aligned line numbers). 4px right padding. Line numbers in `--type-sm`, `--fg-faint`. Active line: `--fg-default`. The gutter also shows a 3px wide column for the active-line indicator (1px `--ink-rule` line at right of gutter, like a caret but for the line).
- **Text area** starts at x=44, runs to the right of the visible editor area. Line height = editor font size × 1.5. Character width = monospace width of the current font.
- **Selection:** background `--ink-mid` (one ink-step lift, no saturated blue). Text inside selection stays `--fg-default`.
- **Cursor:** 2px wide, `--fg-default`, blinks every 500ms (the only blink in the app; everything else is `--dur-loop` 1.2s).
- **Caret row:** background `--ink-faint` (so subtle it's almost invisible — the user notices it more by feel than by sight).
- **Whitespace (spaces, tabs):** shown as a single `--fg-faint` dot when `editor.render_whitespace` is on (default off).
- **Indentation guides:** 1px `--ink-rule` vertical line on the right edge of each indent column, only at indented lines.
- **Scrolling:** mouse wheel and trackpad. No smooth scroll (jump scroll is honest). `Home/End/PageUp/PageDown` for keyboard. `Ctrl+Home/End` to top/bottom of buffer.
- **Minimap:** not in Lite. (`editor.minimap` defaults to false; v1.x feature.)

### 3.3 Split editor (region ② divided)

`Ctrl+\` splits the active editor into two. Splits are nested; a split of a split is a 2×2 grid. Each split has its own tabs. The active split has a 1px `--ink-hi` border (subtle).

### 3.4 Sticky scroll

The first 3 lines of the enclosing function / class are pinned at the top of the editor as the user scrolls. Implemented in Phase 2.4 if the tree-sitter queries for the languages support it; otherwise a simpler "scroll to top of file shows the first 3 lines pinned" fallback.

### 3.5 Ghost text rendering

When `selora-ai` returns a ghost-text suggestion, the editor paints it at the caret in `--fg-faint` (and a 2px `--ink-rule` underline beneath) without affecting the buffer. Tab accepts (replaces the suggestion with a buffer insert). Any other keypress dismisses it. The ghost fades in with the `comet` animation (DESIGN_TOKENS §7.3) the first time it appears in a session.

### 3.6 Diagnostic squiggles

The editor reads `Vec<Diagnostic>` from the buffer's overlay layer and draws a 3px-tall underline at the first character of the diagnostic range. Color: `--diag-err` (red), `--diag-warn` (yellow), or `--diag-info` (gray). Hover on the squiggle shows a tooltip with the message.

---

## 4. Chat sidebar (region ③, when toggled on)

```
┌─ CHAT ── claude-opus-5-5 ▾ ────────────  ⋯ ┐
│                                              │
│  ┌─ Selora ─────────────────┐                │  ← assistant bubble
│  │ Sure — I can help you    │                │    background --ink-low
│  │ with the buffer API.     │                │    radius --radius-1
│  │ Here's the signature:    │                │
│  │                          │                │
│  │ ```rust                  │                │
│  │ pub fn insert(...)      │                │
│  │ ```                      │                │
│  │                          │                │
│  │ [Copy]                   │                │
│  └──────────────────────────┘                │
│                                              │
│  ┌─ You ────────────────────┐                │  ← user bubble
│  │ how do I open a buffer?  │                │    background --ink-mid
│  │                          │                │    right-aligned
│  └──────────────────────────┘                │
│                                              │
│  ┌─ Selora ─────────── ⠿⠿⠿  ─┐              │  ← streaming
│  │ The Buffer::open method   │              │
│  │ takes a path and...       │              │
│  │                           │              │
│  └───────────────────────────┘              │
│                                              │
│                                              │
├──────────────────────────────────────────────┤
│ Ask anything…  Enter to send, Shift+Enter   │  ← composer
│ for newline                              [⏎]│
└──────────────────────────────────────────────┘
```

- 1px hairline left border (`--ink-hi`).
- Header bar (40px tall):
  - Left: "CHAT" label in `--type-xs`, `--fg-faint`, tracking `--tracking-wide`.
  - Center: model picker (e.g. `claude-opus-5-5 ▾`). `--type-md`, `--fg-default`. Clicking opens the model picker dropdown. When the dropdown is closed, the chevron is `--fg-faint`.
  - Right: `⋯` overflow menu (clear conversation, copy transcript, switch model).
- Conversation area: 12px padding. Bubbles are full-width minus 16px on each side.
  - User bubble: background `--ink-mid`, radius `--radius-1`, 12px padding. Right-aligned (the bubble hugs the right, the chat area's right padding becomes the bubble's right edge).
  - Assistant bubble: background `--ink-low`, radius `--radius-1`, 12px padding. Left-aligned. No avatar.
  - Code blocks inside bubbles: background `--ink-canvas`, 1px `--ink-hi` border, radius `--radius-1`, 12px padding, monospace font, with a "Copy" button in the top-right of the block (only on hover).
- Streaming bubble: same as assistant bubble, but the text is replaced chunk-by-chunk as SSE deltas arrive. The `⠿⠿⠿` wave indicator appears only when the model is "thinking" between deltas (longer than 800ms with no chunk).
- Composer (60px tall): 12px padding all around. Single-line `egui::TextEdit` with placeholder "Ask anything…". Hint text below in `--type-xs`, `--fg-faint`. Send button is the `⏎` glyph right of the input. Background `--ink-faint`, hover `--ink-low`, `--radius-1`.

### 4.1 Model picker dropdown

```
┌─ MODEL ─────────────────────┐
│ claude-opus-5-5         ●   │  ← selected, ● = ok
│ claude-sonnet-5-5           │
│ gpt-5.1                     │
│ kimi-k3                     │
│ glm-5.3                     │
│                              │
│ ─── live from gateway ──── │  ← footer
│ 5 models · 142ms            │
└──────────────────────────────┘
```

- 320px wide, max 8 models visible (scroll after).
- Each row: 32px tall, model id in `--type-md`, family tag in `--type-xs --fg-faint` on the right.
- Selected: row background `--ink-low`, `--fg-default`. A `--ok` dot on the right.
- Footer: 24px tall, `--type-xs`, `--fg-faint`. Shows "live from gateway", model count, last-fetch latency.

---

## 5. Command palette (`Ctrl+Shift+P`)

```
            ┌────────────────────────────────────────────┐
            │ > Type a command…                          │
            ├────────────────────────────────────────────┤
            │  File: Open File…              ⏎ Enter     │  ← top result
            │  File: Open Folder…            ⏎ Enter     │
            │  File: Save                     Ctrl+S     │
            │  Edit: Toggle Comment           Ctrl+/     │
            │  View: Toggle Chat Sidebar      Ctrl+L     │
            │  View: Toggle File Tree         Ctrl+B     │
            │  Selora: Switch Model…                    │
            │  Selora: Clear API Key                    │
            │  ...                                       │  ← scroll
            └────────────────────────────────────────────┘
```

- Centered horizontally, top 25% of the workbench.
- 560px wide, max 320px tall (scrolls after 8 rows).
- Background `--ink-mid`, 1px `--ink-hi` border, `--radius-2`, opens with the `bloom` animation (DESIGN_TOKENS §7.3).
- Input: 12px top padding, full width minus 24px, `--type-lg`, placeholder "Type a command…". A `>` prefix glyph in `--fg-faint` to the left.
- Rows: 28px tall. Label in `--type-md`, keybind hint in `--type-xs --fg-faint` on the right.
- Selected row: `--ink-low` background.
- Esc closes; ↑/↓ moves selection; Enter runs.
- Fuzzy match uses `nucleo-matcher` over the command's name + tags. No regex.

### 5.1 Fuzzy file finder (`Ctrl+P`)

Same shape, different content. Results are files from the workspace. Header reads "GO TO FILE". Each result shows:

```
│  src/editor/mod.rs                  editor      │
│  src/editor/buffer.rs               editor      │
```

The right column is a category tag in `--type-xs --fg-faint`.

### 5.2 Global symbols (`Ctrl+Shift+O`)

Same shape, results are symbols from the current file (functions, types). Header reads "GO TO SYMBOL".

---

## 6. Settings dialog

A modal opened from the menu (File → Settings or `Ctrl+,`).

```
        ┌─ SETTINGS ─────────────────────────────────────┐
        │                                                  │
        │  ┌─ SIDEBAR ────────┐  ┌─ EDITOR ──────────────┐ │
        │  │ General          │  │ Font Family           │ │
        │  │ Editor      ●    │  │ [ JetBrains Mono    ]  │ │
        │  │ UI               │  │                       │ │
        │  │ Selora           │  │ Font Size             │ │
        │  │ LSP              │  │ [ 14                ]  │ │
        │  │ Git              │  │                       │ │
        │  │ Keybinds         │  │ Tab Size              │ │
        │  │                  │  │ [ 4                 ]  │ │
        │  │                  │  │                       │ │
        │  │                  │  │ ☑ Show line numbers   │ │
        │  │                  │  │ ☐ Word wrap           │ │
        │  │                  │  │ ☐ Render whitespace   │ │
        │  └──────────────────┘  └───────────────────────┘ │
        │                                                  │
        └──────────────────────────────────────────────────┘
```

- Centered modal, 720×520, `--ink-mid` background, 1px `--ink-hi` border, `--radius-2`.
- Two-column layout: 200px left sidebar of section labels (just text, no nested tree), flex right pane of fields.
- Each field: label on top in `--type-sm --fg-muted`, control below. Controls use the same inputs as elsewhere (no special "settings input" widget).
- The "Selora" section: shows current status (`● connected · opus-5-5` or `○ disconnected`), a "Connect with API key" button that opens a small inline key-paste input + "Save to keyring" button. The key input has `type="password"` masking.
- The "LSP" section: text fields for `rust_analyzer_path` and `typescript_server_path` with "Auto-detect" buttons next to each.
- Saving is implicit on field blur. No "Apply" / "Cancel" buttons. A toast confirms "Settings saved".
- `Esc` closes the modal.

---

## 7. First-run / splash

When the app launches with no Selora key, a full-window splash appears over an empty editor:

```
┌────────────────────────────────────────────────────────────────────────────┐
│ ⬤  Selora IDE   File   Edit   Selection   View   Run   Help      ─ □ ✕    │
├────────────────────────────────────────────────────────────────────────────┤
│                                                                            │
│                                                                            │
│                                                                            │
│                                  ⬤ ←                                        │  ← 64px orbital mark
│                                                                            │
│                              Selora IDE                                     │  ← --type-xxl
│                                                                            │
│                       The editor born connected.                            │  ← --type-md --fg-muted
│                                                                            │
│                                                                            │
│             ┌────────────────────────────────────────────┐                  │
│             │ Paste your sk-gw-… API key                 │                  │
│             │                                            │                  │
│             └────────────────────────────────────────────┘                  │
│             [ Connect ]            or    [ Skip for now ]                   │
│                                                                            │
│                                                                            │
│                                                                            │
└────────────────────────────────────────────────────────────────────────────┘
```

- Background `--ink-canvas`. The editor area is not visible (the splash is a `CentralPanel` cover, not a regular panel).
- The mark uses the `orbit` animation (DESIGN_TOKENS §7.3) — the satellite traces a slow ellipse around the core.
- A single password-masked text input (360px wide, `--ink-low` background, 1px `--ink-hi` border, `--radius-1`, 12px padding).
- Two buttons: "Connect" (primary, `--ink-mid` background, `--fg-default`) and "Skip for now" (text button, `--fg-muted`).
- "Connect" calls `SeloraClient::list_models` as a validation. On success: key is written to keyring, status flips to `Connected`, splash closes after a 1.2s delay with the `fade` animation.
- "Skip" closes the splash; the user can connect later from the status bar.

---

## 8. Git panel (region ④ when "git" activity is selected)

```
┌─ GIT ─ main ────────────────────── 2 changed ──┐
│                                                │
│  ▾ Changes (2)                                 │
│    M  src/editor/mod.rs                        │  ← modified
│    M  src/ui/theme.rs                          │
│                                                │
│  Commit message:                               │
│  ┌────────────────────────────────────────────┐│
│  │                                            ││
│  └────────────────────────────────────────────┘│
│                              [ Commit (Ctrl+Enter) ]  │  ← disabled if no message
│                                                │
│                                                │
└────────────────────────────────────────────────┘
```

- Panel height 200px by default, resizable.
- Header: "GIT" label, current branch, change count.
- Section "Changes" lists files with their status letter (`M`, `A`, `?`, `D`) in `--fg-muted` and the file path in `--fg-default`. Hover: row background `--ink-faint`. Click: opens the file diff in a temporary read-only editor (or selects it in the tree).
- Each row has a hover `+` (stage) and `−` (unstage) button on the right.
- Commit message: multi-line `egui::TextEdit`, `--type-md`, `--ink-low` background, 1px `--ink-hi` border, `--radius-1`. Min 2 lines, max 6.
- Commit button: right-aligned, 32px tall, `--ink-mid` background, `--fg-default`, `--radius-1`. Disabled (with `--fg-disabled` label) when the message is empty or no changes are staged.
- `Ctrl+Enter` in the message box commits.

---

## 9. Search in files (region ④ when "search" activity is selected)

```
┌─ SEARCH ───────────────────────────────────────┐
│ [ buffer  ×  ]   [ case  ]   [ word  ]   [ ⌄ ]  │  ← query bar
│                                                  │
│  src/editor/buffer.rs                            │
│    42:   pub fn insert(&mut self, pos: Position) │
│    88:   pub fn line_col(&self, pos: Position)   │
│                                                  │
│  src/ui/theme.rs                                 │
│    12:   pub const INK_CANVAS: Color32 = ...     │
│                                                  │
│  3 results · 2 files                             │
└──────────────────────────────────────────────────┘
```

- Query bar: 32px tall. Input is full-width minus 24px padding. Three toggles to the right: `case` (Aa), `word` (\\b), and a `⌄` to expand regex (regex is v1.x, so for Lite the `⌄` opens a "coming soon" toast and stays disabled).
- Result rows: file path on top in `--type-sm --fg-muted`, then a line in `--type-md --fg-default` with the matching substring highlighted in `--syn-string` background.
- Click a result: opens the file at that line.

---

## 10. Problems panel (region ④, "problems" sub-section)

```
┌─ PROBLEMS ─ 2 errors · 1 warning ──────────────┐
│  ⛔ src/editor/mod.rs:42  expected `,`, found `}`│
│  ⛔ src/editor/mod.rs:88  cannot find value `x` │
│  ⚠  src/ui/theme.rs:12    unused constant        │
└────────────────────────────────────────────────┘
```

- Auto-populated from LSP diagnostics.
- Click a row: jumps to the diagnostic in the editor.
- Tab at the top of the panel toggles between "Git" and "Problems".

---

## 11. About dialog

```
                ⬤  ← 48px mark
            Selora IDE
            Lite Edition · v0.1.0
            ─────────────────
            Open source · MIT

            github.com/ToxicAngelX/Selora-IDE

            [ Check for updates ]    [ Close ]
```

- 400×280 modal, `--ink-mid` background, 1px `--ink-hi` border, `--radius-2`.
- "Check for updates" calls the GitHub Releases API (see `RELEASE.md`); shows a toast with the result.

---

## 12. Empty states

When the editor is open with no folder:

```
┌─ editor ──────────────────────────────────────────────────────────────┐
│                                                                      │
│                                                                      │
│                          No folder open                               │
│                                                                      │
│                  Open a folder to start editing.                     │
│                                                                      │
│                  [ Open Folder ]   [ Clone Repo ]                    │
│                                                                      │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

Centered. `--type-lg` for "No folder open", `--type-md --fg-muted` for the hint, two `--ink-mid` buttons. Italic is used here per the rules in `DESIGN_TOKENS.md` §2.5 (placeholders).

When the editor is open with a folder but no file is selected:

```
┌─ editor ──────────────────────────────────────────────────────────────┐
│                                                                      │
│                                                                      │
│                  ⌘P to find a file, ⇧⌘P for commands                │
│                                                                      │
│                                                                      │
└──────────────────────────────────────────────┘
```

Single line of centered `--type-md --fg-muted` with keybind chips rendered in `--ink-mid` background, `--radius-1`, `--type-xs`. Italic placeholder per the rules.

---

## 13. Toasts

Bottom-right, above the status bar, 12px from each edge. Stacked vertically, 8px apart. Each toast: `--ink-mid` background, 1px `--ink-hi` border, `--radius-1`, 12px padding. Max 3 visible at once; older ones fade out.

Toasts auto-dismiss after 4s (info) or 8s (error). The user can dismiss with a click on the `×` on the right of each toast.

Severity icon: `i` (info, `--info`), `!` (warning, `--warn`), `!` (error, `--err`). All in 16×16 monochrome.

---

## 14. Drag and drop

- Drag a file from the OS into the editor area: opens the file. Drop a folder: opens the folder as workspace.
- Drag a file onto an editor split: opens the file in that split.
- Drag a tab to reorder or to move to another split.

Visual feedback during drag: 1px `--border-focus` outline on the target drop zone. No animation; instant.

---

## 15. Keyboard model

The full keybind table is in `settings.json` (see `ARCHITECTURE.md` §8.1). The defaults:

| Action | Key |
|---|---|
| Open file | `Ctrl+O` |
| Open folder | `Ctrl+K Ctrl+O` |
| Save | `Ctrl+S` |
| Save all | `Ctrl+Shift+S` |
| Close editor | `Ctrl+W` |
| Close all editors | `Ctrl+K Ctrl+W` |
| Command palette | `Ctrl+Shift+P` |
| Fuzzy find | `Ctrl+P` |
| Go to symbol | `Ctrl+Shift+O` |
| Go to definition | `F12` |
| Hover | `Ctrl+K Ctrl+I` |
| Toggle chat | `Ctrl+L` |
| Toggle file tree | `Ctrl+B` |
| Toggle panel | `Ctrl+J` |
| Split editor | `Ctrl+\` |
| Settings | `Ctrl+,` |
| Find in file | `Ctrl+F` |
| Replace in file | `Ctrl+H` |
| Accept ghost text | `Tab` |
| Dismiss ghost text | `Esc` |
| Commit (git panel focused) | `Ctrl+Enter` |

Keybinds are rebindable in Settings → Keybinds. Each row in that section is a label + two-button capture ("press a key"). Rebinds are stored in `settings.json` and applied live.

---

## 16. Responsive behavior

- Below 960px wide: the chat sidebar auto-hides (and the toggle button is still available, but the minimum workbench width is enforced at 720px — below that we show a single-pane "mobile isn't supported in Lite" notice, because we are not a mobile app).
- Below 600px tall: the file tree and chat sidebar both collapse to icons-only and the panel hides. (Practically, the IDE is unusable below this — the user should resize.)
- DPI: fonts and spacing scale by 1.0×, 1.25×, 1.5×, 2.0× based on the OS DPI setting. The `theme` module is unitless; every value is multiplied by the DPI scale at draw time.

---

## 17. Accessibility (Lite floor)

- Every interactive control has a keyboard equivalent (no mouse-only actions).
- Focus is visible (1px `--border-focus` on the focused control).
- Color is never the only signal (e.g. a modified file has a dot, not just an orange color).
- Reduced motion: respects `ui.reduce_motion` setting per `DESIGN_TOKENS.md` §7.4.
- No flashing faster than 3 Hz; no animation under 80ms.
- Screen reader: out of Lite scope (egui is not screen-reader friendly). The owner can document this honestly in the README.

---

## 18. The "no decoration" rule applied

Every screen in this document is built on the principle: **structure, not decoration**. The IDE is an instrument. Every pixel either communicates a piece of state or it is removed.

If a future contributor wants to add a gradient, a drop shadow, a bold heading, a saturated hover color, a sound effect, an emoji, an animated transition, or a decorative icon — the answer is no, and the answer comes from `DESIGN_TOKENS.md` §12.
