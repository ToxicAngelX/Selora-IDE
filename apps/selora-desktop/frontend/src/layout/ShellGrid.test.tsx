// ShellGrid smoke test: the seven-region shell renders from real bootstrap
// data — chrome, activity bar, sidebar empty state, editor empty state,
// status bar contents — and region open/close state follows the live layout
// store (event-driven, not optimistic).
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { invoke } from '@tauri-apps/api/core';
import { ShellGrid } from './ShellGrid';
import { buildCommandList } from '../features/keymap/commands';
import { layoutStore } from '../features/layout/layout-store';
import { makeBootstrap, makeLayout } from '../testing/fixtures';
import type { SurfaceId } from '../bridge/bridge';

const invokeMock = vi.mocked(invoke);

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  layoutStore.reset();
});

function renderShell(layout = makeLayout()) {
  const bootstrap = makeBootstrap({ layout });
  layoutStore.hydrate(bootstrap);
  layoutStore.setWorkbenchBox({ width: 1440, height: 900, activityBarWidth: 48 });
  const utils = render(
    <ShellGrid bootstrap={bootstrap} commands={buildCommandList(bootstrap.settings)} onRunCommand={vi.fn()} />,
  );
  return { bootstrap, ...utils };
}

describe('ShellGrid (DESIGN §1 — the seven regions)', () => {
  it('renders the chrome: title bar, menus, and the real status bar data', async () => {
    const { getByText, getByRole } = renderShell();
    await act(async () => {});
    expect(getByText('Selora IDE')).toBeTruthy();
    for (const menu of ['File', 'Edit', 'View', 'Help']) {
      expect(getByRole('button', { name: menu })).toBeTruthy();
    }
    // Real bootstrap data, not decoration: workspace name + core epoch.
    expect(getByText('No workspace open')).toBeTruthy();
    expect(getByText(/core core-000/)).toBeTruthy();
    expect(getByText(/selora disconnected/)).toBeTruthy();
    expect(getByRole('button', { name: /connect/ }).hasAttribute('disabled')).toBe(true);
  });

  it('renders the activity bar with seven toggles, settings pinned last', async () => {
    const { getAllByRole, getByRole } = renderShell();
    const bar = getByRole('navigation', { name: 'Activity bar' });
    const buttons = getAllByRole('button').filter((button) => bar.contains(button));
    expect(buttons.map((button) => button.getAttribute('aria-label'))).toEqual([
      'Files',
      'Search',
      'Git',
      'Problems',
      'Agent',
      'Workflows',
      'Settings',
    ]);
    expect(getByRole('button', { name: 'Files' }).classList.contains('is-selected')).toBe(true);
    expect(getByRole('button', { name: 'Workflows' }).getAttribute('aria-pressed')).toBe('false');
    await act(async () => {});
  });

  it('renders the sidebar and the live, buffer-driven editor empty state', async () => {
    const { container, findByText } = renderShell();
    // The sidebar is a lazy region (P1 startup split): its Suspense fallback
    // holds the landmark until the chunk resolves.
    expect(container.querySelector('.primary-sidebar-slot')).not.toBeNull();
    expect(
      container.querySelector('.primary-sidebar-loading')?.getAttribute('aria-busy'),
    ).toBe('true');
    // The header label is uppercased by CSS text-transform — the DOM text is
    // the plain view name.
    expect((await findByText('Files')).classList.contains('sidebar-header')).toBe(true);
    // The bootstrap fixture has no buffers, so the editor presents the real
    // no-workspace state instead of mock tabs.
    const editor = container.querySelector<HTMLElement>('.editor-region');
    expect(editor).not.toBeNull();
    expect(within(editor!).getByText('No folder open')).toBeTruthy();
    const openFolder = within(editor!).getByRole('button', { name: 'Open Folder' });
    expect(openFolder.hasAttribute('disabled')).toBe(false);
    // The shortcut chip row stays keyboard-readable in the empty state.
    for (const chip of ['Ctrl+B', 'Ctrl+J', 'Ctrl+Shift+P']) {
      expect(within(editor!).getByText(chip).tagName).toBe('KBD');
    }
  });

  it('closed regions render nothing — only the sidebar handle exists', async () => {
    const { container } = renderShell();
    const handles = container.querySelectorAll('.region-handle');
    expect(handles).toHaveLength(1); // primary sidebar only
    expect(container.querySelector('.agent-dock-slot')).toBeNull();
    expect(container.querySelector('.bottom-dock-slot')).toBeNull();
    // A closed region never mounts its component (so its chunk is never
    // requested) — no dock markup, no loading placeholder.
    expect(container.querySelector('.bottom-dock')).toBeNull();
    expect(container.querySelector('.bottom-dock-loading')).toBeNull();
    await act(async () => {});
  });

  it('a closed sidebar never mounts its lazy chunk, and toggling loads it', async () => {
    const { container, findByText } = renderShell(makeLayout({ primary_sidebar_open: false }));
    expect(container.querySelector('.primary-sidebar-slot')).toBeNull();
    expect(container.querySelector('.primary-sidebar-loading')).toBeNull();
    expect(container.querySelector('.primary-sidebar')).toBeNull();

    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ primary_sidebar_open: true }));
    });
    // Landmark is present immediately; the content arrives with the chunk.
    expect(container.querySelector('.primary-sidebar-slot')).not.toBeNull();
    expect((await findByText('Files')).classList.contains('sidebar-header')).toBe(true);
    expect(container.querySelector('.primary-sidebar-loading')).toBeNull();
  });

  it('the agent dock appears when its layout event opens it, with the inert composer chrome', async () => {
    const { container, findByText, findAllByRole, queryAllByRole } = renderShell();
    expect(container.querySelector('.agent-dock-slot')).toBeNull();
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ agent_dock_open: true }));
    });
    expect(container.querySelector('.agent-dock-slot')).not.toBeNull();
    // "Ask anything" heads the welcome; the inert composer carries its own
    // placeholder whose copy is gated by workspace/model readiness.
    // Lazy chunk (Z6 bundle split) — resolve before asserting its chrome.
    expect((await findByText('Ask anything')).classList.contains('empty-heading')).toBe(true);
    expect(
      container.querySelector('.agent-composer-placeholder')?.textContent?.trim(),
    ).toBeTruthy();
    // Suggestion chips are part of the composer chrome; they are disabled by
    // the workspace/model readiness gate, so only their presence is asserted.
    // No buffers are open in the fixture: the buffer- and selection-dependent
    // chips stay hidden, and only the workspace chip renders.
    const chips = await findAllByRole('button', { name: 'Summarize the workspace' });
    expect(chips.length).toBeGreaterThan(0);
    expect(queryAllByRole('button', { name: 'Explain this file' })).toHaveLength(0);
    expect(queryAllByRole('button', { name: 'Find a bug in the selection' })).toHaveLength(0);
  });

  it('the bottom dock renders its three views — terminal is the clearly-labeled placeholder', async () => {
    const { container, findByText, findByRole, queryByText } = renderShell(
      makeLayout({ bottom_dock_open: true, bottom_view: 'terminal_placeholder' }),
    );
    // Lazy dock region: the landmark exists before the chunk resolves.
    expect(container.querySelector('.bottom-dock-slot')).not.toBeNull();
    expect(
      container.querySelector('.bottom-dock-loading')?.getAttribute('aria-busy'),
    ).toBe('true');
    expect((await findByRole('tab', { name: 'Terminal' })).classList.contains('is-active')).toBe(true);
    expect(await findByText('No terminal session is running')).toBeTruthy();
    expect(await findByText('The integrated shell arrives with the terminal slice.')).toBeTruthy();
    // Output's honest placeholder is not shown while the terminal view is active.
    expect(queryByText('No output yet — tasks land with the terminal slice.')).toBeNull();

    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ bottom_dock_open: true, bottom_view: 'output' }));
    });
    expect(await findByText('No output yet — tasks land with the terminal slice.')).toBeTruthy();
  });

  it('a closed bottom dock never mounts its chunk, and toggling loads it', async () => {
    const { container, findByRole } = renderShell();
    expect(container.querySelector('.bottom-dock-slot')).toBeNull();
    expect(container.querySelector('.bottom-dock-loading')).toBeNull();
    expect(container.querySelector('.bottom-dock')).toBeNull();

    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ bottom_dock_open: true, bottom_view: 'problems' }));
    });
    expect(container.querySelector('.bottom-dock-slot')).not.toBeNull();
    expect((await findByRole('tab', { name: 'Problems' })).classList.contains('is-active')).toBe(true);
    expect(container.querySelector('.bottom-dock-loading')).toBeNull();
    await act(async () => {});
  });

  it('the sidebar switches views from the live layout, not the bootstrap snapshot', async () => {
    const { findByText, getAllByText } = renderShell();
    expect((await findByText('Files')).classList.contains('sidebar-header')).toBe(true);
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ activity_view: 'search' }));
    });
    expect((await findByText('Search')).classList.contains('sidebar-header')).toBe(true);
    expect(await findByText('Open a folder to search')).toBeTruthy();
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ activity_view: 'settings' }));
    });
    expect((await findByText('Settings')).classList.contains('sidebar-header')).toBe(true);
    expect(await findByText('The other controls are read-only in this build.')).toBeTruthy();
    // The settings echo carries keybinds
    expect(getAllByText('Ctrl+Shift+P').length).toBeGreaterThan(0);
    expect(getAllByText('Ctrl+L').length).toBeGreaterThan(0);
  });

  it('the Workflows activity selects the route, mounts the lazy panel, and leaves the other views working', async () => {
    const { container, findByText, getByRole } = renderShell();
    expect((await findByText('Files')).classList.contains('sidebar-header')).toBe(true);

    const workflows = getByRole('button', { name: 'Workflows' });
    expect(workflows.getAttribute('aria-pressed')).toBe('false');

    const toggle = vi.spyOn(layoutStore, 'toggle');
    fireEvent.click(workflows);
    expect(toggle).toHaveBeenCalledWith('workflows');
    toggle.mockRestore();

    // A click is a ToggleSurface round-trip: the view arrives with the
    // validating layout event, exactly like every other activity.
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ activity_view: 'workflows' as SurfaceId }));
    });
    await waitFor(() =>
      expect(container.querySelector('.sidebar-header')?.textContent).toBe('Workflows'),
    );
    expect(getByRole('button', { name: 'Workflows' }).getAttribute('aria-pressed')).toBe('true');
    expect(getByRole('button', { name: 'Workflows' }).classList.contains('is-selected')).toBe(true);

    // The route mounts the nested lazy panel; it resolves to the real panel
    // (or, if the module is absent/renamed, to the documented fallback —
    // covered by the export-contract test — never to a blank region).
    const slot = container.querySelector('.workflows-panel-slot');
    expect(slot).not.toBeNull();
    await waitFor(() => {
      expect(slot!.querySelector('[data-workflow-panel="loading"]')).toBeNull();
      expect(slot!.querySelector('[data-testid="workflows-panel"]')).not.toBeNull();
    });
    // The store is empty in this test: the panel's own empty state, not
    // invented run data.
    expect(slot!.textContent).toContain('No workflow runs');

    // Existing views still switch from the same live layout.
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ activity_view: 'git' }));
    });
    await waitFor(() => expect(container.querySelector('.sidebar-header')?.textContent).toBe('Git'));
    expect(container.querySelector('.workflows-panel-slot')).toBeNull();
    expect(getByRole('button', { name: 'Git' }).getAttribute('aria-pressed')).toBe('true');
    await act(async () => {});
  });

  it('grid geometry: the shell carries the region custom properties from Rust state', async () => {
    const { container } = renderShell(makeLayout({ primary_sidebar_width_px: 320, agent_dock_open: true, agent_dock_width_px: 400 }));
    const shell = container.querySelector<HTMLElement>('.shell');
    expect(shell).not.toBeNull();
    expect(shell!.style.getPropertyValue('--sidebar-w')).toBe('320px');
    expect(shell!.style.getPropertyValue('--agent-w')).toBe('400px');
    expect(shell!.style.getPropertyValue('--bottom-h')).toBe('200px');
    expect(shell!.style.getPropertyValue('--hit-zone-sidebar')).toBe('6px');
    expect(shell!.style.getPropertyValue('--hit-zone-agent')).toBe('6px');
    expect(shell!.style.getPropertyValue('--hit-zone-bottom')).toBe('0px');
    await act(async () => {});
  });

  it('one Agent activity click opens and selects the agent view without closing it', async () => {
    const { container, getByRole } = renderShell(makeLayout({ agent_dock_open: false }));
    expect(getByRole('button', { name: 'Agent' }).getAttribute('aria-pressed')).toBe('false');

    const toggle = vi.spyOn(layoutStore, 'toggle');
    fireEvent.click(getByRole('button', { name: 'Agent' }));

    // Exactly one toggle, not the old split-then-close pair.
    expect(toggle).toHaveBeenCalledTimes(1);
    expect(toggle).toHaveBeenCalledWith('agent');
    toggle.mockRestore();

    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ agent_dock_open: true }));
    });
    await waitFor(() => expect(container.querySelector('.agent-dock-slot')).not.toBeNull());
    const workbench = container.querySelector<HTMLElement>('.workbench');
    expect(workbench!.getAttribute('data-agent-open')).toBe('true');
    expect(workbench!.getAttribute('data-view-mode')).toBe('agent');
    expect(getByRole('button', { name: 'Agent' }).getAttribute('aria-pressed')).toBe('true');

    // Clicking the already-open activity focuses the agent view; it never
    // sends the close toggle that the status pill owns.
    const again = vi.spyOn(layoutStore, 'toggle');
    fireEvent.click(getByRole('button', { name: 'Agent' }));
    expect(again).not.toHaveBeenCalled();
    again.mockRestore();
    await act(async () => {});
  });

  it('workbench carries data attributes for open states and responds to agent view mode', async () => {
    const { container, getByRole } = renderShell(
      makeLayout({ primary_sidebar_open: false, agent_dock_open: true, bottom_dock_open: false })
    );
    const workbench = container.querySelector<HTMLElement>('.workbench');
    expect(workbench).not.toBeNull();
    expect(workbench!.getAttribute('data-sidebar-open')).toBe('false');
    expect(workbench!.getAttribute('data-bottom-open')).toBe('false');
    expect(workbench!.getAttribute('data-agent-open')).toBe('true');

    // Switch to agent view mode via titlebar pill
    const agentPill = getByRole('button', { name: 'Agent View' });
    expect(agentPill).toBeTruthy();
    act(() => {
      agentPill.click();
    });
    expect(workbench!.getAttribute('data-view-mode')).toBe('agent');
    await act(async () => {});
  });
});
