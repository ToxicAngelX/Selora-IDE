// Bridge wire-shape and gate tests (ARCHITECTURE §2.6/§10): the exact §2.1
// command/event JSON, the envelope transport rules, the command-error path,
// and the stale/gap/epoch/version validation.
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { invoke } from '@tauri-apps/api/core';
import {
  agentDecision,
  connectApiKey,
  deleteSession,
  disconnectApiKey,
  dispatchCommand,
  fetchModels,
  getApiKeyStatus,
  getBootstrapState,
  getEventsAfter,
  newSession,
  onBridgeCommandError,
  PROTOCOL_VERSION,
  resizeRegion,
  saveLayout,
  sendAgent,
  sendChat,
  stopChat,
  switchSession,
  terminalClose,
  terminalInput,
  terminalResize,
  createTerminal,
  toggleSurface,
  validateEvent,
  type BootstrapState,
  type EventEnvelope,
} from './bridge';
import { BOOT_TOAST_PAYLOAD, makeBootstrap, makeEvent } from '../testing/fixtures';

const invokeMock = vi.mocked(invoke);

const CONTEXT = { core_instance_id: 'core-0001', workspace_id: 'no-workspace', session_id: null };

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe('bootstrap contract', () => {
  it('keeps the reconnect snapshot fields explicit', () => {
    const state: Pick<BootstrapState, 'protocol_version' | 'core_instance_id' | 'snapshot_sequence'> = {
      protocol_version: 1,
      core_instance_id: 'core-1',
      snapshot_sequence: 4,
    };
    expect(state.snapshot_sequence).toBe(4);
  });
});

describe('Z2 command wire shapes (ARCHITECTURE §2.1)', () => {
  it('toggle_surface serializes to the exact contract JSON', async () => {
    await toggleSurface('agent', CONTEXT);
    expect(invokeMock).toHaveBeenCalledTimes(1);
    const [command, args] = invokeMock.mock.calls[0];
    expect(command).toBe('dispatch_command');
    expect(JSON.stringify((args as { envelope: { payload: unknown } }).envelope.payload)).toBe(
      '{"type":"toggle_surface","payload":{"surface":"agent"}}',
    );
  });

  it('resize_region serializes to the exact contract JSON', async () => {
    await resizeRegion('primary_sidebar', 240, CONTEXT);
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(JSON.stringify(args.envelope.payload)).toBe(
      '{"type":"resize_region","payload":{"region":"primary_sidebar","size_px":240}}',
    );
  });

  it('save_layout is a unit variant — no payload key', async () => {
    await saveLayout(CONTEXT);
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(JSON.stringify(args.envelope.payload)).toBe('{"type":"save_layout"}');
  });

  it('builds the RequestEnvelope transport per §2.1', async () => {
    await toggleSurface('agent', CONTEXT);
    await toggleSurface('agent', CONTEXT);
    const first = (invokeMock.mock.calls[0][1] as { envelope: Record<string, unknown> }).envelope;
    const second = (invokeMock.mock.calls[1][1] as { envelope: Record<string, unknown> }).envelope;
    expect(first.protocol_version).toBe(PROTOCOL_VERSION);
    expect(first.core_instance_id).toBe('core-0001');
    expect(first.workspace_id).toBe('no-workspace');
    expect(first.session_id).toBeNull();
    // A fresh request_id per dispatch — distinct and UUID-shaped.
    expect(first.request_id).not.toBe(second.request_id);
    expect(String(first.request_id)).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
  });

  it('getEventsAfter rides the get_events_after command with the camelCase key', async () => {
    await getEventsAfter(7);
    expect(invokeMock).toHaveBeenCalledWith('get_events_after', { afterSequence: 7 });
  });

  it('getBootstrapState sends the protocol version as camelCase', async () => {
    await getBootstrapState();
    expect(invokeMock).toHaveBeenCalledWith('get_bootstrap_state', { protocolVersion: 1 });
  });
});

describe('Z3 command wire shapes (ARCHITECTURE §2.1 & selora-protocol)', () => {
  it('send_agent serializes to the exact contract JSON', async () => {
    const { requestId, promise } = sendAgent('fix it', 'execute', null, {
      ...CONTEXT,
      session_id: 'session-1',
    });
    await promise;
    expect(invokeMock).toHaveBeenCalledTimes(1);
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(args.envelope.payload).toEqual({
      type: 'send_agent',
      payload: {
        request_id: requestId,
        request: {
          session_id: 'session-1',
          prompt: 'fix it',
          model_id: null,
          mode: 'execute',
        },
      },
    });
  });

  it('send_agent includes reasoning_effort, system_instructions, and temperature when configured', async () => {
    const { requestId, promise } = sendAgent(
      'fix it with deep reasoning',
      'execute',
      'deepseek-reasoner',
      {
        ...CONTEXT,
        session_id: 'session-1',
      },
      {
        reasoningEffort: 'max',
        systemInstructions: 'Verify types',
        temperature: 0.2,
      },
    );
    await promise;
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(args.envelope.payload).toEqual({
      type: 'send_agent',
      payload: {
        request_id: requestId,
        request: {
          session_id: 'session-1',
          prompt: 'fix it with deep reasoning',
          model_id: 'deepseek-reasoner',
          mode: 'execute',
          reasoning_effort: 'max',
          system_instructions: 'Verify types',
          temperature: 0.2,
        },
      },
    });
  });

  it('send_agent includes editor_context when supplied', async () => {
    const { requestId, promise } = sendAgent(
      'explain it',
      'execute',
      'deepseek-reasoner',
      { ...CONTEXT, session_id: 'session-1' },
      {
        editorContext: {
          path: 'crates/selora-ai/src/agent/turn.rs',
          language: 'rust',
          selection: {
            anchor: { line: 6, utf16_column: 0 },
            active: { line: 6, utf16_column: 13 },
            version: 3,
          },
          dirty: true,
        },
      },
    );
    await promise;
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(args.envelope.payload).toEqual({
      type: 'send_agent',
      payload: {
        request_id: requestId,
        request: {
          session_id: 'session-1',
          prompt: 'explain it',
          model_id: 'deepseek-reasoner',
          mode: 'execute',
          editor_context: {
            path: 'crates/selora-ai/src/agent/turn.rs',
            language: 'rust',
            selection: {
              anchor: { line: 6, utf16_column: 0 },
              active: { line: 6, utf16_column: 13 },
              version: 3,
            },
            dirty: true,
          },
        },
      },
    });
  });

  it('send_agent omits editor_context when not supplied', async () => {
    const { requestId, promise } = sendAgent('explain it', 'execute', 'deepseek-reasoner', {
      ...CONTEXT,
      session_id: 'session-1',
    });
    await promise;
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(args.envelope.payload).toEqual({
      type: 'send_agent',
      payload: {
        request_id: requestId,
        request: {
          session_id: 'session-1',
          prompt: 'explain it',
          model_id: 'deepseek-reasoner',
          mode: 'execute',
        },
      },
    });
    const request = (args.envelope.payload as { payload: { request: object } }).payload.request;
    expect('editor_context' in request).toBe(false);
  });

  it('fetchModels calls fetch_models Tauri invoke', async () => {
    invokeMock.mockResolvedValueOnce([{ id: 'model-1', display_name: 'Model 1', available: true }]);
    const models = await fetchModels();
    expect(invokeMock).toHaveBeenCalledWith('fetch_models');
    expect(models).toEqual([{ id: 'model-1', display_name: 'Model 1', available: true }]);
  });

  it('send_chat serializes to the exact contract JSON', async () => {
    const { requestId, promise } = sendChat('hello', 'glm-5', {
      ...CONTEXT,
      session_id: 'session-1',
    });
    await promise;
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(args.envelope.payload).toEqual({
      type: 'send_chat',
      payload: {
        request_id: requestId,
        request: {
          session_id: 'session-1',
          message: 'hello',
          model_id: 'glm-5',
        },
      },
    });
  });

  it('stop_chat serializes to the exact contract JSON', async () => {
    await stopChat('request-1', CONTEXT);
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(args.envelope.payload).toEqual({
      type: 'stop_chat',
      payload: {
        request_id: 'request-1',
      },
    });
  });

  it('agent_decision serializes to the exact contract JSON', async () => {
    await agentDecision('request-1', 'approval-1', 'approve_all_this_turn', CONTEXT);
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(args.envelope.payload).toEqual({
      type: 'agent_decision',
      payload: {
        request_id: 'request-1',
        approval_id: 'approval-1',
        decision: 'approve_all_this_turn',
      },
    });
  });

  it('agent_decision carries the additive Phase 9a resolution only when supplied', async () => {
    await agentDecision('request-1', 'approval-1', 'approve', CONTEXT, {
      kind: 'question',
      option_id: 'opt-main',
      text: 'keep the header',
    });
    await agentDecision('request-2', 'approval-2', 'deny', CONTEXT, {
      kind: 'plan',
      decision: 'request_changes',
      feedback: 'Split the step',
    });
    const first = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    const second = invokeMock.mock.calls[1][1] as { envelope: { payload: unknown } };
    expect(first.envelope.payload).toEqual({
      type: 'agent_decision',
      payload: {
        request_id: 'request-1',
        approval_id: 'approval-1',
        decision: 'approve',
        resolution: { kind: 'question', option_id: 'opt-main', text: 'keep the header' },
      },
    });
    expect(second.envelope.payload).toEqual({
      type: 'agent_decision',
      payload: {
        request_id: 'request-2',
        approval_id: 'approval-2',
        decision: 'deny',
        resolution: { kind: 'plan', decision: 'request_changes', feedback: 'Split the step' },
      },
    });
  });
});

describe('Z3 session delete wire shape', () => {
  it('delete_session serializes to the exact contract JSON', async () => {
    await deleteSession('sess-disk', CONTEXT);
    const args = invokeMock.mock.calls[0][1] as { envelope: { payload: unknown } };
    expect(args.envelope.payload).toEqual({
      type: 'delete_session',
      payload: { session_id: 'sess-disk' },
    });
  });
});

describe('Z2 event wire shapes (ARCHITECTURE §2.1)', () => {
  it('a Layout payload serializes to the exact contract field names', () => {
    const layout = {
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
    expect(JSON.stringify(layout)).toBe(
      '{"primary_sidebar_width_px":240,"agent_dock_width_px":360,"agent_dock_open":false,"bottom_dock_height_px":200,"bottom_dock_open":false,"bottom_view":"output","editor_splits":1,"activity_view":"files","primary_sidebar_open":true}',
    );
  });

  it('a toast payload serializes to the contract shape', () => {
    const toast = { severity: 'info', message: 'Saved.' };
    expect(JSON.stringify(toast)).toBe('{"severity":"info","message":"Saved."}');
  });
});

describe('command-error path (ARCHITECTURE §2.3 item 4, §4.4)', () => {
  it('publishes a ProtocolErrorDto rejection to the error stream and rejects', async () => {
    const seen: Array<{ code: string; message: string; retryable: boolean }> = [];
    const stop = onBridgeCommandError((error) => seen.push(error));
    invokeMock.mockRejectedValueOnce({ code: 'layout_save_failed', message: 'disk full', retryable: true });
    await expect(saveLayout(CONTEXT)).rejects.toMatchObject({ code: 'layout_save_failed' });
    expect(seen).toEqual([{ code: 'layout_save_failed', message: 'disk full', retryable: true }]);
    stop();
  });

  it('normalizes a serde-boundary string rejection', async () => {
    const seen: Array<{ code: string; message: string; retryable: boolean }> = [];
    const stop = onBridgeCommandError((error) => seen.push(error));
    invokeMock.mockRejectedValueOnce('invalid args `payload` for command `dispatch_command`');
    await expect(toggleSurface('files', CONTEXT)).rejects.toBeTruthy();
    expect(seen).toEqual([
      { code: 'command_failed', message: 'invalid args `payload` for command `dispatch_command`', retryable: false },
    ]);
    stop();
  });

  it('a successful dispatch publishes nothing', async () => {
    const seen: unknown[] = [];
    const stop = onBridgeCommandError((error) => seen.push(error));
    await dispatchCommand({
      protocol_version: 1,
      workspace_id: 'no-workspace',
      core_instance_id: 'core-0001',
      request_id: '00000000-0000-0000-0000-000000000000',
      session_id: null,
      payload: { type: 'save_layout' },
    });
    expect(seen).toEqual([]);
    stop();
  });
});

describe('event validation gate (ARCHITECTURE §3.4)', () => {
  const bootstrap = makeBootstrap();

  it('drops a stale sequence (duplicate from the replay/live overlap)', () => {
    const event = makeEvent(4, BOOT_TOAST_PAYLOAD);
    expect(validateEvent(event, bootstrap, 4)).toBe('stale');
  });

  it('detects a sequence gap', () => {
    const event = makeEvent(6, BOOT_TOAST_PAYLOAD);
    expect(validateEvent(event, bootstrap, 4)).toBe('gap');
  });

  it('rejects a foreign core epoch', () => {
    const event = makeEvent(5, BOOT_TOAST_PAYLOAD, 'core-9999');
    expect(validateEvent(event, bootstrap, 4)).toBe('epoch-mismatch');
  });

  it('rejects a protocol version mismatch', () => {
    const event = { ...makeEvent(5, BOOT_TOAST_PAYLOAD), protocol_version: 2 } as EventEnvelope;
    expect(validateEvent(event, bootstrap, 4)).toBe('version-mismatch');
  });

  it('the null seed applies pre-bootstrap events (the boot toast case)', () => {
    // An event with sequence ≤ snapshot_sequence applies when the gate is
    // seeded null — the §3.4 rule that keeps the boot toast alive.
    const snapshot = makeBootstrap({ snapshot_sequence: 3 });
    const bootToast = makeEvent(1, BOOT_TOAST_PAYLOAD);
    expect(bootToast.sequence).toBeLessThanOrEqual(snapshot.snapshot_sequence);
    expect(validateEvent(bootToast, snapshot, null)).toBe('applied');
  });

  it('the negative case: seeding with S drops the boot toast as stale', () => {
    // Seeding the gate with snapshot_sequence (the choice §3.4 forbids)
    // would silently drop the boot toast — proving the seed matters.
    const snapshot = makeBootstrap({ snapshot_sequence: 3 });
    const bootToast = makeEvent(1, BOOT_TOAST_PAYLOAD);
    expect(validateEvent(bootToast, snapshot, snapshot.snapshot_sequence)).toBe('stale');
  });
});

describe('terminal wire shapes', () => {
  const envelopePayload = (call: number): { type: string; payload: Record<string, unknown> } => {
    const args = invokeMock.mock.calls[call][1] as {
      envelope: { payload: { type: string; payload: Record<string, unknown> } };
    };
    return args.envelope.payload;
  };

  it('TerminalCreate serializes exactly { request_id, cols, rows }', async () => {
    const { requestId, promise } = createTerminal(100, 30, CONTEXT);
    await promise;
    const payload = envelopePayload(0);
    expect(payload.type).toBe('terminal_create');
    expect(payload.payload).toEqual({ request_id: requestId, cols: 100, rows: 30 });
  });

  it('TerminalInput serializes exactly { terminal_id, data }', async () => {
    await terminalInput('term-1', 'ls\r', CONTEXT);
    expect(envelopePayload(0)).toEqual({
      type: 'terminal_input',
      payload: { terminal_id: 'term-1', data: 'ls\r' },
    });
  });

  it('TerminalResize serializes exactly { terminal_id, cols, rows }', async () => {
    await terminalResize('term-1', 120, 40, CONTEXT);
    expect(envelopePayload(0)).toEqual({
      type: 'terminal_resize',
      payload: { terminal_id: 'term-1', cols: 120, rows: 40 },
    });
  });

  it('TerminalClose serializes exactly { terminal_id }', async () => {
    await terminalClose('term-1', CONTEXT);
    expect(envelopePayload(0)).toEqual({
      type: 'terminal_close',
      payload: { terminal_id: 'term-1' },
    });
  });
});

describe('API key and provider bridge commands', () => {
  it('connectApiKey invokes connect_api_key with apiKey and baseUrl', async () => {
    invokeMock.mockResolvedValueOnce({
      connected: true,
      models_count: 5,
      active_model_id: 'glm-4',
      base_url: 'https://open.bigmodel.cn/api/paas/v4',
    });
    const result = await connectApiKey('my-secret-key', 'https://open.bigmodel.cn/api/paas/v4');
    expect(invokeMock).toHaveBeenCalledWith('connect_api_key', {
      apiKey: 'my-secret-key',
      baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    });
    expect(result.connected).toBe(true);
    expect(result.models_count).toBe(5);
  });

  it('connectApiKey passes null baseUrl when omitted', async () => {
    invokeMock.mockResolvedValueOnce({
      connected: true,
      models_count: 2,
      active_model_id: null,
      base_url: 'https://api.selora.lol',
    });
    await connectApiKey('my-secret-key');
    expect(invokeMock).toHaveBeenCalledWith('connect_api_key', {
      apiKey: 'my-secret-key',
      baseUrl: null,
    });
  });

  it('disconnectApiKey invokes disconnect_api_key', async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await disconnectApiKey();
    expect(invokeMock).toHaveBeenCalledWith('disconnect_api_key');
  });

  it('getApiKeyStatus invokes get_api_key_status and returns status', async () => {
    invokeMock.mockResolvedValueOnce({
      connected: true,
      has_key: true,
      source: 'keyring',
      base_url: 'https://api.selora.lol',
      key_hint: 'sk-••••1234',
      active_model_id: 'glm-4',
    });
    const status = await getApiKeyStatus();
    expect(invokeMock).toHaveBeenCalledWith('get_api_key_status');
    expect(status.has_key).toBe(true);
    expect(status.key_hint).toBe('sk-••••1234');
  });
});
