import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BootstrapState, BufferSnapshot, Selection } from '../../bridge/bridge';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  const holder: { snapshot: unknown } = {
    snapshot: { phase: 'connecting', bootstrap: null, error: null },
  };
  return {
    app: {
      holder,
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
      openFile: vi.fn(() => Promise.resolve(undefined)),
      closeBuffer: vi.fn(() => Promise.resolve(undefined)),
      saveBuffer: vi.fn(() => Promise.resolve(undefined)),
      reloadBufferFromDisk: vi.fn(() => Promise.resolve(undefined)),
      keepBufferEdits: vi.fn(() => Promise.resolve(undefined)),
    },
  };
});

vi.mock('../../app/app-store', () => ({
  getAppSnapshot: harness.app.getSnapshot,
  subscribeApp: harness.app.subscribe,
  currentEnvelopeContext: harness.app.currentEnvelopeContext,
}));

vi.mock('../../bridge/bridge', () => ({
  openFile: harness.bridge.openFile,
  closeBuffer: harness.bridge.closeBuffer,
  saveBuffer: harness.bridge.saveBuffer,
  reloadBufferFromDisk: harness.bridge.reloadBufferFromDisk,
  keepBufferEdits: harness.bridge.keepBufferEdits,
}));

import { activeEditorContext, editorStore, recordEditorSelection } from './editor-store';

function buffer(
  id: string,
  path: string,
  version = 1,
  text = 'fn main() {}\n',
  selection: Selection | null = null,
  dirty = false,
): BufferSnapshot {
  return {
    buffer_id: id,
    path,
    language: 'rust',
    text,
    version,
    dirty,
    selection,
  };
}

function appWith(buffers: BufferSnapshot[], activeBufferId: string | null): {
  phase: 'ready';
  bootstrap: BootstrapState;
  error: null;
} {
  return {
    phase: 'ready',
    bootstrap: makeBootstrap({
      buffers,
      active_buffer_id: activeBufferId,
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
          children: [],
        },
      },
    }),
    error: null,
  };
}

describe('editor store', () => {
  beforeEach(() => {
    harness.bridge.openFile.mockClear();
    harness.bridge.closeBuffer.mockClear();
    harness.bridge.saveBuffer.mockClear();
    harness.bridge.reloadBufferFromDisk.mockClear();
    harness.bridge.keepBufferEdits.mockClear();
    harness.app.setSnapshot(appWith([], null));
    editorStore.reset();
  });

  it('hydrates tabs and the active buffer from bootstrap.buffers', () => {
    harness.app.setSnapshot(
      appWith(
        [
          buffer('b1', 'C:/workspace/src/main.rs', 3),
          buffer('b2', 'C:/workspace/src/lib.rs', 7),
        ],
        'b2',
      ),
    );
    editorStore.reset();

    const state = editorStore.getSnapshot();
    expect(state.tabs).toEqual([
      {
        id: 'b1',
        path: 'C:/workspace/src/main.rs',
        filename: 'main.rs',
        language: 'rust',
        isDirty: false,
        externallyChanged: false,
        version: 3,
      },
      {
        id: 'b2',
        path: 'C:/workspace/src/lib.rs',
        filename: 'lib.rs',
        language: 'rust',
        isDirty: false,
        externallyChanged: false,
        version: 7,
      },
    ]);
    expect(state.activeBufferId).toBe('b2');
    expect(state.activeBuffer?.buffer_id).toBe('b2');
    expect(state.workspaceRoot).toBe('C:/workspace');
  });

  it('marks a tab externally changed and dispatches the chosen resolution', () => {
    harness.app.setSnapshot({
      ...appWith([buffer('b1', 'C:/workspace/src/main.rs')], 'b1'),
      bufferConflicts: { b1: 'C:/workspace/src/main.rs' },
    });
    editorStore.reset();

    const state = editorStore.getSnapshot();
    expect(state.conflicts).toEqual({ b1: 'C:/workspace/src/main.rs' });
    expect(state.tabs[0].externallyChanged).toBe(true);

    editorStore.reloadFromDisk('b1');
    expect(harness.bridge.reloadBufferFromDisk).toHaveBeenCalledWith(
      'b1',
      { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
    );

    editorStore.keepMyEdits('b1');
    expect(harness.bridge.keepBufferEdits).toHaveBeenCalledWith(
      'b1',
      { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
    );
  });

  it('falls back to the first remaining buffer when the active one closes', () => {
    harness.app.setSnapshot(
      appWith(
        [buffer('b1', 'C:/workspace/src/main.rs'), buffer('b2', 'C:/workspace/src/lib.rs')],
        'b2',
      ),
    );
    editorStore.reset();
    expect(editorStore.getSnapshot().activeBufferId).toBe('b2');

    harness.app.setSnapshot(appWith([buffer('b1', 'C:/workspace/src/main.rs')], 'b2'));
    expect(editorStore.getSnapshot().activeBufferId).toBe('b1');
  });

  it('dispatches openFile for a path that is not already open', async () => {
    await editorStore.openPath('C:/workspace/src/engine.rs');
    expect(harness.bridge.openFile).toHaveBeenCalledWith(
      'C:/workspace/src/engine.rs',
      { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
    );
  });

  it('focuses an already-open path without re-opening it', async () => {
    harness.app.setSnapshot(
      appWith(
        [buffer('b1', 'C:/workspace/src/main.rs'), buffer('b2', 'C:/workspace/src/lib.rs')],
        'b1',
      ),
    );
    editorStore.reset();

    await editorStore.openPath('C:/workspace/src/lib.rs');
    expect(harness.bridge.openFile).not.toHaveBeenCalled();
    expect(editorStore.getSnapshot().activeBufferId).toBe('b2');
  });

  it('activates a newly opened buffer when its snapshot arrives', async () => {
    harness.app.setSnapshot(appWith([buffer('b1', 'C:/workspace/src/main.rs')], 'b1'));
    editorStore.reset();

    await editorStore.openPath('C:/workspace/src/engine.rs');
    harness.app.setSnapshot(
      appWith(
        [
          buffer('b1', 'C:/workspace/src/main.rs'),
          buffer('b2', 'C:/workspace/src/engine.rs'),
        ],
        'b1',
      ),
    );

    expect(editorStore.getSnapshot().activeBufferId).toBe('b2');
  });

  it('focuses a single buffer added outside the editor (e.g. the Files sidebar)', () => {
    harness.app.setSnapshot(appWith([buffer('b1', 'C:/workspace/src/main.rs')], 'b1'));
    editorStore.reset();

    harness.app.setSnapshot(
      appWith(
        [
          buffer('b1', 'C:/workspace/src/main.rs'),
          buffer('b2', 'C:/workspace/src/sidebar.rs'),
        ],
        'b1',
      ),
    );

    expect(editorStore.getSnapshot().activeBufferId).toBe('b2');
  });

  it('saves the active buffer and closes tabs through the bridge', () => {
    harness.app.setSnapshot(appWith([buffer('b1', 'C:/workspace/src/main.rs')], 'b1'));
    editorStore.reset();

    editorStore.saveActive();
    editorStore.closeBuffer('b1');

    expect(harness.bridge.saveBuffer).toHaveBeenCalledWith(
      'b1',
      { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
    );
    expect(harness.bridge.closeBuffer).toHaveBeenCalledWith(
      'b1',
      { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
    );
  });

  it('saves a specific buffer, including a non-active tab', async () => {
    harness.app.setSnapshot(
      appWith(
        [buffer('b1', 'C:/workspace/src/main.rs'), buffer('b2', 'C:/workspace/src/lib.rs')],
        'b1',
      ),
    );
    editorStore.reset();

    await expect(editorStore.saveBuffer('b2')).resolves.toBe(true);
    expect(harness.bridge.saveBuffer).toHaveBeenCalledWith(
      'b2',
      { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
    );
    expect(harness.bridge.saveBuffer).toHaveBeenCalledTimes(1);
  });

  it('resolves false when a buffer save rejects so the caller can keep the tab open', async () => {
    harness.app.setSnapshot(appWith([buffer('b1', 'C:/workspace/src/main.rs')], 'b1'));
    editorStore.reset();
    harness.bridge.saveBuffer.mockRejectedValueOnce(new Error('disk full'));

    await expect(editorStore.saveBuffer('b1')).resolves.toBe(false);
    expect(harness.bridge.closeBuffer).not.toHaveBeenCalled();
  });

  it('does not dispatch a save for a buffer that is no longer open', async () => {
    harness.app.setSnapshot(appWith([buffer('b1', 'C:/workspace/src/main.rs')], 'b1'));
    editorStore.reset();

    await expect(editorStore.saveBuffer('missing')).resolves.toBe(false);
    expect(harness.bridge.saveBuffer).not.toHaveBeenCalled();
  });

  it('stores and clears a line reveal request', async () => {
    harness.app.setSnapshot(appWith([buffer('b1', 'C:/workspace/src/main.rs')], 'b1'));
    editorStore.reset();

    await editorStore.openPathAtLine('C:/workspace/src/main.rs', 12);
    const reveal = editorStore.getSnapshot().revealTarget;
    expect(reveal?.path).toBe('C:/workspace/src/main.rs');
    expect(reveal?.line).toBe(12);

    editorStore.clearReveal(reveal?.requestId);
    expect(editorStore.getSnapshot().revealTarget).toBeNull();
  });

  it('activeEditorContext is null without an active buffer', () => {
    harness.app.setSnapshot(appWith([], null));
    editorStore.reset();

    expect(activeEditorContext()).toBeNull();
  });

  it('activeEditorContext carries path, language, dirty, and omits an empty selection', () => {
    const caret = { line: 0, utf16_column: 0 };
    harness.app.setSnapshot(
      appWith(
        [
          buffer(
            'b1',
            'C:/workspace/src/main.rs',
            1,
            'fn main() {}\n',
            { anchor: caret, active: caret, version: 1 },
            true,
          ),
        ],
        'b1',
      ),
    );
    editorStore.reset();

    const context = activeEditorContext();
    expect(context).toEqual({
      path: 'C:/workspace/src/main.rs',
      language: 'rust',
      dirty: true,
    });
    expect(context && 'selection' in context).toBe(false);
  });

  it('recordEditorSelection exposes a non-empty selection and clears it on empty', () => {
    harness.app.setSnapshot(appWith([buffer('b1', 'C:/workspace/src/main.rs')], 'b1'));
    editorStore.reset();

    const selection: Selection = {
      anchor: { line: 2, utf16_column: 0 },
      active: { line: 2, utf16_column: 5 },
      version: 0,
    };
    recordEditorSelection('b1', selection);
    expect(activeEditorContext()?.selection).toEqual(selection);

    recordEditorSelection('b1', {
      anchor: { line: 2, utf16_column: 5 },
      active: { line: 2, utf16_column: 5 },
      version: 0,
    });
    const context = activeEditorContext();
    expect(context && 'selection' in context).toBe(false);
  });
});
