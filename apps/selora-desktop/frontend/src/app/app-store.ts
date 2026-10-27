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
  type BufferSnapshot,
  type CompletionResult,
  type DefinitionResult,
  type EnvelopeContext,
  type EventEnvelope,
  type HoverResult,
} from '../bridge/bridge';
import { layoutStore } from '../features/layout/layout-store';
import { toastStore } from '../features/toasts/toast-store';
import { agentStore } from '../features/agent/agent-store';
import { workflowsStore } from '../features/workflows/workflows-store';
import {
  installWorkflowHandlers,
  toWorkflowStoreEvent,
} from '../features/workflows/workflows-bridge';

export type AppPhase = 'connecting' | 'ready' | 'error';

/** Z4 — the search stream's latest result (the SearchPanel's view model). */
export type SearchResultsState = {
  request_id: string | null;
  query: { query: string; case_sensitive: boolean; whole_word: boolean; use_regex: boolean };
  hits: { path: string; line: number; column: number; preview: string }[];
};

/** Z4 — the Git panel's read-only diff overlay payload. */
export type GitDiffState = { path: string; unified_diff: string };

export type AppState = {
  phase: AppPhase;
  bootstrap: BootstrapState | null;
  error: string | null;
  /** Z4: latest SearchResults event; null until the first search. */
  searchResults: SearchResultsState | null;
  /** Z4: latest GitDiffReady event; null when the diff overlay is closed. */
  gitDiff: GitDiffState | null;
  /** Z4b: the latest LSP answers, keyed to the buffer they were asked for. */
  hoverResult: HoverResult | null;
  completionResult: CompletionResult | null;
  definitionResult: DefinitionResult | null;
};

type Listener = () => void;
const listeners = new Set<Listener>();

let state: AppState = {
  phase: 'connecting',
  bootstrap: null,
  error: null,
  searchResults: null,
  gitDiff: null,
  hoverResult: null,
  completionResult: null,
  definitionResult: null,
};
let snapshot = state;
let lastSequence: number | null = null;
let generation = 0;
let started = false;
let unlisten: (() => void) | null = null;
const liveBuffer: EventEnvelope[] = [];
let draining = false;
/**
 * True until the `getEventsAfter(0)` replay has finished. The listener only
 * buffers while it is set: applying a live event before the replay would
 * advance the gate ahead of the replayed history, so the replay would be
 * dropped as stale (or applied out of order) and gaps could trigger a
 * rebootstrap loop.
 */
let replaying = false;

function notify(): void {
  snapshot = {
    phase: state.phase,
    bootstrap: state.bootstrap,
    error: state.error,
    searchResults: state.searchResults,
    gitDiff: state.gitDiff,
    hoverResult: state.hoverResult,
    completionResult: state.completionResult,
    definitionResult: state.definitionResult,
  };
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

/**
 * Z4 — the envelope context every command helper needs. Read from the live
 * bootstrap so a workspace switch changes the id every helper sends.
 *
 * `session_id` is always null: Rust dispatch (state.rs) rejects any session
 * on the envelope unless the command is session-scoped (SendChat, StopChat,
 * SendAgent, AgentDecision). Stamping the active chat session here made every
 * editor/search/git/settings command fail with `stale_workspace_or_session`
 * while a turn was streaming.
 */
export function currentEnvelopeContext(): EnvelopeContext | null {
  const bootstrap = snapshot.bootstrap;
  if (!bootstrap) return null;
  return {
    core_instance_id: bootstrap.core_instance_id,
    workspace_id: bootstrap.workspace.id,
    session_id: null,
  };
}

/**
 * The session-scoped envelope context: SendChat/SendAgent/StopChat/
 * AgentDecision are the only commands allowed to carry a `session_id`, and
 * they need the live chat session so stop/decision still target the turn in
 * flight. Everything else must use {@link currentEnvelopeContext}.
 */
export function currentSessionEnvelopeContext(
  sessionId?: string | null,
): EnvelopeContext | null {
  const context = currentEnvelopeContext();
  if (!context) return null;
  return {
    ...context,
    session_id: sessionId ?? snapshot.bootstrap?.chat.active_session_id ?? null,
  };
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
  // Phase 8 wave 2: the Workflows store's Stop / Approve / Deny transports
  // ride the non-session envelope context and need no bootstrap payload, so
  // they are registered once here (StrictMode-safe via `started`).
  installWorkflowHandlers();
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
    workflowsStore.hydratePersisted(bootstrap.workflows ?? []);
    notify();
    return bootstrap;
  } catch (err) {
    console.error('refreshBootstrap failed:', err);
    return null;
  }
}

async function runBootstrapFlow(): Promise<void> {
  const gen = ++generation;
  // Reset the held state *and* the gate before subscribing (§3.4): the gate
  // is seeded null so the boot toast still replays.
  state = emptyState();
  lastSequence = null;
  replaying = true;
  liveBuffer.length = 0;
  notify();
  try {
    // 1. Bootstrap → record the envelope context. snapshot_sequence is
    //    diagnostics only — never the gate's seed (§3.4).
    const bootstrap = await getBootstrapState();
    if (gen !== generation) return;
    state = { ...state, phase: 'ready', bootstrap, error: null };
    applyReducedMotion(bootstrap.settings.reduced_motion);
    layoutStore.hydrate(bootstrap);
    agentStore.hydrate(bootstrap);
    workflowsStore.hydratePersisted(bootstrap.workflows ?? []);
    notify();

    // 2. Subscribe, buffering events until the replay has run.
    unlisten?.();
    unlisten = null;
    const stop = await listenProtocolEvent((event) => {
      if (gen !== generation) return;
      liveBuffer.push(event);
      // During the replay the listener must only buffer; draining here would
      // advance lastSequence past the replay and reorder it.
      if (!replaying) drainLive(gen);
    });
    if (gen !== generation) {
      stop();
      return;
    }
    unlisten = stop;

    // 3. Replay everything that preceded the subscription, through the same
    //    gate, in sequence order, with the gate seeded null.
    const replay = await getEventsAfter(0);
    if (gen !== generation) return;
    for (const event of replay) {
      if (gen !== generation) return;
      applyEvent(event, gen);
    }
    if (gen !== generation) return;

    // Seed the gate from the replay result, but never move it backwards if
    // it was already advanced further (e.g. a rebootstrap mid-replay).
    if (replay.length > 0) {
      const replayedThrough = replay[replay.length - 1]!.sequence;
      lastSequence =
        lastSequence === null ? replayedThrough : Math.max(lastSequence, replayedThrough);
    }

    // 4. Drain the buffered live events through the same gate; afterwards
    //    they apply as they arrive.
    replaying = false;
    drainLive(gen);
  } catch (cause) {
    if (gen !== generation) return;
    replaying = false;
    state = {
      ...state,
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
    case 'bootstrap_state':
      // Z4 — a workspace opened: the id, buffers and tree all move at once.
      state = { ...state, bootstrap: payload.payload, phase: 'ready' };
      applyReducedMotion(payload.payload.settings.reduced_motion);
      layoutStore.hydrate(payload.payload);
      agentStore.hydrate(payload.payload);
      notify();
      return;
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
    case 'session_todos_changed':
      agentStore.applySessionTodos(payload.payload);
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
    case 'buffer_snapshot':
      upsertBuffer(payload.payload);
      return;
    case 'buffer_changed': {
      if (!state.bootstrap) return;
      const changed = payload.payload;
      state.bootstrap = {
        ...state.bootstrap,
        buffers: state.bootstrap.buffers.map((buffer) =>
          buffer.buffer_id === changed.buffer_id
            ? { ...buffer, version: changed.version, dirty: true }
            : buffer,
        ),
        active_buffer_id: state.bootstrap.active_buffer_id ?? changed.buffer_id,
      };
      notify();
      return;
    }
    case 'file_tree_changed':
      if (state.bootstrap) {
        state.bootstrap = { ...state.bootstrap, file_tree: payload.payload };
        notify();
      }
      return;
    case 'diagnostics_changed':
      if (state.bootstrap) {
        state.bootstrap = { ...state.bootstrap, diagnostics: payload.payload };
        notify();
      }
      return;
    case 'git_status_changed':
      if (state.bootstrap) {
        state.bootstrap = { ...state.bootstrap, git: payload.payload };
        notify();
      }
      return;
    case 'search_results':
      state = { ...state, searchResults: payload.payload };
      notify();
      return;
    case 'git_diff_ready':
      state = { ...state, gitDiff: payload.payload };
      notify();
      return;
    case 'hover_result':
      state = { ...state, hoverResult: payload.payload };
      notify();
      return;
    case 'completion_result':
      state = { ...state, completionResult: payload.payload };
      notify();
      return;
    case 'definition_result':
      state = { ...state, definitionResult: payload.payload };
      notify();
      return;
    default: {
      // Phase 8 workflow progress: one pure mapper decides which protocol
      // events the Workflows panel consumes. Everything else stays explicitly
      // ignored (ARCHITECTURE §2.5).
      const workflowEvent = toWorkflowStoreEvent(payload);
      if (workflowEvent) workflowsStore.applyEvent(workflowEvent);
      return;
    }
  }
}

/** The pristine connecting state (also the discard/reset shape). */
function emptyState(): AppState {
  return {
    phase: 'connecting',
    bootstrap: null,
    error: null,
    searchResults: null,
    gitDiff: null,
    hoverResult: null,
    completionResult: null,
    definitionResult: null,
  };
}

/** Z4 — thread a BufferSnapshot into the bootstrap buffer list (upsert). */
function upsertBuffer(nextBuffer: BufferSnapshot): void {
  if (!state.bootstrap) return;
  const buffers = state.bootstrap.buffers;
  const existing = buffers.findIndex((buffer) => buffer.buffer_id === nextBuffer.buffer_id);
  state.bootstrap = {
    ...state.bootstrap,
    buffers:
      existing >= 0
        ? buffers.map((buffer, index) => (index === existing ? nextBuffer : buffer))
        : [...buffers, nextBuffer],
    active_buffer_id: state.bootstrap.active_buffer_id ?? nextBuffer.buffer_id,
  };
  notify();
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
  workflowsStore.reset();
  state = emptyState();
  notify();
}
