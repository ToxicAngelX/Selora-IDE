# Selora IDE — Lite Edition: Master Plan

**Owner:** Toxic
**Repo:** https://github.com/ToxicAngelX/Selora-IDE
**Status:** Planning (no product code yet)
**Working directory:** `C:\Users\Toxic\Desktop\Selora IDE`
**Target platform:** Windows first; macOS/Linux compile-targets documented as future work

---

## 1. Vision

**Selora IDE is the editor born connected to Selora.** A real, open-source, native-feel code editor in Rust whose chat, completions, and model switching are billed through `api.selora.lol` — the owner's own AI gateway.

It looks and behaves like the Selora brand: monochrome, hairline, calm. The differentiator is not the editing surface (Zed/VS Code already do that well) — it is that the AI is *native* to the editor and the gateway is *first-class*. No API key to paste in a third-party settings panel, no extension marketplace between the user and the models, no "BYO provider" detours.

And the AI is not a spectator: **Agent Mode** (in scope for Lite) lets the model read, edit, create, and delete files in the workspace and search the web — Claude Code-style — with every write gated behind an approve-the-diff card in the chat. The agent runs through Selora, so the owner's gateway is also the agent's brain.

### Lite Edition = the shippable core

This plan covers a **Lite Edition** that ships real, working features end-to-end. Not a demo. Not a mockup. Every feature in scope has a verification step that proves it works.

---

## 2. Non-goals (Lite Edition)

These are **explicitly out of scope** to keep the Lite Edition honest and shippable. Each item here has a note on *why* it is deferred and where it would live in a future edition.

| Out of scope (Lite) | Why deferred | Future home |
|---|---|---|
| Real-time collaboration (CRDT/OT) | Zed-grade multiplayer is a multi-engineer investment; we don't need it on day one | v2+ if Selora collab backend ever exists |
| Built-in terminal | Quality shells are deep work; users can run `wt`/`Terminal.app` for now | v1.x |
| Agent sandboxing (devcontainer/VM isolation for bash) | Lite's agent runs commands with per-command approval gates, not sandboxing; true isolation is a large systems project | v1.x |
| Background/unattended agents | Lite's agent loop only runs while the user is watching the chat; background agents need a trust + scheduling design | v2 |
| Debugger / DAP | DAP client complexity rivals LSP | v1.x after LSP is solid |
| Extension marketplace / VS Code extension compat | The whole point of Selora IDE is *not* being a VS Code reskin | If ever — own extension model |
| Mobile / web client | IDE is a desktop app | Never (the gateway itself is web-accessible separately) |
| macOS / Linux installers | Windows-only owner; cross-platform is buildable but packagable Linux/macOS installers are separate effort | v1.1 — repo is structured to allow it |
| Code signing (Authenticode) | Costs money + EV cert; documented as optional in `RELEASE.md` | When owner is ready |
| Custom GPUI-style UI framework | Lite ships on a proven framework; a custom framework is a 6-month detour | v3.0 — only if egui becomes a ceiling |
| Profiler, flame charts, traces | Performance work happens after v1 | v1.x |

### Non-goals are load-bearing

Every non-goal is a deliberate trade. The owner + agents will be tempted to drift into them. The `Lite Edition` tag is the answer.

---

## 3. Architecture overview

```
                       ┌────────────────────────────────────────────────────┐
                       │                  selora-app (bin)                  │
                       │   tokio main → egui frame loop on main thread      │
                       └────────────┬──────────────────────────┬────────────┘
                                    │                          │
                ┌───────────────────▼────────────┐   ┌─────────▼─────────────┐
                │       selora-ui  (egui)        │   │   tokio runtime        │
                │  chrome, panels, palette,      │   │   - HTTP (reqwest)     │
                │  command palette, modals,      │   │   - SSE + agent loop   │
                │  settings, first-run           │   │   - file I/O           │
                └────────┬──────────┬────────────┘   │   - keyring            │
                         │          │                │   - LSP client actor   │
        ┌────────────────▼─┐   ┌────▼────────────┐    └────────┬──────────────┘
        │ selora-editor    │   │  selora-ai      │             │
        │  - ropey buffer  │   │  - model list   │   ┌─────────▼─────────────┐
        │  - text layout   │   │  - chat stream  │   │ spawned subprocesses   │
        │  - gestures      │   │  - ghost text   │   │  - rust-analyzer       │
        │  - ghost text    │   │  - cost meter   │   │  - typescript-language │
        └────────┬─────────┘   └────────┬───────┘   │    -server             │
                 │                      │           │  - (ripgrep)           │
        ┌────────▼────────┐   ┌─────────▼────────┐  └────────────────────────┘
        │ selora-fs       │   │ selora-settings  │
        │  - workspace    │   │  - JSON config   │
        │  - file tree    │   │  - keyring ref   │
        │  - fuzzy find   │   └──────────────────┘
        │  - search       │
        └────────┬────────┘
                 │
        ┌────────▼────────┐   ┌──────────────────┐
        │ selora-git      │   │  selora-lsp      │
        │  (git2-rs)      │   │  (lsp-types)     │
        └─────────────────┘   └──────────────────┘
```

**Key facts:**
- **Single binary, single process.** No extension host, no separate renderer.
- **Tokio runtime** drives all I/O: HTTP, SSE, file IO, LSP.
- **Main thread** runs the egui frame loop. UI state is owned by the main thread; workers communicate back via `tokio::mpsc` channels.
- **LSP servers are spawned subprocesses** of the main process (rust-analyzer, typescript-language-server). Messages flow over stdio JSON-RPC.
- **Agent Mode is a tool loop in the tokio runtime.** The model's tool_calls (read/edit/write/delete files, web search/fetch) execute there; every write pauses the loop until the user approves a diff card in the chat. Full design: `ARCHITECTURE.md` §17.
- **The editor pane is a custom widget** drawn on top of egui's canvas (egui is for chrome; we paint text ourselves to control ghost-text rendering and animations).

---

## 4. Tech stack — final decisions

Each pick is scored for: solo owner + AI-agent team, Windows-first, rendering perf, text-editor fitness, long-term maintainability. Detailed scoring in `RISKS.md` and `ARCHITECTURE.md`.

### 4.1 UI framework: **egui** (with eframe + a custom editor pane)

**Decision matrix score (1-5):**

| Criterion | egui | iced | Tauri+webview | custom (GPUI-like) |
|---|---|---|---|---|
| Solo + agent friendliness | 5 | 4 | 3 | 1 |
| Windows-first | 5 | 4 | 4 | 3 |
| Rendering perf | 4 | 4 | 3 | 5 |
| Text-editor fitness | 3 (chrome) | 3 (chrome) | 2 | 5 |
| Long-term maintainability | 5 | 4 | 3 | 1 |
| **Total** | **22** | **19** | **15** | **15** |

**Why egui wins for Lite:**
- AI agents (Kimi K3, GLM 5.3, Claude) all know egui deeply; faster cold-start, fewer hallucinations.
- Mature, used in production, fast iteration loop.
- Pure Rust, no webview tax, no Node.js, no Chromium.
- We **don't** use egui for the editor pane itself — we draw text on a custom rendering surface (via `egui::Painter`) so we can do ghost text, subpixel positioning, custom selection rendering, and Selora's exact hairline aesthetic.
- egui's immediate mode is a perfect fit for AI-driven feature work: state is a plain struct, easy to reason about, easy to refactor.

**What we borrow from Zed (without taking GPUI):** the *idea* of a custom-rendered editor pane on top of a general UI framework. We implement that idea differently with egui.

**What we do NOT take from Zed:** GPUI itself, their collaboration engine, their branding, their assets.

### 4.2 Text buffer: **ropey**

- Mature, used by Lapce, Helix, others.
- Rope data structure → O(log n) for insert/delete, perfect for large files.
- Lines/columns are first-class (text editors care).

### 4.3 Syntax highlighting: **tree-sitter**

- `tree-sitter` + `tree-sitter-highlight` + language grammars as separate crates.
- Languages for Lite: Rust, TypeScript, JavaScript, Python, JSON, TOML, Markdown, HTML, CSS.
- Re-parse is incremental → fast even on big files.

### 4.4 Fuzzy matching: **nucleo-matcher** (from Helix)

- High-quality, fast, predictable ranking.
- Used by Helix — battle-tested.

### 4.5 LSP: hand-rolled client on top of **lsp-types** + **tokio**

- `tower-lsp` is server-side. For client, we use `lsp-types` for protocol types and a thin actor over `tokio::process::Child` + stdio JSON-RPC.
- The actor handles: spawn server, send initialize, route didOpen/didChange/didSave/hover/definition, surface diagnostics.
- ~600-800 LOC for a working client. A Lite LSP client that handles the 80% is small.

### 4.6 Git: **git2-rs** (libgit2 bindings)

- Pure-Rust API surface over a C library. Faster and more reliable than shelling out for our use cases.
- Use it for: status, diff, stage/unstage, commit. Shell out only for push/pull/fetch (out of Lite scope).

### 4.7 HTTP / SSE: **reqwest** + manual SSE parsing

- `reqwest` is the de-facto Rust HTTP client. Mature, agent-friendly.
- SSE is a thin text format: `event:`/`data:` lines, `\n\n` terminator. We parse it in-house (no `eventsource` crate dependency needed; ~80 LOC). Reasoning: SSE is a 4-line grammar; one less crate to vet.
- TLS: rustls via reqwest (no OpenSSL on Windows).

### 4.8 Keyring: **keyring** crate

- Cross-platform, uses Windows Credential Manager on Windows.
- The Selora `sk-gw-…` key is stored here, never in plaintext config.

### 4.9 Async runtime: **tokio** (multi-threaded)

- Standard. No discussion needed.

### 4.10 Installer: **Inno Setup**

- Scriptable, well-documented, AI agents know it.
- Windows-native (no MSI complexity).
- Outputs a single `.exe` installer that GitHub Releases can attach.

### 4.11 License: **MIT**

- Simple, permissive, friendly to a solo open-source maintainer.
- No patent grant, but no patent grant is fine for an editor — the patent risk is in the gateway, not the IDE.
- Apache 2.0 is a fine alternative; the plan's recommendation is MIT for simplicity. Decision reversible.

### 4.12 CI: **GitHub Actions**

- Free for public repos.
- `windows-latest` runner for Windows builds.
- Single workflow: build → test → package → attach `.exe` to a GitHub Release on tag.

---

## 5. Crate / module map

Workspace `Cargo.toml` at the root. All crates are workspace members.

```
selora-ide/
├── Cargo.toml                          # workspace root
├── rust-toolchain.toml                 # pin stable toolchain
├── README.md
├── LICENSE                             # MIT
├── CONTRIBUTING.md
├── docs/
│   ├── ARCHITECTURE.md
│   ├── DESIGN_TOKENS.md
│   ├── UI_SPEC.md
│   ├── ROADMAP.md
│   ├── RISKS.md
│   ├── AGENT_ORCHESTRATION.md
│   └── RELEASE.md
├── .github/
│   ├── workflows/
│   │   ├── ci.yml                      # build + test on every push
│   │   └── release.yml                 # build installer on tag, attach to release
│   ├── ISSUE_TEMPLATE/
│   └── PULL_REQUEST_TEMPLATE.md
├── assets/
│   ├── icons/
│   │   ├── selora-ide.ico              # Windows app icon
│   │   ├── selora-ide-256.png
│   │   ├── selora-ide-512.png
│   │   └── selora-ide-mono.svg         # source mark
│   ├── fonts/
│   │   ├── SpaceGrotesk-Regular.ttf
│   │   ├── SpaceGrotesk-Medium.ttf
│   │   ├── SpaceGrotesk-SemiBold.ttf
│   │   ├── JetBrainsMono-Regular.ttf
│   │   └── JetBrainsMono-Medium.ttf
│   └── screenshots/                    # populated by Phase 5
├── installer/
│   ├── selora-ide.iss                  # Inno Setup script
│   └── README.md                       # how to invoke the installer
├── crates/
│   ├── selora-core/                    # buffer, workspace model, paths
│   ├── selora-fs/                      # file tree, fuzzy finder, search
│   ├── selora-editor/                  # editor widget (text paint, gestures, ghost text)
│   ├── selora-lsp/                     # LSP client actor
│   ├── selora-git/                     # git2-rs wrapper
│   ├── selora-ai/                      # Selora API client + SSE + ghost-text provider
│   ├── selora-settings/                # JSON config, keyring ref
│   ├── selora-ui/                      # egui chrome (panels, palette, modals)
│   └── selora-app/                     # binary: wires it all together
└── .gitignore
```

### Crate responsibilities

| Crate | Responsibility | Public surface |
|---|---|---|
| `selora-core` | `Buffer` (ropey wrapper), `Workspace`, `PathId`, position types, undo/redo | `Buffer`, `BufferId`, `Position`, `Range` |
| `selora-fs` | File tree traversal (with `.gitignore` via `ignore` crate), fuzzy finder, search | `FileTree`, `fuzzy_find`, `search_in_files` |
| `selora-editor` | Editor widget: text paint, cursor, selection, scroll, gestures, ghost text overlay, gutter | `Editor` (egui widget) |
| `selora-lsp` | LSP client actor over stdio JSON-RPC; queue messages, route responses | `LspClient`, `LspMessage` |
| `selora-git` | Status, diff, stage/unstage, commit | `GitRepo`, `GitStatus` |
| `selora-ai` | `GET /v1/models`, `POST /v1/chat/completions` (streaming + non-streaming), ghost-text provider, cost meter, Agent Mode tool loop (§17: tools, router, turn controller) | `SeloraClient`, `ChatEvent`, `Model`, `AgentTurn` |
| `selora-settings` | JSON config read/write, keyring get/set | `Settings`, `KeyringStore` |
| `selora-ui` | Top-level egui UI: panel layout, command palette, file tree, chat sidebar, status bar, modals | App-level structs + `UiApp` |
| `selora-app` | `main.rs`: tokio runtime, egui frame loop, crate wiring, app state | `fn main()` |

### Inter-crate rules

- `selora-core` depends on no other workspace crate (the lowest layer).
- `selora-app` depends on every other crate (the only "knows about everything" crate).
- `selora-ui` depends on `selora-editor`, `selora-fs`, `selora-ai`, `selora-settings`, `selora-git`, `selora-lsp` (it composes them).
- `selora-ai` depends on `selora-core` and `selora-settings` only.
- `selora-lsp` depends on `selora-core` only.
- No circular deps. Enforced by `cargo metadata` + CI.

---

## 6. Data flow diagrams

### 6.1 Editor event loop (per frame)

```
egui::Context.run(closure):
  1. drain mpsc channels from workers → apply to AppState
  2. for each open Editor instance: paint
       - read buffer (ropey)
       - compute visible range from scroll/cursor
       - layout glyphs (cosmic-text or similar)
       - paint text + selection + cursor + gutter + ghost text
  3. paint chrome: file tree, command palette, status bar, chat sidebar
  4. handle input: keyboard (keymap), mouse (gesture state machine)
  5. on input that mutates state: enqueue effects to workers
       - file save → tokio task writes to disk
       - LSP didChange → lsp actor receives
       - ghost text request → selora-ai debounced task
```

### 6.2 LSP message flow

```
Editor keypress "."
  → buffer.insert(".")
  → enqueue: (buffer_id, version, full_text) → lsp_actor inbox
  → lsp_actor: didChange → write to LSP server stdin
  → LSP server processes (rust-analyzer, etc.)
  → LSP server: publishDiagnostics → write to its stdout
  → lsp_actor: read stdout, parse JSON-RPC
  → publish to mpsc diagnostics channel
  → next frame: AppState receives diagnostic → editor overlay paints red squiggle
```

### 6.3 Selora SSE streaming flow (chat)

```
User submits chat message in selora-ui
  → selora-ai: build request { model, messages: [..history, new], stream: true }
  → reqwest: POST https://api.selora.lol/v1/chat/completions
  → response body is an SSE byte stream
  → tokio task: read line-by-line, parse "data: {...}" chunks
  → emit ChatEvent::Delta(text) per chunk
  → mpsc: ChatEvent → selora-ui inbox
  → next frame: chat panel appends delta to streaming bubble
  → on final "[DONE]": emit ChatEvent::Done(receipt) → cost meter updates
```

### 6.4 Ghost text inline completion

```
Editor idle for 350ms (debounce)
  → selora-ai: build request { model: <completion model>, prompt: <preceding+context> }
  → reqwest: POST /v1/chat/completions (non-streaming)
  → response: completion string
  → mpsc: GhostText(suggestion) → selora-ui
  → next frame: editor paints ghost text at caret in muted ink-low
  → user presses Tab → editor accepts → buffer.insert(suggestion)
  → any other keypress → ghost text dismissed
```

---

## 7. Persistence model

| What | Where | Format |
|---|---|---|
| Selora API key | OS keyring (Windows Credential Manager) | via `keyring` crate; service = "selora-ide", account = "default" |
| User settings | `%APPDATA%\selora-ide\settings.json` | JSON, versioned with `_v` field for migrations |
| Workspace state (open tabs, recent folders) | `%APPDATA%\selora-ide\workspace.json` | JSON, on graceful close + every 5s |
| Crash log | `%LOCALAPPDATA%\selora-ide\logs\app-YYYY-MM-DD.log` | Plain text, env-filter |
| LSP server logs | `%LOCALAPPDATA%\selora-ide\logs\lsp\<server>.log` | Plain text |

No state is stored in the project directory. No `.selora/` folder is written into the user's repos. (We can add per-workspace settings later if asked.)

---

## 8. Error model

- Every fallible operation returns `Result<T, SeloraError>`.
- `SeloraError` is an `enum` with `thiserror` derives. Variants: `Io`, `Net`, `Lsp`, `Git`, `Ai`, `Auth`, `Config`, `Other(String)`.
- Errors are surfaced to the user as toast notifications in the status bar, with a "Show logs" action.
- No silent failures. No swallowed errors. No `unwrap()` in non-test code.
- A `tracing` setup: `tracing-subscriber` with env filter `RUST_LOG=info` default; `debug` for dev.

---

## 9. Phase breakdown (summary; see `ROADMAP.md` for details)

| Phase | Goal | Exit gate |
|---|---|---|
| 0 — Foundation | Repo skeleton, CI, design tokens in code, hello-window | `cargo run` opens a dark Selora-styled window with a "Selora IDE" title |
| 1 — Risk spike | UI framework + text buffer + live Selora SSE call **in one window** | Open a temp file, edit it, see SSE text stream into a panel |
| 2 — Editor core | Tabs, splits, file tree, fuzzy find, syntax highlight, command palette, settings | Open a folder, navigate to a Rust file, see it highlighted, fuzzy-find it, edit it |
| 3 — LSP & Git | Real LSP (rust-analyzer + ts-ls), git panel | Open a Rust file, "go to definition" works, hover shows type, status bar shows git branch + changed count |
| 4 — Selora AI | Key connection, model picker, chat sidebar, ghost text, cost meter | Connect a real `sk-gw-` key, switch models, stream a real chat reply, accept a ghost-text suggestion |
| 5 — Agent Mode | Claude Code-style tool loop: file read/edit/write/delete, bash, web search — approval-gated | A real agent turn reads files, proposes diffs, runs commands, fetches the web; Deny leaves disk untouched |
| 5 — Agent Mode | Tool loop: model reads/edits/writes/deletes files, runs bash, searches web; approval cards in chat | Real multi-tool agent turn against a real workspace; every write on disk traceable to an approved diff |
| 6 — Polish & ship | First-run, splash, about, installer, CI release pipeline, docs | Download installer from CI, install on a clean Windows VM, all Lite features work |

Each phase has its own subagent brief, acceptance tests, and explicit DONE criterion. Detailed in `ROADMAP.md`. Orchestration across phases in `AGENT_ORCHESTRATION.md`.

---

## 10. The hard rules, restated

1. **Real, not mocked.** Every feature ships working. Every feature has a verification step.
2. **No Zed/VS Code code or branding.** Borrow architecture, not assets or trademarks.
3. **Rust core, UI in Rust, webview only with a written justification.** We do not use a webview.
4. **Fully packaged installer.** Inno Setup, CI-built, attached to GitHub Releases.
5. **Open source.** MIT, README, CONTRIBUTING, issue templates, CI badge.
6. **Subagent-executable.** Every phase brief is cold-startable. No "see context" steps.

---

## 11. Build handoff

The build handoff line, verbatim:

> **Take my task → split it into 5 logical phases → identify dependencies → run all independent phases in parallel using subagents → merge and review everything → complete the final result.**
