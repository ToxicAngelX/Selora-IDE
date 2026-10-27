// Shared vitest fixtures — a BootstrapState shaped exactly like the Z2
// protocol wire (§1.1 defaults: sidebar 240 open, agent 360 closed, bottom
// 200 closed) plus matching layout_constraints.
import type { BootstrapState, EventEnvelope, FrontendEvent, Layout, LayoutConstraints } from '../bridge/bridge';

export const FIXTURE_CONSTRAINTS: LayoutConstraints = {
  primary_sidebar_min_px: 160,
  primary_sidebar_max_px: 480,
  agent_dock_min_px: 240,
  agent_dock_max_px: 600,
  bottom_dock_min_px: 120,
  bottom_dock_max_px: 800,
  bottom_dock_max_workbench_ratio: 0.5,
  editor_min_width_px: 400,
  resize_hit_zone_min_px: 6,
};

export const FIXTURE_LAYOUT: Layout = {
  primary_sidebar_width_px: 240,
  agent_dock_width_px: 360,
  agent_dock_open: false,
  bottom_dock_height_px: 200,
  bottom_dock_open: false,
  bottom_view: 'output',
  editor_splits: 1,
  activity_view: 'files',
  primary_sidebar_open: true,
};

export function makeLayout(overrides: Partial<Layout> = {}): Layout {
  return { ...FIXTURE_LAYOUT, ...overrides };
}

export function makeBootstrap(overrides: Partial<BootstrapState> = {}): BootstrapState {
  return {
    protocol_version: 1,
    core_instance_id: 'core-0001',
    snapshot_sequence: 0,
    workspace: { id: 'no-workspace', core_instance_id: 'core-0001', root: null, display_name: 'No workspace open' },
    buffers: [],
    active_buffer_id: null,
    file_tree: { root: null },
    models: [],
    active_model_id: null,
    connection: 'disconnected',
    chat: {
      sessions: [],
      messages: [],
      active_session_id: null,
      active_request_id: null,
      active_turn_id: null,
      pending_approval: null,
    },
    settings: {
      reduced_motion: false,
      theme: 'dark',
      keybind_command_palette: 'Ctrl+Shift+P',
      keybind_toggle_chat: 'Ctrl+L',
    },
    layout: FIXTURE_LAYOUT,
    layout_constraints: FIXTURE_CONSTRAINTS,
    git: { branch: null, changed_paths: [], staged_paths: [] },
    diagnostics: [],
    feature_flags: { agent: false, git: false, search: false, editor: false, later_slices: [] },
    ...overrides,
  };
}

export function makeEvent(
  sequence: number,
  payload: FrontendEvent,
  coreInstanceId = 'core-0001',
): EventEnvelope {
  return {
    protocol_version: 1,
    workspace_id: null,
    core_instance_id: coreInstanceId,
    sequence,
    request_id: null,
    session_id: null,
    payload,
  };
}

export const BOOT_TOAST_PAYLOAD: Extract<FrontendEvent, { type: 'toast' }> = {
  type: 'toast',
  payload: { severity: 'error', message: 'Settings were reset after a load failure.' },
};
