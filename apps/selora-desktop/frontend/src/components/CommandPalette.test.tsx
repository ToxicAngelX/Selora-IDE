// CommandPalette interaction tests: the combobox/listbox pattern, keyboard
// navigation (arrows wrap, Home/End, Enter runs + closes), fuzzy filtering
// in the DOM, and focus returning to the invoking control on close
// (DESIGN §7).
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render } from '@testing-library/react';
import { CommandPalette } from './CommandPalette';
import { buildCommandList } from '../features/keymap/commands';
import { makeBootstrap } from '../testing/fixtures';

const commands = buildCommandList(makeBootstrap().settings);

function setup(open: boolean) {
  const onRun = vi.fn();
  const onClose = vi.fn();
  const utils = render(<CommandPalette open={open} commands={commands} onRun={onRun} onClose={onClose} />);
  return { onRun, onClose, ...utils };
}

beforeEach(() => {
  const launcher = document.createElement('button');
  launcher.textContent = 'Launcher';
  launcher.dataset.testid = 'launcher';
  document.body.appendChild(launcher);
});

describe('CommandPalette (UI_SPEC §5, ARCHITECTURE §7.2)', () => {
  it('renders closed as nothing', () => {
    const { container } = setup(false);
    expect(container.querySelector('.palette')).toBeNull();
  });

  it('opens as a labeled dialog with a combobox and the eleven commands', () => {
    const { getByRole, getAllByRole } = setup(true);
    const dialog = getByRole('dialog', { name: 'Command palette' });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    const input = getByRole('combobox', { name: 'Search commands' }) as HTMLInputElement;
    expect(input.getAttribute('aria-controls')).toBe('palette-listbox');
    expect(document.activeElement).toBe(input);
    expect(getAllByRole('option')).toHaveLength(11);
  });

  it('typing fuzzy-filters the list live', () => {
    const { getByRole, getAllByRole, queryAllByRole } = setup(true);
    const input = getByRole('combobox', { name: 'Search commands' });
    act(() => {
      fireEvent.change(input, { target: { value: 'save' } });
    });
    expect(queryAllByRole('option')).toHaveLength(1);
    expect(getAllByRole('option')[0].textContent).toContain('Save Layout');
    act(() => {
      fireEvent.change(input, { target: { value: 'zzzz' } });
    });
    expect(queryAllByRole('option')).toHaveLength(0);
    expect(getByRole('status').textContent).toBe('No matching commands.');
  });

  it('ArrowDown/ArrowUp move the selection with wrap-around; aria-activedescendant follows', () => {
    const { getByRole } = setup(true);
    const input = getByRole('combobox', { name: 'Search commands' });
    const selected = () => document.querySelector('.palette-option.is-active')?.textContent ?? '';
    expect(selected()).toContain('Toggle Primary Sidebar');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(selected()).toContain('Toggle Agent Dock');
    fireEvent.keyDown(input, { key: 'End' });
    expect(selected()).toContain('Save Layout');
    fireEvent.keyDown(input, { key: 'ArrowDown' }); // wraps to the first
    expect(selected()).toContain('Toggle Primary Sidebar');
    fireEvent.keyDown(input, { key: 'ArrowUp' }); // wraps back to the last
    expect(selected()).toContain('Save Layout');
    expect(input.getAttribute('aria-activedescendant')).toBe('palette-option-10');
  });

  it('Enter runs the active command and closes; Home jumps to the first', () => {
    const { getByRole, onRun, onClose } = setup(true);
    const input = getByRole('combobox', { name: 'Search commands' });
    fireEvent.keyDown(input, { key: 'End' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onRun).toHaveBeenCalledWith('selora.saveLayout');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Tab is trapped (the combobox is the only tab stop)', () => {
    const { getByRole } = setup(true);
    const input = getByRole('combobox', { name: 'Search commands' });
    expect(() => fireEvent.keyDown(input, { key: 'Tab' })).not.toThrow();
    expect(document.activeElement).toBe(input);
  });

  it('a pointerdown on the backdrop closes without running anything', () => {
    const { getByTestId, onRun, onClose } = setup(true);
    fireEvent.pointerDown(getByTestId('palette-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onRun).not.toHaveBeenCalled();
  });

  it('focus returns to the invoking control when the palette closes', () => {
    const launcher = document.querySelector<HTMLElement>('[data-testid="launcher"]')!;
    launcher.focus();
    const { getByRole, onClose, rerender } = setup(true);
    expect(document.activeElement).toBe(getByRole('combobox', { name: 'Search commands' }));
    rerender(<CommandPalette open={false} commands={commands} onRun={vi.fn()} onClose={onClose} />);
    expect(document.activeElement).toBe(launcher);
    launcher.remove();
  });
});
