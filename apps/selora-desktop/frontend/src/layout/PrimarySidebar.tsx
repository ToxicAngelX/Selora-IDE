// Selora IDE — Phase Z2 primary sidebar (DESIGN §3.3, ARCHITECTURE §8.1).
//
// --ink-low surface with a 1px right hairline, 24px header carrying the
// active view's uppercase label. The Z2 activity set is files / search /
// git / settings; modular views provide rich interactivity and empty-state
// parity.
//
// Phase 8 (ROADMAP §"Phase 8 — Workflows", task 8d) adds one more route:
// `workflows`. The panel itself is a separate feature module and a separate
// chunk — it is mounted lazily, and only while the route is active. Because
// the panel lands in its own slice, the loader resolves it structurally:
// a missing module, a renamed export, or a component that throws on the
// props this shell passes all degrade to an explicit, readable fallback
// instead of a blank sidebar.
import {
  Component,
  Suspense,
  lazy,
  useSyncExternalStore,
  type ComponentType,
  type ErrorInfo,
  type ReactNode,
} from 'react';
import type { BootstrapState, SurfaceId } from '../bridge/bridge';
import { getLayoutSnapshot, subscribeLayout, type LayoutState } from '../features/layout/layout-store';
import { agentStore } from '../features/agent/agent-store';
import { AgentSessionRail } from '../features/agent/components/AgentSessionRail';
import { FileTreeView } from '../features/sidebar/FileTreeView';
import { SearchPanelView } from '../features/sidebar/SearchPanelView';
import { GitPanelView } from '../features/sidebar/GitPanelView';
import { SettingsPanelView } from '../features/sidebar/SettingsPanelView';
import { WORKFLOWS_SURFACE } from './ActivityBar';
// CSS ownership follows the component (P1 startup split): ShellGrid loads this
// module lazily, so a closed sidebar never downloads sidebar.css. The region
// skeleton in shell.css keeps the slot laid out until the chunk arrives.
import '../styles/sidebar.css';

const VIEW_LABEL: Record<SurfaceId, string> = {
  files: 'Files',
  search: 'Search',
  git: 'Git',
  settings: 'Settings',
  problems: 'Problems',
  agent: 'Agent',
  workflows: 'Workflows',
  output: 'Output',
  terminal_placeholder: 'Terminal',
};

/** Header label for the live route (the Phase 8 route is not in `SurfaceId`). */
function viewLabel(view: SurfaceId): string {
  return view === WORKFLOWS_SURFACE ? 'Workflows' : VIEW_LABEL[view];
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

// --- Phase 8 · Workflows panel (lazy, structurally resolved) ----------------

/**
 * What this shell hands the panel. The panel slice owns its own store
 * (`src/features/workflows/workflows-store.ts`), so it needs nothing from the
 * shell today; the bootstrap is passed anyway because it is the only state the
 * shell owns, and an unused prop is cheaper than a second coupling. Keep this
 * bag optional — the panel's props belong to its slice, not to the shell.
 */
export type WorkflowsPanelProps = { bootstrap?: BootstrapState };

type WorkflowsPanelModule = { WorkflowsPanel?: unknown; default?: unknown };

/**
 * The panel module is authored by the Phase 8 panel slice. A glob lookup (not
 * a static import) keeps this shell compiling — and the route testable — while
 * that file is absent, and still emits a real on-demand chunk once it exists.
 * (`import.meta.glob` is a Vite transform; this tree carries no
 * `vite/client` ambient types, hence the one suppression.)
 */
// @ts-ignore -- Vite provides import.meta.glob; no vite/client types in this tree.
const workflowPanelModules = import.meta.glob('../features/workflows/WorkflowsPanel.tsx') as Record<
  string,
  () => Promise<unknown>
>;

/** A function component or a `memo`/`lazy` exotic component. */
function isRenderableComponent(value: unknown): boolean {
  if (typeof value === 'function') return true;
  return typeof value === 'object' && value !== null && '$$typeof' in value;
}

/**
 * The panel's export contract, in one place. The slice owns its file, so this
 * shell accepts either a named `WorkflowsPanel` or a default export and
 * rejects anything else (renamed export, data-only module, non-component
 * value) by returning `null` — the loader then shows the fallback.
 */
export function resolveWorkflowsPanelExport(
  module: unknown,
): ComponentType<WorkflowsPanelProps> | null {
  if (typeof module !== 'object' || module === null) return null;
  const candidate =
    (module as WorkflowsPanelModule).WorkflowsPanel ?? (module as WorkflowsPanelModule).default;
  return isRenderableComponent(candidate)
    ? (candidate as unknown as ComponentType<WorkflowsPanelProps>)
    : null;
}

function WorkflowsPanelLoading() {
  return (
    <div
      className="workflows-panel-loading"
      role="status"
      aria-live="polite"
      data-workflow-panel="loading"
      style={{ color: 'var(--fg-muted)', fontSize: 'var(--type-sm)', lineHeight: 'var(--type-sm-lh)' }}
    >
      Loading workflow runs…
    </div>
  );
}

/**
 * The honest empty state. It is shown when the panel module is not in this
 * build, exports something this shell cannot render, or fails on the props it
 * receives — never with invented run, phase or cost data.
 */
function WorkflowsPanelUnavailable({ detail }: { bootstrap?: BootstrapState; detail?: string }) {
  return (
    <div
      className="workflows-panel-fallback"
      role="status"
      data-workflow-panel="unavailable"
      style={{ color: 'var(--fg-muted)', fontSize: 'var(--type-sm)', lineHeight: 'var(--type-sm-lh)' }}
    >
      <p style={{ margin: '0 0 var(--space-2)', color: 'var(--fg-default)' }}>
        The Workflows panel is not available in this build.
      </p>
      <p style={{ margin: 0 }}>
        The route is wired; the panel module
        (<code>src/features/workflows/WorkflowsPanel.tsx</code>) ships with the Phase 8 UI
        slice. Runs, the phase graph, artifacts and the cost meter appear here once it
        lands.
      </p>
      {detail ? (
        <p style={{ margin: 'var(--space-2) 0 0' }} data-workflow-panel-detail="">
          {detail}
        </p>
      ) : null}
    </div>
  );
}

async function loadWorkflowsPanel(): Promise<{ default: ComponentType<WorkflowsPanelProps> }> {
  const load = Object.values(workflowPanelModules)[0];
  if (!load) return { default: WorkflowsPanelUnavailable };
  try {
    const resolved = resolveWorkflowsPanelExport(await load());
    return { default: resolved ?? WorkflowsPanelUnavailable };
  } catch {
    return { default: WorkflowsPanelUnavailable };
  }
}

const WorkflowsPanel = lazy(loadWorkflowsPanel);

/** Keeps a panel-side render error inside the sidebar region. */
class WorkflowsPanelBoundary extends Component<{ children?: ReactNode }, { failure: string | null }> {
  state: { failure: string | null } = { failure: null };

  static getDerivedStateFromError(error: unknown): { failure: string } {
    return { failure: error instanceof Error ? error.message : 'Unknown panel error' };
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error('Workflows panel failed to render', error, info.componentStack);
  }

  render(): ReactNode {
    if (this.state.failure !== null) {
      return <WorkflowsPanelUnavailable detail={this.state.failure} />;
    }
    return this.props.children;
  }
}

function WorkflowsSidebarView({ bootstrap }: { bootstrap: BootstrapState }) {
  return (
    <div className="workflows-panel-slot" data-workflow-view="workflows">
      <WorkflowsPanelBoundary>
        <Suspense fallback={<WorkflowsPanelLoading />}>
          <WorkflowsPanel bootstrap={bootstrap} />
        </Suspense>
      </WorkflowsPanelBoundary>
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
      <h2 className="sidebar-header">{viewLabel(view)}</h2>
      <div className="sidebar-body">
        {view === 'files' ? <FileTreeView bootstrap={bootstrap} /> : null}
        {view === 'search' ? <SearchPanelView bootstrap={bootstrap} /> : null}
        {view === 'git' ? <GitPanelView bootstrap={bootstrap} /> : null}
        {view === 'settings' ? <SettingsPanelView bootstrap={bootstrap} onOpenConnect={onOpenConnect} /> : null}
        {view === 'agent' ? <AgentSidebarView /> : null}
        {view === WORKFLOWS_SURFACE ? <WorkflowsSidebarView bootstrap={bootstrap} /> : null}
      </div>
    </div>
  );
}