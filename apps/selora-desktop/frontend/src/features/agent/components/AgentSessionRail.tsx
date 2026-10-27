// Selora IDE — ZCode Multi-Session Drawer (ROADMAP.md Phase 9, ZCode parity)
//
// Chronologically clustered session history, new session trigger,
// and session branch & token badges.
import React from 'react';
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
  return (
    <aside className="agent-session-rail" aria-label="Agent Session History">
      <div className="session-rail-header">
        <span className="session-rail-title">Sessions</span>
        <button
          type="button"
          className="btn-new-session"
          onClick={() => {
            agentStore.createSession();
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
    </aside>
  );
};
