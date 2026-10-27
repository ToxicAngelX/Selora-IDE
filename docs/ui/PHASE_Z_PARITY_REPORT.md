# Phase Z parity report

Audit date: 2026-10-26. This report describes the working tree as found; it does not treat historical prose as evidence. The repository has no `.git` directory, so there is no commit, branch, or merge record to cite. Z6 changed only files under `docs/ui/`; no source file was changed.

## Gate results

All commands below were run from the current tree on 2026-10-26.

| Command | Result |
|---|---|
| `cargo build --workspace` | exit 0; `selora-desktop` compiled; finished in 9.85s |
| `cargo test --workspace --no-fail-fast` | exit 0; 24 suites; **905 passed, 0 failed, 4 ignored** |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0; no warnings |
| `npm run typecheck` (from `apps/selora-desktop/frontend`) | exit 0 |
| `npm test` (same directory) | exit 0; **40 files, 290 tests passed** |
| `npm run build` (same directory) | exit 0; 1,136 modules transformed; built in 11.51s |

The Rust count is the sum of every `test result:` line in the captured output: 905 passed, 0 failed, 4 ignored across 24 suites. The frontend count is Vitest's summary: 40 files and 290 tests. Automated green does not prove the WebView2 runtime path; see gap 10.

## Protocol/event map

The wire enums are `FrontendCommand` (32 variants) at `crates/selora-protocol/src/lib.rs:353-466` and `FrontendEvent` (25 variants) at `crates/selora-protocol/src/lib.rs:1029-1058`. The frontend enumerates 27 command variants at `apps/selora-desktop/frontend/src/bridge/bridge.ts:186` (the five commands it cannot yet send are absent) and all 25 events at `apps/selora-desktop/frontend/src/bridge/bridge.ts:227`. Dispatch arms live at `apps/selora-desktop/src-tauri/src/state.rs:584-850`; the fallback is `unsupported_phase_slice` at `apps/selora-desktop/src-tauri/src/adapter.rs:3-8`.

`request_id` has two levels. Every `RequestEnvelope` carries a transport UUID (`bridge.ts:317-325`). Session commands additionally carry a stable payload `request_id` that `StopChat` and `AgentDecision` name and that stream events echo; the envelope id remains transport-only (`state.rs:585-589`, `sessions.rs:955-969`). Workspace, search, Git, and layout events use the command envelope id in `EventEnvelope.request_id` (`state.rs:641-845`). LSP bridge events record with `None` (`state.rs:400-402`) and are routed by `buffer_id` instead. `SearchResultsDto.request_id` is populated as `None` by the workspace service (`workspace.rs:365-369`); its envelope id is the usable correlation. `SetSelection` emits no event (`state.rs:776-782`). `CloseBuffer` also emits no event (`state.rs:732-743`), and the frontend ignores the command's returned bootstrap (`editor-store.ts:261-268`), which is a real reconciliation defect listed below.

### Commands

| Command | Rust status | Frontend consumer |
|---|---|---|
| `get_bootstrap_state` | implemented (`state.rs:584`); also a direct Tauri command (`commands.rs:8-21`) | bootstrap flow in `app-store.ts` |
| `open_folder` | implemented as a `SELORA_WORKSPACE_ROOT` override path (`state.rs:677-697`); normal UI uses `open_folder_at` | bridge union only |
| `open_folder_at` | implemented (`state.rs:698-710`) | `FileTreeView.tsx:189-195`, `EditorRegion.tsx:112-114` |
| `refresh_tree` | implemented (`state.rs:711-714`) | `FileTreeView.tsx:215` |
| `open_file` | implemented (`state.rs:716-731`) | `editor-store.ts:231` |
| `close_buffer` | implemented in Rust (`state.rs:732-743`) but emits no event | `editor-store.ts:261-268` |
| `save_buffer` | implemented (`state.rs:745-756`) | `editor-store.ts:280` |
| `apply_editor_change` | implemented (`state.rs:758-774`) | `CodeMirrorPane.tsx:287` |
| `set_selection` | implemented, silent (`state.rs:776-782`) | `CodeMirrorPane.tsx:264` |
| `hover_at` | implemented (`state.rs:783-791`) | `CodeMirrorPane.tsx:316` |
| `definition_at` | implemented (`state.rs:792-800`) | `CodeMirrorPane.tsx:383` |
| `completion_at` | implemented (`state.rs:801-809`) | `CodeMirrorPane.tsx:354` |
| `send_chat` | implemented (`state.rs:590-602`) | `agent-store.ts:501`; stop at `:578` |
| `stop_chat` | implemented (`state.rs:603-609`) | `agent-store.ts:578`, `:901` |
| `send_agent` | implemented (`state.rs:610-622`) | `agent-store.ts:501` |
| `agent_decision` | implemented (`state.rs:623-635`) | `agent-store.ts:842` |
| `retry_message` | `unsupported_phase_slice` | no production caller; union only |
| `switch_session` | `unsupported_phase_slice` | local `agentStore.switchSession` only (`AgentSessionRail.tsx:48`); bridge helper at `bridge.ts:523` is not the production path |
| `new_session` | `unsupported_phase_slice` | local store only; bridge helper at `bridge.ts:526` |
| `search` | implemented, literal matching only (`state.rs:810-814`) | `SearchPanelView.tsx` |
| `find_in_buffer` | `unsupported_phase_slice` | none |
| `replace_current` | `unsupported_phase_slice` | none |
| `replace_all` | `unsupported_phase_slice` | none |
| `git_stage` | implemented (`state.rs:815-823`) | `GitPanelView.tsx` |
| `git_unstage` | implemented (`state.rs:824-833`) | `GitPanelView.tsx` |
| `git_commit` | implemented (`state.rs:834-842`) | `GitPanelView.tsx` |
| `open_git_diff` | implemented (`state.rs:843-846`) | `GitPanelView.tsx` |
| `toggle_surface` | implemented (`state.rs:636-647`) | `layout-store.ts:312` |
| `resize_region` | implemented with clamping (`state.rs:648-666`) | `layout-store.ts:207` |
| `save_layout` | implemented (`state.rs:667-676`) | `layout-store.ts:214`, close-time save at `main.rs:49-53` |
| `open_external` | `unsupported_phase_slice` | none; About/Connect use inert `target="_blank"` links |
| `settings_update` | `unsupported_phase_slice` | none; Settings edits local state only |

The eight unsupported commands are exactly `RetryMessage`, `SwitchSession`, `NewSession`, `FindInBuffer`, `ReplaceCurrent`, `ReplaceAll`, `OpenExternal`, and `SettingsUpdate`. All other command variants reach a concrete dispatch arm.

### Events

| Event | Producer | Frontend consumer | Status |
|---|---|---|---|
| `bootstrap_state` | `state.rs:693`, `:706` | `app-store.ts:237-243` | live |
| `chat_delta` | `sessions.rs:1090` | `app-store.ts:254-255` | live |
| `chat_completed` | `sessions.rs:957`, `:1004` | `app-store.ts:257-258` | live |
| `chat_failed` | `sessions.rs:982` | `app-store.ts:260-261` | live |
| `agent_event` | no production emitter | `app-store.ts:278-279` | declared only |
| `tool_call_started` | `sessions.rs:1105` | `app-store.ts:263-264` | live |
| `tool_result` | `sessions.rs:1121` | `app-store.ts:266-267` | live |
| `approval_needed` | `sessions.rs:1159` | `app-store.ts:269-270` | live |
| `agent_state_changed` | `sessions.rs:1073` | `app-store.ts:272-273` | live |
| `agent_turn_done` | `sessions.rs:1202`, `:1250` | `app-store.ts:275-276` | live |
| `diagnostics_changed` | `lsp_bridge.rs:169`, `:219` | `app-store.ts:332-336` | live |
| `hover_result` | `lsp_bridge.rs:223` | `app-store.ts:352-354` | live |
| `completion_result` | `lsp_bridge.rs:243` | `app-store.ts:356-358` | live |
| `definition_result` | `lsp_bridge.rs:234` | `app-store.ts:360-362` | live |
| `file_tree_changed` | `state.rs:713` | `app-store.ts:326-330` | live |
| `buffer_snapshot` | `state.rs:720`, `:749` | `app-store.ts:308-309` | live |
| `buffer_changed` | `state.rs:766` | `app-store.ts:311-324` | live |
| `search_results` | `state.rs:812` | `app-store.ts:344-346` | live |
| `git_status_changed` | `state.rs:819`, `:829`, `:838` | `app-store.ts:338-342` | live |
| `git_diff_ready` | `state.rs:845` | `app-store.ts:348-350` | live |
| `models_changed` | `sessions.rs:704`; `commands.rs:143`, `:202`, `:241` | `app-store.ts:294-306` | live |
| `selora_status_changed` | `sessions.rs:1448`; `commands.rs:151`, `:210`, `:249` | `app-store.ts:281-293` | live |
| `workspace_layout_changed` | `state.rs:643`, `:660` | `app-store.ts:245-246` | live |
| `toast` | `state.rs:244`, `:1554`; `lsp_bridge.rs:249` | `app-store.ts:248-249` | live |
| `error` | no production emitter | `app-store.ts:251-252` | declared only |

`FeatureFlagsDto` is emitted with `agent`, `git`, `search`, and `editor` all `true`, and `later_slices` set to `["lsp-bridge", "workspace-trust"]` (`state.rs:518-524`). The UI does not branch on these flags; the type exists at `bridge.ts:102`, but no feature gate consumes it.

## ZCode pattern → Selora implementation map

Only mappings visible in the tree are listed. This is not a claim of behavioral parity.

| ZCode-class pattern | Selora implementation in this tree | Verified status |
|---|---|---|
| Custom title bar plus workbench shell | `ShellGrid.tsx:62-186` renders title bar, activity bar, sidebar, editor, bottom dock, agent dock, and status bar | mounted |
| Activity/sidebar navigation | `TitleBar.tsx:239-242` menus; `ActivityBar`; `PrimarySidebar.tsx:54-57` mounts Files/Search/Git/Settings | mounted |
| First-class agent dock | `AgentDock.tsx:154-299` mounts the session rail, transcript, plan spine, and composer | mounted |
| Composer behavior: Enter, Shift+Enter, prompt history, stop | `AgentComposer.tsx:4-10`; key handling `:265-322`; in-flight stop `:773-821` | implemented in the local component |
| Permission-mode selector | `AgentComposer.tsx:69-101` offers Full/Plan/Ask; `agent-store.ts:488` maps `plan_only` to `explore` and every other mode to `execute` | UI local; not a Rust authority |
| Tool cards and approval cards | `AgentExecutionFeed.tsx:43-451`; approval handling `agent-store.ts:1034-1045` | live from protocol events |
| Plan/todo spine | `AgentPlanSpine.tsx:121-347`; todos are created locally (`agent-store.ts:448-460`) and parsed from results | presentation is mounted; roster/list is not backend-authoritative |
| Session rail and switching | `AgentSessionRail.tsx:48`; sessions start from `INITIAL_SESSIONS` (`agent-store.ts:95-105`) | local only; Rust session commands are unsupported |
| Presence and live status | `ThinkingOrb` in `AgentDock.tsx:50` and `AgentExecutionFeed.tsx:86`; `BotAvatar` in `AgentExecutionFeed.tsx:291`, `:451` | mounted |
| Composer activity effects | local `BorderBeam` at `AgentComposer.tsx:441`; local `VoiceBeam` at `AgentComposer.tsx:828` | mounted |
| Command palette, cheat sheet, About | `App.tsx:106-125`; commands at `features/keymap/commands.ts:65-78`; `Ctrl+/` at `:76` | mounted |
| Dynamic workflows and subagents | No workflow engine, no `spawn_subagent` runtime, no live subagent events | not implemented; Phase 8/9 |
| Plan mode / plan approval / escalations | No Rust plan gate or question-card route; only the local `plan_only` mode mapping | not implemented; Phase 9 |

## Libraries.dev components and their actual mounted surface

The specification names seven packages at `PHASE_Z_TAURI_PROMPT.md:382-525`. The installed package set is only `img-fx@0.5.1` and `metal-fx@2.0.11` (`apps/selora-desktop/frontend/package.json:27-28`). `img-fx`'s peer `three@0.186.1` is present; it is not a direct dependency. The other five named packages are **not installed**; the tree contains local ports under `features/agent/presence/`.

| Specification package | Installed? | Actual production mount |
|---|---|---|
| `border-beam` | not installed | local `BorderBeam`; `AgentComposer.tsx:441` only, `active={inFlight}` |
| `thinking-orbs` | not installed | local `ThinkingOrb`; `AgentDock.tsx:50`, `AgentExecutionFeed.tsx:86`, `:450` |
| `bot-avatars` | not installed | local `BotAvatar`; `AgentExecutionFeed.tsx:291`, `:451`; `AgentPlanSpine.tsx:341-347`, `:534`, `:737`, `:922` |
| `liquid-gooey` | not installed | local `LiquidPill` exists at `features/agent/presence/LiquidPill.tsx:17`, but its only import is the test `ThinkingOrb.test.tsx:7`; it is **not mounted in production** |
| `voice-glow` | not installed | local `VoiceBeam`; `AgentComposer.tsx:828`, driven by streaming activity, not microphone input (`VoiceBeam.tsx:4`) |
| `metal-fx` | installed `2.0.11` | `BrandFx.tsx:53-54` dynamically imports it; `MetalBrand` is mounted in `AboutDialog.tsx:169` |
| `img-fx` | installed `0.5.1` | `BrandFx.tsx:48-51` dynamically imports it; `OnboardingHero` is mounted at `OnboardingOverlay.tsx:103` |

`img-fx` and `metal-fx` are capability-gated and lazy: `BrandFx.tsx:77`, `:105` probe WebGL/WebGL2, and the fallbacks render at `:80`/`:109`. There is no installed `border-beam`, `thinking-orbs`, `bot-avatars`, `liquid-gooey`, or `voice-glow` package to cite.

## Accessibility and reduced-motion behavior

- Keyboard focus is visible through the global `:focus-visible` rule at `styles/base.css:165-170`, with component-specific focus blocks at `shell.css:264-268`, `:381`, `:619-620`, and `:714`.
- The command palette exposes a combobox/listbox contract (`CommandPalette.tsx:255-260`, `:311-323`). The cheat sheet is a labelled modal with a focus trap (`CheatSheet.tsx:115-120`). Onboarding is a labelled modal with initial focus, Tab wrapping, Escape, and focus restoration (`OnboardingOverlay.tsx:44-100`).
- Editor tabs use `role="tablist"`, `role="tab"`, `aria-selected`, and roving `tabIndex` (`components/editor/EditorTabBar.tsx:99-114`). The minimap exposes `role="slider"` and value text (`EditorMinimap.tsx:75-83`).
- Reduced motion has two CSS paths: the OS media query and the in-app root attribute. Both set `--motion-loops: 0` (`styles/tokens.css:229-248`); loop and one-shot overrides live at `styles/animations.css:288-330`.
- The app applies the flag from `bootstrap.settings.reduced_motion` at `app-store.ts:114-120` and `:240`. The JavaScript presence hook reads both OS preference and the root attribute (`features/onboarding/motion.ts:5-46`), and passes `paused` to `img-fx`/`metal-fx` (`BrandFx.tsx:90`, `:119`).
- The Settings panel's Reduced Motion switch is local React state only (`SettingsPanelView.tsx:51`, `:411-415`); it never sends `SettingsUpdate`, and `SettingsUpdate` is unsupported. Therefore the switch cannot actually change the root `data-reduced-motion` attribute. The panel itself states that editing is a later slice (`SettingsPanelView.tsx:496-499`).

## Security decisions

- **Workspace rooting.** `Workspace::open` requires a directory and canonicalizes the root (`workspace.rs:110-126`). Relative paths reject control characters, absolute/UNC forms, drive prefixes, empty segments, `.` and `..` (`workspace.rs:445-461`). Resolution canonicalizes the candidate and then checks `starts_with(&self.root)` (`workspace.rs:464-506`); symlink escape tests exist at `workspace.rs:904-928`.
- **File limits.** The editor limit is 2 MiB (`workspace.rs:26`). Both metadata and actual bytes are checked (`workspace.rs:179-202`), NUL bytes are rejected as binary, and non-UTF-8 is rejected. Saves use a same-directory temporary file, `sync_all`, and rename (`workspace.rs:685-715`).
- **UTF-16 positions.** The wire uses `line` plus `utf16_column` (`selora-protocol/src/lib.rs:170-174`). `lsp_bridge.rs:421-455` converts protocol UTF-16 columns to code-point positions, clamps out-of-range lines, and resolves an offset inside a surrogate pair to the nearest character boundary.
- **LSP bounds.** Completions are capped at 200 and definitions at 50 (`lsp_bridge.rs:52-55`). Closing a buffer prunes its diagnostics and emits the pruned list (`lsp_bridge.rs:157-170`).
- **SSRF.** The agent's `web_fetch` path refuses non-HTTP(S), localhost names, private/loopback/link-local literals, and IPv4-mapped private IPv6; redirects are followed manually and re-checked per hop (`crates/selora-ai/src/agent/tools/web.rs:63-129`, `:468-548`). Separately, `connect_api_key` accepts any user-entered `http://` or `https://` gateway URL, including loopback, with no private-range restriction (`commands.rs:82-93`). That is deliberate for a local/mock gateway, but it is a policy choice the owner must confirm for production.
- **Approval gate.** `validate_and_consume_approval` checks request, session, turn, and approval ids and consumes the card only on a match (`selora-protocol/src/lib.rs:1170-1197`). The engine calls it before forwarding the decision; mismatches leave the card parked (`sessions.rs:564-645`).
- **External links and navigation.** The protocol's external-URL validator accepts only absolute HTTPS with a host and no userinfo (`selora-protocol/src/lib.rs:131-149`), but `OpenExternal` is unsupported. The Tauri window denies new windows and restricts navigation to the local origin (plus the debug Vite origin) (`main.rs:74-75`, `security.rs:33-51`). The CSP allows GitHub's release API but no general external connect origin (`tauri.conf.json:34-35`).

## Performance

`npm run build` produced these files from the current tree:

| Output | Minified | Gzip |
|---|---:|---:|
| `index.html` | 0.36 kB | 0.26 kB |
| `assets/ConnectModal-DolYKu3H.css` | 11.45 kB | 2.66 kB |
| `assets/index-7tyRdQcP.css` | 176.21 kB | 29.43 kB |
| `assets/ConnectModal-C5Wd-g9j.js` | 17.40 kB | 4.91 kB |
| `assets/AgentDock-C540MghV.js` | 81.88 kB | 20.77 kB |
| `assets/index.es-D3pFN5mn.js` (metal-fx) | 89.55 kB | 30.51 kB |
| `assets/index-BAXdd9OK.js` (startup entry) | 476.20 kB | 120.54 kB |
| `assets/index.es-DEQ2Pe0K.js` (img-fx + three) | 585.87 kB | 150.17 kB |
| `assets/CodeMirrorPane-DhPA_8BE.js` | 706.82 kB | 245.80 kB |

The startup entry is `dist/index.html`'s sole module script, so the initial payload is the 476.20 kB JS plus the 176.21 kB CSS entry: 652.41 kB minified and 149.97 kB gzip before any lazy route. `AgentDock` (81.88 kB), `CodeMirrorPane` (706.82 kB), `ConnectModal` (17.40 kB), and both visual-effect chunks are dynamic imports (`ShellGrid.tsx:37-40`, `EditorRegion.tsx:23-25`, `App.tsx:29-32`, `BrandFx.tsx:48-54`).

Known performance facts and regressions:

- Vite warns that `CodeMirrorPane` and the img-fx/three chunk exceed 500 kB; the 585.87 kB effect chunk is larger than the entire startup entry.
- No cold-start, time-to-interactive, typing-latency, or streaming-framerate measurement exists in the tree. Bundle size is the only measured number; it must not be presented as startup performance.
- `@codemirror/lint` is imported by `CodeMirrorPane.tsx:45` and exists transitively in `package-lock.json:454-456`, but it is not declared in `package.json` dependencies. A clean install can change or drop it without a direct dependency failure until typecheck/build.

## Remaining parity gaps

1. **Open Folder permission gap — deferred (source-owner fix; Z6 may not edit source).** `pick_folder` is registered at `main.rs:106`, but the capability list omits `allow-pick-folder` (`capabilities/default.json:7-22`) and `build.rs:29-37` omits `pick_folder` from the app manifest. A Tauri build cannot prove the runtime ACL will allow the invoke. The owner/orchestrator must add both entries and smoke-test Open Folder.
2. **CloseBuffer reconciliation — deferred (source-owner fix).** Rust removes the buffer (`state.rs:732-743`) but emits no event; the frontend ignores the returned bootstrap (`editor-store.ts:261-268`). The tab can remain visible until another bootstrap-state event. A `BufferClosed` event or frontend reply handling is required.
3. **Settings editing — deferred (SettingsUpdate is unsupported and the panel is presentation-only).** Theme, reduced motion, editor, gateway, and accent controls are local state (`SettingsPanelView.tsx:33-52`, `:395-433`); the panel advertises a later slice at `:496-499`.
4. **External links — deferred (OpenExternal is unsupported and new windows are denied).** About and Connect use `target="_blank"` (`AboutDialog.tsx:176`, `ConnectModal.tsx:534-535`), while `main.rs:75` denies new windows. GitHub release metadata is fetched in the webview under a fixed CSP origin, but repository/provider docs links have no working native opener.
5. **Session lifecycle — deferred (RetryMessage, SwitchSession, and NewSession are unsupported).** The session rail mutates `agentStore` locally (`AgentSessionRail.tsx:48`, `agent-store.ts:95-105`, `:324-346`); Rust has no session switch/new/retry arm.
6. **Subagent and plan/todo authority — deferred (Phase 9 runtime absent).** `INITIAL_SUBAGENTS` is seeded locally (`agent-store.ts:109-133`), tool results only update the first local roster entry (`:1018-1031`), and todos are local/prompt-derived (`:448-460`). The plan spine is a renderer, not a Rust-authoritative plan.
7. **Regex search — deferred (Lite search is literal-only).** `workspace.rs:329-336` returns `unsupported_query` when `use_regex` is true; hits are capped at 500 (`workspace.rs:345`).
8. **`@codemirror/lint` is imported but undeclared — deferred (dependency-owner fix; Z6 may not edit source or manifests).** `CodeMirrorPane.tsx:45` imports it and `package-lock.json:454-456` resolves it transitively, but `package.json` does not list it. It survives today because CodeMirror ships it as a dependency of the editor packages; a clean install or a future CodeMirror bump can change the resolved version without any direct-dependency failure. Add it to `dependencies` explicitly.
9. **Five of the seven named Libraries.dev packages are not installed — blocked on owner decision.** Only `img-fx@0.5.1` and `metal-fx@2.0.11` are installed (`package.json:27-28`); `border-beam`, `thinking-orbs`, `bot-avatars`, `liquid-gooey`, and `voice-glow` are absent. The tree substitutes local ports under `features/agent/presence/`, and `LiquidPill` is imported only by a test. The owner must choose: install and verify the real packages (license, CSP, WebView2), or accept the local ports as the shipped implementation and record that deviation.
10. **No manual WebView2 or installer verification — deferred (packaging slice; requires an owner-run smoke test).** Every gate here is automated and headless. `tauri.conf.json:39` sets `bundle.active=false`; Windows packaging is instead the separate Inno Setup script (`installer/selora-ide.iss:23`), and no built installer is present (`installer/output` does not exist). No one has exercised the real WebView2 runtime, GPU fallbacks, or the native folder dialog on a clean Windows machine. Green Rust tests cannot substitute for that walkthrough.
11. **No runtime performance baseline — deferred (needs profiling hardware and a session).** The tree contains bundle sizes only. There is no cold-start, time-to-interactive, typing-latency, streaming-framerate, or memory measurement, and no benchmark harness to produce one. The 149.97 kB gzip initial payload must not be quoted as startup time.
12. **No `.git` repository — blocked on owner decision.** `Test-Path .git` is `False`. There is no history, no branches, and no diff-based review; work can only be verified by building and reading the tree. If merge gates or rollback are required, the owner must initialize a repository (or point this checkout at one) before further parallel slices.
13. **Phase 8 and Phase 9 are unimplemented — deferred (out of the shipped Lite scope).** `ROADMAP.md:587` (workflows / dynamic multi-agent orchestration) and `ROADMAP.md:647` (Agent Workbench) have no runtime in the tree: no workflow engine, no `spawn_subagent` command, no live subagent protocol events, no plan approval gate, and no Rust-authoritative plan/todo state. The agent dock renders local rosters and prompt-derived todos only. The reusable extension points are the typed command/event enums, the `state.rs` dispatch/`record_event` pattern, the `LayoutStore` read-modify-write precedent, and the event-replay gate; new slices should add variants and dispatch arms rather than new Tauri commands, and must make `AgentPlanSpine.tsx`, `AgentSessionRail.tsx`, and `AgentExecutionFeed.tsx` data Rust-authoritative before it can be trusted.

## Whether the egui shell can be safely cut over

**No.** The automated gates are green, but a cutover would remove the only surface that works today while several user-visible flows are still broken or absent. The Tauri shell is safe to keep shipping *additively*; it is not safe to make the sole shell yet. Each blocking item is verified in the tree:

1. **Open Folder may be denied at runtime.** `pick_folder` is registered at `main.rs:106`, but `capabilities/default.json:8-22` has no `allow-pick-folder` and `build.rs:29-37` omits it from the app manifest. Build success does not prove the ACL permits the invoke, so the primary way to open a workspace is unproven on a packaged build.
2. **Closing a buffer can leave a phantom tab.** `state.rs:732-743` removes the Rust buffer and emits no event; `editor-store.ts:261-268` discards the returned bootstrap. The tab stays until an unrelated bootstrap event arrives.
3. **Settings do not persist.** The panel writes local React state only (`SettingsPanelView.tsx:33-52`), and `SettingsUpdate` is `unsupported_phase_slice`. Theme, reduced motion, editor, gateway, and accent changes are lost on restart.
4. **External links do nothing.** `OpenExternal` is unsupported and `main.rs:75` denies new windows, while About and Connect still render `target="_blank"` links (`AboutDialog.tsx:176`, `ConnectModal.tsx:534-535`).
5. **Session lifecycle is local-only.** `RetryMessage`, `SwitchSession`, and `NewSession` are unsupported; the session rail mutates `agentStore` in memory (`agent-store.ts:95-105`, `:324-346`) and loses state on reload.
6. **No one has run the product.** With `bundle.active=false`, a separate unbuilt Inno script (`installer/selora-ide.iss:23`), and no recorded execution of the 18-step walkthrough at `PHASE_Z_TAURI_PROMPT.md:707-726`, the shell has never been validated as an end-user artifact.

Cutover becomes defensible once items 1-3 are fixed in source, sessions are either wired or explicitly scoped out, and the owner completes a manual Windows walkthrough covering open-folder, edit/save/close, LSP hover, a chat round-trip, and layout restore. Until then the egui shell should remain the fallback and the Tauri shell should stay additive.

---

## Orchestrator addendum (2026-10-26, after the report was written)

Three findings above were fixed in the same session, after this report's snapshot:

1. **Gap 1 (Open Folder ACL) - fixed.** `apps/selora-desktop/src-tauri/build.rs`
   now declares `pick_folder` in the app manifest and
   `capabilities/default.json` grants `allow-pick-folder` (plus
   `allow-open-external` and `allow-update-settings` for the two seams below).
   `cargo build -p selora-desktop` regenerates the permission sets from the
   edited manifest, so the ACL is generated from source rather than assumed.
2. **Gap 2 (CloseBuffer reconciliation) - fixed.** The `CloseBuffer` arm in
   `state.rs` now records `FrontendEvent::BootstrapState` with the post-close
   snapshot (the union has no `BufferClosed`), and a state test
   (`z4_close_buffer_publishes_a_bootstrap_event`) asserts both the returned
   bootstrap and the emitted event drop the buffer.
3. **Gap 8 (`@codemirror/lint` undeclared) - fixed** by adding it to
   `apps/selora-desktop/frontend/package.json` as a direct dependency.

4. **Gap 7 (regex search) - fixed.** `crates/selora-fs/src/search.rs` now ships a regex
   matcher (`search_in_files_regex`) that compiles the pattern once, honours
   `case_sensitive` and `whole_word`, reports 1-based character columns and the same
   500-hit cap; `workspace.rs` dispatches on `use_regex` and returns `invalid_query`
   for a bad pattern instead of `unsupported_query`.

5. **Gap 3 (settings) - fixed.** `FrontendCommand::SettingsUpdate` now validates
   `theme`/`reduced_motion`, writes through `selora-settings`, and republishes a
   `BootstrapState`; every control with no backend field renders disabled with an
   explanatory title, so nothing claims a persistence it does not have.
6. **Gap 4 (external links) - fixed.** `FrontendCommand::OpenExternal` validates
   scheme, host (eight-entry allowlist), credentials and port, then opens the URL
   through `tauri-plugin-opener` (Win32 `ShellExecuteExW`, never a shell string).
   About's repository link and Connect's provider links both use it.

The remaining gaps stand as recorded; gaps 9 and 12 need an owner decision, and
gap 11 (no runtime performance baseline) still has no measurement.

---

## 2026-10-26 hardening addendum

A later pass the same day fixed the defects below. References are to the code
that now carries each fix; this is not a claim that a window was launched or
that the app was exercised by hand. The audit above is left unchanged as the
snapshot it was.

1. **Editor version, multi-change, and tab switching.** Local CodeMirror
   transactions are converted to protocol changes with one range per change
   (`CodeMirrorPane.tsx:194-215`), sent with the current backend version as
   their base (`:288-315`), and unacknowledged edits survive a buffer switch
   with a monotonic local version. Equal-version stale echoes are ignored;
   newer remote snapshots apply. Tests: `CodeMirrorPane.test.ts:19-46`,
   `CodeMirrorPane.mount.test.tsx:109-162`.
2. **Session envelopes, replay, and hydration.** `SendChat`, `SendAgent`,
   `StopChat`, and `AgentDecision` reject an envelope session that disagrees
   with the payload (`sessions.rs:361-365`, `:424-428`, `:548-552`, `:644`),
   and the tests cover version/session refusals (`:1796-1868`). The startup
   path replays `getEventsAfter(0)` through a null-seeded gate without
   reordering live events (`app-store.ts:172-242`, `events.rs`), and the agent
   store hydrates the Rust session rail and transcript rather than seeding a
   synthetic session (`agent-store.ts:303`).
3. **Approval key scoping.** `validate_and_consume_approval` matches request,
   session, turn, and approval ids and consumes the id once
   (`selora-protocol/src/lib.rs:1198-1226`); a mismatch returns
   `approval_mismatch`, leaves the card parked, and executes nothing
   (`sessions.rs:608-644`, `:2344-2397`).
4. **LSP paths.** The URI helpers strip Windows' `\\?\` verbatim prefix in
   both directions, including `\\?\UNC\` (`selora-lsp/src/uri.rs:61-66`). The
   registry probes `.exe` first, then npm's `.cmd`/`.bat` shims (the
   extensionless POSIX script no longer wins), and those shims are spawned
   through `cmd.exe` with a quoted command line (`registry.rs:182-212`,
   `:354-376`; `client.rs:389-390`).
5. **Bash allowlist.** The auto-run check is a prefix match with a word
   boundary on simple commands only, so `cargo testx`, `git status ; ...`, and
   a command that merely contains an allowlisted prefix all fall through to a
   card (`crates/selora-ai/src/agent/tools/bash.rs:156-170`, `router.rs:367`).
6. **File reads and writes.** Large reads support a bounded line slice, and
   every truncation cuts on a character boundary
   (`crates/selora-ai/src/agent/tools/fs.rs:207-229`, `:305-388`). Saves use a
   unique sibling temp name (`.<name>.selora-tmp-<pid>-<seq>`, `create_new`)
   so two concurrent writes cannot truncate each other's temp (`:786-833`).
7. **Git staged diffs.** A staged row now shows HEAD-to-index instead of an
   empty working-tree diff; a file with both staged and unstaged changes shows
   both sides, staged first (`workspace.rs:417-446`, test at `:1321-1358`).
8. **Settings truthfulness.** `SettingsUpdate` validates and persists `theme`
   and `reduced_motion` and republishes bootstrap (`state.rs:554`,
   `:1050-1051`, tests at `:1561-1618`); every control without a backend field
   renders disabled with an explanatory title and the panel states that the
   remaining controls are read-only (`SettingsPanelView.tsx:40`, `:305-330`,
   `:420`).
9. **Release gate.** `release.yml` now requires the tag to be an ancestor of
   `origin/main` (`:61-74`), equal to `Cargo.toml`, `tauri.conf.json`, **and**
   `frontend/package.json`, with a `## [<version>]` CHANGELOG heading
   (`:76-115`), and re-runs fmt, clippy, and cargo-deny (`:188-204`).
   `CHANGELOG.md` now has the `0.1.0` section; `RELEASE.md` §4.3/§5 match.
10. **Accessibility.** Dialog focus traps and focus restoration are in place
    for the cheat sheet, About, palette, and Connect
    (`CheatSheet.tsx:55-67`, `AboutDialog.tsx:93-106`,
    `CommandPalette.tsx:116-147`, `ConnectModal.tsx:253-277`); status and
    agent-log updates announce through polite live regions
    (`StatusBar.tsx:168`, `AgentExecutionFeed.tsx:143`). The full record stays
    in `docs/ui/Z6_A11Y_AUDIT.md`.

### Remaining gaps (still open)

- **Session lifecycle.** `SwitchSession`, `NewSession`, and `RetryMessage`
  still fall through to `unsupported_phase_slice` (`state.rs:1053-1055`); the
  rail remains local UI state (`agent-store.ts:502-524`).
- **Subagent and plan authority.** The roster is seeded locally
  (`agent-store.ts:160-184`) and the plan/todo spine is prompt-derived; no
  Rust-authoritative plan or `spawn_subagent` runtime exists.
- **Manual walkthrough.** No one has run the packaged app. Open Folder on a
  packaged build, LSP hover/definition against a real server, the WebView2
  path, and the installer all still need an owner-run pass.
- **Installer artifacts.** `installer/output/` does not exist and
  `tauri.conf.json` keeps `bundle.active=false`; no built installer is in the
  tree.
- **Libraries packages.** Only `img-fx` and `metal-fx` are installed; the other
  five named packages remain local ports (see the table above).
- **Performance baseline.** Still no cold-start, typing-latency,
  streaming-framerate, or memory measurement; bundle sizes are the only
  numbers.
- **Git repository.** This checkout still has no `.git`, so there is no
  commit, branch, or diff history.
