// Toast-store tests (ARCHITECTURE §10): cap 3, the 4s/8s auto-dismiss
// timers, manual × dismissal, duplicate protocol keys, and clearAll.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TOAST_ATTENTION_MS, TOAST_CALM_MS, toastStore } from './toast-store';
import type { ToastSeverity } from '../../bridge/bridge';

beforeEach(() => {
  vi.useFakeTimers();
  // The exit fade reads --dur-base from the token layer; happy-dom has no
  // stylesheet, so provide it the way the real app does (inline custom
  // properties resolve through getComputedStyle — verified by probe).
  document.documentElement.style.setProperty('--dur-base', '240ms');
  toastStore.clearAll();
});

afterEach(() => {
  toastStore.clearAll();
  document.documentElement.style.removeProperty('--dur-base');
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

  it('clearAll drops everything and stops timers (the rebootstrap path)', () => {
    toastStore.pushProtocol(1, { severity: 'error', message: 'stale epoch' });
    toastStore.clearAll();
    expect(toastStore.getSnapshot().toasts).toHaveLength(0);
    expect(toastStore.getSnapshot().leaving).toHaveLength(0);
    vi.advanceTimersByTime(TOAST_ATTENTION_MS * 2);
    expect(toastStore.getSnapshot().toasts).toHaveLength(0);
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
