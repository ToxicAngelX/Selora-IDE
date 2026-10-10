// ShellGrid smoke test: the seven-region shell renders from real bootstrap
// data — chrome, activity bar, sidebar empty state, editor empty state,
// status bar contents — and region open/close state follows the live layout
// store (event-driven, not optimistic).
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render } from '@testing-library/react';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { invoke } from '@tauri-apps/api/core';
import { ShellGrid } from './ShellGrid';
import { buildCommandList } from '../features/keymap/commands';
import { layoutStore } from '../features/layout/layout-store';
import { makeBootstrap, makeLayout } from '../testing/fixtures';

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

  it('renders the activity bar with six toggles, settings pinned last', () => {
    const { getAllByRole, getByRole } = renderShell();
    const bar = getByRole('navigation', { name: 'Activity bar' });
    const buttons = getAllByRole('button').filter((button) => bar.contains(button));
    expect(buttons.map((button) => button.getAttribute('aria-label'))).toEqual([
      'Files',
      'Search',
      'Git',
      'Problems',
      'Agent',
      'Settings',
    ]);
    expect(getByRole('button', { name: 'Files' }).classList.contains('is-selected')).toBe(true);
  });

  it('renders the sidebar and editor empty states with disabled actions', () => {
    const { getByText, getAllByRole, getByRole, getAllByText } = renderShell();
    // The header label is uppercased by CSS text-transform — the DOM text is
    // the plain view name.
    expect(getByText('Files').classList.contains('sidebar-header')).toBe(true);
    expect(getAllByText('No folder open').length).toBeGreaterThanOrEqual(2); // sidebar + editor
    // Both Open Folder buttons (sidebar + editor) are disabled with the
    // later-slice caption.
    const openFolderButtons = getAllByRole('button', { name: 'Open Folder' });
    expect(openFolderButtons).toHaveLength(2);
    for (const button of openFolderButtons) {
      expect(button.hasAttribute('disabled')).toBe(true);
      expect(button.getAttribute('title')).toBe('Workspace opening lands in a later slice');
    }
    expect(getByRole('button', { name: 'Clone Repo' }).hasAttribute('disabled')).toBe(true);
    // The shortcut chip row (DESIGN §6).
    for (const chip of ['Ctrl+B', 'Ctrl+L', 'Ctrl+J', 'Ctrl+Shift+P']) {
      expect(getByText(chip).tagName).toBe('KBD');
    }
  });

  it('closed regions render nothing — only the sidebar handle exists', () => {
    const { container } = renderShell();
    const handles = container.querySelectorAll('.region-handle');
    expect(handles).toHaveLength(1); // primary sidebar only
    expect(container.querySelector('.agent-dock-slot')).toBeNull();
    expect(container.querySelector('.bottom-dock-slot')).toBeNull();
  });

  it('the agent dock appears when its layout event opens it, with the inert composer chrome', () => {
    const { container, getByText, getByRole } = renderShell();
    expect(container.querySelector('.agent-dock-slot')).toBeNull();
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ agent_dock_open: true }));
    });
    expect(container.querySelector('.agent-dock-slot')).not.toBeNull();
    // "Ask anything" heads the welcome; the inert composer carries its own
    // placeholder with the ellipsis.
    expect(getByText('Ask anything').classList.contains('empty-heading')).toBe(true);
    expect(getByText('Ask anything…').classList.contains('agent-composer-placeholder')).toBe(true);
    expect(getByText('Agent conversations arrive in a later phase.')).toBeTruthy();
    for (const chip of ['Explain this file', 'Find a bug in the selection', 'Summarize the workspace']) {
      expect(getByRole('button', { name: chip }).hasAttribute('disabled')).toBe(true);
    }
  });

  it('the bottom dock renders its three views — terminal is the clearly-labeled placeholder', () => {
    const { getByText, getByRole, queryByText } = renderShell(
      makeLayout({ bottom_dock_open: true, bottom_view: 'terminal_placeholder' }),
    );
    expect(getByRole('tab', { name: 'Terminal' }).classList.contains('is-active')).toBe(true);
    expect(getByText('The integrated terminal arrives in a later phase. Commands the agent runs appear in Output.')).toBeTruthy();
    // Output's honest placeholder is not shown while the terminal view is active.
    expect(queryByText('Nothing has run yet.')).toBeNull();

    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ bottom_dock_open: true, bottom_view: 'output' }));
    });
    expect(getByText('Nothing has run yet.')).toBeTruthy();
  });

  it('the sidebar switches views from the live layout, not the bootstrap snapshot', () => {
    const { getByText, getByPlaceholderText, getAllByText } = renderShell();
    expect(getByText('Files').classList.contains('sidebar-header')).toBe(true);
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ activity_view: 'search' }));
    });
    expect(getByText('Search').classList.contains('sidebar-header')).toBe(true);
    expect(getByPlaceholderText('Search the workspace').hasAttribute('disabled')).toBe(true);
    act(() => {
      layoutStore.applyLayoutEvent(makeLayout({ activity_view: 'settings' }));
    });
    expect(getByText('Settings').classList.contains('sidebar-header')).toBe(true);
    expect(getByText('Settings editing lands in a later slice.')).toBeTruthy();
    // The settings echo carries both keybinds; each also appears in the
    // editor's chip row.
    expect(getAllByText('Ctrl+Shift+P').length).toBe(2);
    expect(getAllByText('Ctrl+L').length).toBe(2);
  });

  it('grid geometry: the shell carries the region custom properties from Rust state', () => {
    const { container } = renderShell(makeLayout({ primary_sidebar_width_px: 320, agent_dock_open: true, agent_dock_width_px: 400 }));
    const shell = container.querySelector<HTMLElement>('.shell');
    expect(shell).not.toBeNull();
    expect(shell!.style.getPropertyValue('--sidebar-w')).toBe('320px');
    expect(shell!.style.getPropertyValue('--agent-w')).toBe('400px');
    expect(shell!.style.getPropertyValue('--bottom-h')).toBe('200px');
    expect(shell!.style.getPropertyValue('--hit-zone-sidebar')).toBe('6px');
    expect(shell!.style.getPropertyValue('--hit-zone-agent')).toBe('6px');
    expect(shell!.style.getPropertyValue('--hit-zone-bottom')).toBe('0px');
  });

  it('workbench carries data attributes for open states and responds to agent view mode', () => {
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
  });
});
