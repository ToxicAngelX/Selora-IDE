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
    openFile: vi.fn(),
    gitBranches: vi.fn(),
    gitCreateBranch: vi.fn(),
    gitSwitchBranch: vi.fn(),
    gitDeleteBranch: vi.fn(),
    gitLog: vi.fn(),
    gitShowCommit: vi.fn(),
    gitDiscard: vi.fn(),
    gitStashList: vi.fn(),
    gitStashSave: vi.fn(),
    gitStashApply: vi.fn(),
    gitStashPop: vi.fn(),
    gitStashDrop: vi.fn(),
    gitMerge: vi.fn(),
    gitConflictResolve: vi.fn(),
    gitStageAll: vi.fn(),
    gitUnstageAll: vi.fn(),
    gitPush: vi.fn(),
    gitPull: vi.fn(),
    gitFetch: vi.fn(),
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
  openFile: harness.openFile,
  gitBranches: harness.gitBranches,
  gitCreateBranch: harness.gitCreateBranch,
  gitSwitchBranch: harness.gitSwitchBranch,
  gitDeleteBranch: harness.gitDeleteBranch,
  gitLog: harness.gitLog,
  gitShowCommit: harness.gitShowCommit,
  gitDiscard: harness.gitDiscard,
  gitStashList: harness.gitStashList,
  gitStashSave: harness.gitStashSave,
  gitStashApply: harness.gitStashApply,
  gitStashPop: harness.gitStashPop,
  gitStashDrop: harness.gitStashDrop,
  gitMerge: harness.gitMerge,
  gitConflictResolve: harness.gitConflictResolve,
  gitStageAll: harness.gitStageAll,
  gitUnstageAll: harness.gitUnstageAll,
  gitPush: harness.gitPush,
  gitPull: harness.gitPull,
  gitFetch: harness.gitFetch,
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
  conflicts?: string[];
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

async function openBranchMenu() {
  fireEvent.click(screen.getByRole('button', { name: 'Branch: main' }));
  return screen.findByRole('region', { name: 'Branches' });
}

async function openStashMenu() {
  fireEvent.click(screen.getByRole('button', { name: 'Stash menu' }));
  return screen.findByRole('region', { name: 'Stashes' });
}

async function openHistory() {
  fireEvent.click(screen.getByRole('button', { name: /HISTORY/ }));
  await waitFor(() => expect(harness.gitLog).toHaveBeenCalled());
}

function confirmDialog(): HTMLElement {
  return screen.getByRole('dialog');
}

describe('GitPanelView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    for (const mock of Object.values(harness)) {
      if (typeof mock === 'function' && 'mockReset' in mock) {
        (mock as { mockReset: () => void }).mockReset();
      }
    }
    harness.gitStage.mockResolvedValue(undefined);
    harness.gitUnstage.mockResolvedValue(undefined);
    harness.gitCommit.mockResolvedValue(undefined);
    harness.openGitDiff.mockResolvedValue(undefined);
    harness.openFile.mockResolvedValue(undefined);
    harness.gitBranches.mockResolvedValue([]);
    harness.gitCreateBranch.mockResolvedValue(undefined);
    harness.gitSwitchBranch.mockResolvedValue(undefined);
    harness.gitDeleteBranch.mockResolvedValue(undefined);
    harness.gitLog.mockResolvedValue([]);
    harness.gitShowCommit.mockResolvedValue(null);
    harness.gitDiscard.mockResolvedValue(undefined);
    harness.gitStashList.mockResolvedValue([]);
    harness.gitStashSave.mockResolvedValue(undefined);
    harness.gitStashApply.mockResolvedValue(undefined);
    harness.gitStashPop.mockResolvedValue(undefined);
    harness.gitStashDrop.mockResolvedValue(undefined);
    harness.gitMerge.mockResolvedValue(undefined);
    harness.gitConflictResolve.mockResolvedValue(undefined);
    harness.gitStageAll.mockResolvedValue(undefined);
    harness.gitUnstageAll.mockResolvedValue(undefined);
    harness.gitPush.mockResolvedValue(undefined);
    harness.gitPull.mockResolvedValue(undefined);
    harness.gitFetch.mockResolvedValue(undefined);
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

  it('reports a commit failure without clearing the message', async () => {
    harness.gitCommit.mockRejectedValue(new Error('nothing staged to commit'));
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    const message = screen.getByLabelText('Commit message');
    fireEvent.change(message, { target: { value: 'feat: nope' } });
    fireEvent.click(screen.getByRole('button', { name: /^Commit/ }));

    expect(await screen.findByText('nothing staged to commit')).toBeTruthy();
    expect((message as HTMLTextAreaElement).value).toBe('feat: nope');
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
    const stagedUnstage = within(section('STAGED CHANGES')).getByRole('button', {
      name: 'Unstage both.ts',
    });
    // Wait for the stage action's busy flag to clear: a click on the disabled
    // button would be a no-op and would not prove the slot mapping.
    await waitFor(() => expect((stagedUnstage as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(stagedUnstage);
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

  it('stages all and unstages all through the additive commands', async () => {
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    fireEvent.click(screen.getByRole('button', { name: 'Stage all changes' }));
    await waitFor(() => expect(harness.gitStageAll).toHaveBeenCalledWith(expect.any(Object)));
    fireEvent.click(screen.getByRole('button', { name: 'Unstage all changes' }));
    await waitFor(() => expect(harness.gitUnstageAll).toHaveBeenCalledWith(expect.any(Object)));
  });

  it('disables stage-all/unstage-all when there is nothing to act on', () => {
    const state = makeGitState({ branch: 'main', changed_paths: [], staged_paths: [] });
    harness.setSnapshot(state);
    render(<GitPanelView bootstrap={state.bootstrap} />);
    expect(
      (screen.getByRole('button', { name: 'Stage all changes' }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(
      (screen.getByRole('button', { name: 'Unstage all changes' }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it('loads only the branches the backend reports and switches branch', async () => {
    harness.gitBranches.mockResolvedValue([
      { name: 'main', current: true },
      { name: 'feature/git-ui', current: false },
    ]);
    render(<GitPanelView bootstrap={readyState().bootstrap} />);

    const menu = await openBranchMenu();
    await waitFor(() =>
      expect(within(menu).getByRole('button', { name: 'Current branch main' })).toBeTruthy(),
    );
    expect(
      within(menu).queryByRole('button', { name: 'Switch to main' }),
    ).toBeNull();
    fireEvent.click(within(menu).getByRole('button', { name: 'Switch to feature/git-ui' }));
    await waitFor(() =>
      expect(harness.gitSwitchBranch).toHaveBeenCalledWith('feature/git-ui', expect.any(Object)),
    );
  });

  it('renders the named-key replies the desktop backend sends', async () => {
    harness.gitBranches.mockResolvedValue({
      branches: [
        { name: 'main', current: true },
        { name: 'topic', current: false },
      ],
    });
    harness.gitLog.mockResolvedValue({
      commits: [
        {
          sha: 'abcdef1234567890',
          summary: 'feat: backend shape',
          author: 'Ada',
          timestamp: 1700000000,
        },
      ],
    });
    harness.gitStashList.mockResolvedValue({
      stashes: [{ index: 0, message: 'wip: backend shape' }],
    });
    render(<GitPanelView bootstrap={readyState().bootstrap} />);

    const menu = await openBranchMenu();
    expect(
      await within(menu).findByRole('button', { name: 'Switch to topic' }),
    ).toBeTruthy();

    await openHistory();
    expect(await screen.findByRole('button', { name: 'Show commit abcdef1' })).toBeTruthy();

    const stash = await openStashMenu();
    expect(await within(stash).findByRole('button', { name: 'Apply stash 0' })).toBeTruthy();
  });

  it('shows a branch-list error and no invented rows when the backend is absent', async () => {
    harness.gitBranches.mockRejectedValue(new Error('git_branches is not supported'));
    render(<GitPanelView bootstrap={readyState().bootstrap} />);

    const menu = await openBranchMenu();
    expect(
      await within(menu).findByText('Branch list unavailable: git_branches is not supported'),
    ).toBeTruthy();
    expect(within(menu).queryByRole('menu')).toBeNull();
    expect(within(menu).queryByRole('button', { name: /^Switch to/ })).toBeNull();
    // The real checked-out branch from bootstrap is still shown, not a fake list.
    expect(screen.getByText('main')).toBeTruthy();
  });

  it('creates a branch (and checks it out) from the branch menu', async () => {
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    const menu = await openBranchMenu();
    await within(menu).findByText('No branches reported.');

    fireEvent.change(within(menu).getByLabelText('New branch name'), {
      target: { value: '  feature/new  ' },
    });
    fireEvent.click(within(menu).getByRole('button', { name: 'Create branch' }));

    await waitFor(() =>
      expect(harness.gitCreateBranch).toHaveBeenCalledWith(
        'feature/new',
        true,
        expect.any(Object),
      ),
    );
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Branches' })).toBeNull());
  });

  it('confirms before deleting a branch and does nothing on cancel', async () => {
    harness.gitBranches.mockResolvedValue([
      { name: 'main', current: true },
      { name: 'old', current: false },
    ]);
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    const menu = await openBranchMenu();
    fireEvent.click(await within(menu).findByRole('button', { name: 'Delete branch old' }));

    expect(within(confirmDialog()).getByText(/Delete branch "old"/)).toBeTruthy();
    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Cancel' }));
    expect(harness.gitDeleteBranch).not.toHaveBeenCalled();

    fireEvent.click(within(menu).getByRole('button', { name: 'Delete branch old' }));
    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Delete branch' }));
    await waitFor(() =>
      expect(harness.gitDeleteBranch).toHaveBeenCalledWith('old', expect.any(Object)),
    );
  });

  it('confirms before merging another branch into the current one', async () => {
    harness.gitBranches.mockResolvedValue([
      { name: 'main', current: true },
      { name: 'topic', current: false },
    ]);
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    const menu = await openBranchMenu();
    fireEvent.click(await within(menu).findByRole('button', { name: 'Merge topic into main' }));
    expect(within(confirmDialog()).getByText(/Merge branch "topic" into "main"/)).toBeTruthy();
    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Merge' }));
    await waitFor(() =>
      expect(harness.gitMerge).toHaveBeenCalledWith('topic', expect.any(Object)),
    );
  });

  it('loads history, shows a commit detail, and renders its diff', async () => {
    harness.gitLog.mockResolvedValue([
      { sha: 'abcdef1234567890', summary: 'feat: add git ui', author: 'Ada', timestamp: 1700000000 },
    ]);
    harness.gitShowCommit.mockResolvedValue({
      sha: 'abcdef1234567890',
      summary: 'feat: add git ui',
      author: 'Ada',
      timestamp: 1700000000,
      message: 'feat: add git ui\n\nReal body.',
      unified_diff: 'diff --git a/a.ts b/a.ts\n@@ -1 +1 @@\n+added line',
    });
    render(<GitPanelView bootstrap={readyState().bootstrap} />);

    await openHistory();
    await waitFor(() => expect(harness.gitLog).toHaveBeenCalledWith(50, expect.any(Object)));
    fireEvent.click(await screen.findByRole('button', { name: 'Show commit abcdef1' }));

    await waitFor(() =>
      expect(harness.gitShowCommit).toHaveBeenCalledWith(
        'abcdef1234567890',
        expect.any(Object),
      ),
    );
    expect(await screen.findByText(/Real body\./)).toBeTruthy();
    expect(screen.getByText('+added line')).toBeTruthy();
    const detailCard = document.querySelector('.git-commit-detail');
    if (!detailCard) throw new Error('missing commit detail card');
    expect(within(detailCard as HTMLElement).getByText('Ada · 2023-11-14')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Close commit details' }));
    expect(screen.queryByText(/Real body\./)).toBeNull();
  });

  it('reports history and commit-detail failures truthfully', async () => {
    harness.gitLog.mockRejectedValue(new Error('git_log is not supported'));
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    await openHistory();
    expect(await screen.findByText('History unavailable: git_log is not supported')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /^Show commit/ })).toBeNull();

    harness.gitLog.mockResolvedValue([
      { sha: 'deadbeef0000', summary: 'fix: thing', author: 'Bo', timestamp: 0 },
    ]);
    fireEvent.click(screen.getByRole('button', { name: 'Refresh history' }));
    await waitFor(() =>
      expect(harness.gitLog).toHaveBeenCalledTimes(2),
    );
    harness.gitShowCommit.mockRejectedValue(new Error('git_show_commit is not supported'));
    fireEvent.click(await screen.findByRole('button', { name: 'Show commit deadbee' }));
    expect(await screen.findByText('git_show_commit is not supported')).toBeTruthy();
  });

  it('discards a worktree file only after confirmation', async () => {
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    fireEvent.click(screen.getByRole('button', { name: 'Discard changes to src/App.tsx' }));
    expect(within(confirmDialog()).getByText(/Discard all changes to "src\/App\.tsx"/)).toBeTruthy();
    expect(harness.gitDiscard).not.toHaveBeenCalled();

    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Cancel' }));
    expect(harness.gitDiscard).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Discard changes to src/App.tsx' }));
    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Discard' }));
    await waitFor(() =>
      expect(harness.gitDiscard).toHaveBeenCalledWith('src/App.tsx', expect.any(Object)),
    );
    expect(await screen.findByText('Discarded changes in App.tsx.')).toBeTruthy();
  });

  it('saves, lists, applies, pops, and drops stashes', async () => {
    harness.gitStashList.mockResolvedValue([{ index: 0, message: 'wip: panel' }]);
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    const menu = await openStashMenu();
    await waitFor(() => expect(harness.gitStashList).toHaveBeenCalled());

    fireEvent.change(within(menu).getByLabelText('Stash message'), {
      target: { value: 'wip stash' },
    });
    fireEvent.click(within(menu).getByRole('button', { name: 'Stash changes' }));
    await waitFor(() =>
      expect(harness.gitStashSave).toHaveBeenCalledWith('wip stash', expect.any(Object)),
    );

    const apply = await within(menu).findByRole('button', { name: 'Apply stash 0' });
    await waitFor(() => expect((apply as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(apply);
    await waitFor(() =>
      expect(harness.gitStashApply).toHaveBeenCalledWith(0, expect.any(Object)),
    );
    const pop = within(menu).getByRole('button', { name: 'Pop stash 0' });
    await waitFor(() => expect((pop as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(pop);
    await waitFor(() => expect(harness.gitStashPop).toHaveBeenCalledWith(0, expect.any(Object)));

    fireEvent.click(within(menu).getByRole('button', { name: 'Drop stash 0' }));
    expect(within(confirmDialog()).getByText(/Drop stash@\{0\}/)).toBeTruthy();
    expect(harness.gitStashDrop).not.toHaveBeenCalled();
    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Drop stash' }));
    await waitFor(() =>
      expect(harness.gitStashDrop).toHaveBeenCalledWith(0, expect.any(Object)),
    );
  });

  it('shows no stash rows and an error when the stash backend is absent', async () => {
    harness.gitStashList.mockRejectedValue(new Error('git_stash_list is not supported'));
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    const menu = await openStashMenu();
    expect(
      await within(menu).findByText('Stash list unavailable: git_stash_list is not supported'),
    ).toBeTruthy();
    expect(within(menu).queryByRole('button', { name: /^Apply stash/ })).toBeNull();
  });

  it('disables stash save when the working tree is clean', async () => {
    const state = makeGitState({ branch: 'main', changed_paths: [], staged_paths: [] });
    harness.setSnapshot(state);
    render(<GitPanelView bootstrap={state.bootstrap} />);
    const menu = await openStashMenu();
    expect(
      (within(menu).getByRole('button', { name: 'Stash changes' }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it('fetches, pulls, and pushes with busy and error states', async () => {
    render(<GitPanelView bootstrap={readyState().bootstrap} />);

    fireEvent.click(screen.getByRole('button', { name: 'Fetch' }));
    await waitFor(() => expect(harness.gitFetch).toHaveBeenCalledWith(expect.any(Object)));

    fireEvent.click(screen.getByRole('button', { name: 'Pull' }));
    await waitFor(() => expect(harness.gitPull).toHaveBeenCalledWith(expect.any(Object)));

    let finishPush: (() => void) | undefined;
    harness.gitPush.mockReturnValue(
      new Promise<void>((resolve) => {
        finishPush = resolve;
      }),
    );
    const push = screen.getByRole('button', { name: 'Push' });
    fireEvent.click(push);
    await waitFor(() => expect((push as HTMLButtonElement).disabled).toBe(true));
    expect(push.getAttribute('aria-busy')).toBe('true');
    await act(async () => {
      finishPush?.();
      await Promise.resolve();
    });
    await waitFor(() => expect((push as HTMLButtonElement).disabled).toBe(false));
    expect(await screen.findByText('Pushed.')).toBeTruthy();

    harness.gitPush.mockRejectedValue(new Error('no configured remote'));
    fireEvent.click(push);
    expect(await screen.findByText('no configured remote')).toBeTruthy();
    await waitFor(() => expect((push as HTMLButtonElement).disabled).toBe(false));
  });

  it('resolves merge conflicts with ours/theirs and opens the file', async () => {
    const state = makeGitState({
      branch: 'main',
      changed_paths: ['src/a.ts'],
      staged_paths: [],
      conflicts: ['src/a.ts'],
    });
    harness.setSnapshot(state);
    render(<GitPanelView bootstrap={state.bootstrap} />);

    const banner = screen.getByRole('alert', { name: 'Merge conflicts' });
    expect(within(banner).getByText('src/a.ts')).toBeTruthy();

    fireEvent.click(within(banner).getByRole('button', { name: 'Use ours for src/a.ts' }));
    expect(within(confirmDialog()).getByText(/with the ours version/)).toBeTruthy();
    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Use ours' }));
    await waitFor(() =>
      expect(harness.gitConflictResolve).toHaveBeenCalledWith(
        'src/a.ts',
        'ours',
        expect.any(Object),
      ),
    );

    const theirs = within(banner).getByRole('button', { name: 'Use theirs for src/a.ts' });
    await waitFor(() => expect((theirs as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(theirs);
    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Use theirs' }));
    await waitFor(() =>
      expect(harness.gitConflictResolve).toHaveBeenCalledWith(
        'src/a.ts',
        'theirs',
        expect.any(Object),
      ),
    );

    const openConflict = within(banner).getByRole('button', { name: 'Open src/a.ts' });
    await waitFor(() => expect((openConflict as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(openConflict);
    await waitFor(() =>
      expect(harness.openFile).toHaveBeenCalledWith('src/a.ts', expect.any(Object)),
    );
  });

  it('shows no conflict banner when the payload has no conflicts', () => {
    render(<GitPanelView bootstrap={readyState().bootstrap} />);
    expect(screen.queryByRole('alert', { name: 'Merge conflicts' })).toBeNull();
  });

  it('surfaces failures for file actions, stage-all, stash save, fetch, and resolve', async () => {
    harness.gitStageAll.mockRejectedValue(new Error('stage-all failed'));
    harness.gitDiscard.mockRejectedValue(new Error('discard failed'));
    harness.gitStashSave.mockRejectedValue(new Error('stash failed'));
    harness.gitFetch.mockRejectedValue(new Error('fetch failed'));

    const state = makeGitState({
      branch: 'main',
      changed_paths: ['src/App.tsx'],
      staged_paths: [],
      conflicts: ['src/App.tsx'],
    });
    harness.setSnapshot(state);
    render(<GitPanelView bootstrap={state.bootstrap} />);

    fireEvent.click(screen.getByRole('button', { name: 'Stage all changes' }));
    expect(await screen.findByText('stage-all failed')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Discard changes to src/App.tsx' }));
    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Discard' }));
    expect(await screen.findByText('discard failed')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Fetch' }));
    expect(await screen.findByText('fetch failed')).toBeTruthy();

    const stash = await openStashMenu();
    fireEvent.click(within(stash).getByRole('button', { name: 'Stash changes' }));
    expect(await screen.findByText('stash failed')).toBeTruthy();

    harness.gitConflictResolve.mockRejectedValue(new Error('resolve failed'));
    fireEvent.click(
      screen.getByRole('button', { name: 'Use ours for src/App.tsx' }),
    );
    fireEvent.click(within(confirmDialog()).getByRole('button', { name: 'Use ours' }));
    expect(await screen.findByText('resolve failed')).toBeTruthy();
  });
});