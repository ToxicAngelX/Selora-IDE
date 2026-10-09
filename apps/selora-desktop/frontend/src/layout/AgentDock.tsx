// Selora IDE — Phase Z2 agent dock chrome (DESIGN §3.5, ARCHITECTURE §8.1).
//
// The Z2 surface is the dock's *chrome* only: the 40px header with the
// static 8px presence dot in the 20px slot (ThinkingOrb is Z3 — base.css
// has no orb by design), the empty state ("Ask anything", the italic hint,
// the status line, three disabled suggestion chips), and the composer
// card's visual geometry. Everything renders inert — no key, chip, or
// Enter press dispatches a command until Z3.
import { EmptyState } from '../components/EmptyState';

const SUGGESTIONS = ['Explain this file', 'Find a bug in the selection', 'Summarize the workspace'];

export function AgentDock() {
  return (
    <div className="agent-dock">
      <header className="agent-header">
        <span className="agent-presence-slot" aria-hidden="true">
          <span className="agent-presence-dot" />
        </span>
        <span className="agent-session-label">Agent</span>
      </header>
      <div className="agent-body">
        <EmptyState
          heading="Ask anything"
          hint={
            <>
              <em>Ask about the code, or tell Selora what to change.</em>
              <span className="agent-status-line">Agent conversations arrive in a later phase.</span>
            </>
          }
        >
          <div className="agent-suggestions" aria-label="Suggested prompts">
            {SUGGESTIONS.map((suggestion) => (
              <button
                key={suggestion}
                type="button"
                className="agent-chip"
                disabled
                title="The composer arrives in a later slice"
              >
                {suggestion}
              </button>
            ))}
          </div>
        </EmptyState>
      </div>
      <div className="agent-composer" aria-hidden="true">
        <p className="agent-composer-placeholder">Ask anything…</p>
      </div>
    </div>
  );
}
