# Phase Z migration

Existing native UI remains the product surface. The Tauri shell is additive and can be built independently while the Rust core and current UI continue to evolve. Settings are read through `selora-settings`; only the safe `SettingsViewDto` reaches the frontend.

Migration proceeds by vertical slice: bootstrap and reconnect first, then workspace and editor state, followed by agent/chat streams, Git/search, and diagnostics. Each slice adds protocol DTOs and event replay before exposing a frontend control. Existing commands are not silently reinterpreted: unsupported slices return a typed error and the bootstrap advertises the later-slice flag.

Libraries.dev package research is deferred to the relevant Z3/Z5 slices. No package is required for this diagnostic shell, and no dependency is treated as integrated until its API, license, CSP, and WebView2 behavior are verified.
