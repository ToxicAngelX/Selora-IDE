// Selora IDE — Phase Z2/Z5a toast store.
//
// The live toast stack (ARCHITECTURE §8.2, UI_SPEC §13): bottom-right above
// the status bar, max 3 visible, 4s auto-dismiss for info/success and 8s for
// warning/error, manual × dismissal, fade-only enter/exit. Protocol toasts
// are keyed by the event's sequence; locally generated command-error toasts
// use negative ids. Accepted toasts also append to a bounded notification
// history (newest first, max 50) persisted under `selora:notification_history`.
import { readTokenNumber } from '../layout/dom-tokens';
import type { ProtocolError, ToastSeverity } from '../../bridge/bridge';

/** ARCHITECTURE §8.2: 4s for info/success (calm), 8s for warning/error. */
export const TOAST_CALM_MS = 4000;
export const TOAST_ATTENTION_MS = 8000;

/** UI_SPEC §13: at most 3 visible; older ones fade out. */
export const TOAST_MAX_VISIBLE = 3;

/** Z5a: the notification center keeps the newest 50 entries. */
export const NOTIFICATION_HISTORY_MAX = 50;
export const NOTIFICATION_HISTORY_KEY = 'selora:notification_history';

export type Toast = {
  id: number;
  severity: ToastSeverity;
  message: string;
  /** Phase 9e: present when clicking should reveal a subagent result. */
  action?: NotificationAction;
};

export type NotificationEntry = {
  id: number;
  severity: ToastSeverity;
  message: string;
  at: number;
  read: boolean;
  action?: NotificationAction;
};

/**
 * Phase 9e: a clickable notification target. Selecting the parent session
 * and revealing the child result is the UI's job; the store only carries
 * the intent so history survives a reload.
 */
export type NotificationAction = {
  kind: 'subagent';
  childId: string;
  sessionId: string;
};

export type ToastState = {
  toasts: Toast[];
  leaving: number[];
  history: NotificationEntry[];
  unreadCount: number;
};

export function toastDuration(severity: ToastSeverity): number {
  return severity === 'warning' || severity === 'error' ? TOAST_ATTENTION_MS : TOAST_CALM_MS;
}

type Listener = () => void;
const listeners = new Set<Listener>();

const VALID_SEVERITIES = new Set<ToastSeverity>(['info', 'success', 'warning', 'error']);

function getStorage(): Storage | null {
  if (typeof window === 'undefined') return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function isNotificationAction(value: unknown): value is NotificationAction {
  if (!value || typeof value !== 'object') return false;
  const action = value as Record<string, unknown>;
  return (
    action.kind === 'subagent' &&
    typeof action.childId === 'string' &&
    typeof action.sessionId === 'string'
  );
}

function isNotificationEntry(value: unknown): value is NotificationEntry {
  if (!value || typeof value !== 'object') return false;
  const entry = value as Record<string, unknown>;
  return (
    typeof entry.id === 'number' &&
    Number.isFinite(entry.id) &&
    typeof entry.message === 'string' &&
    typeof entry.at === 'number' &&
    Number.isFinite(entry.at) &&
    Math.abs(entry.at) <= 8_640_000_000_000_000 &&
    typeof entry.read === 'boolean' &&
    typeof entry.severity === 'string' &&
    VALID_SEVERITIES.has(entry.severity as ToastSeverity) &&
    (entry.action === undefined || isNotificationAction(entry.action))
  );
}

function loadHistory(): NotificationEntry[] {
  try {
    const raw = getStorage()?.getItem(NOTIFICATION_HISTORY_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter(isNotificationEntry)
      .sort((left, right) => right.at - left.at)
      .slice(0, NOTIFICATION_HISTORY_MAX);
  } catch {
    return [];
  }
}

function persistHistory(): void {
  try {
    getStorage()?.setItem(NOTIFICATION_HISTORY_KEY, JSON.stringify(history));
  } catch {
    // Storage can be unavailable or full; notifications still work in memory.
  }
}

function countUnread(entries: NotificationEntry[]): number {
  return entries.reduce((count, entry) => (entry.read ? count : count + 1), 0);
}

let toasts: Toast[] = [];
let leaving: number[] = [];
let history: NotificationEntry[] = loadHistory();
let snapshot: ToastState = {
  toasts: [],
  leaving: [],
  history: [...history],
  unreadCount: countUnread(history),
};
const timers = new Map<number, ReturnType<typeof setTimeout>>();
/**
 * Phase 9e: a child completion notifies exactly once. Protocol sequence
 * dedupe already catches replay, but the child id is the durable key: a
 * second delivery under a new sequence must not stack another toast.
 */
const notifiedSubagentIds = new Set<string>();
const lowestStoredLocalId = history.reduce(
  (lowest, entry) => (entry.id < 0 ? Math.min(lowest, entry.id) : lowest),
  -1,
);
let nextLocalId = lowestStoredLocalId - 1;

function notify(): void {
  snapshot = {
    toasts: [...toasts],
    leaving: [...leaving],
    history: [...history],
    unreadCount: countUnread(history),
  };
  for (const listener of listeners) listener();
}

function recordNotification(toast: Toast): void {
  const entry: NotificationEntry = {
    id: toast.id,
    severity: toast.severity,
    message: toast.message,
    at: Date.now(),
    read: false,
  };
  if (toast.action) entry.action = { ...toast.action };
  history = [entry, ...history].slice(0, NOTIFICATION_HISTORY_MAX);
  persistHistory();
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
  if (!toasts.some((toast) => toast.id === id)) return;
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
  recordNotification(toast);
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

  /**
   * Phase 9e: a background child completion. Exactly one toast and one
   * history entry per child id. `ok: false` is a warning - never a success.
   * Returns the toast id, or null when this child was already notified.
   */
  pushSubagentCompletion(
    sequence: number,
    completion: {
      childId: string;
      sessionId: string;
      ok: boolean;
      message: string;
    },
  ): number | null {
    if (notifiedSubagentIds.has(completion.childId)) return null;
    notifiedSubagentIds.add(completion.childId);
    const toast: Toast = {
      id: sequence,
      severity: completion.ok ? 'success' : 'warning',
      message: completion.message,
      action: {
        kind: 'subagent',
        childId: completion.childId,
        sessionId: completion.sessionId,
      },
    };
    push(toast);
    return sequence;
  },

  /** The click target for a history entry (survives read/clear of live toasts). */
  actionFor(id: number): NotificationAction | undefined {
    return history.find((entry) => entry.id === id)?.action;
  },

  /** Click on × — immediate leave (UI_SPEC §13). */
  dismiss(id: number): void {
    beginLeave(id);
  },

  /** Explicit transient-only dismissal used by the notification surfaces. */
  dismissTransient(id: number): void {
    beginLeave(id);
  },

  /** Dismiss the topmost (newest) toast — the Esc-with-no-modal path (§7.1). */
  dismissTopmost(): void {
    const topmost = toasts[toasts.length - 1];
    if (topmost) beginLeave(topmost.id);
  },

  /** Mark every history entry read without touching transient toasts. */
  markAllRead(): void {
    if (!history.some((entry) => !entry.read)) return;
    history = history.map((entry) => (entry.read ? entry : { ...entry, read: true }));
    persistHistory();
    notify();
  },

  /** Mark one history entry read. Unknown or already-read ids are no-ops. */
  markRead(id: number): void {
    if (!history.some((entry) => entry.id === id && !entry.read)) return;
    history = history.map((entry) => (entry.id === id ? { ...entry, read: true } : entry));
    persistHistory();
    notify();
  },

  /** Clear the persisted notification history only. */
  clearHistory(): void {
    history = [];
    persistHistory();
    notify();
  },

  /** Clear transient toasts only; history survives a rebootstrap. */
  clearAll(): void {
    for (const id of [...timers.keys()]) clearTimer(id);
    toasts = [];
    leaving = [];
    notify();
  },
};
