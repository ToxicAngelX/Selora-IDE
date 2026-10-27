// Selora IDE — Output dock placeholder (Phase Z4)
//
// The protocol does not expose a task/output stream yet, so this panel never
// fabricates log rows. Real task output arrives with the terminal slice.
import { Icon } from '../../components/Icon';

export function OutputPanel() {
  return (
    <div className="dock-panel output-panel" data-testid="output-panel">
      <div className="dock-toolbar output-toolbar">
        <div className="dock-toolbar-left">
          <span className="output-title">Output</span>
        </div>
        <div className="dock-toolbar-right">
          <span className="dock-placeholder-badge">Not wired yet</span>
        </div>
      </div>

      <div
        className="output-logs-container wrap-lines"
        role="log"
        aria-label="Output"
        aria-live="polite"
      >
        <div className="output-empty-state" role="status">
          <span className="output-empty-icon" aria-hidden="true">
            <Icon name="terminal" size={20} />
          </span>
          <span>No output yet — tasks land with the terminal slice.</span>
        </div>
      </div>
    </div>
  );
}

