// Selora IDE — Phase Z2 primary sidebar (DESIGN §3.3, ARCHITECTURE §8.1).
//
// --ink-low surface with a 1px right hairline, 24px header carrying the
// active view's uppercase label. The Z2 activity set is files / search /
// git / settings; every view renders its honest §6 empty state — no fake
// data. The settings view is a read-only echo of bootstrap.settings. The
// active view comes from the live layout store, not the bootstrap snapshot.
import { useSyncExternalStore } from 'react';
import type { BootstrapState, SurfaceId } from '../bridge/bridge';
import { getLayoutSnapshot, subscribeLayout, type LayoutState } from '../features/layout/layout-store';
import { EmptyState, EmptyStateButton } from '../components/EmptyState';
import { agentStore } from '../features/agent/agent-store';
import { AgentSessionRail } from '../features/agent/components/AgentSessionRail';

const VIEW_LABEL: Record<SurfaceId, string> = {
  files: 'Files',
  search: 'Search',
  git: 'Git',
  settings: 'Settings',
  problems: 'Problems',
  agent: 'Agent',
  output: 'Output',
  terminal_placeholder: 'Terminal',
};

function FilesView() {
  return (
    <EmptyState heading="No folder open" hint={<em>Open a folder to start editing.</em>}>
      <EmptyStateButton disabled title="Workspace opening lands in a later slice">
        Open Folder
      </EmptyStateButton>
    </EmptyState>
  );
}

function SearchView() {
  return (
    <div className="sidebar-search">
      <label className="visually-hidden" htmlFor="sidebar-search-input">
        Search the workspace
      </label>
      <input
        id="sidebar-search-input"
        className="sidebar-search-input"
        type="text"
        placeholder="Search the workspace"
        disabled
        title="Workspace search arrives in a later slice"
      />
    </div>
  );
}

function GitView() {
  return <EmptyState hint="Git integration lands in a later slice" />;
}

function SettingsView({
  bootstrap,
  onOpenConnect,
}: {
  bootstrap: BootstrapState;
  onOpenConnect?: () => void;
}) {
  const settings = bootstrap.settings;
  const isConnected = bootstrap.connection === 'connected';

  return (
    <div className="sidebar-settings" style={{ padding: '12px 14px', display: 'flex', flexDirection: 'column', gap: 14 }}>
      {/* Model Provider & Key Card */}
      <div style={{
        background: '#13171f',
        border: '1px solid rgba(255, 255, 255, 0.08)',
        borderRadius: 8,
        padding: '12px 14px',
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ fontSize: 12, fontWeight: 600, color: '#f3f4f6' }}>AI Provider & Key</span>
          <span style={{
            fontSize: 10,
            padding: '2px 6px',
            borderRadius: 4,
            fontWeight: 500,
            background: isConnected ? 'rgba(16, 185, 129, 0.15)' : 'rgba(245, 158, 11, 0.15)',
            color: isConnected ? '#34d399' : '#fbbf24',
          }}>
            {isConnected ? 'Connected' : 'Not Connected'}
          </span>
        </div>
        <p style={{ margin: 0, fontSize: 11, color: '#9ca3af', lineHeight: 1.4 }}>
          Connect GLM, DeepSeek, OpenAI, OpenRouter, or custom completions gateways.
        </p>
        <button
          type="button"
          onClick={onOpenConnect}
          style={{
            marginTop: 4,
            padding: '6px 12px',
            fontSize: 11.5,
            fontWeight: 500,
            borderRadius: 6,
            background: '#10b981',
            color: '#032014',
            border: 'none',
            cursor: 'pointer',
            textAlign: 'center',
          }}
        >
          {isConnected ? 'Manage API Key & Gateway' : 'Configure API Key'}
        </button>
      </div>

      <dl className="settings-echo">
        <div className="settings-echo-row">
          <dt>Theme</dt>
          <dd>{settings.theme}</dd>
        </div>
        <div className="settings-echo-row">
          <dt>Reduced motion</dt>
          <dd>{settings.reduced_motion ? 'on' : 'off'}</dd>
        </div>
        <div className="settings-echo-row">
          <dt>Command palette</dt>
          <dd>{settings.keybind_command_palette}</dd>
        </div>
        <div className="settings-echo-row">
          <dt>Toggle agent dock</dt>
          <dd>{settings.keybind_toggle_chat}</dd>
        </div>
      </dl>
      <p className="settings-later">Settings editing lands in a later slice.</p>
    </div>
  );
}

function AgentSidebarView() {
  const agentState = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <AgentSessionRail
        sessions={agentState.sessions}
        activeSessionId={agentState.activeSessionId}
      />
    </div>
  );
}

export function PrimarySidebar({
  bootstrap,
  onOpenConnect,
}: {
  bootstrap: BootstrapState;
  onOpenConnect?: () => void;
}) {
  const state: LayoutState = useSyncExternalStore(subscribeLayout, getLayoutSnapshot);
  const layout = state.layout;
  if (!layout) return null;
  const view = layout.activity_view;
  return (
    <div className="primary-sidebar">
      <div className="sidebar-header">{VIEW_LABEL[view]}</div>
      <div className="sidebar-body">
        {view === 'files' ? <FilesView /> : null}
        {view === 'search' ? <SearchView /> : null}
        {view === 'git' ? <GitView /> : null}
        {view === 'settings' ? <SettingsView bootstrap={bootstrap} onOpenConnect={onOpenConnect} /> : null}
        {view === 'agent' ? <AgentSidebarView /> : null}
      </div>
    </div>
  );
}
