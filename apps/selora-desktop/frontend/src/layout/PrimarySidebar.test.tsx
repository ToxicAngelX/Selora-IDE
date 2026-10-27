// Selora IDE — PrimarySidebar Test Suite (Z4: live views)
//
// The detailed per-view behaviour lives in the colocated view tests
// (features/sidebar/*.test.tsx). This suite proves the sidebar delegates to
// the LIVE views with the bootstrap's real data — no mock file lists.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react';
import { memo } from 'react';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { PrimarySidebar, resolveWorkflowsPanelExport } from './PrimarySidebar';
import { layoutStore } from '../features/layout/layout-store';
import { makeBootstrap, makeLayout } from '../testing/fixtures';
import type { BootstrapState, SurfaceId } from '../bridge/bridge';

const treeRoot = {
  path: 'demo',
  name: 'demo',
  kind: 'directory' as const,
  children: [
    {
      path: 'demo/src',
      name: 'src',
      kind: 'directory' as const,
      children: [
        { path: 'demo/src/main.rs', name: 'main.rs', kind: 'file' as const, children: [] },
      ],
    },
    { path: 'demo/README.md', name: 'README.md', kind: 'file' as const, children: [] },
  ],
};

function liveBootstrap(activityView: SurfaceId): BootstrapState {
  return makeBootstrap({
    layout: makeLayout({ activity_view: activityView }),
    workspace: {
      id: 'ws_demo',
      core_instance_id: 'core-0001',
      root: 'C:/demo',
      display_name: 'demo',
    },
    file_tree: { root: treeRoot },
    git: { branch: 'main', changed_paths: ['demo/README.md'], staged_paths: [] },
    diagnostics: [
      { path: 'demo/src/main.rs', line: 2, column: 1, severity: 'warning', message: 'unused import' },
    ],
    feature_flags: { agent: true, git: true, search: true, editor: true, later_slices: [] },
  });
}

function renderView(activityView: SurfaceId): void {
  const bootstrap = liveBootstrap(activityView);
  layoutStore.hydrate(bootstrap);
  render(<PrimarySidebar bootstrap={bootstrap} />);
}

describe('PrimarySidebar (DESIGN §3.3)', () => {
  beforeEach(() => {
    layoutStore.reset();
  });

  it('renders the live file tree from the bootstrap — no mock list', () => {
    renderView('files');
    expect(screen.getByText('Files').classList.contains('sidebar-header')).toBe(true);
    expect(screen.getByText('src')).toBeTruthy();
    expect(screen.getByText('README.md')).toBeTruthy();
    expect(screen.queryByText('PrimarySidebar.tsx')).toBeNull();
  });

  it('expands a directory from the live tree', () => {
    renderView('files');
    fireEvent.click(screen.getByText('src'));
    expect(screen.getByText('main.rs')).toBeTruthy();
  });

  it('shows the workspace-search input when a folder is open', () => {
    renderView('search');
    expect(screen.getByText('Search').classList.contains('sidebar-header')).toBe(true);
    expect(screen.getByPlaceholderText('Search the workspace')).toBeTruthy();
  });

  it('shows the no-workspace search state when nothing is open', () => {
    const bootstrap = makeBootstrap({ layout: makeLayout({ activity_view: 'search' }) });
    layoutStore.hydrate(bootstrap);
    render(<PrimarySidebar bootstrap={bootstrap} />);
    expect(screen.getByText('Open a folder to search')).toBeTruthy();
  });

  it('renders the live Git branch and changed paths', () => {
    renderView('git');
    expect(screen.getByText('Git').classList.contains('sidebar-header')).toBe(true);
    expect(screen.getByText('main')).toBeTruthy();
    expect(screen.getByText('README.md')).toBeTruthy();
  });

  it('keeps the settings echo and switches views from the live layout', () => {
    renderView('settings');
    expect(screen.getByText('Settings').classList.contains('sidebar-header')).toBe(true);
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ activity_view: 'files' }));
    });
    expect(screen.getByText('Files').classList.contains('sidebar-header')).toBe(true);
  });

  it('mounts the lazy Workflows panel for the workflows route', async () => {
    renderView('workflows' as SurfaceId);
    expect(document.querySelector('.sidebar-header')?.textContent).toBe('Workflows');
    const slot = document.querySelector('.workflows-panel-slot');
    expect(slot).not.toBeNull();
    // The lazy boundary resolves to the panel module; the region is never
    // blank and never shows invented run data while the store is empty.
    await waitFor(() => {
      expect(slot!.querySelector('[data-workflow-panel="loading"]')).toBeNull();
      expect(slot!.querySelector('[data-testid="workflows-panel"]')).not.toBeNull();
    });
    expect(slot!.textContent).toContain('No workflow runs');
    expect(slot!.querySelector('[data-workflow-panel="unavailable"]')).toBeNull();
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ activity_view: 'search' }));
    });
    expect(document.querySelector('.sidebar-header')?.textContent).toBe('Search');
    expect(document.querySelector('.workflows-panel-slot')).toBeNull();
  });

  it('accepts only a renderable Workflows panel export', () => {
    const Panel = () => null;
    expect(resolveWorkflowsPanelExport({ WorkflowsPanel: Panel })).toBe(Panel);
    expect(resolveWorkflowsPanelExport({ default: Panel })).toBe(Panel);
    // Named export wins over default, and memo()/lazy() objects are allowed.
    expect(resolveWorkflowsPanelExport({ WorkflowsPanel: Panel, default: () => null })).toBe(Panel);
    expect(resolveWorkflowsPanelExport({ WorkflowsPanel: memo(Panel) })).not.toBeNull();
    // Everything else degrades to the fallback instead of crashing the shell.
    expect(resolveWorkflowsPanelExport({ PhaseGraph: Panel })).toBeNull();
    expect(resolveWorkflowsPanelExport({})).toBeNull();
    expect(resolveWorkflowsPanelExport(null)).toBeNull();
    expect(resolveWorkflowsPanelExport('WorkflowsPanel')).toBeNull();
  });
});