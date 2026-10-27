# Phase Z2 architecture — premium shell layout

Status: implementation contract. The phase brief (`PHASE_Z_TAURI_PROMPT.md`) is
law; where this contract refines the brief for implementability the refinement
is stated as a decision with its reason. Every wire name in §2 was verified by
executing a serialization test against `crates/selora-protocol` (see §2.6).

Z2 delivers, per the brief §12 Z2 entry: the seven-region shell (title bar,
activity bar, primary sidebar, editor region, agent dock, bottom dock, status
bar), drag-resize with constraints, layout persistence, keyboard toggles and
command-palette entries, empty states, the CSS token layer, and the initial
toast surface.

**Explicit non-goals for Z2** (later slices — do not gold-plate):

- Editor bridge: no CodeMirror, no buffers, no `OpenFile`/`ApplyEditorChange`
  wiring (Z4).
- Real file-tree data: the Files view renders its empty state only (Z4).
- Agent streaming and the composer's **behavior** contract (Enter submits,
  Shift+Enter newline, history, stop — Z3). The agent dock's Z2 *chrome* —
  header, empty state, and composer card geometry per DESIGN §3.5 — ships
  inert; no control in it dispatches a command until Z3.
- Libraries.dev packages: none are installed or mounted in Z2 (Z3/Z5). No new
  npm runtime dependencies at all.
- Workspace opening / picker: `OpenFolder` stays unsupported; the workspace id
  remains `"no-workspace"` (later slice; see `state.rs:115-120`).
- Git, search, diagnostics, LSP data (Z4).
- Settings editing UI and `SettingsUpdate` (later slice).
- Notifications center / history (Z5; Z2 ships the live toast stack only).
- Fuzzy find (`Ctrl+P` is not bound in Z2 — it needs file data).

Every `FrontendCommand` variant other than the ones named in §2.1 keeps
returning the existing `unsupported_phase_slice` error (`adapter.rs:3-9`).

---

## 1. Regions and constraints

The shell grid (all heights/widths token-driven, never literals):

```
┌──────────────────────── title bar (32px) ────────────────────────┐
│ activity │ primary  │   editor region    │      agent dock       │
│ bar(48px)│ sidebar  ├────────────────────┤     (full height)     │
│          │ (full    │    bottom dock     │                       │
│          │ height)  │ (editor column)    │                       │
├──────────────────────── status bar (24px) ───────────────────────┤
```

The grid geometry is `PHASE_Z2_DESIGN.md` §1's and is **not re-specified
here** — one grid spec, the design doc's: the workbench is a six-column track
grid (activity bar | primary sidebar | 6px handle track | center column | 6px
handle track | agent dock); the center column stacks editor region | 6px
handle track | bottom dock; and the **primary sidebar and agent dock run the
full workbench height while the bottom dock spans the editor column only**
(UI_SPEC §2's diagram, where panel ④ sits under region ② — the geometry that
keeps the agent dock a full-height first-class surface per brief §2). Handles
consume their own 6px grid tracks and collapse to 0 together with their
region (DESIGN §1).

Chrome heights come from UI_SPEC §1.1 (title bar 32px), §1.2 (status bar
24px), §2.1 (activity bar 48px wide). The designer's `tokens.css` exposes them
as `--chrome-titlebar-h: 32px`, `--chrome-statusbar-h: 24px`,
`--chrome-activitybar-w: 48px` (tokens.css, "Web chrome metrics" block — each
variable carries its UI_SPEC section in an inline comment); component CSS
consumes the variables.

### 1.1 Region size bounds (Rust-enforced, single source of truth)

New constants in `crates/selora-protocol/src/lib.rs`. Numbers are taken from
UI_SPEC §2's region table and the brief §6's 6px hit-zone rule; the bottom-dock
floor is a Z2 decision (justified below):

| Region | Min | Max | Default (stored) | First-run state |
|---|---|---|---|---|
| `primary_sidebar` width | 160px | 480px | 240px | open |
| `agent_dock` width | 240px | 600px | 360px | **closed** |
| `bottom_dock` height | 120px | 800px | 200px | **closed** |

Sources: bounds 160/480, 240/600 and the editor's 400px floor from UI_SPEC
§2's region table; defaults 240/360/200 and the closed first-run states from
the same table (① "visible", ③ "hidden", ④ "hidden") — which is also what
the egui settings defaults (`settings.rs:58-64`: 240/360) and the egui
persistence default (`persistence.rs:49-56`, `chat_open` false) say, and
what the first-delivered `tokens.css` `--region-*` tokens pinned
(240/200) before the designer's revision single-sourced region values on
Rust (below).
**The Z1 stub values in `state.rs:68-76` (280/220, agent dock open) and the
protocol test fixture `lib.rs:800-808` are diagnostic placeholders, not
product decisions — Z2 changes them to this table's values.** After that
change the protocol constants are the single source of truth. `tokens.css`
carries **no** `--region-*` values (the designer's revision removed them,
for exactly this reason): region sizes and bounds reach the frontend only
through `bootstrap.layout` / `bootstrap.layout_constraints` (§5.1) and reach
CSS only as inline custom properties on the grid container (DESIGN §1) — no
second copy exists to drift. UI_SPEC §2 gives the bottom dock "min 0, max 50% of workbench
height": min 0 meant *hidden*, which the protocol models as
`bottom_dock_open: false`, so an **open** bottom dock gets a usable floor of
120px (Z2 decision, DESIGN §2). The 50% rule is dynamic and cannot be
validated in Rust (Rust does not know the webview's workbench height), so it
is split: Rust enforces the static ceiling 800px, and the frontend
additionally clamps to `0.5 × measured workbench height` before dispatching
(§2.4, §3.1).

Derived constraint (frontend-enforced, no protocol value): the editor region
never goes below 400px (UI_SPEC §2 region ② min). The frontend clamps resize
proposals so `editor ≥ 400px`. Arithmetic at the 960×640 minimum window
(`main.rs:20`, `tauri.conf.json:20-21`), including the two width-wise 6px
handle tracks from DESIGN §1's grid (they are real layout and must be
reserved in the clamping code): 48 + 160 + 6 + 400 + 6 + 240 = 860 ≤ 960 —
the minimums fit simultaneously (DESIGN §2.1's budget, restated here so the
clamping code and the spec agree).

Exact Rust additions:

```rust
// crates/selora-protocol/src/lib.rs
pub const PRIMARY_SIDEBAR_MIN_PX: f32 = 160.0;
pub const PRIMARY_SIDEBAR_MAX_PX: f32 = 480.0;
pub const AGENT_DOCK_MIN_PX: f32 = 240.0;
pub const AGENT_DOCK_MAX_PX: f32 = 600.0;
pub const BOTTOM_DOCK_MIN_PX: f32 = 120.0;
pub const BOTTOM_DOCK_MAX_PX: f32 = 800.0;
pub const BOTTOM_DOCK_MAX_WORKBENCH_RATIO: f32 = 0.5;
pub const EDITOR_MIN_WIDTH_PX: f32 = 400.0;
pub const RESIZE_HIT_ZONE_MIN_PX: f32 = 6.0; // brief §6
pub const EDITOR_SPLITS_MIN: u8 = 1;
pub const EDITOR_SPLITS_MAX: u8 = 4;

/// Clamp a ResizeRegion value into the region's bounds. Finite out-of-range
/// values are clamped, never rejected (drag-end values routinely land a
/// fraction past a bound). The non-finite check is a Rust-internal invariant
/// guard: JSON cannot carry NaN/±Inf and serde_json rejects out-of-range
/// number literals at the parse boundary, so no wire input can reach it —
/// it exists for future non-wire callers, not as a wire threat model.
pub fn clamp_region_size(
    region: RegionId,
    size_px: f32,
) -> Result<f32, ProtocolValidationError>;
```

`ProtocolValidationError` gains one variant: `InvalidRegionSize`. (Additive;
the only exhaustive matcher is this crate's own `Display` impl.)

---

## 2. Command / event wiring contract

### 2.1 Exact serde wire names (verified — §2.6)

`FrontendCommand` is `#[serde(tag = "type", content = "payload", rename_all =
"snake_case")]` (`lib.rs:237-239`). The three Z2 commands the frontend sends:

```json
{"type":"toggle_surface","payload":{"surface":"agent"}}
{"type":"resize_region","payload":{"region":"primary_sidebar","size_px":240.0}}
{"type":"save_layout"}
```

Note: `SaveLayout` is a unit variant — there is **no** `payload` key (same
shape as the existing `{"type":"open_folder"}` asserted at `lib.rs:856-861`).

`FrontendEvent` uses the same envelope (`lib.rs:565-567`). The two Z2 events
the frontend receives:

```json
{"type":"workspace_layout_changed","payload":{"primary_sidebar_width_px":240.0,"agent_dock_width_px":360.0,"agent_dock_open":false,"bottom_dock_height_px":200.0,"bottom_dock_open":false,"bottom_view":"output","editor_splits":1,"activity_view":"files","primary_sidebar_open":true}}
{"type":"toast","payload":{"severity":"info","message":"..."}}
```

Enum wire spellings (all from `rename_all = "snake_case"`, `lib.rs:212-230`
and `lib.rs:551-558`):

- `SurfaceId`: `"files"`, `"search"`, `"git"`, `"problems"`, `"settings"`,
  `"agent"`, `"output"`, `"terminal_placeholder"`
- `RegionId`: `"primary_sidebar"`, `"agent_dock"`, `"bottom_dock"`
- `ToastSeverity`: `"info"`, `"success"`, `"warning"`, `"error"`

All three commands ride the existing `dispatch_command` Tauri command inside
the existing `RequestEnvelope` (`commands.rs:21-27`) — no new Tauri command
for them, no capability change for them (§6.3). Transport rules:

- `envelope.protocol_version` = 1 (`PROTOCOL_VERSION`, `lib.rs:10`).
- `envelope.core_instance_id` = the bootstrap's `core_instance_id`.
- `envelope.workspace_id` = `"no-workspace"`; `envelope.session_id` = `null`
  (the Z1-era validation stays, §2.3).
- `envelope.request_id` = a fresh `crypto.randomUUID()` per dispatch (WebView2
  API, CSP-safe; no dependency).
- JS invoke argument names: Tauri converts camelCase JS keys to snake_case
  Rust parameters — `invoke('get_bootstrap_state', { protocolVersion: 1 })`
  (`bridge.ts:20`), `invoke('dispatch_command', { envelope })`
  (`bridge.ts:21`), and the new `invoke('get_events_after', { afterSequence })`
  (§3.3).

### 2.2 ToggleSurface routing table (Rust-side semantics)

`ToggleSurface` carries a `SurfaceId`; the region it acts on is determined in
Rust, never by the frontend:

| `surface` | Target region | Semantics |
|---|---|---|
| `files`, `search`, `git`, `settings` | primary sidebar | if `primary_sidebar_open && activity_view == surface` → close sidebar; else → `primary_sidebar_open = true`, `activity_view = surface` |
| `agent` | agent dock | flip `agent_dock_open` |
| `problems`, `output`, `terminal_placeholder` | bottom dock | if `bottom_dock_open && bottom_view == surface` → close dock; else → `bottom_dock_open = true`, `bottom_view = surface` |

This is the VS Code convention and satisfies brief §6 ("a collapsed region
must have an accessible toggle and a command-palette command"). **`problems`
is a bottom-dock view in Z2** — UI_SPEC §2's panel ④, brief §7's bottom-dock
list, DESIGN §3.6's tab strip, and VS Code's placement all put it there; brief
§2's sidebar parenthetical is the superseded reading, and DESIGN §14.2's open
question is resolved to this (one surface, the bottom dock's tab; a second
sidebar density would need a new protocol id and is not in Z2). The activity
bar's `error` icon (DESIGN §3.2) and `Ctrl+Shift+M` both dispatch
`ToggleSurface{surface:"problems"}` and target the bottom dock.

Every accepted toggle changes state, so Rust emits `WorkspaceLayoutChanged`
after each one (§3.2).

### 2.3 Validation Rust applies to every command

In order, for all three commands (existing code, unchanged —
`state.rs:100-127`):

1. `envelope.protocol_version != PROTOCOL_VERSION` →
   `ProtocolErrorDto{code:"protocol_version_mismatch", retryable:false}`.
2. `envelope.core_instance_id != this process's id` →
   `code:"stale_core_instance", retryable:true`.
3. `envelope.workspace_id != Some("no-workspace")` **or** `session_id.is_some()`
   → `code:"stale_workspace_or_session", retryable:true`.

Then per command:

4. **Serde boundary (before any Rust arm runs):** unknown `surface`/`region`
   strings, wrong field types, or `size_px: null` fail enum/struct
   deserialization and Tauri rejects the invoke with its own error string —
   this is *not* a `ProtocolErrorDto`. The frontend's `invoke` promise rejects;
   the bridge surfaces it through the command-error path (§3.4).
5. `ResizeRegion`: `clamp_region_size` — finite out-of-range values are
   clamped into the §1.1 bounds, **not** an error. The non-finite branch
   (`ProtocolErrorDto{code:"invalid_region_size", retryable:false}`) cannot
   be reached from the wire: JSON has no NaN/±Inf spelling and serde_json
   rejects out-of-range number literals during deserialization, so malformed
   magnitudes die at the serde boundary like any other type error (item 4);
   the check stands as a Rust-internal invariant for future non-wire callers.
   Resizing a closed region is allowed (the size applies when it next opens).
   If the clamped value equals the stored value: success, **no** event
   emitted (idempotent drag end).
6. `ToggleSurface`: no validation beyond steps 1–4; all eight variants are
   routable per §2.2.
7. `SaveLayout`: persistence failure →
   `ProtocolErrorDto{code:"layout_save_failed", retryable:true}`; the
   in-memory layout is unaffected.

`surface`/`region` id validity is therefore enforced twice: once by serde
(unknown spellings) and once by the routing table (each known spelling has
exactly one meaning).

### 2.4 Authoritative layout application (Rust)

`DesktopState` gains `layout: Arc<Mutex<LayoutDto>>`, seeded at startup from
settings (§4). On an accepted `ToggleSurface`/`ResizeRegion`: mutate, then
`record_event(app, FrontendEvent::WorkspaceLayoutChanged(layout.clone()),
request_id: Some(envelope.request_id))` — the existing emitter
(`state.rs:134-155`, currently `dead_code`) goes live; `dispatch_command`
therefore gains an `app: tauri::AppHandle` parameter. The dispatch response
remains a full `BootstrapState` (`commands.rs:22-27`), so the response carries
the new layout too, but the frontend applies layout changes **only** from the
event (§3.2) — the response is used for error inspection, not state.

### 2.5 bridge.ts extensions (exact)

```ts
export type SurfaceId = 'files'|'search'|'git'|'problems'|'settings'|'agent'|'output'|'terminal_placeholder';
export type RegionId = 'primary_sidebar'|'agent_dock'|'bottom_dock';
export type Layout = {
  primary_sidebar_width_px: number; agent_dock_width_px: number; agent_dock_open: boolean;
  bottom_dock_height_px: number; bottom_dock_open: boolean; bottom_view: SurfaceId;
  editor_splits: number; activity_view: SurfaceId; primary_sidebar_open: boolean;
};
export type LayoutConstraints = {
  primary_sidebar_min_px: number; primary_sidebar_max_px: number;
  agent_dock_min_px: number; agent_dock_max_px: number;
  bottom_dock_min_px: number; bottom_dock_max_px: number;
  bottom_dock_max_workbench_ratio: number; editor_min_width_px: number;
  resize_hit_zone_min_px: number;
};
// FrontendCommand gains:
//   { type:'toggle_surface'; payload:{ surface: SurfaceId } }
//   { type:'resize_region'; payload:{ region: RegionId; size_px: number } }
//   { type:'save_layout' }
// FrontendEvent gains:
//   { type:'workspace_layout_changed'; payload: Layout }
//   (toast and error already exist — bridge.ts:16)
// BootstrapState.layout becomes Layout; BootstrapState gains
//   layout_constraints: LayoutConstraints
// BootstrapState.settings gains keybind_command_palette: string,
//   keybind_toggle_chat: string (§5.1, consumed by §7.1)
export const toggleSurface = (surface: SurfaceId, ctx: EnvelopeContext): Promise<unknown>;
export const resizeRegion = (region: RegionId, sizePx: number, ctx: EnvelopeContext): Promise<unknown>;
export const saveLayout = (ctx: EnvelopeContext): Promise<unknown>;
export const getEventsAfter = (afterSequence: number): Promise<EventEnvelope[]>;
```

`EnvelopeContext` is the bridge-held `{ core_instance_id, workspace_id,
session_id }` from the last applied bootstrap; the helpers build the
`RequestEnvelope` so components never hand-assemble one. The bridge's
`FrontendEvent` type enumerates **all** protocol variants; the Z2 reducer
handles `workspace_layout_changed`, `toast`, `error` and explicitly ignores
the rest.

### 2.6 Verification performed for this contract

The wire names and envelope shapes in §2.1 are not derived by hand — they
were produced by a temporary integration test against the real crate:
`cargo test -p selora-protocol --test z2_wire_check -- --nocapture` (test file
created, run — 1 passed — and deleted; `git status` clean afterwards; the
numeric values in the test's fixtures were arbitrary — what was verified is
the envelope shape, the field names, and every enum spelling; the §2.1
examples carry §1.1's post-Z2 defaults). It additionally confirmed: `size_px: null` is rejected at the serde boundary, and
a partial `LayoutDto` JSON fails to deserialize today (no serde defaults yet —
motivating §4.2). Baseline at time of writing: `cargo check -p selora-desktop
-p selora-settings` clean; `npm run typecheck` clean.

---

## 3. Event cadence contract

### 3.1 Drag resize

- Resize handles: a 6px hit zone (`RESIZE_HIT_ZONE_MIN_PX`, brief §6) as its
  own grid track centered on the region boundary — 3px overlapping each
  neighbor — so the hit zone is honest layout, never an overlay guess
  (DESIGN §1/§4). The visual boundary stays the adjacent region's own 1px
  `--ink-hi` hairline; hover lifts it to `--ink-rule`, and active drag or
  keyboard focus lights it `--fg-default` (DESIGN §4 — that lit line is the
  handle's focus indicator; no outline around a 6px strip).
  `cursor: col-resize` / `row-resize`. Handles are keyboard reachable:
  `role="separator"`, `tabindex="0"`, `aria-orientation`, `aria-label`,
  `aria-valuenow` (current px), `aria-valuemin`/`aria-valuemax` from
  `layout_constraints`. **Keyboard steps follow DESIGN §4 exactly** — Arrow
  ±4px (`--space-1`), Shift+Arrow ±16px (`--space-4`), Home/End jump to the
  region min/max, and double-click resets to the region default. Every
  completed interaction — a keyboard step, a Home/End jump, or a double-click
  reset — is a completed resize and sends the same `ResizeRegion` +
  `SaveLayout` pair as a drag end. Final sizes snap to whole CSS pixels
  (DESIGN §4).
- `pointerdown`: `setPointerCapture`, record `{region, startPos, startSize}`.
- `pointermove`: update **local React state only** (CSS custom-property write
  on the grid container, §6.2). **Zero IPC during a drag.** The proposal is
  clamped client-side first: `clamp(px, region_min, min(region_max,
  bottom ? 0.5 × workbenchHeight : ∞))` and never letting the editor drop
  below `editor_min_width_px` — all constants from
  `bootstrap.layout_constraints`, never literals.
- `pointerup`: release capture, then send exactly **one**
  `ResizeRegion{region, size_px: final}`; `await` its resolution, **then**
  send `SaveLayout`. Ordering is mandatory — Tauri commands run concurrently,
  and a `SaveLayout` that lands first would persist the pre-drag layout.
- The local preview is a *preview*: once the `WorkspaceLayoutChanged` event
  applies, React state is the authoritative layout and the CSS variables are
  driven from it again.

### 3.2 Toggles

Keyboard shortcut, activity-bar click, or palette entry → send `ToggleSurface`
immediately (no debounce, no optimistic UI — brief §5 allows optimism only
with defined rollback; localhost IPC makes the event round trip effectively
instant), then `await` it and send `SaveLayout`. The frontend applies the
resulting `WorkspaceLayoutChanged` only through the bridge gate (§3.4).
Rapid successive toggles converge: every toggle is absolute (§2.2), events are
ordered, and the frontend's layout is exactly the last applied event.

### 3.3 SaveLayout triggers and close-time save

`SaveLayout` fires: (a) after every accepted toggle, (b) after every drag end
/ keyboard-resize step, (c) on window close-requested. For (c) the save runs
**in Rust**: `main.rs` registers `.on_window_event` and, on
`WindowEvent::CloseRequested`, synchronously persists the layout (the §4
atomic write) before allowing the close. This is a deliberate refinement of
"SaveLayout fires on window close-requested": an IPC command cannot be
guaranteed to flush during webview teardown (`beforeunload` is unreliable in
WebView2), so the close-time save is owned by the process that owns the file.
The frontend never attempts close-time persistence. No debouncing anywhere in
Z2 — toggles/drags are low-frequency and the write is a small atomic file
replace.

### 3.4 Event application gate and replay

All `selora://event` payloads (`state.rs:153`) are applied only after
`validateEvent` (`bridge.ts:22-28`) returns `'applied'`:

- `'stale'` → ignore.
- `'gap'`, `'epoch-mismatch'`, `'version-mismatch'` → discard all held state
  and re-run the bootstrap flow. (Z1's blunt "reload bootstrap on any event"
  in `main.tsx:11` is removed in Z2.)

Startup/reconnect sequence (this is brief §5's "reconnect and bootstrap
replay" made concrete):

1. `getBootstrapState()` → record the envelope context. Record
   `snapshot_sequence` S for diagnostics only — **S is never the gate's
   seed**.
2. Subscribe to `selora://event`; buffer incoming events.
3. `getEventsAfter(0)` → replay events that preceded the subscription (the
   boot-time settings toast, §4.4, is the first real one — it was recorded
   during `DesktopState::new`, so its sequence is ≤ S) through the same gate,
   in sequence order, **with the gate seeded `lastSequence = null`**
   (`validateEvent`'s null case, `bridge.ts:22-28`, means "no history
   check"). Seeding the gate with S would return `'stale'` for every
   pre-bootstrap event (`bridge.ts:25-26`) and silently drop the boot toast,
   breaking the §10 acceptance check — the seed is always null.
4. Drain the live buffer through the gate; then apply live events as they
   arrive. `lastSeen` is monotonic from the first replayed event across
   replay and live streams; duplicates from the overlap are dropped by the
   stale check.

New Tauri command (the only one Z2 adds):

```rust
#[tauri::command]
pub fn get_events_after(
    after_sequence: u64,
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<EventEnvelope<FrontendEvent>>, ProtocolErrorDto>
```

 backed by the existing `events_after` (`state.rs:157-166`). `record_event`
 gains a retention cap: keep the most recent **1024** events; a replay request
 older than the oldest retained event simply yields a gap at the gate and the
 frontend re-bootstraps — the failure mode is already designed for.

---

## 4. Layout persistence

### 4.1 Decision: a `layout` section in `settings.json`

The layout lives in **`selora-settings`' `settings.json`**, not the
`workspace.json` persistence model (`crates/selora-app/src/persistence.rs`).
Reasons:

1. `workspace.json`'s model lives inside `selora-app`, the eframe
   *application* crate, and is entangled with egui UI types
   (`persistence.rs:20` imports `selora_ui::sidebar::chat::ChatSession`) —
   the desktop shell would inherit the whole app crate, not a reusable data
   model. (Egui itself is already in the desktop binary's dependency tree
   via `selora-settings`' keybind parser — that is not the objection; the
   application-crate coupling is.) `selora-settings` is the shared
   dependency the desktop crate already uses (`state.rs:19` and the desktop
   app's `Cargo.toml`).
2. `workspace.json` is itself not per-workspace — it records the *last*
   workspace (`persistence.rs:1-9`). The brief's "per workspace" cannot be
   honestly delivered by either store today, and the workspace-picker slice
   that would give it meaning lands later. Z2 therefore ships a **global
   default layout** with the per-workspace keying strategy pinned for that
   slice (§4.5). This deviation from brief §6's "per workspace" wording is
   owner-authorized by the Z2 tasking and recorded here plainly.
3. Settings gives atomic writes, a schema-version gate, and serde-default
   forward compatibility for free (`settings.rs:372-458`), including the
   established precedent that additive serde-defaulted sections let old files
   load unchanged (`settings.rs:647-664`, the pre-Phase-5 test).

`selora-app`'s own layout state (`ui.sidebar_width`, `ui.chat_sidebar_width`)
is **untouched** — the egui shell keeps its keys and behavior; reconciling the
two at cutover is a Z6 decision (§9).

### 4.2 Exact schema addition

`crates/selora-settings/src/settings.rs` gains:

```rust
/// `layout.*` — the Tauri shell's workbench layout (Phase Z2). Global in Z2;
/// the workspace-picker slice adds per-workspace overrides (§4.5 of
/// docs/ui/PHASE_Z2_ARCHITECTURE.md). Every field is serde-defaulted so a
/// pre-Z2 settings.json loads unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutSettings {
    #[serde(default = "default_primary_sidebar_width_px")] // 240.0
    pub primary_sidebar_width_px: f32,
    #[serde(default = "default_agent_dock_width_px")]      // 360.0
    pub agent_dock_width_px: f32,
    #[serde(default = "default_agent_dock_open")]          // false
    pub agent_dock_open: bool,
    #[serde(default = "default_bottom_dock_height_px")]    // 200.0
    pub bottom_dock_height_px: f32,
    #[serde(default)]                                      // false
    pub bottom_dock_open: bool,
    #[serde(default = "default_bottom_view")]              // SurfaceId::Output
    pub bottom_view: SurfaceId,
    #[serde(default = "default_editor_splits")]            // 1
    pub editor_splits: u8,
    #[serde(default = "default_activity_view")]            // SurfaceId::Files
    pub activity_view: SurfaceId,
    #[serde(default = "default_primary_sidebar_open")]     // true
    pub primary_sidebar_open: bool,
}

// Compiles only with an explicit Default — and the Default impl must
// delegate to the same default_* fns (the pattern every existing section
// follows, e.g. EditorSettings at settings.rs:311-322), so the serde
// defaults, `LayoutSettings::default()`, and the §1.1 constants can never
// diverge:

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            primary_sidebar_width_px: default_primary_sidebar_width_px(),
            agent_dock_width_px: default_agent_dock_width_px(),
            agent_dock_open: default_agent_dock_open(),
            bottom_dock_height_px: default_bottom_dock_height_px(),
            bottom_dock_open: false,
            bottom_view: default_bottom_view(),
            editor_splits: default_editor_splits(),
            activity_view: default_activity_view(),
            primary_sidebar_open: default_primary_sidebar_open(),
        }
    }
}

// Settings gains (and Settings::default(), settings.rs:297-309, gains
// `layout: LayoutSettings::default()`):
#[serde(default)]
pub layout: LayoutSettings,
```

The default functions delegate to the §1.1 protocol constants, which are the
single source of truth. `SurfaceId` is reused directly:
`crates/selora-settings/Cargo.toml` gains `selora-protocol.workspace = true`
(no dependency cycle — protocol depends only on serde/serde_json/url; the egui
app's transitive build of protocol via settings is harmless). The
`settings.json` spellings are the protocol's snake_case wire names
(`"bottom_view": "output"`), so the file and the wire share one vocabulary.

Wire shape of a full section:

```json
"layout": {
  "primary_sidebar_width_px": 240.0,
  "agent_dock_width_px": 360.0,
  "agent_dock_open": false,
  "bottom_dock_height_px": 200.0,
  "bottom_dock_open": false,
  "bottom_view": "output",
  "editor_splits": 1,
  "activity_view": "files",
  "primary_sidebar_open": true
}
```

`Settings.version` (`_v`) **stays 1**: additive sections with serde defaults
are the schema's own compatibility mechanism (`settings.rs:6-7`), and a
missing `layout` key deserializes to the defaults. Writes go through the
existing `Settings::save` / `save_to` (atomic temp+rename, `settings.rs:435-458`).
`DesktopState::new` (`state.rs:18-24`) already calls `load_or_default`; Z2
reads `settings.layout` into the bootstrap's `LayoutDto`, sanitizing first
(§4.3).

### 4.3 Load-time sanitization (protocol)

`LayoutDto` gains `pub fn sanitized(self) -> Self` in `selora-protocol`:
clamp all sizes into the §1.1 bounds (and non-finite persisted values to the
defaults), clamp `editor_splits` into `1..=4`, and reset `activity_view` /
`bottom_view` if they name a surface outside the valid sets:

```rust
pub const ACTIVITY_VIEWS: [SurfaceId; 4] =
    [SurfaceId::Files, SurfaceId::Search, SurfaceId::Git, SurfaceId::Settings];
pub const BOTTOM_VIEWS: [SurfaceId; 3] =
    [SurfaceId::Output, SurfaceId::TerminalPlaceholder, SurfaceId::Problems];
```

`ACTIVITY_VIEWS` excludes `Problems` (bottom-dock placement, §2.2) — brief
§2's sidebar list is the superseded reading there. Sanitization runs on
settings load and after every accepted resize — a persisted value can never
wedge the shell.

### 4.4 Save failure and the first real toast

`SaveLayout` maps a `Settings::save` error to `layout_save_failed`
(retryable, §2.3) — command errors surface through the frontend's toast
component via its command-error path. Separately, `DesktopState::new` already
captures a settings load/wipe error via `load_or_default` (`state.rs:19`);
Z2 records it at startup as `FrontendEvent::Toast(ToastDto{severity: Error,
message})`, which the replay flow (§3.4) delivers to the frontend. These are
the two Z2 toast producers; no others are invented.

### 4.5 Per-workspace keying strategy (documented, implemented later)

When the workspace-picker slice lands, `LayoutSettings` gains a sibling map:

```rust
#[serde(default)]
pub workspaces: BTreeMap<String, LayoutSettings>, // key: canonical absolute workspace root
```

Lookup order: `layout.workspaces[canonical_root]` → else the global `layout`.
The key is the Rust-canonicalized root path string (the same canonicalization
`resolve_workspace_path` already performs, `security.rs:13-21`). Z2 code must
route all layout reads/writes in `state.rs` through one `LayoutStore` seam
(`load_layout(&Settings)`, `persist_layout(&mut Settings, &LayoutDto)`) so the
map drops in without touching dispatch. Whether the key stays a raw path or
becomes a hash is left to that slice (§9).

---

## 5. LayoutDto evolution

### 5.1 Exact field additions

```rust
// crates/selora-protocol/src/lib.rs — LayoutDto gains:
#[serde(default = "default_activity_view")]        // SurfaceId::Files
pub activity_view: SurfaceId,
#[serde(default = "default_primary_sidebar_open")] // true
pub primary_sidebar_open: bool,
```

`activity_view` covers brief §6's "active activity view"; `bottom_view`
(already present, `lib.rs:427`) covers "active bottom-dock view".
`primary_sidebar_open` is added because `Ctrl+B` toggling needs Rust-owned
open state — the brief's persist list implies it ("agent dock width and
open/closed state" sets the pattern; a collapsed sidebar must restore).

Additionally, **every existing `LayoutDto` field gains a serde default**
(matching §4.2's values). Today a partial `LayoutDto` fails to deserialize
(verified, §2.6); defaults make the DTO self-sufficient for persistence and
forward-compatible in both directions.

`BootstrapState` gains `pub layout_constraints: LayoutConstraintsDto`
(field-for-field mirror of the §1.1 constants, frontend-facing so no bound is
ever a JS literal — brief §6's "no hardcoded magic numbers"). `BootstrapState`
fields carry no serde defaults today and stay that way: the bootstrap is
always generated whole; the one protocol test fixture (`lib.rs:751-823`) is
updated.

`SettingsViewDto` (`lib.rs:415-419`) gains two serde-defaulted fields for the
§7 keyboard contract:

```rust
#[serde(default = "default_keybind_command_palette")] // "Ctrl+Shift+P"
pub keybind_command_palette: String,
#[serde(default = "default_keybind_toggle_chat")]     // "Ctrl+L"
pub keybind_toggle_chat: String,
```

populated from `settings.keybinds` so a user's egui-side rebind of the
rebindable pair carries into the new shell. The other two settings keybinds
(`fuzzy_find`, `save`) are not exposed until their slices use them.

### 5.2 `protocol_version` stays 1

Justification: every Z2 change is additive-with-serde-defaults. serde ignores
unknown fields by default (no `deny_unknown_fields` anywhere in the crate), so
an old reader tolerates new fields and a new reader tolerates old JSON — both
directions interoperate without a version gate. The frontend is bundled with
the backend in one binary (`frontendDist`, `tauri.conf.json:9`), so mismatched
peers cannot occur in the field; the `protocol_version` handshake
(`commands.rs:11-17`) exists to catch stale frontend assets after an upgrade,
and additive DTO growth does not invalidate them. A bump is reserved for
semantic breaks: field removal, rename, or meaning change. If frontend assets
are ever shipped decoupled from the binary, revisit.

---

## 6. Security contract for Z2 (brief §10 applied)

### 6.1 What the frontend may never do

Unchanged from brief §3.1, restated for this slice's surface area: no file
reads/writes, no shell, no keyring, no LSP, no gateway calls, no approvals, no
authoritative-state mutation outside the typed protocol. Concretely in Z2:

- **No `localStorage`/`IndexedDB`/cookie persistence of layout or anything
  else.** Rust is the only persistence authority (§4); the frontend holds
  layout in memory only. (Brief §10: no secrets in localStorage — extended to
  layout to keep one writer.)
- No Tauri plugins are added (no fs, dialog, shell, os, notification). Z2's
  only IPC is `get_bootstrap_state`, `dispatch_command`, `get_events_after`,
  and the `core:event`/`core:window` items in §6.3.
- Toasts render as text only — React's default escaping; no
  `dangerouslySetInnerHTML` anywhere in Z2 (brief §10's untrusted-HTML rule).
- No remote sources of any kind: no CDN fonts, no remote images, no external
  scripts/styles. `connect-src` stays `ipc: http://ipc.localhost` (plus the
  dev-only Vite websocket), exactly as configured (`tauri.conf.json:30-31`).

### 6.2 CSP: no change required

Fonts for the web shell are **bundled locally**: `assets/fonts/` already
contains `SpaceGrotesk-{Regular,Medium,SemiBold}.ttf`,
`JetBrainsMono-{Regular,Medium}.ttf` and both OFL license files (verified by
directory listing). They are copied to
`apps/selora-desktop/frontend/public/fonts/` (designer's task) and served
from the app origin, so the existing `font-src 'self'`
(`tauri.conf.json:30`) already covers them — **no CSP change in Z2**. The
`@font-face` declarations live in the designer's `base.css` with
`font-display: swap` and OFL attribution comments.

One `style-src` caveat implementers must respect: the production CSP is
`style-src 'self'` (no `'unsafe-inline'`; the dev CSP relaxes it,
`tauri.conf.json:31`). In Chromium, `style-src` governs `<style>` elements
and style-attribute *parsing* (`setAttribute('style', ...)`, HTML
`style="…"`), but **not** CSSOM writes — `el.style.setProperty(...)`, React
`style` props, and even `cssText` assignment are not blocked by it. The
mandated practice still stands: all drag-resize writes go through
`el.style.setProperty('--…', …)` — individual property writes keep the
shell clear of the spec's gray areas and keep the drag preview on one code
path. Because dev mode masks CSP behavior entirely, "drag-resize works under
the production CSP" is a Z2 acceptance check (§10), not an assumption.

### 6.3 Capabilities: exact delta

`capabilities/default.json` (currently `allow-get-bootstrap-state`,
`allow-dispatch-command`, `core:event:allow-listen`, `core:event:allow-unlisten`)
gains exactly six entries — the replay command and the custom-title-bar window
controls (identifiers verified against the local schema
`gen/schemas/windows-schema.json`):

```json
"allow-get-events-after",
"core:window:allow-minimize",
"core:window:allow-toggle-maximize",
"core:window:allow-is-maximized",
"core:window:allow-close",
"core:window:allow-start-dragging"
```

The window permissions are local-window, low-risk, and required because Z2
renders a custom title bar: brief §2's anatomy and §7's app chrome say
"custom title bar", which supersedes UI_SPEC §1.1's OS-chrome decision (that
decision was written for the egui Lite shell and stays in force there).
`tauri.conf.json` windows[0] gains `"decorations": false` **and** the builder
in `main.rs` gains `.decorations(false)` — the builder is authoritative for
this manually created window (the existing code duplicates title/size in both
places for the same reason, `main.rs:13-23`). The title bar is a 32px
`data-tauri-drag-region` with minimize / maximize-restore / close buttons
calling `@tauri-apps/api/window`'s `getCurrentWindow()` methods; double-click
on the drag region toggles maximize natively. Close routes through
`CloseRequested`, so the Rust close-time save (§3.3) always runs.

Navigation stays locked to the Tauri origin (+ the Vite dev origin in debug)
and new windows stay denied (`security.rs:33-39`, `main.rs:21-22`).
`resolve_workspace_path` (`security.rs:8-25`) exists but is not exercised in
Z2 — no workspace opens. The window title in both `main.rs` and
`tauri.conf.json` becomes `"Selora IDE"` (the "diagnostic shell" subtitle
ends with Z2).

### 6.4 Stale-event posture

Rust-side envelope rejection (§2.3) and the frontend gate (§3.4) together
implement brief §10's "stale request/session/version events discarded" for
the layout slice. No approval, credential, or tool-decision paths exist in Z2.

---

## 7. Keyboard and command-palette contract

### 7.1 Canonical shortcuts

Aligned with the egui app's bindings: the rebindable pair comes from
`settings.keybinds` (defaults at `settings.rs:70-84`, parser at
`keybind.rs:36-67`) via the new `SettingsViewDto` fields (§5.1); the rest
mirror the egui fixed table (`shell.rs:2357-2370`) and UI_SPEC §15.

| Shortcut | Palette command id | Protocol command | Precedent |
|---|---|---|---|
| `Ctrl+B` | `selora.togglePrimarySidebar` | `ToggleSurface{surface: <current activity_view>}` | egui `ToggleFileTree` (`shell.rs:2358`), UI_SPEC §15 |
| `Ctrl+L` *(rebindable)* | `selora.toggleAgentDock` | `ToggleSurface{surface:"agent"}` | egui `ToggleChat` (`settings.rs:78-80`) |
| `Ctrl+J` | `selora.toggleBottomDock` | `ToggleSurface{surface: <current bottom_view>}` | egui `TogglePanel` (`shell.rs:2363`), UI_SPEC §15 |
| `Ctrl+Shift+E` | `selora.showFiles` | `ToggleSurface{surface:"files"}` | new (activity bar is new to the web shell) |
| `Ctrl+Shift+F` | `selora.showSearch` | `ToggleSurface{surface:"search"}` | egui `SearchInFiles` (`shell.rs:2364`) |
| `Ctrl+Shift+G` | `selora.showGit` | `ToggleSurface{surface:"git"}` | new |
| `Ctrl+Shift+M` | `selora.showProblems` | `ToggleSurface{surface:"problems"}` (bottom dock) | VS Code's problems-panel chord; problems is a bottom-dock view (§2.2) |
| `Ctrl+,` | `selora.showSettings` | `ToggleSurface{surface:"settings"}` | egui `OpenSettings` (`shell.rs:2361`) |
| `Ctrl+Shift+P` *(rebindable)* | `selora.openCommandPalette` | none (opens the palette) | `settings.rs:70-72` |
| *(none)* | `selora.showOutput` | `ToggleSurface{surface:"output"}` | — |
| *(none)* | `selora.showTerminal` | `ToggleSurface{surface:"terminal_placeholder"}` | — |
| *(none)* | `selora.saveLayout` | `SaveLayout` | — |

`Esc` closes the palette; with no modal open it dismisses the topmost toast.
`Ctrl+P` (fuzzy find) and `Ctrl+S` (save buffer) are **not bound in Z2**
(their slices own them). No conflicts: the four bindings the egui fixed table
already defines (`Ctrl+B`, `Ctrl+J`, `Ctrl+Comma`, `Ctrl+Shift+F`,
`shell.rs:2357-2370`) are **deliberately mirrored with the same meanings**,
and the additions (`Ctrl+Shift+E/G/M` plus the rebindable pair) are unused
there — no entry in the table carries a different meaning than its egui
counterpart, and nothing collides with WebView2's debug-only devtools chord.

The region toggles deliberately route through §2.2's toggle semantics:
`Ctrl+B` on an open sidebar showing Files closes it; on a closed sidebar it
opens the last active view. That reproduces the egui behavior with zero
region-targeted protocol additions.

Implementation: one `keydown` listener in a `features/shortcuts.ts` hook.
Matching is `ctrlKey` (Windows-first; no `metaKey` mapping, mirroring
`keybind.rs:14-17`), `shiftKey`, `altKey`, and case-insensitive `e.key`.
Rebindable strings are parsed with the same grammar as `Keybind::parse`
(modifier aliases, single key); an unparseable stored string falls back to
the default and logs a console warning — never throws. Shortcuts are
swallowed (`preventDefault`) only when matched, and are inert while a text
input inside a surface has focus, except `Esc` and the palette chord.

### 7.2 Command palette

`Ctrl+Shift+P` opens a `--ink-mid`, 1px `--ink-hi`, `--radius-2` overlay (the
§6-token shapes), `bloom` entrance (`--dur-base`, survives reduced motion per
DESIGN_TOKENS §7.4), listing exactly the eleven commands above with their
shortcut labels. Z2's palette contains **only** shell commands — no file or
symbol modes (those arrive with their slices; do not scaffold them).

### 7.3 Menus

The title bar carries File / Edit / View / Help buttons (brief §7's set). In
Z2 only **View** opens a working dropdown — it lists the same eleven commands.
File/Edit/Help open a dropdown with one disabled row, "Available in a later
slice". No fake menu items.

---

## 8. Empty states, toasts, and the token layer

### 8.1 Empty states (exact placements)

Copy, tone rules, and button/chip styling are DESIGN §6's — the designer's
canonical copy table is the single copy source; ARCHITECTURE does not
maintain a second one. This section pins which rows are live in Z2 and their
liveness boundaries:

- **Editor region** (no workspace — DESIGN §6's editor row): the 64px
  Selora mark (the UI_SPEC §7 splash mark — Z2's always-empty editor is the
  first-run surface), heading "No folder open", italic hint *Open a folder
  to start editing.*, **Open Folder, disabled**, titled
  "Workspace opening lands in a later slice", and **Clone Repo, disabled**,
  carrying its later-slice caption — UI_SPEC §12's second action has no
  `FrontendCommand` variant (`lib.rs:239-326`), none is invented in Z2, and
  it gains wiring only when a protocol command exists (DESIGN §6's row and
  §14.4 record the same rendered-disabled decision) — plus a
  shortcut-chip row (`Ctrl+B`, `Ctrl+L`, `Ctrl+J`, `Ctrl+Shift+P`).
- **Files view**: "No folder open" + the same disabled Open Folder action
  (DESIGN's file-tree-empty-folder row is data-driven, Z4).
- **Search view**: the input renders in Z2 with placeholder *Search the
  workspace*, controls disabled — no search engine until Z4; the results
  rows are the Z4+ product copy (DESIGN §6).
- **Git view**: "Git integration lands in a later slice" (DESIGN's git rows
  are data-driven, Z4).
- **Settings view**: read-only echo of `bootstrap.settings` (theme,
  reduced-motion, the two keybinds) + "Settings editing lands in a later
  slice".
- **Agent dock** (DESIGN §3.5 + §6's agent row): the 40px header with the
  **static 8px `--fg-muted` dot** in the 20px orb slot (`base.css` has no
  orb and none is added — `ThinkingOrb` is Z3), and the empty state with
  heading "Ask anything", italic hint, a `--type-xs` `--fg-faint` status
  line "Agent conversations arrive in a later phase.", the three
  suggestion chips in disabled styling, and the composer card visual with
  its placeholder — **all rendered, all inert**: no key, chip, or Enter
  press dispatches a command until Z3 (`SendChat`/`SendAgent` are
  unsupported; Z3 wires the chips, composer, and beam per DESIGN §6's
  agent row; the beam mount point stays unmounted). No Libraries.dev
  package is used for any of this.
- **Bottom dock — Problems tab**: "Diagnostics land with the editor slice"
  (DESIGN's "No problems." row activates when diagnostics data exists, Z4).
  **Output**: "Nothing has run yet." **TerminalPlaceholder**: heading
  "Terminal", "The integrated terminal arrives in a later phase. Commands
  the agent runs appear in Output." (brief §7's clearly-labeled
  placeholder; DESIGN §6's terminal row is the copy source).
- **Status bar** (24px, never empty): left = workspace display name
  (`--fg-muted`, `--type-xs`); center = empty in Z2 (editor metadata lands
  with the editor slice); right = the UI_SPEC §1.2 disconnected form
  "○ selora disconnected · connect" (the word *connect* inert in Z2 —
  settings editing is a later slice) or the connected form when
  `bootstrap.connection` says so.

### 8.2 Toast surface

Stack bottom-right, 12px from the right and bottom edges, above the status
bar; 8px gaps; max 3 visible (older ones fade); `--ink-mid` background, 1px
`--ink-hi` border, `--radius-1`, 12px padding (UI_SPEC §13). Auto-dismiss:
**4s for `info`/`success`, 8s for `warning`/`error`** (§13 specifies
4s info / 8s error; the band assignment for success/warning is a documented
Z2 extension). Click × dismisses. Enter/exit animation is `fade`
(`--dur-base`) only — `fade` survives reduced motion (DESIGN_TOKENS §7.4); no
slide, no spring. Frontend keys toasts by the event's `sequence` (`ToastDto`
carries no id, `lib.rs:546-550`); locally generated command-error toasts use
negative local ids. The store is a small `features/toasts/` module; it does
not persist anything.

### 8.3 CSS token layer (ownership boundary)

- `apps/selora-desktop/frontend/src/styles/tokens.css` — **designer-owned**:
  every DESIGN_TOKENS value as CSS custom properties (ink ramp, fg ramp,
  ok/warn/err/info, syntax ramp, scrim, 4px spacing steps, radii, type scale,
  weights, tracking, hairlines, motion durations incl. `--dur-lap: 2400ms`,
  easings) plus the §1 shell chrome variables.
- `apps/selora-desktop/frontend/src/styles/base.css` — **designer-owned**:
  reset, body/root colors, `@font-face`, `:focus-visible` ring, scrollbar
  styling, and the reduced-motion strategy (`prefers-reduced-motion` media
  query **and** a `[data-reduced-motion]` root attribute driven from
  `bootstrap.settings.reduced_motion` — effective reduced motion is the OR of
  the two).
- The frontend developer **imports both, never edits them**; a missing token
  is reported as a deviation, never patched over with a literal. Import
  convention (matching DESIGN §13.1 and the delivered CSS): `main.tsx`
  imports `'./styles/base.css'` — which `@import`s `'./tokens.css'`
  (`base.css:12`) so the variables always precede their first use —
  followed by component stylesheets. The Z1 `src/styles.css` is consumed by
  this split and deleted — its hardcoded literals are exactly what the
  token layer replaces.
- Both files are delivered by the designer's parallel Z2 task and were
  verified against this section at contract time: `tokens.css` carries the
  chrome tokens under the names in §1; `base.css` carries the `@font-face`
  blocks with `font-display: swap`, the `[data-reduced-motion]` attribute
  strategy, and the `prefers-reduced-motion` media query. The TTFs and OFL
  licenses are copied to `public/fonts/` (§6.2).

Component CSS (frontend-owned) uses grid/flex with the token variables only —
no gradients, no shadows, no saturated accents, no durations under 80ms or
over 2000ms (the 2400ms lap token excepted), no emoji (DESIGN_TOKENS §12).

---

## 9. File-ownership map

**Rust developer owns** (all paths repo-relative):

- `crates/selora-protocol/src/lib.rs` — §1.1 constants, `LayoutDto` field
  additions + serde defaults + `sanitized()`, `LayoutConstraintsDto`,
  `clamp_region_size`, `ACTIVITY_VIEWS`/`BOTTOM_VIEWS`, the new
  `ProtocolValidationError::InvalidRegionSize`, `SettingsViewDto` fields,
  `BootstrapState.layout_constraints`; protocol tests (§10).
- `crates/selora-settings/src/settings.rs` (+ `lib.rs` re-export,
  `Cargo.toml` dep on `selora-protocol`) — §4.2 `LayoutSettings`, defaults,
  round-trip and old-file tests.
- `apps/selora-desktop/src-tauri/src/state.rs` — layout state, `LayoutStore`
  seam, dispatch arms for the three commands, close-time persist, event
  retention cap, boot toast.
- `apps/selora-desktop/src-tauri/src/commands.rs` — `get_events_after`;
  `dispatch_command` gains `app: tauri::AppHandle`.
- `apps/selora-desktop/src-tauri/src/main.rs` — `.decorations(false)`,
  `.on_window_event(CloseRequested → persist)`, register `get_events_after`,
  title "Selora IDE".
- `apps/selora-desktop/src-tauri/tauri.conf.json` — `"decorations": false`,
  title. CSP unchanged (§6.2).
- `apps/selora-desktop/src-tauri/capabilities/default.json` — §6.3's six
  additions.
- `apps/selora-desktop/src-tauri/src/security.rs`, `events.rs`, `adapter.rs`
  — unchanged in Z2 (owned, untouched: `security.rs`'s guards and
  `adapter.rs`'s `unsupported_phase_slice` stay exactly as they are).

**Frontend developer owns**:

- `apps/selora-desktop/frontend/src/bridge.ts` and `bridge.test.ts` — §2.5
  extensions; wire-shape tests asserting the exact §2.1 JSON strings.
- `apps/selora-desktop/frontend/src/main.tsx` — reduced to style imports +
  bootstrap render.
- `apps/selora-desktop/frontend/src/app/` — bootstrap/replay orchestration
  (§3.4), the single app store (`useSyncExternalStore`; no state library).
- `apps/selora-desktop/frontend/src/layout/` — `ShellGrid`, `TitleBar`,
  `ActivityBar`, `PrimarySidebar`, `EditorRegion`, `AgentDock`, `BottomDock`,
  `StatusBar`, `RegionHandle`, shared `EmptyState`.
- `apps/selora-desktop/frontend/src/features/` — `shortcuts.ts`, `palette/`,
  `toasts/`.
- `apps/selora-desktop/frontend/src/components/` — the inline-SVG `Icon` set
  (16×16, 1.5px stroke, `currentColor`, per DESIGN_TOKENS §8; Z2 subset:
  file, search, git-branch, error, settings, chat, close, check, command).
- Component stylesheets (e.g. `src/styles/shell.css`) and
  `public/fonts/` consumption. May add npm **dev**Dependencies for tests;
  no runtime dependencies. Never touches `src-tauri/**`, `crates/**`.

**Designer already owns** (frontend imports, never edits):

- `apps/selora-desktop/frontend/src/styles/tokens.css`
- `apps/selora-desktop/frontend/src/styles/base.css`
- `apps/selora-desktop/frontend/public/fonts/` (mechanical copies of
  `assets/fonts/*.ttf` + OFL license files, per §6.2)

---

## 10. Test contract for Z2

Rust (run with the workspace suite — brief §13):

- protocol: `LayoutDto` round-trip incl. the two new fields; a pre-Z2 layout
  JSON (seven fields) deserializes to the §4.2 defaults; `clamp_region_size`
  bounds table + non-finite rejection; `sanitized()` fixes out-of-set
  `activity_view`/`bottom_view`, out-of-range sizes, and `editor_splits: 0`;
  `LayoutConstraintsDto` matches the constants.
- settings: `layout` section round-trip; the ARCHITECTURE.md §8.1 example
  file (no `layout` key) loads unchanged with defaults; a save preserves all
  other sections; **`LayoutSettings::default()` equals the §1.1 values
  exactly** (240 / 360-closed / 200-closed) — the divergence guard between
  the serde defaults, the `Default` impl, and the §1.1 constants
  (tokens.css intentionally carries no region values).
- desktop state: §2.2 toggle table (each surface × open/closed ×
  same/different view); resize clamp + no-op-no-event; envelope rejection for
  the three new commands (stale core/workspace/session); persist-on-close
  function; the 1024-event retention cap.

Frontend (`vitest`):

- bridge wire-shape tests asserting the exact §2.1 command/event JSON.
- gate tests: stale dropped; gap/epoch/version → rebootstrap path; the §3.4
  null-seeded replay — an event with `sequence ≤ snapshot_sequence` (the
  boot toast) applies when the gate is seeded `null` and is dropped as
  `'stale'` when seeded with `S` (the negative case, proving the seed choice
  matters).
- cadence test: synthesized pointer sequence on `RegionHandle` → zero
  `invoke` during move, exactly `[ResizeRegion, SaveLayout]` (in that order)
  on up; the same single pair for each §3.1 keyboard completion (Arrow ±4px,
  Shift+Arrow ±16px, Home/End, double-click reset).
- palette registry test: every `SurfaceId` appears in ≥1 command, ids unique,
  shortcut labels match §7.1; rebindable-string parser mirrors
  `Keybind::parse` grammar incl. garbage fallback.
- toast store: cap 3, 4s/8s timers, manual dismiss.

Manual acceptance (Z2 additions to the brief §13 walkthrough): resize each
dock, restart, sizes restored; toggle each surface, restart, open/active
state restored; first run shows the §1.1 defaults (sidebar open at 240, agent
dock and bottom dock closed); double-click a handle and confirm the reset
lands on the same default; enable reduced motion, loops freeze; **run a
production build and confirm drag-resize works under the production CSP**
(§6.2); point `SELORA_SETTINGS_DIR` at a `_v: 2` file and confirm the wipe
toast appears via replay.

---

## 11. Documented decisions and deviations (recap)

1. Global layout in Z2; per-workspace keying pinned for the workspace-picker
   slice (§4.1, §4.5) — deviation from brief §6's "per workspace" wording,
   owner-authorized by the Z2 tasking.
2. Custom in-webview title bar per brief §2/§7 supersedes UI_SPEC §1.1's OS
   chrome for the Tauri shell only; the egui shell is unchanged (§6.3).
3. Close-time save runs in Rust on `CloseRequested`, not over IPC (§3.3).
4. Toast timing extended: success→4s band, warning→8s band (§8.2).
5. `problems` routes to the bottom dock in Z2 (UI_SPEC §2's panel ④, brief
   §7, DESIGN §3.6; brief §2's sidebar parenthetical superseded; DESIGN
   §14.2's open question resolved to one surface) (§2.2).
6. Region defaults are unified at 240 / 360-closed / 200-closed (UI_SPEC
   §2, the egui settings/persistence defaults; tokens.css carries no region
   values by design — Rust via bootstrap is the single source); the Z1 stub
   values 280/220/agent-open in `state.rs:68-76` and the protocol test
   fixture change in this slice. A new `layout.*` settings section is
   used instead of the egui app's
   `ui.sidebar_width`/`ui.chat_sidebar_width` (legacy keys untouched;
   reconciliation at cutover) (§1.1, §4.1).
7. `protocol_version` stays 1 (§5.2).
8. Handle geometry, hit zones, the workbench grid, and keyboard resize steps
   follow DESIGN §1/§4 verbatim (6px handle tracks; Arrow ±4px,
   Shift+Arrow ±16px, Home/End, double-click reset) — one step contract, not
   two (§1, §3.1).
9. Empty-state copy is DESIGN §6's table with Z2-liveness boundaries pinned
   here (§8.1); the replay gate is seeded `null`, never
   `snapshot_sequence` (§3.4).
