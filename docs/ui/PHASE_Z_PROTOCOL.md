# Phase Z protocol

`crates/selora-protocol` is the shared Rust DTO boundary for the native shell. It defines the protocol version, stable validated identifiers, workspace-relative paths, UTF-16 text positions, versioned selections, request/event envelopes, bootstrap state, and typed command/event payloads.

Every envelope carries protocol/workspace/core epoch metadata. Snapshot sequences are monotonic; stale sequence and sequence-gap conditions are represented as typed validation errors. The `core_instance_id` changes on a new Rust process, so a reconnect must request a fresh bootstrap before applying events. Approval identifiers are validated and consumed once. External URLs require absolute HTTPS with an explicit host and no userinfo. These checks keep path handling, URL opening, and approval state out of untrusted frontend code.

The frontend passes `protocolVersion` when requesting bootstrap. Unsupported commands return `unsupported_phase_slice` while later vertical slices are added behind feature flags. The protocol tests cover validation, ordering, stale/gap detection, approval consumption, and URL restrictions.

## Z4, Z4b and Z5 as shipped

Z4 (editor and workbench parity), Z4b (the LSP bridge), and Z5 (brand/onboarding) reuse the Z1 transport instead of adding Tauri commands. Every invoke still goes through `dispatch_command`, so `capabilities/default.json:8-22` is unchanged and the only IPC surface remains `get_bootstrap_state`, `dispatch_command`, `get_events_after`, `connect_api_key`, `disconnect_api_key`, `get_api_key_status`, and `fetch_models` (`build.rs:29-37`).

`FrontendCommand` now has **32 variants** (`crates/selora-protocol/src/lib.rs:353-466`); **24 reach a dispatch arm** in `state.rs:584-850`, and the remaining **8** fall through to `unsupported_phase_slice` (`adapter.rs:3-8`). `FrontendEvent` has **25 variants** (`lib.rs:1029-1058`). The TypeScript union at `bridge.ts:186-212` enumerates 27 commands: the five never-exposed commands (`find_in_buffer`, `replace_current`, `replace_all`, `open_external`, `settings_update`) are absent from the frontend type entirely, and `open_folder` is present but not used by the normal UI path.

### Command payloads

Wire discrimination is `{ "type": "...", "payload": ... }` (`#[serde(tag = "type", content = "payload", rename_all = "snake_case")]`, `lib.rs:351-352`). Field names below are the serialized names.

| Command | Payload fields | Status | Result event |
|---|---|---|---|
| `get_bootstrap_state` | none | implemented | `bootstrap_state` |
| `open_folder` | none | implemented via `SELORA_WORKSPACE_ROOT`; returns `use_folder_picker` otherwise (`state.rs:677-697`) | `bootstrap_state` |
| `open_folder_at` | `root: string` | implemented (`state.rs:698-710`) | `bootstrap_state` |
| `refresh_tree` | none | implemented (`state.rs:711-714`) | `file_tree_changed` |
| `open_file` | `path: string` (workspace-relative) | implemented (`state.rs:716-731`) | `buffer_snapshot` |
| `close_buffer` | `buffer_id: string` | implemented, **silent** (`state.rs:732-743`) | none |
| `save_buffer` | `buffer_id: string` | implemented (`state.rs:745-756`) | `buffer_snapshot` |
| `apply_editor_change` | `buffer_id: string`, `version: number`, `changes: TextChange[]` | implemented (`state.rs:758-774`) | `buffer_changed` |
| `set_selection` | `buffer_id: string`, `selection: Selection` | implemented, **silent** (`state.rs:776-782`) | none |
| `hover_at` | `buffer_id: string`, `position: TextPosition` | implemented (`state.rs:783-791`) | `hover_result` |
| `definition_at` | `buffer_id: string`, `position: TextPosition` | implemented (`state.rs:792-800`) | `definition_result` |
| `completion_at` | `buffer_id: string`, `position: TextPosition` | implemented (`state.rs:801-809`) | `completion_result` |
| `send_chat` | `request_id: string`, `request: { session_id, message, model_id: string\|null }` | implemented (`state.rs:590-602`) | `chat_delta*`, then `chat_completed` or `chat_failed` |
| `stop_chat` | `request_id: string` | implemented (`state.rs:603-609`) | `chat_completed` (cancelled; partial kept) |
| `send_agent` | `request_id: string`, `request: { session_id, prompt, model_id: string\|null, mode: "execute"\|"explore", reasoning_effort?: "quick"\|"standard"\|"max", system_instructions?: string, temperature?: number }` | implemented (`state.rs:610-622`) | `tool_call_started`, `tool_result`, `approval_needed`, `agent_state_changed`, `agent_turn_done` |
| `agent_decision` | `request_id: string`, `approval_id: string`, `decision: "approve"\|"deny"\|"approve_all_this_turn"\|"stop"` | implemented (`state.rs:623-635`) | continued `agent_*` stream |
| `retry_message` | `session_id: string`, `message_id: string` | `unsupported_phase_slice` | — |
| `switch_session` | `session_id: string` | `unsupported_phase_slice` | — |
| `new_session` | none | `unsupported_phase_slice` | — |
| `search` | `query: { query: string, case_sensitive: bool, whole_word: bool, use_regex: bool }` | implemented, literal only (`state.rs:810-814`) | `search_results` |
| `find_in_buffer` | `buffer_id: string`, `query: string`, `options: { case_sensitive, whole_word, use_regex }` | `unsupported_phase_slice` | — |
| `replace_current` | `buffer_id: string`, `replacement: string` | `unsupported_phase_slice` | — |
| `replace_all` | `buffer_id: string`, `replacement: string` | `unsupported_phase_slice` | — |
| `git_stage` | `path: string` | implemented (`state.rs:815-823`) | `git_status_changed` |
| `git_unstage` | `path: string` | implemented (`state.rs:824-833`) | `git_status_changed` |
| `git_commit` | `message: string` | implemented (`state.rs:834-842`) | `git_status_changed` |
| `open_git_diff` | `path: string` | implemented (`state.rs:843-846`) | `git_diff_ready` |
| `toggle_surface` | `surface: "files"\|"search"\|"git"\|"problems"\|"agent"\|...` | implemented (`state.rs:636-647`) | `workspace_layout_changed` |
| `resize_region` | `region: string`, `size_px: number` | implemented with clamping (`state.rs:648-666`) | `workspace_layout_changed` only when the size changed |
| `save_layout` | none | implemented (`state.rs:667-676`) | none |
| `open_external` | `url: string` | `unsupported_phase_slice` | — |
| `settings_update` | `patch: SettingsPatchDto` | `unsupported_phase_slice` | — |

Nested editor types are `TextPosition { line: u32, utf16_column: u32 }`, `TextChange { start: TextPosition, end: TextPosition, replacement: String }`, and `Selection { anchor: TextPosition, active: TextPosition, version: u64 }` (`lib.rs:171-188`). Search is `SearchQueryDto { query, case_sensitive, whole_word, use_regex }` (`lib.rs:230-235`).

### Event payloads and routing

Events arrive on `selora://event` as `EventEnvelope` (`lib.rs:1064-1073`): `protocol_version`, `workspace_id`, `core_instance_id`, `sequence`, `request_id`, `session_id`, `payload`. `request_id` is optional because silent and LSP paths record `None`.

| Event | Payload | `request_id` source |
|---|---|---|
| `bootstrap_state` | `BootstrapState` | command envelope id |
| `chat_delta` / `chat_completed` / `chat_failed` | `ChatDeltaDto` / `ChatCompletedDto` / `ChatFailedDto` | the payload `request_id`, echoed into the envelope (`sessions.rs:957`, `:982`, `:1004`) |
| `agent_event` | `AgentEventDto` | none — **no production emitter** |
| `tool_call_started` / `tool_result` / `approval_needed` / `agent_state_changed` / `agent_turn_done` | typed `Agent*Dto` | envelope id where the turn owns one; otherwise session-routed |
| `diagnostics_changed` | `DiagnosticDto[]` | `None`; route by `buffer_id` in the DTO |
| `hover_result` / `completion_result` / `definition_result` | `HoverResultDto` / `CompletionResultDto` / `DefinitionResultDto` | `None`; route by `buffer_id` (`state.rs:400-402`) |
| `file_tree_changed` | `FileTreeDto` | command envelope id |
| `buffer_snapshot` / `buffer_changed` | `BufferSnapshotDto` / `BufferChangedDto` | command envelope id; `buffer_changed` carries `version` for stale-edit rejection |
| `search_results` | `SearchResultsDto` | command envelope id; `SearchResultsDto.request_id` is populated as `None` (`workspace.rs:365-369`), so the envelope is the only correlation |
| `git_status_changed` / `git_diff_ready` | `GitStatusDto` / `GitDiffDto` | command envelope id |
| `models_changed` / `selora_status_changed` | `ModelsDto` / `SeloraStatusDto` | command envelope id, or `None` for engine-originated pushes |
| `workspace_layout_changed` | `LayoutDto` | command envelope id |
| `toast` | `ToastDto` | envelope id when tied to a command; `None` for unsolicited LSP toasts (`lsp_bridge.rs:249`) |
| `error` | `ProtocolErrorDto` | none — **no production emitter** |

### `request_id` echo rules

1. **Transport id.** Every `RequestEnvelope` carries a fresh UUID in `request_id` (`bridge.ts:317-325`). This id is for the invoke round-trip only.
2. **Session stream id.** `send_chat`, `stop_chat`, `send_agent`, and `agent_decision` also carry a payload `request_id`. `stop_chat` and `agent_decision` name that id to act on an in-flight request, and every resulting `chat_*` event repeats it in both the DTO and the envelope. A stale or unknown id is rejected rather than retargeted.
3. **Command-scoped events.** Workspace, search, Git, and layout events put the command envelope id in `EventEnvelope.request_id`.
4. **LSP events.** `hover_at`, `definition_at`, and `completion_at` record with `None` and route by `buffer_id`, so a late answer for a closed tab is dropped by the editor rather than applied to another buffer.
5. **Silent commands.** `set_selection` and a successful `close_buffer` emit nothing. `resize_region` emits only when the clamped size actually changes, and an idempotent drag end is silent (`state.rs:648-666`).

Unsupported commands are rejected before touching state and return `unsupported_phase_slice` through `adapter.rs:3-8`. `FeatureFlagsDto` advertises `agent`, `git`, `search`, and `editor` as `true`, with `later_slices: ["lsp-bridge", "workspace-trust"]` (`state.rs:518-524`); no UI component branches on those flags.
