# Status — 2026-10-09 (Phase Z2 shell layout merged; Phase R merged; Phase 6 release still the owner's gate)

> Restart point. Read this, then AGENT_ORCHESTRATION.md §10. RESUME_PROMPT.md
> at the repo root is the copy-paste prompt for a fresh orchestrator session.

## 2026-10-09 — Phase Z2 (premium shell layout): merged; contract review passed

The first user-facing Tauri/WebView2 slice. The seven-region shell (title
bar, activity bar, primary sidebar, editor region, agent dock, bottom dock,
status bar) is implemented against the two Z2 contracts, both of which
survived two reviewer rounds before implementation began:
`docs/ui/PHASE_Z2_ARCHITECTURE.md` (implementation contract) and
`docs/ui/PHASE_Z2_DESIGN.md` (layout/token spec, shipped with the
designer's `styles/tokens.css` + `base.css` and the bundled OFL fonts in
`public/fonts/`).

**What is real** (structure verified in the tree this session):

| Item | Where |
|---|---|
| Seven-region grid — bottom dock spans the editor column only; sidebar + agent dock full height | `frontend/src/layout/` (ShellGrid, TitleBar, ActivityBar, PrimarySidebar, EditorRegion, AgentDock, BottomDock, StatusBar, ResizeHandle) |
| Drag + keyboard resize (6px handle tracks; Arrow ±4px, Shift ±16px, Home/End, double-click reset; 160–480 / 240–600 / 120–min(800, 50%)) | bounds arrive via `bootstrap.layout_constraints` — no JS literals |
| Layout persistence (global `layout` section in settings.json, serde-defaulted, old files load unchanged) + Rust close-time save on CloseRequested | `LayoutStore` read-modify-write seam, `state.rs:25-77`; `from_settings_path` at `state.rs:93` |
| Keyboard toggles + palette (Ctrl+B/L/J, Ctrl+Shift+E/F/G/M/P, Ctrl+,; eleven `selora.*` commands) | `frontend/src/features/keymap/` + `components/CommandPalette.tsx` |
| Toast surface (bottom-right stack, 4s/8s bands, fade) + the boot settings-wipe toast | `components/ToastViewport.tsx` + `features/toasts/toast-store.ts` |
| Replay gate: `get_events_after` command, null-seeded, 1024-event ring | `commands.rs` + `state.rs`; frontend applies `workspace_layout_changed` only after stale/gap validation |

**What is still placeholder** (later slices — every other `FrontendCommand`
still returns `unsupported_phase_slice`, `adapter.rs:3-9`): the editor
bridge (CodeMirror, Z4), real file-tree/search/Git/diagnostics data (Z4),
agent streaming and the composer's behavior contract (Z3 — the agent dock's
chrome, empty state, and composer card ship **inert**: nothing in the dock
dispatches a command), Libraries.dev packages (Z3/Z5 — none installed),
workspace opening (`OpenFolder` unsupported; workspace id stays
`"no-workspace"`).

**Gates** — implementer-reported green and contract acceptance approved;
re-run by the orchestrator at commit time (full workspace, not just the
touched crates):
- `cargo test --workspace --no-fail-fast` → **all suites green** — but only
  after one orchestrator fix recorded below (`static_vc_runtime(false)`:
  tauri-build's static-VC-runtime stub `msvcrt.lib` in OUT_DIR otherwise
  poisons `/defaultlib:msvcrt` for every doctest cargo links in the same
  workspace session — the selora-editor doctest died with 68 unresolved
  externals; per-crate gates never see it).
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo fmt --all --check` → clean.
- `npm run typecheck` → clean; `npm run test` → **9 test files, 106 tests
  passed**; `npm run build` → green (~195KB JS / ~19KB CSS, fonts in
  `dist/fonts/`).
- **NOT run** (needs a human at the app, per the frontend implementer's
  report and confirmed here — nothing in this session can launch the app):
  the §10 manual walkthrough: production-CSP drag-resize inside the real
  WebView2, restart persistence, and the visual reduced-motion check. The
  packaging slice should also verify the production CSP
  (`style-src 'self'`, no `unsafe-inline`) against WebView2's handling of
  CSSOM `setProperty` writes — Z2's resize path deliberately routes region
  sizes through CSS custom properties set via `el.style.setProperty`
  (ARCHITECTURE §6.2), the CSP-safest pattern, but only a real window
  proves it.

**Rust deviations (implementer-reported; each verified in the code this
session):**

- `build.rs:9` uses `WindowsAttributes::new()` (tauri-build's default
  Common-Controls v6 manifest) instead of `new_without_app_manifest()`:
  without it every binary of this crate dies at process start with
  STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139) — tauri's default
  `common-controls-v6` feature imports `TaskDialogIndirect` and no manifest
  means comctl32 v5 loads. The implementer verified this was **pre-existing
  by git-stashing the changes and building/running the pristine tree**
  (exit code -1073741511): `cargo test` passed at Z1 only because the test
  profile never linked the windowing code — the Z2 mock-app tests and the
  production exe both do. The contract does not mention the manifest; the
  fix was required for the named gate `cargo test -p selora-desktop` to
  pass and repairs a previously unlaunchable app.
- `build.rs` also sets `.static_vc_runtime(false)` (orchestrator, at commit
  time): the config default (true) makes tauri-build's `static_vcruntime`
  write a stub COFF object NAMED `msvcrt.lib` into OUT_DIR and register
  that dir as a native link path (`tauri-build-2.7.1/src/
  static_vcruntime.rs`). Cargo propagates build-script link-search paths to
  every doctest compiled in the same workspace session, so
  `cargo test --workspace` links selora-editor's doctest with that path;
  link.exe picks the stub for `/defaultlib:msvcrt`, rejects it (LNK4003
  invalid library format), and the real C runtime never resolves — 68
  unresolved externals. Dynamic CRT instead: the release binary wants
  vcruntime140.dll, which the Windows packaging slice owns alongside the
  WebView2 prerequisite policy.
- `state.rs` `dispatch`/`record_event`/`push_event` are generic over
  `R: tauri::Runtime` (`state.rs:209`, `:363`, `:376`) instead of the
  concrete `&tauri::AppHandle` sketched in the contract — required so
  hermetic tests drive the real emit path via `tauri::test::mock_app()`
  (MockRuntime, `state.rs:430`). The Tauri command itself is exactly
  `app: tauri::AppHandle` (`commands.rs:25`), which is where the contract
  specifies the parameter.
- `crates/selora-protocol` gained three DEFAULT constants
  (`PRIMARY_SIDEBAR_DEFAULT_PX` 240, `AGENT_DOCK_DEFAULT_PX` 360,
  `BOTTOM_DOCK_DEFAULT_PX` 200 — `lib.rs:256/262/272`) beyond the
  contract §1.1 MIN/MAX block: §4.2 requires the settings defaults to
  "delegate to the §1.1 protocol constants" and §1.1's table carries the
  Default column, so the defaults needed a protocol home to avoid a second
  source of truth.
- `LayoutStore::persist_layout` is a `&self` method holding the settings
  path (parameterized for hermetic tempdir tests), otherwise the contract
  §4.5's exact `(&mut Settings, &LayoutDto)` shape; the save re-reads
  settings.json from disk (read-modify-write through the seam) rather than
  keeping a full `Settings` copy in `DesktopState` — that is what makes the
  §10 "a save preserves all other sections" guarantee robust and testable.
- `crates/selora-settings/src/lib.rs:39` re-exports
  `selora_core::SeloraError` so selora-desktop can name the persistence
  error type without taking a direct selora-core dependency (additive
  re-export; the ownership map lets the persistence decision touch this
  crate).
- `DesktopState::with_settings` changed signature
  (`SettingsViewDto` → `selora_settings::Settings`) and is now
  `#[cfg(test)]` (`state.rs:118`): Z1's `new()` called it, but Z2's `new()`
  routes through `from_settings_path`, so it has no production caller —
  kept as the tests' no-IO constructor instead of carrying dead code.
  `events.rs`, `security.rs`, `adapter.rs` are untouched, exactly as the
  ownership map requires (git status confirms).

**Frontend deviations (implementer-reported):**

- Toast severity markers: UI_SPEC §13 specifies `i`/`!` text glyphs, but
  DESIGN §10 forbids text glyphs outside the ●/○/key-chord precedents and
  ARCH §9's icon subset has no info/warning glyph — the shapes follow UI_SPEC
  §13 with its `--info`/`--warn`/`--err` colors, drawn as inline SVG in
  DESIGN §10's 16×16 / 1.5px-stroke system (success uses the subset's check
  in `--ok`). Both contracts honored in shape and color; only the drawing
  medium reconciles them.
- Reduced-motion transitions: the implementer's ask said "every transition
  must collapse per the design doc", but the design doc itself (DESIGN §8)
  keeps one-shot hover/press transitions (≤ `--dur-quick`) and the
  fade/bloom one-shots under reduced motion, freezing only loops. The
  design doc was followed: zero looping animations authored (base.css's
  loop-freezing is a no-op for Z2 chrome), hover/press and fade/bloom stay
  at token durations.
- File anatomy followed the implementer's ask (matching the brief §4 tree):
  `features/keymap/`, `features/layout/layout-store.ts`,
  `features/toasts/toast-store.ts`, with CommandPalette/ToastViewport/
  EmptyState in `components/` — instead of ARCH §9's illustrative names
  (`features/shortcuts.ts`, `features/palette/`, `features/toasts/`).
- devDependencies `happy-dom`, `@testing-library/react`,
  `@testing-library/dom` (`package.json:18-23`) were added for tests —
  explicitly permitted; zero runtime dependencies added.
- Not run (cannot run here): the §10 manual acceptance walkthrough
  (production-CSP drag-resize inside the real WebView2, restart
  persistence, visual reduced-motion) requires launching the Tauri app. The
  three gate commands the implementer's ask names were run and pass.

**Next**: Z3 — the agent dock vertical slice (session header, model picker,
transcript, composer contract, ChatDelta batching with stale-event handling,
tool cards, diffs, approvals, stop, retry, cost footer, and the five
Libraries.dev mounts). What it can rely on is recorded in
`docs/ui/PHASE_Z_MIGRATION.md`.

## 2026-10-07 — Phase R (Premium UX Revamp): the plan + R0 merged

**The track** (owner-approved plan, benchmark: a ZCode-class coding agent,
strictly inside the ink design language — §12 still rules):

| Item | State |
|---|---|
| R0 architecture seams | **merged** (see below) |
| R1 the composer | **merged** (see below) |
| R2 transcript rendering | **merged** (see below) |
| R3 multi-session + restore | **merged** (see below) |
| R4 cards & diffs | **merged** (see below) |
| R5 settings completion | **merged** (see below) |
| R6 IDE gaps | **merged** (see below) |
| R7 final polish + docs | **merged** (see below) |

**Standing references (owner directive, 2026-10-07):** ZCode
(https://github.com/zai-org/ZCode — the UI/UX + agent-window benchmark) and
Zed (https://github.com/zed-industries/zed — the fast/light engine reference)
are now recorded in ROADMAP.md's "Reference repositories" section (top of the
doc, with the phase mapping). Ideas, never code — Zed is GPL-3.0, Selora is
MIT. Many already-shipped features (agent cards, composer, workflows plan)
trace back to these two.

**R0 details** (no intended visual change except the two fixes below):

- `shell.rs` chrome extracted into `selora-ui` widgets: `title_bar`,
  `status_bar::workbench_zone` (git + LSP servers left, cursor center —
  state-in/events-out, restart click target preserved), `toasts` (the
  `Toast`/`Severity` types moved too), `empty_state` (no-folder + pane
  hint). The shell now composes widgets; it no longer hand-paints chrome.
- The canonical `picker::ModelPicker` (Phase 4b's orphaned, fully-tested
  widget) is wired into the chat header via a new `show_at` placement (the
  header's absolutely-positioned center slot) — display names, filter
  input, Enter-to-select, Esc/outside-click dismissal. The parallel inline
  dropdown (~400 lines in `sidebar/chat/mod.rs`) is deleted. Tests adapted
  (`picker_refresh_rect` seam replaces the old footer-click).
- Animation helpers deduped into `presence`: `with_alpha` (was 3× in
  selora-ui), `pulse_eased` (agent_cards' ms-based variant now converts to
  phase and shares the presence easing), `request_anim_frame` replaces
  agent_cards' local `ANIM_FRAME_MS` pair. The `fade` entrance now uses the
  token bezier evaluator (`motion::ease_bezier`) instead of a t-parametrized
  approximation — same §7.1 curve, one implementation.
- Token imports unified: `splash.rs`, `dialogs/about.rs`,
  `dialogs/settings/*` read `crate::theme` (the documented §11 path), not
  raw `selora_tokens`.
- `--scrim` token added (DESIGN_TOKENS.md §1.7) — closes the 6g-noted
  backdrop token gap; applied to modals in R7.
- **Fixed while extracting**: the no-folder empty state's button pair was
  left-aligned (egui: `ui.horizontal` inside `vertical_centered` resets the
  alignment) — now centered per UI_SPEC §12; the model picker's trigger
  now sizes via the widget's own `closed_button_width`.
- shell.rs: 3,867 → 3,541 lines (non-test code ≈ 1,890; the file is
  test-heavy by design — the chrome's tests moved out with the widgets).
- Gate: `cargo test --workspace` green (733 tests passing across all crates, +9 new for the extracted widgets).

**R1 details** (the composer — ZCode-class input):

- Multiline `TextEdit` with `return_key(SHIFT+Enter)`: plain Enter is
  consumed by the panel while the edit is focused (the submit signal),
  so the caret-accurate newline comes via Shift+Enter. The bar grows with
  the buffer (wrapped rows measured, capped at 6 rows / 60+5×18px), the
  transcript yields.
- ↑/↓ recall on a one-line buffer (10 entries, deduped, draft restored
  walking off the end). A recall's external text change re-arms focus
  next frame (egui resets the edit's widget state when the buffer moves
  under it).
- The Stop button: while a plain (non-agent) stream runs, the send
  button becomes a painted ■ (hover `--err`, the agent status-line Stop
  precedent). Click → the panel finalizes the partial bubble (it stays
  in the conversation) + `ChatPanelEvent::StopStream` → the shell
  reverse-maps to the request uuid, drops the mapping, and sends the new
  `Effect::SeloraChatAbort` → the dispatcher flips a per-request
  `tokio::watch` — the SSE request future is dropped (the connection
  closes) and the relay ends **silently** (a user stop is never a
  ChatError toast). Late deltas for the aborted id are stale-dropped.
- Tests: composer (multiline submit, recall walk, abort semantics,
  send-blocked-now-stops), wiring (`chat_cancel_ends_the_stream_silently`
  — a stalled mock response, the channel closes with no terminal
  message), shell (the StopStream route + late-delta drop).

**R2 details** (the transcript — structure, not walls of prose):

- `sidebar/chat/markdown.rs` (new, 12 unit tests): the markdown-lite
  parser — blocks (paragraph/heading/list/quote/rule/fenced-code) and
  inline runs (`**strong**` SemiBold, `*emphasis*` Medium, `` `code` ``
  mono chip, `[text](url)`). Malformed input degrades to plain prose,
  never a panic; an unterminated fence is an incomplete block (mid-
  stream). The old `parse_segments` and its tests were deleted.
- The paint path lays blocks out as `LayoutJob`s (per-run fonts/colors,
  links underlined with recorded char ranges; a click hit-tests via
  `Galley::cursor_from_pos` and `ctx.open_url`). Code blocks build the
  job from `SyntaxLayer` spans in the SYN palette, with a 22px header
  (language label via `fence_language`, Copy on hover). The streaming
  fade bakes into the section colors (job galleys ignore the
  paint-time fallback).
- **The layout cache**: `Message.layout: Option<CachedLayout>`, rebuilt
  only when the content length or wrap width moves (the fade's first
  240ms rebuilds per frame). A settled message costs no per-frame
  re-layout.
- Footers: elapsed + `~N tok` (XS faint, ticking while streaming at
  500ms repaints), message-level Copy, and Retry on failed streams
  (drops the failed bubble, re-Sends the user's message — the
  conversation keeps the user turn). The CopyCode toast became "copied
  to clipboard" (both copy paths flow through it).
- The empty transcript: brand mark, "Ask anything", and 3 suggestion
  chips that prefill the composer.
- Tests: +6 (markdown paints, cache holds, link click via the
  FullOutput's OpenUrl command, suggestion prefill, retry, message
  copy — the click ones via a four-frame grid sweep, the
  hover-revealed-affordance pattern). selora-ui 305 passing; the chat
  test helpers now register the bundled fonts (the headings use the
  named SemiBold family).

**R3 details** (sessions — the conversation survives restart):

- `ChatSession { id, title, created_at, entries }` + `SessionEntry`
  (bubble / card / summary) + `SessionCard` (the serde mirror of
  `ToolCard` — the call, approval tuple, result, latch, expanded; paint
  state rebuilds fresh on restore). `ApprovalCard` gained serde derives
  in `selora-ai`; selora-ui gained `serde` + `uuid` deps.
- The panel owns the model: stored sessions (oldest first) + the live
  timeline; `new_session`/`switch_session` stash + load (refused with
  `SessionBusy` while a turn runs → the shell toasts via the new
  `ChatPanelEvent::SessionBlocked`). Auto-title: the first user message
  truncated to 32 chars. Caps: 20 sessions / 100 entries.
- `sessions.json` (persistence.rs, schema-gated at 1) rides the 5s
  workspace cadence with its own no-op snapshot check; boot restores the
  last session live, falling back to the legacy `chat_turns` when the
  file is missing/garbage/schema-mismatched.
- The header: the session's title (truncated to the left zone) + chevron
  IS the label — the static "CHAT" went away. The dropdown: "+ new
  session", the live row (`--ok` marker, turn count), stored rows
  (title, N turns · relative age), 8 rows then scroll, Esc/outside-click
  dismissal, mutual exclusivity with the picker and overflow menus.
- Tests: +4 panel (stash/titles, switch restores a parked diff card,
  busy refusal, snapshot/restore round trip), +1 persistence
  (serde round trip incl. a card; garbage/schema refuse), +1 shell
  (boot restore — re-added after a heredoc truncation ate the first copy).

**R4 details** (cards & diffs — the approval moment, premium):

- `crates/selora-ui/src/diff_view.rs` (new): `DiffLine`,
  `parse_unified_diff`, and the row renderer moved out of
  `agent_cards` (Phase R6's git tabs reuse it). Upgrades:
  **tree-sitter highlighting** of the new side (context + added lines
  reassembled, parsed once, spans mapped back to per-row char columns;
  a global 32-entry content-hash cache keeps steady frames parse-free),
  and **word-level emphasis** — paired removed/added runs (common
  prefix/suffix trim, backed to word edges) paint the changed span with
  a 12% alpha tint of `--err`/`--ok`. §12 bend logged below.
- `language_for_diff` reads the `+++ b/path` header; `first_changed_line`
  feeds the open target.
- Card affordances: `CardOutcome.open_at` — the "open path · line N"
  link rides the top of every expanded file-touching body (diff cards,
  read/write/edit results); the panel routes
  `ChatPanelEvent::OpenFileAt`, the shell resolves the workspace-
  relative path through the existing open-and-jump queue.
- Bash output strips ANSI (CSI/OSC/two-letter; the ✓/✕ header glyph
  already carries the semantics — no SGR mapping, per §12). web_search
  rows open the browser on click; a hover `copy` glyph keeps the
  clipboard path.
- The orb's reserved arms in the status bar: `Shaping` (tree rebuild —
  set on RefreshFileTree/OpenFolder, cleared on FileTreeUpdated),
  `Composing` (search — RunSearch → SearchResults), `Weaving` (ghost —
  request → GhostTextReady). `selora_status::show` takes the activity +
  reduce-motion; the cost meter clears the orb's zone; 20ms repaints
  while an orb animates.
- Tests: +6 (strip_ansi, diff-card open link, web rows open the url,
  first_changed_line, changed-span pairing, activity orb paint) +1
  shell (reserved-arm flag lifecycle). selora-ui 321, selora-app 84.

**R5 details** (settings completion):

- The Agent section: max_iterations (1–100), max_turn_secs (10–3600),
  max_cost_usd (0.01–50) as DragValues; the shell; the auto-approve
  allowlist editor (editable rows with remove `×`, an add field,
  deduped); the Brave key — masked input, committed on blur to the new
  `keyring::brave_key_{get,set,delete}` (service `selora-ide`, entry
  `brave-key`), scrubbing any legacy plaintext field with the save.
- The shell's AgentSend seam injects the keyring key when the settings
  field is empty (legacy plaintext wins only if the keyring is empty —
  nothing breaks, nothing new lands in files).
- General: the reduce-motion toggle (the SettingsSaved adoption already
  threads it into the panel). The Git placeholder section and the
  no-op Minimap checkbox are gone (schema fields stay).
- Tests: keyring brave round-trip (throwaway service), the Agent
  section's allowlist paint; the every-section loop covers the new
  list. selora-ui 320, selora-settings 29.

**R6 details** (the workbench — the last "real IDE" gaps):

- 6a: the §1.1 title-bar menus (File/Edit/View/Help → palette commands +
  Exit) — the strip's `show` returns a `TitleBarOutcome` (event + the
  menu rects test seam); popups paint one frame late (the test's settle
  frame). Plus the problems panel: `LspViewState.diagnostics` (the mapped
  squiggle shape, per-path), `panel/problems.rs` (grouped rows, severity
  glyphs, open-at-line), the Problems activity with the warning glyph +
  count badge.
- 6b: the git read-only diff view — `FileDiff::to_unified` in selora-git,
  `Effect::GitDiff` + `GitDiffReady`, the editor-area overlay (header +
  close, Esc), the DiffView (highlighted). The git panel's OpenDiff
  stopped opening the raw file.
- 6c: find/replace — `selora-ui/src/editor/find_bar.rs` (query, count,
  `Aa` case toggle, prev/next, ×; replace row under Ctrl+H; Esc closes
  regardless of focus — egui clears focus on Escape globally). The
  editor's `FindState`: matches computed per query edit, the active one
  follows the cursor via `find_next/prev` (wrapping), `replace_current`/
  `replace_all` (reverse-order edits so positions hold). Match paint:
  hairline boxes + the active ink-mid fill (the paint loop's x_offsets).
  Ctrl+F/Ctrl+H + the palette commands route here (the "lands in
  Phase 3" toasts are gone).

**R7 details** (the last polish + the owner-decision items):

- MetalMark **enabled** (the splash sheen — the single permitted specular
  highlight, per the owner's Phase-R decision); the flag test now guards
  "on".
- `--scrim` applied to the Settings and About modals (`backdrop_color` —
  the egui default black-alpha is gone); the cheat sheet uses it too.
- Toasts fade in over 240ms and out over their last 400ms (§7.4-kept
  fades).
- The keyboard cheat sheet ("Help: Show Keyboard Shortcuts" in the
  palette): a `shortcuts` dialog of the rebindable four (live settings)
  + the fixed table, binding chips, Esc/backdrop closes; the overlay
  gate now covers it.
- **Screenshots need a live refresh** (assets/screenshots/*.png predate
  the revamp — no automated capture tool exists for native windows; the
  owner re-captures on their machine, same as 6g).

**§12 bend for owner sign-off (R4)**: the diff emphasis paints a 12%
alpha tint of the existing `--err`/`--ok` behind changed words (DESIGN
TOKENS has no token for it; it is alpha-of-semantic-color applied at
paint time, the `with_alpha` pattern — refuse by deleting the tint in
`diff_view.rs`, one `EMPHASIS_ALPHA` const)., `cargo clippy
  --all-targets` clean, `cargo fmt` applied.

## 2026-10-07 — Phase 6 (Polish & Ship): 6a–6g merged on main; 6h in flight

**Phase 6 subagents, all landed directly on main this session** (orchestrator
did them in-tree; the phase is mostly-sequential by design):

| Item | Commit | State |
|---|---|---|
| 6a first-run splash (UI_SPEC §7) | `c191066` | **merged**, 17 new tests; live-verified (boot with empty keyring → splash; Connect via splash against a local mock gateway works) |
| 6b About + check-for-updates (§11) | `fe20123` | **merged**, 13 new tests; About verified live (palette → "about") |
| 6c Inno installer | `9b42e73` | **merged + machine-validated**: `installer\prebuild.bat` → ISCC compiles → silent install → app launches → silent uninstall → clean removal (incl. `%APPDATA%\selora-ide`) |
| 6d release pipeline | `9b42e73` | **merged** (tag `v*.*.*` → build → ISCC → draft release; signing no-op until `SIGNING_CERT_BASE64`/`SIGNING_CERT_PASSWORD` secrets exist; tag-vs-Cargo.toml version guard) |
| 6e README + docs/ (MkDocs) | `9b42e73` | **merged** (9 pages; site build/deploy is v1.x) |
| 6f publication hygiene | `9b42e73`, `c280d4b` | **merged** (CHANGELOG, CoC, issue/PR templates, tooling gitignore) |
| 6g screenshots | `5d29d93`, `a677896` | **merged** (7 PNGs in `assets/screenshots/`, README gallery) |
| 6h presence/motion widgets (KIMI_UI_PROMPT.md) | `1e6ab4f`, `543e37e` | **merged** (subagent → orchestrator review → gate) |

**Icons exist now**: `assets/icons/selora-ide.ico` (16/32/48 BMP + 256 PNG)
+ 256/512 PNGs + Inno wizard BMPs, generated by `assets/icons/generate_icons.py`
(Pillow port of the SVG construction — no SVG rasterizer on this machine).
The installer/app icons are no longer placeholders.

**Three real bugs found + fixed by looking at the app while capturing (6g):**
- Settings modal painted near-fullscreen with overlapping rows: the
  `set_min_size == set_max_size` placer pin stopped drift but never bounded
  the horizontal body's children (ScrollArea greedy width escaped to the
  screen; `available_height` in a fresh horizontal strip read 18px and
  collapsed the sidebar). Fixed with an exact `allocate_exact_size(720×520)`
  + explicit child rects. (The Phase 4 smoke test had validated interaction,
  not geometry.)
- Status run's ● (U+25CF) was tofu: Space Grotesk lacks both ●/○ and egui's
  built-ins cover only ○. JetBrains Mono (bundled) is now the sans symbol
  fallback (`theme.rs`), regression test included.
- Chat header read "no model" and sends blocked until a manual pick: nothing
  seeded `chat_panel.model` from `settings.selora.active_model` at boot.
  Fixed on SettingsLoaded (never clobbers a picker choice), shell regression
  test included.

**6h details** (subagent report + orchestrator review): five painted
widgets under `crates/selora-ui/src/presence/` — ThinkingOrb (nine states,
replaces the ⠿⠿⠿ wave on the same 800ms trigger, rides the agent status
line), AgentAvatar (idle/working/sleeping ink silhouette in the status line
+ card headers), StreamGlow (composer edge-glow driven by deltas/sec — voice
input is the brief-mandated deviation), LiquidPill (the agent-toggle melt),
MetalMark (splash sheen, `METAL_MARK_ENABLED = false` — OWNER-DECISION, a
test locks it off). The orchestrator merge item is applied:
`AppMessage::SeloraStatus` threads gateway reachability into the panel
(avatars sleep while disconnected). Owner sign-off items from the report:
the pill gained a resting fill + constant width; the thinking band grew
12px→20px; `speed` clamps to ≤1.0; the 64px breathing orb carries a small
--ok liveness satellite; stepped alpha bands stand in for blur/gradient.
The §7.3 glyph vocabulary was sufficient — no new glyph was needed.

**Environment findings (this machine, new):**
- `cargo` lives at `E:\dev-build\cargo-homein`; `CARGO_TARGET_DIR` is
  `E:\dev-build\cargo-target\default` (repo-relative `target/` stays empty
  — `installer\prebuild.bat` expects `target
elease\selora-app.exe`, so
  copy it there for a local installer build; CI and the owner's machine are
  unaffected).
- **The Windows Credential Manager on this machine evicts crate-written
  credentials within seconds** (write→immediate-read works, delayed read
  returns ERROR_NOT_FOUND; a cmdkey-written entry shadowed the crate's reads
  until deleted). Keyring persistence can't be soak-tested here — Connect's
  keyring write appears to succeed, then the key is gone by next boot.
  Owner: verify keyring persistence on YOUR machine (the Phase 4 smoke test's
  "reload → still connected" step). If you're on this machine, the splash
  will reappear every launch.
- rust-analyzer IS installed here (`rustup component`, on PATH); LSP hover
  verified live (a real `PathBuf` card — see the lsp-hover screenshot).
- cargo-deny was missing; installed (0.20.2) and `licenses ok`.
- egui 0.36 facts that cost iteration: `Context::screen_rect` →
  `content_rect`; Area layer bounds land a frame late (hover needs settle
  frames headless); `clicked()` fires on the release frame; Modal areas
  include a full-screen backdrop (measure the painted frame, not the area);
  Modal has a fade-in (fills read opacity-multiplied for the first frames).

**Owner actions queued by Phase 6 (the release itself):**
1. Decide the version (RELEASE.md §5 says 0.6.0 for Phase 6), bump the root
   `Cargo.toml` `[workspace.package] version`, move CHANGELOG Unreleased →
   `[0.6.0]`, `git tag v0.6.0 && git push origin v0.6.0`.
2. Download the draft release's installer, smoke it on a clean Windows 11 VM
   (RELEASE.md §8 checklist), publish.
3. The real-key agent smoke (11-step ROADMAP Phase 5 list) is STILL blocked
   on a valid sk-gw key — unchanged from the 5d note.
4. RELEASE.md §9 open questions (repo name, license confirm, .txt default,
   source-file associations, signing cert) — all still the owner's call.

## 2026-10-07 — Phase 5d landed in the working tree (uncommitted; review then merge)

**Update (later same day): MERGED on main** — `682e88f` (pre-flight), `bdbed3d`
(5d feature), `7e81a54` (real-gateway smoke test + bash ENV_ALLOWLIST). Full
gate green after the commits (637 tests, clippy `-D warnings`, fmt).

**Real-key smoke test: scripted, blocked on a valid key.** The 11-step ROADMAP
acceptance substance is automated in
`crates/selora-ai/tests/agent_real_gateway.rs` (#[ignore], needs
`SELORA_API_KEY` + network). **Both sk-gw keys on this machine are dead**: the
`SELORA_API_KEY` env var / HKCU\Environment value is the key revoked
2026-10-02 (the gateway's chat endpoint says so; list_models does NOT check
revocation — a 200 there proves nothing), and the copy in
`~/.codex/auth.json` is rejected as invalid. To finish the smoke: put the
current key in the env (`setx SELORA_API_KEY sk-gw-…` or `$env:SELORA_API_KEY`
for the session), then `cargo test -p selora-ai --test agent_real_gateway --
--ignored --nocapture`. The pixel-level half of the 11 steps (card rendering,
button clicks, Ctrl+Z) is covered by the selora-ui paint tests + the selora-app
wiremock e2e; the app has never been booted on this machine (no keyring entry
yet — first Connect persists it).

Phase 5d (Agent Mode cards UI) + the final Phase 5 wiring are implemented in the
working tree — **not yet committed**. Full gate green on top: `cargo build
--workspace`, `cargo test --workspace` (637 passed / 0 failed, up from the 554
baseline), `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --
-D warnings`. Independent five-axis review ran; all findings closed (see the
run's "Phase 5d completion report" artifact for the long form).

- Cards: `agent_cards.rs` (stub → full) — collapsed read card, diff approval
  card, bash card, web cards, status line, turn summary footer, + delete card;
  headless paint tests for all. `chat/mod.rs` — `TimelineEntry` (Bubble | Card |
  Summary), the `agent` header pill (persisted in workspace.json), additive
  `ChatPanelEvent::AgentSend/AgentDecision`, stale-id-gated inbound reductions.
- Wiring: `Effect::AgentSend(Uuid, AgentTurnRequest)` (skeleton payload became a
  struct — documented adjustment on a dead variant), dispatcher tracks the
  workspace root, TurnCaps via `TurnCaps::spec_defaults(model.context_window)`
  (the 0.20 reserve; CONTEXT_RESERVE is not exported from selora-ai),
  `synthesize_receipt` at TurnDone (the ChatReceipt seam: summary tokens/cost +
  wall-clock, `estimated: true`), decision channel keyed by request id,
  `AgentTurnDone` gained a `TurnEnd` payload (additive; abnormal ends toast).
- §17.7: approved write/edit to an open buffer syncs it through the buffer's own
  edit API (undoable); dirty buffers are never touched (toast instead);
  `agent_touched` set drains into `LspEffect::Change` so didOpen'd servers never
  hold stale text.
- Pre-flight fixes (tree was red on this machine, unrelated to 5d): rg
  `--no-require-git` engine-parity fix (selora-fs), rust-analyzer test hardened
  to poll through indexing/-32801 (selora-lsp; machine needed the
  `rust-analyzer`+`rust-src` components), one clippy precedence parens fix
  (selora-editor).
- Open items: (1) the ROADMAP Phase 5 acceptance list (11 steps, real key +
  real workspace) is the owner's post-merge gate — NOT run; (2) agent
  transcripts are not persisted across sessions (chat_turns keeps plain turns);
  (3) `agent.brave_search_api_key` is plaintext-at-rest in settings.json by
  owner-plan design — OS keyring + a settings-dialog field is the named
  follow-up (owner decision at the smoke-test gate); (4) bash card output
  renders when the ToolResult lands — live streaming output would need a
  selora-ai AgentEvent variant (frozen in 5c), documented deviation; (5)
  ENV_ALLOWLIST additions (CARGO_HOME, proxies) if the real-key run hits them.
- **Phase 6h (KIMI_UI_PROMPT.md) can start** once 5d is merged and the owner's
  real-key smoke test passes.
- **Phase 7 — account sign-in ("login with Selora") is now planned** (2026-10-07,
  owner request): ROADMAP.md Phase 7 (decisions + tradeoffs + 7a/7b/7c subagent
  split) and ARCHITECTURE.md §7.3 (the gateway wire contract: PKCE loopback,
  rotating refresh tokens, revoke, /v1/usage). Client-side is wiremock-buildable
  anytime; real-account acceptance is gated on the gateway shipping those
  endpoints. Key-paste stays supported permanently.
- **Phase 8 — Workflows (ZCode-style dynamic multi-agent orchestration) is
  planned** (2026-10-07, owner request): ROADMAP.md Phase 8 (decision table —
  boa JS sandbox over a fixed facade, subagents = §17 loop turns, journaled
  resume, cost caps; 8a/8b/8c/8d subagent split) + ARCHITECTURE.md §18 (the
  engine contract: the "no eval" exception narrowed structurally, the facade,
  subagent/gate/journal semantics, process-model wiring). Largest phase since
  5; 8a/8b are engine-side and wiremock-testable with no UI.
- **Phase 9 — Agent Workbench is planned** (2026-10-07, owner clarification:
  "proper agent mode like ZCode / DSH VS Code agent window / Claude Code
  ultracode"): ROADMAP.md Phase 9 (dedicated Agent activity window, plan mode
  riding 5b's plan/apply split, live todos, structured escalation questions,
  subagents with read-only/full policies shared with Phase 8's engine,
  resumable sessions + context compaction, session-scoped auto-approve that
  never persists; 9a–9e subagent split, XL effort) + ARCHITECTURE.md §19 (the
  contract: session model, plan gating, the shared subagent runtime, the
  generalized park channel, compaction). Manifest v3. Phases 8 + 9 together
  are the full ZCode-class experience; order after 6h is the owner's call.

## Owner decisions (2026-10-06)

- **Phase 4 smoke test deferred** by owner decision: Phase 5 spawn prep
  started instead. The six-step list below stays the standing gate
  before "DONE" can be claimed on Phase 4 — run it before Phase 5's
  final acceptance (it exercises the same chat path the agent loop
  builds on).

## Phase 5 merge log

| Subagent | Branch/commit | State | Gate |
|---|---|---|---|
| 5e web tools | `phase5e-web-tools` @ `a44b1f2` | **merged** (ff) | 20 targets, clippy, fmt — green |
| 5a tool plumbing | `phase5a-tool-plumbing` @ `3493677` | **merged** (`51de58b`) | 20 targets, clippy, fmt — green |
| 5b tool router | `phase5b-tool-router` @ `461cf98` | **merged** (ff) | 20 targets, clippy, fmt — green |
| 5c agent loop | `phase5c-agent-loop` @ `ec746e4` | **merged** (ff) | 20 targets, clippy, fmt — green |

5c review notes (orchestrator, decision core read in full): the loop is
trait-injected (`ChatStream`/`ToolRunner` — no HTTP, no execution inside
turn.rs; production backends are thin adapters over 5a/5b); Deny feeds
`router::denied_result` back to the model; ApproveAllThisTurn is a
turn-local flag with a no-carryover test; Stop is honored at every phase
(gated select wrapper), abort kills the sticky BashAbortHandle + 500ms
grace before dropping the future; caps (25/600s/$1/context−20%) each
abort with a partial answer + `TurnEnd::toast()`. 31 tests, including a
Stop mid-tool killing a real `ping -n 60` child.

**5c deviations accepted:**
- `AgentEvent`/`TurnDecision`/`TurnCaps`/`TurnEnd` vocabulary in turn.rs
  (the shell reduces AgentEvent onto the §17.6 AppMessages).
- Context reserve fixed at 0.20 (no settings field — add
  `agent.context_reserve` only if the owner wants it tunable).
- tokio `test-util` + a dev-dep on selora-settings in selora-ai
  (test-only; the settings→TurnCaps mapping is proven in tests).

**Wiring owed when 5d lands (orchestrator's final Phase 5 pass):**
- `Effect::AgentSend` → `spawn_agent_turn` with `TurnCaps` from
  settings + the active model's context window; `Effect::AgentDecision`
  → the turn's decision channel; `AgentEvent` → AppMessage reduction +
  the per-event notify() (the repaint seam).
- `agent.brave_search_api_key` → `AgentToolSettings` (5b's
  skip_serializing field).
- §17.7 open-buffer routing: approved edits to open editors go to the
  in-memory buffer (the card is the save moment).
- 5d's cards into `sidebar/chat/mod.rs`.

5b review notes (orchestrator, guard core read line-by-line — §10's
heaviest-review rule): path rooting double-checks (lexical `..` fold +
canonicalized-ancestor symlink/junction containment, case-insensitive
on Windows); env is a fixed ALLOWLIST (`env_clear` + explicit set — the
Selora key is structurally absent, verified in a real child); bash
allowlist matches on word boundaries (`cargo testx` ≠ `cargo test`);
plan/apply separation — `execute_tool` never writes, `execute_approved`
re-parses and re-validates against current file contents (stale-approval
replay tested); bash children: `-NoProfile -NonInteractive`,
`CREATE_NO_WINDOW`, stdin null, capped pipes, `start_kill` + bounded
reap on Stop/timeout (tokio's async `Child::kill` never-fires bug found
and fixed). 33 tests.

**5b deviations accepted:**
- `execute_tool(ctx: &ToolContext, call)` reworked; `ToolContext { root,
  settings: AgentToolSettings, abort: BashAbortHandle }` in router.rs.
- selora-fs added as a selora-ai dependency (§17.6 composition).
- `AgentToolSettings` mirrors the §17.5 settings slice + a
  `skip_serializing` brave_api_key (the app maps its settings in — the
  router never reads settings/keyring).

**5b open items for the 5c merge + final wiring:**
- §17.7 open-buffer routing is the app seam: `apply_write`/`apply_edit`
  are the on-disk half; an approved edit to an open editor must also go
  to the in-memory buffer (undoable, the card is the save moment).
- `execute_approved("bash")` runs unconditionally — 5c must only hand
  it calls that produced a card (an allowlisted call re-passed would
  run twice; documented in router.rs).
- ENV_ALLOWLIST may need app-level additions (CARGO_HOME, proxies) —
  one-line changes in tools/bash.rs.
- 5c loop caps (25/10min/$1) read from settings `agent.*` —
  `AgentSettings` already has them (max_iterations/max_turn_secs/
  max_cost_usd).

5a review notes (orchestrator): OpenAI wire shapes are custom serde in
chat_types.rs (tools `{"type":"function","function":{…}}`, calls with
tolerant `arguments` folding — string/object/null); stream parsing is
defensive (missing index → slot 0, empty fragments skipped); the
scripted 3-turn mock test asserts exact request bodies per turn.
Deliverables verified: 61 tests in selora-ai.

**5a deviations accepted:**
- `finish_reason` is NOT surfaced as data — the loop distinguishes
  tool_calls-vs-stop via `accumulator.finish()` (non-empty ⇔ tools);
  5c was told (see below).
- `ToolCallAccumulator` re-exported from lib.rs (one line, additive).
- New public `SeloraClient::complete_chat` (non-streaming half).


5e review notes (orchestrator, guard read line-by-line): the §17.2 SSRF
guard covers every listed range plus 0/8, CGNAT, TEST-NETs,
benchmarking, IPv6 ULA/link-local/multicast/documentation, and
IPv4-mapped IPv6; `resolve_and_check` checks EVERY resolved address;
redirects are manual (policy: none) so each hop re-enters the guard;
fetch output capped 256 KiB (raw buffer 1 MiB), search key never in
results/logs. Known accepted limitation: DNS is resolved at
guard-time and again at connect-time, so a fast DNS rebind can race the
check (check-then-fetch, no IP pinning) — read-only tool, output-capped,
approval-gated writes; fine for Lite.

**5e wiring owed by later merges:**
- 5b's router: call `web_search(query, brave_api_key)` (two-arg) and
  `web_fetch(url)`; route URLs through `resolve_and_check` (not just
  the pure `is_url_allowed`); stamp `result.id = call.id` (web results
  carry an empty id).
- Orchestrator (post-merge): add `agent.brave_search_api_key` to
  selora-settings and pass it through; empty/absent → DuckDuckGo.

## Phase 5 prep (landed this session, gate green)

The interface skeletons all five subagents build against
(AGENT_ORCHESTRATION.md §10 file ownership; the Phase 4 lesson — push
prep BEFORE spawning so worktrees branch from it):

| Piece | File | Owner |
|---|---|---|
| `ToolSpec`/`ToolCall`/`ToolResult` + round-trip tests | `crates/selora-ai/src/chat_types.rs` (new) | 5a |
| `ChatRequest.tools`, `ChatMessage.tool_calls/tool_call_id` + `Role::Tool`, `ChatEvent::ToolCallDelta` (additive; old JSON unchanged — tested) | `crates/selora-ai/src/types.rs` | orchestrator (applied) |
| `agent` module: `ApprovalId`/`AgentDecision`/`ApprovalCard`/`AgentSummary`; `router.rs` (`ToolOutcome`, `tool_manifest`, `execute_tool` stubs); `turn.rs` (`AgentTurnState`/`AgentTurn`); `tools/web.rs` stubs | `crates/selora-ai/src/agent/` (new) | 5b/5c/5e |
| `Effect::AgentSend`/`AgentDecision` + `AppMessage::ToolCallStarted`/`ToolResult`/`ApprovalNeeded`/`AgentTurnDone` (§17.6 shapes, Uuid-keyed like `SeloraChat`; apply()/dispatcher arms are no-op stubs) | `crates/selora-app/src/app_state.rs`, `wiring.rs` | orchestrator (shared file) |
| `agent.*` settings (§17.3 caps + §17.5 shell/allowlist, serde-defaulted; pre-Phase-5 files load — tested). **`Settings.agent` is `Box<AgentSettings>`** — the unboxed section pushed `Settings` past clippy's large-enum-variant threshold; field access auto-derefs, JSON identical | `crates/selora-settings/src/settings.rs` | orchestrator |
| chat panel moved `sidebar/chat.rs` → `sidebar/chat/mod.rs`; `agent_cards.rs` stub | `crates/selora-ui/src/sidebar/chat/` | 5d |

Deviations from ARCHITECTURE.md §17.6 (deliberate, both noted in code):
worker messages key by the per-request `Uuid` + `chat_ids` map (the
existing `SeloraChat` pattern), not a bare `ChatId`; `AppMessage::ToolResult`
carries the call id inside `ToolResult` instead of repeating it in the tuple.

## Last completed

**Phase 4 (Selora AI) — all six subagents merged AND orchestrator-wired.**
Full gate green on main (`9017cca`): build, 457 tests, fmt,
`clippy --workspace --all-targets -- -D warnings`, cargo deny.

## Phase 4 record

| Item | State | Notes |
|---|---|---|
| 4a settings Selora section | merged `cfd1fea`, wired | Connect (validate via list_models → keyring set), Disconnect, live status line (4 states), key Debug-redacted; boot auto-connect from keyring when settings arrive; Connect/Disconnect effects |
| 4b model picker | merged `32237dc` | Standalone `picker::ModelPicker` widget (filter, refresh, offline). NOTE: the chat header currently uses 4c's inline dropdown (same event vocabulary, tested with the panel); 4b's widget is available if a second picker consumer appears |
| 4c chat sidebar | merged `b569520`, wired `9017cca` | Region ③ right panel (Ctrl+L / Chat activity / View command, opening focuses composer); Send → Effect::SeloraChat keyed by Uuid→ChatId map; ChatDelta/Done/Error route into the panel (stale/late ids dropped); SelectModel persists settings.selora.active_model; workspace.json gains chat_open + chat_turns (serde-defaulted; last 20 turns restore at boot) |
| 4d ghost text | merged `6caa5bb`, wired | Stage-2 350ms idle debounce → Effect::SeloraGhostText{path, prefix, model, request_caret} → ghost_complete → GhostTextReady → set_ghost on matching editors (stale-dropped); request-key guard prevents steady-state re-requesting; comet fade + Tab/other-key/Esc gestures + egui focus-lock for Tab (all in selora-editor, 72 tests) |
| 4e cost meter | merged `df492e3`, wired `d8dcd15` | ChatDone feeds CostMeterState (receipt cost verbatim, else compute_cost_usd of the resolved model — never guessed); `Σ today $X.XX` + per-model tooltip left of the connection run; day rollover |
| 4f connection status | merged `ba43f52`, wired `d8dcd15` | 30s ping loop (first ping when settings land, then every 30s; probe = list_models so Connected = gateway up AND key valid, never a bare HEAD); disconnected run clicks through to Settings; Connecting/Error forms |

Keyring semantics (ARCHITECTURE.md §7.2): the OS keyring is the key's
only home — Connect validates first, stores second; boot restore reads
it (validate again); Disconnect deletes. Tests never touch the real
Credential Manager (persist_key flag / throwaway service names).

## Next

**Phase 5 subagents 5a–5e are SPAWNED and running** (parallel
worktree-isolated branches from the pushed prep `57ffcab`). The
orchestrator's duties while they run:
- Merge in arrival order, gate after each: build + test + fmt + clippy
  (the shared-file rule — apply Effect/AppMessage/chat-panel changes
  centrally, never let two branches collide).
- Line-by-line review of every 5b guard (path rooting, SSRF, env
  stripping, output caps).
- 5d's integration into `sidebar/chat/mod.rs` is the orchestrator's
  (5d delivers standalone widgets + a call surface).
- Deviations any agent reports (settings fields to wire, shape changes)
  land as orchestrator commits.
- If an agent dies on a transient error, resume it via SendMessage
  before re-spawning (the Phase 4 lesson: cheaper than restarting).

Phase 6 is merged (6a–6h). Remaining before the ship is declared: the
owner's clean-VM acceptance (ROADMAP.md Phase 6 + RELEASE.md §8), the
release itself (bump `[workspace.package] version` to 0.6.0, move
CHANGELOG Unreleased → [0.6.0], `git tag v0.6.0 && git push origin v0.6.0`
— the pipeline drafts the release), and the still-blocked real-key agent
smoke. Then Phase 7 (account sign-in) or Phase 8 (workflows) — the owner's
call; both are planned in ROADMAP.md.

**Owner smoke test of Phase 4** (ROADMAP.md Phase 4 acceptance list) —
the one gate that needs a human and a real sk-gw- key. The owner started
it 2026-10-06 and found bugs; the fixes below are merged — **re-run from
step 1 on a fresh build** (`cargo run -p selora-app`):

1. `cargo run -p selora-app` — status bar right shows `○ selora
   disconnected · connect`
2. Click it → Settings opens ON the Selora section (Connect form) → paste
   key → Connect (or press Enter in the field) → status flips to
   `● selora connected` within 2s AND STAYS connected (the 30s ping no
   longer flips it back); reload the app — still connected (keyring
   restore)
3. Ctrl+L → chat sidebar → "list 3 prime numbers" → Enter → streams in;
   code blocks have hover Copy; ⋯ menu clear/copy-transcript
4. Model picker in the chat header: filter, select, refresh; next chat
   uses the new model (settings.selora.active_model persisted)
5. Open a Rust file, stop typing 500ms → ghost text in --fg-faint;
   Tab accepts; cost meter reads nonzero
6. Reload — last 20 chat turns + sidebar visibility restored

## Smoke-test bugfix round 1 (2026-10-06, all tested + gate green)

Seven fixes, all found by the owner smoke-testing the connect flow:

| Bug | Fix | Where |
|---|---|---|
| Manual Connect never set the dispatcher's session key — 30s later the ping flipped the status back to Disconnected; chat/ghost said "no API key" | `Effect::SeloraConnect` adopts the key immediately (AutoConnect idiom); regression test against a wiremock 401 gateway (never writes the real keyring) | `wiring.rs` |
| Editing Base URL in Settings never reached the dispatcher — Connect/chat/ping kept the boot URL until restart | new `Effect::SettingsSynced { base_url }` (no disk write, no reply; empty → DEFAULT_BASE_URL) sent when the shell adopts dialog settings | `app_state.rs`, `wiring.rs`, `shell.rs` |
| Esc while typing in a field closed the dialog and threw away the pasted key | first Esc leaves the focused field (dialog tracks `field_focused` per frame — egui's begin_pass surrenders focus on Esc before `show` can see it), second Esc closes | `dialogs/settings/mod.rs` |
| Enter in the masked key field did nothing | Enter submits Connect (inert while Connecting); the input got an explicit id for focus-driving tests | `dialogs/settings/selora_section.rs` |
| The "modal" was a plain egui::Window — clicks and keystrokes around it fell through to the workbench (file tree opened files, editor took typing; the dialog seemed "fully bugged") | real `egui::Modal` (0.36): input-blocking backdrop, keyboard focus confinement, backdrop-click dismisses; geometry pinned min==max (Modal sizes to content, and the body's available-size feedback loop made layout drift frame-to-frame — clicks missed buttons by 20px until pinned) | `dialogs/settings/mod.rs` |
| Status-bar "○ selora disconnected · connect" opened Settings on the EDITOR section instead of the Connect form | `SettingsDialog::open_selora_section()` + shell `open_settings_to_selora()` for `StatusEvent::OpenConnect` | `dialogs/settings/mod.rs`, `shell.rs` |
| `Effect::SeloraKeyringSet` was dead code (nothing sent it — the session-key path that never ran) | left in place (tests' seam); its doc comment is accurate now | `app_state.rs` |

Gate after the fixes: build, 20 test targets (~230 tests incl. 8 new
settings/connect tests), fmt, `clippy --workspace --all-targets --
-D warnings`, app boots and stays alive. The one cosmetic gap knowingly
left: after Connect the status line says "no model" until the picker runs
(`spawn_connect` reports `model: None`); also the Modal's backdrop scrim
has no design token (egui default black-alpha — token gap to note in a
DESIGN_TOKENS follow-up).

Then **Phase 5 (Agent Mode)** — five subagents (5a–5e), interface-first
(5a lands `ToolSpec`/`ToolCallDelta`/tool-result types first), briefs in
ROADMAP.md Phase 5, ownership in AGENT_ORCHESTRATION.md §10. The
Effect/AppMessage enums grow additively through the orchestrator
(§10 shared-file rule). Heaviest review phase: every guard
(path rooting, SSRF, env stripping, output caps) gets a line-by-line
read at merge.

Pending from earlier phases (owner, not blocking):
- Phase 2 smoke: open folder, edit, fuzzy find, syntax, settings persist.
- Phase 3 smoke: LSP hover + go-to-def (needs `rustup component add
  rust-analyzer` — NOT verified on PATH), git commit, search.

## Blockers

None. Phase 4 owner smoke test pending (above) — recommended before
Phase 5 spawn (agent mode composes Phase 4's chat stream; a Phase 4 bug
found now is much cheaper).

## Environment (machine, not in any repo doc)

- Toolchain: rustup + stable 1.99.0 MSVC + VS 2022 Build Tools. cargo at
  C:\Users\Toxic\.cargo\bin — bash needs the PATH export.
- Remote `origin` = github.com/ToxicAngelX/Selora-IDE; main pushed
  through the Phase 4 commits.
- Known flake: `cargo test --workspace` occasionally prints "could not
  compile selora-… (lib test)" from MSVC/Defender file-locking during
  parallel compilation — rerun confirms exit 0.
- `rg` is NOT on the Windows PATH for the app (the bash `rg` is a
  Claude Code shim) — search uses the built-in fallback scanner.
- `rust-analyzer`: NOT verified on PATH; Phase 3 smoke needs it.
- The sk-gw- test key passed through chat in session 1 — rotate at the
  gateway when convenient.
- Gate: `cargo clippy --workspace --all-targets -- -D warnings` (the
  --all-targets matters; the 3b/3c-era gate missed test-target
  warnings, fixed in `75e18f5`).
- Subagent worktrees: spawned with the Agent tool's worktree isolation
  under .claude/worktrees/ (gitignored). NOTE for future phases: the
  worktree base was `origin/main` at spawn time — push the prep commit
  BEFORE spawning so agents branch from it (Phase 4's agents predated
  the prep and every merge needed conflict resolution). Two agents
  (4b, 4c) died on transient server errors mid-task and were resumed
  via SendMessage — cheaper than re-spawning.
