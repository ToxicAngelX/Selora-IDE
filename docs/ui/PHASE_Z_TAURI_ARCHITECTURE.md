# Phase Z Tauri architecture

The native shell lives in `apps/selora-desktop/src-tauri` and targets Windows/WebView2 through Tauri 2. Rust owns workspace paths, settings, credentials, approvals, and protocol sequencing. The React frontend receives a bootstrap snapshot through the typed `get_bootstrap_state` command and subscribes to `selora://event` for ordered events.

The current shell is an honest Z0/Z1 diagnostic surface: it starts with an empty workspace, no models, disconnected core status, and later-slice feature flags. `main` is declared in `tauri.conf.json`; the Windows icon is `icons/icon.ico`, copied from the repository icon asset. Navigation is restricted to the local Tauri origin (and the Vite dev origin in debug builds), new-window requests are denied, and the CSP blocks network/file/embed primitives beyond the IPC and development websocket origins.

The shell intentionally does not package a production installer yet (`bundle.active=false`). A future Windows packaging slice should add signing, WebView2 prerequisite policy, update channels, and installer verification without moving secrets into the webview.
