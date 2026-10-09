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

function ConnectionStatusView({ status }: { status: ConnectionStatus }) {
  if (status === 'connected') {
    return (
      <span className="statusbar-connection is-connected">
        <span className="statusbar-dot" aria-hidden="true">
          ●
        </span>
        selora connected
      </span>
    );
  }
  const label = status === 'connecting' ? 'selora connecting' : 'selora disconnected';
  return (
    <span className="statusbar-connection is-disconnected">
      <span className="statusbar-dot" aria-hidden="true">
        ○
      </span>
      {label}
      <span className="statusbar-separator" aria-hidden="true">
        ·
      </span>
      <button type="button" className="statusbar-connect" disabled title="Settings editing lands in a later slice">
        connect
      </button>
    </span>
  );
}

export function StatusBar({ bootstrap }: { bootstrap: BootstrapState }) {
  const epoch = bootstrap.core_instance_id.slice(0, 8);
  return (
    <footer className="statusbar">
      <span className="statusbar-zone statusbar-workspace">{bootstrap.workspace.display_name}</span>
      <span className="statusbar-zone statusbar-center" aria-hidden="true" />
      <span className="statusbar-zone statusbar-right">
        <span className="statusbar-epoch" title={`Core instance ${bootstrap.core_instance_id}`}>
          core {epoch}
        </span>
        <ConnectionStatusView status={bootstrap.connection} />
      </span>
    </footer>
  );
}
