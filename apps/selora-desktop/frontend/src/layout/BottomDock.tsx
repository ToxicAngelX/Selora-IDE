// Selora IDE — Phase Z2/Z3 bottom dock (DESIGN §3.6, ARCHITECTURE §8.1).
//
// Spans the editor column only. Tab strip: PROBLEMS / OUTPUT / TERMINAL
// (problems is a bottom-dock view in Z2, §2.2) plus the close ×. Every tab
// click and the × dispatch ToggleSurface per the routing table; content
// renders rich modular developer panels: ProblemsPanel, OutputPanel, TerminalPanel.
// The active view comes from the live layout store.
import { lazy, Suspense, useEffect, useState, useSyncExternalStore, type KeyboardEvent } from 'react';
import type { SurfaceId } from '../bridge/bridge';
import { getLayoutSnapshot, subscribeLayout, type LayoutState } from '../features/layout/layout-store';
import { layoutStore } from '../features/layout/layout-store';
import { Icon } from '../components/Icon';
import { ProblemsPanel } from '../features/dock/ProblemsPanel';
import { OutputPanel } from '../features/dock/OutputPanel';
// xterm is a substantial dependency; keep it out of the dock chunk until the
// terminal view is actually selected.
const TerminalPanel = lazy(async () => {
  const module = await import('../features/dock/TerminalPanel');
  return { default: module.TerminalPanel };
});
// CSS ownership follows the component (P1 startup split): ShellGrid loads this
// module lazily, so a closed dock never downloads dock.css. The slot skeleton
// in shell.css keeps the region laid out until the chunk arrives.
import '../styles/dock.css';

const BOTTOM_TABS: Array<{ surface: SurfaceId; label: string }> = [
  { surface: 'problems', label: 'Problems' },
  { surface: 'output', label: 'Output' },
  { surface: 'terminal_placeholder', label: 'Terminal' },
];

function BottomDockContent({ view }: { view: SurfaceId }) {
  // Once the terminal view is opened, keep it mounted while the dock shows
  // another view. Unmounting would dispose xterm and lose the scrollback;
  // TerminalCreated/TerminalOutput carry no replay, so the buffer cannot be
  // rebuilt from the backend.
  const [terminalMounted, setTerminalMounted] = useState(view === 'terminal_placeholder');
  useEffect(() => {
    if (view === 'terminal_placeholder') setTerminalMounted(true);
  }, [view]);

  return (
    <>
      {view === 'problems' ? <ProblemsPanel /> : null}
      {view === 'output' || (view !== 'problems' && view !== 'terminal_placeholder') ? (
        <OutputPanel />
      ) : null}
      {terminalMounted || view === 'terminal_placeholder' ? (
        <div className="terminal-host" hidden={view !== 'terminal_placeholder'}>
          <Suspense
            fallback={
              <div className="dock-panel terminal-panel" data-testid="terminal-panel" role="status">
                <span className="terminal-loading">Loading terminal…</span>
              </div>
            }
          >
            <TerminalPanel />
          </Suspense>
        </div>
      ) : null}
    </>
  );
}

export function BottomDock() {
  const state: LayoutState = useSyncExternalStore(subscribeLayout, getLayoutSnapshot);
  const layout = state.layout;
  if (!layout) return null;
  const view = layout.bottom_view;

  const focusTab = (surface: SurfaceId) => {
    document.getElementById(`bottom-tab-${surface}`)?.focus();
  };

  // Roving tabindex + automatic activation (WAI-ARIA tabs pattern): moving to
  // another tab selects it, matching the pointer click handler. Moving onto the
  // already-selected tab only focuses it so Home/End never toggle the dock shut.
  const selectTab = (surface: SurfaceId) => {
    if (surface !== view) {
      void layoutStore.toggle(surface);
    }
    focusTab(surface);
  };

  const handleTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const lastIndex = BOTTOM_TABS.length - 1;
    let nextIndex: number;
    if (event.key === 'ArrowRight') {
      nextIndex = (index + 1) % BOTTOM_TABS.length;
    } else if (event.key === 'ArrowLeft') {
      nextIndex = (index - 1 + BOTTOM_TABS.length) % BOTTOM_TABS.length;
    } else if (event.key === 'Home') {
      nextIndex = 0;
    } else if (event.key === 'End') {
      nextIndex = lastIndex;
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      void layoutStore.toggle(BOTTOM_TABS[index].surface);
      return;
    } else {
      return;
    }
    event.preventDefault();
    const next = BOTTOM_TABS[nextIndex];
    if (next) selectTab(next.surface);
  };

  return (
    <div className="bottom-dock">
      <div className="bottom-tabstrip" role="tablist" aria-label="Bottom dock views">
        {BOTTOM_TABS.map((tab, index) => (
          <button
            key={tab.surface}
            id={`bottom-tab-${tab.surface}`}
            type="button"
            role="tab"
            className={`bottom-tab${view === tab.surface ? ' is-active' : ''}`}
            aria-selected={view === tab.surface}
            aria-controls="bottom-dock-panel"
            tabIndex={view === tab.surface ? 0 : -1}
            onClick={() => void layoutStore.toggle(tab.surface)}
            onKeyDown={(event) => handleTabKeyDown(event, index)}
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
      <div
        id="bottom-dock-panel"
        className="bottom-content"
        role="tabpanel"
        aria-label={BOTTOM_TABS.find((tab) => tab.surface === view)?.label}
      >
        <BottomDockContent view={view} />
      </div>
    </div>
  );
}
