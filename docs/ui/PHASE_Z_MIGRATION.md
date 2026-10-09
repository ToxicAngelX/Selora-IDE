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
