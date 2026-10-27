// CheatSheet tests (Z5c): the dialog contract (role/aria-modal, search focus,
// Esc + backdrop close, focus restoration), grouping/filtering over the real
// registry, both open paths (palette command and window event), and the two
// new help entries wired to their deps callbacks.
import { describe, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { CheatSheet, OPEN_CHEATSHEET_EVENT, useCheatSheetTrigger } from './CheatSheet';
import { CommandPalette } from './CommandPalette';
import {
  buildCommandList,
  runCommandAction,
  type CommandDeps,
  type CommandSpec,
} from '../features/keymap/commands';
import { makeBootstrap } from '../testing/fixtures';

const commands: CommandSpec[] = buildCommandList(makeBootstrap().settings);

function makeDeps(overrides: Partial<CommandDeps> = {}): CommandDeps {
  return {
    layout: () => null,
    toggle: vi.fn(),
    saveLayout: vi.fn(),
    openPalette: vi.fn(),
    openCheatsheet: vi.fn(),
    showAbout: vi.fn(),
    ...overrides,
  };
}

describe('CheatSheet (Z5c keyboard shortcut cheat sheet)', () => {
  it('renders closed as nothing', () => {
    const { container } = render(<CheatSheet open={false} commands={commands} onClose={vi.fn()} />);
    expect(container.querySelector('.cheatsheet')).toBeNull();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('opens as a modal dialog listing every registry command grouped by category', () => {
    render(<CheatSheet open commands={commands} onClose={vi.fn()} />);

    const dialog = screen.getByRole('dialog', { name: 'Keyboard Shortcuts' });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(screen.getByRole('searchbox', { name: 'Filter shortcuts' })).toBe(document.activeElement);

    expect(screen.getAllByTestId('cheatsheet-row')).toHaveLength(commands.length);
    for (const id of [
      'selora.togglePrimarySidebar',
      'selora.toggleAgentDock',
      'selora.toggleBottomDock',
      'selora.showFiles',
      'selora.showSearch',
      'selora.showGit',
      'selora.showProblems',
      'selora.showSettings',
      'selora.showOutput',
      'selora.showTerminal',
      'selora.saveLayout',
      'selora.openCommandPalette',
      'selora.showKeyboardShortcuts',
      'selora.about',
    ]) {
      expect(document.querySelector(`[data-command-id="${id}"]`)).not.toBeNull();
    }

    expect(screen.getByRole('heading', { name: 'View & Layout' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'File Actions' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Agent Tools' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Help' })).toBeTruthy();
  });

  it('filters by title and by literal key chord', () => {
    render(<CheatSheet open commands={commands} onClose={vi.fn()} />);
    const search = screen.getByRole('searchbox', { name: 'Filter shortcuts' });

    fireEvent.change(search, { target: { value: 'save' } });
    let rows = screen.getAllByTestId('cheatsheet-row');
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain('Save Layout');

    fireEvent.change(search, { target: { value: 'ctrl+b' } });
    rows = screen.getAllByTestId('cheatsheet-row');
    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain('Toggle Primary Sidebar');

    fireEvent.change(search, { target: { value: 'zzzz' } });
    expect(screen.queryAllByTestId('cheatsheet-row')).toHaveLength(0);
    expect(screen.getByRole('status').textContent).toBe('No matching shortcuts.');
  });

  it('shows the live settings override for a rebindable command', () => {
    const rebound = buildCommandList({
      ...makeBootstrap().settings,
      keybind_toggle_chat: 'Alt+Q',
    });
    render(<CheatSheet open commands={rebound} onClose={vi.fn()} />);
    const row = document.querySelector('[data-command-id="selora.toggleAgentDock"]');
    expect(row?.textContent).toContain('Alt');
    expect(row?.textContent).toContain('Q');
    expect(row?.textContent).not.toContain('Ctrl+L');
  });

  it('Escape closes the sheet and restores focus to the opener', () => {
    const launcher = document.createElement('button');
    launcher.textContent = 'Shortcuts launcher';
    document.body.appendChild(launcher);
    launcher.focus();

    const onClose = vi.fn();
    function StatefulSheet() {
      const [open, setOpen] = useState(true);
      return (
        <CheatSheet
          open={open}
          commands={commands}
          onClose={() => {
            onClose();
            setOpen(false);
          }}
        />
      );
    }

    const { getByRole, queryByRole } = render(<StatefulSheet />);
    const search = getByRole('searchbox', { name: 'Filter shortcuts' });
    expect(document.activeElement).toBe(search);

    fireEvent.keyDown(search, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(queryByRole('dialog', { name: 'Keyboard Shortcuts' })).toBeNull();
    expect(document.activeElement).toBe(launcher);
    launcher.remove();
  });

  it('a pointerdown on the backdrop closes the sheet', () => {
    const onClose = vi.fn();
    render(<CheatSheet open commands={commands} onClose={onClose} />);
    fireEvent.pointerDown(screen.getByTestId('cheatsheet-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('opens from the palette command (Keyboard Shortcuts) through the registry', () => {
    const openCheatsheet = vi.fn();
    function PaletteHarness() {
      const [paletteOpen, setPaletteOpen] = useState(true);
      const [sheetOpen, setSheetOpen] = useState(false);
      const deps = makeDeps({
        openCheatsheet: () => {
          openCheatsheet();
          setSheetOpen(true);
        },
      });
      return (
        <>
          <CommandPalette
            open={paletteOpen}
            commands={commands}
            onRun={(id) => runCommandAction(id, deps)}
            onClose={() => setPaletteOpen(false)}
          />
          <CheatSheet open={sheetOpen} commands={commands} onClose={() => setSheetOpen(false)} />
        </>
      );
    }

    render(<PaletteHarness />);
    const paletteInput = screen.getByRole('combobox', { name: 'Search commands' });
    fireEvent.change(paletteInput, { target: { value: 'keyboard' } });
    fireEvent.keyDown(paletteInput, { key: 'Enter' });

    expect(openCheatsheet).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('dialog', { name: 'Keyboard Shortcuts' })).toBeTruthy();
    expect(screen.queryByRole('combobox', { name: 'Search commands' })).toBeNull();
  });

  it('opens from the selora:open-cheatsheet window event', () => {
    function EventHarness() {
      const [open, setOpen] = useState(false);
      useCheatSheetTrigger(() => setOpen(true));
      return <CheatSheet open={open} commands={commands} onClose={() => setOpen(false)} />;
    }

    render(<EventHarness />);
    expect(screen.queryByRole('dialog', { name: 'Keyboard Shortcuts' })).toBeNull();
    act(() => {
      window.dispatchEvent(new Event(OPEN_CHEATSHEET_EVENT));
    });
    expect(screen.getByRole('dialog', { name: 'Keyboard Shortcuts' })).toBeTruthy();
  });

  it('wires the two new help entries to their deps callbacks', () => {
    const deps = makeDeps();
    runCommandAction('selora.showKeyboardShortcuts', deps);
    runCommandAction('selora.about', deps);
    expect(deps.openCheatsheet).toHaveBeenCalledTimes(1);
    expect(deps.showAbout).toHaveBeenCalledTimes(1);
  });
});
