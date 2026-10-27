import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { makeBootstrap } from '../../testing/fixtures';
import type { BootstrapState } from '../../bridge/bridge';

const harness = vi.hoisted(() => {
  let snapshot: unknown = null;
  const listeners = new Set<() => void>();
  return {
    gitStage: vi.fn(),
    gitUnstage: vi.fn(),
    gitCommit: vi.fn(),
    openGitDiff: vi.fn(),
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
  gitStage: harness.gitStage,
  gitUnstage: harness.gitUnstage,
  gitCommit: harness.gitCommit,
  openGitDiff: harness.openGitDiff,
}));

vi.mock('../../app/app-store', () => ({
  getAppSnapshot: harness.getSnapshot,
  subscribeApp: harness.subscribe,
  currentEnvelopeContext: harness.context,
}));

vi.mock('../editor/editor-store', () => ({
  editorStore: {
    openPath: vi.fn(),
    openPathAtLine: vi.fn(),
  },
}));

import { GitPanelView } from './GitPanelView';

/** P2's additive wire field — old payloads simply omit `files`. */
type GitPayload = {
  branch: string | null;
  changed_paths: string[];
  staged_paths: string[];
  files?: { path: string; staged: boolean; worktree: boolean }[];
};

function makeGitState(git: GitPayload) {
  const bootstrap = makeBootstrap({
    workspace: {
      ...makeBootstrap().workspace,
      id: 'ws-1',
      root: 'C:/workspace',
      display_name: 'workspace',
    },
    // `files` rides a payload the shared bridge type has not learned yet.
    git: git as unknown as BootstrapState['git'],
  });
  return { phase: 'ready', bootstrap, error: null, searchResults: null, gitDiff: null };
}

function readyState() {
  return makeGitState({
    branch: 'main',
    changed_paths: ['src/App.tsx'],
    staged_paths: ['src/README.md'],
  });
}

function section(title: 'STAGED CHANGES' | 'CHANGES'): HTMLElement {
  const element = screen.getByText(title).closest('.git-changes-section');
  if (!element) throw new Error('missing ' + title + ' section');
  return element as HTMLElement;
}

describe('GitPanelView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.gitStage.mockResolvedValue(undefined);
    harness.gitUnstage.mockResolvedValue(undefined);
    harness.gitCommit.mockResolvedValue(undefined);
    harness.openGitDiff.mockResolvedValue(undefined);
    harness.setSnapshot(readyState());
  });

  it('stages and unstages with the bridge and commits the real message', async () => {
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    expect(screen.getByText('main')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Stage App.tsx' }));
    await waitFor(() =>
      expect(harness.gitStage).toHaveBeenCalledWith('src/App.tsx', expect.any(Object)),
    );

    fireEvent.click(screen.getByRole('button', { name: 'Unstage README.md' }));
    await waitFor(() =>
      expect(harness.gitUnstage).toHaveBeenCalledWith('src/README.md', expect.any(Object)),
    );

    const message = screen.getByLabelText('Commit message');
    fireEvent.change(message, { target: { value: 'feat: live git panel' } });
    fireEvent.click(screen.getByRole('button', { name: /^Commit/ }));

    await waitFor(() =>
      expect(harness.gitCommit).toHaveBeenCalledWith('feat: live git panel', expect.any(Object)),
    );
    await waitFor(() => expect((message as HTMLTextAreaElement).value).toBe(''));
  });

  it('loads a real unified diff and colors added lines', async () => {
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    fireEvent.click(screen.getByRole('button', { name: 'View diff for src/App.tsx' }));

    await waitFor(() =>
      expect(harness.openGitDiff).toHaveBeenCalledWith('src/App.tsx', expect.any(Object)),
    );

    await act(async () => {
      harness.setSnapshot({
        ...readyState(),
        gitDiff: {
          path: 'src/App.tsx',
          unified_diff: 'diff --git a/src/App.tsx b/src/App.tsx\n@@ -1 +1 @@\n-const old = 1;\n+const next = 2;',
        },
      });
      await Promise.resolve();
    });

    const added = screen.getByText('+const next = 2;').closest('.git-diff-line');
    const removed = screen.getByText('-const old = 1;').closest('.git-diff-line');
    expect(added!.classList.contains('is-add')).toBe(true);
    expect(removed!.classList.contains('is-del')).toBe(true);
  });

  it('shows the no-repository state without inventing a branch', () => {
    const empty = { ...readyState(), bootstrap: makeBootstrap() };
    harness.setSnapshot(empty);
    render(<GitPanelView bootstrap={makeBootstrap()} />);
    expect(screen.getByText('No Git repository in this workspace')).toBeTruthy();
  });

  it('renders a file with both slots in the staged and unstaged sections', async () => {
    const state = makeGitState({
      branch: 'main',
      // The legacy arrays are flattened: the path sits in both, which used to
      // subtract it from CHANGES and hide (and strand) its unstaged delta.
      changed_paths: ['src/both.ts'],
      staged_paths: ['src/both.ts'],
      files: [{ path: 'src/both.ts', staged: true, worktree: true }],
    });
    harness.setSnapshot(state);
    render(<GitPanelView bootstrap={state.bootstrap} />);

    expect(
      within(section('STAGED CHANGES')).getByRole('button', { name: 'Unstage both.ts' }),
    ).toBeTruthy();
    expect(within(section('CHANGES')).getByRole('button', { name: 'Stage both.ts' })).toBeTruthy();
    expect(screen.getAllByRole('button', { name: 'View diff for src/both.ts' })).toHaveLength(2);

    // Each row acts on its own slot: the unstaged row stages, the staged row
    // unstages.
    fireEvent.click(within(section('CHANGES')).getByRole('button', { name: 'Stage both.ts' }));
    await waitFor(() =>
      expect(harness.gitStage).toHaveBeenCalledWith('src/both.ts', expect.any(Object)),
    );
    fireEvent.click(
      within(section('STAGED CHANGES')).getByRole('button', { name: 'Unstage both.ts' }),
    );
    await waitFor(() =>
      expect(harness.gitUnstage).toHaveBeenCalledWith('src/both.ts', expect.any(Object)),
    );
    // The CHANGES row staged the worktree side; the STAGED row unstage the
    // index side. Both calls are asserted so the buttons cannot swap slots.
    expect(harness.gitStage).toHaveBeenCalledWith('src/both.ts', expect.any(Object));
    expect(harness.gitUnstage).toHaveBeenCalledWith('src/both.ts', expect.any(Object));
  });

  it('keeps staged-only and untracked-only files in their own sections', () => {
    const state = makeGitState({
      branch: 'main',
      changed_paths: ['src/staged.ts', 'src/untracked.ts'],
      staged_paths: ['src/staged.ts'],
      files: [
        { path: 'src/staged.ts', staged: true, worktree: false },
        { path: 'src/untracked.ts', staged: false, worktree: true },
      ],
    });
    harness.setSnapshot(state);
    render(<GitPanelView bootstrap={state.bootstrap} />);

    const staged = section('STAGED CHANGES');
    const changed = section('CHANGES');
    expect(within(staged).getByRole('button', { name: 'Unstage staged.ts' })).toBeTruthy();
    expect(within(staged).queryByRole('button', { name: 'Stage staged.ts' })).toBeNull();
    expect(within(changed).queryByRole('button', { name: 'Stage staged.ts' })).toBeNull();
    expect(within(changed).getByRole('button', { name: 'Stage untracked.ts' })).toBeTruthy();
    expect(within(changed).queryByRole('button', { name: 'Unstage untracked.ts' })).toBeNull();
    expect(within(staged).queryByRole('button', { name: 'Unstage untracked.ts' })).toBeNull();
  });

  it('falls back to the legacy arrays when the payload has no files', () => {
    const state = makeGitState({
      branch: 'main',
      changed_paths: ['src/App.tsx'],
      staged_paths: ['src/README.md'],
    });
    harness.setSnapshot(state);
    render(<GitPanelView bootstrap={state.bootstrap} />);

    expect(
      within(section('STAGED CHANGES')).getByRole('button', { name: 'Unstage README.md' }),
    ).toBeTruthy();
    expect(within(section('CHANGES')).getByRole('button', { name: 'Stage App.tsx' })).toBeTruthy();
    // The legacy shape keeps its old subtraction rule: a path in both arrays
    // stays staged-only, because there is no slot information to go on.
    const flattened = makeGitState({
      branch: 'main',
      changed_paths: ['src/App.tsx', 'src/README.md'],
      staged_paths: ['src/README.md'],
    });
    act(() => harness.setSnapshot(flattened));
    expect(
      within(section('CHANGES')).queryByRole('button', { name: 'Stage README.md' }),
    ).toBeNull();
  });
});

