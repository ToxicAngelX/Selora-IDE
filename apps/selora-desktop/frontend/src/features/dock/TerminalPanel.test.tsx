import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';

const CONTEXT = { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null };

const harness = vi.hoisted(() => {
  const bridge = {
    createTerminal: vi.fn(),
    terminalInput: vi.fn(),
    terminalResize: vi.fn(),
    terminalClose: vi.fn(),
  };
  const context = vi.fn(
    (): { core_instance_id: string; workspace_id: string; session_id: null } | null => ({
      core_instance_id: 'core-0001',
      workspace_id: 'ws-1',
      session_id: null,
    }),
  );
  const instances: FakeTerminal[] = [];
  let openFailure = false;

  class FakeTerminal {
    static instances = instances;
    cols = 80;
    rows = 24;
    options: Record<string, unknown> = {};
    host: HTMLElement | null = null;
    writes: string[] = [];
    lastPaste: string | null = null;
    selection = '';
    disposed = false;
    focus = vi.fn();
    private dataHandlers = new Set<(data: string) => void>();
    private resizeHandlers = new Set<(size: { cols: number; rows: number }) => void>();

    constructor() {
      instances.push(this);
    }

    loadAddon(): void {}
    open(host: HTMLElement): void {
      if (openFailure) {
        openFailure = false;
        throw new Error('renderer unavailable');
      }
      this.host = host;
    }
    onData(handler: (data: string) => void) {
      this.dataHandlers.add(handler);
      return { dispose: () => this.dataHandlers.delete(handler) };
    }
    onResize(handler: (size: { cols: number; rows: number }) => void) {
      this.resizeHandlers.add(handler);
      return { dispose: () => this.resizeHandlers.delete(handler) };
    }
    attachCustomKeyEventHandler(): void {}
    hasSelection(): boolean {
      return this.selection !== '';
    }
    getSelection(): string {
      return this.selection;
    }
    paste(text: string): void {
      this.lastPaste = text;
      this.emitData(text);
    }
    write(data: string): void {
      this.writes.push(data);
    }
    dispose(): void {
      this.disposed = true;
    }
    emitData(data: string): void {
      for (const handler of this.dataHandlers) handler(data);
    }
    emitResize(cols: number, rows: number): void {
      this.cols = cols;
      this.rows = rows;
      for (const handler of this.resizeHandlers) handler({ cols, rows });
    }
  }

  return {
    bridge,
    context,
    instances,
    FakeTerminal,
    failNextOpen() {
      openFailure = true;
    },
  };
});

vi.mock('@xterm/xterm', () => ({ Terminal: harness.FakeTerminal }));
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit(): void {}
  },
}));
vi.mock('../../bridge/bridge', () => harness.bridge);
vi.mock('../../app/app-store', () => ({ currentEnvelopeContext: harness.context }));

import { TerminalPanel } from './TerminalPanel';
import { terminalStore } from './terminal-store';

const created = (terminalId = 'term-1', shell = '/usr/bin/bash', cwd = '/workspace') => ({
  terminal_id: terminalId,
  shell,
  cwd,
});

function createOk() {
  harness.bridge.createTerminal.mockReturnValue({
    requestId: 'req-1',
    promise: Promise.resolve(undefined),
  });
}

async function renderWithSession(terminalId = 'term-1') {
  const view = render(<TerminalPanel />);
  await waitFor(() => expect(harness.bridge.createTerminal).toHaveBeenCalled());
  act(() => {
    terminalStore.handleCreated(created(terminalId));
  });
  await waitFor(() => expect(harness.instances).toHaveLength(1));
  return view;
}

beforeEach(() => {
  terminalStore.reset();
  harness.instances.length = 0;
  vi.clearAllMocks();
  harness.context.mockReturnValue(CONTEXT);
  createOk();
  harness.bridge.terminalInput.mockResolvedValue(undefined);
  harness.bridge.terminalResize.mockResolvedValue(undefined);
  harness.bridge.terminalClose.mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      writeText: vi.fn().mockResolvedValue(undefined),
      readText: vi.fn().mockResolvedValue('pasted text'),
    },
  });
});

describe('TerminalPanel shell lifecycle', () => {
  it('sends TerminalCreate with the initial grid and shows no fake shell while waiting', async () => {
    render(<TerminalPanel />);

    await waitFor(() => expect(harness.bridge.createTerminal).toHaveBeenCalledTimes(1));
    expect(harness.bridge.createTerminal).toHaveBeenCalledWith(80, 24, CONTEXT);
    expect(screen.getByText('Starting shell…')).toBeTruthy();
    expect(screen.getByText('Waiting for the backend to create a real shell.')).toBeTruthy();
    expect(harness.instances).toHaveLength(0);
  });

  it('renders a tab and xterm after TerminalCreated carries the real shell and cwd', async () => {
    await renderWithSession();

    expect(screen.getByRole('tab', { name: 'bash 1' })).toBeTruthy();
    expect(screen.getByRole('tab', { name: 'bash 1' }).getAttribute('title')).toBe(
      '/usr/bin/bash — /workspace',
    );
    expect(screen.getByTestId('terminal-xterm-term-1')).toBeTruthy();
    expect(terminalStore.getSnapshot().active_id).toBe('term-1');
  });

  it('reports a failed xterm view and retries it without inventing a shell', async () => {
    harness.failNextOpen();
    await renderWithSession();

    expect(screen.getByText(/Terminal view failed to start: renderer unavailable/)).toBeTruthy();
    expect(screen.queryByTestId('terminal-xterm-term-1')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Retry view' }));
    await waitFor(() => expect(screen.getByTestId('terminal-xterm-term-1')).toBeTruthy());
    expect(harness.instances).toHaveLength(2);
  });

  it('degrades truthfully when the backend rejects TerminalCreate', async () => {
    harness.bridge.createTerminal.mockReturnValue({
      requestId: 'req-err',
      promise: Promise.reject({ code: 'unavailable', message: 'terminal backend is not installed' }),
    });

    render(<TerminalPanel />);

    const alert = await screen.findByTestId('terminal-error');
    expect(alert.textContent).toContain('unavailable: terminal backend is not installed');
    expect(screen.queryByTestId('terminal-xterm-term-1')).toBeNull();
    expect(harness.instances).toHaveLength(0);
    expect(screen.queryByText(/cargo check/i)).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(harness.bridge.createTerminal).toHaveBeenCalledTimes(2));
  });

  it('explains that a workspace is required when no envelope context exists', async () => {
    harness.context.mockReturnValue(null);
    render(<TerminalPanel />);

    expect((await screen.findByTestId('terminal-error')).textContent).toContain(
      'Open a folder before starting a terminal.',
    );
    expect(harness.bridge.createTerminal).not.toHaveBeenCalled();
  });
});

describe('TerminalPanel input, resize, output and exit', () => {
  it('forwards xterm input as TerminalInput', async () => {
    await renderWithSession();
    const terminal = harness.instances[0];

    act(() => terminal.emitData('echo hello\r'));

    expect(harness.bridge.terminalInput).toHaveBeenCalledWith('term-1', 'echo hello\r', CONTEXT);
  });

  it('forwards an xterm fit as TerminalResize', async () => {
    await renderWithSession();
    const terminal = harness.instances[0];

    act(() => terminal.emitResize(132, 43));

    expect(harness.bridge.terminalResize).toHaveBeenCalledWith('term-1', 132, 43, CONTEXT);
  });

  it('writes TerminalOutput into the live xterm', async () => {
    await renderWithSession();

    act(() => {
      terminalStore.handleOutput({ terminal_id: 'term-1', data: '\u001b[32mok\u001b[0m\r\n' });
    });

    expect(harness.instances[0].writes).toContain('\u001b[32mok\u001b[0m\r\n');
  });

  it('shows the real exit code and keeps the output on screen', async () => {
    await renderWithSession();
    act(() => {
      terminalStore.handleOutput({ terminal_id: 'term-1', data: 'last line\r\n' });
      terminalStore.handleExited({ terminal_id: 'term-1', code: 3 });
    });

    const banner = screen.getByTestId('terminal-exit-banner');
    expect(banner.textContent).toContain('Process exited with code 3');
    expect(harness.instances[0].writes).toContain('last line\r\n');
    expect(harness.instances[0].disposed).toBe(false);

    // Input after exit is not sent.
    act(() => harness.instances[0].emitData('should-not-send'));
    expect(harness.bridge.terminalInput).not.toHaveBeenCalled();
  });
});

describe('TerminalPanel tabs', () => {
  it('switches tabs without recreating or losing the xterm buffer', async () => {
    await renderWithSession('term-1');
    const first = harness.instances[0];

    fireEvent.click(screen.getByRole('button', { name: 'New terminal' }));
    await waitFor(() => expect(harness.bridge.createTerminal).toHaveBeenCalledTimes(2));
    act(() => {
      terminalStore.handleCreated(created('term-2', 'pwsh', 'C:\\work'));
    });
    await waitFor(() => expect(harness.instances).toHaveLength(2));

    fireEvent.click(screen.getByRole('tab', { name: 'bash 1' }));

    expect(terminalStore.getSnapshot().active_id).toBe('term-1');
    expect(screen.getByRole('tab', { name: 'bash 1' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('tab', { name: 'pwsh 2' }).getAttribute('aria-selected')).toBe('false');
    expect(harness.instances).toHaveLength(2);
    expect(first.disposed).toBe(false);
    expect(first.focus).toHaveBeenCalled();
  });

  it('sends TerminalClose and removes the tab', async () => {
    await renderWithSession();

    fireEvent.click(screen.getByRole('button', { name: 'Close bash 1' }));

    await waitFor(() => expect(harness.bridge.terminalClose).toHaveBeenCalledWith('term-1', CONTEXT));
    await waitFor(() => expect(screen.queryByRole('tab', { name: 'bash 1' })).toBeNull());
    expect(terminalStore.getSnapshot().sessions).toEqual([]);
  });
});

describe('TerminalPanel clipboard and focus', () => {
  it('copies the selection and pastes clipboard text through xterm', async () => {
    await renderWithSession();
    const terminal = harness.instances[0];
    terminal.selection = 'selected text';

    fireEvent.click(screen.getByRole('button', { name: 'Copy terminal selection' }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith('selected text'));

    fireEvent.click(screen.getByRole('button', { name: 'Paste into terminal' }));
    await waitFor(() => expect(terminal.lastPaste).toBe('pasted text'));
    expect(harness.bridge.terminalInput).toHaveBeenCalledWith('term-1', 'pasted text', CONTEXT);
  });
});