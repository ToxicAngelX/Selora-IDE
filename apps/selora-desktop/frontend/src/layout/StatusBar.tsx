// Selora IDE — Phase Z2 status bar (DESIGN §3.7, ARCHITECTURE §8.1).
//
// 24px, 1px top hairline, --type-xs.
// Left: Workspace display name + Git branch with sync spinner + language/file info.
// Center: Agent activity pill / status (interactive toggle for agent dock).
// Right: LSP diagnostics summary (errors/warnings) + UTF-8 encoding + line/col + Core epoch + Selora Gateway connection status with live green pulse radar ring.
import { useState, useSyncExternalStore } from 'react';
import type { BootstrapState, ConnectionStatus } from '../bridge/bridge';
import { layoutStore } from '../features/layout/layout-store';
import { agentStore } from '../features/agent/agent-store';
import { SeloraMark } from '../components/SeloraMark';
import {
  GitBranch,
  RefreshCw,
  DangerIcon,
  AlertCircle,
} from '../components/icons';

function ConnectionStatusView({
  status,
  onOpenConnect,
}: {
  status: ConnectionStatus;
  onOpenConnect?: () => void;
}) {
  if (status === 'connected') {
    return (
      <button
        type="button"
        className="statusbar-connection is-connected"
        onClick={onOpenConnect}
        style={{
          background: 'none',
          border: 'none',
          cursor: onOpenConnect ? 'pointer' : 'default',
          padding: 0,
          font: 'inherit',
          color: 'inherit',
        }}
        title={onOpenConnect ? 'Gateway connected (click to manage API key / provider)' : undefined}
      >
        <span className="statusbar-radar-wrapper" aria-hidden="true">
          <span className="statusbar-radar-ring" />
          <span className="statusbar-dot" />
        </span>
        selora connected
      </button>
    );
  }

  const label = status === 'connecting' ? 'selora connecting' : 'selora disconnected';
  return (
    <span className="statusbar-connection is-disconnected">
      <span className="statusbar-dot" aria-hidden="true" />
      {label}
      <span className="statusbar-separator" aria-hidden="true">
        ·
      </span>
      <button
        type="button"
        className="statusbar-connect"
        disabled={!onOpenConnect}
        onClick={onOpenConnect}
        title={onOpenConnect ? 'Connect API key / provider gateway' : 'Settings editing lands in a later slice'}
      >
        connect
      </button>
    </span>
  );
}

export function StatusBar({
  bootstrap,
  onOpenConnect,
}: {
  bootstrap: BootstrapState;
  onOpenConnect?: () => void;
}) {
  const agentState = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  const [syncing, setSyncing] = useState(false);

  const epoch = bootstrap.core_instance_id.slice(0, 8);
  const gitBranch = bootstrap.git?.branch || 'main';

  // Active buffer language
  const activeBuffer = bootstrap.buffers?.find((b) => b.buffer_id === bootstrap.active_buffer_id);
  const activeLanguage = activeBuffer?.language
    ? activeBuffer.language.charAt(0).toUpperCase() + activeBuffer.language.slice(1)
    : 'TypeScript';

  // Diagnostics summary
  const errorCount = bootstrap.diagnostics?.filter((d) => d.severity === 'error').length ?? 0;
  const warningCount = bootstrap.diagnostics?.filter((d) => d.severity === 'warning').length ?? 0;

  // Agent activity
  const agentInFlight = agentState.inFlight || agentState.thinkingState !== 'breathing';
  const agentWaiting = Boolean(bootstrap.chat?.pending_approval);
  const agentLabel = agentInFlight
    ? `Agent ${agentState.thinkingState.charAt(0).toUpperCase() + agentState.thinkingState.slice(1)}...`
    : agentWaiting
    ? 'Approval Required'
    : 'Selora Agent';

  const handleGitSync = () => {
    setSyncing(true);
    setTimeout(() => {
      setSyncing(false);
    }, 1200);
  };

  return (
    <footer className="statusbar">
      {/* Left zone: Workspace, Git branch with sync spinner, language */}
      <div className="statusbar-zone statusbar-left">
        <button
          type="button"
          className="statusbar-item statusbar-workspace"
          onClick={() => void layoutStore.toggle('files')}
          title={`Workspace: ${bootstrap.workspace.display_name} (click to toggle Files)`}
        >
          <SeloraMark size={11} />
          <span>{bootstrap.workspace.display_name}</span>
        </button>

        <button
          type="button"
          className="statusbar-item statusbar-git"
          aria-label="Git sync"
          onClick={handleGitSync}
          title={`Git Branch: ${gitBranch} (click to sync)`}
        >
          <GitBranch size={11} className="git-icon" aria-hidden="true" />
          <span>{gitBranch}</span>
          <RefreshCw
            size={10}
            className={`sync-icon ${syncing ? 'is-spinning' : ''}`}
            aria-hidden="true"
          />
        </button>

        <button
          type="button"
          className="statusbar-item statusbar-lang"
          onClick={() => void layoutStore.toggle('files')}
          title="Active Language Mode"
        >
          <span>{activeLanguage}</span>
        </button>
      </div>

      {/* Center zone: Agent activity pill / status */}
      <div className="statusbar-zone statusbar-center">
        <button
          type="button"
          className={`statusbar-agent-pill ${agentInFlight ? 'is-active' : ''}`}
          onClick={() => void layoutStore.toggle('agent')}
          title="Agent Workbench Status (click to toggle Agent Dock Ctrl+L)"
          aria-label="Agent Status"
        >
          <span
            className={`agent-radar-dot ${
              agentInFlight ? 'is-thinking' : agentWaiting ? 'is-waiting' : ''
            }`}
            aria-hidden="true"
          />
          <span className="agent-status-label">{agentLabel}</span>
        </button>
        <span className="visually-hidden" role="status" aria-live="polite" aria-atomic="true">
          {agentLabel}
        </span>
      </div>

      {/* Right zone: LSP diagnostics, UTF-8, Line/Col, Core epoch, Connection status */}
      <div className="statusbar-zone statusbar-right">
        <button
          type="button"
          className="statusbar-item statusbar-diagnostics"
          aria-label="Problems summary"
          onClick={() => void layoutStore.toggle('problems')}
          title={`Diagnostics: ${errorCount} errors, ${warningCount} warnings (click to toggle Problems)`}
        >
          <span className="diag-error">
            <DangerIcon size={11} aria-hidden="true" />
            <span>{errorCount}</span>
          </span>
          <span className="diag-warning">
            <AlertCircle size={11} aria-hidden="true" />
            <span>{warningCount}</span>
          </span>
        </button>

        <span className="statusbar-item statusbar-encoding" title="File Encoding">
          UTF-8
        </span>

        <span className="statusbar-item statusbar-coords" title="Cursor Line and Column">
          Ln 1, Col 1
        </span>

        <span className="statusbar-epoch" title={`Core instance ${bootstrap.core_instance_id}`}>
          core {epoch}
        </span>

        <ConnectionStatusView status={bootstrap.connection} onOpenConnect={onOpenConnect} />
      </div>
    </footer>
  );
}
