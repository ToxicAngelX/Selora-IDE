import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { makeBootstrap } from '../../testing/fixtures';
import type { FileTreeNode } from '../../bridge/bridge';

const harness = vi.hoisted(() => {
  let snapshot: unknown = null;
  const listeners = new Set<() => void>();
  return {
    openPath: vi.fn(),
    openPathAtLine: vi.fn(),
    pickFolder: vi.fn(),
    openFolderAt: vi.fn(),
    refreshTree: vi.fn(),
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
  pickFolder: harness.pickFolder,
  openFolderAt: harness.openFolderAt,
  refreshTree: harness.refreshTree,
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

import { FileTreeView } from './FileTreeView';

const tree: FileTreeNode = {
  path: 'workspace',
  name: 'workspace',
  kind: 'directory',
  children: [
    {
      path: 'src',
      name: 'src',
      kind: 'directory',
      children: [{ path: 'src/App.tsx', name: 'App.tsx', kind: 'file', children: [] }],
    },
    { path: 'README.md', name: 'README.md', kind: 'file', children: [] },
  ],
};

function readyState(fileRoot: FileTreeNode | null) {
  const bootstrap = makeBootstrap({
    workspace: {
      ...makeBootstrap().workspace,
      id: 'ws-1',
      root: fileRoot ? 'C:/workspace' : null,
      display_name: 'workspace',
    },
    file_tree: { root: fileRoot },
    buffers: [
      {
        buffer_id: 'buffer-1',
        path: 'src/App.tsx',
        language: 'tsx',
        text: 'export default function App() {}',
        version: 1,
        dirty: true,
        selection: null,
      },
    ],
    active_buffer_id: 'buffer-1',
  });
  return { phase: 'ready', bootstrap, error: null, searchResults: null, gitDiff: null };
}

describe('FileTreeView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.pickFolder.mockResolvedValue(null);
    harness.openFolderAt.mockResolvedValue(undefined);
    harness.refreshTree.mockResolvedValue(undefined);
    harness.setSnapshot(readyState(tree));
  });

  it('renders the live tree, reveals nested nodes, and opens a file through the editor store', () => {
    render(<FileTreeView bootstrap={readyState(tree).bootstrap} />);

    expect(screen.getByText('2 files')).toBeTruthy();
    expect(screen.getByText('src')).toBeTruthy();
    expect(screen.queryByText('App.tsx')).toBeNull();

    fireEvent.click(screen.getByText('src'));
    const appRow = screen.getByText('App.tsx').closest('[role="treeitem"]');
    expect(appRow).not.toBeNull();
    expect(appRow!.getAttribute('aria-expanded')).toBeNull();
    expect(screen.getByLabelText('Modified')).toBeTruthy();

    fireEvent.click(appRow!);
    expect(harness.openPath).toHaveBeenCalledWith('src/App.tsx');
    expect(appRow!.classList.contains('is-selected')).toBe(true);
  });

  it('keeps one roving tab stop and moves focus with arrow, Home, and End keys', () => {
    render(<FileTreeView bootstrap={readyState(tree).bootstrap} />);

    const rows = () => screen.getAllByRole('treeitem') as HTMLElement[];
    expect(rows().map((row) => row.querySelector('.tree-item-name')?.textContent)).toEqual([
      'workspace',
      'src',
      'README.md',
    ]);

    const tabStops = () => rows().filter((row) => row.getAttribute('tabindex') === '0');
    expect(tabStops()).toHaveLength(1);
    expect(tabStops()[0].textContent).toBe('workspace');
    for (const row of rows().slice(1)) {
      expect(row.getAttribute('tabindex')).toBe('-1');
    }

    const [rootRow] = rows();
    const srcRow = rows()[1];
    rootRow.focus();
    fireEvent.keyDown(rootRow, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(srcRow);
    expect(srcRow.getAttribute('tabindex')).toBe('0');
    expect(rootRow.getAttribute('tabindex')).toBe('-1');

    // Enter expands the focused directory and the next ArrowDown descends
    // into the newly revealed child.
    fireEvent.keyDown(srcRow, { key: 'Enter' });
    expect(srcRow.getAttribute('aria-expanded')).toBe('true');
    fireEvent.keyDown(srcRow, { key: 'ArrowDown' });
    const appRow = screen.getByText('App.tsx').closest('[role="treeitem"]') as HTMLElement;
    expect(document.activeElement).toBe(appRow);

    fireEvent.keyDown(appRow, { key: 'End' });
    const readmeRow = screen.getByText('README.md').closest('[role="treeitem"]') as HTMLElement;
    expect(document.activeElement).toBe(readmeRow);

    fireEvent.keyDown(readmeRow, { key: 'Home' });
    expect(document.activeElement).toBe(rows()[0]);
    expect(tabStops()).toHaveLength(1);

    // Space activates the focused file row through the editor store.
    fireEvent.keyDown(appRow, { key: ' ' });
    expect(harness.openPath).toHaveBeenCalledWith('src/App.tsx');
  });

  it('refreshes the live tree with the envelope context', async () => {
    render(<FileTreeView bootstrap={readyState(tree).bootstrap} />);
    fireEvent.click(screen.getByRole('button', { name: 'Refresh Explorer' }));

    await waitFor(() => expect(harness.refreshTree).toHaveBeenCalledTimes(1));
    expect(harness.refreshTree).toHaveBeenCalledWith(
      expect.objectContaining({ core_instance_id: 'core-1', workspace_id: 'ws-1' }),
    );
  });

  it('offers a live Open Folder action when no folder is open', async () => {
    harness.setSnapshot(readyState(null));
    harness.pickFolder.mockResolvedValue('C:/workspace');
    render(<FileTreeView bootstrap={readyState(null).bootstrap} />);

    expect(screen.getByText('No folder open')).toBeTruthy();
    const button = screen.getByRole('button', { name: 'Open Folder' });
    expect(button.hasAttribute('disabled')).toBe(false);

    fireEvent.click(button);
    await waitFor(() => expect(harness.pickFolder).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(harness.openFolderAt).toHaveBeenCalledWith(
        'C:/workspace',
        expect.objectContaining({ core_instance_id: 'core-1', workspace_id: 'ws-1' }),
      ),
    );
  });
});

