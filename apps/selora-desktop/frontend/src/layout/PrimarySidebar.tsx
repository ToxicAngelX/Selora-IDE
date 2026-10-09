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
      />
    </div>
  );
}

function GitView() {
  return <EmptyState hint="Git integration lands in a later slice" />;
}

function SettingsView({ bootstrap }: { bootstrap: BootstrapState }) {
  const settings = bootstrap.settings;
  return (
    <div className="sidebar-settings">
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

export function PrimarySidebar({ bootstrap }: { bootstrap: BootstrapState }) {
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
        {view === 'settings' ? <SettingsView bootstrap={bootstrap} /> : null}
      </div>
    </div>
  );
}
