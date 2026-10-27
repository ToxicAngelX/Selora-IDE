// Selora IDE — Phase 9a session history rail.
//
// The rail mirrors the same backend session state as the workbench tabs. New
// and select dispatch the real protocol commands; while the backend does not
// support them, the command's message is shown verbatim and no session is
// created locally. Token/cost figures are omitted because the current
// `SessionDto` does not report them (never show a made-up 0.0k).
import React, { useSyncExternalStore } from 'react';
import { Plus } from '../../../components/icons';
import type { AgentSession } from '../types';
import { agentStore } from '../agent-store';

export type AgentSessionRailProps = {
  sessions: AgentSession[];
  activeSessionId: string;
  onClose?: () => void;
};

export const AgentSessionRail: React.FC<AgentSessionRailProps> = ({
  sessions,
  activeSessionId,
  onClose,
}) => {
  const state = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  return (
    <aside className="agent-session-rail" aria-label="Agent Session History">
      <div className="session-rail-header">
        <span className="session-rail-title">Sessions</span>
        <button
          type="button"
          className="btn-new-session"
          onClick={() => {
            void agentStore.requestNewSession().then((ok) => {
              if (ok) onClose?.();
            });
          }}
          disabled={state.sessionBusy}
          aria-busy={state.sessionBusy}
          title="New agent session"
        >
          <Plus size={12} aria-hidden="true" />
          <span>{state.sessionBusy ? 'Starting…' : 'New'}</span>
        </button>
      </div>

      {state.sessionError ? (
        <p className="agent-session-rail-error" role="alert">
          {state.sessionError}
        </p>
      ) : null}

      {sessions.length === 0 ? (
        <p className="agent-session-rail-empty">No sessions yet</p>
      ) : (
        <div className="session-list">
          {sessions.map((session) => {
            const isActive = session.id === activeSessionId;
            const activateSession = () => {
              void agentStore.requestSwitchSession(session.id).then((ok) => {
                if (ok) onClose?.();
              });
            };

            return (
              <button
                key={session.id}
                type="button"
                className={`session-item ${isActive ? 'is-active' : ''}`}
                onClick={activateSession}
                aria-current={isActive ? 'true' : undefined}
                title={session.title}
              >
                <div className="session-item-row">
                  <span className="session-indicator" />
                  <span className="session-title">{session.title}</span>
                </div>
                <div className="session-item-meta">
                  <span className="session-id">{session.id}</span>
                  {session.isRunning ? (
                    <span className="session-running-tag">Running</span>
                  ) : null}
                </div>
              </button>
            );
          })}
        </div>
      )}
    </aside>
  );
};