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

export type SurfaceId = 'files' | 'search' | 'git' | 'problems' | 'settings' | 'agent' | 'output' | 'terminal_placeholder';
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

// --- DTOs referenced by the full FrontendEvent union ------------------------

export type BufferSnapshot = {
  buffer_id: Id; path: string; language: string; text: string; version: number;
  dirty: boolean; selection: Selection | null;
};
export type TextChange = { start: TextPosition; end: TextPosition; replacement: string };
export type Model = { id: string; display_name: string; available: boolean };
export type SearchQuery = { query: string; case_sensitive: boolean; whole_word: boolean; use_regex: boolean };
export type SearchHit = { path: string; line: number; column: number; preview: string };
export type ChatSession = { session_id: Id; title: string; message_ids: Id[]; active: boolean };
export type ChatMessage = { message_id: Id; session_id: Id; role: 'user' | 'assistant' | 'tool' | 'system'; text: string };

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
  git: { branch: string | null; changed_paths: string[]; staged_paths: string[] };
  diagnostics: Diagnostic[];
  feature_flags: { agent: boolean; git: boolean; search: boolean; editor: boolean; later_slices: string[] };
};

export type ProtocolError = { code: string; message: string; retryable: boolean };

export type FrontendCommand =
  | { type: 'get_bootstrap_state' }
  | { type: 'open_folder' }
  | { type: 'open_file'; payload: { path: string } }
  | { type: 'toggle_surface'; payload: { surface: SurfaceId } }
  | { type: 'resize_region'; payload: { region: RegionId; size_px: number } }
  | { type: 'save_layout' };

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
  | { type: 'chat_completed'; payload: { request_id: Id; session_id: Id; turn_id: Id; message_id: Id; text: string } }
  | { type: 'chat_failed'; payload: { request_id: Id; session_id: Id; turn_id: Id; error: ProtocolError } }
  | { type: 'agent_event'; payload: { request_id: Id; session_id: Id; turn_id: Id; approval: Approval | null; message: string } }
  | { type: 'diagnostics_changed'; payload: Diagnostic[] }
  | { type: 'file_tree_changed'; payload: { root: FileTreeNode | null } }
  | { type: 'buffer_snapshot'; payload: BufferSnapshot }
  | { type: 'buffer_changed'; payload: { buffer_id: Id; version: number; changes: TextChange[] } }
  | { type: 'search_results'; payload: { request_id: Id | null; query: SearchQuery; hits: SearchHit[] } }
  | { type: 'git_status_changed'; payload: { branch: string | null; changed_paths: string[]; staged_paths: string[] } }
  | { type: 'git_diff_ready'; payload: { path: string; unified_diff: string } }
  | { type: 'models_changed'; payload: { models: Model[]; active_model_id: string | null } }
  | { type: 'selora_status_changed'; payload: { status: ConnectionStatus; active_model_id: string | null } }
  | { type: 'workspace_layout_changed'; payload: Layout }
  | { type: 'toast'; payload: { severity: ToastSeverity; message: string } }
  | { type: 'error'; payload: ProtocolError };

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

export const getEventsAfter = (afterSequence: number): Promise<EventEnvelope[]> =>
  invoke('get_events_after', { afterSequence });

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
