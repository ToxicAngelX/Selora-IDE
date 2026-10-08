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
export type BootstrapState = { protocol_version: number; core_instance_id: Id; snapshot_sequence: number; workspace: { id: Id; core_instance_id: Id; root: string | null; display_name: string }; buffers: Array<{ buffer_id: Id; path: string; language: string; text: string; version: number; dirty: boolean; selection: Selection | null }>; active_buffer_id: Id | null; file_tree: { root: FileTreeNode | null }; models: Array<{ id: string; display_name: string; available: boolean }>; active_model_id: string | null; connection: ConnectionStatus; chat: { sessions: Array<{ session_id: Id; title: string; message_ids: Id[]; active: boolean }>; messages: Array<{ message_id: Id; session_id: Id; role: 'user' | 'assistant' | 'tool' | 'system'; text: string }>; active_session_id: Id | null; active_request_id: Id | null; active_turn_id: Id | null; pending_approval: Approval | null }; settings: { reduced_motion: boolean; theme: string }; layout: { primary_sidebar_width_px: number; agent_dock_width_px: number; agent_dock_open: boolean; bottom_dock_height_px: number; bottom_dock_open: boolean; bottom_view: string; editor_splits: number }; git: { branch: string | null; changed_paths: string[]; staged_paths: string[] }; diagnostics: Diagnostic[]; feature_flags: { agent: boolean; git: boolean; search: boolean; editor: boolean; later_slices: string[] } };
export type ProtocolError = { code: string; message: string; retryable: boolean };
export type FrontendCommand = { type: 'get_bootstrap_state' } | { type: 'open_folder' } | { type: 'open_file'; payload: { path: string } };
export type RequestEnvelope = { protocol_version: number; workspace_id: Id | null; core_instance_id: Id | null; request_id: Id; session_id: Id | null; payload: FrontendCommand };
export type FrontendEvent = { type: 'toast'; payload: { severity: string; message: string } } | { type: 'error'; payload: ProtocolError };
export type EventEnvelope = { protocol_version: number; workspace_id: Id | null; core_instance_id: Id; sequence: number; request_id: Id | null; session_id: Id | null; payload: FrontendEvent };
export type EventHandlerResult = 'applied' | 'stale' | 'gap' | 'epoch-mismatch' | 'version-mismatch';

export const getBootstrapState = (): Promise<BootstrapState> => invoke('get_bootstrap_state', { protocolVersion: PROTOCOL_VERSION });
export const dispatchCommand = <T>(envelope: RequestEnvelope): Promise<T> => invoke<T>('dispatch_command', { envelope });
export const validateEvent = (event: EventEnvelope, bootstrap: BootstrapState, lastSequence: number | null): EventHandlerResult => {
  if (event.protocol_version !== PROTOCOL_VERSION) return 'version-mismatch';
  if (event.core_instance_id !== bootstrap.core_instance_id) return 'epoch-mismatch';
  if (lastSequence !== null && event.sequence <= lastSequence) return 'stale';
  if (lastSequence !== null && event.sequence !== lastSequence + 1) return 'gap';
  return 'applied';
};
export const listenProtocolEvent = (handler: (event: EventEnvelope) => void): Promise<UnlistenFn> => listen<EventEnvelope>('selora://event', event => handler(event.payload));
