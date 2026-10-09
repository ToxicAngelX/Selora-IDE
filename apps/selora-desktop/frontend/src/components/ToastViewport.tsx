// Selora IDE — Phase Z2 toast viewport.
//
// The live toast stack (ARCHITECTURE §8.2, UI_SPEC §13): bottom-right above
// the status bar, 12px from the edges, 8px gaps, max 3 visible, --ink-mid
// fill, 1px --ink-hi border, --radius-1, 12px padding, fade-only
// enter/exit at --dur-base. Severity shows as a small glyph in its
// semantic color (UI_SPEC §13's 'i'/'!' drawn in the §10 stroke system;
// success uses the check). Click × dismisses.
import { useSyncExternalStore } from 'react';
import { toastStore } from '../features/toasts/toast-store';
import { Icon, type IconName } from './Icon';

const SEVERITY_ICON: Record<string, IconName> = {
  info: 'toast-info',
  success: 'check',
  warning: 'toast-warning',
  error: 'toast-warning',
};

export function ToastViewport() {
  const state = useSyncExternalStore(toastStore.subscribe, toastStore.getSnapshot);
  if (state.toasts.length === 0) return null;
  return (
    <div className="toast-viewport" role="region" aria-label="Notifications" aria-live="polite">
      {state.toasts.map((toast) => {
        const leaving = state.leaving.includes(toast.id);
        return (
          <div
            key={toast.id}
            className={`toast toast-${toast.severity}${leaving ? ' is-leaving' : ''}`}
            role="status"
          >
            <span className={`toast-severity toast-severity-${toast.severity}`} aria-hidden="true">
              <Icon name={SEVERITY_ICON[toast.severity] ?? 'toast-info'} />
            </span>
            <p className="toast-message">{toast.message}</p>
            <button
              type="button"
              className="toast-dismiss"
              aria-label={`Dismiss notification: ${toast.message}`}
              onClick={() => toastStore.dismiss(toast.id)}
            >
              <Icon name="close" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
