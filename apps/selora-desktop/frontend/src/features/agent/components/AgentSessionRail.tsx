// Selora IDE — Phase 9a session history rail.
//
// The rail mirrors the same backend session state as the workbench tabs. New
// and select dispatch the real protocol commands; while the backend does not
// support them, the command's message is shown verbatim and no session is
// created locally. Token/cost figures are omitted because the current
// `SessionDto` does not report them (never show a made-up 0.0k).
import React, { useSyncExternalStore } from 'react';
import { Plus } from '../../../components/icons';
import type { AgentSession, ResumableAgentSession } from '../types';
import { agentStore } from '../agent-store';

export type AgentSessionRailProps = {
  sessions: AgentSession[];
  activeSessionId: string;
  /** Phase 9d: durable sessions from Rust not yet open in this process. */
  resumableSessions?: ResumableAgentSession[];
  onClose?: () => void;
};

export const AgentSessionRail: React.FC<AgentSessionRailProps> = ({
  sessions,
  activeSessionId,
  resumableSessions = [],
  onClose,
}) => {
  const state = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  const closedIds = new Set(sessions.map((session) => session.id));
  const resumable = resumableSessions.filter((session) => !closedIds.has(session.id));
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
          style={{ display: 'inline-flex', alignItems: 'center', gap: '3px' }}
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

      {sessions.length === 0 && <p className="agent-session-rail-empty">No sessions yet</p>}
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
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  activateSession();
                }
              }}
              aria-current={isActive ? 'true' : undefined}
              style={{ textAlign: 'left' }}
            >
              <div className="session-item-row">
                <span className="session-indicator" />
                <span className="session-title" title={session.title}>
                  {session.title}
                </span>
              </div>
              <div className="session-item-meta">
                {session.branchName && (
                  <span className="session-branch-tag">
                    {session.branchName}
                  </span>
                )}
                <span className="session-id">{session.id}</span>
                {session.isRunning && <span className="session-running-tag">Running</span>}
              </div>
            </button>
          );
        })}
      </div>

      {resumable.length > 0 && (
        <div className="session-list" role="group" aria-label="Resume session">
          <div className="session-rail-subtitle">Resume session</div>
          {resumable.map((session) => {
            const resume = () => {
              agentStore.resumeSession(session.id);
              onClose?.();
            };
            return (
              <button
                key={session.id}
                type="button"
                className="session-item session-item-resumable"
                onClick={resume}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    resume();
                  }
                }}
                title={session.pending ? session.pending.summary : session.title}
              >
                <div className="session-item-row">
                  <span className="session-indicator" />
                  <span className="session-title">{session.title}</span>
                </div>
                <div className="session-item-meta">
                  <span className="session-tokens">
                    {session.messageCount} messages · {session.mode}
                  </span>
                  {session.pending && (
                    <span className="session-pending-badge">{session.pending.kind}</span>
                  )}
                  {session.compacted && (
                    <span className="session-compacted-badge">compacted</span>
                  )}
                </div>
              </button>
            );
          })}
        </div>
      )}
    </aside>
  );
};
