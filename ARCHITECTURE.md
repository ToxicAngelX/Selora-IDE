# Selora IDE — Architecture (deep design)

This document is the technical contract. Build agents (Kimi K3, GLM 5.3, Claude with subagents) **must** read this end-to-end before starting any crate. It is intentionally specific: no "consider", no "could", no "maybe".

---

## 1. Process and thread model

### 1.1 Single process, three layers

Selora IDE runs as **one OS process** with three logical layers:

1. **Main / UI thread** — runs the `egui` event loop. This thread is special: `egui` paints on whatever thread the `eframe` harness gives it. On Windows that is the thread that called `run()`. UI state is owned here.
2. **Tokio runtime** — multi-threaded, started *before* the egui loop begins. The runtime handles every I/O concern: HTTP, SSE, file IO, keyring, LSP client actor.
3. **LSP server subprocesses** — each is a real OS child process (e.g. `rust-analyzer.exe`). They are owned by the LSP client actor in the Tokio runtime. We never run LSP code in-process.

```
                    Selora IDE process
   ┌──────────────────────────────────────────────────────────────┐
   │                                                              │
   │   ┌──────────────────┐         ┌────────────────────────┐    │
   │   │  Main / UI thread │  msgs   │   tokio runtime         │    │
   │   │  (egui loop)      │ ◀─────▶ │   (multi-threaded)      │    │
   │   │                   │  mpsc   │                         │    │
   │   │  AppState lives   │         │  - HTTP client tasks    │    │
   │   │  here             │         │  - SSE consumer tasks   │    │
   │   │                   │         │  - File IO tasks        │    │
   │   │                   │         │  - Keyring tasks        │    │
   │   │                   │         │  - LSP client actors    │    │
   │   └──────────────────┘         │                         │    │
   │                                └────┬────────┬───────────┘    │
   │                                     │        │                │
   │                                  spawn     spawn              │
   │                                     ▼        ▼                │
   │                          ┌────────────┐ ┌────────────┐         │
   │                          │  rust-     │ │  typescript│         │
   │                          │  analyzer   │ │  -ls       │         │
   │                          │  (subproc) │ │  (subproc) │         │
   │                          └────────────┘ └────────────┘         │
   └──────────────────────────────────────────────────────────────┘
```

### 1.2 Why a single process

- One binary to install and to update.
- No IPC tax between editor and AI.
- Simpler mental model for the owner and for AI agents.
- Zed proves it works for an editor; we follow.

### 1.3 Why main-thread UI state

- `egui` is immediate mode. State lives in closures; closures need a stable owner. The main thread is that owner.
- Cross-thread mutation of UI state requires locks or channels, both of which add complexity for zero gain in a single-process editor.
- Pattern: workers send `Message` values; the main thread receives them at the top of each frame and applies them to `AppState`. Mutations from input flow the other way: main thread → `tokio::mpsc::Sender<Effect>` to workers.

### 1.4 Tokio runtime configuration

```rust
fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");

    // Build app state (does not block).
    let app_state = AppState::new(runtime.handle().clone());

    // Hand off to egui. The runtime keeps running on its worker threads
    // for the lifetime of the process; the main thread joins the egui loop.
    eframe::run_native(
        "Selora IDE",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(SeloraApp::new(cc, app_state)))),
    );
}
```

Four worker threads is a deliberate default: enough for HTTP, SSE, file IO, and one LSP server without contention. Tunable later.

---

## 2. Event loop and frame pipeline

A single frame is composed of seven ordered stages. Build agents **must not** reorder them; the order is the contract.

```
1. Drain worker messages                  (apply to AppState)
2. Process timeouts                       (ghost text debounce, autosave)
3. Update derived state                   (selection, scroll, dirty flags)
4. Compute layout                         (panel sizes, splitter positions)
5. Paint chrome                           (panels, sidebars, status bar)
6. Paint editor pane(s)                   (custom text rendering)
7. Enqueue effects to workers             (saves, LSP didChange, etc.)
```

### 2.1 The Message enum

Workers send one of these to the main thread:

```rust
pub enum AppMessage {
    // Selora AI
    ModelsLoaded(Result<Vec<Model>, SeloraError>),
    ChatDelta(uuid::Uuid, String),         // (chat_id, delta_text)
    ChatDone(uuid::Uuid, ChatReceipt),     // receipt for cost meter
    ChatError(uuid::Uuid, SeloraError),
    GhostTextReady(BufferId, Option<String>),
    SeloraStatusChanged(SeloraStatus),

    // LSP
    Diagnostics(BufferId, Vec<Diagnostic>),
    LspHover(BufferId, Position, Option<Hover>),
    LspDefinition(BufferId, Position, Option<Vec<Location>>),
    LspCompletions(BufferId, Position, Vec<CompletionItem>),

    // File system
    FileTreeUpdated { root: PathBuf, tree: FileTreeNode },
    SearchResults(SearchId, Vec<SearchHit>),

    // Git
    GitStatusChanged(GitStatus),

    // App
    KeyringError(SeloraError),
}
```

### 2.2 The Effect enum

The main thread sends one of these *to* the tokio runtime:

```rust
pub enum Effect {
    // File system
    SaveBuffer(BufferId),
    OpenFile(PathBuf),
    OpenFolder(PathBuf),
    RefreshFileTree(PathBuf),
    Search(SearchQuery),

    // LSP
    LspOpen(BufferId, PathBuf, LanguageId, String),
    LspChange(BufferId, String, Version),
    LspSave(BufferId, PathBuf),
    LspClose(BufferId),
    LspRequestHover(BufferId, Position),
    LspRequestDefinition(BufferId, Position),

    // Selora
    SeloraConnect(String),                 // raw sk-gw-… key
    SeloraListModels,
    SeloraChat(uuid::Uuid, ChatRequest),
    SeloraCancelChat(uuid::Uuid),
    SeloraGhostText(BufferId, String),     // debounced
    SeloraKeyringGet,
    SeloraKeyringSet(String),

    // Git
    GitStatus(PathBuf),
    GitStage(PathBuf),
    GitUnstage(PathBuf),
    GitCommit(PathBuf, String),

    // Settings
    SettingsLoad,
    SettingsSave(Settings),
}
```

The runtime has a single consumer task that owns an `mpsc::Receiver<Effect>` and dispatches.

---

## 3. State management

### 3.1 AppState

`AppState` is the single struct that holds all editor state. It is `!Send` (it stays on the main thread).

```rust
pub struct AppState {
    pub workspace: Option<Workspace>,
    pub open_buffers: BTreeMap<BufferId, BufferHandle>,
    pub active_editor: Option<EditorId>,
    pub editors: Vec<Editor>,              // each pane is one Editor
    pub file_tree: FileTreeView,
    pub chat_panels: Vec<ChatPanel>,
    pub active_model: Option<ModelId>,
    pub selora_status: SeloraStatus,
    pub git_status: Option<GitStatus>,
    pub settings: Settings,
    pub toasts: Vec<Toast>,

    // channels
    pub tx_effects: mpsc::UnboundedSender<Effect>,
    pub rx_messages: mpsc::UnboundedReceiver<AppMessage>,
}
```

The AI agent writes here, the UI reads from here. The state is rebuildable from disk + keyring + Selora API on startup.

### 3.2 Buffer, BufferHandle, BufferId

- `Buffer` owns the rope and the undo log. It is reference-counted via `Arc<RwLock<Buffer>>` so that LSP, file IO, and the editor can hold handles.
- `BufferId` is a `Uuid` assigned on creation.
- `BufferHandle` is a typed wrapper around `Arc<RwLock<Buffer>>` for clarity.

### 3.3 Workspace

```rust
pub struct Workspace {
    pub root: PathBuf,
    pub open_files: Vec<PathBuf>,         // last-known open file set
    pub active_path: Option<PathBuf>,
}
```

`Workspace` is what `Open Folder` produces. It is the unit of "session" the user can return to.

### 3.4 Undo/redo

Each `Buffer` has an `UndoLog` (a `Vec<Edit>` with a cursor). Edits coalesce if adjacent and within 500ms. There is **no** group-undo across buffers in Lite; cross-buffer undo is v2.

### 3.5 Selection model

- `Selection` is a `Range<Position>` plus an `Affinity` (forward/backward) for "drag direction".
- A buffer can have multiple cursors (basic multi-cursor: Ctrl+D in Lite). Phase 2 decision.

---

## 4. File and workspace model

### 4.1 Opening a file

1. UI fires `Effect::OpenFile(path)`.
2. Runtime task: `tokio::fs::read_to_string(path)` → if `> 4MB`, reject with toast "file too large for Lite".
3. If success, runtime creates a `Buffer`, assigns a `BufferId`, fires `Effect::LspOpen(...)` (if a server is registered for the language), and emits an internal `AppMessage` that the main thread maps to "open this buffer in the active editor".
4. LSP round-trip is async; the editor opens immediately with no LSP data. Diagnostics arrive later and overlay on top.

### 4.2 Saving a file

1. UI fires `Effect::SaveBuffer(id)`.
2. Runtime: serialize buffer text → `tokio::fs::write(path, text)` (atomic via temp+rename).
3. Fire `Effect::LspSave(id, path)` so LSP can do a workspace-wide refresh.
4. Main thread marks buffer clean.

### 4.3 Open Folder

1. UI fires `Effect::OpenFolder(path)`.
2. Runtime: `ignore::WalkBuilder` over `path` with `.gitignore` respected, hidden dirs skipped.
3. Returns a `FileTreeNode` rooted at the folder, lazily expanded.
4. UI shows the file tree in the left sidebar.

### 4.4 File tree virtualization

- The tree is rendered as a flat list of `(depth, name, is_dir, is_expanded)` rows.
- For folders with > 2000 children, the runtime returns a partial tree and the UI shows a "Load more" affordance. (Don't try to render 50k nodes — that is a v2 problem.)

### 4.5 File watching (Lite)

- Lite **does not** watch the filesystem. The tree is refreshed on `Open Folder` and on explicit user action.
- Why: cross-platform file watching is its own tar pit. We can add `notify` in a later phase. The plan acknowledges this as a documented v1.x feature, not a Lite omission to be ashamed of.

---

## 5. LSP client design

### 5.1 One actor per server

Each LSP server is a long-lived child process with a single actor that owns:
- `Child` (stdin/stdout/stderr)
- An outgoing message queue (so the editor can call `client.hover(...)` without blocking)
- A pending-request table (so we can route responses to the right `request_id`)

The actor is a `tokio::select!` loop over:
- An `mpsc::Receiver<LspRequest>` (incoming requests from the editor)
- The child's stdout (framed JSON-RPC messages)

### 5.2 Message framing

LSP uses `Content-Length: N\r\n\r\n<body>`. We use a small framing helper:

```rust
async fn read_lsp_message<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<LspMessage, io::Error> { /* … */ }
```

~40 LOC. Reused for every server.

### 5.3 Server registration

A static `LspRegistry` knows how to spawn known servers:

| Language | Command | Args |
|---|---|---|
| Rust | `rust-analyzer` | (none) |
| TypeScript / JavaScript | `typescript-language-server` | `--stdio` |

The registry reads from settings (`lsp.rust_analyzer_path`, `lsp.typescript_server_path`) with these defaults. If the binary is not on PATH, the user gets a one-time toast: "rust-analyzer not found; install it or set the path in settings".

### 5.4 Capabilities we exercise in Lite

- `textDocument/didOpen`, `didChange`, `didSave`, `didClose`
- `textDocument/hover` → on Ctrl+K or hover gesture
- `textDocument/definition` → F12
- `textDocument/completion` → on `.` (member access) or Ctrl+Space
- `textDocument/publishDiagnostics` → automatic overlay
- `window/workDoneProgress` → show in status bar (e.g. "rust-analyzer: indexing 42%")

Everything else (formatting, refactor, code action) is v1.x.

### 5.5 Stale request handling

If a request is canceled (e.g. user moved cursor before hover resolved), the response is dropped silently. The pending table is keyed by request id, so we can just dequeue and ignore.

### 5.6 Server crashes

If the child's stdout closes unexpectedly, the actor emits `LspMessage::ServerDied(server_name)`. The main thread surfaces a toast and disables the server until the user restarts it (button in the status bar).

---

## 6. Networking layer

### 6.1 The SeloraClient

A single struct in `selora-ai`:

```rust
pub struct SeloraClient {
    base_url: String,           // "https://api.selora.lol"
    api_key: String,            // pulled from keyring on construct
    http: reqwest::Client,
}
```

- `reqwest::Client` is constructed once with a 60s connect timeout and 5min read timeout (SSE can stream for a long time).
- rustls is used for TLS (no OpenSSL on Windows).

### 6.2 List models

```rust
pub async fn list_models(&self) -> Result<Vec<Model>, SeloraError> {
    let resp = self.http.get(format!("{}/v1/models", self.base_url))
        .bearer_auth(&self.api_key)
        .send().await?;
    let body: ModelsResponse = resp.json().await?;
    Ok(body.data)
}
```

### 6.3 Streaming chat (SSE)

The streaming flow uses `reqwest::Response::bytes_stream()` and a small parser:

```rust
pub async fn stream_chat(
    &self,
    req: ChatRequest,
    tx: mpsc::UnboundedSender<ChatEvent>,
) -> Result<ChatReceipt, SeloraError> {
    let resp = self.http.post(format!("{}/v1/chat/completions", self.base_url))
        .bearer_auth(&self.api_key)
        .json(&req.with_stream(true))
        .send().await?;

    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::new();
    while let Some(chunk) = stream.try_next().await? {
        buffer.extend_from_slice(&chunk);
        while let Some(event) = parse_sse_event(&mut buffer)? {
            match event {
                SseEvent::Data(s) if s == "[DONE]" => {
                    let receipt = build_receipt(&req);
                    let _ = tx.send(ChatEvent::Done(receipt));
                    return Ok(receipt);
                }
                SseEvent::Data(s) => {
                    let chunk: ChatChunk = serde_json::from_str(&s)?;
                    if let Some(delta) = chunk.choices.first().and_then(|c| c.delta.content.as_ref()) {
                        let _ = tx.send(ChatEvent::Delta(delta.clone()));
                    }
                }
                SseEvent::Event(_) | SseEvent::Comment(_) => {}
            }
        }
    }
    Err(SeloraError::Ai("stream closed before [DONE]".into()))
}
```

`parse_sse_event` consumes a complete event from the buffer and returns the remainder. It's ~80 LOC, no extra crate.

### 6.4 Cancellation

Each chat request has a `Uuid`. The runtime tracks `(uuid, JoinHandle)` and can call `handle.abort()` on cancel. The SSE parser doesn't care; it just stops getting data and the function exits.

### 6.5 Retries

- Network errors → 1 retry with 500ms backoff.
- HTTP 429 / 5xx → up to 3 retries with exponential backoff (1s, 2s, 4s) capped at 8s.
- HTTP 401 → no retry; surface "key invalid" toast.
- HTTP 4xx (other) → no retry; surface the message from the gateway.

### 6.6 Cost / usage data

The receipt is computed locally from the streamed chunks: count `prompt_tokens`, `completion_tokens` from the final chunk (or fall back to a character/4 estimate if the gateway doesn't return them in Lite). The cost in USD is computed from a static `Model::price_per_1k` table, which is fetched from the gateway and cached in settings. The cost meter shows `Σ today` and `current request` in the status bar.

---

## 7. Keyring integration

### 7.1 Storage

```rust
const SERVICE: &str = "selora-ide";
const ACCOUNT: &str = "default";

pub struct KeyringStore { /* … */ }

impl KeyringStore {
    pub fn get(&self) -> Result<Option<String>, SeloraError>;
    pub fn set(&self, key: &str) -> Result<(), SeloraError>;
    pub fn clear(&self) -> Result<(), SeloraError>;
}
```

The `keyring` crate uses Windows Credential Manager on Windows. Keys are written as `Generic` credentials with the `service` and `account` set as above. The key is never written to a file, never logged, never returned by the public `Settings` struct.

### 7.2 Key validation on connect

When the user pastes a key, the runtime fires:
1. `KeyringStore::set(key)`
2. `SeloraClient::list_models()` as a sanity check
3. If `200 OK` → status = `Connected`, drop the in-memory copy
4. If error → status = `Error("invalid key")`, leave the key in the keyring (so user can fix the gateway, not retype)

### 7.3 Account sign-in (Phase 7) — the gateway contract

Phase 7 replaces key-pasting for humans with an OAuth-style login; the
client-side spec lives in ROADMAP.md Phase 7 (decision table included). This
section is the wire contract the gateway must implement — the client codes
against it and wiremock-tests it before the gateway ships. Both auth modes end
in the same `Authorization: Bearer` header `SeloraClient` already sends, so
`sk-gw-` keys and account tokens are interchangeable below the auth layer.

**Keyring layout (extends §7.1):** the key stays under entry `api-key`; the
session lives under a second entry, `auth-session`, holding one JSON blob
`{ "refresh_token": …, "account": { "id", "email", "plan" }, "expires_at": … }`.
The **access token is never persisted anywhere** — memory only, and its
`Debug`/log rendering is redacted exactly like the key's. One auth mode is
active at a time: a successful sign-in clears `api-key`, a key paste clears
`auth-session`.

**The flow (Authorization Code + PKCE, RFC 8252 native-app BCP):**

1. The client generates a 43–128-char random `code_verifier` (memory only) and
   its S256 `code_challenge`, plus a random `state`; binds a loopback listener
   on `127.0.0.1:{ephemeral_port}`; and opens the **system browser** (the app
   never renders the login page — it must never see the account password) at:

   `GET {base}/oauth/authorize?response_type=code&client_id=selora-ide&redirect_uri=http://127.0.0.1:{port}/callback&state={state}&code_challenge={challenge}&code_challenge_method=S256&scope=chat+models+usage`

2. The gateway shows its consent page and redirects the browser to the
   loopback URI with `?code=…&state=…`. The listener accepts **exactly one
   request** (validating `state`), answers with a plain "you can close this
   tab" page, and closes — hard 5-minute timeout either way.

3. The client exchanges the code:

   `POST {base}/oauth/token` (form-encoded) —
   `grant_type=authorization_code&code=…&redirect_uri=…&client_id=selora-ide&code_verifier=…`

   ```json
   { "access_token": "…", "token_type": "Bearer", "expires_in": 3600,
     "refresh_token": "…", "scope": "chat models usage",
     "account": { "id": "…", "email": "…", "plan": "…" } }
   ```

4. **Refresh (rotating):** `POST {base}/oauth/token` with
   `grant_type=refresh_token&refresh_token=…&client_id=selora-ide` returns a
   *new* refresh token and invalidates the old one (single-use). The client
   persists the new token to the keyring **before** first use of the new
   access token; a rejected refresh drops the session to `Disconnected`
   ("sign in again") — never a retry loop. Refresh fires proactively when a
   token is < 60 s from expiry; a 401 triggers one refresh-and-retry, then the
   same signed-out path.

5. **Sign-out:** `POST {base}/oauth/revoke` (Bearer) with the refresh token,
   then the keyring entry is deleted. Offline sign-out deletes locally and
   revokes on next contact.

6. **Usage feed:** `GET {base}/v1/usage` (Bearer) →

   ```json
   { "plan": "…", "spent_usd": 12.40, "limit_usd": 50.0, "period_reset": "2026-11-01" }
   ```

   Fetched on the existing 30s status-ping cadence when signed in, plus once
   after connect and after each chat/agent turn. The limit is **advisory
   display only** — enforcement stays server-side; client-side caps
   (`agent.*`) remain the user's own local guardrails.

**Fallbacks:** the device-code grant (RFC 8628) is a documented post-7
addition for RDP/locked-down environments; raw key paste (§7.1–7.2) remains
supported permanently and is the automation/CI path.

**Invariants (extend §7.1):** verifier, state, and access token are memory
only; the refresh token exists only in the keyring; none of them ever appear
in `settings.json`, `workspace.json`, logs, tool results, or the bash child
environment (the ENV_ALLOWLIST keeps them structurally absent); the loopback
listener binds 127.0.0.1 only and lives at most 5 minutes.

---

## 8. Persistence

### 8.1 settings.json

```json
{
  "_v": 1,
  "editor": {
    "font_family": "JetBrains Mono",
    "font_size": 14,
    "tab_size": 4,
    "line_numbers": true,
    "minimap": false,
    "word_wrap": false
  },
  "ui": {
    "theme": "selora-dark",
    "sidebar_width": 240,
    "chat_sidebar_width": 360
  },
  "lsp": {
    "rust_analyzer_path": null,
    "typescript_server_path": null
  },
  "selora": {
    "base_url": "https://api.selora.lol",
    "active_model": null
  },
  "keybinds": {
    "command_palette": "Ctrl+Shift+P",
    "fuzzy_find": "Ctrl+P",
    "toggle_chat": "Ctrl+L",
    "save": "Ctrl+S"
  }
}
```

`_v` is the schema version. On read, if `_v != 1`, run a migration (or wipe with a toast).

### 8.2 Workspace state

Persisted on `graceful close` and every 5s while running. On startup, restored if a path matches an existing folder. Otherwise, ignored.

### 8.3 Logging

- `tracing` + `tracing-subscriber` with env filter.
- Default `RUST_LOG=info,selora_ide=debug`.
- Daily rotated files in `%LOCALAPPDATA%\selora-ide\logs\`.
- Every log line that includes the API key redacts it (`[REDACTED]`).

---

## 9. Editor pane (custom rendering)

egui's default text widget is good for static text, not for an editor. We paint the editor ourselves.

### 9.1 Components

- **Buffer view** — reads from `Buffer` (ropey), computes the visible line range.
- **Glyph cache** — uses `cosmic-text` (or `swash` directly) to shape glyphs, caches by `(font, glyph_id, subpixel_offset)`. Cache is an `egui::Texture` so the GPU upload is amortized.
- **Paint** — `egui::Painter::text(...)` with per-glyph positions for subpixel rendering. Selection is drawn as semi-transparent rectangles behind glyphs. Cursor is a 2px line.
- **Gestures** — mouse down → start selection. Drag → extend. Double-click → word select. Triple-click → line select. Wheel → scroll. The state machine is documented in code, not in a separate spec.

### 9.2 Ghost text

A ghost suggestion is stored as `Option<String>` in the `Editor` struct. When set, the editor paints the suggestion at the caret position in `ink-faint` (#3a3a3a). Tab accepts (insert into buffer + clear). Any other keypress clears it.

### 9.3 Gutter

- 4-char wide.
- Shows: line number, change marker (modified ●, saved ✓), git status (M/A/?/!).
- Click on gutter selects the line. Shift-click extends.

---

## 10. Crate internals — must-know contracts

### 10.1 selora-core

- `Buffer::insert(pos, text) -> Edit` — applies the edit, returns the inverse for the undo log.
- `Buffer::line_col(pos) -> (Line, Col)`, inverse `Buffer::position(line, col)`.
- No IO. No async. No UI. Pure logic.

### 10.2 selora-fs

- `pub fn build_tree(root: &Path) -> impl Stream<Item = FileTreeNode>` — async, uses `ignore`.
- `pub fn fuzzy_find(needle: &str, items: &[(PathBuf, String)]) -> Vec<Match>` — uses `nucleo-matcher`.
- `pub async fn search_in_files(root: &Path, query: &str) -> impl Stream<Item = SearchHit>` — uses `grep` (or `rg` if present) as a subprocess. **We shell out** because the rust port of ripgrep is a big dependency; shelling out is ~30 LOC and gives us the same UX for free.

### 10.3 selora-editor

- `pub struct Editor` (implements `egui::Widget`).
- Owns: `BufferHandle`, `EditorSettings`, gesture state, scroll state, ghost text.
- No IO. Receives diagnostics as `Vec<Diagnostic>` from the main thread.

### 10.4 selora-lsp

- `pub struct LspClient` — spawns a server, exposes typed methods: `did_open`, `did_change`, `hover`, `definition`, `completion`.
- All methods return `Receiver<Result<Response, SeloraError>>` or are fire-and-forget for notifications.

### 10.5 selora-git

- `pub fn open_repo(path: &Path) -> Result<GitRepo>`.
- `pub fn status(&self) -> GitStatus`.
- `pub fn stage(&self, path: &Path) -> Result<()>`.
- `pub fn unstage(&self, path: &Path) -> Result<()>`.
- `pub fn commit(&self, message: &str) -> Result<Oid>`.
- We use `git2` for these. `push` / `fetch` / `pull` are out of Lite.

### 10.6 selora-ai

- `pub struct SeloraClient` — see §6.
- `pub fn list_models(&self) -> impl Future<Output = Result<Vec<Model>>>`
- `pub fn stream_chat(&self, req: ChatRequest, tx: mpsc::UnboundedSender<ChatEvent>) -> impl Future<Output = Result<ChatReceipt>>`
- `pub fn ghost_complete(&self, prompt: &str) -> impl Future<Output = Result<Option<String>>>`
- `pub fn status(&self) -> SeloraStatus` — pings the gateway with a cheap HEAD, returns reachable/unreachable.

### 10.7 selora-settings

- `pub fn load() -> Result<Settings>` — reads from `%APPDATA%\selora-ide\settings.json`.
- `pub fn save(&self) -> Result<()>`.
- `pub fn default_path() -> PathBuf` — used by both.

### 10.8 selora-ui

- The top-level `UiApp` is an `eframe::App`.
- It owns `AppState`.
- Each frame: drain messages, process timeouts, paint.
- Layout: a `egui::SidePanel` for the file tree, a `CentralPanel` for editors, a right `SidePanel` for the chat sidebar (toggleable), a bottom panel for the git status / problems, a top `menu` bar, a `StatusBar` at the bottom.

### 10.9 selora-app

- `main.rs` only. Sets up tokio, builds state, hands off to eframe.

---

## 11. Data model (Selora-specific)

```rust
pub struct Model {
    pub id: String,                 // "claude-opus-5-5" etc.
    pub display_name: String,
    pub context_window: u32,
    pub price_per_1k_input: f64,    // USD
    pub price_per_1k_output: f64,
    pub supports_streaming: bool,
    pub supports_tools: bool,
    pub family: String,             // "claude" / "gpt" / "kimi" / "glm"
}

pub struct ChatMessage {
    pub role: Role,                 // System | User | Assistant
    pub content: String,
}

pub struct ChatRequest {
    pub model: ModelId,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub stream: bool,
}

pub enum ChatEvent {
    Delta(String),
    Done(ChatReceipt),
    Error(SeloraError),
}

pub struct ChatReceipt {
    pub request_id: Uuid,
    pub model: ModelId,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub duration_ms: u32,
}

pub enum SeloraStatus {
    Disconnected,
    Connecting,
    Connected { model: Option<ModelId>, latency_ms: u32 },
    Error(String),
}
```

---

## 12. Thread safety contracts

- `Buffer` is `Send + Sync` (via `Arc<RwLock<…>>`).
- `LspClient` actor owns its state; it is `!Send` internally, but its `tx` channel is `Send + Sync`.
- `SeloraClient` wraps a `reqwest::Client` which is `Send + Sync`; the struct is `Clone`.
- `AppState` is `!Send` and `!Sync`; the main thread is its only owner.
- `Settings` is `Send + Sync` (JSON struct, no interior mutability).
- `KeyringStore` is `Send + Sync`.

---

## 13. Testing strategy

- **Unit tests** in every crate, run with `cargo test --workspace`.
- **Integration tests** for `selora-ai` use a local mock gateway (a tiny axum server) that returns canned SSE. Verified in Phase 1 spike.
- **UI snapshot tests** are out of Lite scope (egui is hard to snapshot). The verification surface is "the app launches and the listed behavior works".
- **LSP tests** use a real `rust-analyzer` against a fixture workspace. If `rust-analyzer` is not on PATH in CI, that test is skipped (not failed).

---

## 14. Build profile and binary size

- `release` profile uses `lto = "thin"`, `codegen-units = 1`, `strip = true`, `opt-level = 3`, `panic = "abort"`.
- Target binary size goal: **< 25 MB** (egui is small; the bulk will be the bundled fonts at ~600 KB and a few megabytes of tree-sitter grammars).
- Bundled tree-sitter grammars are compiled in (not loaded at runtime) to avoid shipping `.so`/`.dll` files separately.

---

## 15. Reference-architecture study anchors

These are concrete files/dirs in the reference projects that build agents should **read for design study** (not copy).

| Concept | Zed study anchor | VS Code study anchor |
|---|---|---|
| Cargo workspace layout | `zed-industries/zed/crates/` (just the directory shape; the contents are GPUI-specific) | n/a |
| Multi-process / extension isolation | n/a (Zed is single-process for collab) | `microsoft/vscode/src/vs/workbench/services/extensions/` |
| LSP server wiring | `zed-industries/zed/crates/lsp/` (read for the actor pattern) | `microsoft/vscode/src/vs/workbench/api/browser/mainThreadLanguageModels.ts` (frontend side) |
| Command palette UX | `zed-industries/zed/crates/command_palette/` (study the fuzzy-match UX) | `microsoft/vscode/src/vs/workbench/contrib/quickaccess/` |
| File tree rendering | `zed-industries/zed/crates/project_panel/` (study the virtualization idea) | `microsoft/vscode/src/vs/workbench/contrib/files/` |
| Settings persistence | `zed-industries/zed/crates/settings/` (study the JSON + migration pattern) | `microsoft/vscode/src/vs/platform/userDataSync/` |
| Workbench layout | `zed-industries/zed/crates/workspace/` (study the pane model) | `microsoft/vscode/src/vs/workbench/browser/layout.ts` |

**We do not copy any of these files.** We study them and re-implement in our own style, with our own crate names, our own idioms.

### What we explicitly do NOT take from Zed

- GPUI itself.
- Their collaboration / CRDT engine.
- Their branding, assets, fonts beyond Space Grotesk / JetBrains Mono which are open-licensed anyway.
- Their `extensions/` model.
- Their telemetry infrastructure.

### What we explicitly do NOT take from VS Code

- Electron.
- TypeScript / JavaScript as the core implementation language.
- The extension marketplace (`extensions.gallery`).
- Theia / Monaco / Language Server Protocol only insofar as we implement the same standard — we are not copying their client.

---

## 16. What changes if egui becomes a ceiling

If the Lite Edition ships and we hit a real performance or rendering ceiling with egui (e.g. ghost text animation is janky, custom glyph layout is too constrained), the next move is **not** to write a GPUI clone. It is to:

1. Profile to confirm egui is the bottleneck (not, say, our layout).
2. Move just the editor pane to a lower-level renderer (raw OpenGL via `glow`, or `wgpu` directly), keeping egui for chrome.

This is documented as the v2 path. The Lite plan does not depend on it.

---

## 17. Agent Mode (the Claude Code-style tool loop)

Agent Mode turns the chat sidebar into an agent: the model receives a tool manifest with every request, may respond with `tool_calls`, and the IDE executes those tools and feeds results back into the conversation — streaming, in a loop, until the model produces a final answer with no tool calls. The model can read, edit, create, and delete files, run shell commands, and search/fetch the web. Every side-effecting action pauses the loop until the user approves it in the chat.

### 17.1 Tool set (Lite)

| Tool | Implementation | Approval |
|---|---|---|
| `read_file(path, start_line?, end_line?)` | runtime fs read, ≤4MB, path resolved against workspace root | none (read-only) |
| `list_dir(path)` | `selora-fs` tree node | none |
| `fuzzy_find(query)` | `selora-fs::fuzzy_find` over workspace files | none |
| `search_files(query)` | `selora-fs::search_in_files` (ripgrep if on PATH) | none |
| `write_file(path, content)` | create/overwrite via runtime fs write | **diff card** (create shown as all-additions) |
| `edit_file(path, old_str, new_str, replace_all?)` | exact-match string replace; applied to the open buffer if the file is open, then saved | **diff card** (unified diff) |
| `delete_file(path)` | runtime fs delete (git-tracked files recoverable via git) | **approval card** |
| `bash(command, timeout_secs?, cwd?)` | `tokio::process` spawn | **command card**, auto-run if allowlisted (§17.5) |
| `web_search(query)` | pluggable provider, default DuckDuckGo Lite HTML (no API key); Brave Search API supported via settings key | none |
| `web_fetch(url)` | reqwest GET → strip HTML to text, 256KB cap | none, SSRF-guarded |

### 17.2 Path and network safety (non-negotiable invariants)

- **Path rooting:** every path tool resolves against the workspace root. `..` traversal or an absolute path outside the root is rejected with a tool-result error (which the model sees and can correct). Escape outside the root is only possible with a per-session user opt-in.
- **SSRF guard on `web_fetch`:** only `http`/`https`; localhost, `127.0.0.0/8`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, and non-routable/reserved ranges are refused.
- **The API key never leaves the client.** It is never included in tool results, messages, bash environment, or logs.
- **No eval.** The model cannot execute code except through the `bash` tool, which is approval-gated.
- **Bounded I/O:** reads ≤4MB, fetch output ≤256KB, bash output ≤256KB (truncated with a notice in the tool result).

### 17.3 The loop

```
user sends agent message (chat composer; agent toggle is on)
  → ChatRequest { model, messages, tools: TOOL_MANIFEST, stream: true }
  → stream deltas render live in the chat bubble
  → finish_reason == "tool_calls"
  → for each tool_call, in order:
      read-type tool  → execute immediately in the runtime
      write/edit/delete → build diff/approval card → PAUSE loop → wait for user
      bash            → allowlist check → auto-run or command card → maybe PAUSE
  → results appended as { role: "tool", tool_call_id, content } messages
  → next ChatRequest with the grown conversation → repeat
  → finish_reason == "stop" → agent turn done → receipt → cost meter
```

**Loop caps (defaults, user-tunable in settings `agent.*`):** max 25 tool iterations per turn, max 10 minutes per turn, max `$1.00` estimated cost per turn. Any cap exceeded → abort the loop, surface a toast + final partial answer. A **Stop** button on the streaming bubble aborts immediately, killing any running bash child.

### 17.4 Approval protocol

- While the loop is paused on an approval, the runtime holds the conversation in an `AgentTurn` state machine (`Thinking → RunningTool(tool) → AwaitingApproval(id) → RunningTool → … → Done`).
- The UI renders an approval card (UI_SPEC §4.2) with the diff or command. The user's decision returns to the runtime as `Effect::AgentDecision(turn_id, approval_id, Decision)` where `Decision ∈ { Approve, ApproveAllThisTurn, Deny, Stop }`.
- `ApproveAllThisTurn` pre-approves remaining writes and non-allowlisted bash for the rest of this turn only — never persisted across turns or sessions.
- A denied tool returns a tool-result of `"user denied this action"`, and the loop continues (the model usually adapts); `Stop` aborts.

### 17.5 The bash tool

- **Shell:** PowerShell (`pwsh` if installed, else `powershell`) on Windows; user-configurable in settings (`agent.shell`).
- **cwd:** workspace root (or the tool's `cwd` argument, still root-contained).
- **Allowlist (auto-run, no card):** read-only commands by prefix match — `git status`, `git diff`, `git log`, `git show`, `ls`, `dir`, `cat`, `type`, `Get-Content`, `cargo check`, `cargo build`, `cargo test`, `cargo clippy`, `cargo tree`, `node --version`, `rustc --version`, `where`, `Get-ChildItem`. The list is user-editable in settings (`agent.auto_approve_commands`). Everything else shows a command card.
- **Timeout:** default 120s per command, max 600s; on timeout the child is killed and the tool result reports it.
- **Environment:** a clean environment — the Selora key and any secrets from the app's process env are stripped from the child's env.
- **Output:** stdout+stderr combined, 256KB cap, truncated with a notice.

### 17.6 Tool plumbing in the existing types

The `Effect`/`AppMessage` enums grow (additive — existing variants untouched):

```rust
// Effect
AgentSend(ChatId, String),                      // starts a turn with tools armed
AgentDecision(ChatId, ApprovalId, AgentDecision),

// AppMessage
ToolCallStarted(ChatId, ToolCall),
ToolResult(ChatId, ToolCallId, ToolResult),
ApprovalNeeded(ChatId, ApprovalId, ApprovalCard),  // diff card / command card / delete card
AgentTurnDone(ChatId, ChatReceipt, AgentSummary),  // iterations, tools used, cost
```

`ChatRequest` gains `tools: Option<Vec<ToolSpec>>`; `ChatEvent` gains `ToolCallDelta` (streaming partial tool-call JSON, same accumulate-then-parse pattern as content deltas). Tool execution lives in one `agent` module in `selora-ai` — **it does not get its own crate**; it composes `selora-core`, `selora-fs`, and the runtime, and is the only module allowed to spawn bash children.

### 17.7 Interaction with open buffers

`edit_file`/`write_file` on a file that is open in an editor: the edit is applied to the in-memory buffer (so the user sees it live and undo works), the buffer is marked dirty, and saving is the normal user flow (the agent does not silently save; the diff card is the save moment — approving it writes the file to disk). Edits to closed files write directly to disk with the same diff card. This keeps a single source of truth (the buffer) and makes agent edits undoable with Ctrl+Z like any human edit.

### 17.8 What Agent Mode is NOT (still non-goals)

- No background/unattended agents — the loop only runs while the user is watching.
- No sandboxing (approval gates are the trust boundary; container isolation is v1.x).
- No agent-initiated git operations beyond read-only git commands through `bash` (no agent commit/push).
- No MCP/external tool servers — the manifest is fixed, defined by us, versioned with the app.
