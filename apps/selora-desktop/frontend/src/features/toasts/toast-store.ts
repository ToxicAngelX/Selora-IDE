// Selora IDE — Phase Z2 toast store.
//
// The live toast stack (ARCHITECTURE §8.2, UI_SPEC §13): bottom-right above
// the status bar, max 3 visible, 4s auto-dismiss for info/success and 8s for
// warning/error, manual × dismissal, fade-only enter/exit. Protocol toasts
// are keyed by the event's sequence; locally generated command-error toasts
// use negative ids. Nothing persists.
import { readTokenNumber } from '../layout/dom-tokens';
import type { ProtocolError, ToastSeverity } from '../../bridge/bridge';

/** ARCHITECTURE §8.2: 4s for info/success (calm), 8s for warning/error. */
export const TOAST_CALM_MS = 4000;
export const TOAST_ATTENTION_MS = 8000;

/** UI_SPEC §13: at most 3 visible; older ones fade out. */
export const TOAST_MAX_VISIBLE = 3;

export type Toast = {
  id: number;
  severity: ToastSeverity;
  message: string;
};

export type ToastState = {
  toasts: Toast[];
  leaving: number[];
};

export function toastDuration(severity: ToastSeverity): number {
  return severity === 'warning' || severity === 'error' ? TOAST_ATTENTION_MS : TOAST_CALM_MS;
}

type Listener = () => void;
const listeners = new Set<Listener>();

let toasts: Toast[] = [];
let leaving: number[] = [];
let snapshot: ToastState = { toasts, leaving: [] };
const timers = new Map<number, ReturnType<typeof setTimeout>>();
let nextLocalId = -1;

function notify(): void {
  snapshot = { toasts: [...toasts], leaving: [...leaving] };
  for (const listener of listeners) listener();
}

function clearTimer(id: number): void {
  const timer = timers.get(id);
  if (timer !== undefined) {
    clearTimeout(timer);
    timers.delete(id);
  }
}

/** The exit fade length comes from the token layer (DESIGN §8.2: fade, --dur-base). */
function exitFadeMs(): number {
  const ms = readTokenNumber('--dur-base');
  return ms !== null && ms > 0 ? ms : 0;
}

function removeFromList(id: number): void {
  clearTimer(id);
  const index = toasts.findIndex((toast) => toast.id === id);
  if (index !== -1) toasts.splice(index, 1);
  leaving = leaving.filter((leavingId) => leavingId !== id);
  notify();
}

function beginLeave(id: number): void {
  if (leaving.includes(id)) return;
  clearTimer(id);
  leaving = [...leaving, id];
  notify();
  const fade = exitFadeMs();
  if (fade <= 0) {
    removeFromList(id);
    return;
  }
  const timer = setTimeout(() => removeFromList(id), fade);
  timers.set(id, timer);
}

function push(toast: Toast): void {
  // A duplicate key (replayed protocol event) is dropped, not stacked.
  if (toasts.some((existing) => existing.id === toast.id)) return;
  const overflow = toasts.length + 1 - TOAST_MAX_VISIBLE;
  toasts = [...toasts, toast];
  for (let i = 0; i < overflow; i += 1) {
    const oldest = toasts[i];
    if (oldest) beginLeave(oldest.id);
  }
  notify();
  const timer = setTimeout(() => beginLeave(toast.id), toastDuration(toast.severity));
  timers.set(toast.id, timer);
}

export const toastStore = {
  subscribe(listener: Listener): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },

  getSnapshot(): ToastState {
    return snapshot;
  },

  /** A protocol Toast event, keyed by its envelope sequence (ARCHITECTURE §8.2). */
  pushProtocol(sequence: number, toast: { severity: ToastSeverity; message: string }): void {
    push({ id: sequence, severity: toast.severity, message: toast.message });
  },

  /** A protocol Error event surfaced as an error toast. */
  pushProtocolError(sequence: number, error: ProtocolError): void {
    push({ id: sequence, severity: 'error', message: error.message });
  },

  /** A local command error from the bridge's command-error path. */
  pushLocal(severity: ToastSeverity, message: string): number {
    const id = nextLocalId;
    nextLocalId -= 1;
    push({ id, severity, message });
    return id;
  },

  /** Click on × — immediate leave (UI_SPEC §13). */
  dismiss(id: number): void {
    beginLeave(id);
  },

  /** Dismiss the topmost (newest) toast — the Esc-with-no-modal path (§7.1). */
  dismissTopmost(): void {
    const topmost = toasts[toasts.length - 1];
    if (topmost) beginLeave(topmost.id);
  },

  /** Drop everything (the rebootstrap path discards held state). */
  clearAll(): void {
    for (const id of [...timers.keys()]) clearTimer(id);
    toasts = [];
    leaving = [];
    notify();
  },
};
