// Selora IDE — Phase Z2 frontend/protocol bridge.
//
// The typed seam between the web shell and the Rust core
// (docs/ui/PHASE_Z2_ARCHITECTURE.md §2.5). Components never hand-assemble a
// RequestEnvelope and never call `invoke` directly; they go through the
// helpers below so every dispatch carries the Z1-era transport validation
// context (protocol version, core epoch, workspace id) and every command
// failure surfaces through one observable error path.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export const PROTOCOL_VERSION = 1;

export type Id = string;
export type ConnectionStatus = 'connected' | 'connecting' | 'disconnected';
export type TextPosition = { line: number; utf16_column: number };
export type Selection = { anchor: TextPosition; active: TextPosition; version: number };
export type FileTreeNode = { path: string; name: string; kind: 'directory' | 'file'; children: FileTreeNode[] };
export type Approval = { approval_id: Id; request_id: Id; turn_id: Id; session_id: Id; summary: string; can_approve_all_this_turn: boolean };
export type Diagnostic = { path: string; line: number; column: number; severity: 'error' | 'warning' | 'info' | 'hint'; message: string };

// --- Z2 additions (ARCHITECTURE §2.5) ---------------------------------------

export type SurfaceId =
  | 'files'
  | 'search'
  | 'git'
  | 'problems'
  | 'settings'
  | 'agent'
  | 'workflows'
  | 'output'
  | 'terminal_placeholder';
export type RegionId = 'primary_sidebar' | 'agent_dock' | 'bottom_dock';
export type ToastSeverity = 'info' | 'success' | 'warning' | 'error';

export type Layout = {
  primary_sidebar_width_px: number;
  agent_dock_width_px: number;
  agent_dock_open: boolean;
  bottom_dock_height_px: number;
  bottom_dock_open: boolean;
  bottom_view: SurfaceId;
  editor_splits: number;
  activity_view: SurfaceId;
  primary_sidebar_open: boolean;
};

export type LayoutConstraints = {
  primary_sidebar_min_px: number;
  primary_sidebar_max_px: number;
  agent_dock_min_px: number;
  agent_dock_max_px: number;
  bottom_dock_min_px: number;
  bottom_dock_max_px: number;
  bottom_dock_max_workbench_ratio: number;
  editor_min_width_px: number;
  resize_hit_zone_min_px: number;
};

export type SettingsView = {
  reduced_motion: boolean;
  theme: string;
  keybind_command_palette: string;
  keybind_toggle_chat: string;
};

/** The backend-supported writable subset of `SettingsView` (SettingsPatchDto). */
export type SettingsPatch = {
  reduced_motion?: boolean;
  theme?: string;
};

// --- DTOs referenced by the full FrontendEvent union ------------------------

export type BufferSnapshot = {
  buffer_id: Id; path: string; language: string; text: string; version: number;
  dirty: boolean; selection: Selection | null;
};
/** P2 gap 3: a dirty buffer's file also changed on disk; local text is intact. */
export type BufferConflict = { buffer_id: Id; path: string };
/** P2 gap 3: the external-change marker for this buffer is cleared. */
export type BufferConflictCleared = { buffer_id: Id; path: string };
export type TextChange = { start: TextPosition; end: TextPosition; replacement: string };
/** Where the user was when the turn was sent. Reference only, never file text. */
export type EditorContextDto = {
  path: string;
  language: string;
  /** Omitted by the sender when the selection is empty. */
  selection?: Selection;
  dirty: boolean;
};
export type Model = { id: string; display_name: string; available: boolean };
export type SearchQuery = { query: string; case_sensitive: boolean; whole_word: boolean; use_regex: boolean };
export type SearchHit = { path: string; line: number; column: number; preview: string };
export type ChatSession = { session_id: Id; title: string; message_ids: Id[]; active: boolean };
export type ChatMessage = { message_id: Id; session_id: Id; role: 'user' | 'assistant' | 'tool' | 'system'; text: string };
export type TextRange = { start: TextPosition; end: TextPosition };
export type HoverResult = { buffer_id: Id; contents: string | null; range: TextRange | null };
export type CompletionItem = { label: string; detail: string | null; insert_text: string | null; kind: string | null };
export type CompletionResult = { buffer_id: Id; items: CompletionItem[] };
export type DefinitionLocation = { path: string; line: number; utf16_column: number };
export type DefinitionResult = { buffer_id: Id; locations: DefinitionLocation[] };
/** P1 — one language server's live lifecycle. */
export type LspServerHealth = 'starting' | 'ready' | 'failed';
export type LspServerState = {
  language: string;
  server: string;
  state: LspServerHealth;
  /** The truthful failure reason; only present when state is `failed`. */
  error?: string | null;
};

export type BootstrapState = {
  protocol_version: number;
  core_instance_id: Id;
  snapshot_sequence: number;
  workspace: { id: Id; core_instance_id: Id; root: string | null; display_name: string };
  buffers: BufferSnapshot[];
  active_buffer_id: Id | null;
  file_tree: { root: FileTreeNode | null };
  models: Model[];
  active_model_id: string | null;
  connection: ConnectionStatus;
  chat: {
    sessions: ChatSession[];
    messages: ChatMessage[];
    active_session_id: Id | null;
    active_request_id: Id | null;
    active_turn_id: Id | null;
    pending_approval: Approval | null;
  };
  settings: SettingsView;
  layout: Layout;
  layout_constraints: LayoutConstraints;
  git: {
    branch: string | null;
    changed_paths: string[];
    staged_paths: string[];
    files?: { path: string; staged: boolean; worktree: boolean }[];
  };
  diagnostics: Diagnostic[];
  /** P1: live language-server state (empty until the first file of a
   * supported language opens). */
  lsp_servers: LspServerState[];
  feature_flags: { agent: boolean; git: boolean; search: boolean; editor: boolean; later_slices: string[] };
  workflows?: WorkflowPersistedRun[];
};

export type WorkflowPersistedRun = {
  run_id: Id;
  name: string;
  status: WorkflowStatus;
  resumable: boolean;
  detail?: string | null;
  /** Additive Phase 8 history from the run's terminal journal report. */
  phases?: string[];
  steps?: WorkflowStep[];
  reports?: WorkflowReport[];
  artifacts?: WorkflowArtifact[];
  tokens?: number;
  cost_usd?: number;
};

export type ProtocolError = { code: string; message: string; retryable: boolean };

export type ConnectResult = {
  connected: boolean;
  models_count: number;
  active_model_id: string | null;
  base_url: string;
};

export type ApiKeyStatus = {
  connected: boolean;
  has_key: boolean;
  source: string;
  base_url: string;
  key_hint: string | null;
  active_model_id: string | null;
};

export type Decision = 'approve' | 'deny' | 'approve_all_this_turn' | 'stop';
export type AgentMode = 'execute' | 'explore' | 'auto';
export type AgentPhase = 'thinking' | 'running_tool' | 'awaiting_approval' | 'done';

export type ToolCallStartedPayload = {
  request_id: Id;
  session_id: Id;
  turn_id: Id;
  call_id: string;
  tool_name: string;
  args_json: string;
};

export type ToolResultPayload = {
  request_id: Id;
  session_id: Id;
  turn_id: Id;
  call_id: string;
  result_summary: string;
  is_err: boolean;
};

export type ApprovalNeededPayload = {
  request_id: Id;
  session_id: Id;
  turn_id: Id;
  approval_id: Id;
  tool_name: string;
  path_or_command: string;
  diff_or_input?: string;
  /**
   * Phase 8 (§18.1): the `run_workflow` confirmation payload. Present only on
   * a workflow card; absent on every other card and on pre-8 payloads.
   */
  workflow?: WorkflowApproval;
};

export type AgentStateChangedPayload = {
  request_id: Id;
  session_id: Id;
  turn_id: Id;
  state: AgentPhase;
  tool_name?: string;
};

export type AgentTurnDonePayload = {
  request_id: Id;
  session_id: Id;
  turn_id: Id;
  total_tokens: number;
  cost_usd: number;
  num_tools: number;
  approved: number;
  denied: number;
  completed: boolean;
  end_message: string;
};

export type ChatCompletedPayload = {
  request_id: Id;
  session_id: Id;
  turn_id: Id;
  message_id: Id;
  text: string;
  total_tokens: number;
  cost_usd: number;
};

// --- Phase 8 workflow additions (ARCHITECTURE §18) --------------------------
// Additive to protocol_version 1. IDs are engine-minted opaque strings (script
// names are common), not the stable-id types used by chat/agent ids.

export type WorkflowStatus = 'pending' | 'running' | 'awaiting_approval' | 'completed' | 'failed' | 'stopped';

export type WorkflowPhase = { name: string; status: WorkflowStatus };

export type WorkflowStep = {
  id: string;
  phase: string;
  label: string;
  status: WorkflowStatus;
  agent?: string;
  detail?: string;
};

export type WorkflowReport = {
  id: string;
  item: string;
  artifact_id?: string;
  /** Phase marker in effect when the item was reported, when known. */
  phase?: string;
};

export type WorkflowArtifact = {
  id: string;
  title: string;
  kind: string;
  body: string;
  /** Phase marker in effect when the artifact was produced, when known. */
  phase?: string;
};

/** The full run snapshot (WorkflowRunDto) — the panel's resume/start state. */
export type WorkflowRun = {
  run_id: Id;
  name: string;
  status: WorkflowStatus;
  phases: WorkflowPhase[];
  steps: WorkflowStep[];
  reports: WorkflowReport[];
  artifacts: WorkflowArtifact[];
  tokens: number;
  cost_usd: number;
  max_cost_usd: number;
  error?: string;
};

/** The confirmation card's approved payload (WorkflowSpecDto). */
export type WorkflowSpec = {
  name: string;
  script: string;
  commands: string[];
  max_cost_usd: number;
};

/**
 * The `run_workflow` confirmation payload on an agent `approval_needed` card
 * (`ApprovalNeededDto.workflow` in Rust, `WorkflowApprovalDto` on the wire).
 * Same four facts as `WorkflowSpec`, carried on the tool card so the chat can
 * render the grant where the model requested it.
 */
export type WorkflowApproval = {
  name: string;
  script: string;
  commands: string[];
  max_cost_usd: number;
};

export type WorkflowStartedPayload = {
  run_id: Id;
  request_id: Id;
  workspace_id?: Id;
  name: string;
  max_cost_usd: number;
};

export type WorkflowPhasePayload = { run_id: Id; phase: WorkflowPhase };
export type WorkflowStepPayload = { run_id: Id; step: WorkflowStep };
export type WorkflowReportPayload = { run_id: Id; report: WorkflowReport };
export type WorkflowArtifactPayload = { run_id: Id; artifact: WorkflowArtifact };

export type WorkflowApprovalNeededPayload = {
  run_id: Id;
  request_id: Id;
  approval_id: Id;
  summary: string;
  path_or_command: string;
  diff_or_input?: string;
  can_approve_all_this_turn: boolean;
};

export type WorkflowFinishedPayload = {
  run_id: Id;
  request_id: Id;
  status: WorkflowStatus;
  tokens: number;
  cost_usd: number;
  max_cost_usd: number;
  error?: string;
};

export type FrontendCommand =
  | { type: 'get_bootstrap_state' }
  | { type: 'open_folder' }
  | { type: 'open_file'; payload: { path: string } }
  | { type: 'toggle_surface'; payload: { surface: SurfaceId } }
  | { type: 'resize_region'; payload: { region: RegionId; size_px: number } }
  | { type: 'save_layout' }
  | { type: 'settings_update'; payload: { patch: SettingsPatch } }
  | { type: 'send_chat'; payload: { request_id: Id; request: { session_id: Id; message: string; model_id: string | null } } }
  | { type: 'stop_chat'; payload: { request_id: Id } }
  | { type: 'send_agent'; payload: { request_id: Id; request: { session_id: Id; prompt: string; model_id: string | null; mode: AgentMode; reasoning_effort?: 'quick' | 'standard' | 'max'; system_instructions?: string; temperature?: number; editor_context?: EditorContextDto } } }
  | { type: 'agent_decision'; payload: { request_id: Id; approval_id: Id; decision: Decision } }
  | { type: 'retry_message'; payload: { session_id: Id; message_id: Id } }
  | { type: 'switch_session'; payload: { session_id: Id } }
  | { type: 'new_session' }
  | { type: 'open_folder_at'; payload: { root: string } }
  | { type: 'close_folder' }
  | { type: 'refresh_tree' }
  | { type: 'reveal_in_explorer'; payload: { path: string } }
  | { type: 'close_buffer'; payload: { buffer_id: Id } }
  | { type: 'save_buffer'; payload: { buffer_id: Id } }
  | { type: 'reload_buffer_from_disk'; payload: { buffer_id: Id } }
  | { type: 'keep_buffer_edits'; payload: { buffer_id: Id } }
  | { type: 'apply_editor_change'; payload: { buffer_id: Id; version: number; changes: TextChange[] } }
  | { type: 'set_selection'; payload: { buffer_id: Id; selection: Selection } }
  | { type: 'hover_at'; payload: { buffer_id: Id; position: TextPosition } }
  | { type: 'definition_at'; payload: { buffer_id: Id; position: TextPosition } }
  | { type: 'completion_at'; payload: { buffer_id: Id; position: TextPosition } }
  | { type: 'lsp_restart'; payload: { language: string } }
  | { type: 'search'; payload: { query: SearchQuery } }
  | { type: 'git_stage'; payload: { path: string } }
  | { type: 'git_unstage'; payload: { path: string } }
  | { type: 'git_commit'; payload: { message: string } }
  | { type: 'open_git_diff'; payload: { path: string } }
  | { type: 'workflow_run'; payload: { request_id: Id; spec: WorkflowSpec } }
  | { type: 'workflow_resume'; payload: { request_id: Id; run_id: Id; allow_incomplete_asks?: boolean } }
  | { type: 'workflow_stop'; payload: { request_id: Id } }
  | { type: 'workflow_decision'; payload: { request_id: Id; approval_id: Id; decision: Decision } }
  | { type: 'open_external'; payload: { url: string } };

export type RequestEnvelope = {
  protocol_version: number;
  workspace_id: Id | null;
  core_instance_id: Id | null;
  request_id: Id;
  session_id: Id | null;
  payload: FrontendCommand;
};

// The complete protocol event union (ARCHITECTURE §2.5: the bridge enumerates
// ALL variants; the Z2 reducer handles workspace_layout_changed, toast and
// error, and explicitly ignores the rest).
export type FrontendEvent =
  | { type: 'bootstrap_state'; payload: BootstrapState }
  | { type: 'chat_delta'; payload: { request_id: Id; session_id: Id; turn_id: Id; message_id: Id; sequence: number; text: string } }
  | { type: 'chat_completed'; payload: ChatCompletedPayload }
  | { type: 'chat_failed'; payload: { request_id: Id; session_id: Id; turn_id: Id; error: ProtocolError } }
  | { type: 'tool_call_started'; payload: ToolCallStartedPayload }
  | { type: 'tool_result'; payload: ToolResultPayload }
  | { type: 'approval_needed'; payload: ApprovalNeededPayload }
  | { type: 'agent_state_changed'; payload: AgentStateChangedPayload }
  | { type: 'agent_turn_done'; payload: AgentTurnDonePayload }
  | { type: 'agent_event'; payload: { request_id: Id; session_id: Id; turn_id: Id; approval: Approval | null; message: string } }
  | { type: 'diagnostics_changed'; payload: Diagnostic[] }
  | { type: 'lsp_server_state_changed'; payload: LspServerState }
  | { type: 'hover_result'; payload: HoverResult }
  | { type: 'completion_result'; payload: CompletionResult }
  | { type: 'definition_result'; payload: DefinitionResult }
  | { type: 'file_tree_changed'; payload: { root: FileTreeNode | null } }
  | { type: 'buffer_snapshot'; payload: BufferSnapshot }
  | { type: 'buffer_changed'; payload: { buffer_id: Id; version: number; changes: TextChange[] } }
  | { type: 'buffer_conflict'; payload: BufferConflict }
  | { type: 'buffer_conflict_cleared'; payload: BufferConflictCleared }
  | { type: 'search_results'; payload: { request_id: Id | null; query: SearchQuery; hits: SearchHit[] } }
  | {
      type: 'git_status_changed';
      payload: {
        branch: string | null;
        changed_paths: string[];
        staged_paths: string[];
        files?: { path: string; staged: boolean; worktree: boolean }[];
      };
    }
  | { type: 'git_diff_ready'; payload: { path: string; unified_diff: string } }
  | { type: 'models_changed'; payload: { models: Model[]; active_model_id: string | null } }
  | { type: 'selora_status_changed'; payload: { status: ConnectionStatus; active_model_id: string | null } }
  | { type: 'workspace_layout_changed'; payload: Layout }
  | { type: 'workflow_started'; payload: WorkflowStartedPayload }
  | { type: 'workflow_phase'; payload: WorkflowPhasePayload }
  | { type: 'workflow_step'; payload: WorkflowStepPayload }
  | { type: 'workflow_report'; payload: WorkflowReportPayload }
  | { type: 'workflow_artifact'; payload: WorkflowArtifactPayload }
  | { type: 'workflow_approval_needed'; payload: WorkflowApprovalNeededPayload }
  | { type: 'workflow_finished'; payload: WorkflowFinishedPayload }
  | { type: 'toast'; payload: { severity: ToastSeverity; message: string } }
  | { type: 'error'; payload: ProtocolError };

/** The seven workflow progress events, narrowed out of `FrontendEvent`. */
export type WorkflowEvent =
  | Extract<FrontendEvent, { type: 'workflow_started' }>
  | Extract<FrontendEvent, { type: 'workflow_phase' }>
  | Extract<FrontendEvent, { type: 'workflow_step' }>
  | Extract<FrontendEvent, { type: 'workflow_report' }>
  | Extract<FrontendEvent, { type: 'workflow_artifact' }>
  | Extract<FrontendEvent, { type: 'workflow_approval_needed' }>
  | Extract<FrontendEvent, { type: 'workflow_finished' }>;

export const isWorkflowEvent = (event: FrontendEvent): event is WorkflowEvent =>
  event.type.startsWith('workflow_');

/** Every workflow event routes by `run_id` (the panel key). */
export const workflowEventRunId = (event: WorkflowEvent): Id => event.payload.run_id;

export type EventEnvelope = {
  protocol_version: number;
  workspace_id: Id | null;
  core_instance_id: Id;
  sequence: number;
  request_id: Id | null;
  session_id: Id | null;
  payload: FrontendEvent;
};
export type EventHandlerResult = 'applied' | 'stale' | 'gap' | 'epoch-mismatch' | 'version-mismatch';

// --- Command error path (ARCHITECTURE §2.3 item 4, §4.4) --------------------
//
// A dispatch can fail two ways: the Rust handler returns a ProtocolErrorDto
// (invoke rejects with that object) or the serde boundary rejects the payload
// (invoke rejects with Tauri's own error string). Both are normalized here and
// published to one observable stream so the toast surface can show them; the
// promise still rejects so callers can skip follow-up commands.

export type BridgeCommandError = { code: string; message: string; retryable: boolean };

type CommandErrorListener = (error: BridgeCommandError) => void;
const commandErrorListeners = new Set<CommandErrorListener>();

export function onBridgeCommandError(listener: CommandErrorListener): () => void {
  commandErrorListeners.add(listener);
  return () => {
    commandErrorListeners.delete(listener);
  };
}

function reportCommandError(cause: unknown): void {
  const error = normalizeCommandError(cause);
  for (const listener of commandErrorListeners) listener(error);
}

function normalizeCommandError(cause: unknown): BridgeCommandError {
  if (typeof cause === 'object' && cause !== null) {
    const candidate = cause as { code?: unknown; message?: unknown; retryable?: unknown };
    if (typeof candidate.code === 'string' && typeof candidate.message === 'string') {
      return {
        code: candidate.code,
        message: candidate.message,
        retryable: typeof candidate.retryable === 'boolean' ? candidate.retryable : false,
      };
    }
  }
  const message = cause instanceof Error ? cause.message : String(cause);
  return { code: 'command_failed', message, retryable: false };
}

// --- Envelope assembly (ARCHITECTURE §2.1) ----------------------------------

export type EnvelopeContext = {
  core_instance_id: Id;
  workspace_id: Id | null;
  session_id: Id | null;
};

export function newRequestId(): string {
  return crypto.randomUUID();
}

function envelopeFor(payload: FrontendCommand, ctx: EnvelopeContext): RequestEnvelope {
  return {
    protocol_version: PROTOCOL_VERSION,
    workspace_id: ctx.workspace_id,
    core_instance_id: ctx.core_instance_id,
    request_id: newRequestId(),
    session_id: ctx.session_id,
    payload,
  };
}

export const getBootstrapState = (): Promise<BootstrapState> =>
  invoke('get_bootstrap_state', { protocolVersion: PROTOCOL_VERSION });

export const dispatchCommand = <T>(envelope: RequestEnvelope): Promise<T> =>
  invoke<T>('dispatch_command', { envelope });

const dispatchZ2Command = (payload: FrontendCommand, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchCommand<unknown>(envelopeFor(payload, ctx)).catch((cause: unknown) => {
    reportCommandError(cause);
    throw cause;
  });

export const toggleSurface = (surface: SurfaceId, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'toggle_surface', payload: { surface } }, ctx);

export const resizeRegion = (region: RegionId, sizePx: number, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'resize_region', payload: { region, size_px: sizePx } }, ctx);

export const saveLayout = (ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'save_layout' }, ctx);

export const updateSettings = (patch: SettingsPatch, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'settings_update', payload: { patch } }, ctx);

// --- Z4 workspace / editor commands (ARCHITECTURE §7) -----------------------
// Every helper rides dispatch_command; results arrive as replayable events
// (BufferSnapshot, FileTreeChanged, SearchResults, GitStatusChanged, …), so the
// return value is intentionally opaque — the store applies events, not replies.

/** Native folder picker (a plain Tauri command, not a protocol command). */
export const pickFolder = (): Promise<string | null> => invoke<string | null>('pick_folder');

export const openFolderAt = (root: string, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'open_folder_at', payload: { root } }, ctx);

/** P1: close the open folder (stops every language server). */
export const closeFolder = (ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'close_folder' }, ctx);

/** P1: restart one language server; status follows as an event. */
export const restartLsp = (language: string, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'lsp_restart', payload: { language } }, ctx);

export const openExternal = (url: string, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'open_external', payload: { url } }, ctx);

export const refreshTree = (ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'refresh_tree' }, ctx);

export const openFile = (path: string, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'open_file', payload: { path } }, ctx);

export const closeBuffer = (bufferId: Id, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'close_buffer', payload: { buffer_id: bufferId } }, ctx);

export const saveBuffer = (bufferId: Id, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'save_buffer', payload: { buffer_id: bufferId } }, ctx);

/** Select one workspace-relative file in the OS file manager. */
export const revealInExplorer = (path: string, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'reveal_in_explorer', payload: { path } }, ctx);

/** P2 gap 3: discard the local edits and take the disk text. */
export const reloadBufferFromDisk = (bufferId: Id, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'reload_buffer_from_disk', payload: { buffer_id: bufferId } }, ctx);

/** P2 gap 3: keep the local edits and clear the external-change marker. */
export const keepBufferEdits = (bufferId: Id, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'keep_buffer_edits', payload: { buffer_id: bufferId } }, ctx);

export const applyEditorChange = (
  bufferId: Id,
  version: number,
  changes: TextChange[],
  ctx: EnvelopeContext,
): Promise<unknown> =>
  dispatchZ2Command(
    { type: 'apply_editor_change', payload: { buffer_id: bufferId, version, changes } },
    ctx,
  );

export const setSelection = (
  bufferId: Id,
  selection: Selection,
  ctx: EnvelopeContext,
): Promise<unknown> =>
  dispatchZ2Command({ type: 'set_selection', payload: { buffer_id: bufferId, selection } }, ctx);
export const hoverAt = (
  bufferId: Id,
  position: TextPosition,
  ctx: EnvelopeContext,
): Promise<unknown> =>
  dispatchZ2Command({ type: 'hover_at', payload: { buffer_id: bufferId, position } }, ctx);

export const definitionAt = (
  bufferId: Id,
  position: TextPosition,
  ctx: EnvelopeContext,
): Promise<unknown> =>
  dispatchZ2Command({ type: 'definition_at', payload: { buffer_id: bufferId, position } }, ctx);

export const completionAt = (
  bufferId: Id,
  position: TextPosition,
  ctx: EnvelopeContext,
): Promise<unknown> =>
  dispatchZ2Command({ type: 'completion_at', payload: { buffer_id: bufferId, position } }, ctx);

export const searchFiles = (query: SearchQuery, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'search', payload: { query } }, ctx);

export const gitStage = (path: string, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'git_stage', payload: { path } }, ctx);

export const gitUnstage = (path: string, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'git_unstage', payload: { path } }, ctx);

export const gitCommit = (message: string, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'git_commit', payload: { message } }, ctx);

export const openGitDiff = (path: string, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'open_git_diff', payload: { path } }, ctx);

export const sendChat = (
  message: string,
  modelId: string | null,
  ctx: EnvelopeContext,
): { requestId: string; promise: Promise<unknown> } => {
  const requestId = newRequestId();
  const sessionId = ctx.session_id ?? 'sess_default';
  return {
    requestId,
    promise: dispatchZ2Command(
      {
        type: 'send_chat',
        payload: {
          request_id: requestId,
          request: {
            session_id: sessionId,
            message,
            model_id: modelId,
          },
        },
      },
      ctx,
    ),
  };
};

export const stopChat = (requestId: Id, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'stop_chat', payload: { request_id: requestId } }, ctx);

export const sendAgent = (
  prompt: string,
  mode: AgentMode,
  modelId: string | null,
  ctx: EnvelopeContext,
  options?: {
    reasoningEffort?: 'quick' | 'standard' | 'max';
    systemInstructions?: string;
    temperature?: number;
    editorContext?: EditorContextDto;
  },
): { requestId: string; promise: Promise<unknown> } => {
  const requestId = newRequestId();
  const sessionId = ctx.session_id ?? 'sess_default';
  const request: {
    session_id: Id;
    prompt: string;
    model_id: string | null;
    mode: AgentMode;
    reasoning_effort?: 'quick' | 'standard' | 'max';
    system_instructions?: string;
    temperature?: number;
    editor_context?: EditorContextDto;
  } = {
    session_id: sessionId,
    prompt,
    model_id: modelId,
    mode,
  };
  if (options?.reasoningEffort !== undefined) {
    request.reasoning_effort = options.reasoningEffort;
  }
  if (options?.systemInstructions !== undefined && options.systemInstructions.trim() !== '') {
    request.system_instructions = options.systemInstructions.trim();
  }
  if (options?.temperature !== undefined) {
    request.temperature = options.temperature;
  }
  if (options?.editorContext !== undefined && options.editorContext.path.trim() !== '') {
    request.editor_context = options.editorContext;
  }
  return {
    requestId,
    promise: dispatchZ2Command(
      {
        type: 'send_agent',
        payload: {
          request_id: requestId,
          request,
        },
      },
      ctx,
    ),
  };
};

export const agentDecision = (
  requestId: Id,
  approvalId: Id,
  decision: Decision,
  ctx: EnvelopeContext,
): Promise<unknown> =>
  dispatchZ2Command(
    {
      type: 'agent_decision',
      payload: {
        request_id: requestId,
        approval_id: approvalId,
        decision,
      },
    },
    ctx,
  );

export const switchSession = (sessionId: Id, ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'switch_session', payload: { session_id: sessionId } }, ctx);

export const newSession = (ctx: EnvelopeContext): Promise<unknown> =>
  dispatchZ2Command({ type: 'new_session' }, ctx);

// --- Phase 8 workflow commands (ARCHITECTURE §18.4) -------------------------

export const runWorkflow = (
  spec: WorkflowSpec,
  ctx: EnvelopeContext,
): { requestId: string; promise: Promise<unknown> } => {
  const requestId = newRequestId();
  return {
    requestId,
    promise: dispatchZ2Command(
      { type: 'workflow_run', payload: { request_id: requestId, spec } },
      ctx,
    ),
  };
};

/**
 * Resume a journaled run. `allowIncompleteAsks` is the user's explicit
 * confirmation that unfinished asks may re-dispatch tools whose earlier
 * effects already landed; omitting it keeps the backend's fail-closed default.
 */
export const resumeWorkflow = (
  runId: Id,
  ctx: EnvelopeContext,
  allowIncompleteAsks = false,
): { requestId: string; promise: Promise<unknown> } => {
  const requestId = newRequestId();
  return {
    requestId,
    promise: dispatchZ2Command(
      {
        type: 'workflow_resume',
        payload: {
          request_id: requestId,
          run_id: runId,
          allow_incomplete_asks: allowIncompleteAsks,
        },
      },
      ctx,
    ),
  };
};

export const stopWorkflow = (
  requestId: Id,
  ctx: EnvelopeContext,
): { requestId: string; promise: Promise<unknown> } => ({
  requestId,
  promise: dispatchZ2Command(
    { type: 'workflow_stop', payload: { request_id: requestId } },
    ctx,
  ),
});

/** Answer a `workflow_approval_needed` card (single-use, like `agentDecision`). */
export const workflowDecision = (
  requestId: Id,
  approvalId: Id,
  decision: Decision,
  ctx: EnvelopeContext,
): Promise<unknown> =>
  dispatchZ2Command(
    {
      type: 'workflow_decision',
      payload: {
        request_id: requestId,
        approval_id: approvalId,
        decision,
      },
    },
    ctx,
  );

export const getEventsAfter = (afterSequence: number): Promise<EventEnvelope[]> =>
  invoke('get_events_after', { afterSequence });

export const connectApiKey = (apiKey: string, baseUrl?: string): Promise<ConnectResult> =>
  invoke<ConnectResult>('connect_api_key', { apiKey, baseUrl: baseUrl || null });

export const disconnectApiKey = (): Promise<void> =>
  invoke<void>('disconnect_api_key');

export const getApiKeyStatus = (): Promise<ApiKeyStatus> =>
  invoke<ApiKeyStatus>('get_api_key_status');

export const fetchModels = (): Promise<Model[]> =>
  invoke<Model[]>('fetch_models');

// --- Event validation (unchanged from Z1; ARCHITECTURE §3.4) -----------------

export const validateEvent = (
  event: EventEnvelope,
  bootstrap: BootstrapState,
  lastSequence: number | null,
): EventHandlerResult => {
  if (event.protocol_version !== PROTOCOL_VERSION) return 'version-mismatch';
  if (event.core_instance_id !== bootstrap.core_instance_id) return 'epoch-mismatch';
  if (lastSequence !== null && event.sequence <= lastSequence) return 'stale';
  if (lastSequence !== null && event.sequence !== lastSequence + 1) return 'gap';
  return 'applied';
};

export const listenProtocolEvent = (handler: (event: EventEnvelope) => void): Promise<UnlistenFn> =>
  listen<EventEnvelope>('selora://event', (event) => handler(event.payload));
