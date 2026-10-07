# Selora IDE — Roadmap (Lite Edition)

This is the build contract. Each phase below is a discrete unit of work a single subagent can run cold. Each phase has:

- **Goal** — the one thing the phase exists to prove
- **Inputs** — what the subagent must read or have access to before starting
- **Tasks** — concrete deliverables
- **Subagent brief** — the literal prompt to give a subagent
- **Acceptance tests** — the runnable commands / observable behaviors that prove the phase is done
- **Estimated complexity** — small / medium / large
- **DONE means** — the exact exit condition for the merge gate

**Phase 1 is the risk spike** — it must prove the UI framework + text buffer + live Selora SSE call all work in one window before any other work is allowed. If Phase 1 fails, the project fails fast and we pivot, not after we have built 10 other features on a broken foundation.

---

## Phase 0 — Foundation

**Goal:** Repo skeleton, CI green, Selora design tokens compiled in, hello-window with a title that says "Selora IDE".

**Why first:** Every other phase builds on the workspace, the theme module, the CI, and the app boot sequence. None of that is unique enough to defer.

**Inputs:**
- `PLAN.md`, `ARCHITECTURE.md`, `DESIGN_TOKENS.md`, `UI_SPEC.md` (this plan pack)

**Tasks:**
1. Create a Cargo workspace at the repo root with members for all 9 crates (see `PLAN.md` §5).
2. Pin `rust-toolchain.toml` to a stable version (the latest stable at build time, e.g. `1.84.0` or whatever is current).
3. Add `LICENSE` (MIT), `README.md`, `CONTRIBUTING.md`, `.gitignore` (Rust template), issue + PR templates.
4. Implement `crates/selora-ui/src/theme.rs` exactly per `DESIGN_TOKENS.md` §11.
5. Add a `set_fonts(ctx)` function that loads Space Grotesk + JetBrains Mono from `assets/fonts/`.
6. In `selora-app/src/main.rs`: tokio runtime, eframe harness, a `SeloraApp` that paints the brand mark + "Selora IDE" title in the title bar.
7. Add `.github/workflows/ci.yml`: `cargo build --workspace`, `cargo test --workspace`, `cargo fmt --check`, `cargo clippy -- -D warnings`. Runs on `windows-latest`.
8. Add `assets/fonts/`: bundle Space Grotesk Regular/Medium/SemiBold + JetBrains Mono Regular/Medium. (Confirm OFL license and re-distribute per the OFL terms.)
9. Add `assets/icons/selora-ide.ico` (Windows), `selora-ide-256.png`, `selora-ide-512.png`, `selora-ide-mono.svg`.

**Subagent brief (paste verbatim):**

```
You are Phase 0 of the Selora IDE build. Your job is to lay the foundation.

READ FIRST, in full, before writing any code:
- C:\Users\Toxic\Desktop\Selora IDE\PLAN.md
- C:\Users\Toxic\Desktop\Selora IDE\ARCHITECTURE.md
- C:\Users\Toxic\Desktop\Selora IDE\DESIGN_TOKENS.md (especially §11)
- C:\Users\Toxic\Desktop\Selora IDE\UI_SPEC.md (especially §1)

DELIVERABLES:
1. Cargo workspace at C:\Users\Toxic\Desktop\Selora IDE\ with all 9 crates from PLAN.md §5.
2. crates/selora-ui/src/theme.rs implemented EXACTLY as DESIGN_TOKENS.md §11 specifies.
3. selora-app/src/main.rs boots eframe, paints a window with title "Selora IDE" in Space Grotesk, the brand mark on the left, and an otherwise empty CentralPanel using the ink-canvas background.
4. .github/workflows/ci.yml that builds + tests on windows-latest.
5. LICENSE (MIT), README.md (project description, build instructions, link to PLAN.md), CONTRIBUTING.md (refer to AGENT_ORCHESTRATION.md for agent instructions), .gitignore.
6. assets/fonts/ with Space Grotesk Regular, Medium, SemiBold and JetBrains Mono Regular, Medium as TTF files. If you cannot fetch these in this environment, leave a placeholder NOTE.md explaining where to download them and how to verify the OFL license.
7. assets/icons/ with selora-ide-mono.svg (the orbital planet mark — design and ship a simple SVG that matches the description in DESIGN_TOKENS.md §9). For .ico and .png, leave a NOTE.md explaining they can be generated from the SVG.

DO NOT:
- Write any feature code (no file tree, no editor, no chat, no settings).
- Add a webview. Pure Rust + egui only.
- Use any color or font outside DESIGN_TOKENS.md.

ACCEPTANCE:
- `cargo build --workspace` succeeds.
- `cargo run -p selora-app` opens a 1280x800 dark window titled "Selora IDE" with the brand mark on the left and nothing else.
- `cargo test --workspace` runs (even with 0 tests, it must not fail).
- GitHub Actions CI is green on first push.

WHEN DONE, report:
- The exact commands you ran and their outputs.
- Any deviations from the spec and why.
- The next-phase blocking issues (if any).
```

**Acceptance tests:**
```bash
cd "C:\Users\Toxic\Desktop\Selora IDE"
cargo build --workspace
cargo run -p selora-app
# verify: window opens, title bar reads "Selora IDE", background is #000000
cargo test --workspace
# verify: all tests pass (zero is fine)
cargo fmt --check
cargo clippy --workspace -- -D warnings
```

**Estimated complexity:** Medium (CI setup + workspace boilerplate is straightforward; the brand mark SVG needs care to look right at all sizes).

**DONE means:** A dark window with "Selora IDE" opens, the theme module is complete, CI is green, and the workspace structure matches `PLAN.md` §5.

---

## Phase 1 — Risk Spike

**Goal:** Prove — in one window, in one session — that the three hardest technical bets all work:
1. The UI framework (egui) is suitable for our needs.
2. A real text buffer (ropey) can be edited with a custom rendering layer.
3. A live `POST https://api.selora.lol/v1/chat/completions` SSE stream can be consumed and displayed.

This phase ships a "tech demo window": open a temp file, type into it, click a "test selora" button, see a real chat reply stream in a side panel. No polish, no production UI, no error handling beyond "show the error". Just the three pieces talking to each other.

**Why second:** If egui chokes on text editing, or ropey is wrong, or the SSE parser is fragile, we find out *now*, not after building 10 features on top. The risk is concentrated here so the rest of the project can move in parallel with confidence.

**Inputs:**
- Phase 0 merged.
- A real `sk-gw-…` API key (the owner will paste it into the demo for the verification step).

**Tasks (each can be a parallel subagent — see `AGENT_ORCHESTRATION.md`):**
- **1a — Custom editor pane:** Build `crates/selora-editor` with a minimal `Editor` widget. Uses `ropey` for the buffer. Renders text on `egui::Painter` (no `egui::TextEdit` for the actual content). Supports: insert at cursor, backspace, arrow keys, mouse click to position cursor, basic selection with drag, scroll with wheel. Painted in JetBrains Mono 14px on `--ink-canvas` with a 44px gutter showing line numbers in `--fg-faint`.
- **1b — Selora API client + SSE:** Build `crates/selora-ai` with `SeloraClient` (reqwest + rustls), `list_models`, `stream_chat` (with manual SSE parser), `ChatEvent` types. No UI yet — just a function that, given a key and a prompt, returns an `mpsc::Receiver<ChatEvent>` and a future that resolves to a `ChatReceipt`. Mocked tests in `selora-ai` use `wiremock` to simulate the gateway.
- **1c — Spike app:** In `selora-app`, build a 3-region layout: a `CentralPanel` with the editor from 1a, a right `SidePanel` with a "Test Selora" button, a single-line prompt input, and a "stream output" `TextEdit` (read-only) that appends `ChatEvent::Delta` chunks. A top menu with a "Connect Selora key" field that calls `SeloraClient::list_models` as validation. Status indicator that reads `SeloraStatus` and renders the brand-mark-style dot.

**Subagent brief (paste verbatim):**

```
You are Phase 1c of the Selora IDE build. Phase 1a (editor widget) and 1b (selora-ai client) are running in parallel sibling subagents. Your job is to wire them together in a tech-demo window.

READ FIRST, in full:
- C:\Users\Toxic\Desktop\Selora IDE\PLAN.md
- C:\Users\Toxic\Desktop\Selora IDE\ARCHITECTURE.md (especially §6 Networking, §10 selora-ai, §10 selora-editor)
- C:\Users\TOXIC\Desktop\Selora IDE\DESIGN_TOKENS.md (only the color + type tokens you need)
- C:\Users\TOXIC\Desktop\Selora IDE\UI_SPEC.md (only §1 + §2 + §4 — just enough to draw the panels)

THE TWO DEPENDENCIES YOU CONSUME:
- crates/selora-editor — exposes `Editor` (an egui widget). Methods: `Editor::new(buffer: Arc<RwLock<Buffer>>)`, `Editor::set_text(&mut self, s: &str)`, `Editor::text(&self) -> String`. You do NOT need to know about ropey internals.
- crates/selora-ai — exposes `SeloraClient::new(base_url, api_key) -> Self`, `client.stream_chat(req, tx)`, `ChatEvent::{Delta(String), Done(ChatReceipt), Error(SeloraError)}`, `client.list_models() -> Vec<Model>`.

DELIVERABLES:
1. selora-app/src/main.rs builds a window that:
   - Has a left CentralPanel containing the selora-editor Editor, pre-loaded with a temp file's text.
   - Has a right SidePanel with: a "Selora key" text field, a "Connect" button, a "Test prompt" text field, a "Send" button, and a read-only "Stream output" TextEdit that grows as deltas arrive.
   - On "Connect": writes the key into a `SeloraClient`, calls `list_models`, shows "connected (N models)" or "error: <message>".
   - On "Send": builds a `ChatRequest` with the prompt as the user message, calls `stream_chat`, appends each Delta to the stream output, and on Done shows "tokens: X / cost: $Y".
   - NO THEMING, NO POLISH, NO PERSISTENCE. This is a tech demo.
2. The three subagents (1a, 1b, you) must coordinate via the crate APIs in ARCHITECTURE.md §10. If the APIs don't match, you negotiate with the other subagents and update ARCHITECTURE.md if the change is significant. If you cannot agree, escalate to the human.
3. At the end, push your branch and report the run instructions.

DO NOT:
- Add any chrome beyond the spike window.
- Add settings, persistence, keyring, or installation.
- Use a webview. SSE is in-process via reqwest.

ACCEPTANCE (the human will run this with a real sk-gw- key):
- `cargo run -p selora-app` opens the spike window.
- Typing in the editor updates the buffer.
- Pasting a real sk-gw- key, clicking Connect, then sending a prompt "say hi" produces real streaming text in the stream output panel within 5 seconds.
- Closing the window does not crash.
- The whole build compiles on windows-latest.

WHEN DONE, report:
- The exact prompt you sent and the first 200 chars of the streamed response.
- Any architectural changes you had to negotiate.
- Whether you think egui / ropey / reqwest are fit for the Lite Edition.
```

**Acceptance tests (run by the human, not by you):**
```bash
cargo run -p selora-app
# 1. window opens, editor pane takes the left 70%
# 2. type "hello world" into the editor, the buffer contains "hello world"
# 3. paste a real sk-gw-… key, click Connect, status reads "connected (N models)"
# 4. type "say hi" in the prompt, click Send
# 5. within 5s, the stream output shows real text streaming in
# 6. on Done, the output shows "tokens: <real number> / cost: $<real number>"
# 7. close the window — clean exit
```

**Estimated complexity:** Large (this is the hardest phase; three concurrent subagents, all new code).

**DONE means:** The spike window works end-to-end with a real Selora key. The architecture for the rest of the project is validated. The three subagents have merged.

---

## Phase 2 — Editor core

**Goal:** Multi-tab, multi-pane, file tree, fuzzy finder, syntax highlighting, command palette, settings persistence. The editor *feels* like an editor.

**Inputs:**
- Phase 1 merged (selora-editor, selora-ai stable; selora-app boots; design tokens are real).

**Tasks (parallel subagents — see `AGENT_ORCHESTRATION.md`):**

- **2a — Tabs + splits:** Build the tab bar (UI_SPEC §3.1), tab management in `AppState` (a `Vec<Tab>` + active index), split editor (UI_SPEC §3.3) with `Ctrl+\`, close tab (`Ctrl+W`), open recent files dropdown.
- **2b — File tree:** Build `selora-fs::FileTree` with `ignore::WalkBuilder`. UI per UI_SPEC §2.2. Activity bar (§2.1) with 4 icons. Open Folder action. New File, Rename, Delete context menu. Toggle visibility with `Ctrl+B`.
- **2c — Fuzzy finder:** Build `selora-fs::fuzzy_find` with `nucleo-matcher`. Wire `Ctrl+P` (UI_SPEC §5.1) and `Ctrl+Shift+O` (symbols — uses tree-sitter queries in 2d for the active file).
- **2d — Syntax highlighting:** Build `selora-editor` syntax layer. `tree-sitter` + `tree-sitter-highlight` for Rust, TypeScript, JavaScript, Python, JSON, TOML, Markdown, HTML, CSS. Incremental re-parse on edit. Highlight painter uses `theme::color::SYN_*` tokens. The 9 grammars are added as workspace deps with `tree-sitter-language` mapping crates.
- **2e — Command palette:** Build `selora-ui::command_palette` (UI_SPEC §5). Loads commands from a static registry. Fuzzy match via `nucleo-matcher`. Each command has a name, category, optional keybind hint, and a closure that fires an `Effect`.
- **2f — Settings:** Build `selora-settings` with JSON read/write at `%APPDATA%\selora-ide\settings.json`. Build the Settings dialog (UI_SPEC §6) — General, Editor, UI, Selora, LSP, Git, Keybinds. The Selora section shows connection status and a "Connect" flow that calls `KeyringStore::set` then `SeloraClient::list_models`.

**Subagent brief template (e.g. 2d syntax highlighting):**

```
You are Phase 2d of the Selora IDE build (syntax highlighting). 2a, 2b, 2c, 2e, 2f are running in parallel sibling subagents. Your job is to add real syntax highlighting to the editor.

READ FIRST:
- C:\Users\Toxic\Desktop\Selora IDE\PLAN.md
- C:\Users\Toxic\Desktop\Selora IDE\ARCHITECTURE.md (§9 Editor pane, §10 selora-editor)
- C:\Users\TOXIC\Desktop\Selora IDE\DESIGN_TOKENS.md (§1.5 Syntax palette)
- C:\Users\ToXic\Desktop\Selora IDE\UI_SPEC.md (§3.2 Editor canvas)

DELIVERABLES:
1. Add to crates/selora-editor: a SyntaxLayer that wraps a tree-sitter Parser and a tree-sitter-highlight Highlighter.
2. Languages supported in Lite: Rust, TypeScript, JavaScript, Python, JSON, TOML, Markdown, HTML, CSS. Use the official `tree-sitter-<lang>` crates plus `tree-sitter-highlight`.
3. The highlight runs incrementally: only re-parse changed ranges on edit. For buffers over 50k lines, fall back to full re-parse every 500ms (debounced).
4. The Editor widget paints spans using theme::color::SYN_* tokens per the palette in DESIGN_TOKENS.md §1.5. No saturated colors.
5. Comments in every language are italic (use egui's `JobFontFamily::Monospace` + the comment color). Use `egui::RichText::italics()` for the comment color spans.
6. Add 3 unit tests that parse small Rust/TS/Python files and assert the expected highlight scopes are produced.

DO NOT:
- Add custom theme colors outside DESIGN_TOKENS.md §1.5.
- Add a separate syntax theme file. The theme is the source of truth.
- Try to be VS Code (no semantic highlighting, no bracket pair colorization in Lite).

ACCEPTANCE:
- Open a 200-line Rust file in the editor. `pub fn` appears in `--syn-fn` (white), `fn` keyword in `--syn-keyword` (light gray), strings in `--syn-string` (dim), `//` comments in italic `--syn-comment` (faint).
- Open a TypeScript file with `interface` and `class`. Same correctness.
- Edit a line — the change re-highlights within 50ms (visible to the user as no perceptible delay).
- `cargo test -p selora-editor` passes.
- `cargo build --workspace` passes.
```

**Acceptance tests (after merge of all 2a-2f):**
```bash
cargo run -p selora-app
# 1. File → Open Folder → pick the selora-ide repo itself
# 2. The file tree populates with crates/, docs/, etc.
# 3. Click crates/selora-app/src/main.rs — opens in an editor
# 4. Rust syntax is highlighted
# 5. Ctrl+P — fuzzy finder opens, type "theme" — finds theme.rs
# 6. Ctrl+Shift+P — command palette opens, type "toggle chat" — Ctrl+L command visible
# 7. Ctrl+\ — splits the editor into two panes
# 8. Ctrl+, — Settings dialog opens; close it
# 9. Close the app, re-open — same folder is restored, last open file is restored
```

**Estimated complexity:** Large (six parallel subagents; the file tree + syntax highlighting are each substantial).

**DONE means:** An editor session with a real folder feels usable: navigate, open, edit, highlight, search, settings persist.

---

## Phase 3 — LSP & Git

**Goal:** Real language intelligence (rust-analyzer + typescript-language-server) wired in. Real git workflow (status, stage, commit) from a panel. Search in files working.

**Inputs:** Phase 2 merged.

**Tasks (parallel):**

- **3a — LSP client:** Build `selora-lsp` (LspClient, LspMessage, LspRegistry). Spawn rust-analyzer and typescript-language-server as child processes over stdio. Implement didOpen/didChange/didSave/didClose, hover, definition, completion, publishDiagnostics. UI hooks: F12 go-to-def, Ctrl+K Ctrl+I hover, Ctrl+Space completion, diagnostics squiggles (UI_SPEC §3.6). Status bar shows LSP server status (e.g. "rust-analyzer: indexing 42%"). Architecture: ARCHITECTURE.md §5.
- **3b — Git panel:** Build `selora-git` (git2-rs wrapper) + UI_SPEC §8 git panel. Open repo on Open Folder, status decorations in the file tree (M/A/?/D next to each path), stage/unstage buttons, commit panel with Ctrl+Enter. No push/pull/fetch in Lite.
- **3c — Search in files:** Build `selora-fs::search_in_files` that shells out to `ripgrep` if on PATH, else falls back to a built-in line scanner (slow but functional). UI_SPEC §9. Click result opens file at line. Update the activity bar to switch between File Tree / Search / Git.

**Subagent brief template (e.g. 3a LSP):**

```
You are Phase 3a of the Selora IDE build (LSP client). 3b and 3c are running in parallel. Your job is to add real language intelligence.

READ FIRST:
- C:\Users\Toxic\Desktop\Selora IDE\PLAN.md
- C:\Users\Toxic\Desktop\Selora IDE\ARCHITECTURE.md (§5 LSP — this is your spec)
- C:\Users\Toxic\Desktop\Selora IDE\UI_SPEC.md (§3.6 Diagnostics, §3.2 Editor canvas)

DELIVERABLES:
1. crates/selora-lsp/ implemented per ARCHITECTURE.md §5.
2. The LspClient actor pattern: one tokio task per server, owns the Child, frames stdin/stdout JSON-RPC, routes by request id.
3. The LspRegistry: knows how to spawn rust-analyzer and typescript-language-server from settings (default: PATH lookup).
4. Wire into selora-editor: editor emits didOpen on first paint, didChange on every commit (debounced 250ms), didSave on Ctrl+S, didClose when the buffer is closed.
5. Wire into AppState: diagnostics arrive on the AppMessage channel, get applied to the editor's overlay layer.
6. UI hooks: F12 = definition, Ctrl+K Ctrl+I = hover, Ctrl+Space = completion, hover gesture = hover (after 600ms hold).
7. Status bar shows LSP server status. rust-analyzer's workDoneProgress gets a "rust-analyzer: indexing 42%" text in the status bar center.

DO NOT:
- Add code action, refactor, rename, format-on-save. v1.x features.
- Add support for languages other than Rust and TypeScript in Lite. The registry is extensible but only those two are wired.
- Use tower-lsp. Write a thin client on lsp-types + tokio. ARCHITECTURE.md §5 explains why.

ACCEPTANCE:
- Open a Rust file in the selora-ide repo (e.g. crates/selora-core/src/buffer.rs).
- Wait for rust-analyzer to finish indexing (status bar shows "ready").
- Hover on a type name — within 500ms a tooltip with the type signature appears.
- F12 on a function call — within 1s the cursor jumps to the definition.
- Type a syntax error (e.g. `fn x(` with no closing paren) — within 1s a red squiggle appears under the bad token.
- Press Ctrl+Space after `let x: Vec<u8> = vec!` — completion popup with .new(), .with_capacity(), etc.
- `cargo test -p selora-lsp` passes (you write at least 3 tests: framing, request id routing, server crash recovery).
```

**Acceptance tests (after merge):**
```bash
cargo run -p selora-app
# 1. Open Folder → selora-ide repo
# 2. Open crates/selora-editor/src/lib.rs
# 3. Wait for "rust-analyzer: ready" in status bar
# 4. F12 on a function name → jumps to definition
# 5. Hover on a type → tooltip
# 6. Type a syntax error → red squiggle
# 7. Open the git panel (activity bar git icon)
# 8. See "Changes" section with modified files
# 9. Type a commit message, Ctrl+Enter, commit succeeds
# 10. Ctrl+Shift+F → search "ropey" → find all uses across the workspace
```

**Estimated complexity:** Large. LSP is the second-hardest piece (after the editor pane). The git2-rs crate is well-trodden but the UI wiring takes care.

**DONE means:** The editor understands code (definitions, hovers, completions, diagnostics) and the git workflow is real.

---

## Phase 4 — Selora AI

**Goal:** The differentiator. Live `api.selora.lol` integration: account connection, model picker, chat sidebar with streaming, ghost text, cost meter. This is what makes Selora IDE the editor born connected.

**Inputs:** Phase 3 merged. `selora-ai` from Phase 1 is already stable; this phase grows the UI on top of it.

**Tasks (parallel):**

- **4a — Account connection UI:** Settings → Selora section (already exists from Phase 2f, but expand it). Status indicator, "Connect" inline flow (paste key, validate via `list_models`, write to keyring). Status bar reflects connection state (UI_SPEC §1.2). Disconnect button.
- **4b — Model picker:** UI_SPEC §4.1. Live fetch on app boot, refresh on demand (button in the picker header). Cached in settings for offline display. Search/filter inside the dropdown.
- **4c — Chat sidebar:** UI_SPEC §4. Full chat panel: bubbles, streaming, code block copy buttons, conversation history per session, "clear conversation" action. Composer with Enter/Shift+Enter behavior. `Ctrl+L` toggle. Persists last N turns to workspace state.
- **4d — Ghost text inline completions:** Editor subscribes to `selora-ai::ghost_complete`. Debounce 350ms idle. Display as `--fg-faint` text + 2px `--ink-rule` underline. Tab to accept, any other keypress dismisses. The `comet` animation on first appearance.
- **4e — Cost meter:** Status bar right zone shows `Σ today $X.XX`. Per-request: when a chat or ghost-text request completes, the receipt is added to the running total. Per-model breakdown available in a tooltip.
- **4f — Connection status:** Live ping every 30s; "● selora connected · <model>" or "○ selora disconnected · connect" per UI_SPEC §1.2. On disconnect mid-chat, in-flight chats complete but new ones get a toast.

**Subagent brief template (e.g. 4c chat sidebar):**

```
You are Phase 4c of the Selora IDE build (chat sidebar UI). 4a, 4b, 4d, 4e, 4f are running in parallel. Your job is the chat panel itself.

READ FIRST:
- C:\Users\Toxic\Desktop\Selora IDE\PLAN.md
- C:\Users\Toxic\Desktop\Selora IDE\ARCHITECTURE.md (§6 Networking, §11 Data model)
- C:\Users\Toxic\Desktop\Selora IDE\DESIGN_TOKENS.md (§7 Motion for the wave glyph)
- C:\Users\Toxic\Desktop\Selora IDE\UI_SPEC.md (§4 Chat sidebar — this is your wireframe)

THE DEPENDENCY YOU CONSUME:
- crates/selora-ai — exposes `SeloraClient::stream_chat(req, tx)`, `ChatEvent::{Delta, Done, Error}`, `ChatReceipt`, `ChatRequest`, `ChatMessage`, `Model`.

DELIVERABLES:
1. crates/selora-ui/src/chat.rs implementing the chat sidebar per UI_SPEC §4.
2. The chat panel is a right SidePanel, toggled by Ctrl+L. Persisted visibility in workspace state.
3. Bubbles per spec: user (--ink-mid, right-aligned), assistant (--ink-low, left-aligned), streaming (same as assistant with comet-fade-in on first delta).
4. Code blocks inside assistant bubbles are detected (``` rust ... ```), rendered with --ink-canvas background, --ink-hi border, JetBrains Mono, with a "Copy" button in the top-right of the block (only on hover).
5. Composer: single-line input, Enter sends, Shift+Enter inserts newline. Send button on the right (⏎ glyph). Disabled when input empty.
6. The model picker dropdown in the header is implemented in Phase 4b; you wire to its API.
7. Conversations persist to workspace.json as the last 20 turns.

DO NOT:
- Add conversation branching, system-prompt editing, or tool use (v1.x).
- Add a custom font for chat; use --font-sans (Space Grotesk) for prose and --font-mono (JetBrains Mono) for code blocks.
- Use any saturated color outside --ok and --diag-err.

ACCEPTANCE:
- With a real sk-gw- key connected, click the chat icon in the activity bar.
- Type "write a haiku about rust", press Enter.
- Within 1s the streaming bubble appears; within 5s a 3-line haiku is fully visible.
- Each ```code block``` has a Copy button on hover; clicking puts the code on the clipboard.
- Switch the model via the model picker — next chat uses the new model.
- Reload the app — the last 20 turns are restored.
- `cargo test -p selora-ui chat` passes.
```

**Acceptance tests (after merge):**
```bash
cargo run -p selora-app
# 1. Status bar right says "○ selora disconnected" or "● selora connected"
# 2. If disconnected, open settings, paste a real sk-gw- key, click Connect
# 3. Status bar flips to "● selora connected · claude-opus-5-5" within 2s
# 4. Click the chat icon in the activity bar → chat sidebar appears
# 5. Type "list 3 prime numbers" → Enter
# 6. Within 5s, the response is fully streamed in
# 7. Open a Rust file. Stop typing for 500ms. Ghost text appears in --fg-faint.
# 8. Press Tab → ghost text is accepted into the buffer.
# 9. Check status bar right → cost meter reads a non-zero USD value
# 10. Switch model via picker → next chat uses new model
```

**Estimated complexity:** Large (six parallel subagents, but each is medium-small because the AI client is already stable from Phase 1).

**DONE means:** The IDE is a real client of Selora. Chat streams. Ghost text works. Cost is visible. The owner can demo this and it looks like a real product.

---

## Phase 5 — Agent Mode

**Goal:** The Claude Code parity feature. The model can read, create, edit, and delete workspace files, run shell commands, and search/fetch the web — through the Selora gateway, with every side-effecting action gated behind an approval card in the chat.

**Why its own phase:** It is the largest single feature in the product and the biggest trust surface. It composes everything from Phases 2–4 (buffers, file tree, search, streaming chat, cost meter) and adds the tool loop, the tool router, and the approval UI. It runs after Phase 4 so every tool it exposes sits on proven primitives.

**Inputs:**
- Phase 4 merged (streaming chat, model picker, receipts, cost meter all stable).
- `ARCHITECTURE.md` §17 (the tool loop spec) and `UI_SPEC.md` §4.2 (the card specs) — these are the contract.

**Tasks (parallel subagents — see `AGENT_ORCHESTRATION.md` §10 Phase 5):**

- **5a — Tool types + request plumbing:** extend `selora-ai` with `ToolSpec`, tool-call streaming (`ChatEvent::ToolCallDelta` with accumulate-then-parse), `tools` field on `ChatRequest`, and the tool-result message shape. Unit tests with the mock gateway: a scripted 3-turn conversation that exercises read → write → final-answer.
- **5b — Tool router + runtime executor:** the `agent` module in `selora-ai`: map tool calls to implementations over `selora-core`/`selora-fs`/runtime fs/`tokio::process`. Path rooting, SSRF guard, size caps, output truncation. All read tools fully implemented; write/bash/delete produce `ApprovalCard`s and pause the loop. Unit tests for every guard (path escape rejected, localhost fetch rejected, oversize read rejected).
- **5c — Agent loop controller:** the `AgentTurn` state machine (`Thinking → RunningTool → AwaitingApproval → … → Done`), loop caps (25 iterations / 10 min / $1), Stop button wiring, `ApproveAllThisTurn`. Turn summary. Tests: cap-abort, user-stop, deny-and-continue.
- **5d — Approval + tool cards UI:** chat sidebar renders tool cards, diff cards (unified diff with +/- coloring), bash command cards (live output streaming, exit status), web cards, the agent status line, the turn summary footer. `UI_SPEC.md` §4.2 is the wireframe.
- **5e — Web tools:** `web_search` (DuckDuckGo Lite default, no key; Brave API optional via settings) + `web_fetch` (HTML→text, 256KB cap, SSRF guard). Results as tool cards in the chat.

**Subagent brief (5c, paste verbatim):**

```
You are Phase 5c of the Selora IDE build (agent loop controller). 5a, 5b, 5d, 5e run in parallel. Your job is the loop that turns chat into an agent turn.

READ FIRST, in full:
- C:\Users\Toxic\Desktop\Selora IDE\ARCHITECTURE.md §17 (17.3 The loop, 17.4 Approval protocol, 17.5 bash — this is your spec)
- C:\Users\Toxic\Desktop\Selora IDE\UI_SPEC.md §4.2 (agent status line, Stop button, turn summary)
- C:\Users\Toxic\Desktop\Selora IDE\ROADMAP.md Phase 5

THE DEPENDENCIES YOU CONSUME:
- crates/selora-ai (Phase 5a): ChatRequest now has `tools: Option<Vec<ToolSpec>>`; ChatEvent has ToolCallDelta; the gateway conversation grows tool-result messages.
- crates/selora-ai agent module (Phase 5b): `execute_tool(call) -> ToolOutcome` where ToolOutcome is Ready(result) or NeedsApproval(card).

DELIVERABLES:
1. AgentTurn state machine in selora-ai/src/agent/turn.rs: Thinking → RunningTool → AwaitingApproval → RunningTool → … → Done.
2. The loop task: stream a ChatRequest; on finish_reason == "tool_calls", execute calls in order via 5b's router; on NeedsApproval, emit ApprovalNeeded and park the loop until Effect::AgentDecision arrives (tokio::mpsc, same pattern as the rest of the app).
3. Decision enum: Approve / ApproveAllThisTurn / Deny / Stop. ApproveAllThisTurn pre-approves remaining write/bash cards this turn only — never persisted.
4. Loop caps with abort: 25 tool iterations, 10 minutes, $1.00 estimated cost (from streamed receipts). On any cap: abort, surface the partial answer, toast the reason.
5. Stop button: kills any running bash child (the router exposes an abort handle), cancels the stream, ends the turn with a "stopped by user" summary.
6. Turn summary: iterations, tools used, approved/denied counts, tokens, cost.
7. Tests (mock gateway, no network): a 3-tool scripted conversation completes and produces a correct final message + summary; a cap-exceeded turn aborts; a Stop mid-tool aborts and kills the child; a Deny continues the loop with a "user denied" tool result.

DO NOT:
- Touch the chat UI (5d renders your states).
- Execute tools yourself — the router (5b) does. You only sequence and gate.
- Persist any approval across turns or sessions.
- Add tools to the manifest — the manifest is fixed by ARCHITECTURE.md §17.1.

ACCEPTANCE:
- cargo test -p selora-ai agent::turn passes with the four scenarios above.
- The four loop caps are configurable via settings `agent.*` and enforced in tests.
- No unbounded growth: conversation length is capped at the model's context window minus a 20% reserve; the turn aborts with a clear error if the context overflows.

WHEN DONE, report: commands run, outputs, deviations, open issues.
```

**Acceptance tests (after merge):**
```bash
cargo run -p selora-app
# 1. Chat sidebar: turn agent mode on. Ask: "read src/main.rs and summarize it"
# 2. A read_file card appears and completes; the model answers with real file content
# 3. Ask: "add a comment at the top of src/main.rs saying 'entry point'"
# 4. A diff card appears showing the + line; press Deny → model adapts, no file changed on disk
# 5. Ask again; press Approve → the file (open in editor) gains the comment, undoable with Ctrl+Z
# 6. Ask: "run cargo check"
# 7. If allowlisted, command card auto-runs with live output; else Approve first
# 8. Ask: "search the web for the ropey crate docs and summarize"
# 9. web_search + web_fetch cards appear; answer cites real fetched content
# 10. Mid-turn, press Stop → turn ends within 1s, "stopped by user" summary
# 11. Turn summary shows tools used, tokens, and a real cost
```

**Estimated complexity:** Large. The largest single phase after Phase 2.

**DONE means:** With a real Selora key and a real workspace open, the model autonomously reads files, proposes edits that the user approves as diffs, runs commands with output visible, and fetches the web — and every write on disk is traceable to an approved card.

---

## Phase 6 — Polish & Ship

**Goal:** First-run experience, splash, about, installer, CI release pipeline, docs, screenshots. The whole thing installs on a clean Windows VM and works.

**Inputs:** Phase 4 merged. Every Lite feature in scope works.

**Tasks (mostly sequential, a few parallel):**

- **6a — First-run + splash:** UI_SPEC §7. If no key in keyring on first launch, show the splash. The brand mark uses the `orbit` animation. After Connect, splash fades out with the `fade` animation. After Skip, the empty-state editor shows.
- **6b — About + check-for-updates:** UI_SPEC §11. About dialog with version, license, link to repo. "Check for updates" calls the GitHub Releases API for the latest release tag. If newer, show a toast "vX.Y.Z is available — download" with a link.
- **6c — Windows installer (Inno Setup):** `installer/selora-ide.iss` per RELEASE.md. Builds a `Selora-IDE-Setup-X.Y.Z.exe` that installs to `%LOCALAPPDATA%\Programs\Selora IDE\`, adds a Start Menu shortcut, an uninstaller, and registers a default file association for `.txt` (Lite only; no source files). Icon, version, publisher all populated.
- **6d — GitHub Actions release pipeline:** `.github/workflows/release.yml`. On tag push `v*.*.*`: build with `cargo build --release`, run the Inno Setup compiler, attach the `.exe` to a GitHub Release. CHANGELOG generated from commit messages. Code signing step is present but no-op until the owner provides a cert.
- **6e — Docs site plan + README polish:** `README.md` with screenshots, build instructions, contributing link, MIT badge, build status badge. A `docs/` folder with a `MkDocs` config for a real site (just writes the source — building/deploying the site is a future step).
- **6f — Open-source publication checklist:** LICENSE, README, CONTRIBUTING, issue templates, PR template, CI badge, code of conduct. The owner runs the actual `gh repo create --public --source=. --push` after this phase is approved.
- **6g — Screenshots:** 4-5 PNGs in `assets/screenshots/`: main window, chat streaming, LSP hover, git panel, settings. Used in README and docs.

**Subagent brief (e.g. 6c installer):**

```
You are Phase 6c of the Selora IDE build (Windows installer). 6a, 6b, 6d, 6e, 6f, 6g are running in parallel. Your job is the Inno Setup installer.

READ FIRST:
- C:\Users\Toxic\Desktop\Selora IDE\RELEASE.md (this is your spec for the installer)

DELIVERABLES:
1. installer/selora-ide.iss — Inno Setup script.
2. Build produces installer/Selora-IDE-Setup-X.Y.Z.exe (~25 MB).
3. Installs to %LOCALAPPDATA%\Programs\Selora IDE\
4. Adds Start Menu shortcut "Selora IDE"
5. Adds uninstaller
6. Registers .txt default open (Lite only)
7. App icon is selora-ide.ico from assets/icons/
8. Version pulled from Cargo.toml via Inno Setup preprocessor or hard-coded for now (the build script can read Cargo.toml later).
9. The installer shows the brand mark in its header.
10. Tested by: build the installer on windows-latest, upload to a draft release, the human downloads and installs on a clean Windows 11 VM.

DO NOT:
- Add code signing. RELEASE.md documents that as optional and the step is a no-op.
- Add auto-update logic. That's a future phase; the installer just installs the version it's told to.

ACCEPTANCE:
- `iscc installer/selora-ide.iss` succeeds.
- The produced .exe runs on a clean Windows 11 VM.
- After install, "Selora IDE" appears in the Start Menu; clicking it launches the IDE.
- The uninstaller in Settings → Apps removes everything cleanly.
```

**Acceptance tests (final, the owner runs):**
```bash
# On a clean Windows 11 VM with no Rust toolchain:
# 1. Download Selora-IDE-Setup-X.Y.Z.exe from GitHub Releases
# 2. Run it, accept defaults, install
# 3. Launch "Selora IDE" from Start Menu
# 4. App opens, splash shows
# 5. Paste sk-gw- key, Connect, status flips to connected
# 6. Open Folder → selora-ide repo
# 7. Open crates/selora-core/src/buffer.rs — syntax highlighted
# 8. F12 on a function — jumps to definition
# 9. Open chat sidebar, send a message, real response streams in
# 10. Open git panel, see Changes, commit a typo, commit succeeds
# 11. Settings → Keybinds → rebind a key, restart, new keybind persists
# 12. Uninstall — clean removal
```

**Estimated complexity:** Medium. The hard work was Phases 1-4. Phase 6 is packaging and polish.

**DONE means:** A stranger can install Selora IDE from a GitHub Release on a clean Windows VM, connect their Selora key, open a folder, edit code with LSP, chat with the AI, and commit to git — all within 10 minutes. That is the Lite Edition ship.

---

## Phase 7 — Account sign-in (post-Lite; client ready anytime, acceptance gated on the gateway)

**Goal:** "Log in with your Selora account" replaces key-pasting for humans: the browser opens, the user consents once, and every Selora model is live immediately — with account usage/limits auto-syncing into the IDE. The raw-key path stays forever (automation, CI, power users).

**Why its own phase:** it is the biggest *trust-surface* change since Agent Mode (token lifecycle in a desktop app) and it has a hard **cross-repo dependency**: the gateway must speak OAuth + usage. The client work is fully buildable and wiremock-testable before the gateway ships; only the real-key acceptance waits.

**The two-sided dependency (owner's gateway work, not this repo):** the contract the client codes against — `POST /oauth/token`, `POST /oauth/revoke`, `GET /v1/usage`, a browser consent page at `/oauth/authorize`, and rotating refresh tokens — is specified in ARCHITECTURE.md §7.3. `sk-gw-` keys keep working unchanged during and after (both flows end in the same `Authorization: Bearer` the client already sends).

**Decisions and tradeoffs (the shape of the phase):**

| Decision | Choice | Rejected alternative, and why |
|---|---|---|
| Primary auth flow | **Authorization Code + PKCE over a 127.0.0.1 loopback listener** (RFC 8252, the native-app BCP) | Password field in the app: never — the app must not see account credentials. Device-code (RFC 8628): kept as a documented *fallback* for RDP/locked-down browsers, not the primary (code-typing UX, slower, more phishable). |
| Fallback flows | Device-code grant (post-7 follow-up) + raw key paste (always) | Key-paste-only: no server-side identity, no refresh, no usage feed — that is exactly today's gap. |
| Token storage | Access token **memory only**; refresh token + account in the existing keyring service under a new `auth-session` entry | File-based storage (settings.json): contradicts §7.1's invariant. |
| Refresh strategy | Proactive (refresh when < 60 s to expiry, before any request) + one retry on a 401, then "sign in again" | On-401-only: an SSE stream killed mid-flight by expiry cannot be resumed — proactive refresh makes that rare instead of routine. |
| Usage sync cadence | Piggyback the existing 30 s status ping (fetch `/v1/usage` when signed in) + once after connect/turn-end | Per-receipt fetch: an agent turn emits dozens of receipts — chatty and pointless; per-launch only: stale within minutes. |
| Sign-out | Server-side revocation first, then keyring delete; offline sign-out deletes locally and revokes on next contact | Local-delete-only: leaves a live session on the server after "Disconnect" — a real dangling credential. |
| Active credentials | **One auth mode at a time** — signing in clears the `api-key` entry; pasting a key clears `auth-session` | Both live: two sources of truth for who is billed and which limit applies; confusion, not capability. |

**Tasks (three subagents, 7a can start the moment this phase is approved — wiremock needs no gateway):**

- **7a — Auth runtime:** the `auth-session` keyring slot (JSON: refresh token, account, expiry — never the access token); token exchange/refresh/revoke against §7.3's endpoints; rotating-refresh persistence (persist the new token *before* first use; a rejected refresh drops to Disconnected, never a retry loop); the `SeloraClient` seam (proactive refresh + one 401 retry); §7.1-style redaction for every token in Debug/logs. Wiremock tests: exchange, rotation, rejected-refresh, 401-then-retry, revoke, key/token mutual exclusion.
- **7b — Sign-in UX:** splash "Sign in with Selora" (primary) + "paste an API key instead" (secondary, today's flow verbatim); browser handoff via `webbrowser` (system browser, never an embedded view — the app never touches the password page); the loopback listener (ephemeral 127.0.0.1 port, `state` validation, exactly one request accepted, 5-minute hard timeout); Settings Selora section shows account email + plan + usage when signed in; Disconnect = revoke-then-delete. Headless paint tests + a scripted callback test (the test acts as the browser).
- **7c — Usage feed:** `/v1/usage` client type; piggyback on the 30 s ping; a status-bar chip and the Settings usage bar (`--type-xs`, ink only — no new saturated colors, DESIGN_TOKENS §12); period-reset handling. Wiremock tests for the feed + rollover.

**Acceptance tests (owner, real account):**
```bash
# 1. Fresh install, no keyring entries → splash shows "Sign in with Selora"
# 2. Click it → browser opens at the consent page → approve → IDE is connected (no code typed anywhere)
# 3. Status bar: connected; every model in the picker (unchanged behavior)
# 4. Settings → Selora: account email, plan, live usage vs limit
# 5. Kill the app, relaunch → still connected (silent refresh on boot)
# 6. Revoke the session from the gateway side → next request flips the IDE to "sign in again" cleanly
# 7. Disconnect in Settings → server session gone (gateway confirms), keyring empty
# 8. Paste an sk-gw key on the same install → works; signing in afterwards replaces it (one active mode)
# 9. Chat + a full agent turn run on the signed-in session; usage numbers move
```

**Estimated complexity:** Medium client-side (~1-2 sessions, three subagents). The gateway account system it depends on is the larger half and lives outside this repo.

**DONE means:** A user with a Selora account clicks "Sign in with Selora", approves once in their browser, and is connected forever after across relaunches — every model available, their usage and limits visible and auto-syncing — while an `sk-gw-` key still works the day an account doesn't exist.

---

## Total effort framing

- **Phase 0:** ~1 session (4-6 hours of agent time).
- **Phase 1:** ~1-2 sessions. The risk spike. Three parallel subagents.
- **Phase 2:** ~2-3 sessions. Six parallel subagents, then merge.
- **Phase 3:** ~2 sessions. Three parallel subagents, then merge.
- **Phase 4:** ~2-3 sessions. Six parallel subagents, then merge.
- **Phase 5:** ~2-3 sessions. Agent Mode. Five parallel subagents, then merge.
- **Phase 6:** ~1-2 sessions. Mostly sequential.
- **Phase 7:** ~1-2 client sessions (three subagents), gated on the gateway's account endpoints for acceptance only — the code is wiremock-buildable anytime.

**Total: ~10-16 overnight sessions for one owner + one orchestrating agent running subagents.** Realistic for a single motivated owner who runs the orchestrator each evening.

If the owner can run two or three sessions a day, Lite ships in roughly two and a half weeks. If only one session a day, three to four weeks. Either way, the plan is sized for a solo maintainer + AI agent team, not a studio.

---

## The "DONE means" rule

Every phase above ends with **DONE means**. A phase is not done when the subagent thinks it's done. A phase is done when:

1. All acceptance tests pass (run by the human or a CI gate).
2. The `DONE means` sentence is literally true.
3. The next phase's subagent can read the merged code and find no surprises.

No "good enough for now" exits. No "we'll fix that in Phase N+1". If a phase can't honestly meet its DONE, the orchestrator decides: fix it in this phase, or move it to the Future list and update the plan.
