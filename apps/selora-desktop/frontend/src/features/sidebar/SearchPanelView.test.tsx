import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => {
  let snapshot: unknown = null;
  const listeners = new Set<() => void>();
  return {
    searchFiles: vi.fn(),
    openPath: vi.fn(),
    openPathAtLine: vi.fn(),
    setSnapshot(next: unknown) {
      snapshot = next;
      for (const listener of listeners) listener();
    },
    getSnapshot() {
      return snapshot;
    },
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    context: () => ({ core_instance_id: 'core-1', workspace_id: 'ws-1', session_id: null }),
  };
});

vi.mock('../../bridge/bridge', () => ({
  searchFiles: harness.searchFiles,
}));

vi.mock('../../app/app-store', () => ({
  getAppSnapshot: harness.getSnapshot,
  subscribeApp: harness.subscribe,
  currentEnvelopeContext: harness.context,
}));

vi.mock('../editor/editor-store', () => ({
  editorStore: {
    openPath: harness.openPath,
    openPathAtLine: harness.openPathAtLine,
  },
}));

import { SearchPanelView } from './SearchPanelView';

function readyState() {
  const bootstrap = makeBootstrap({
    workspace: {
      ...makeBootstrap().workspace,
      id: 'ws-1',
      root: 'C:/workspace',
      display_name: 'workspace',
    },
  });
  return { phase: 'ready', bootstrap, error: null, searchResults: null, gitDiff: null };
}

const results = {
  request_id: 'search-1',
  query: { query: 'needle', case_sensitive: false, whole_word: false, use_regex: false },
  hits: [
    { path: 'src/App.tsx', line: 3, column: 7, preview: 'const needle = 1;' },
    { path: 'src/App.tsx', line: 9, column: 1, preview: 'needle();' },
  ],
};

describe('SearchPanelView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    harness.searchFiles.mockResolvedValue(undefined);
    harness.setSnapshot(readyState());
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('debounces searchFiles, renders grouped live hits, and opens a hit at its line', async () => {
    render(<SearchPanelView bootstrap={readyState().bootstrap} />);
    const input = screen.getByLabelText('Search');

    fireEvent.change(input, { target: { value: 'needle' } });
    expect(harness.searchFiles).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(250);
      await Promise.resolve();
    });

    expect(harness.searchFiles).toHaveBeenCalledTimes(1);
    expect(harness.searchFiles).toHaveBeenCalledWith(
      { query: 'needle', case_sensitive: false, whole_word: false, use_regex: false },
      expect.objectContaining({ core_instance_id: 'core-1', workspace_id: 'ws-1' }),
    );

    await act(async () => {
      harness.setSnapshot({ ...readyState(), searchResults: results });
      await Promise.resolve();
    });

    expect(screen.getByText('App.tsx')).toBeTruthy();
    expect(screen.getAllByText('needle').length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole('button', { name: 'Open src/App.tsx at line 3' }));
    expect(harness.openPathAtLine).toHaveBeenCalledWith('src/App.tsx', 3);
  });

  it('passes case-sensitive and whole-word options to the bridge', async () => {
    render(<SearchPanelView bootstrap={readyState().bootstrap} />);
    fireEvent.click(screen.getByRole('button', { name: 'Match Case' }));
    fireEvent.click(screen.getByRole('button', { name: 'Match Whole Word' }));
    fireEvent.change(screen.getByLabelText('Search'), { target: { value: 'Needle' } });

    await act(async () => {
      vi.advanceTimersByTime(250);
      await Promise.resolve();
    });

    expect(harness.searchFiles).toHaveBeenCalledWith(
      { query: 'Needle', case_sensitive: true, whole_word: true, use_regex: false },
      expect.any(Object),
    );
  });

  it('passes the regex option to the bridge', async () => {
    render(<SearchPanelView bootstrap={readyState().bootstrap} />);
    const regexToggle = screen.getByRole('button', { name: 'Regex' });
    expect(regexToggle.getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(regexToggle);
    expect(regexToggle.getAttribute('aria-pressed')).toBe('true');
    fireEvent.change(screen.getByLabelText('Search'), { target: { value: 'needle.*' } });

    await act(async () => {
      vi.advanceTimersByTime(250);
      await Promise.resolve();
    });

    expect(harness.searchFiles).toHaveBeenCalledWith(
      { query: 'needle.*', case_sensitive: false, whole_word: false, use_regex: true },
      expect.any(Object),
    );
  });

  it('shows invalid regex errors inline without clearing the previous results', async () => {
    harness.setSnapshot({ ...readyState(), searchResults: results });
    render(<SearchPanelView bootstrap={readyState().bootstrap} />);

    fireEvent.change(screen.getByLabelText('Search'), { target: { value: 'needle' } });
    expect(screen.getByText('App.tsx')).toBeTruthy();

    harness.searchFiles.mockRejectedValueOnce({
      code: 'invalid_query',
      message: 'regex parse error: unclosed group',
      retryable: false,
    });
    fireEvent.click(screen.getByRole('button', { name: 'Regex' }));
    fireEvent.change(screen.getByLabelText('Search'), { target: { value: '(' } });

    await act(async () => {
      vi.advanceTimersByTime(250);
      await Promise.resolve();
    });

    expect(screen.getByRole('status').textContent).toBe('regex parse error: unclosed group');
    expect(screen.getByText('App.tsx')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Open src/App.tsx at line 3' })).toBeTruthy();
  });

  it('shows the no-workspace state', () => {
    harness.setSnapshot({ ...readyState(), bootstrap: makeBootstrap(), searchResults: null });
    render(<SearchPanelView bootstrap={makeBootstrap()} />);
    expect(screen.getByText('Open a folder to search')).toBeTruthy();
  });
});

