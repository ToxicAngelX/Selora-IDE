# Selora IDE — Project Progress Tracker

> **Last Updated:** 2026-10-10  
> **Status:** Active Development (Phase Z3+ Agentic Coding Harness & Persistence Hardening)  
> **Repository Architecture:** Dual-core hybrid (Rust workspace engine + Tauri v2 / React 18 frontend)  
> **License:** MIT  

---

## 1. Executive Summary & Health Dashboard

| Domain | Status | Tests Passing | Build Status | Notes |
|---|---|---|---|---|
| **Rust Workspace (`crates/*`)** | **Green** | 730+ tests passing | Clean | Crate workspace covering core, protocol, tokens, settings, ui, editor, ai, git, lsp |
| **Settings & Keyring (`selora-settings`)** | **Green** | 34 tests passing | Clean | Windows Credential Manager (`WinCredential`) + resilient AppData fallback |
| **Desktop Shell Backend (`selora-desktop`)** | **Green** | 31 tests passing | Clean | Tauri v2 commands, session engine, security boundaries, event ring buffer |
| **Desktop Frontend (`frontend`)** | **Green** | 143 tests passing | Clean (Vite 458KB JS / 71KB CSS) | React 18 + TypeScript + Vitest + Happy-DOM; strict token adherence |
| **API Persistence & Auth** | **Green** | Verified | Persistent | Boots in `Connected` status across full OS restarts; zero key leakage |
| **Agentic Coding Harness** | **Green** | Verified | Active | Prompt Queue, Mid-flight Steer, Copy / Rewind / Edit prompt controls |

---

## 2. Phase Roadmap & Completion Matrix

```
[Phase 0: Foundation] ─────────► [Phase 1: Risk Spike] ─────────► [Phase 2: Editor Core]
        DONE                             DONE                             DONE
         │                                │                                │
         ▼                                ▼                                ▼
[Phase 3: File Tree] ──────────► [Phase 4: AI Stream] ──────────► [Phase 5: Agent Loop]
        DONE                             DONE                             DONE
         │                                │                                │
         ▼                                ▼                                ▼
[Phase R: UX Revamp R0-R7] ────► [Phase Z: Tauri Shell Z0-Z2] ──► [Phase Z3+: Agent Harness]
        DONE                             DONE                             IN PROGRESS / ACTIVE
                                                                           │
                                                                           ▼
                                                                  [Phase 6: Release & Pack]
                                                                        UPCOMING
```

### Detailed Phase Status

| Phase | Description | Status | Key Deliverables |
|---|---|---|---|
| **Phase 0: Foundation** | Rust workspace, token definitions, font assets, CI | **Completed** | 9 Cargo crates, `DESIGN_TOKENS.md`, Space Grotesk + JetBrains Mono OFL fonts. |
| **Phase 1: Risk Spike** | UI framework + text buffer + live SSE streaming proof | **Completed** | Single-window proof of concept linking streaming tokens to editor buffer. |
| **Phase 2: Editor Core** | Ropey buffer, tree-sitter syntax highlighting | **Completed** | Incremental parse, fast text buffer, cursor and selection management. |
| **Phase 3: Workspace & Files** | File tree, fuzzy file search, workspace indexing | **Completed** | Path sandboxing, ignore rules (`.gitignore`, `.zcodeignore`), project navigation. |
| **Phase 4: AI Engine** | Multi-provider streaming client, model management | **Completed** | OpenAI-compatible, OpenRouter, Anthropic, Ollama clients, model picker widget. |
| **Phase 5: Agent Mode** | Autonomous loop, tool sandboxing, diff approvals | **Completed** | Tool execution pipeline, safe file read/write, unified diff generation. |
| **Phase R: UX Revamp (R0–R7)** | ZCode-class premium coding agent experience | **Completed** | **R0:** Widget architecture & presence easing.<br>**R1:** Multiline growing composer with Shift+Enter & history.<br>**R2:** Transcript markdown rendering & code blocks.<br>**R3:** Multi-session tab management & persistence.<br>**R4:** Agent cards, diff visualizers, approval buttons.<br>**R5:** Settings dialog (AI, Keyring, General, Editor).<br>**R6:** Title bar menus, Problems panel, Git diff view, Find/Replace.<br>**R7:** Backdrop scrims, toast transitions, cheat sheets. |
| **Phase Z: Tauri Migration (Z0–Z2)** | Tauri v2 + React 18 / TS desktop migration | **Completed** | **Z0:** `selora-protocol` DTOs and IPC contracts.<br>**Z1:** Tauri v2 + WebView2 diagnostic shell.<br>**Z2:** 7-Region desktop layout (TitleBar, ActivityBar, PrimarySidebar, EditorRegion, AgentDock, BottomDock, StatusBar), drag & keyboard resize handles, layout persistence in `settings.json`. |
| **Phase Z3+: Agentic Harness** | Modern coding agent harness & persistence | **Active** | Full Agent Session Engine, Prompt Queue, Steer, Rewind/Edit/Copy, OS Keyring with Windows-native storage & AppData fallback. |
| **Phase 6: Packaging & Release** | Production installer, auto-updater, signing | **Upcoming** | Windows MSI/EXE packaging, WebView2 prerequisite checks, release automation. |

---

## 3. High-Value Agentic Capabilities (Shipped)

### 3.1 Prompt Controls & Harness Features
- **Prompt Queue**: Queue follow-up instructions while the agent is executing tasks without interrupting current streaming context. Prompts execute in FIFO order once current turn concludes.
- **Steer**: Inject real-time course correction during multi-step agent actions without wiping conversation memory or resetting tool execution state.
- **Copy Prompt**: Quick copy of sent prompt text to clipboard with instant confirmation.
- **Rewind**: Truncates conversation transcript back to the selected user turn, allowing clean alternative exploration paths.
- **Edit & Re-run**: Loads previous prompt into the active composer for rapid modification and re-execution.
- **Stop Stream**: Gracefully cancels active streaming turn, keeping the partial assistant bubble in the transcript.
- **Approval Cards**: Human-in-the-loop authorization gates for tool calls (e.g. file writing, shell execution) with explicit diff inspection.
- **Explore Mode**: Read-only safe mode that enforces a sandboxed read-only security manifest on tool calls.

### 3.2 Credential & Auth Resilience Engine
- **Windows Credential Manager Integration**: Powered by `keyring` with explicit `windows-native` feature flag targeting `api-key.selora-ide` via `WinCredential`.
- **AppData Resilient Fallback**: Secondary encrypted local fallback store (`%APPDATA%\selora-ide\.api_key` and `.brave_key`) to withstand OS credential flushes or sandbox edge-cases.
- **Instant Boot Reconnect**: `SessionEngine::new` checks stored credentials upon launch, exports `SELORA_API_KEY`, seeds default models, and initializes the app in `ConnectionStatus::Connected` rather than `Disconnected`.
- **Zero-Leakage Security Policy**: API keys are never echoed in logs, diagnostic toasts, or frontend IPC event envelopes.

---

## 4. Architecture & Key Crate Map

```
selora-ide/
├── apps/
│   └── selora-desktop/          # Tauri v2 Desktop Application
│       ├── frontend/            # React 18 + TypeScript + Vite + Vitest
│       │   ├── src/layout/      # 7-region desktop grid & resize handles
│       │   ├── src/features/    # agent, auth, keymap, layout, toasts
│       │   └── src/styles/      # Design tokens (tokens.css, shell.css, agent.css)
│       └── src-tauri/           # Rust Tauri backend (commands, state, sessions, security)
├── crates/
│   ├── selora-protocol/         # Strict IPC DTOs & frontend-backend contracts
│   ├── selora-settings/         # Settings store, OS keyring & fallback persistence
│   ├── selora-ai/               # LLM clients, streaming, agent router & turn loop
│   ├── selora-tokens/           # Atomic design token definitions
│   ├── selora-ui/               # UI widgets, theme evaluators, presence animations
│   ├── selora-editor/           # Text buffer (ropey), tree-sitter highlighting
│   ├── selora-core/             # Common error types, logging, and utilities
│   ├── selora-git/              # Git status, diff generation, and repository tools
│   └── selora-lsp/              # Language Server Protocol client & diagnostics
└── docs/                        # Architecture & design specifications
```

---

## 5. Verification & Test Gate Results

| Test Suite | Command | Result | Details |
|---|---|---|---|
| **Crates Settings** | `cargo test -p selora-settings` | **PASSED** (34/34) | Keyring persistence, fallback migration, serialization |
| **Desktop Shell** | `cargo test -p selora-desktop` | **PASSED** (31/31) | Session engine, bootstrap, bounds clamping, event ring |
| **Full Workspace** | `cargo test --workspace` | **PASSED** (730+) | All core, editor, AI, UI, and protocol crate suites green |
| **Frontend Unit** | `npm --prefix apps/selora-desktop/frontend test` | **PASSED** (143/143) | Vitest suites for agent dock, composer, shell grid, bridge |
| **Frontend Typecheck** | `npm --prefix apps/selora-desktop/frontend run typecheck` | **PASSED** | 0 TypeScript errors |
| **Frontend Build** | `npm --prefix apps/selora-desktop/frontend run build` | **PASSED** | Optimized production bundle created in `dist/` |
| **Release Binary** | `cargo build --release -p selora-desktop` | **PASSED** | Production binary linked: `selora-desktop.exe` (9.3MB) |

---

## 6. Current Workstream & Next Milestones

- [x] Implement prompt queue, steer, and prompt action controls (copy, rewind, edit).
- [x] Fix keyring `windows-native` feature flag to permanently resolve API key disconnections.
- [x] Add resilient fallback key storage in `%APPDATA%\selora-ide\`.
- [x] Ensure `selora-desktop` boots in `Connected` state with cached model list on restart.
- [x] Compile and verify the production release binary `selora-desktop.exe`.
- [ ] Finalize Phase Z4 CodeMirror editor bridge integration inside the `EditorRegion`.
- [ ] Implement Phase 6 packaging & Windows distribution pipeline.
