// Selora IDE — Phase Z2 status bar (DESIGN §3.7, ARCHITECTURE §8.1).
//
// 24px, 1px top hairline, --type-xs. Z2 zones: left = workspace display
// name; center = empty (editor metadata lands with the editor slice);
// right = real bootstrap data — the core epoch and the connection status
// in UI_SPEC §1.2's form, with the word "connect" inert in Z2 (settings
// editing is a later slice). The ●/○ glyphs are text glyphs in the mono
// family (the UI_SPEC §1.2 precedent, not emoji); numeric segments ride
// --font-mono.
import type { BootstrapState, ConnectionStatus } from '../bridge/bridge';

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
        <span className="statusbar-dot" aria-hidden="true" />
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
  const epoch = bootstrap.core_instance_id.slice(0, 8);
  return (
    <footer className="statusbar">
      <span className="statusbar-zone statusbar-workspace">{bootstrap.workspace.display_name}</span>
      <span className="statusbar-zone statusbar-center" aria-hidden="true" />
      <span className="statusbar-zone statusbar-right">
        <span className="statusbar-epoch" title={`Core instance ${bootstrap.core_instance_id}`}>
          core {epoch}
        </span>
        <ConnectionStatusView status={bootstrap.connection} onOpenConnect={onOpenConnect} />
      </span>
    </footer>
  );
}
