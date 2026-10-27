// Selora IDE — Terminal dock placeholder (Phase Z4)
//
// No shell bridge exists in this slice. Keep the surface explicitly labeled as
// a placeholder instead of rendering simulated terminals or commands.
import { Icon } from '../../components/Icon';

export function TerminalPanel() {
  return (
    <div className="dock-panel terminal-panel" data-testid="terminal-panel">
      <div className="terminal-tabstrip">
        <div className="terminal-tabs">
          <span className="terminal-placeholder-label">
            <Icon name="terminal" size={12} />
            <span>Terminal</span>
          </span>
        </div>
        <div className="dock-toolbar-right">
          <span className="dock-placeholder-badge">Not wired yet</span>
        </div>
      </div>

      <div className="terminal-placeholder-body" role="status">
        <div className="terminal-notice-banner">
          <Icon name="terminal" size={12} />
          <span>The integrated shell arrives with the terminal slice.</span>
        </div>
        <div className="terminal-placeholder-copy">
          <h3>No terminal session is running</h3>
          <p>Commands and task output will appear here once the shell bridge lands.</p>
        </div>
      </div>
    </div>
  );
}

