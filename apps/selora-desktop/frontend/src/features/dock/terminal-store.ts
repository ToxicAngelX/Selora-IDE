// Selora IDE — terminal session store (terminal slice).
//
// Holds only what the protocol actually reports: terminal ids, shell, cwd,
// running/exited status and the exit code. Output is never stored in React
// state: the bridge events are pushed straight to the mounted xterm view
// (with a bounded buffer for the short window between TerminalCreated and the
// view's subscription). There is no fake shell and no fabricated prompt.
import {
  createTerminal,
  terminalClose,
  terminalInput,
  terminalResize,
  type EnvelopeContext,
  type TerminalCreatedPayload,
  type TerminalExitedPayload,
  type TerminalOutputPayload,
} from '../../bridge/bridge';

export type TerminalSessionStatus = 'running' | 'exited';

export type TerminalSession = {
  terminal_id: string;
  shell: string;
  cwd: string;
  status: TerminalSessionStatus;
  exit_code: number | null;
};

export type TerminalState = {
  sessions: TerminalSession[];
  active_id: string | null;
  /** A TerminalCreate round-trip is in flight. */
  creating: boolean;
  /** Truthful create/close failure text; null when no failure is showing. */
  error: string | null;
};

type Listener = () => void;
type OutputListener = (data: string) => void;

export const TERMINAL_CREATE_TIMEOUT_MS = 15_000;
/** Output held while no view is attached yet; old chunks are dropped past this. */
const OUTPUT_BUFFER_LIMIT = 512 * 1024;

const EMPTY_STATE: TerminalState = {
  sessions: [],
  active_id: null,
  creating: false,
  error: null,
};

let state: TerminalState = EMPTY_STATE;
let snapshot: TerminalState = state;
let createTimer: ReturnType<typeof setTimeout> | null = null;

const listeners = new Set<Listener>();
const outputListeners = new Map<string, Set<OutputListener>>();
const pendingOutput = new Map<string, string[]>();
let pendingOutputBytes = 0;

function notify(): void {
  snapshot = {
    ...state,
    sessions: state.sessions.slice(),
  };
  for (const listener of listeners) listener();
}

function describeCause(cause: unknown): string {
  if (typeof cause === 'object' && cause !== null) {
    const candidate = cause as { code?: unknown; message?: unknown };
    if (typeof candidate.message === 'string') {
      return typeof candidate.code === 'string'
        ? `${candidate.code}: ${candidate.message}`
        : candidate.message;
    }
  }
  return cause instanceof Error ? cause.message : String(cause);
}

function clearCreateTimer(): void {
  if (createTimer !== null) {
    clearTimeout(createTimer);
    createTimer = null;
  }
}

function setError(message: string | null): void {
  state = { ...state, error: message };
  notify();
}

function sessionLabel(session: TerminalSession): string {
  const normalized = session.shell.trim();
  if (normalized === '') return 'Terminal';
  const parts = normalized.split(/[\\/]/);
  return parts[parts.length - 1] || 'Terminal';
}

export function terminalTabLabel(session: TerminalSession, index: number): string {
  return `${sessionLabel(session)} ${index + 1}`;
}

function removeSession(terminalId: string): void {
  const index = state.sessions.findIndex((session) => session.terminal_id === terminalId);
  if (index < 0) return;
  const sessions = state.sessions.filter((session) => session.terminal_id !== terminalId);
  let activeId = state.active_id;
  if (activeId === terminalId) {
    const next = sessions[Math.min(index, sessions.length - 1)];
    activeId = next ? next.terminal_id : null;
  }
  state = { ...state, sessions, active_id: activeId };
  outputListeners.delete(terminalId);
  pendingOutput.delete(terminalId);
  pendingOutputBytes = 0;
  for (const chunks of pendingOutput.values()) {
    for (const chunk of chunks) pendingOutputBytes += chunk.length;
  }
  notify();
}

export const terminalStore = {
  subscribe(listener: Listener): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },

  getSnapshot(): TerminalState {
    return snapshot;
  },

  /**
   * Send TerminalCreate with the measured grid. A second request is ignored
   * while one is in flight so double effects cannot open two shells.
   */
  requestCreate(cols: number, rows: number, ctx: EnvelopeContext): Promise<void> {
    if (state.creating) return Promise.resolve();
    clearCreateTimer();
    state = { ...state, creating: true, error: null };
    notify();
    const request = createTerminal(cols, rows, ctx);
    createTimer = setTimeout(() => {
      createTimer = null;
      if (!state.creating) return;
      state = { ...state, creating: false };
      setError(
        `No terminal was created within ${TERMINAL_CREATE_TIMEOUT_MS / 1000}s. ` +
          'The backend did not answer TerminalCreate; the terminal is not running.',
      );
    }, TERMINAL_CREATE_TIMEOUT_MS);
    return request.promise.then(
      () => undefined,
      (cause: unknown) => {
        clearCreateTimer();
        state = { ...state, creating: false };
        setError(describeCause(cause));
      },
    );
  },

  /** Tab-local activation; no command is sent for a switch. */
  activate(terminalId: string): void {
    if (!state.sessions.some((session) => session.terminal_id === terminalId)) return;
    if (state.active_id === terminalId) return;
    state = { ...state, active_id: terminalId };
    notify();
  },

  /**
   * TerminalClose. The tab is removed optimistically so the UI stays
   * responsive, but a refused close restores it: hiding a live PTY would lie
   * about the process still running on the backend.
   */
  async close(terminalId: string, ctx: EnvelopeContext): Promise<void> {
    const before = state;
    if (!before.sessions.some((session) => session.terminal_id === terminalId)) return;
    removeSession(terminalId);
    try {
      await terminalClose(terminalId, ctx);
    } catch (cause) {
      state = {
        ...before,
        error: `Could not close the terminal: ${describeCause(cause)}`,
      };
      notify();
    }
  },

  /** TerminalInput; only a running terminal accepts keystrokes. */
  sendInput(terminalId: string, data: string, ctx: EnvelopeContext): void {
    const session = state.sessions.find((entry) => entry.terminal_id === terminalId);
    if (!session || session.status !== 'running' || data === '') return;
    void terminalInput(terminalId, data, ctx).catch(() => {
      // The bridge already reports every dispatch failure on its global
      // error stream (toast surface); no duplicate banner per keystroke.
    });
  },

  /** TerminalResize; the caller suppresses unchanged dimensions. */
  sendResize(terminalId: string, cols: number, rows: number, ctx: EnvelopeContext): void {
    if (cols <= 0 || rows <= 0) return;
    const session = state.sessions.find((entry) => entry.terminal_id === terminalId);
    if (!session) return;
    void terminalResize(terminalId, cols, rows, ctx).catch(() => {
      // Same global error path as TerminalInput.
    });
  },

  subscribeOutput(terminalId: string, listener: OutputListener): () => void {
    let handlers = outputListeners.get(terminalId);
    if (!handlers) {
      handlers = new Set();
      outputListeners.set(terminalId, handlers);
    }
    handlers.add(listener);
    const buffered = pendingOutput.get(terminalId);
    if (buffered) {
      pendingOutput.delete(terminalId);
      for (const chunk of buffered) {
        pendingOutputBytes -= chunk.length;
        listener(chunk);
      }
    }
    return () => {
      handlers?.delete(listener);
    };
  },

  handleCreated(payload: TerminalCreatedPayload): void {
    clearCreateTimer();
    const existing = state.sessions.find(
      (session) => session.terminal_id === payload.terminal_id,
    );
    const created: TerminalSession = {
      terminal_id: payload.terminal_id,
      shell: payload.shell,
      cwd: payload.cwd,
      status: 'running',
      exit_code: null,
    };
    state = {
      ...state,
      sessions: existing
        ? state.sessions.map((session) =>
            session.terminal_id === payload.terminal_id ? created : session,
          )
        : [...state.sessions, created],
      active_id: payload.terminal_id,
      creating: false,
      error: null,
    };
    notify();
  },

  handleOutput(payload: TerminalOutputPayload): void {
    if (!state.sessions.some((session) => session.terminal_id === payload.terminal_id)) return;
    const handlers = outputListeners.get(payload.terminal_id);
    if (handlers && handlers.size > 0) {
      for (const handler of handlers) handler(payload.data);
      return;
    }
    const chunks = pendingOutput.get(payload.terminal_id) ?? [];
    chunks.push(payload.data);
    pendingOutput.set(payload.terminal_id, chunks);
    pendingOutputBytes += payload.data.length;
    while (pendingOutputBytes > OUTPUT_BUFFER_LIMIT && chunks.length > 1) {
      const dropped = chunks.shift();
      pendingOutputBytes -= dropped?.length ?? 0;
    }
  },

  handleExited(payload: TerminalExitedPayload): void {
    if (!state.sessions.some((session) => session.terminal_id === payload.terminal_id)) return;
    state = {
      ...state,
      sessions: state.sessions.map((session) =>
        session.terminal_id === payload.terminal_id
          ? { ...session, status: 'exited', exit_code: payload.code }
          : session,
      ),
    };
    notify();
  },

  /** A truthful non-protocol failure (no workspace open, view failure). */
  reportError(message: string): void {
    clearCreateTimer();
    state = { ...state, creating: false };
    setError(message);
  },

  dismissError(): void {
    if (state.error === null) return;
    setError(null);
  },

  reset(): void {
    clearCreateTimer();
    outputListeners.clear();
    pendingOutput.clear();
    pendingOutputBytes = 0;
    state = EMPTY_STATE;
    notify();
  },
};