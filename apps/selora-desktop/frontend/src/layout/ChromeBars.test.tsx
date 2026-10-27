// Selora IDE — Phase Z2/Z3 Chrome Bars Comprehensive Tests (Agent 2)
//
// Tests TitleBar, ActivityBar, and StatusBar behaviors:
// - TitleBar: Menus, Command search pill trigger, workspace session & branch, view switcher, window controls
// - ActivityBar: Surface toggles, active indicator, notification badges, shortcuts, motion toggle
// - StatusBar: Left (workspace, git branch, sync, lang), Center (agent status pill), Right (LSP diagnostics, UTF-8, epoch, live gateway connection)
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { act, fireEvent, render } from '@testing-library/react';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: vi.fn(() => ({
    isMaximized: vi.fn().mockResolvedValue(false),
    onResized: vi.fn().mockResolvedValue(() => {}),
    minimize: vi.fn().mockResolvedValue(undefined),
    toggleMaximize: vi.fn().mockResolvedValue(undefined),
    close: vi.fn().mockResolvedValue(undefined),
  })),
}));

import { TitleBar } from './TitleBar';
import { ActivityBar } from './ActivityBar';
import { StatusBar } from './StatusBar';
import { layoutStore } from '../features/layout/layout-store';
import { agentStore } from '../features/agent/agent-store';
import { toastStore } from '../features/toasts/toast-store';
import { buildCommandList } from '../features/keymap/commands';
import { makeBootstrap, makeLayout } from '../testing/fixtures';
import type { SurfaceId } from '../bridge/bridge';

describe('Chrome Bars Polish (Agent 2: TitleBar, ActivityBar, StatusBar)', () => {
  beforeEach(() => {
    layoutStore.reset();
    toastStore.clearAll();
    toastStore.clearHistory();
  });

  describe('TitleBar', () => {
    it('renders identity, menus, and opens view menu dropdown with animations', () => {
      const commands = buildCommandList({
        reduced_motion: false,
        theme: 'selora-dark',
        keybind_command_palette: 'Ctrl+Shift+P',
        keybind_toggle_chat: 'Ctrl+L',
        keybind_fuzzy_find: 'Ctrl+P',
        keybind_save: 'Ctrl+S',
        editor_font_size: 14,
        editor_tab_size: 4,
        editor_word_wrap: false,
      });
      const onRunCommand = vi.fn();
      const bootstrap = makeBootstrap({
        workspace: { id: 'ws-1', core_instance_id: 'core-1', root: '/repo', display_name: 'selora-monorepo' },
        git: { branch: 'feat/z3-revamp', changed_paths: [], staged_paths: [] },
      });

      const { getByRole, getByText, queryByRole } = render(
        <TitleBar
          commands={commands}
          onRunCommand={onRunCommand}
          bootstrap={bootstrap}
          viewMode="split"
          onSelectViewMode={vi.fn()}
        />,
      );

      expect(getByText('Selora IDE')).toBeTruthy();
      expect(getByText('selora-monorepo')).toBeTruthy();
      expect(getByText('feat/z3-revamp')).toBeTruthy();

      // View menu
      const viewBtn = getByRole('button', { name: 'View' });
      expect(queryByRole('menu', { name: 'View menu' })).toBeNull();
      fireEvent.click(viewBtn);

      const menu = getByRole('menu', { name: 'View menu' });
      expect(menu).toBeTruthy();
      expect(menu.classList.contains('titlebar-dropdown')).toBe(true);

      // Select command item
      const toggleSidebarItem = getByRole('menuitem', { name: /Toggle Primary Sidebar/i });
      fireEvent.click(toggleSidebarItem);
      expect(onRunCommand).toHaveBeenCalledWith('selora.togglePrimarySidebar');
    });

    it('triggers command search palette when clicking search pill', () => {
      const commands = buildCommandList({
        reduced_motion: false,
        theme: 'selora-dark',
        keybind_command_palette: 'Ctrl+Shift+P',
        keybind_toggle_chat: 'Ctrl+L',
        keybind_fuzzy_find: 'Ctrl+P',
        keybind_save: 'Ctrl+S',
        editor_font_size: 14,
        editor_tab_size: 4,
        editor_word_wrap: false,
      });
      const onRunCommand = vi.fn();
      const { getByRole } = render(
        <TitleBar commands={commands} onRunCommand={onRunCommand} />,
      );

      const searchPill = getByRole('button', { name: 'Open Command Palette' });
      fireEvent.click(searchPill);
      expect(onRunCommand).toHaveBeenCalledWith('selora.openCommandPalette');
    });

    it('allows switching workbench view modes (Agent, Split, Editor)', () => {
      const onSelectViewMode = vi.fn();
      const { getByRole } = render(
        <TitleBar
          commands={[]}
          onRunCommand={vi.fn()}
          viewMode="split"
          onSelectViewMode={onSelectViewMode}
        />,
      );

      const agentBtn = getByRole('button', { name: 'Agent View' });
      const editorBtn = getByRole('button', { name: 'Editor View' });

      fireEvent.click(agentBtn);
      expect(onSelectViewMode).toHaveBeenCalledWith('agent');

      fireEvent.click(editorBtn);
      expect(onSelectViewMode).toHaveBeenCalledWith('editor');
    });

    it('mounts the notification bell with an unread badge and opens the center', () => {
      const { getByRole, getByText } = render(
        <TitleBar commands={[]} onRunCommand={vi.fn()} />,
      );

      const bell = getByRole('button', { name: 'Notifications' });
      act(() => {
        toastStore.pushLocal('info', 'Workspace indexed.');
        toastStore.pushLocal('error', 'Save failed.');
      });
      expect(getByText('2')).toBeTruthy();

      fireEvent.click(bell);
      expect(getByRole('dialog', { name: 'Notifications' })).toBeTruthy();
      expect(getByText('Workspace indexed.')).toBeTruthy();
    });
  });

  it('File, Edit, and Help menus expose real commands instead of a placeholder', () => {
    const commands = buildCommandList({
      reduced_motion: false,
      theme: 'selora-dark',
      keybind_command_palette: 'Ctrl+Shift+P',
      keybind_toggle_chat: 'Ctrl+L',
      keybind_fuzzy_find: 'Ctrl+P',
      keybind_save: 'Ctrl+S',
      editor_font_size: 14,
      editor_tab_size: 4,
      editor_word_wrap: false,
    });
    const onRunCommand = vi.fn();
    const { getByRole, queryByText } = render(
      <TitleBar commands={commands} onRunCommand={onRunCommand} />,
    );

    fireEvent.click(getByRole('button', { name: 'File' }));
    expect(queryByText('Available in a later slice')).toBeNull();
    const openFolder = getByRole('menuitem', { name: 'Open Folder…' });
    expect(openFolder.hasAttribute('disabled')).toBe(false);
    fireEvent.click(openFolder);
    expect(onRunCommand).toHaveBeenCalledWith('selora.fileOpenFolder');

    fireEvent.click(getByRole('button', { name: 'Edit' }));
    fireEvent.click(getByRole('menuitem', { name: /Find in Files/ }));
    expect(onRunCommand).toHaveBeenCalledWith('selora.editFindInFiles');

    fireEvent.click(getByRole('button', { name: 'Help' }));
    fireEvent.click(getByRole('menuitem', { name: 'Documentation' }));
    expect(onRunCommand).toHaveBeenCalledWith('selora.helpDocumentation');
  });

  describe('ActivityBar', () => {
    it('renders activity items with active state and badges for problems & git', () => {
      const bootstrap = makeBootstrap({
        diagnostics: [
          { path: 'src/main.rs', line: 10, column: 2, severity: 'error', message: 'Syntax error' },
          { path: 'src/lib.rs', line: 20, column: 1, severity: 'warning', message: 'Unused var' },
        ],
        git: { branch: 'main', changed_paths: ['file1.ts', 'file2.ts'], staged_paths: ['file3.ts'] },
      });
      layoutStore.hydrate(bootstrap);

      const { getByRole, getByText } = render(<ActivityBar bootstrap={bootstrap} />);

      // Files is selected by default in makeLayout
      const filesBtn = getByRole('button', { name: 'Files' });
      expect(filesBtn.classList.contains('is-selected')).toBe(true);

      // Problems badge: 2 total (1 error, 1 warning)
      expect(getByText('2')).toBeTruthy();

      // Git badge: 3 total changed + staged
      expect(getByText('3')).toBeTruthy();
    });

    it('has a motion/calm mode toggle switch that toggles data-reduced-motion', () => {
      const bootstrap = makeBootstrap();
      layoutStore.hydrate(bootstrap);

      const { getByRole } = render(<ActivityBar bootstrap={bootstrap} />);
      const motionToggle = getByRole('switch', { name: 'Toggle motion effects' });
      expect(motionToggle).toBeTruthy();
      expect(motionToggle.tagName).toBe('BUTTON');

      fireEvent.click(motionToggle);
      expect(document.documentElement.dataset.reducedMotion).toBe('true');

      fireEvent.click(motionToggle);
      expect(document.documentElement.dataset.reducedMotion).toBeUndefined();
    });

    it('exposes Workflows as a keyboard-reachable activity with an accessible label and selected state', () => {
      const bootstrap = makeBootstrap({
        layout: makeLayout({ activity_view: 'workflows' as SurfaceId }),
      });
      layoutStore.hydrate(bootstrap);

      const { getByRole } = render(<ActivityBar bootstrap={bootstrap} />);
      const workflows = getByRole('button', { name: 'Workflows' });

      // A native button: it is a tab stop and Enter/Space activate it.
      expect(workflows.tagName).toBe('BUTTON');
      expect(workflows.getAttribute('type')).toBe('button');
      expect(workflows.tabIndex).toBe(0);
      expect(workflows.getAttribute('aria-pressed')).toBe('true');
      expect(workflows.classList.contains('is-selected')).toBe(true);
      // No keybind ships with this route yet, so no hint is fabricated.
      expect(workflows.getAttribute('aria-keyshortcuts')).toBeNull();

      // Existing activities are not affected by the new one.
      expect(getByRole('button', { name: 'Files' }).getAttribute('aria-pressed')).toBe('false');
      expect(getByRole('button', { name: 'Agent' }).getAttribute('aria-pressed')).toBe('false');

      const toggle = vi.spyOn(layoutStore, 'toggle');
      fireEvent.click(workflows);
      expect(toggle).toHaveBeenCalledWith('workflows');
      toggle.mockRestore();
    });
    it('renders the Agent completion badge from unread background results', () => {
      const bootstrap = makeBootstrap();
      layoutStore.hydrate(bootstrap);
      agentStore.handleSubagentCompleted({
        child_id: 'child-badge-1',
        parent_session_id: 'sess-badge',
        policy: 'read_only',
        ok: true,
        final_message: 'done',
      });

      const { container } = render(<ActivityBar bootstrap={bootstrap} />);
      const agentButton = container.querySelector('button[aria-label="Agent"]');
      expect(agentButton?.querySelector('.activity-badge.is-success')?.textContent).toBe('1');
      agentStore.clearSubagentReveal();
      agentStore.markSubagentRead('child-badge-1');
    });
  });

  describe('StatusBar', () => {
    it('renders workspace, git sync button, and language in left zone', () => {
      const bootstrap = makeBootstrap({
        workspace: { id: 'ws-1', core_instance_id: 'core-99', root: '/app', display_name: 'selora-workspace' },
        git: { branch: 'feature/agentic-shell', changed_paths: [], staged_paths: [] },
      });
      layoutStore.hydrate(bootstrap);

      const { getByText, getByRole } = render(<StatusBar bootstrap={bootstrap} />);

      expect(getByText('selora-workspace')).toBeTruthy();
      expect(getByText('feature/agentic-shell')).toBeTruthy();
      expect(getByText('TypeScript')).toBeTruthy();

      // Git sync click animates spinner
      const gitBtn = getByRole('button', { name: /Git sync/i });
      fireEvent.click(gitBtn);
    });

    it('renders interactive agent status pill in center zone', () => {
      const bootstrap = makeBootstrap();
      layoutStore.hydrate(bootstrap);
      agentStore.hydrate(bootstrap);

      const { getByRole } = render(<StatusBar bootstrap={bootstrap} />);
      const agentPill = getByRole('button', { name: 'Agent Status' });
      expect(agentPill).toBeTruthy();
      const label = agentPill.querySelector('.agent-status-label');
      expect(label?.textContent).toContain('Selora Agent');
      expect(label?.getAttribute('aria-live')).toBeNull();

      const liveRegion = getByRole('status');
      expect(liveRegion.getAttribute('aria-live')).toBe('polite');
      expect(liveRegion.getAttribute('aria-atomic')).toBe('true');
      expect(liveRegion.textContent).toContain('Selora Agent');
      expect(liveRegion.classList.contains('visually-hidden')).toBe(true);
      expect(agentPill.contains(liveRegion)).toBe(false);

      // Clicking toggles agent dock
      const toggleSpy = vi.spyOn(layoutStore, 'toggle');
      fireEvent.click(agentPill);
      expect(toggleSpy).toHaveBeenCalledWith('agent');
    });

    it('renders diagnostics count, encoding, core epoch, and gateway status with radar in right zone', () => {
      const onOpenConnect = vi.fn();
      const bootstrap = makeBootstrap({
        core_instance_id: 'core-abcdef123456',
        diagnostics: [
          { path: 'a.rs', line: 1, column: 1, severity: 'error', message: 'err' },
          { path: 'b.rs', line: 2, column: 1, severity: 'warning', message: 'warn' },
        ],
        connection: 'connected',
      });
      layoutStore.hydrate(bootstrap);

      const { getByText, getByRole, container } = render(
        <StatusBar bootstrap={bootstrap} onOpenConnect={onOpenConnect} />,
      );

      // Diagnostics
      const diagBtn = getByRole('button', { name: /Problems summary/i });
      expect(diagBtn).toBeTruthy();

      // UTF-8 & Epoch
      expect(getByText('UTF-8')).toBeTruthy();
      expect(getByText('core core-abc')).toBeTruthy();

      // Gateway connected with radar ring
      expect(getByText(/selora connected/i)).toBeTruthy();
      const radarRing = container.querySelector('.statusbar-radar-ring');
      expect(radarRing).not.toBeNull();

      // Click connected status button
      const connBtn = getByRole('button', { name: /selora connected/i });
      fireEvent.click(connBtn);
      expect(onOpenConnect).toHaveBeenCalled();
    });

    it('renders disconnected state with connect button when not connected', () => {
      const onOpenConnect = vi.fn();
      const bootstrap = makeBootstrap({ connection: 'disconnected' });
      layoutStore.hydrate(bootstrap);

      const { getByText, getByRole } = render(
        <StatusBar bootstrap={bootstrap} onOpenConnect={onOpenConnect} />,
      );

      expect(getByText(/selora disconnected/i)).toBeTruthy();
      const connectBtn = getByRole('button', { name: 'connect' });
      expect(connectBtn.hasAttribute('disabled')).toBe(false);

      fireEvent.click(connectBtn);
      expect(onOpenConnect).toHaveBeenCalled();
    });
  });
});
