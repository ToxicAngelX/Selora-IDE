// Selora IDE — Phase Z2 app store: bootstrap, replay, and the event gate.
//
// Implements ARCHITECTURE §3.4 exactly: bootstrap → subscribe+buffer →
// getEventsAfter(0) replayed through the gate **seeded `lastSequence = null`**
// (never `snapshot_sequence` — seeding with S would drop the boot toast) →
// drain the live buffer → apply live events as they arrive. `stale` events
// are dropped; `gap`/`epoch-mismatch`/`version-mismatch` discard all held
// state and re-run the bootstrap flow. Z1's reload-bootstrap-on-any-event
// behavior is gone.
import {
  getBootstrapState,
  getEventsAfter,
  listenProtocolEvent,
  validateEvent,
  type BootstrapState,
  type EventEnvelope,
} from '../bridge/bridge';
import { layoutStore } from '../features/layout/layout-store';
import { toastStore } from '../features/toasts/toast-store';
import { agentStore } from '../features/agent/agent-store';

export type AppPhase = 'connecting' | 'ready' | 'error';
export type AppState = { phase: AppPhase; bootstrap: BootstrapState | null; error: string | null };

type Listener = () => void;
const listeners = new Set<Listener>();

let state: AppState = { phase: 'connecting', bootstrap: null, error: null };
let snapshot = state;
let lastSequence: number | null = null;
let generation = 0;
let started = false;
let unlisten: (() => void) | null = null;
const liveBuffer: EventEnvelope[] = [];
let draining = false;

function notify(): void {
  snapshot = { phase: state.phase, bootstrap: state.bootstrap, error: state.error };
  for (const listener of listeners) listener();
}

export function subscribeApp(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getAppSnapshot(): AppState {
  return snapshot;
}

/** Reflect the `ui.reduce_motion` setting on the root (DESIGN §13.2). */
function applyReducedMotion(reduced: boolean): void {
  const root = document.documentElement;
  if (reduced) {
    root.dataset.reducedMotion = 'true';
  } else {
    delete root.dataset.reducedMotion;
  }
}

/** Start the bootstrap flow once; StrictMode-safe (module singleton). */
export function startApp(): Promise<void> {
  if (started) return Promise.resolve();
  started = true;
  return runBootstrapFlow();
}

/** The Reconnect button on the error screen. */
export async function retryBootstrap(): Promise<void> {
  await runBootstrapFlow();
}

/** Refresh the bootstrap snapshot (e.g. after API key connect/disconnect). */
export async function refreshBootstrap(): Promise<BootstrapState | null> {
  try {
    const bootstrap = await getBootstrapState();
    state = { ...state, bootstrap, phase: 'ready' };
    applyReducedMotion(bootstrap.settings.reduced_motion);
    layoutStore.hydrate(bootstrap);
    agentStore.hydrate(bootstrap);
    notify();
    return bootstrap;
  } catch (err) {
    console.error('refreshBootstrap failed:', err);
    return null;
  }
}

async function runBootstrapFlow(): Promise<void> {
  const gen = ++generation;
  state = { phase: 'connecting', bootstrap: null, error: null };
  notify();
  try {
    // 1. Bootstrap → record the envelope context. snapshot_sequence is
    //    diagnostics only — never the gate's seed (§3.4).
    const bootstrap = await getBootstrapState();
    if (gen !== generation) return;
    state = { phase: 'ready', bootstrap, error: null };
    applyReducedMotion(bootstrap.settings.reduced_motion);
    layoutStore.hydrate(bootstrap);
    agentStore.hydrate(bootstrap);
    notify();

    // 2. Subscribe, buffering events until the replay has run.
    unlisten?.();
    unlisten = null;
    const stop = await listenProtocolEvent((event) => {
      if (gen !== generation) return;
      liveBuffer.push(event);
      drainLive(gen);
    });
    if (gen !== generation) {
      stop();
      return;
    }
    unlisten = stop;

    // 3. Replay everything that preceded the subscription, through the same
    //    gate, in sequence order, with the gate seeded null.
    lastSequence = null;
    const replay = await getEventsAfter(0);
    if (gen !== generation) return;
    for (const event of replay) {
      if (gen !== generation) return;
      applyEvent(event, gen);
    }

    // 4. Drain the buffered live events; afterwards they apply as they arrive.
    drainLive(gen);
  } catch (cause) {
    if (gen !== generation) return;
    state = {
      phase: 'error',
      bootstrap: null,
      error: cause instanceof Error ? cause.message : String(cause),
    };
    notify();
  }
}

function drainLive(gen: number): void {
  if (draining) return;
  draining = true;
  try {
    while (liveBuffer.length > 0) {
      if (gen !== generation) return;
      const event = liveBuffer.shift()!;
      applyEvent(event, gen);
    }
  } finally {
    draining = false;
  }
}

function applyEvent(event: EventEnvelope, gen: number): void {
  const bootstrap = state.bootstrap;
  if (!bootstrap) return;
  const result = validateEvent(event, bootstrap, lastSequence);
  if (result === 'stale') return; // duplicate from the replay/live overlap
  if (result === 'applied') {
    lastSequence = event.sequence;
    reduceEvent(event);
    return;
  }
  // gap | epoch-mismatch | version-mismatch: discard all held state and
  // re-run the bootstrap flow (§3.4).
  discardHeldState();
  void runBootstrapFlow();
}

function reduceEvent(event: EventEnvelope): void {
  const payload = event.payload;
  switch (payload.type) {
    case 'workspace_layout_changed':
      layoutStore.applyLayoutEvent(payload.payload);
      return;
    case 'toast':
      toastStore.pushProtocol(event.sequence, payload.payload);
      return;
    case 'error':
      toastStore.pushProtocolError(event.sequence, payload.payload);
      return;
    case 'chat_delta':
      agentStore.handleChatDelta(payload.payload);
      return;
    case 'chat_completed':
      agentStore.handleChatCompleted(payload.payload);
      return;
    case 'chat_failed':
      agentStore.handleChatFailed(payload.payload);
      return;
    case 'tool_call_started':
      agentStore.handleToolCallStarted(payload.payload);
      return;
    case 'tool_result':
      agentStore.handleToolResult(payload.payload);
      return;
    case 'approval_needed':
      agentStore.handleApprovalNeeded(payload.payload);
      return;
    case 'agent_state_changed':
      agentStore.handleAgentStateChanged(payload.payload);
      return;
    case 'agent_turn_done':
      agentStore.handleAgentTurnDone(payload.payload);
      return;
    case 'agent_event':
      agentStore.handleAgentEvent(payload.payload);
      return;
    case 'selora_status_changed':
      if (state.bootstrap) {
        state.bootstrap = {
          ...state.bootstrap,
          connection: payload.payload.status,
          active_model_id: payload.payload.active_model_id ?? state.bootstrap.active_model_id,
        };
        if (payload.payload.active_model_id) {
          agentStore.setSelectedModel(payload.payload.active_model_id);
        }
        notify();
      }
      return;
    case 'models_changed':
      if (state.bootstrap) {
        state.bootstrap = {
          ...state.bootstrap,
          models: payload.payload.models,
          active_model_id: payload.payload.active_model_id ?? state.bootstrap.active_model_id,
        };
        if (payload.payload.active_model_id) {
          agentStore.setSelectedModel(payload.payload.active_model_id);
        }
        agentStore.setAvailableModels(payload.payload.models);
        notify();
      }
      return;
    default:
      return; // explicitly ignored in Z2 (ARCHITECTURE §2.5)
  }
}

function discardHeldState(): void {
  generation += 1; // invalidate any in-flight flow
  liveBuffer.length = 0;
  lastSequence = null;
  unlisten?.();
  unlisten = null;
  toastStore.clearAll();
  layoutStore.reset();
  agentStore.reset();
  state = { phase: 'connecting', bootstrap: null, error: null };
  notify();
}
