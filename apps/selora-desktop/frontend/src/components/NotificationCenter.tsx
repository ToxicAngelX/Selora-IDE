// Selora IDE — Phase Z5a notification center.
//
// The title-bar bell opens a history popover. History is owned by the toast
// store (bounded to 50, persisted); this component only presents it and
// delegates read/clear actions. Escape and pointer-down outside close the
// dialog and return focus to the bell (DESIGN §7).
import { useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { toastStore, type NotificationEntry } from '../features/toasts/toast-store';
import type { ToastSeverity } from '../bridge/bridge';

export const OPEN_NOTIFICATIONS_EVENT = 'selora:open-notifications';

const SEVERITY_LABEL: Record<ToastSeverity, string> = {
  info: 'Info',
  success: 'Success',
  warning: 'Warning',
  error: 'Error',
};

type InertedElement = {
  element: HTMLElement;
  hadInert: boolean;
  ariaHidden: string | null;
};

/**
 * `aria-modal` only tells assistive tech to ignore the background; it does not
 * stop keyboard or pointer interaction. On open, inert every element that is a
 * sibling of the notification panel's ancestor chain (the app content) and
 * restore the previous state on cleanup. The panel and the bell stay live.
 */
function inertBackground(panel: HTMLElement): () => void {
  const inerted: InertedElement[] = [];
  let node: HTMLElement | null = panel;

  while (node && node !== document.body && node.parentElement) {
    const parent: HTMLElement = node.parentElement;
    for (const child of Array.from(parent.children)) {
      if (child === node || !(child instanceof HTMLElement)) continue;
      if (child.tagName === 'SCRIPT' || child.tagName === 'STYLE' || child.tagName === 'LINK') {
        continue;
      }
      inerted.push({
        element: child,
        hadInert: child.hasAttribute('inert'),
        ariaHidden: child.getAttribute('aria-hidden'),
      });
      child.setAttribute('inert', '');
      child.setAttribute('aria-hidden', 'true');
    }
    node = parent;
  }

  return () => {
    for (const record of inerted) {
      if (!record.hadInert) record.element.removeAttribute('inert');
      if (record.ariaHidden === null) record.element.removeAttribute('aria-hidden');
      else record.element.setAttribute('aria-hidden', record.ariaHidden);
    }
  };
}

function BellIcon() {
  return (
    <svg
      className="notification-bell-icon"
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="square"
      strokeLinejoin="miter"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M4 11h8l-1.2-1.6V6.7a2.8 2.8 0 0 0-5.6 0v2.7L4 11z" />
      <path d="M6.5 12.5a1.5 1.5 0 0 0 3 0" />
    </svg>
  );
}

export function formatNotificationAge(at: number, now: number = Date.now()): string {
  const elapsed = Math.max(0, now - at);
  if (elapsed < 60_000) return 'just now';

  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 60) return `${minutes}m`;

  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;

  return `${Math.floor(hours / 24)}d`;
}

function NotificationRow({ entry, now }: { entry: NotificationEntry; now: number }) {
  const age = formatNotificationAge(entry.at, now);

  return (
    <li className="notification-list-item">
      <button
        type="button"
        className={`notification-item${entry.read ? '' : ' is-unread'}`}
        onClick={() => toastStore.markRead(entry.id)}
        title={entry.read ? SEVERITY_LABEL[entry.severity] : 'Mark as read'}
        aria-label={`${entry.message}, ${age}${entry.read ? '' : ', unread'}`}
      >
        <span
          className={`notification-dot notification-dot-${entry.severity}`}
          aria-hidden="true"
        />
        <span className="notification-item-message">{entry.message}</span>
        <time className="notification-age" dateTime={new Date(entry.at).toISOString()}>
          {age}
        </time>
      </button>
    </li>
  );
}

export function NotificationCenter() {
  const state = useSyncExternalStore(toastStore.subscribe, toastStore.getSnapshot);
  const [open, setOpen] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const bellRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const onOpen = () => setOpen(true);
    window.addEventListener(OPEN_NOTIFICATIONS_EVENT, onOpen);
    return () => window.removeEventListener(OPEN_NOTIFICATIONS_EVENT, onOpen);
  }, []);

  useEffect(() => {
    if (!open) return;

    setNow(Date.now());
    const ageTimer = window.setInterval(() => setNow(Date.now()), 30_000);

    // Move focus into the dialog without stealing it from the rest of the
    // app: the first enabled action (or the panel itself when the history is
    // empty) receives focus for screen-reader and keyboard users. Focus moves
    // first so inerting the background never hides a focused element.
    const panel = panelRef.current;
    const firstAction = panel?.querySelector<HTMLElement>('button:not([disabled])');
    (firstAction ?? panel)?.focus();

    // Keep every other app surface out of the tab order / hit-testing while
    // the modal popover is open.
    const restoreBackground = inertBackground(rootRef.current ?? panelRef.current ?? document.body);

    const onPointerDown = (event: MouseEvent) => {
      if (rootRef.current && event.target instanceof Node && !rootRef.current.contains(event.target)) {
        setOpen(false);
        bellRef.current?.focus();
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      setOpen(false);
      bellRef.current?.focus();
    };

    window.addEventListener('pointerdown', onPointerDown);
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.clearInterval(ageTimer);
      window.removeEventListener('pointerdown', onPointerDown);
      window.removeEventListener('keydown', onKeyDown);
      restoreBackground();
    };
  }, [open]);

  const trapFocus = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Tab') return;
    const focusables = Array.from(
      panelRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ) ?? [],
    ).filter((node) => node !== panelRef.current);
    if (focusables.length === 0) {
      event.preventDefault();
      panelRef.current?.focus();
      return;
    }
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (document.activeElement === panelRef.current) {
      if (event.shiftKey) {
        event.preventDefault();
        last.focus();
      }
      return;
    }
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const unreadLabel = state.unreadCount > 0 ? `${state.unreadCount} unread` : 'All read';

  return (
    <div ref={rootRef} className="notification-center">
      <button
        ref={bellRef}
        type="button"
        className={`notification-bell${open ? ' is-open' : ''}`}
        aria-label="Notifications"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? 'notifications-panel' : undefined}
        title={state.unreadCount > 0 ? `Notifications (${state.unreadCount} unread)` : 'Notifications'}
        onClick={() => setOpen((value) => !value)}
      >
        <BellIcon />
        {state.unreadCount > 0 ? (
          <span className="notification-badge" aria-hidden="true">
            {state.unreadCount > 9 ? '9+' : state.unreadCount}
          </span>
        ) : null}
      </button>

      {open ? (
        <div
          ref={panelRef}
          id="notifications-panel"
          className="notification-panel"
          role="dialog"
          aria-modal="true"
          aria-labelledby="notifications-panel-title"
          tabIndex={-1}
          onKeyDown={trapFocus}
        >
          <header className="notification-panel-header">
            <div className="notification-panel-heading">
              <h2 id="notifications-panel-title" className="notification-panel-title">
                Notifications
              </h2>
              <span className="notification-panel-count">{unreadLabel}</span>
            </div>
            <div className="notification-panel-actions">
              <button
                type="button"
                className="notification-action"
                onClick={() => toastStore.markAllRead()}
                disabled={state.unreadCount === 0}
                title="Mark all notifications as read"
              >
                Mark all read
              </button>
              <button
                type="button"
                className="notification-action"
                onClick={() => toastStore.clearHistory()}
                disabled={state.history.length === 0}
                title="Clear notification history"
              >
                Clear
              </button>
            </div>
          </header>

          {state.history.length === 0 ? (
            <div className="notification-empty">
              <p className="notification-empty-title">No notifications yet.</p>
              <p className="notification-empty-detail">Messages and alerts will appear here.</p>
            </div>
          ) : (
            <ul className="notification-list" aria-label="Notification history">
              {state.history.map((entry) => (
                <NotificationRow key={`${entry.id}:${entry.at}`} entry={entry} now={now} />
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </div>
  );
}
