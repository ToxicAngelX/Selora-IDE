import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => {
  let snapshot: unknown = null;
  const listeners = new Set<() => void>();
  return {
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
  };
});

vi.mock('../../app/app-store', () => ({
  getAppSnapshot: harness.getSnapshot,
  subscribeApp: harness.subscribe,
}));

vi.mock('../editor/editor-store', () => ({
  editorStore: {
    openPath: harness.openPath,
    openPathAtLine: harness.openPathAtLine,
  },
}));

import { ProblemsPanel } from './ProblemsPanel';

function readyState() {
  const bootstrap = makeBootstrap({
    diagnostics: [
      {
        path: 'src/main.ts',
        line: 3,
        column: 5,
        severity: 'error',
        message: 'Type string is not assignable',
      },
      {
        path: 'src/main.ts',
        line: 8,
        column: 2,
        severity: 'warning',
        message: 'Unused import',
      },
      {
        path: 'src/util.ts',
        line: 1,
        column: 1,
        severity: 'hint',
        message: 'Consider a named export',
      },
    ],
  });
  return { phase: 'ready', bootstrap, error: null, searchResults: null, gitDiff: null };
}

describe('ProblemsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.setSnapshot(readyState());
  });

  it('groups live diagnostics by file and severity and shows counts', () => {
    render(<ProblemsPanel />);

    expect(screen.getByTitle('src/main.ts')).toBeTruthy();
    expect(screen.getByTitle('src/util.ts')).toBeTruthy();
    expect(screen.getByRole('tab', { name: /All/ }).textContent).toContain('3');
    expect(screen.getByRole('tab', { name: /Errors/ }).textContent).toContain('1');
    expect(screen.getByRole('tab', { name: /Warnings/ }).textContent).toContain('1');
    expect(screen.getByRole('tab', { name: /Hints/ }).textContent).toContain('1');
    expect(screen.getAllByText('Errors').length).toBeGreaterThanOrEqual(2);
  });

  it('opens the selected diagnostic at its line through the editor store', () => {
    render(<ProblemsPanel />);
    fireEvent.click(
      screen.getByRole('button', { name: 'Open src/main.ts at line 3, column 5' }),
    );
    // LSP lines are 0-based; the diagnostic on line 3 reveals 1-based line 4.
    expect(harness.openPathAtLine).toHaveBeenCalledWith('src/main.ts', 4);
  });

  it('clamps a 0-based line 0 diagnostic to the first line', () => {
    harness.setSnapshot({
      phase: 'ready',
      bootstrap: makeBootstrap({
        diagnostics: [
          {
            path: 'src/main.ts',
            line: 0,
            column: 0,
            severity: 'error',
            message: 'Missing semicolon',
          },
        ],
      }),
      error: null,
      searchResults: null,
      gitDiff: null,
    });
    render(<ProblemsPanel />);
    fireEvent.click(
      screen.getByRole('button', { name: 'Open src/main.ts at line 0, column 0' }),
    );
    expect(harness.openPathAtLine).toHaveBeenCalledWith('src/main.ts', 1);
  });
});

