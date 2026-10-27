# Phase Z Tauri architecture

The native shell lives in `apps/selora-desktop/src-tauri` and targets Windows/WebView2 through Tauri 2. Rust owns workspace paths, settings, credentials, approvals, and protocol sequencing. The React frontend receives a bootstrap snapshot through the typed `get_bootstrap_state` command and subscribes to `selora://event` for ordered events.

The current shell is an honest Z0/Z1 diagnostic surface: it starts with an empty workspace, no models, disconnected core status, and later-slice feature flags. `main` is declared in `tauri.conf.json`; the Windows icon is `icons/icon.ico`, copied from the repository icon asset. Navigation is restricted to the local Tauri origin (and the Vite dev origin in debug builds), new-window requests are denied, and the CSP blocks network/file/embed primitives beyond the IPC and development websocket origins.

The shell intentionally does not package a production installer yet (`bundle.active=false`). A future Windows packaging slice should add signing, WebView2 prerequisite policy, update channels, and installer verification without moving secrets into the webview.

## Appendix: implementation status

This appendix is additive; the contract text above is unchanged. The authoritative status is `docs/ui/PHASE_Z_PARITY_REPORT.md`, which records the gate results, the full command/event map, the Libraries.dev mount audit, accessibility and security review, bundle measurements, and the remaining gaps.

As of the Z6 review: `FrontendCommand` has 32 variants with 24 implemented and 8 returning `unsupported_phase_slice`; `FrontendEvent` has 25 variants, of which `agent_event` and `error` have no production emitter. Z3/Z4/Z4b/Z5 reuse the original seven Tauri commands, so the capability file and app manifest above still describe the IPC surface. Two contract-adjacent defects remain: `pick_folder` is registered but absent from `capabilities/default.json` and the `build.rs` manifest, and `close_buffer` emits no event. The parity report therefore recommends keeping the Tauri shell additive and the egui shell as the fallback until the source fixes and the `PHASE_Z_TAURI_PROMPT.md` §13 manual walkthrough are complete.
