// Selora IDE — ZCode Multi-Session Drawer (ROADMAP.md Phase 9, ZCode parity)
//
// Chronologically clustered session history, new session trigger,
// and session branch & token badges.
import React from 'react';
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
            agentStore.newDurableSession();
            onClose?.();
          }}
          title="New agent session (Ctrl+N)"
          style={{ display: 'inline-flex', alignItems: 'center', gap: '3px' }}
        >
          <Plus size={12} />
          <span>New</span>
        </button>
      </div>

      <div className="session-list">
        {sessions.map((session) => {
          const isActive = session.id === activeSessionId;
          const activateSession = () => {
            agentStore.switchSession(session.id);
            onClose?.();
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
                <span className="session-tokens">
                  {(session.tokenCount / 1000).toFixed(1)}k tokens
                </span>
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
