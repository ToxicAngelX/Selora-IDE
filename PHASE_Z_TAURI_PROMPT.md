# GPT-6.1 SOL brief — Phase Z: Tauri/WebView2 premium shell migration

Copy everything below the line into the next implementation session.

---

You are Phase Z of the Selora IDE build: the architectural UI migration that
combines a Rust performance and trust core with a premium React/WebView2 shell.

The product direction is locked:

> Beautiful and expressive like ZCode, lightweight like Tauri, and operationally
> safe and responsive because Rust remains the core.

Selora is moving from a pure egui shell to a Tauri 2 desktop shell using the
Windows WebView2 runtime. Rust remains the owner of application state, editor
buffers, AI requests, agent execution, approvals, files, LSP, Git, persistence,
keyring access, and security-sensitive actions. React/TypeScript owns the visual
shell, layout, interaction presentation, and premium motion language.

This is a staged migration, not a destructive rewrite. The existing eframe/egui
application must remain buildable while the new shell reaches feature parity.
Do not delete the old shell until the parity gate at the end of this brief passes.

## 1. First actions and stop conditions

Work in `E:\Projects\Selora IDE`.

Before editing anything:

```powershell
git status --short
git log --oneline --decorate -12
cargo build --workspace
cargo test --workspace --no-fail-fast
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
node --version
npm --version
```

If the Rust baseline is red, diagnose and report the failure before beginning the
Tauri migration. Do not build new architecture on top of an unexplained failure.
If Node or npm is unavailable, report it and still complete the Rust protocol and
shell architecture work that can be verified locally.

Read these files before writing code:

- `README.md`
- `PLAN.md`
- `ARCHITECTURE.md` §§1–4, 7–8, 17–19
- `ROADMAP.md`, especially Phases 7–9
- `STATUS.md`
- `UI_SPEC.md`
- `DESIGN_TOKENS.md`
- `docs/ui/REVAMP_SPEC.md`
- `docs/ui/REVAMP_REPORT.md` if present
- `RELEASE.md`
- `crates/selora-app/src/app_state.rs`
- `crates/selora-app/src/shell.rs`
- `crates/selora-app/src/persistence.rs`
- `crates/selora-ui/src/sidebar/chat/mod.rs`
- `crates/selora-ui/src/sidebar/chat/agent_cards.rs`
- `crates/selora-ui/src/presence/`

Also inspect the current working tree instead of trusting historical prose. Some
Phase R and Phase Z documents may describe work that is still uncommitted or
partially implemented.

## 2. Product outcome

Build a premium ZCode-grade desktop shell with this anatomy:

```text
custom title bar
activity bar
primary sidebar (files, search, Git, problems, settings)
editor center (CodeMirror 6 in the first web shell)
resizable agent dock on the right
resizable bottom dock (problems, output, terminal-class surfaces)
status bar
```

The agent dock is a first-class product surface. It is not an afterthought sidebar.
The composer remains the visual hero and its behavior is sacred:

- Enter submits.
- Shift+Enter inserts a newline.
- Up/down recalls the last ten prompts on a one-line buffer.
- The composer grows to a bounded number of rows.
- Send changes to Stop during a plain stream.
- Stop cancels the actual request and preserves the partial assistant message.
- Model selection, session switching, agent mode, approvals, retries, copy,
  links, code blocks, and open-at-line actions continue to work.

The user must be able to use the complete IDE from the keyboard. Every dock,
activity view, dialog, session action, approval action, and editor command must
have a command-palette entry and a documented shortcut where appropriate.

## 3. Non-negotiable architecture

### 3.1 Rust remains the product core

Keep these crates and responsibilities in Rust:

- `selora-ai`: Selora gateway client, SSE streaming, model catalog, receipts,
  ghost text, agent loop, tool router, approval decisions, turn caps, web tools.
- `selora-editor`: buffers, rope, selections, edits, undo, syntax/highlighting
  support and editor-domain behavior.
- `selora-fs`: tree walking, fuzzy matching, workspace search.
- `selora-lsp`: language-server process management and protocol handling.
- `selora-git`: status, stage/unstage, commits, diffs.
- `selora-settings`: JSON settings, keyring references, keybinds, migration.
- `selora-core`: shared workspace and buffer types.
- `selora-tokens`: canonical design constants and motion values.

The browser frontend must never directly:

- read or write arbitrary workspace files;
- execute shell commands;
- access the OS keyring;
- spawn LSP processes;
- call the Selora gateway with user credentials;
- approve a tool call without a Rust-side command and decision check;
- mutate authoritative editor or agent state outside the typed protocol.

### 3.2 Add a typed protocol boundary

Create a small shared protocol crate, preferably `crates/selora-protocol`, or
another additive location justified by the existing workspace structure. It owns
serializable command/event/view-model types shared by the Tauri adapter and tests.

Use typed `serde` enums and stable IDs. Do not expose internal Rust structs
directly to the browser. The protocol should include at least:

```rust
pub enum FrontendCommand {
    GetBootstrapState,
    OpenFolder,
    OpenFile { path: String },
    CloseBuffer { buffer_id: String },
    SaveBuffer { buffer_id: String },
    ApplyEditorChange { buffer_id: String, version: u64, changes: Vec<TextChange> },
    SetSelection { buffer_id: String, selection: Selection },
    SendChat { request_id: String, request: ChatRequestDto },
    StopChat { request_id: String },
    SendAgent { request_id: String, request: AgentRequestDto },
    AgentDecision { request_id: String, approval_id: String, decision: DecisionDto },
    RetryMessage { session_id: String, message_id: String },
    SwitchSession { session_id: String },
    NewSession,
    Search { query: SearchQueryDto },
    FindInBuffer { buffer_id: String, query: String, options: FindOptionsDto },
    ReplaceCurrent { buffer_id: String, replacement: String },
    ReplaceAll { buffer_id: String, replacement: String },
    GitStage { path: String },
    GitUnstage { path: String },
    GitCommit { message: String },
    OpenGitDiff { path: String },
    ToggleSurface { surface: SurfaceId },
    ResizeRegion { region: RegionId, size_px: f32 },
    SaveLayout,
    OpenExternal { url: String },
    SettingsUpdate { patch: SettingsPatchDto },
}

pub enum FrontendEvent {
    BootstrapState(BootstrapState),
    ChatDelta(ChatDeltaDto),
    ChatCompleted(ChatCompletedDto),
    ChatFailed(ChatFailedDto),
    AgentEvent(AgentEventDto),
    DiagnosticsChanged(DiagnosticsDto),
    FileTreeChanged(FileTreeDto),
    BufferSnapshot(BufferSnapshotDto),
    BufferChanged(BufferChangedDto),
    SearchResults(SearchResultsDto),
    GitStatusChanged(GitStatusDto),
    GitDiffReady(GitDiffDto),
    ModelsChanged(ModelsDto),
    SeloraStatusChanged(SeloraStatusDto),
    WorkspaceLayoutChanged(LayoutDto),
    Toast(ToastDto),
    Error(ProtocolErrorDto),
}
```

The exact type names may differ, but the boundary must be explicit, versionable,
and tested. Include `protocol_version` in bootstrap state. Include workspace,
session, buffer, request, turn, approval, and message IDs in all relevant events.

### 3.3 Snapshot plus incremental events

On startup, the frontend calls one bootstrap command and receives:

- workspace and current root;
- open buffers and active editor;
- file tree summary;
- active model and connection status;
- chat sessions and active timeline;
- settings that are safe to display;
- layout and dock sizes;
- Git status;
- diagnostics;
- feature flags and protocol version.

After bootstrap, use incremental events. Do not resend the entire workspace or
transcript for each token, keystroke, or diagnostic. Batch plain chat deltas at a
bounded cadence (approximately 30–60 updates per second) while preserving final
receipt accuracy and ordering.

Rust remains authoritative if the frontend disconnects, reloads, or crashes. A
frontend reconnect must be able to request a fresh bootstrap state and resume
rendering without corrupting the workspace.

## 4. Tauri shell structure

Use Tauri 2 and WebView2 on Windows. Keep the legacy `selora-app` eframe binary
buildable during migration. Add the new application additively, for example:

```text
apps/selora-desktop/
  src-tauri/
    Cargo.toml
    src/
      main.rs
      commands.rs
      events.rs
      state.rs
      security.rs
      adapter.rs
    tauri.conf.json
  frontend/
    package.json
    vite.config.ts
    tsconfig.json
    src/
      main.tsx
      app/
      bridge/
      components/
      features/
      layout/
      editor/
      presence/
      styles/
```

If repository conventions make `crates/selora-tauri` and `frontend/` a better fit,
use that structure, document the decision, and keep the protocol crate independent.
Do not entangle the frontend with `selora-app/src/shell.rs` internals.

Use Tauri capabilities to allow only the commands and events the application
needs. Configure a strict CSP. Do not use arbitrary remote script sources. Keep
external links behind the Rust `OpenExternal` command or an explicitly reviewed
Tauri capability.

The first Windows target is WebView2. Do not add macOS/Linux packaging work in
this phase. Avoid platform-specific UI assumptions in the protocol so future
ports remain possible.

## 5. Frontend stack and state rules

Use:

- React 18+;
- TypeScript with strict mode;
- Vite;
- Tauri 2 frontend APIs;
- CSS modules or a disciplined global CSS layer;
- CodeMirror 6 for the initial editor surface;
- a small typed bridge module wrapping `invoke` and event subscriptions.

Do not put business logic in components. Use feature stores/hooks for view state,
while Rust owns authoritative domain state. Do not mirror large buffers in many
React stores. Keep one editor model per visible buffer and apply incremental
changes.

The frontend must support:

- reconnect and bootstrap replay;
- stale-event rejection using request/session/version IDs;
- optimistic UI only where rollback is defined;
- error and loading states for every command;
- reduced-motion mode;
- keyboard navigation and visible focus;
- screen-reader labels for controls and canvas/WebGL effects;
- high-DPI Windows displays.

## 6. Shell layout and persistence

Implement the shell with CSS Grid/Flex and explicit resize handles. Regions:

- title bar;
- activity bar;
- primary sidebar;
- editor region;
- agent dock;
- bottom dock;
- status bar.

Resize hit zones must be at least 6px. Every region has min/max constraints. A
collapsed region must have an accessible toggle and a command-palette command.
Persist layout per workspace in the existing workspace persistence model or a
versioned extension of it. Use serde defaults so old workspaces load safely.

Persist at least:

- primary sidebar width;
- agent dock width and open/closed state;
- bottom dock height and open/closed state;
- active activity view;
- active bottom-dock view;
- editor split arrangement;
- active session and open buffers where the existing model supports them.

Do not use layout values as hardcoded magic numbers. Add canonical web tokens
derived from the existing Selora token contract. Document any new spacing, radius,
or motion values before using them.

## 7. Required feature parity

Port the current product behavior into the new shell:

### Agent dock

- session title and switcher;
- new session;
- model picker with filtering;
- transcript with Markdown-lite blocks;
- syntax-highlighted fenced code blocks;
- message copy and retry;
- token/cost/time footer;
- tool cards for read/write/edit/delete/bash/web;
- diff rendering and approve/deny/approve-all-this-turn;
- open path at line;
- agent status line;
- stop control;
- empty state with orb, calm copy, and suggestion chips;
- composer with the exact existing behavior contract.

### Primary sidebar

- file tree;
- workspace search;
- Git status and stage/unstage/commit;
- problems grouped by file and severity;
- settings entry;
- activity badges and keyboard navigation.

### Editor

- tabs and dirty state;
- split panes;
- syntax highlighting;
- selections and edits;
- ghost text acceptance;
- LSP diagnostics, hover, completion, go-to-definition;
- find/replace;
- read-only Git diff overlay;
- open-at-line actions from search/problems/agent cards.

### Bottom dock

- Problems;
- output/task surface;
- a clearly labeled terminal-class placeholder or future-ready surface if the
  built-in terminal is still out of scope;
- toggle, resize, keyboard access, persistence, and empty states.

### App chrome

- custom title bar;
- File/Edit/View/Help menus;
- command palette coverage for every surface;
- keyboard shortcut cheat sheet;
- toasts with a notifications center or history surface;
- first-run onboarding;
- About and update check;
- settings and keyring-safe account connection.

## 8. Libraries.dev integration — all seven packages are mandatory

Use the MIT-licensed libraries from:

`https://github.com/Jakubantalik/Libraries.dev`

Install the published packages in the frontend. Do not copy their source into
Selora. Read each package README and its current TypeScript types before wiring it.
Record package versions and license notices in the frontend lockfile and release
documentation. `img-fx` requires `three`; verify the package's current peer
dependency and bundle impact before enabling it.

The libraries are visual behavior references and actual frontend dependencies in
this phase. Keep the old Rust egui presence widgets as a fallback/reference until
the new shell reaches parity.

### 8.1 `border-beam`

Use `BorderBeam` on the chat composer only. This is the signature active surface.
Configure it for Selora's dark monochrome language:

- `colorVariant="mono"`;
- dark theme;
- low strength;
- static colors;
- duration no faster than the existing motion floor;
- effect layers have `pointer-events: none`;
- the underlying composer keeps its 1px hairline;
- disabled when idle and active only while a request is in flight;
- reduced motion freezes it to a static quiet edge.

Do not add beam effects to every card. The beam is meaningful because it marks the
active composer, not because it is everywhere.

### 8.2 `thinking-orbs`

Use `ThinkingOrb` in the agent dock and every waiting state that communicates real
work:

```text
working      model thinking
searching    read/search tool activity
solving      bash/write/edit tool activity
listening    approval or user input required
connecting   streamed deltas / gateway connection
weaving      workspace search
composing    ghost-text generation
shaping      file-tree refresh/indexing
breathing    idle/finished state
```

Use size 20 for inline/status surfaces and size 64 for the empty agent welcome
hero. Use `paused` or a static representative frame for disconnected and reduced
motion states. Preserve accessible labels and do not let decorative canvas output
replace semantic status text.

### 8.3 `bot-avatars`

Use `BotAvatar` as the agent identity glyph:

- 20–24px in the agent status line;
- tool-card headers;
- session header when an agent run is active;
- larger welcome/avatar surface only when it improves orientation.

Use one restrained shape family and a monochrome or low-saturation Selora palette.
Map states:

```text
default    idle agent
working    active model/tool turn
sleeping   disconnected or suspended gateway
```

Keep the semantic status text beside the avatar. Respect reduced motion. Do not
use the full colorful carousel in the IDE; the library is being used for living
identity and state, not visual noise.

### 8.4 `liquid-gooey`

Use the `Gooey`/`Liquid` API for binary mode transitions and compact controls:

- Agent mode toggle;
- Plan mode toggle when Phase 9 work begins;
- session/live-state switcher where a morphing pill improves clarity;
- future dock mode toggles if the interaction is genuinely binary.

Keep the real DOM content layer above the goo silhouette so text remains crisp,
focus rings remain accessible, and pointer/keyboard behavior remains ordinary.
Use the monochrome Selora surface fill through CSS variables. Respect reduced
motion by snapping the transition. Do not apply goo filters to code, long text, or
large transcript regions.

### 8.5 `voice-glow`

Use `VoiceBeam` as the composer activity meter, but do not request microphone
permission. Drive it with a Rust/frontend metric derived from streaming deltas,
agent tool activity, or playback/processing state:

```tsx
<VoiceBeam
  level={() => streamMetrics.level}
  motion={() => streamMetrics.processingMotion}
  colorVariant="mono"
  active={composerActive}
  paused={reducedMotion}
>
  {composer}
</VoiceBeam>
```

The metric must be explicitly documented as an AI activity meter, not voice input.
Use a low-opacity mono palette and an attack/release profile that feels calm. When
an agent tool is running, use the gathered/processing motion. When the request is
idle, use no glow or the quiet resting state. Do not ship a microphone permission
prompt in this phase.

### 8.6 `metal-fx`

Use `MetalFx` selectively for premium brand moments:

- first-run splash mark;
- About/brand surface;
- optional connection/update badge if it remains calm and useful.

Use the silver/monochrome direction, low strength, and a reduced-motion paused
state. Call `isMetalFxSupported()` and render the normal Selora mark when WebGL2
is unavailable. Do not make WebGL a requirement for core editing or agent work.
Use only one shared renderer/context as the package supports.

### 8.7 `img-fx`

The new web shell has image-bearing surfaces, so use `ImageGeneration` in a
meaningful, restrained way:

- onboarding/project preview card;
- image/artifact result cards returned by future agent tools;
- optional workspace welcome art when a project has a configured preview image.

It must be lazy-loaded, virtualized when inside a list, and disabled when there is
no image. Use `pixels-organic` or `pixels-mechanic` at a low frame cap. Do not put
it behind every empty state or create an automatic image loop in the editor. The
plain card must remain correct if WebGL or `three` fails. Provide a manual reveal
or explicit lifecycle tied to actual content, not decorative infinite motion.

## 9. Design system translation

Translate the existing `DESIGN_TOKENS.md` contract into CSS variables. Keep the
Selora identity:

- monochrome dark canvas;
- ink ramp and semantic green/error/warning colors only where meaning requires;
- hairline borders;
- no generic shadows or neon gradients;
- low motion density;
- loop durations at least 1200ms;
- reduced-motion support for every effect;
- visible keyboard focus;
- no bold/emoji as a substitute for hierarchy;
- typography using bundled or licensed Selora fonts where practical;
- JetBrains Mono for code and symbol/status fallback.

Define variables for colors, spacing, radius, typography, and motion. Do not copy
package demo colors blindly. Use CSS overrides or package props to bring every
Libraries.dev component into the Selora visual language.

## 10. Security and trust requirements

The Tauri boundary is a security boundary. Implement and test:

- allowlisted Tauri commands only;
- no arbitrary filesystem path access from the frontend;
- workspace-root containment checked in Rust;
- external URL validation before opening a browser;
- approval decisions validated against the active request and approval ID;
- stale request/session/version events discarded;
- no secrets in frontend state, logs, URLs, or localStorage;
- gateway key remains in the OS keyring;
- strict Content Security Policy;
- no untrusted HTML injection from model output;
- Markdown rendered through a safe structured renderer;
- tool output treated as data, never as frontend instructions.

Add protocol and security tests before calling the migration complete.

## 11. Performance requirements

Measure the legacy egui app before comparing the new shell. Record:

- cold start to first usable frame;
- idle memory after opening a workspace;
- typing latency in a large buffer;
- scroll latency in a large buffer;
- chat delta-to-paint latency;
- agent card update latency;
- dock resize responsiveness;
- WebGL effect count and frame cost.

The new shell must meet these practical budgets:

- no visible input lag during normal typing;
- no full-buffer React rerender for a single edit;
- no one-event-per-token IPC flood;
- no more than one shared WebGL renderer per library family where supported;
- effects pause offscreen and under reduced motion;
- img-fx stays at its documented low frame cap;
- MetalFx and img-fx have graceful non-WebGL fallbacks;
- a closed dock does not continue repainting its effects;
- idle CPU remains quiet when nothing is streaming or animating.

If the WebView2 shell causes a material regression, document it with measurements
and optimize the offending seam before expanding the migration.

## 12. Delivery phases

Implement in vertical slices. Keep each slice buildable and tested.

### Z0 — Baseline and protocol

- capture current build/test results;
- inspect uncommitted work and record it;
- create the protocol crate/types;
- define bootstrap, commands, events, IDs, versions, and error semantics;
- add protocol serialization tests;
- do not change the current UI behavior yet.

### Z1 — Tauri/WebView2 scaffold

- add the Tauri application and Vite React frontend;
- configure Windows/WebView2;
- configure capabilities and CSP;
- connect `GetBootstrapState`;
- render a diagnostic shell with live Rust status;
- keep `cargo build --workspace` green;
- add npm typecheck/build scripts.

### Z2 — Premium shell layout

- title/activity/sidebar/editor/agent/bottom/status regions;
- drag-resize handles and constraints;
- layout persistence per workspace;
- keyboard toggles and palette entries;
- empty states;
- CSS token layer;
- initial toast/notification surface.

### Z3 — Agent dock vertical slice

- session header and model picker;
- transcript and Markdown blocks;
- composer contract;
- ChatDelta batching and stale-event handling;
- ThinkingOrb, BotAvatar, BorderBeam, VoiceBeam, Liquid Gooey;
- tool cards, diffs, approvals, stop, retry, cost footer;
- live smoke test with a mock gateway.

### Z4 — Editor and workbench parity

- CodeMirror 6 buffer bridge;
- open/save/edit/undo flow;
- LSP hover/completion/diagnostics/definition;
- ghost text;
- tabs/splits/find-replace;
- file tree/search/Git/problems/diff/bottom dock;
- open-at-line routing.

### Z5 — Brand, onboarding, and all visual library integrations

- MetalFx splash/About surfaces with fallback;
- img-fx onboarding/artifact surface with fallback;
- notifications center/history;
- settings and keyboard cheat sheet;
- reduced-motion audit;
- accessibility audit;
- screenshots and updated `UI_SPEC.md`/revamp docs.

### Z6 — Parity, performance, and cutover decision

- run the complete parity walkthrough;
- run Rust and frontend tests/builds/lints;
- run security tests;
- run performance measurements against Z0;
- fix regressions;
- leave the egui app available until the owner explicitly approves cutover;
- update `README.md`, `STATUS.md`, `ROADMAP.md`, `CHANGELOG.md`, and release docs.

## 13. Required tests

Rust:

```powershell
cargo build --workspace
cargo test --workspace --no-fail-fast
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Frontend:

```powershell
npm ci
npm run typecheck
npm run build
npm run test
```

Add tests for:

- protocol round trips and versioning;
- bootstrap reconstruction;
- reconnect after frontend reload;
- stale chat/agent event rejection;
- approval ID/request ID validation;
- workspace-root containment;
- dock resize and persistence;
- keyboard command coverage;
- composer Enter/Shift+Enter/history/stop behavior;
- transcript virtualization and incremental updates;
- editor incremental changes and selection sync;
- reduced-motion states for all seven Libraries.dev integrations;
- WebGL fallback for MetalFx and img-fx;
- no microphone permission path for VoiceBeam;
- accessibility labels and keyboard focus;
- empty states and error states.

Manual walkthrough:

1. Launch the Tauri app with no key and see the first-run state.
2. Connect with a mock gateway and observe the status transition.
3. Open a real workspace.
4. Open and edit a Rust file in CodeMirror.
5. Confirm LSP diagnostics, hover, completion, and definition navigation.
6. Run workspace search and open a result.
7. Start a plain chat stream; verify the composer beam and activity meter.
8. Stop the stream; verify the partial answer remains and no error toast appears.
9. Start agent mode; verify avatar/orb/status/tool cards.
10. Deny a write; verify the disk is unchanged.
11. Approve a write; verify the diff, buffer sync, undo, and LSP update.
12. Open Git diff, Problems, Settings, About, and the keyboard cheat sheet.
13. Resize the sidebar, agent dock, and bottom dock; restart and verify persistence.
14. Trigger the onboarding/artifact image surface and verify img-fx fallback.
15. Disable WebGL or simulate unsupported WebGL; verify usable plain fallbacks.
16. Enable reduced motion; verify all loops freeze and semantic state remains.
17. Reload the frontend; verify bootstrap restores the correct state.
18. Close the app and relaunch; verify Rust persistence and session restore.

## 14. Documentation and reports

Update or create:

- `docs/ui/PHASE_Z_TAURI_ARCHITECTURE.md`;
- `docs/ui/PHASE_Z_PROTOCOL.md`;
- `docs/ui/PHASE_Z_MIGRATION.md`;
- `UI_SPEC.md` for the new shell;
- `STATUS.md` with exact implementation state;
- `ROADMAP.md` with the new migration phases;
- `CHANGELOG.md` for user-visible changes;
- release/license notices for Tauri, React, CodeMirror, Libraries.dev packages,
  Three.js, and any transitive visual dependency that requires attribution.

The final report must include:

- exact files changed;
- exact commands and outputs;
- what is implemented versus scaffolded;
- protocol/event map;
- ZCode pattern → Selora implementation map;
- all seven Libraries.dev components and their actual mounted surfaces;
- accessibility and reduced-motion behavior;
- security decisions;
- performance measurements and regressions;
- remaining parity gaps;
- whether the egui shell can be safely cut over;
- what Phase 8 workflows and Phase 9 Agent Workbench should rely on.

## 15. Rules for judgment

- Prefer additive seams over rewrites.
- Preserve the existing Rust behavior before improving presentation.
- Never let the frontend become a second authority for files, tools, approvals,
  cost, or sessions.
- Use the actual Libraries.dev APIs and current package types; do not invent props.
- Keep all effects semantic, sparse, and low-motion.
- Every decorative effect needs a reduced-motion state and a fallback.
- Every new command needs a test and a keyboard-accessible path where applicable.
- If a visual effect hurts typing, scrolling, streaming, or startup, reduce or
  disable the effect before compromising core responsiveness.
- Do not claim ZCode parity from screenshots alone; verify the interaction flows.

The success criterion is:

> Selora feels like a premium ZCode-class agent IDE, starts and edits responsively
> like a lightweight desktop tool, keeps all authority and trust boundaries in
> Rust, and uses every Libraries.dev component deliberately rather than as noise.
