// Selora IDE — Phase Z2 toast viewport (Elevated).
//
// The live toast stack (ARCHITECTURE §8.2, UI_SPEC §13): bottom-right above
// the status bar, 12px from the edges, 8px gaps, max 3 visible.
// Rich variants: info, success (functional green glow), warning (amber),
// error (red). Features slide-up entrance animation, dismiss "×" button,
// and auto-dismiss progress timer bar on the bottom edge.
import { useSyncExternalStore } from 'react';
import { toastDuration, toastStore } from '../features/toasts/toast-store';
import { agentStore } from '../features/agent/agent-store';
import { Icon, type IconName } from './Icon';

const SEVERITY_ICON: Record<string, IconName> = {
  info: 'toast-info',
  success: 'check',
  warning: 'toast-warning',
  error: 'toast-warning',
};

const SEVERITY_LABEL: Record<string, string> = {
  info: 'Info',
  success: 'Success',
  warning: 'Warning',
  error: 'Error',
};

export function ToastViewport() {
  const state = useSyncExternalStore(toastStore.subscribe, toastStore.getSnapshot);
  if (state.toasts.length === 0) return null;

  return (
    <div className="toast-viewport" role="region" aria-label="Live notifications">
      {state.toasts.map((toast) => {
        const leaving = state.leaving.includes(toast.id);
        const duration = toastDuration(toast.severity);

        return (
          <div
            key={toast.id}
            className={`toast toast-${toast.severity}${leaving ? ' is-leaving' : ''}`}
            role="status"
          >
            <div className="toast-body">
              <span className={`toast-severity toast-severity-${toast.severity}`} aria-hidden="true">
                <Icon name={SEVERITY_ICON[toast.severity] ?? 'toast-info'} />
              </span>
              <div className="toast-text-area">
                <span className="toast-type-pill" aria-hidden="true">
                  {SEVERITY_LABEL[toast.severity] ?? 'Notification'}
                </span>
                <p className="toast-message">{toast.message}</p>
              </div>
              {toast.action ? (
                <button
                  type="button"
                  className="toast-open"
                  onClick={() => {
                    if (toast.action?.kind === 'subagent') {
                      agentStore.revealSubagent(toast.action.sessionId, toast.action.childId);
                    }
                    toastStore.dismissTransient(toast.id);
                  }}
                >
                  Open
                </button>
              ) : null}
              <button
                type="button"
                className="toast-dismiss"
                aria-label={`Dismiss notification: ${toast.message}`}
                onClick={() => toastStore.dismissTransient(toast.id)}
              >
                <Icon name="close" />
              </button>
            </div>
            {/* Auto-dismiss progress timer bar on bottom edge */}
            <div className="toast-progress-track" aria-hidden="true">
              <div
                className={`toast-progress-bar toast-progress-${toast.severity}${leaving ? ' is-leaving' : ''}`}
                style={{ animationDuration: `${duration}ms` }}
              />
            </div>
          </div>
        );
      })}
      <button
        type="button"
        className="toast-view-all"
        title="Open notification center"
        onClick={() => {
          if (typeof window !== 'undefined') {
            window.dispatchEvent(new CustomEvent('selora:open-notifications'));
          }
        }}
      >
        View all
      </button>
    </div>
  );
}
