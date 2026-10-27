import { beforeEach, describe, expect, it, vi } from 'vitest';

const bridgeMocks = vi.hoisted(() => ({
  createTerminal: vi.fn(),
  terminalInput: vi.fn(),
  terminalResize: vi.fn(),
  terminalClose: vi.fn(),
}));

vi.mock('../../bridge/bridge', () => bridgeMocks);

import { terminalStore } from './terminal-store';

const CONTEXT = { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null };

const created = (terminalId = 'term-1', shell = '/usr/bin/bash', cwd = '/workspace') => ({
  terminal_id: terminalId,
  shell,
  cwd,
});

beforeEach(() => {
  terminalStore.reset();
  vi.clearAllMocks();
  bridgeMocks.createTerminal.mockReturnValue({
    requestId: 'req-1',
    promise: Promise.resolve(undefined),
  });
  bridgeMocks.terminalInput.mockResolvedValue(undefined);
  bridgeMocks.terminalResize.mockResolvedValue(undefined);
  bridgeMocks.terminalClose.mockResolvedValue(undefined);
});

describe('terminalStore create lifecycle', () => {
  it('sends TerminalCreate and keeps the truthful creating state until TerminalCreated', async () => {
    await terminalStore.requestCreate(90, 28, CONTEXT);

    expect(bridgeMocks.createTerminal).toHaveBeenCalledWith(90, 28, CONTEXT);
    expect(terminalStore.getSnapshot().creating).toBe(true);
    expect(terminalStore.getSnapshot().sessions).toEqual([]);

    terminalStore.handleCreated(created());
    expect(terminalStore.getSnapshot().creating).toBe(false);
    expect(terminalStore.getSnapshot().sessions[0]).toMatchObject({
      terminal_id: 'term-1',
      shell: '/usr/bin/bash',
      cwd: '/workspace',
      status: 'running',
      exit_code: null,
    });
    expect(terminalStore.getSnapshot().active_id).toBe('term-1');
  });

  it('surfaces a create failure instead of inventing a shell', async () => {
    bridgeMocks.createTerminal.mockReturnValue({
      requestId: 'req-2',
      promise: Promise.reject({ code: 'unavailable', message: 'terminal backend is not installed' }),
    });

    await terminalStore.requestCreate(80, 24, CONTEXT);

    expect(terminalStore.getSnapshot().creating).toBe(false);
    expect(terminalStore.getSnapshot().sessions).toEqual([]);
    expect(terminalStore.getSnapshot().error).toBe(
      'unavailable: terminal backend is not installed',
    );
  });

  it('ignores a second create while one is in flight', async () => {
    bridgeMocks.createTerminal.mockReturnValue({
      requestId: 'req-3',
      promise: new Promise(() => undefined),
    });
    void terminalStore.requestCreate(80, 24, CONTEXT);
    await terminalStore.requestCreate(80, 24, CONTEXT);
    expect(bridgeMocks.createTerminal).toHaveBeenCalledTimes(1);
  });
});

describe('terminalStore events', () => {
  it('routes output to the mounted view and buffers output that arrives first', () => {
    terminalStore.handleCreated(created());
    terminalStore.handleOutput({ terminal_id: 'term-1', data: 'before-mount' });

    const seen: string[] = [];
    const unsubscribe = terminalStore.subscribeOutput('term-1', (data) => seen.push(data));
    terminalStore.handleOutput({ terminal_id: 'term-1', data: 'after-mount' });

    expect(seen).toEqual(['before-mount', 'after-mount']);
    unsubscribe();
    terminalStore.handleOutput({ terminal_id: 'term-1', data: 'unsubscribed' });
    expect(seen).toEqual(['before-mount', 'after-mount']);
  });

  it('records the real exit code and refuses input after exit', () => {
    terminalStore.handleCreated(created());
    terminalStore.sendInput('term-1', 'echo hi\r', CONTEXT);
    expect(bridgeMocks.terminalInput).toHaveBeenCalledWith('term-1', 'echo hi\r', CONTEXT);

    terminalStore.handleExited({ terminal_id: 'term-1', code: 130 });

    expect(terminalStore.getSnapshot().sessions[0]).toMatchObject({
      status: 'exited',
      exit_code: 130,
    });
    terminalStore.sendInput('term-1', 'ignored', CONTEXT);
    expect(bridgeMocks.terminalInput).toHaveBeenCalledTimes(1);
  });

  it('sends TerminalResize with the measured grid', () => {
    terminalStore.handleCreated(created());
    terminalStore.sendResize('term-1', 132, 43, CONTEXT);
    expect(bridgeMocks.terminalResize).toHaveBeenCalledWith('term-1', 132, 43, CONTEXT);

    terminalStore.sendResize('term-1', 0, 43, CONTEXT);
    expect(bridgeMocks.terminalResize).toHaveBeenCalledTimes(1);
  });

  it('drops output and exit events for an unknown terminal id', () => {
    const listener = vi.fn();
    terminalStore.subscribeOutput('missing', listener);
    terminalStore.handleOutput({ terminal_id: 'missing', data: 'leak' });
    terminalStore.handleExited({ terminal_id: 'missing', code: 1 });
    expect(listener).not.toHaveBeenCalled();
    expect(terminalStore.getSnapshot().sessions).toEqual([]);
  });
});

describe('terminalStore close', () => {
  it('removes the tab after TerminalClose succeeds and activates a neighbour', async () => {
    terminalStore.handleCreated(created('term-1'));
    terminalStore.handleCreated(created('term-2'));
    await terminalStore.close('term-2', CONTEXT);

    expect(bridgeMocks.terminalClose).toHaveBeenCalledWith('term-2', CONTEXT);
    expect(terminalStore.getSnapshot().sessions.map((session) => session.terminal_id)).toEqual([
      'term-1',
    ]);
    expect(terminalStore.getSnapshot().active_id).toBe('term-1');
  });

  it('restores the tab when the backend refuses TerminalClose', async () => {
    terminalStore.handleCreated(created('term-1'));
    bridgeMocks.terminalClose.mockRejectedValue({ message: 'close refused' });

    await terminalStore.close('term-1', CONTEXT);

    expect(terminalStore.getSnapshot().sessions).toHaveLength(1);
    expect(terminalStore.getSnapshot().error).toBe('Could not close the terminal: close refused');
  });

  it('closes an exited tab locally because the backend already dropped it', async () => {
    terminalStore.handleCreated(created('term-1'));
    terminalStore.handleExited({ terminal_id: 'term-1', code: 0 });

    await terminalStore.close('term-1', CONTEXT);

    expect(bridgeMocks.terminalClose).not.toHaveBeenCalled();
    expect(terminalStore.getSnapshot().sessions).toEqual([]);
    expect(terminalStore.getSnapshot().error).toBeNull();
  });
});

describe('terminalStore activation and reset', () => {
  it('switches the active tab without sending a command', () => {
    terminalStore.handleCreated(created('term-1'));
    terminalStore.handleCreated(created('term-2'));
    terminalStore.activate('term-1');
    expect(terminalStore.getSnapshot().active_id).toBe('term-1');
    expect(bridgeMocks.createTerminal).not.toHaveBeenCalled();
  });

  it('reset clears sessions, errors and buffered output', () => {
    terminalStore.handleCreated(created());
    terminalStore.handleExited({ terminal_id: 'term-1', code: 0 });
    terminalStore.reset();
    expect(terminalStore.getSnapshot()).toMatchObject({
      sessions: [],
      active_id: null,
      creating: false,
      error: null,
    });
  });
});