# Phase Z2 — Premium shell layout & token spec

Status: build-ready for the frontend developer. Audience: the Phase Z2 shell
implementer. Basis: every value below is traceable to `DESIGN_TOKENS.md`,
`crates/selora-tokens/src/lib.rs`, `UI_SPEC.md`, `docs/ui/REVAMP_SPEC.md`,
`PHASE_Z_TAURI_PROMPT.md`, or the co-normative implementation contract
`docs/ui/PHASE_Z2_ARCHITECTURE.md` (cited as ARCH from here on) — cited
inline. New web-only constants are marked **(web addition)** and collected
in §11 per PHASE_Z §6 ("document any new spacing, radius, or motion values
before using them"). This revision reconciles the two contracts wherever
they had drifted (region defaults, keyboard steps, the grid diagram,
empty-state copy, the bottom-dock tabs, the replay gate's initial state) —
each reconciliation is flagged where it lands. A CTO follow-up then
corrected two of those resolutions to the product-spec side — the region
defaults (§2) and the problems placement (§3.6) — because the revision had
chased the pre-review ARCH's Z1 stub values; both carry inline withdrawal
notes where they land.

Ships with this spec:

- `apps/selora-desktop/frontend/src/styles/tokens.css` — custom properties only.
- `apps/selora-desktop/frontend/src/styles/base.css` — reset, fonts, focus
  ring, scrollbars, reduced-motion behavior.
- `apps/selora-desktop/frontend/public/fonts/` — the five bundled TTFs plus
  their OFL license files, copied from `assets/fonts/`.

This spec replaces the Z1 diagnostic shell styling (`frontend/src/styles.css`,
a one-file stopgap with hardcoded hex values and a 48px/32px header/footer
that do not match the chrome anatomy; ARCH §8.3 has it deleted in Z2).
Component CSS is the frontend developer's; everything here is the contract
it must read from.

---

## 1. The seven-region grid

```text
┌──────────────────────────────────────────────────────────────────────────┐
│ 1 · TITLE BAR — 32px — mark · Selora IDE · menus · window controls       │
├──┬───────────┬───────────────────────────────────────────┬──────────────┤
│  │           │                                           │              │
│ 2│ 3         │ 4 · EDITOR REGION — tab bar 32px + canvas │ 5 · AGENT    │
│  │ PRIMARY   │                                           │   DOCK       │
│  │ SIDEBAR   ├───────────────────────────────────────────┤              │
│  │           │ 6 · BOTTOM DOCK — problems/output/term    │              │
├──┴───────────┴───────────────────────────────────────────┴──────────────┤
│ 7 · STATUS BAR — 24px — workspace · editor metadata · selora status     │
└──────────────────────────────────────────────────────────────────────────┘
```

Region order follows PHASE_Z §6's list. Geometry follows UI_SPEC.md §2
(region ④ spans column ② only): the bottom dock sits under the **editor
column only** — the primary sidebar and the agent dock run the full height
of the workbench row. That keeps the agent dock a first-class, full-height
surface (PHASE_Z §2: "not an afterthought sidebar") rather than a pane
clipped by the panel. ARCH §1's diagram was redrawn in this revision to
match this geometry exactly (it previously showed the bottom dock spanning
sidebar + editor; UI_SPEC.md:48-58's ASCII is the authority both docs now
follow).

Reference structure (the frontend developer may implement it as nested
flex/grid; the track logic is the contract). Region sizes are **not CSS
tokens** — they arrive from Rust (§2) and are written as inline custom
properties on the grid container:

```css
.shell {
  display: grid;
  grid-template-rows: var(--chrome-titlebar-h) 1fr var(--chrome-statusbar-h);
}
.workbench {                       /* middle row */
  display: grid;
  grid-template-columns:
    var(--chrome-activitybar-w)    /* 2 · activity bar — fixed, token */
    var(--sidebar-w, 0px)          /* 3 · primary sidebar, 0 when collapsed */
    var(--hit-zone, 0px)           /* sidebar resize handle */
    1fr                            /* 4+6 · center column */
    var(--hit-zone, 0px)           /* agent-dock resize handle */
    var(--agent-w, 0px);           /* 5 · agent dock, 0 when collapsed */
}
.center {                          /* inside track 4 */
  display: grid;
  grid-template-rows:
    1fr                            /* 4 · editor region */
    var(--hit-zone, 0px)           /* bottom-dock resize handle */
    var(--bottom-h, 0px);          /* 6 · bottom dock, 0 when collapsed */
}
```

`--sidebar-w` / `--agent-w` / `--bottom-h` / `--hit-zone` are set via
`el.style.setProperty` from React state — bootstrapped from
`bootstrap.layout` (a `LayoutDto`) and driven by the
`workspace_layout_changed` event, with the drag preview writing the same
properties locally (ARCH §3.1, §6.2). Their fallbacks collapse the tracks
before the first paint after bootstrap. The fallback `0px` is structural
(collapsed), not a magic size — every real number comes from Rust.

## 2. Region sizes, constraints, collapsed states

| # | Region | Default | Min (open) | Max | Collapsed | Toggle |
|---|---|---|---|---|---|---|
| 1 | Title bar | 32px fixed | 32px | 32px | never | — |
| 2 | Activity bar | 48px fixed | 48px | 48px | never | — |
| 3 | Primary sidebar | 240px | 160px | 480px | 0 (activity bar remains) | `Ctrl+B` |
| 4 | Editor region | flex (1fr) | 400px wide | — | never | — |
| 5 | Agent dock | 360px, **hidden by default** | 240px | 600px | 0 (**default state**) | `Ctrl+L` |
| 6 | Bottom dock | 200px tall | 120px | min(800px, 50% of workbench) | 0 (**default state**) | `Ctrl+J` |
| 7 | Status bar | 24px fixed | 24px | 24px | never | — |

**Single source of truth — Rust.** These numbers live in the protocol
constants and the `LayoutDto` (ARCH §1.1). The defaults are 240 / 360 /
200 with the agent dock and bottom dock **closed** first-run — UI_SPEC
§2's region table (① visible, ③ hidden, ④ hidden), the egui settings
defaults (`crates/selora-settings/src/settings.rs:58-64`: 240/360), and
the egui persistence default (`crates/selora-app/src/persistence.rs:49-56`,
`chat_open` false) all agree. The Z1 stub values in
`state.rs:68-76` (280/220, agent open) and the protocol test fixture
`lib.rs:800-808` are diagnostic placeholders — ARCH §1.1 has Z2 change
them to this table's values. `tokens.css` deliberately carries **no**
`--region-*` values (ARCH §1.1/§10/§11.6 record the same decision): the
frontend receives bounds in `bootstrap.layout_constraints` and sizes in
`bootstrap.layout` (ARCH §5.1 — "no bound is ever a JS literal"), so a
CSS duplicate could drift and violate both docs' no-magic-numbers rule.

The first revision of this spec recorded the divergence (280 / open /
220) as "intentional" — that note is withdrawn: those were Z1 scaffold
numbers rationalized after the fact, not a product decision. ARCH §11.6
now records the unification at UI_SPEC §2's values, and this spec follows
them. Chrome heights (32/48/24) are token owned (UI_SPEC §1.1/§1.2/§2.1);
the open-state defaults (`primary_sidebar_open: true`, agent and bottom
docks closed) are Rust owned (ARCH §1.1). The **bottom-dock open minimum
of 120px** is the Z2 decision
recorded in ARCH §1.1 (UI_SPEC §2's "min 0" meant hidden, which the
protocol models as `bottom_dock_open: false`); the max is ARCH §1.1's
split — Rust enforces the static 800px ceiling, the frontend additionally
clamps to `0.5 × measured workbench height` before dispatching.

Window: 1440×900 default, **960×640 minimum**, enforced by the OS
(`apps/selora-desktop/src-tauri/tauri.conf.json:18-21`). Note the source
discrepancy: UI_SPEC.md's preamble says "960×600 minimum" — the Tauri
config and this phase's brief use 960×640; the window manager value wins
for the web shell.

### 2.1 Minimum-window budget (the reason for every minimum)

At 960×640 with every region open at its minimum:

- Vertical: 32 title + 24 status = 56px chrome; workbench row = 584px.
  Bottom dock at min 120 + one 6px handle leaves the editor ≥ 458px tall.
- Horizontal: 48 + 160 + 400 + 240 = 848px + two 6px handle tracks = 
  **860px ≤ 960px**. The editor flexes to absorb the 100px slack. (ARCH
  §1.1's own sum now includes the handle tracks too — its earlier 848
  arithmetic omitted them.)

The ≥6px hit-zone rule is PHASE_Z §6, enforced as `RESIZE_HIT_ZONE_MIN_PX`
in Rust (delivered via `layout_constraints.resize_hit_zone_min_px`) — never
a CSS literal. Handles are tracks, not overlays: at 960px wide both handles
fit without pushing any region below its floor.

When the window shrinks and the persisted sizes no longer fit, clamp in
this order: **agent dock first, then primary sidebar, then bottom dock**;
the editor region never goes below 400px. At the 960px floor everything
still fits, so no region ever force-collapses on resize. (UI_SPEC §16's
below-960px auto-hide rules are unreachable in the Tauri shell because the
window cannot get that small — recorded so nobody re-implements them.)

### 2.2 Collapsed states and persistence

- Collapse is instant (no width/height animation — jump is honest, the
  same reading as UI_SPEC §3.2's no-smooth-scroll). Content may
  cross-fade at `--dur-quick` / `--ease-fade`; under reduced motion it
  snaps (§8).
- A collapsed region is always recoverable three ways: keyboard toggle,
  command-palette entry, and a visible control (activity-bar icon for
  sidebar and agent dock; the bottom dock's palette/menu entry) —
  PHASE_Z §6 ("an accessible toggle and a command-palette command").
- **Persistence is global in Z2, not per workspace** — the owner-authorized
  deviation recorded in ARCH §4.1/§11.1: `workspace.json` records the
  *last* workspace rather than keying by workspace, so "per workspace"
  cannot be honestly delivered before the workspace-picker slice; the
  per-workspace keying strategy is pinned in ARCH §4.5. Layout lives in
  the `layout` section of `settings.json` (ARCH §4.2) and is written
  through `ResizeRegion` / `ToggleSurface` / `SaveLayout` protocol
commands (PHASE_Z §3.2, §6; cadence in ARCH §3.1/§3.3). Defaults come
from the Rust `LayoutDto` — there is no second default set in CSS (§11).
- Keyboard-step and drag-end sizes persist via the same
  `ResizeRegion` → `await` → `SaveLayout` ordering ARCH §3.1 mandates.

## 3. Region-by-region chrome

### 1 · Title bar — 32px, custom (PHASE_Z §2)

- Background `--ink-canvas`; no bottom border — the workbench's panels
  carry their own hairlines (UI_SPEC §1.3 keeps the canvas flat). The
  window drag region covers the bar except on interactive controls
  (`data-tauri-drag-region`, ARCH §6.3).
- Left: the Selora orbital mark at 16px, 8px (`--space-2`) from the
  window edge (DESIGN_TOKENS §9), drawn as inline SVG — never a raster
  (§9 "render the mark in code"). Then 8px gap, then "Selora IDE" in
  `--type-xl`, `--weight-semibold`, `--tracking-tight` (UI_SPEC §1.1).
- Center-left: menus **File / Edit / View / Help** (PHASE_Z §7's app
  chrome list). Menu buttons: `--type-md`, `--fg-muted`, hover
  `--fg-default` on `--ink-faint`, open `--ink-low`, `--radius-1`, no
  borders (the landed `menu_button_matrix`, REVAMP_SPEC §7). In Z2 only
  View opens a working dropdown (the palette commands); File/Edit/Help
  show one disabled row "Available in a later slice" — no fake menu
  items (ARCH §7.3).
- Right: window controls (minimize, maximize/restore, close) as three
  44×32 buttons — 44 keeps the 4px grid. Glyphs are the §10 icon system
  plus three additions (`win-min`, `win-max`, `win-restore`; `close`
  exists). Idle `--fg-muted`, hover `--fg-default` on `--ink-faint`.
  **No red close-hover** — §12 forbids saturated hover color; the close
  button gets the same treatment as its neighbors. This is a deliberate
  deviation from the Windows convention, flagged here per the senior-
  designer duty rather than silently bent (§14).
- Requires `decorations: false` in both `tauri.conf.json` and the
  `main.rs` window builder, plus the six `core:window:*` capability
  additions — the exact Tauri-side changes are ARCH §6.3's contract
  (the Z1 window currently uses OS chrome).

### 2 · Activity bar — 48px, full workbench height

- Background `--ink-canvas`; 1px right hairline `--ink-hi` separating it
  from the sidebar (or from the editor when the sidebar is collapsed).
- Buttons: 28×28 (`--chrome-activity-pill`) hit areas, horizontally
  centered (10px inset each side), stacked from the top with `--space-2`
  gaps, `--space-2` top padding. Icons are 16×16 (§10).
- Order: `files`, `search`, `git-branch`, `error` (problems), `chat`
  (agent-dock toggle). `settings` is pinned to the bottom with
  `--space-2` bottom padding. (UI_SPEC §2.1's four-icon strip, extended
  by PHASE_Z §7's app-chrome list.) The `error` button dispatches
  `ToggleSurface{surface:"problems"}` and opens the **bottom dock's**
  problems tab (§3.6, ARCH §2.2).
- States (the landed matrix, REVAMP_SPEC §7): idle transparent /
  `--fg-muted`; hover `--ink-faint` / `--fg-default`; selected
  `--ink-low` pill, `--radius-1`, plus the 2×16 leading edge mark in
  `--fg-muted` painted at `x = --space-1` (4px from the bar's left
  edge), vertically centered on the pill, `--radius-1`. The mark is a
  painted rect, not a border — REVAMP_SPEC §11 bend 3.
- Badges (PHASE_Z §7 "activity badges"): a numeric pill on the problems
  icon (≥1), a presence dot on the git icon when the tree is dirty.
  Badge: min 16px wide, `--radius-pill`, `--type-xs`, background
  `--ink-mid` / `--fg-muted`; errors > 0 may use `--err` background
  with `--ink-canvas` text — small-badge saturated color is the §1.4
  sanctioned use. No other badges. (Both badge slots are inert until
  their data slices — Z4.)

### 3 · Primary sidebar — resizable (160 / 240 / 480)

- Background `--ink-low` (DESIGN_TOKENS §1.1: sidebar = ink-low); 1px
  right hairline `--ink-hi` (UI_SPEC §2.2).
- Header: 24px (`--chrome-panel-header-h`), the active view's label in
  `--type-xs`, `--fg-faint`, `--tracking-wide`, uppercase ("FILES",
  "SEARCH", "GIT", "SETTINGS"), `--space-4` horizontal
  padding (UI_SPEC §2.2).
- Content padding: `--space-3` vertical, `--space-4` horizontal
  (DESIGN_TOKENS §3.1).
- Row language (file tree as reference, UI_SPEC §2.2): 22px rows, hover
  `--ink-faint`, selected `--ink-low` + 2×14 edge mark at the row's left
  edge (REVAMP_SPEC §7), no row borders (§5.2).
- The sidebar's activity set is **files / search / git / settings**
  (ARCH §2.2/§4.3: `ACTIVITY_VIEWS`). Problems is a **bottom-dock** view
  (§3.6, ARCH §2.2 — UI_SPEC §2's panel ④ and PHASE_Z §7's bottom-dock
  list place it there, superseding brief §2's sidebar parenthetical; the
  earlier revision's sidebar-only resolution followed the pre-review ARCH
  and is withdrawn). The Z2 view states are the §6 table's Z2 rows.

### 4 · Editor region — flex, min 400px

- Background `--ink-canvas`; no padding on the canvas (DESIGN_TOKENS
  §3.1: text touches the gutter).
- Tab bar: 32px (`--chrome-tabbar-h`), 1px bottom hairline `--ink-hi`,
  background `--ink-canvas` (UI_SPEC §3.1). Active tab: `--fg-default`
  with the 1px `--fg-default` bottom line; inactive `--fg-muted`, hover
  `--ink-faint`. Dirty marker: 6px `--warn` dot. No tab borders (§5.2).
  (Tabs, buffers, and splits are Z4 surfaces — ARCH §1 non-goals; the
  chrome heights are reserved so Z4 changes nothing here.)
- Split editors: active split carries a 1px `--ink-hi` outline
  (UI_SPEC §3.3).
- Selection `--ink-mid`; caret 2px `--fg-default`. UI_SPEC §3.2 pins a
  500ms caret blink — the one blink exception, written before
  DESIGN_TOKENS §12's 1200ms loop floor and grandfathered by this note.
  CodeMirror is not installed in Z2 (ARCH §1 non-goals), so nothing
  ships now; Z4 must set the editor's blink rate explicitly to the
  chosen value rather than rely on CodeMirror 6's own default, and
  should decide whether to keep UI_SPEC's 500ms or adopt the token
  floor's 1200ms — flagged in §14.
- Empty states per §6.

### 5 · Agent dock — resizable (240 / 360 / 600), **hidden by default**

- Background `--ink-low`; 1px left hairline `--ink-hi` (UI_SPEC §4).
- Header: 40px (`--chrome-agent-header-h`), bottom hairline `--ink-hi`
  (REVAMP_SPEC §3). Left: the 20px presence slot at `--space-3`; in Z2
  it holds a static 8px `--fg-muted` dot (a placeholder presence mark —
  `ThinkingOrb` and the `BotAvatar` mount in Z3 per ARCH §1 non-goals;
  **there is no orb CSS in base.css** by design). Then the session
  switcher. Right: overflow menu.
- Transcript: `--space-3` padding; user bubbles `--ink-mid`, assistant
  bubbles `--ink-low` with a 1px `--ink-hi` hairline (the card needs the
  hairline to read against the dock's ink-low; the egui original relied
  on a different panel ink step — this is the honest translation of §6
  "raise one ink step"), `--radius-1`. (Transcript and bubbles are Z3.)
- Composer: the hero surface, docked at the bottom with `--space-3`
  margins on all sides (REVAMP_SPEC §2.1). Card: `--ink-mid` fill so
  the card lifts one ink step off the dock, 1px `--ink-hi` border
  (rest) → `--ink-rule` (hover/focus), `--radius-2`, inner padding
`--space-3`. Placeholder "Ask anything…". Enter submits, Shift+Enter
newline, ↑/↓ history (PHASE_Z §2 — the contract is sacred). Z2 ships the
card's **visual** inert — placeholder, disabled controls, no dispatch
(ARCH §1 non-goals, §8.1); Z3 wires the behavior. The border-beam
mount point is **reserved**: the beam rides this card's perimeter
while a request is in flight (REVAMP_SPEC §8.6), one surface only;
Z2 ships the card geometry, Z3 mounts the effect with `--dur-lap`.
- Empty state per §6 (Z2 row).

### 6 · Bottom dock — resizable (120 / 200 / min(800, 50%)), hidden by default

- Spans the editor column only (§1). 1px top hairline `--ink-hi`
  (UI_SPEC §2).
- Tab strip: 32px, left-aligned tabs **PROBLEMS / OUTPUT / TERMINAL** in
  `--type-xs`, `--tracking-wide`, uppercase; idle `--fg-faint`, hover
  `--fg-muted`, active `--fg-default` with the 1px `--fg-default`
  bottom line (the §3.1 active-tab language). Right side of the strip:
  close (×, `close` icon, `--fg-faint`, hover `--fg-default`).
  **Problems is a bottom-dock view in Z2** (ARCH §2.2/§4.3:
  `BOTTOM_VIEWS = [Output, TerminalPlaceholder, Problems]`;
  `ACTIVITY_VIEWS` drops it) — UI_SPEC §2's panel ④ and PHASE_Z §7's
  bottom-dock list place it here, superseding brief §2's sidebar
  parenthetical, and §14's open question is resolved to this one surface.
  The activity bar's `error` icon (§3.2) and `Ctrl+Shift+M` both dispatch
  `ToggleSurface{surface:"problems"}` to this tab (ARCH §7.1).
- Content background `--ink-canvas`; rows follow the sidebar row
  language (22px, hover `--ink-faint`).
- TERMINAL is the clearly-labeled placeholder PHASE_Z §7 requires — see
  §6 for its copy.

### 7 · Status bar — 24px, 1px top hairline `--ink-hi`

- Background `--ink-low`; `--type-xs`; `--space-2` vertical /
  `--space-4` horizontal padding (DESIGN_TOKENS §3.1).
- **Z2 zones** (ARCH §8.1): left = workspace display name
  (`--fg-muted`, `--type-xs`); center = empty; right = the UI_SPEC §1.2
  disconnected form "○ selora disconnected · connect" with the word
  *connect* **disabled** (settings editing is a later slice), or the
  connected form when `bootstrap.connection` says so. The ●/○ glyphs
  are text glyphs in the mono family — the UI_SPEC §1.2 precedent, not
  emoji. Numeric segments use `--font-mono` (§2.1: status bar
  numerics).
- Full-product zones (later slices, ARCH §8.1 names only the Z2 set):
  left = git branch + change count + ahead/behind (UI_SPEC §1.2);
  center = encoding, line ending, language; right = connection status
  with *connect* clickable. Clickable segments get the landed hover
  target: bounds + `--space-1` padding, `--radius-1`, `--ink-faint`
  fill, `--fg-default` text (REVAMP_SPEC §7).

## 4. Resize handles

- **Hit zone: ≥6px**, centered on the region boundary — 3px of it
  overlaps each neighbor's space. The exact width is
  `layout_constraints.resize_hit_zone_min_px` from Rust (PHASE_Z §6;
  never a CSS literal), applied as the `--hit-zone` grid track (§1).
  The zone is transparent at rest; the adjacent region's own 1px
  `--ink-hi` hairline remains the visual boundary, so rest state shows
  exactly one hairline, never a 6px gutter.
- **Hover:** a 1px line at the boundary center in `--ink-rule`
  (one ink step up — a preview, not yet the focus line).
- **Active drag or keyboard focus:** the 1px line becomes
  `--fg-default` — the `--border-focus` inverse-line language
  (DESIGN_TOKENS §5). This lit line *is* the handle's focus indicator;
  do not add an outline around a 6px strip. (ARCH §3.1 states the
  same two-step rule — both contracts were aligned in this revision;
  it previously had hover painting the focus line.)
- Cursor: `col-resize` on vertical boundaries, `row-resize` on the
  bottom dock's.
- **Keyboard:** every handle is a focusable
  `<div role="separator" aria-orientation="vertical|horizontal"
  aria-label="Resize sidebar|agent dock|panel" aria-valuenow={px}
  aria-valuemin={min} aria-valuemax={max}>`; Arrow keys step 4px
  (`--space-1`), Shift+Arrow steps 16px (`--space-4`), Home/End jump to
  the region min/max, and **double-click resets to the Rust layout
  default** (the `LayoutDto` value delivered in `bootstrap.layout`).
  ARIA min/max come from `bootstrap.layout_constraints` — never
  literals. (ARCH §3.1 was aligned to this exact set in this revision;
  it previously specified ±16/±64px steps, and ±64 is backed by no
  token. Both contracts now say 4/16, and both carry the Home/End and
  double-click behavior.)
- Drag cadence is ARCH §3.1's contract: `setPointerCapture` on
  `pointerdown`, **zero IPC during a drag** (local custom-property
  writes, client-side clamp), then exactly one `ResizeRegion` on
  `pointerup`, `await`ed, followed by `SaveLayout`. A keyboard step is
  a completed resize — the same pair. During drag, disable text
  selection (`user-select: none` on the shell) and snap the final size
  to whole CSS pixels.
- Max constraints respect §2: bottom dock never above the Rust ceiling
  and 50% of the workbench row; sidebar/agent never push the editor
  below 400px.

## 5. (collapsed states live in §2.2)

## 6. Empty states — calm copy (UI_SPEC §12)

Tone rules, binding on every empty/placeholder surface:

1. State what is true, then offer the one next action. No cheerleading,
   no exclamation marks, no blame, no emoji, no internal phase codenames
   (Z-numbers).
2. Heading `--type-lg` `--fg-default` (the agent welcome hero may use
   `--type-xxl` `--weight-semibold`, REVAMP_SPEC §4); hint `--type-md`
   `--fg-muted`, italic where it is a placeholder (§2.5 sanctions italic
   for empty-state placeholders only).
3. At most two actions. Buttons: `--ink-mid` fill, `--fg-default`,
   `--radius-1`, 32px tall (UI_SPEC §12).
4. Keybind hints render as chips: `--ink-mid`, `--radius-1`,
   `--type-xs`, mono family for the key names, `--space-1`/`--space-2`
   padding (UI_SPEC §12).

The Phase column marks when each row is live: Z2 rows ship now (their
controls may be disabled where ARCH §1's non-goals say the slice hasn't
landed); later rows are the product copy those slices render, recorded so
nobody invents it twice. ARCH §8.1 carries the same Z2 copy — the two
contracts were reconciled here (the agent-dock and editor rows previously
disagreed; ARCH now cites this table).

| Surface | Phase | Heading | Hint | Actions |
|---|---|---|---|---|
| Editor, no workspace | **Z2** | No folder open | *Open a folder to start editing.* | **Open Folder** — disabled in Z2, titled "Workspace opening lands in a later slice"; **Clone Repo** — disabled in Z2 with its later-slice caption (no `FrontendCommand` variant exists and none is invented in Z2, ARCH §8.1); plus a 64px Selora mark above the heading (the UI_SPEC §7 splash mark — Z2's always-empty editor is the first-run surface) and a shortcut hint row: `Ctrl+B`, `Ctrl+L`, `Ctrl+J`, `Ctrl+Shift+P` chips |
| Editor, folder, no file | Z4+ | — | `Ctrl+P` to find a file, `Ctrl+Shift+P` for commands | — (unreachable before Z4: no file data, and `Ctrl+P` is unbound in Z2 — ARCH §7.1) |
| Files view, no workspace | **Z2** | No folder open | *Open a folder to start editing.* | the same disabled Open Folder button |
| Files view, empty folder | Z4+ | — | Empty folder. | New File |
| Agent dock, no session | **Z2** (all rendered, all inert) / Z3 (wired) | Ask anything | *Ask about the code, or tell Selora what to change.* | a `--type-xs` `--fg-faint` status line below the hint: "Agent conversations arrive in a later phase."; then the three suggestion chips ("Explain this file" · "Find a bug in the selection" · "Summarize the workspace") in disabled styling (`--fg-disabled`, no hover — the §10 disabled state) plus the composer card visual with its placeholder — all inert in Z2: no key, chip, or Enter press dispatches a command until Z3 (ARCH §8.1); Z3 wires chips, composer, and beam |
| Agent dock, disconnected | Z3+ | Selora is not connected | *Connect a gateway key to start a session.* | Connect |
| Problems tab (bottom dock) | **Z2** (placeholder) / Z4+ (list) | — | "Diagnostics land with the editor slice" (Z2) → "No problems." (Z4+) | — |
| Output tab | **Z2** | — | Nothing has run yet. | — |
| Terminal tab (placeholder) | **Z2** | Terminal | The integrated terminal arrives in a later phase. Commands the agent runs appear in Output. | — |
| Search view | **Z2** (input only, controls disabled) / Z4+ (results) | — | input placeholder *Search the workspace*; results rows land with Z4's engine: "No results for “query”." | — |
| Git view | **Z2** (placeholder) / Z4+ (product) | — | "Git integration lands in a later slice" (Z2) → "Working tree clean." / "This folder is not a Git repository." | — |
| Settings view | **Z2** | — | read-only echo of `bootstrap.settings` (theme, reduced-motion, the two keybinds) + "Settings editing lands in a later slice" (ARCH §8.1) | — |

Rendered, not wired: UI_SPEC §12's second editor action, **Clone Repo**,
has no `FrontendCommand` variant (`crates/selora-protocol/src/lib.rs`
enum: `OpenFolder` exists, nothing clones) — it renders disabled in Z2
with its later-slice caption (ARCH §8.1) and gains wiring only when a
protocol command exists for it. UI_SPEC §12's editor-with-folder row
keeps its two-action form for the workspace-open slice.

**Disabled-state amendment (2026-10-09, owner-test finding):** the
original disabled state — `--fg-disabled` text and no hover, per the §10
state matrix — proved imperceptible on *filled* controls: the owner read
the ink-mid "Open Folder" button as clickable and reported the whole app
as "nothing is clickable" (a disabled control suppresses
mousedown/mouseup/click in Chromium while pointer events still fire,
which briefly looked like a WebView2 input-layer bug — it never was one).
Disabled filled controls therefore drop the fill: transparent background
+ hairline border + `--fg-disabled` text, the agent-chip disabled family;
disabled inputs drop their fill the same way. No new tokens, §12-clean,
and every disabled control carries an explanatory `title`.

## 7. Focus rings

- Two sanctioned forms, both the `--border-focus` token (1px
  `--fg-default`, DESIGN_TOKENS §5):
  - **Form A — bordered controls** (inputs, composer card, cards):
    swap the existing border color `--ink-hi` → `--fg-default` on
    `:focus-visible`; no outline. This is the landed egui behavior
    (`field_frame`, REVAMP_SPEC §7) translated to CSS.
  - **Form B — borderless controls** (buttons, activity pills, tree
    rows, tabs, menu items): `outline: var(--border-focus);
    outline-offset: 2px;` — the base.css default, matching the Z1
    shell's existing focus rule (`styles.css:2`).
- Ring only on `:focus-visible` (keyboard/programmatic), never on
  pointer focus. Never `outline: none` without a Form-A replacement.
- Contrast: `#e5e5e5` on the lightest surface (`--ink-hi` #2a2a2a) is
  ≈9.9:1 — far beyond the 3:1 non-text minimum.
- Focus returns to the invoking control when a dialog/popover closes;
  modals trap focus. (Component duty, recorded here because PHASE_Z §5
  makes keyboard navigation a hard requirement.)

## 8. Motion & reduced motion

Token layer: durations and curves are variables only (tokens.css §7);
no component hardcodes a millisecond. The eight §7.3 glyphs
(orbit/pulse/spark/wave/spin/fade/bloom/comet) are the whole
vocabulary; loops are never faster than `--dur-loop` (1200ms, §12);
`--dur-lap` (2400ms) exists for the composer border beam alone
(REVAMP_SPEC §11 bend 6a).

**Reduced motion — two layers, both shipped in this phase:**

1. **CSS loops.** `prefers-reduced-motion: reduce` (always honored; the
   app never overrides the OS) or `<html data-reduced-motion="true">`
   (the `ui.reduce_motion` setting, DESIGN_TOKENS §7.4; driven from
   `bootstrap.settings.reduced_motion` per ARCH §8.3) both flip
   `--motion-loops` to 0, and base.css forces
   `animation-iteration-count: 1`. Authoring rule that makes this
   correct: **every loop's 0% and 100% frames must be the resting
   frame** — true of all eight glyphs as spec'd in §7.3 — so one forced
   iteration parks the loop exactly at rest (pulse ends at scale 1 /
   opacity 1; the beam at phase 0, the top-left corner, which matches
   REVAMP_SPEC §8.4). One-shot `fade`/`bloom`/`spark` already run once
   and are untouched — §7.4's keep-list survives by construction, and
   spark's single pass *is* the "static flash" §7.4 asks for.
2. **JS/canvas effects** (BorderBeam, VoiceBeam, MetalFx, orbs — Z3+):
   read `--motion-loops` via `getComputedStyle` (or mirror the same two
   conditions) and render one static representative frame; request no
   animation frames. A closed dock must stop its effects entirely
   (PHASE_Z §11).

Hover/press transitions (≤ `--dur-quick`, one-shot state feedback) are
kept under reduced motion — they are instant affordance, not loops.

## 9. High-DPI notes (PHASE_Z §5)

- All tokens are CSS pixels; WebView2 applies the monitor's DPI scale.
  Never author in physical pixels, never use `image-set()`/resolution
  media queries for chrome.
- Hairlines stay 1 CSS px. At 125%/175% scaling Chromium snaps borders
  to device pixels; accept the rounding — do not compensate with
  fractional-px borders (they smear).
- Canvas/WebGL effects must scale their backing store by
  `devicePixelRatio` and re-query it when the window crosses monitors
  (`matchMedia` on `resolution`).
- No raster icons anywhere: the §10 system is inline SVG, and the brand
  mark is drawn in code (DESIGN_TOKENS §9).
- Verify at the four UI_SPEC §16 steps: 100%, 125%, 150%, 200%.

## 10. Iconography — inline SVG, no emoji

Decision: **inline SVG per icon**, the DESIGN_TOKENS §8 glyph set
ported to React components — not text glyphs (only ●/○ status dots and
key chords stay text, the UI_SPEC §1.2/§12 precedents) and never an
icon font, raster, or emoji (§12).

- Component contract: `<svg viewBox="0 0 16 16" fill="none"
  stroke="currentColor" stroke-width="1.5">` inside a React `Icon`
  component; size via `width/height` (16 default). Color always
  `currentColor`, driven by the §10 state matrix (idle `--fg-muted`,
  hover `--fg-default`, disabled `--fg-disabled`; semantic states only
  via `--ok`/`--warn`/`--err`).
- Stroke width: **1.5px**, per DESIGN_TOKENS §8 verbatim ("All icons use
  `currentColor`, 1.5px stroke, no fill") — the canonical and only
  sanctioned value (at the 16px grid, 1.5 is the difference between
  readable and wiry; DESIGN_TOKENS.md is the single source, and no other
  repo document specifies a stroke width).
- Caps/joins: `stroke-linecap="square"`, `stroke-linejoin="miter"` —
  designer's call, traceable to §4 "the brand is sharp"; round caps
  read soft against the square chrome.
- Activity-bar glyph map: files→`file`, search→`search`, git→
  `git-branch`, problems→`error`, agent dock→`chat`, settings→
  `settings`. Window controls add three glyphs in the same system:
  `win-min` (horizontal line), `win-max` (square), `win-restore`
  (offset squares); `close` already exists. (ARCH §9 fixes the Z2
  subset: file, search, git-branch, error, settings, chat, close,
  check, command.)
- The egui sprite set (`sidebar/icons.rs`, 24 glyphs, REVAMP_SPEC §7
  verify-only list) is the path source of truth; port paths, don't
  redraw.

## 11. Web tokens (PHASE_Z §6 documentation duty)

tokens.css carries **presentational chrome metrics only**, each derived
from an existing source:

| Token | Value | Source |
|---|---|---|
| `--chrome-titlebar-h` | 32px | UI_SPEC §1.1 |
| `--chrome-statusbar-h` | 24px | UI_SPEC §1.2 |
| `--chrome-activitybar-w` | 48px | UI_SPEC §2.1 |
| `--chrome-activity-pill` | 28px | REVAMP_SPEC §7 |
| `--chrome-edge-mark-w` | 2px | REVAMP_SPEC §7 (bend 3: painted rect, not a border) |
| `--chrome-panel-header-h` | 24px | UI_SPEC §2.2 |
| `--chrome-tabbar-h` | 32px | UI_SPEC §3.1 |
| `--chrome-agent-header-h` | 40px | REVAMP_SPEC §3 |

**Region sizes and bounds carry no CSS tokens by design.** Rust owns
them as protocol constants (ARCH §1.1: `PRIMARY_SIDEBAR_MIN/MAX_PX`
160/480, `AGENT_DOCK_MIN/MAX_PX` 240/600, `BOTTOM_DOCK_MIN/MAX_PX`
120/800 plus `BOTTOM_DOCK_MAX_WORKBENCH_RATIO` 0.5,
`EDITOR_MIN_WIDTH_PX` 400, `RESIZE_HIT_ZONE_MIN_PX` 6) with defaults in
the `LayoutDto` (ARCH §1.1: 240 / 360 closed / 200 closed — the Z1
state.rs stubs change in Z2). They reach the frontend only through
`bootstrap.layout_constraints` and `bootstrap.layout` (ARCH §5.1), and
reach CSS only as inline custom properties on the grid container (§1).
A second copy of the same numbers would be a second source of truth and
the two had already diverged once (240/200/hidden vs 280/220/open) — the
divergence is resolved to the 240/200/hidden side by ARCH §1.1 (the Z1
Rust stubs change in Z2), and leaving region values out of tokens.css
entirely keeps exactly one set of numbers anywhere (ARCH §1.1/§10/§11.6
record the same decision).

No new colors, radii, or durations were needed; the DESIGN_TOKENS ramp
covers the shell.

## 12. Font loading & licensing

- Five `@font-face` declarations in base.css map the bundled files to
  two families at their token weights: Space Grotesk 400/500/600,
  JetBrains Mono 400/500 (there is no 700 anywhere — §2.3). Sources:
  `assets/fonts/*.ttf`, copied to `frontend/public/fonts/` and served
  as `/fonts/*.ttf` — the CSP already allows `font-src 'self'`
  (`tauri.conf.json:30-31`; ARCH §6.2).
- `font-display: swap` on all faces; the fallback stacks
  (`system-ui`/`Segoe UI`, `Cascadia Code`/`Consolas`) only cover the
  swap window or a missing-file fault — DESIGN_TOKENS §2.1 forbids
  depending on the OS fonts.
- Both families are OFL 1.1; the license texts ship beside the TTFs in
  `public/fonts/` and are attributed in the base.css header comment.
- `font-synthesis-weight: none` on `body` blocks faux bold; style
  synthesis stays enabled for the two §2.5 italics (see §14).

## 13. Adoption notes (frontend developer)

1. In `main.tsx`, replace `import './styles.css'` with
   `import './styles/base.css'` — tokens arrive via base.css's
   `@import './tokens.css'` (base.css:12) — followed by component
   stylesheets: the single convention ARCH §8.3 records, matching the
   delivered CSS. Delete `styles.css` in Z2 — its hardcoded literals are
   exactly what the token layer replaces (ARCH §8.3).
2. Reflect the `ui.reduce_motion` bootstrap setting as
   `document.documentElement.dataset.reducedMotion = 'true'` and keep
   it in sync with settings events; the OS preference needs no code.
   (ARCH §8.3: effective reduced motion is the OR of the two.)
3. `tauri.conf.json` + the `main.rs` builder need `decorations: false`
   and the six `core:window:*` capability entries for the custom title
   bar, and the window title becomes "Selora IDE" — ARCH §6.3's exact
   contract (§3.1 here is the visual dependency).
4. Read every region size, bound, and the hit-zone width from
   `bootstrap.layout` / `bootstrap.layout_constraints`; write them as
   inline custom properties (§1). Component CSS reads tokens only: no
   hex literals, no raw px outside layout arithmetic on `--space-*`,
   no `box-shadow`, no `linear-gradient`, no `font-weight` above 600,
   no animation shorter than `--dur-instant` or longer than
   `--dur-long` (except `--dur-lap` on the composer beam alone).

## 14. Open decisions

1. **Missing italic cuts.** `assets/fonts/` ships JetBrains Mono
   Regular/Medium only (no italic), and Space Grotesk has no italic cut
   at all. The two §2.5-sanctioned italics (syntax comments,
   empty-state placeholders) will render as synthesized oblique. Either
   add the italic TTFs later (OFL permits) or accept synthesis —
   owner's call.
2. **Caret blink rate for Z4.** UI_SPEC §3.2's 500ms is the one
   pre-floor blink exception (§3.4 here); DESIGN_TOKENS §12's loop
   floor is 1200ms. Z4 must set CodeMirror's blink rate explicitly
   rather than inherit its default — and should pick 500 (UI_SPEC) or
   1200 (token floor). Recorded so the Z4 spec resolves it, not the
   implementer's whim.
3. **Custom close-button hover.** The title bar's close button follows
   the ink matrix (no red hover) per §12; Windows users may expect the
   red. Recorded as a deliberate deviation (§3.1).
4. **Clone Repo wiring.** UI_SPEC §12's second editor-empty action
   renders disabled in Z2 with its later-slice caption (§6) — no
   `FrontendCommand` variant exists and none is invented in Z2 (ARCH
   §8.1); it gains wiring only when a protocol command exists for it.

Resolved (recorded where they belong, not left open): problems is a
bottom-dock view in Z2 (§3.2/§3.6, ARCH §2.2/§11.5 — UI_SPEC §2's panel ④
and PHASE_Z §7 win over brief §2's sidebar parenthetical; the revision's
earlier sidebar-only resolution followed the pre-review ARCH and is
withdrawn); the bottom dock spans the editor column only (§1, ARCH §1's
diagram matches); region defaults are the Rust `LayoutDto`'s,
single-sourced at UI_SPEC §2's values (§2/§11, ARCH §1.1/§11.6 — the Z1
stubs change in Z2).
