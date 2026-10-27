// Notification-center tests (Z5a): bell/unread badge, empty state, history
// ordering and relative age, mark-all, clear, Escape focus return, outside
// click, and the toast viewport's open event.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render } from '@testing-library/react';
import {
  NotificationCenter,
  OPEN_NOTIFICATIONS_EVENT,
  formatNotificationAge,
} from './NotificationCenter';
import { toastStore } from '../features/toasts/toast-store';
import { agentStore } from '../features/agent/agent-store';

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-01-01T12:00:00.000Z'));
  localStorage.clear();
  toastStore.clearHistory();
  toastStore.clearAll();
});

afterEach(() => {
  toastStore.clearAll();
  localStorage.clear();
  vi.useRealTimers();
});

describe('NotificationCenter', () => {
  it('renders an accessible bell and empty dialog with disabled actions', () => {
    const { getByRole, getByText } = render(<NotificationCenter />);
    const bell = getByRole('button', { name: 'Notifications' });

    expect(bell.getAttribute('aria-expanded')).toBe('false');
    expect(bell.getAttribute('title')).toBe('Notifications');
    fireEvent.click(bell);

    expect(bell.getAttribute('aria-expanded')).toBe('true');
    expect(getByRole('dialog', { name: 'Notifications' })).toBeTruthy();
    expect(getByText('No notifications yet.')).toBeTruthy();
    expect((getByRole('button', { name: 'Mark all read' }) as HTMLButtonElement).disabled).toBe(true);
    expect((getByRole('button', { name: 'Clear' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('opens a labelled modal dialog and sends focus into the panel', () => {
    const { getByRole } = render(<NotificationCenter />);
    const bell = getByRole('button', { name: 'Notifications' });
    bell.focus();
    fireEvent.click(bell);

    const dialog = getByRole('dialog', { name: 'Notifications' });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.getAttribute('aria-labelledby')).toBe('notifications-panel-title');
    expect(dialog.contains(document.activeElement)).toBe(true);
  });

  it('traps Tab inside the dialog', () => {
    act(() => {
      toastStore.pushLocal('info', 'trap me');
    });
    const { getByRole } = render(<NotificationCenter />);
    fireEvent.click(getByRole('button', { name: 'Notifications' }));

    const first = getByRole('button', { name: 'Mark all read' });
    const row = getByRole('button', { name: /trap me/ });
    first.focus();
    fireEvent.keyDown(first, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(row);
    fireEvent.keyDown(row, { key: 'Tab' });
    expect(document.activeElement).toBe(first);
  });

  it('shows the unread badge capped at 9+ and renders newest first', () => {
    const { container, getByRole, getByText } = render(<NotificationCenter />);
    act(() => {
      for (let index = 1; index <= 12; index += 1) {
        toastStore.pushLocal('info', `note-${index}`);
      }
    });

    const bell = getByRole('button', { name: 'Notifications' });
    expect(getByText('9+')).toBeTruthy();
    expect(bell.getAttribute('title')).toBe('Notifications (12 unread)');

    fireEvent.click(bell);
    const messages = Array.from(container.querySelectorAll('.notification-item-message')).map(
      (node) => node.textContent,
    );
    expect(messages.slice(0, 3)).toEqual(['note-12', 'note-11', 'note-10']);
  });

  it('renders just-now/minute/hour/day relative ages', () => {
    toastStore.pushProtocol(1, { severity: 'info', message: 'days' });
    vi.advanceTimersByTime(3 * DAY - 2 * HOUR);
    toastStore.pushProtocol(2, { severity: 'warning', message: 'hours' });
    vi.advanceTimersByTime(2 * HOUR - 3 * MINUTE);
    toastStore.pushProtocol(3, { severity: 'success', message: 'minutes' });
    vi.advanceTimersByTime(3 * MINUTE);
    toastStore.pushProtocol(4, { severity: 'error', message: 'now' });

    const { getByRole, getByText } = render(<NotificationCenter />);
    fireEvent.click(getByRole('button', { name: 'Notifications' }));

    expect(getByText('now').closest('.notification-item')?.textContent).toContain('just now');
    expect(getByText('minutes').closest('.notification-item')?.textContent).toContain('3m');
    expect(getByText('hours').closest('.notification-item')?.textContent).toContain('2h');
    expect(getByText('days').closest('.notification-item')?.textContent).toContain('3d');
  });

  it('marks all history read and clears the unread badge', () => {
    const { container, getByRole, queryByText } = render(<NotificationCenter />);
    act(() => {
      toastStore.pushLocal('info', 'one');
      toastStore.pushLocal('error', 'two');
    });
    fireEvent.click(getByRole('button', { name: 'Notifications' }));

    expect(container.querySelector('.notification-badge')?.textContent).toBe('2');
    fireEvent.click(getByRole('button', { name: 'Mark all read' }));

    expect(container.querySelector('.notification-badge')).toBeNull();
    expect(queryByText('2')).toBeNull();
    expect(toastStore.getSnapshot().unreadCount).toBe(0);
    expect(toastStore.getSnapshot().history.every((entry) => entry.read)).toBe(true);
    expect(toastStore.getSnapshot().toasts).toHaveLength(2);
  });

  it('clears history but leaves transient toasts alone', () => {
    const { getByRole, getByText } = render(<NotificationCenter />);
    act(() => {
      toastStore.pushLocal('warning', 'clear me');
    });
    fireEvent.click(getByRole('button', { name: 'Notifications' }));
    fireEvent.click(getByRole('button', { name: 'Clear' }));

    expect(getByText('No notifications yet.')).toBeTruthy();
    expect(toastStore.getSnapshot().history).toEqual([]);
    expect(toastStore.getSnapshot().toasts).toHaveLength(1);
  });

  it('closes on Escape and returns focus to the bell', () => {
    const { getByRole, queryByRole } = render(<NotificationCenter />);
    const bell = getByRole('button', { name: 'Notifications' });
    bell.focus();
    fireEvent.click(bell);

    expect(getByRole('dialog', { name: 'Notifications' })).toBeTruthy();
    fireEvent.keyDown(document, { key: 'Escape' });

    expect(queryByRole('dialog', { name: 'Notifications' })).toBeNull();
    expect(document.activeElement).toBe(bell);
  });

  it('marks the background inert while the dialog is open and restores it on close', () => {
    const { getByRole } = render(
      <>
        <div data-testid="app-content">
          <button type="button">Background action</button>
        </div>
        <NotificationCenter />
      </>,
    );
    const background = getByRole('button', { name: 'Background action' }).parentElement!;

    expect(background.hasAttribute('inert')).toBe(false);

    fireEvent.click(getByRole('button', { name: 'Notifications' }));

    expect(getByRole('dialog', { name: 'Notifications' })).toBeTruthy();
    expect(background.hasAttribute('inert')).toBe(true);
    expect(background.getAttribute('aria-hidden')).toBe('true');
    expect(background.contains(document.activeElement)).toBe(false);

    fireEvent.keyDown(document, { key: 'Escape' });

    expect(background.hasAttribute('inert')).toBe(false);
    expect(background.hasAttribute('aria-hidden')).toBe(false);
  });

  it('closes on outside pointer-down and returns focus to the bell', () => {
    const { getByRole, queryByRole } = render(<NotificationCenter />);
    const bell = getByRole('button', { name: 'Notifications' });
    fireEvent.click(bell);

    fireEvent.pointerDown(document.body);

    expect(queryByRole('dialog', { name: 'Notifications' })).toBeNull();
    expect(document.activeElement).toBe(bell);
  });

  it('opens when the toast viewport dispatches the notification event', () => {
    const { getByRole, queryByRole } = render(<NotificationCenter />);
    expect(queryByRole('dialog', { name: 'Notifications' })).toBeNull();

    act(() => {
      window.dispatchEvent(new CustomEvent(OPEN_NOTIFICATIONS_EVENT));
    });

    expect(getByRole('dialog', { name: 'Notifications' })).toBeTruthy();
  });

  it('formats relative-age boundaries', () => {
    expect(formatNotificationAge(1_000, 1_000 + 59_999)).toBe('just now');
    expect(formatNotificationAge(1_000, 1_000 + MINUTE)).toBe('1m');
    expect(formatNotificationAge(1_000, 1_000 + HOUR)).toBe('1h');
    expect(formatNotificationAge(1_000, 1_000 + DAY)).toBe('1d');
  });

  it('opens and selects the parent session when a subagent completion is clicked', () => {
    const reveal = vi.spyOn(agentStore, 'revealSubagent').mockResolvedValue(true);
    act(() => {
      toastStore.pushSubagentCompletion(3001, {
        childId: 'child-notification-1',
        sessionId: 'sess-parent-1',
        ok: true,
        message: 'Subagent finished: review complete',
      });
    });

    const { getByRole, queryByRole } = render(<NotificationCenter />);
    fireEvent.click(getByRole('button', { name: 'Notifications' }));
    const row = getByRole('button', { name: /review complete/ });
    expect(row.getAttribute('aria-label')).toContain('open result');

    fireEvent.click(row);

    expect(reveal).toHaveBeenCalledWith('sess-parent-1', 'child-notification-1');
    expect(queryByRole('dialog', { name: 'Notifications' })).toBeNull();
    reveal.mockRestore();
  });
});
