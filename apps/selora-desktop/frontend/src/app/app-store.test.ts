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

import {
  getBootstrapState,
  getEventsAfter,
  listenProtocolEvent,
  type EnvelopeContext,
  type EventEnvelope,
  type FrontendEvent,
} from '../bridge/bridge';
import { BOOT_TOAST_PAYLOAD, makeBootstrap, makeEvent, makeLayout } from '../testing/fixtures';

const getBootstrapStateMock = vi.mocked(getBootstrapState);
const getEventsAfterMock = vi.mocked(getEventsAfter);
const listenProtocolEventMock = vi.mocked(listenProtocolEvent);

type Harness = {
  startApp: () => Promise<void>;
  subscribeApp: (listener: () => void) => () => void;
  getAppSnapshot: () => {
    phase: string;
    bootstrap: unknown;
    error: string | null;
    bufferConflicts?: Record<string, string>;
  };
  currentEnvelopeContext: () => EnvelopeContext | null;
  currentSessionEnvelopeContext: (sessionId?: string | null) => EnvelopeContext | null;
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
  const { startApp, subscribeApp, getAppSnapshot, currentEnvelopeContext, currentSessionEnvelopeContext } =
    await import('./app-store');
  const { toastStore } = await import('../features/toasts/toast-store');
  const { layoutStore } = await import('../features/layout/layout-store');
  return {
    startApp,
    subscribeApp,
    getAppSnapshot,
    currentEnvelopeContext,
    currentSessionEnvelopeContext,
    toastStore,
    layoutStore,
    emit: (event: EventEnvelope) => handler?.(event),
  };
}

const BOOTSTRAP = makeBootstrap({ snapshot_sequence: 2 });

beforeEach(() => {
  vi.clearAllMocks();
  delete document.documentElement.dataset.theme;
  delete document.documentElement.dataset.reducedMotion;
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

  it('applies the persisted theme and reduced-motion flag at bootstrap', async () => {
    getBootstrapStateMock.mockResolvedValue(
      makeBootstrap({
        settings: {
          reduced_motion: true,
          theme: 'light',
          keybind_command_palette: 'Ctrl+Shift+P',
          keybind_toggle_chat: 'Ctrl+L',
        },
      }),
    );
    const h = await freshHarness();
    await h.startApp();

    expect(document.documentElement.dataset.theme).toBe('light');
    expect(document.documentElement.dataset.reducedMotion).toBe('true');
  });

  it('switches theme live when a bootstrap_state event lands', async () => {
    const h = await freshHarness();
    await h.startApp();
    expect(document.documentElement.dataset.theme).toBe('dark');

    h.emit(
      makeEvent(1, {
        type: 'bootstrap_state',
        payload: makeBootstrap({
          settings: {
            reduced_motion: false,
            theme: 'light',
            keybind_command_palette: 'Ctrl+Shift+P',
            keybind_toggle_chat: 'Ctrl+L',
          },
        }),
      }),
    );

    expect(document.documentElement.dataset.theme).toBe('light');
    expect(document.documentElement.dataset.reducedMotion).toBeUndefined();
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

  it('tracks buffer conflict events and clears the marker on resolution', async () => {
    const h = await freshHarness();
    await h.startApp();
    h.emit(
      makeEvent(3, {
        type: 'buffer_conflict',
        payload: { buffer_id: 'buffer-1', path: 'src/main.rs' },
      }),
    );
    expect(h.getAppSnapshot().bufferConflicts).toEqual({ 'buffer-1': 'src/main.rs' });

    h.emit(
      makeEvent(4, {
        type: 'buffer_conflict_cleared',
        payload: { buffer_id: 'buffer-1', path: 'src/main.rs' },
      }),
    );
    expect(h.getAppSnapshot().bufferConflicts).toEqual({});
  });

  it('buffers live events until the replay finishes, then applies both in order exactly once', async () => {
    let resolveReplay!: (events: EventEnvelope[]) => void;
    getEventsAfterMock.mockImplementation(
      () =>
        new Promise<EventEnvelope[]>((resolve) => {
          resolveReplay = resolve;
        }),
    );
    const h = await freshHarness();
    const started = h.startApp();
    await vi.waitFor(() => expect(getEventsAfterMock).toHaveBeenCalledTimes(1));

    // A live event lands between the subscription and the replay result.
    h.emit(makeEvent(2, { type: 'toast', payload: { severity: 'info', message: 'live-after-subscribe' } }));
    // It must be buffered, not applied ahead of the replay (which would
    // advance the gate and make replay event 1 stale/out of order).
    expect(h.toastStore.getSnapshot().toasts).toHaveLength(0);

    resolveReplay([makeEvent(1, BOOT_TOAST_PAYLOAD)]);
    await started;

    expect(h.toastStore.getSnapshot().toasts.map((toast) => toast.id)).toEqual([1, 2]);
    expect(h.toastStore.getSnapshot().toasts.map((toast) => toast.message)).toEqual([
      BOOT_TOAST_PAYLOAD.payload.message,
      'live-after-subscribe',
    ]);

    // Re-delivery of the drained live event is stale — applied exactly once.
    h.emit(makeEvent(2, { type: 'toast', payload: { severity: 'info', message: 'live-after-subscribe' } }));
    expect(h.toastStore.getSnapshot().toasts.map((toast) => toast.id)).toEqual([1, 2]);
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

  it('routes the seven workflow protocol events into the workflows store', async () => {
    const h = await freshHarness();
    const { workflowsStore } = await import('../features/workflows/workflows-store');
    await h.startApp();

    // Bootstrap installed both outbound transports.
    expect(workflowsStore.getSnapshot().stop_wired).toBe(true);
    expect(workflowsStore.getSnapshot().approval_wired).toBe(true);

    const workflowEvents: FrontendEvent[] = [
      {
        type: 'workflow_started',
        payload: { run_id: 'run_1', request_id: 'req_1', name: 'Review Rust files', max_cost_usd: 5 },
      },
      {
        type: 'workflow_phase',
        payload: { run_id: 'run_1', phase: { name: 'Scan', status: 'running' } },
      },
      {
        type: 'workflow_step',
        payload: {
          run_id: 'run_1',
          step: { id: 'step_glob', phase: 'Scan', label: 'Glob src/**/*.rs', status: 'running', agent: 'scout' },
        },
      },
      {
        type: 'workflow_report',
        payload: { run_id: 'run_1', report: { id: 'report_1', item: 'Found 12 unwrap() calls.' } },
      },
      {
        type: 'workflow_artifact',
        payload: {
          run_id: 'run_1',
          artifact: { id: 'artifact_1', title: 'Review summary', kind: 'markdown', body: '## Findings' },
        },
      },
      {
        type: 'workflow_approval_needed',
        payload: {
          run_id: 'run_1',
          request_id: 'req_2',
          approval_id: 'approval_1',
          summary: 'Write reports/review.md',
          path_or_command: 'reports/review.md',
          can_approve_all_this_turn: false,
        },
      },
      {
        type: 'workflow_finished',
        payload: {
          run_id: 'run_1',
          request_id: 'req_1',
          status: 'completed',
          tokens: 1234,
          cost_usd: 0.42,
          max_cost_usd: 5,
        },
      },
    ];
    workflowEvents.forEach((event, index) => h.emit(makeEvent(index + 1, event)));

    const run = workflowsStore.getSnapshot().runs[0];
    expect(run?.id).toBe('run_1');
    expect(run?.status).toBe('completed');
    expect(run?.phases).toEqual([{ name: 'Scan', status: 'running' }]);
    expect(run?.steps[0]?.label).toBe('Glob src/**/*.rs');
    expect(run?.steps[0]?.agent).toBe('scout');
    expect(run?.reports[0]?.text).toBe('Found 12 unwrap() calls.');
    expect(run?.artifacts[0]?.content).toBe('## Findings');
    expect(run?.approvals[0]?.state).toBe('pending');
    expect(run?.approvals[0]?.request_id).toBe('req_2');
    expect(run?.tokens).toBe(1234);
    expect(run?.cost_usd).toBe(0.42);
  });

  it('hydrates completed journal history and upgrades it with the live run in place', async () => {
    getBootstrapStateMock.mockResolvedValue(makeBootstrap({
      workflows: [{
        run_id: 'run_done',
        name: 'Recovered review',
        status: 'completed',
        resumable: false,
        phases: ['Scan'],
        steps: [{
          id: 'step-1',
          phase: '',
          label: 'inspect src/lib.rs',
          status: 'completed',
          agent: 'reviewer',
        }],
        reports: [{ id: 'report-1', item: 'kept', artifact_id: 'summary' }],
        artifacts: [{ id: 'summary', title: 'Review summary', kind: 'markdown', body: '## Findings' }],
        tokens: 1234,
        cost_usd: 0.42,
      }],
    }));
    const h = await freshHarness();
    const { workflowsStore } = await import('../features/workflows/workflows-store');
    await h.startApp();

    const recovered = workflowsStore.getSnapshot().runs;
    expect(recovered).toHaveLength(1);
    expect(recovered[0]).toMatchObject({
      id: 'run_done',
      status: 'completed',
      resumable: false,
      tokens: 1234,
      cost_usd: 0.42,
    });
    expect(recovered[0]?.phases).toEqual([{ name: 'Scan', status: 'completed' }]);
    expect(recovered[0]?.steps[0]?.label).toBe('inspect src/lib.rs');
    expect(recovered[0]?.reports[0]?.text).toBe('kept');
    expect(recovered[0]?.artifacts[0]?.content).toBe('## Findings');
    expect(recovered[0]?.log[0]?.message).toBe('Recovered from journal: Recovered review');

    // A live started event for the same id upgrades that entry in place.
    h.emit(makeEvent(1, {
      type: 'workflow_started',
      payload: {
        run_id: 'run_done',
        request_id: 'req_live',
        name: 'Recovered review',
        max_cost_usd: 5,
      },
    }));

    const runs = workflowsStore.getSnapshot().runs;
    expect(runs.filter((run) => run.id === 'run_done')).toHaveLength(1);
    expect(runs[0]?.request_id).toBe('req_live');
    expect(runs[0]?.status).toBe('completed');
    expect(runs[0]?.reports[0]?.text).toBe('kept');
  });

  it('keeps non-workflow events out of the workflows store', async () => {
    const h = await freshHarness();
    const { workflowsStore } = await import('../features/workflows/workflows-store');
    await h.startApp();

    h.emit(makeEvent(1, { type: 'toast', payload: { severity: 'info', message: 'not a workflow' } }));
    h.emit(makeEvent(2, { type: 'chat_delta', payload: { request_id: 'r', session_id: 's', turn_id: 't', message_id: 'm', sequence: 1, text: 'x' } }));
    expect(workflowsStore.getSnapshot().runs).toEqual([]);

    // The gate still consumed them in order: the next workflow event applies.
    h.emit(
      makeEvent(3, {
        type: 'workflow_started',
        payload: { run_id: 'run_1', request_id: 'req_1', name: 'Review Rust files', max_cost_usd: 5 },
      }),
    );
    expect(workflowsStore.getSnapshot().runs.map((run) => run.id)).toEqual(['run_1']);
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

  it('the default context never sends the active chat session on non-session commands', async () => {
    getBootstrapStateMock.mockResolvedValue(
      makeBootstrap({ chat: { ...BOOTSTRAP.chat, active_session_id: 'sess-live' } }),
    );
    const h = await freshHarness();
    await h.startApp();
    // settings/search/git/editor commands would be rejected by Rust dispatch
    // with stale_workspace_or_session if this carried the chat session.
    expect(h.currentEnvelopeContext()).toEqual({
      core_instance_id: 'core-0001',
      workspace_id: 'no-workspace',
      session_id: null,
    });
  });

  it('the session context carries the active chat session for the session commands', async () => {
    getBootstrapStateMock.mockResolvedValue(
      makeBootstrap({ chat: { ...BOOTSTRAP.chat, active_session_id: 'sess-live' } }),
    );
    const h = await freshHarness();
    await h.startApp();
    expect(h.currentSessionEnvelopeContext()).toEqual({
      core_instance_id: 'core-0001',
      workspace_id: 'no-workspace',
      session_id: 'sess-live',
    });
    // An explicit override (the agent's live active session) wins.
    expect(h.currentSessionEnvelopeContext('sess-other')?.session_id).toBe('sess-other');
  });
});

function makeLayoutChange(): { type: 'workspace_layout_changed'; payload: ReturnType<typeof makeLayout> } {
  return { type: 'workspace_layout_changed', payload: makeLayout({ primary_sidebar_width_px: 300 }) };
}
