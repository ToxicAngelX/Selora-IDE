// Bridge wire-shape and gate tests (ARCHITECTURE §2.6/§10): the exact §2.1
// command/event JSON, the envelope transport rules, the command-error path,
// and the stale/gap/epoch/version validation.
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { invoke } from '@tauri-apps/api/core';
import {
  dispatchCommand,
  getBootstrapState,
  getEventsAfter,
  onBridgeCommandError,
  PROTOCOL_VERSION,
  resizeRegion,
  saveLayout,
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
