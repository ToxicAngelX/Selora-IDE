// Toast-store tests (ARCHITECTURE §10): cap 3, the 4s/8s auto-dismiss
// timers, manual dismissal, duplicate protocol keys, and the Z5a
// notification history (cap 50, persistence, read state, clear semantics).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  NOTIFICATION_HISTORY_KEY,
  NOTIFICATION_HISTORY_MAX,
  TOAST_ATTENTION_MS,
  TOAST_CALM_MS,
  toastStore,
} from './toast-store';
import type { ToastSeverity } from '../../bridge/bridge';

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-01-01T12:00:00.000Z'));
  // The exit fade reads --dur-base from the token layer; happy-dom has no
  // stylesheet, so provide it the way the real app does (inline custom
  // properties resolve through getComputedStyle — verified by probe).
  document.documentElement.style.setProperty('--dur-base', '240ms');
  localStorage.clear();
  toastStore.clearHistory();
  toastStore.clearAll();
});

afterEach(() => {
  toastStore.clearAll();
  document.documentElement.style.removeProperty('--dur-base');
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('toast store (ARCHITECTURE §8.2)', () => {
  it('pushes protocol toasts keyed by the event sequence', () => {
    toastStore.pushProtocol(1, { severity: 'info', message: 'Indexing complete.' });
    const state = toastStore.getSnapshot();
    expect(state.toasts).toEqual([{ id: 1, severity: 'info', message: 'Indexing complete.' }]);
  });

  it('auto-dismisses info/success after 4s and warning/error after 8s', () => {
    // Auto-dismiss begins the exit fade at the timer; the fade (--dur-base,
    // 240ms here) then removes the toast.
    const fadeMs = 240;
    for (const severity of ['info', 'success'] as ToastSeverity[]) {
      toastStore.pushProtocol(1, { severity, message: 'x' });
      vi.advanceTimersByTime(TOAST_CALM_MS - 1);
      expect(toastStore.getSnapshot().toasts).toHaveLength(1);
      vi.advanceTimersByTime(1);
      expect(toastStore.getSnapshot().leaving).toEqual([1]);
      vi.advanceTimersByTime(fadeMs);
      expect(toastStore.getSnapshot().toasts).toHaveLength(0);
    }
    for (const severity of ['warning', 'error'] as ToastSeverity[]) {
      toastStore.pushProtocol(1, { severity, message: 'x' });
      vi.advanceTimersByTime(TOAST_CALM_MS);
      expect(toastStore.getSnapshot().toasts).toHaveLength(1);
      vi.advanceTimersByTime(TOAST_ATTENTION_MS - TOAST_CALM_MS);
      expect(toastStore.getSnapshot().leaving).toEqual([1]);
      vi.advanceTimersByTime(fadeMs);
      expect(toastStore.getSnapshot().toasts).toHaveLength(0);
    }
  });

  it('caps the stack at 3 visible — the oldest fades out', () => {
    toastStore.pushProtocol(1, { severity: 'info', message: 'one' });
    toastStore.pushProtocol(2, { severity: 'info', message: 'two' });
    toastStore.pushProtocol(3, { severity: 'info', message: 'three' });
    toastStore.pushProtocol(4, { severity: 'info', message: 'four' });
    const state = toastStore.getSnapshot();
    expect(state.toasts).toHaveLength(4); // the oldest is leaving, not gone
    expect(state.leaving).toEqual([1]);
    // The three still-visible toasts are the newest.
    expect(state.toasts.slice(1).map((toast) => toast.id)).toEqual([2, 3, 4]);
  });

  it('the × button dismisses immediately', () => {
    toastStore.pushProtocol(7, { severity: 'error', message: 'Save failed.' });
    toastStore.dismiss(7);
    const state = toastStore.getSnapshot();
    expect(state.leaving).toEqual([7]);
  });

  it('a duplicate protocol key (replayed event) is dropped, not stacked', () => {
    toastStore.pushProtocol(5, { severity: 'info', message: 'once' });
    toastStore.pushProtocol(5, { severity: 'info', message: 'once' });
    expect(toastStore.getSnapshot().toasts).toHaveLength(1);
  });

  it('local command-error toasts use negative ids', () => {
    const first = toastStore.pushLocal('error', 'dispatch failed');
    const second = toastStore.pushLocal('error', 'dispatch failed again');
    expect(first).toBeLessThan(0);
    expect(second).toBeLessThan(first);
    expect(toastStore.getSnapshot().toasts.map((toast) => toast.id)).toEqual([first, second]);
  });

  it('dismissTopmost removes the newest toast (the Esc path)', () => {
    toastStore.pushProtocol(1, { severity: 'info', message: 'one' });
    toastStore.pushProtocol(2, { severity: 'info', message: 'two' });
    toastStore.dismissTopmost();
    expect(toastStore.getSnapshot().leaving).toEqual([2]);
    expect(toastStore.getSnapshot().leaving).not.toContain(1);
  });

  it('clearAll drops transient toasts, stops timers, and preserves history', () => {
    toastStore.pushProtocol(1, { severity: 'error', message: 'stale epoch' });
    expect(toastStore.getSnapshot().history).toHaveLength(1);
    toastStore.clearAll();
    expect(toastStore.getSnapshot().toasts).toHaveLength(0);
    expect(toastStore.getSnapshot().leaving).toHaveLength(0);
    expect(toastStore.getSnapshot().history).toHaveLength(1);
    vi.advanceTimersByTime(TOAST_ATTENTION_MS * 2);
    expect(toastStore.getSnapshot().toasts).toHaveLength(0);
    expect(toastStore.getSnapshot().history).toHaveLength(1);
  });

  it('the auto-dismiss timer does not fire for a manually dismissed toast', () => {
    toastStore.pushProtocol(1, { severity: 'info', message: 'x' });
    toastStore.dismiss(1);
    const before = toastStore.getSnapshot().toasts.map((toast) => toast.id);
    vi.advanceTimersByTime(TOAST_CALM_MS * 2);
    // No double-removal side effects: the list is stable and empty.
    expect(toastStore.getSnapshot().toasts).toEqual([]);
    expect(before).toContain(1);
  });
});

describe('toast store notification history (Z5a)', () => {
  it('records protocol and local toasts as unread history entries', () => {
    toastStore.pushProtocol(41, { severity: 'success', message: 'Saved.' });
    const localId = toastStore.pushLocal('error', 'Dispatch failed.');

    const state = toastStore.getSnapshot();
    expect(state.history).toEqual([
      {
        id: localId,
        severity: 'error',
        message: 'Dispatch failed.',
        at: Date.now(),
        read: false,
      },
      {
        id: 41,
        severity: 'success',
        message: 'Saved.',
        at: Date.now(),
        read: false,
      },
    ]);
    expect(state.unreadCount).toBe(2);
  });

  it('caps history at 50 and keeps the newest first', () => {
    for (let sequence = 1; sequence <= 52; sequence += 1) {
      toastStore.pushProtocol(sequence, { severity: 'info', message: `n${sequence}` });
    }

    const history = toastStore.getSnapshot().history;
    expect(history).toHaveLength(NOTIFICATION_HISTORY_MAX);
    expect(history[0].id).toBe(52);
    expect(history[NOTIFICATION_HISTORY_MAX - 1].id).toBe(3);

    const stored = JSON.parse(localStorage.getItem(NOTIFICATION_HISTORY_KEY) ?? '[]') as unknown[];
    expect(stored).toHaveLength(NOTIFICATION_HISTORY_MAX);
  });

  it('persists history writes and read-state changes as JSON', () => {
    const setItem = vi.spyOn(window.localStorage, 'setItem');
    toastStore.pushProtocol(7, { severity: 'warning', message: 'Disk nearly full.' });

    expect(setItem).toHaveBeenCalledWith(NOTIFICATION_HISTORY_KEY, expect.any(String));
    const written = JSON.parse(localStorage.getItem(NOTIFICATION_HISTORY_KEY) ?? '[]');
    expect(written).toEqual([
      {
        id: 7,
        severity: 'warning',
        message: 'Disk nearly full.',
        at: Date.now(),
        read: false,
      },
    ]);

    toastStore.markRead(7);
    expect(JSON.parse(localStorage.getItem(NOTIFICATION_HISTORY_KEY) ?? '[]')[0].read).toBe(true);
  });

  it('marks one or all entries read without dismissing their transient toasts', () => {
    toastStore.pushProtocol(1, { severity: 'info', message: 'one' });
    toastStore.pushProtocol(2, { severity: 'error', message: 'two' });

    toastStore.markRead(1);
    let state = toastStore.getSnapshot();
    expect(state.history.find((entry) => entry.id === 1)?.read).toBe(true);
    expect(state.history.find((entry) => entry.id === 2)?.read).toBe(false);
    expect(state.unreadCount).toBe(1);
    expect(state.toasts).toHaveLength(2);

    toastStore.markAllRead();
    state = toastStore.getSnapshot();
    expect(state.history.every((entry) => entry.read)).toBe(true);
    expect(state.unreadCount).toBe(0);
    expect(state.toasts).toHaveLength(2);
  });

  it('clears history without clearing transient toasts', () => {
    toastStore.pushProtocol(9, { severity: 'info', message: 'keep the toast' });
    toastStore.clearHistory();

    const state = toastStore.getSnapshot();
    expect(state.history).toEqual([]);
    expect(state.unreadCount).toBe(0);
    expect(state.toasts).toHaveLength(1);
    expect(localStorage.getItem(NOTIFICATION_HISTORY_KEY)).toBe('[]');
  });

  it('dismisses transient toasts without deleting their history entry', () => {
    toastStore.pushProtocol(3, { severity: 'warning', message: 'transient only' });
    toastStore.dismissTransient(3);

    expect(toastStore.getSnapshot().leaving).toEqual([3]);
    expect(toastStore.getSnapshot().history).toHaveLength(1);
  });

  it('loads persisted history newest-first, capped at 50', async () => {
    const entries = Array.from({ length: 55 }, (_, index) => ({
      id: index + 1,
      severity: 'info',
      message: `loaded-${index + 1}`,
      at: 1_000 + index,
      read: false,
    }));
    localStorage.setItem(NOTIFICATION_HISTORY_KEY, JSON.stringify(entries));
    vi.resetModules();

    const { toastStore: reloaded } = await import('./toast-store');
    const history = reloaded.getSnapshot().history;
    expect(history).toHaveLength(NOTIFICATION_HISTORY_MAX);
    expect(history[0].id).toBe(55);
    expect(history[NOTIFICATION_HISTORY_MAX - 1].id).toBe(6);
    expect(reloaded.getSnapshot().unreadCount).toBe(NOTIFICATION_HISTORY_MAX);
  });

  it('tolerates malformed or non-array persisted history', async () => {
    localStorage.setItem(NOTIFICATION_HISTORY_KEY, '{not json');
    vi.resetModules();
    const malformed = await import('./toast-store');
    expect(malformed.toastStore.getSnapshot().history).toEqual([]);

    localStorage.setItem(NOTIFICATION_HISTORY_KEY, JSON.stringify({ history: [] }));
    vi.resetModules();
    const wrongShape = await import('./toast-store');
    expect(wrongShape.toastStore.getSnapshot().history).toEqual([]);
  });
});
