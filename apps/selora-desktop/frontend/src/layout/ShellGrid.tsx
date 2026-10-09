// Selora IDE — Phase Z2 shell grid (DESIGN §1).
//
// The seven-region workbench: `grid-template-rows: titlebar 1fr statusbar`;
// the middle row is the six-column workbench grid (activity bar | sidebar |
// handle track | center column | handle track | agent dock); the center
// column stacks editor | handle track | bottom dock. Region sizes are NOT
// tokens — they arrive from Rust (bootstrap.layout / layout_constraints)
// and reach CSS as inline custom properties on this container; the drag
// preview writes the same properties through `el.style.setProperty`
// (ARCHITECTURE §6.2's CSP-safe path). Handle tracks collapse to 0
// together with their region.
import { useEffect, useRef, useSyncExternalStore, type CSSProperties } from 'react';
import type { BootstrapState, RegionId } from '../bridge/bridge';
import {
  displayedSize,
  getLayoutSnapshot,
  layoutStore,
  regionOpen,
  subscribeLayout,
  type LayoutState,
} from '../features/layout/layout-store';
import type { CommandSpec } from '../features/keymap/commands';
import { TitleBar } from './TitleBar';
import { ActivityBar } from './ActivityBar';
import { PrimarySidebar } from './PrimarySidebar';
import { EditorRegion } from './EditorRegion';
import { AgentDock } from './AgentDock';
import { BottomDock } from './BottomDock';
import { StatusBar } from './StatusBar';
import { ResizeHandle } from './ResizeHandle';

const REGION_PROPERTY: Record<RegionId, string> = {
  primary_sidebar: '--sidebar-w',
  agent_dock: '--agent-w',
  bottom_dock: '--bottom-h',
};

function px(value: number): string {
  return `${value}px`;
}

export type ShellGridProps = {
  bootstrap: BootstrapState;
  commands: CommandSpec[];
  onRunCommand: (id: CommandSpec['id']) => void;
};

export function ShellGrid({ bootstrap, commands, onRunCommand }: ShellGridProps) {
  const state: LayoutState = useSyncExternalStore(subscribeLayout, getLayoutSnapshot);
  const shellRef = useRef<HTMLDivElement>(null);
  const workbenchRef = useRef<HTMLDivElement>(null);
  const activityBarRef = useRef<HTMLDivElement>(null);

  // Measured geometry feeds the clamping math (the editor-floor and the
  // 50%-of-workbench ceiling). The activity bar width is measured from the
  // real element — never a literal.
  useEffect(() => {
    const workbench = workbenchRef.current;
    const activityBar = activityBarRef.current;
    if (!workbench || !activityBar) return;
    const measure = () => {
      layoutStore.setWorkbenchBox({
        width: workbench.getBoundingClientRect().width,
        height: workbench.getBoundingClientRect().height,
        activityBarWidth: activityBar.getBoundingClientRect().width,
      });
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(workbench);
    observer.observe(activityBar);
    return () => observer.disconnect();
  }, []);

  // The drag preview: a direct CSSOM write, zero IPC, zero re-render
  // (ARCHITECTURE §3.1/§6.2).
  const onPreview = (region: RegionId, sizePx: number) => {
    shellRef.current?.style.setProperty(REGION_PROPERTY[region], px(sizePx));
  };

  const { layout, constraints } = state;
  if (!layout || !constraints) return null;

  const hitZone = constraints.resize_hit_zone_min_px;
  const sidebarOpen = regionOpen(layout, 'primary_sidebar');
  const agentOpen = regionOpen(layout, 'agent_dock');
  const bottomOpen = regionOpen(layout, 'bottom_dock');

  const style = {
    '--sidebar-w': px(displayedSize(state, 'primary_sidebar')),
    '--agent-w': px(displayedSize(state, 'agent_dock')),
    '--bottom-h': px(displayedSize(state, 'bottom_dock')),
    '--hit-zone-sidebar': sidebarOpen ? px(hitZone) : '0px',
    '--hit-zone-agent': agentOpen ? px(hitZone) : '0px',
    '--hit-zone-bottom': bottomOpen ? px(hitZone) : '0px',
  } as CSSProperties;

  return (
    <div ref={shellRef} className={`shell${state.dragging !== null ? ' is-dragging' : ''}`} style={style}>
      <TitleBar commands={commands} onRunCommand={onRunCommand} />
      <div className="workbench" ref={workbenchRef}>
        <nav className="activitybar" aria-label="Activity bar" ref={activityBarRef}>
          <ActivityBar />
        </nav>
        {sidebarOpen ? (
          <aside className="primary-sidebar-slot" aria-label="Primary sidebar">
            <PrimarySidebar bootstrap={bootstrap} />
          </aside>
        ) : null}
        {sidebarOpen ? (
          <div className="handle-track handle-track-sidebar">
            <ResizeHandle region="primary_sidebar" onPreview={onPreview} />
          </div>
        ) : null}
        <div className="center" role="presentation">
          <main className="editor-region-slot" aria-label="Editor">
            <EditorRegion />
          </main>
          {bottomOpen ? (
            <div className="handle-track handle-track-bottom">
              <ResizeHandle region="bottom_dock" onPreview={onPreview} />
            </div>
          ) : null}
          {bottomOpen ? (
            <section className="bottom-dock-slot" aria-label="Bottom dock">
              <BottomDock />
            </section>
          ) : null}
        </div>
        {agentOpen ? (
          <div className="handle-track handle-track-agent">
            <ResizeHandle region="agent_dock" onPreview={onPreview} />
          </div>
        ) : null}
        {agentOpen ? (
          <aside className="agent-dock-slot" aria-label="Agent dock">
            <AgentDock />
          </aside>
        ) : null}
      </div>
      <StatusBar bootstrap={bootstrap} />
    </div>
  );
}
