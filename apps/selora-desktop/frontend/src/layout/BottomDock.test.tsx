// Bottom dock tab strip: view switching plus the WAI-ARIA tabs keyboard pattern
// (roving tabindex, Arrow/Home/End movement, automatic activation).
import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest';
import { render, fireEvent, act } from '@testing-library/react';
import { BottomDock } from './BottomDock';
import { layoutStore } from '../features/layout/layout-store';
import { makeBootstrap, makeLayout } from '../testing/fixtures';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

function showView(view: 'problems' | 'output' | 'terminal_placeholder') {
  act(() => {
    layoutStore.applyLayoutEvent(makeLayout({ bottom_dock_open: true, bottom_view: view }));
  });
}

describe('BottomDock', () => {
  beforeEach(() => {
    layoutStore.reset();
    const bootstrap = makeBootstrap({
      layout: makeLayout({ bottom_dock_open: true, bottom_view: 'problems' }),
    });
    layoutStore.hydrate(bootstrap);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders tabstrip with Problems, Output, Terminal and displays active ProblemsPanel', () => {
    const { getByRole, getByTestId } = render(<BottomDock />);

    expect(getByRole('tab', { name: 'Problems' }).classList.contains('is-active')).toBe(true);
    expect(getByRole('tab', { name: 'Output' })).toBeTruthy();
    expect(getByRole('tab', { name: 'Terminal' })).toBeTruthy();

    expect(getByTestId('problems-panel')).toBeTruthy();
  });

  it('switches views to Output and Terminal when tabs are clicked', () => {
    const { getByRole, getByTestId } = render(<BottomDock />);

    // Click Output tab
    fireEvent.click(getByRole('tab', { name: 'Output' }));
    showView('output');
    expect(getByTestId('output-panel')).toBeTruthy();

    // Click Terminal tab
    fireEvent.click(getByRole('tab', { name: 'Terminal' }));
    showView('terminal_placeholder');
    expect(getByTestId('terminal-panel')).toBeTruthy();
  });

  it('triggers toggle when close button is clicked', () => {
    const toggleSpy = vi.spyOn(layoutStore, 'toggle');
    const { getByRole } = render(<BottomDock />);

    const closeBtn = getByRole('button', { name: 'Close bottom dock' });
    fireEvent.click(closeBtn);

    expect(toggleSpy).toHaveBeenCalled();
  });

  it('keeps only the selected tab in the tab order', () => {
    const { getByRole } = render(<BottomDock />);

    expect(getByRole('tab', { name: 'Problems' }).tabIndex).toBe(0);
    expect(getByRole('tab', { name: 'Output' }).tabIndex).toBe(-1);
    expect(getByRole('tab', { name: 'Terminal' }).tabIndex).toBe(-1);
  });

  it('changes selection with ArrowRight and ArrowLeft and wraps around', () => {
    const toggleSpy = vi.spyOn(layoutStore, 'toggle').mockResolvedValue(undefined);
    const { getByRole } = render(<BottomDock />);
    const problems = getByRole('tab', { name: 'Problems' });

    fireEvent.keyDown(problems, { key: 'ArrowRight' });
    expect(toggleSpy).toHaveBeenLastCalledWith('output');
    showView('output');

    const output = getByRole('tab', { name: 'Output' });
    expect(output.getAttribute('aria-selected')).toBe('true');
    expect(output.tabIndex).toBe(0);
    expect(problems.tabIndex).toBe(-1);
    expect(document.activeElement).toBe(output);

    fireEvent.keyDown(output, { key: 'ArrowLeft' });
    expect(toggleSpy).toHaveBeenLastCalledWith('problems');
    showView('problems');
    expect(problems.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(problems);

    // Wraps backward from the first tab to the last.
    fireEvent.keyDown(problems, { key: 'ArrowLeft' });
    expect(toggleSpy).toHaveBeenLastCalledWith('terminal_placeholder');
    showView('terminal_placeholder');
    expect(getByRole('tab', { name: 'Terminal' }).getAttribute('aria-selected')).toBe('true');
  });

  it('jumps to the first and last tabs with Home and End', () => {
    const toggleSpy = vi.spyOn(layoutStore, 'toggle').mockResolvedValue(undefined);
    const { getByRole } = render(<BottomDock />);

    fireEvent.keyDown(getByRole('tab', { name: 'Problems' }), { key: 'End' });
    expect(toggleSpy).toHaveBeenLastCalledWith('terminal_placeholder');
    showView('terminal_placeholder');

    const terminal = getByRole('tab', { name: 'Terminal' });
    expect(terminal.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(terminal);

    fireEvent.keyDown(terminal, { key: 'Home' });
    expect(toggleSpy).toHaveBeenLastCalledWith('problems');
    showView('problems');

    const problems = getByRole('tab', { name: 'Problems' });
    expect(problems.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(problems);
  });

  it('activates the focused tab with Enter and Space, matching the click handler', () => {
    const toggleSpy = vi.spyOn(layoutStore, 'toggle').mockResolvedValue(undefined);
    const { getByRole } = render(<BottomDock />);

    fireEvent.keyDown(getByRole('tab', { name: 'Output' }), { key: 'Enter' });
    expect(toggleSpy).toHaveBeenLastCalledWith('output');

    fireEvent.keyDown(getByRole('tab', { name: 'Terminal' }), { key: ' ' });
    expect(toggleSpy).toHaveBeenLastCalledWith('terminal_placeholder');
  });
});
