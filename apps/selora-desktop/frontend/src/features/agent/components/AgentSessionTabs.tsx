// Selora IDE — Phase 9a Agent session tabs (ROADMAP §"Phase 9", task 9a).
//
// The tabs are a projection of `bootstrap.chat.sessions` (backend truth) and
// nothing else. New/select dispatch the real `NewSession` / `SwitchSession`
// protocol commands through `agentStore`; a rejected command surfaces the
// backend message in the tab strip and never creates a placeholder session.
//
// Keyboard follows the WAI-ARIA tabs pattern: roving tabindex, Left/Right,
// Home/End move focus and activate the backend session.
import React, { useRef } from 'react';
import { Plus } from '../../../components/icons';
import type { AgentSession } from '../types';

export type AgentSessionTabsProps = {
  sessions: AgentSession[];
  activeSessionId: string;
  busy?: boolean;
  error?: string | null;
  onNew: () => void;
  onSelect: (sessionId: string) => void;
};

export const AgentSessionTabs: React.FC<AgentSessionTabsProps> = ({
  sessions,
  activeSessionId,
  busy = false,
  error,
  onNew,
  onSelect,
}) => {
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const focusTab = (index: number) => {
    if (sessions.length === 0) return;
    const bounded = (index + sessions.length) % sessions.length;
    const target = sessions[bounded];
    if (!target) return;
    onSelect(target.id);
    tabRefs.current[bounded]?.focus();
  };

  return (
    <div className="agent-session-tabs-strip">
      <div className="agent-session-tabs" role="tablist" aria-label="Agent sessions">
        {sessions.length === 0 ? (
          <p className="agent-session-tabs-empty">No sessions yet</p>
        ) : (
          sessions.map((session, index) => {
            const selected = session.id === activeSessionId;
            return (
              <button
                key={session.id}
                ref={(element) => {
                  tabRefs.current[index] = element;
                }}
                type="button"
                role="tab"
                id={`agent-session-tab-${session.id}`}
                className={`agent-session-tab${selected ? ' is-active' : ''}`}
                aria-selected={selected}
                aria-controls="agent-session-panel"
                tabIndex={selected ? 0 : -1}
                title={session.id}
                onClick={() => onSelect(session.id)}
                onKeyDown={(event) => {
                  if (event.key === 'ArrowRight') {
                    event.preventDefault();
                    focusTab(index + 1);
                  } else if (event.key === 'ArrowLeft') {
                    event.preventDefault();
                    focusTab(index - 1);
                  } else if (event.key === 'Home') {
                    event.preventDefault();
                    focusTab(0);
                  } else if (event.key === 'End') {
                    event.preventDefault();
                    focusTab(sessions.length - 1);
                  }
                }}
              >
                <span
                  className={`agent-session-tab-dot${session.isRunning ? ' is-running' : ''}`}
                  aria-hidden="true"
                />
                <span className="agent-session-tab-title">{session.title}</span>
              </button>
            );
          })
        )}
      </div>

      <button
        type="button"
        className="agent-session-tab-add"
        onClick={onNew}
        disabled={busy}
        aria-busy={busy}
        title="New agent session"
        aria-label="New session"
      >
        <Plus size={12} aria-hidden="true" />
        <span>{busy ? 'Starting…' : 'New'}</span>
      </button>

      {error ? (
        <p className="agent-session-tabs-error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
};