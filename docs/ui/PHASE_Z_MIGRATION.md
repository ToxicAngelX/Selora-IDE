# Phase Z migration

Existing native UI remains the product surface. The Tauri shell is additive and can be built independently while the Rust core and current UI continue to evolve. Settings are read through `selora-settings`; only the safe `SettingsViewDto` reaches the frontend.

Migration proceeds by vertical slice: bootstrap and reconnect first, then workspace and editor state, followed by agent/chat streams, Git/search, and diagnostics. Each slice adds protocol DTOs and event replay before exposing a frontend control. Existing commands are not silently reinterpreted: unsupported slices return a typed error and the bootstrap advertises the later-slice flag.

Libraries.dev package research is deferred to the relevant Z3/Z5 slices. No package was required for the Z0/Z1 diagnostic shell, and no dependency is treated as integrated until its API, license, CSP, and WebView2 behavior are verified. Z3 is the first slice that mounts them.

The layout slice (Z2) has landed in the working tree: the seven-region grid, drag and keyboard resize against the contract bounds (delivered via `bootstrap.layout_constraints`, never literals), global layout persistence in the serde-defaulted `layout` section of `settings.json` (per-workspace keying still deferred to the workspace-picker slice), keyboard toggles, the command palette, toasts, the null-seeded replay gate (`get_events_after`, 1024-event ring), and the designer's token layer (`tokens.css`/`base.css` plus the bundled OFL fonts). Everything outside the slice still returns `unsupported_phase_slice`.

What Z3 (the agent-dock vertical slice) can rely on:

- The full-height resizable agent dock — 240–600px, hidden by default, `Ctrl+L` and the palette entry already routed through `ToggleSurface{surface:"agent"}` — and the inert dock chrome that shipped in Z2: the header's 20px presence slot, the "Ask anything" empty state, and the composer card geometry with its reserved border-beam perimeter. Z3 replaces the placeholders with the live transcript and mounts the Libraries.dev effects on the reserved mounts.
- The gated event path — envelope validation, stale/gap rejection, null-seeded replay — for ChatDelta batching and stale-request dropping; the toast surface for errors and approval prompts; the store pattern (`app-store.ts` plus the `features/*` stores) for the new agent store.
- The Rust-side test pattern: hermetic event-emission tests through `tauri::test::mock_app()` (the `R: tauri::Runtime`-generic `dispatch`/`record_event`/`push_event` in `state.rs`), and the `LayoutStore` read-modify-write seam as the precedent for any new persisted state.
- The protocol evolution precedent: additive, serde-defaulted DTO fields with `protocol_version` still 1 (the architecture contract §5.2 justification — additive-with-defaults changes interoperate in both directions).
- No new Tauri commands: `SendChat`/`StopChat`/`SendAgent`/`AgentDecision` ride the existing `dispatch_command`, so the capability set is already sufficient; only the event permissions already granted are needed.

## What landed after Z2

Z3 (agent dock), Z4 (editor and workbench parity), Z4b (the LSP bridge), and Z5 (brand/onboarding) all landed without adding a Tauri command. Every new slice rides `dispatch_command`, and commands it does not implement still return `unsupported_phase_slice`. The shipped union (32 commands, 25 events) and its `request_id` rules are in `PHASE_Z_PROTOCOL.md`; the implementation status and the cutover verdict are in `PHASE_Z_PARITY_REPORT.md`.

### Workspace slice (Z4)

`open_folder_at` attaches a real root through `selora-fs`; `refresh_tree` rebuilds the tree from disk and emits `file_tree_changed`; `search` runs workspace search with literal matching and a 500-hit cap; `git_stage`, `git_unstage`, `git_commit`, and `open_git_diff` drive the Git panel and its read-only diff overlay. Consumers are `FileTreeView.tsx`, `SearchPanelView.tsx`, and `GitPanelView.tsx`. The workspace id changes on attach, so Rust emits a full `bootstrap_state` rather than a delta (`state.rs:698-710`).

Open Folder has one unproven runtime edge. The native picker is registered as `pick_folder` (`main.rs:106`), but `capabilities/default.json:8-22` does not grant `allow-pick-folder` and `build.rs:29-37` omits the command from the app manifest. A green workspace build does not prove the packaged ACL allows the invoke; the orchestrator/source owner must add both entries and smoke-test the dialog.

### Editor slice (Z4)

The CodeMirror 6 editor is mounted through the lazy `CodeMirrorPane` chunk: multi-buffer tabs with dirty state, breadcrumbs, a minimap, and the in-buffer find/replace bar. Edits are versioned — `apply_editor_change` carries `buffer_id`, `version`, and `TextChange[]`, and `buffer_changed` echoes the resulting version so stale applies are rejected. `save_buffer` returns a fresh `buffer_snapshot`. Selection sync is fire-and-forget (`set_selection` is silent).

Find/replace inside a buffer is resolved locally by CodeMirror. The protocol's `find_in_buffer`, `replace_current`, and `replace_all` remain `unsupported_phase_slice`; the UI does not wait on them.

Closing a buffer is the one editor-side defect: `close_buffer` removes the Rust buffer and emits no event (`state.rs:732-743`), while the frontend discards the returned bootstrap (`editor-store.ts:261-268`). The tab can linger until an unrelated bootstrap event arrives.

### LSP bridge (Z4b)

`hover_at`, `definition_at`, and `completion_at` cross the `selora-lsp` bridge and answer with `hover_result`, `definition_result`, and `completion_result`. Published diagnostics arrive as `diagnostics_changed` and feed both the Problems panel and the editor's markers. Answers record with `request_id: None` and are routed by `buffer_id`, so a result for a closed tab is dropped instead of applied to the wrong buffer.

### Sidebar and dock live data

The activity bar's four views are backed by protocol data: the file tree by `file_tree_changed`, search by `search_results`, Git by `git_status_changed`/`git_diff_ready`, and Problems by `diagnostics_changed`. The agent dock (Z3) renders the live transcript, tool cards, approval cards, and state from `chat_*` and `agent_*` events.

Two dock tabs are deliberately inert. `OutputPanel.tsx` says "No output yet — tasks land with the terminal slice", and `TerminalPanel.tsx` is labelled "Terminal placeholder" / "Not wired yet". Neither fabricates rows, because the protocol exposes no task or shell stream.

### Notifications

Protocol `toast` events and normalized command errors feed the single toast store (`app-store.ts:248-252`, `toast-store.ts`). This covers command rejection, LSP toasts, and approval/error surfacing; there is no OS-level notification path.

### Cheat sheet

`CheatSheet.tsx` is a labelled modal over the live keybind table with a focus trap and Escape/backdrop close, opened from `Ctrl+/` and the palette.

### Onboarding and About

`OnboardingOverlay.tsx` is the first-run modal; its "seen" flag persists in `localStorage` under `selora:onboarding_seen` (`onboarding-store.ts:8`), not in Rust settings. `BrandFx.tsx` lazily mounts `img-fx` (onboarding) and `metal-fx` (About), probes WebGL/WebGL2, and falls back to plain rendering when unavailable; both respect reduced motion. `AboutDialog.tsx` renders the brand mark, version, and links — but those links are `target="_blank"` while `open_external` is unsupported and new windows are denied (`main.rs:75`), so they are inert.

### Limits carried forward

Settings remain presentation-only: `SettingsPanelView.tsx` writes local React state and never calls the bridge, and `settings_update` is unsupported. Session switching, new session, and retry are local store mutations only. These, plus the Open Folder ACL gap and the buffer-close defect, are the reasons the parity report declines a shell cutover.
