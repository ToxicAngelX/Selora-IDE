import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, waitFor, within } from '@testing-library/react';
import type { BootstrapState, BufferSnapshot } from '../bridge/bridge';
import { makeBootstrap } from '../testing/fixtures';

const harness = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  const holder: { snapshot: unknown } = {
    snapshot: { phase: 'connecting', bootstrap: null, error: null },
  };
  return {
    app: {
      subscribe(listener: () => void) {
        listeners.add(listener);
        return () => {
          listeners.delete(listener);
        };
      },
      getSnapshot() {
        return holder.snapshot;
      },
      setSnapshot(next: unknown) {
        holder.snapshot = next;
        for (const listener of listeners) listener();
      },
      currentEnvelopeContext() {
        return { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null };
      },
    },
    bridge: {
      pickFolder: vi.fn(() => Promise.resolve('/picked/workspace')),
      openFolderAt: vi.fn(() => Promise.resolve(undefined)),
      openFile: vi.fn(() => Promise.resolve(undefined)),
      closeBuffer: vi.fn(() => Promise.resolve(undefined)),
      saveBuffer: vi.fn(() => Promise.resolve(undefined)),
      setActiveBuffer: vi.fn(() => Promise.resolve(undefined)),
      applyEditorChange: vi.fn(() => Promise.resolve(undefined)),
      setSelection: vi.fn(() => Promise.resolve(undefined)),
    },
  };
});

vi.mock('../app/app-store', () => ({
  getAppSnapshot: harness.app.getSnapshot,
  subscribeApp: harness.app.subscribe,
  currentEnvelopeContext: harness.app.currentEnvelopeContext,
}));

vi.mock('../bridge/bridge', () => ({
  pickFolder: harness.bridge.pickFolder,
  openFolderAt: harness.bridge.openFolderAt,
  openFile: harness.bridge.openFile,
  closeBuffer: harness.bridge.closeBuffer,
  saveBuffer: harness.bridge.saveBuffer,
  setActiveBuffer: harness.bridge.setActiveBuffer,
  applyEditorChange: harness.bridge.applyEditorChange,
  setSelection: harness.bridge.setSelection,
}));

vi.mock('../features/editor/CodeMirrorPane', async () => {
  const React = await import('react');
  return {
    CodeMirrorPane: ({ buffer }: { buffer: BufferSnapshot }) =>
      React.createElement('div', { 'data-testid': 'code-pane' }, buffer.path),
  };
});

import { EditorRegion } from './EditorRegion';
import { editorStore } from '../features/editor/editor-store';
import { toastStore } from '../features/toasts/toast-store';
import { ToastViewport } from '../components/ToastViewport';

function buffer(id: string, path: string): BufferSnapshot {
  return {
    buffer_id: id,
    path,
    language: 'rust',
    text: 'fn main() {}\n',
    version: 1,
    dirty: false,
    selection: null,
  };
}

function dirtyBuffer(id: string, path: string): BufferSnapshot {
  return { ...buffer(id, path), dirty: true };
}

function readyApp(overrides: Partial<BootstrapState> = {}): {
  phase: 'ready';
  bootstrap: BootstrapState;
  error: null;
} {
  return {
    phase: 'ready',
    bootstrap: makeBootstrap({
      workspace: {
        id: 'ws-1',
        core_instance_id: 'core-0001',
        root: 'C:/workspace',
        display_name: 'selora-workspace',
      },
      file_tree: {
        root: {
          path: 'C:/workspace',
          name: 'selora-workspace',
          kind: 'directory',
          children: [
            { path: 'C:/workspace/README.md', name: 'README.md', kind: 'file', children: [] },
            { path: 'C:/workspace/src', name: 'src', kind: 'directory', children: [] },
          ],
        },
      },
      ...overrides,
    }),
    error: null,
  };
}

describe('EditorRegion', () => {
  beforeEach(() => {
    harness.bridge.pickFolder.mockClear();
    harness.bridge.openFolderAt.mockClear();
    harness.bridge.openFile.mockClear();
    harness.bridge.closeBuffer.mockClear();
    harness.bridge.saveBuffer.mockClear();
    toastStore.clearAll();
    harness.app.setSnapshot(readyApp());
    editorStore.reset();
  });

  afterEach(() => {
    toastStore.clearAll();
  });

  it('renders protocol buffers as tabs and switches the active pane', async () => {
    harness.app.setSnapshot(
      readyApp({
        buffers: [
          buffer('b1', 'C:/workspace/src/main.rs'),
          buffer('b2', 'C:/workspace/src/lib.rs'),
        ],
        active_buffer_id: 'b1',
      }),
    );
    editorStore.reset();

    const { getByRole, findByTestId } = render(<EditorRegion />);

    const mainTab = getByRole('tab', { name: 'main.rs' });
    const libTab = getByRole('tab', { name: 'lib.rs' });
    expect(mainTab.tagName).toBe('BUTTON');
    expect(mainTab.getAttribute('aria-selected')).toBe('true');
    expect(libTab.getAttribute('aria-selected')).toBe('false');
    // The pane is a lazy chunk (Z6 bundle split); waiting for it also covers
    // the Suspense fallback path.
    expect((await findByTestId('code-pane')).textContent).toBe('C:/workspace/src/main.rs');

    fireEvent.click(libTab);

    expect(libTab.getAttribute('aria-selected')).toBe('true');
    expect((await findByTestId('code-pane')).textContent).toBe('C:/workspace/src/lib.rs');
    const breadcrumbs = getByRole('navigation', { name: 'Breadcrumb navigation' });
    expect(within(breadcrumbs).getByText('lib.rs')).toBeTruthy();
  });

  it('moves tab focus with the arrow keys', () => {
    harness.app.setSnapshot(
      readyApp({
        buffers: [
          buffer('b1', 'C:/workspace/src/main.rs'),
          buffer('b2', 'C:/workspace/src/lib.rs'),
        ],
        active_buffer_id: 'b1',
      }),
    );
    editorStore.reset();

    const { getByRole, getByTestId } = render(<EditorRegion />);
    fireEvent.keyDown(getByRole('tab', { name: 'main.rs' }), { key: 'ArrowRight' });

    expect(getByRole('tab', { name: 'lib.rs' }).getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(getByRole('tab', { name: 'lib.rs' }));
    expect(getByTestId('code-pane').textContent).toBe('C:/workspace/src/lib.rs');
  });

  it('offers a live Open Folder action when no workspace is open', async () => {
    harness.app.setSnapshot(
      readyApp({
        workspace: {
          id: 'no-workspace',
          core_instance_id: 'core-0001',
          root: null,
          display_name: 'No workspace open',
        },
        file_tree: { root: null },
      }),
    );
    editorStore.reset();

    const { getByRole, getByText } = render(<EditorRegion />);
    expect(getByText('No folder open')).toBeTruthy();

    const openFolder = getByRole('button', { name: 'Open Folder' });
    expect(openFolder.hasAttribute('disabled')).toBe(false);
    fireEvent.click(openFolder);

    await waitFor(() => {
      expect(harness.bridge.pickFolder).toHaveBeenCalledTimes(1);
    });
    expect(harness.bridge.openFolderAt).toHaveBeenCalledWith(
      '/picked/workspace',
      { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
    );
  });

  it('surfaces an error toast when the folder picker rejects', async () => {
    harness.app.setSnapshot(
      readyApp({
        workspace: {
          id: 'no-workspace',
          core_instance_id: 'core-0001',
          root: null,
          display_name: 'No workspace open',
        },
        file_tree: { root: null },
      }),
    );
    editorStore.reset();
    harness.bridge.pickFolder.mockRejectedValueOnce(new Error('permission denied'));

    const { getByRole, findByText } = render(
      <>
        <EditorRegion />
        <ToastViewport />
      </>,
    );

    fireEvent.click(getByRole('button', { name: 'Open Folder' }));

    const alert = await findByText('Open folder failed: permission denied');
    expect(alert.closest('[role="status"]')).not.toBeNull();
    expect(harness.bridge.openFolderAt).not.toHaveBeenCalled();
  });

  it('points at the Files sidebar with top-level file chips when no buffer is open', async () => {
    const { getByRole, getByText } = render(<EditorRegion />);
    expect(getByText('Open a file from the Files sidebar')).toBeTruthy();

    const chip = getByRole('button', { name: /README\.md/i });
    fireEvent.click(chip);

    await waitFor(() => {
      expect(harness.bridge.openFile).toHaveBeenCalledWith(
        'C:/workspace/README.md',
        { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
      );
    });
  });

  it('closes a tab through the bridge', () => {
    harness.app.setSnapshot(
      readyApp({
        buffers: [buffer('b1', 'C:/workspace/src/main.rs')],
        active_buffer_id: 'b1',
      }),
    );
    editorStore.reset();

    const { getByRole } = render(<EditorRegion />);
    fireEvent.click(getByRole('button', { name: 'Close main.rs' }));

    expect(harness.bridge.closeBuffer).toHaveBeenCalledWith(
      'b1',
      { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
    );
  });

  it('guards a dirty tab close behind Save, Discard, or Cancel', async () => {
    harness.app.setSnapshot(
      readyApp({
        buffers: [dirtyBuffer('b1', 'C:/workspace/src/main.rs')],
        active_buffer_id: 'b1',
      }),
    );
    editorStore.reset();

    const { getByRole } = render(<EditorRegion />);
    fireEvent.click(getByRole('button', { name: 'Close main.rs' }));

    const dialog = getByRole('dialog', { name: 'Unsaved changes in main.rs' });
    expect(within(dialog).getByRole('button', { name: 'Save' })).toBeTruthy();
    expect(harness.bridge.closeBuffer).not.toHaveBeenCalled();

    fireEvent.click(within(dialog).getByRole('button', { name: 'Save' }));

    await waitFor(() => {
      expect(harness.bridge.saveBuffer).toHaveBeenCalledWith(
        'b1',
        { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
      );
      expect(harness.bridge.closeBuffer).toHaveBeenCalledWith(
        'b1',
        { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
      );
    });
  });
});
