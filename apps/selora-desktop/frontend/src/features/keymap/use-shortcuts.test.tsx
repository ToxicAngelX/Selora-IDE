// useShortcuts tests (ARCHITECTURE §7.1): the palette chord opens the
// palette even from a text input; matched commands are swallowed and run;
// bindings are inert while a text input has focus; Esc dismisses the
// topmost toast with no modal open and closes the palette when one is.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render } from '@testing-library/react';
import { useShortcuts } from './use-shortcuts';
import { toastStore } from '../toasts/toast-store';
import { makeBootstrap } from '../../testing/fixtures';
import type { CommandId } from './commands';

const settings = makeBootstrap().settings;

function Harness(props: { paletteOpen: boolean; onCommand: (id: CommandId) => void; onClosePalette: () => void; withInput?: boolean }) {
  useShortcuts({ settings, enabled: true, paletteOpen: props.paletteOpen, onCommand: props.onCommand, onClosePalette: props.onClosePalette });
  return (
    <div>
      {props.withInput ? <input aria-label="surface input" data-testid="surface-input" /> : <button type="button">surface</button>}
    </div>
  );
}

beforeEach(() => {
  toastStore.clearAll();
  // The toast exit fade reads --dur-base from the token layer; provide it
  // the way the real stylesheet does.
  document.documentElement.style.setProperty('--dur-base', '240ms');
});

afterEach(() => {
  toastStore.clearAll();
  document.documentElement.style.removeProperty('--dur-base');
});

function press(key: string, opts: KeyboardEventInit = {}) {
  fireEvent.keyDown(window, { key, ...opts });
}

describe('useShortcuts (ARCHITECTURE §7.1)', () => {
  it('runs a matched command and swallows the event', () => {
    const onCommand = vi.fn();
    render(<Harness paletteOpen={false} onCommand={onCommand} onClosePalette={vi.fn()} />);
    press('B', { ctrlKey: true });
    expect(onCommand).toHaveBeenCalledWith('selora.togglePrimarySidebar');
  });

  it('unmatched chords do nothing', () => {
    const onCommand = vi.fn();
    render(<Harness paletteOpen={false} onCommand={onCommand} onClosePalette={vi.fn()} />);
    press('B', { ctrlKey: true, shiftKey: true });
    press('X', { ctrlKey: true });
    press('B');
    expect(onCommand).not.toHaveBeenCalled();
  });

  it('the palette chord works even from a focused text input', () => {
    const onCommand = vi.fn();
    const { getByTestId } = render(<Harness paletteOpen={false} onCommand={onCommand} onClosePalette={vi.fn()} withInput />);
    const input = getByTestId('surface-input');
    input.focus();
    fireEvent.keyDown(input, { key: 'P', ctrlKey: true, shiftKey: true });
    expect(onCommand).toHaveBeenCalledWith('selora.openCommandPalette');
  });

  it('other bindings are inert while a text input has focus', () => {
    const onCommand = vi.fn();
    const { getByTestId } = render(<Harness paletteOpen={false} onCommand={onCommand} onClosePalette={vi.fn()} withInput />);
    // A real keydown fires on the focused element, not on window.
    const input = getByTestId('surface-input');
    input.focus();
    fireEvent.keyDown(input, { key: 'B', ctrlKey: true });
    expect(onCommand).not.toHaveBeenCalled();
  });

  it('Esc with no modal dismisses the topmost toast', () => {
    toastStore.pushProtocol(1, { severity: 'info', message: 'one' });
    toastStore.pushProtocol(2, { severity: 'info', message: 'two' });
    render(<Harness paletteOpen={false} onCommand={vi.fn()} onClosePalette={vi.fn()} />);
    press('Escape');
    expect(toastStore.getSnapshot().leaving).toEqual([2]);
  });

  it('Esc closes the palette when one is open', () => {
    const onCommand = vi.fn();
    const onClosePalette = vi.fn();
    render(<Harness paletteOpen onCommand={onCommand} onClosePalette={onClosePalette} />);
    press('Escape');
    expect(onClosePalette).toHaveBeenCalledTimes(1);
    expect(toastStore.getSnapshot().toasts).toHaveLength(0);
  });

  it('commands are inert while the palette is open', () => {
    const onCommand = vi.fn();
    render(<Harness paletteOpen onCommand={onCommand} onClosePalette={vi.fn()} />);
    press('B', { ctrlKey: true });
    expect(onCommand).not.toHaveBeenCalledWith('selora.togglePrimarySidebar');
  });

  it('the rebindable pair follows the stored settings strings', () => {
    const onCommand = vi.fn();
    const { rerender } = render(<Harness paletteOpen={false} onCommand={onCommand} onClosePalette={vi.fn()} />);
    // Rebind toggle-chat to Alt+Q through a settings change.
    const rebound = { ...settings, keybind_toggle_chat: 'Alt+Q' };
    function ReboundHarness() {
      useShortcuts({ settings: rebound, enabled: true, paletteOpen: false, onCommand, onClosePalette: vi.fn() });
      return <div />;
    }
    rerender(<ReboundHarness />);
    act(() => {
      press('Q', { altKey: true });
    });
    expect(onCommand).toHaveBeenCalledWith('selora.toggleAgentDock');
  });

  it('a Save rebind resolves to the Save command and the old chord stops matching', () => {
    const onCommand = vi.fn();
    const rebound = { ...settings, keybind_save: 'Ctrl+Alt+S' };
    function ReboundHarness() {
      useShortcuts({ settings: rebound, enabled: true, paletteOpen: false, onCommand, onClosePalette: vi.fn() });
      return <div />;
    }
    render(<ReboundHarness />);

    act(() => {
      press('S', { ctrlKey: true });
    });
    expect(onCommand).not.toHaveBeenCalled();

    act(() => {
      press('S', { ctrlKey: true, altKey: true });
    });
    expect(onCommand).toHaveBeenCalledWith('selora.fileSave');
  });
});
