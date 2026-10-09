// App-store tests (ARCHITECTURE §3.4/§10): the startup/replay sequence —
// bootstrap, subscribe+buffer, getEventsAfter(0) replayed with the gate
// seeded null (the boot toast applies), live-drain with duplicate
// rejection, and the gap → discard + rebootstrap path.
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../bridge/bridge', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../bridge/bridge')>();
  return {
    ...actual,
    getBootstrapState: vi.fn(),
    getEventsAfter: vi.fn(),
    listenProtocolEvent: vi.fn(),
  };
});

import { getBootstrapState, getEventsAfter, listenProtocolEvent, type EventEnvelope } from '../bridge/bridge';
import { BOOT_TOAST_PAYLOAD, makeBootstrap, makeEvent, makeLayout } from '../testing/fixtures';

const getBootstrapStateMock = vi.mocked(getBootstrapState);
const getEventsAfterMock = vi.mocked(getEventsAfter);
const listenProtocolEventMock = vi.mocked(listenProtocolEvent);

type Harness = {
  startApp: () => Promise<void>;
  subscribeApp: (listener: () => void) => () => void;
  getAppSnapshot: () => { phase: string; bootstrap: unknown; error: string | null };
  toastStore: { getSnapshot: () => { toasts: Array<{ id: number; message: string }> } };
  layoutStore: { getSnapshot: () => { layout: unknown; hydrated: boolean } };
  emit: (event: EventEnvelope) => void;
};

async function freshHarness(): Promise<Harness> {
  vi.resetModules();
  let handler: ((event: EventEnvelope) => void) | null = null;
  listenProtocolEventMock.mockImplementation(async (h) => {
    handler = h;
    return () => {
      handler = null;
    };
  });
  const { startApp, subscribeApp, getAppSnapshot } = await import('./app-store');
  const { toastStore } = await import('../features/toasts/toast-store');
  const { layoutStore } = await import('../features/layout/layout-store');
  return {
    startApp,
    subscribeApp,
    getAppSnapshot,
    toastStore,
    layoutStore,
    emit: (event: EventEnvelope) => handler?.(event),
  };
}

const BOOTSTRAP = makeBootstrap({ snapshot_sequence: 2 });

beforeEach(() => {
  vi.clearAllMocks();
  getBootstrapStateMock.mockResolvedValue(BOOTSTRAP);
  getEventsAfterMock.mockResolvedValue([]);
});

describe('bootstrap and replay (ARCHITECTURE §3.4)', () => {
  it('hydrates the layout store from bootstrap and reaches ready', async () => {
    const h = await freshHarness();
    await h.startApp();
    const snapshot = h.getAppSnapshot();
    expect(snapshot.phase).toBe('ready');
    expect(h.layoutStore.getSnapshot().hydrated).toBe(true);
    expect(h.layoutStore.getSnapshot().layout).toEqual(makeLayout());
    expect(getEventsAfterMock).toHaveBeenCalledWith(0);
  });

  it('replays the boot toast through the null-seeded gate', async () => {
    // The boot-time settings toast (sequence ≤ snapshot_sequence) was
    // recorded before the subscription; replay must deliver it.
    getEventsAfterMock.mockResolvedValue([makeEvent(1, BOOT_TOAST_PAYLOAD), makeEvent(2, makeLayoutChange())]);
    const h = await freshHarness();
    await h.startApp();
    const toasts = h.toastStore.getSnapshot().toasts;
    expect(toasts.map((toast) => toast.id)).toEqual([1]);
    expect(toasts[0].message).toBe(BOOT_TOAST_PAYLOAD.payload.message);
    // The replayed layout event also applied.
    expect(h.layoutStore.getSnapshot().layout).toEqual(makeLayout({ primary_sidebar_width_px: 300 }));
  });

  it('applies live events in order and drops replay/live overlap duplicates', async () => {
    getEventsAfterMock.mockResolvedValue([makeEvent(1, BOOT_TOAST_PAYLOAD)]);
    const h = await freshHarness();
    await h.startApp();
    h.emit(makeEvent(2, makeLayoutChange()));
    h.emit(makeEvent(1, BOOT_TOAST_PAYLOAD)); // duplicate from the overlap → stale
    expect(h.toastStore.getSnapshot().toasts).toHaveLength(1);
    expect(h.layoutStore.getSnapshot().layout).toEqual(makeLayout({ primary_sidebar_width_px: 300 }));
    h.emit(makeEvent(3, { type: 'toast', payload: { severity: 'info', message: 'later' } }));
    expect(h.toastStore.getSnapshot().toasts.map((toast) => toast.id)).toEqual([1, 3]);
  });

  it('a gap discards held state and re-runs the bootstrap flow', async () => {
    getEventsAfterMock.mockResolvedValue([makeEvent(1, BOOT_TOAST_PAYLOAD)]);
    const h = await freshHarness();
    await h.startApp();
    expect(h.toastStore.getSnapshot().toasts).toHaveLength(1);
    h.emit(makeEvent(5, BOOT_TOAST_PAYLOAD)); // gap after 1
    await vi.waitFor(() => expect(getBootstrapStateMock).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(h.getAppSnapshot().phase).toBe('ready'));
    // Held state was discarded: the toast stack is empty until the new flow
    // replays (the second replay returns [] here).
    expect(h.toastStore.getSnapshot().toasts).toHaveLength(0);
  });

  it('an epoch mismatch re-bootstraps the same way', async () => {
    const h = await freshHarness();
    await h.startApp();
    h.emit(makeEvent(1, BOOT_TOAST_PAYLOAD, 'core-9999'));
    await vi.waitFor(() => expect(getBootstrapStateMock).toHaveBeenCalledTimes(2));
  });

  it('ignores non-Z2 protocol events explicitly', async () => {
    const h = await freshHarness();
    await h.startApp();
    h.emit(makeEvent(1, { type: 'chat_delta', payload: { request_id: 'r', session_id: 's', turn_id: 't', message_id: 'm', sequence: 1, text: 'x' } }));
    h.emit(makeEvent(2, { type: 'models_changed', payload: { models: [], active_model_id: null } }));
    expect(h.toastStore.getSnapshot().toasts).toHaveLength(0);
    // The events were applied (consumed) — a subsequent in-order toast lands.
    h.emit(makeEvent(3, { type: 'toast', payload: { severity: 'info', message: 'ok' } }));
    expect(h.toastStore.getSnapshot().toasts).toHaveLength(1);
  });

  it('startApp is StrictMode-safe: a second call does not re-run the flow', async () => {
    const h = await freshHarness();
    await h.startApp();
    await h.startApp();
    expect(getBootstrapStateMock).toHaveBeenCalledTimes(1);
  });

  it('bootstrap.settings.reduced_motion drives the root data-reduced-motion attribute (DESIGN §13.2)', async () => {
    getBootstrapStateMock.mockResolvedValue(makeBootstrap({ settings: { ...BOOTSTRAP.settings, reduced_motion: true } }));
    getEventsAfterMock.mockResolvedValue([makeEvent(1, BOOT_TOAST_PAYLOAD)]);
    const h = await freshHarness();
    await h.startApp();
    expect(document.documentElement.dataset.reducedMotion).toBe('true');
    // A rebootstrap with the setting off clears the attribute — effective
    // reduced motion is the OR of the setting and the OS query.
    getBootstrapStateMock.mockResolvedValue(BOOTSTRAP);
    h.emit(makeEvent(5, BOOT_TOAST_PAYLOAD)); // gap → discard + rebootstrap
    await vi.waitFor(() => expect(getBootstrapStateMock).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(document.documentElement.dataset.reducedMotion).toBeUndefined());
  });
});

function makeLayoutChange(): { type: 'workspace_layout_changed'; payload: ReturnType<typeof makeLayout> } {
  return { type: 'workspace_layout_changed', payload: makeLayout({ primary_sidebar_width_px: 300 }) };
}
