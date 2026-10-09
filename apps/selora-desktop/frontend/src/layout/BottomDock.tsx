// Selora IDE — Phase Z2 bottom dock (DESIGN §3.6, ARCHITECTURE §8.1).
//
// Spans the editor column only. Tab strip: PROBLEMS / OUTPUT / TERMINAL
// (problems is a bottom-dock view in Z2, §2.2) plus the close ×. Every tab
// click and the × dispatch ToggleSurface per the routing table; content
// renders the honest §6 placeholders — TERMINAL is the clearly-labeled
// placeholder brief §7 requires. The active view comes from the live
// layout store.
import { useSyncExternalStore } from 'react';
import type { SurfaceId } from '../bridge/bridge';
import { getLayoutSnapshot, subscribeLayout, type LayoutState } from '../features/layout/layout-store';
import { layoutStore } from '../features/layout/layout-store';
import { EmptyState } from '../components/EmptyState';
import { Icon } from '../components/Icon';

const BOTTOM_TABS: Array<{ surface: SurfaceId; label: string }> = [
  { surface: 'problems', label: 'Problems' },
  { surface: 'output', label: 'Output' },
  { surface: 'terminal_placeholder', label: 'Terminal' },
];

function BottomDockContent({ view }: { view: SurfaceId }) {
  if (view === 'problems') {
    return <EmptyState hint="Diagnostics land with the editor slice" />;
  }
  if (view === 'terminal_placeholder') {
    return (
      <EmptyState
        heading="Terminal"
        hint="The integrated terminal arrives in a later phase. Commands the agent runs appear in Output."
      />
    );
  }
  return <EmptyState hint="Nothing has run yet." />;
}

export function BottomDock() {
  const state: LayoutState = useSyncExternalStore(subscribeLayout, getLayoutSnapshot);
  const layout = state.layout;
  if (!layout) return null;
  const view = layout.bottom_view;
  return (
    <div className="bottom-dock">
      <div className="bottom-tabstrip" role="tablist" aria-label="Bottom dock views">
        {BOTTOM_TABS.map((tab) => (
          <button
            key={tab.surface}
            type="button"
            role="tab"
            className={`bottom-tab${view === tab.surface ? ' is-active' : ''}`}
            aria-selected={view === tab.surface}
            onClick={() => void layoutStore.toggle(tab.surface)}
          >
            {tab.label}
          </button>
        ))}
        <button
          type="button"
          className="bottom-close"
          aria-label="Close bottom dock"
          onClick={() => void layoutStore.toggle(layout.bottom_view)}
        >
          <Icon name="close" />
        </button>
      </div>
      <div className="bottom-content" role="tabpanel" aria-label={BOTTOM_TABS.find((tab) => tab.surface === view)?.label}>
        <BottomDockContent view={view} />
      </div>
    </div>
  );
}
