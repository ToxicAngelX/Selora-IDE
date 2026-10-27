import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, waitFor } from '@testing-library/react';
import { EditorView } from '@codemirror/view';
import { undo } from '@codemirror/commands';
import type { BufferSnapshot } from '../../bridge/bridge';

vi.mock('../../bridge/bridge', () => ({
  applyEditorChange: vi.fn(() => Promise.resolve(undefined)),
  setSelection: vi.fn(() => Promise.resolve(undefined)),
}));

const appHarness = vi.hoisted(() => {
  let snapshot: unknown = { phase: 'connecting', bootstrap: null, error: null };
  const listeners = new Set<() => void>();
  return {
    getSnapshot: () => snapshot,
    setSnapshot(next: unknown) {
      snapshot = next;
      for (const listener of listeners) listener();
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
  getAppSnapshot: appHarness.getSnapshot,
  subscribeApp: appHarness.subscribe,
  currentEnvelopeContext: () => ({
    core_instance_id: 'core-0001',
    workspace_id: 'ws-1',
    session_id: null,
  }),
}));

import { CodeMirrorPane } from './CodeMirrorPane';
import { editorStore } from './editor-store';
import { applyEditorChange, setSelection } from '../../bridge/bridge';

const buffer: BufferSnapshot = {
  buffer_id: 'b1',
  path: 'C:/workspace/src/main.rs',
  language: 'rust',
  text: 'fn main() {\n    println!("hi");\n}\n',
  version: 1,
  dirty: false,
  selection: null,
};

describe('CodeMirrorPane mount', () => {
  beforeEach(() => {
    appHarness.setSnapshot({ phase: 'connecting', bootstrap: null, error: null });
  });

  it('mounts a real EditorView with the buffer document', async () => {
    const { container } = render(<CodeMirrorPane buffer={buffer} />);
    await waitFor(() => {
      expect(container.querySelector('.cm-content')).not.toBeNull();
    });
    expect(container.querySelector('.cm-content')?.textContent).toContain('fn main');
  });

  it('reveals the requested 1-based line in the active buffer', async () => {
    const { container } = render(
      <CodeMirrorPane
        buffer={buffer}
        revealTarget={{ path: buffer.path, line: 2, requestId: 1 }}
      />,
    );
    await waitFor(() => {
      expect(container.querySelector('.cm-editor')).not.toBeNull();
    });
    const view = EditorView.findFromDOM(container.querySelector<HTMLElement>('.cm-editor')!);
    await waitFor(() => {
      expect(view!.state.selection.main.head).toBe(view!.state.doc.line(2).from);
    });
  });

  it('sends local transactions as versioned protocol changes', async () => {
    const { container } = render(<CodeMirrorPane buffer={buffer} />);
    await waitFor(() => {
      expect(container.querySelector('.cm-editor')).not.toBeNull();
    });
    const view = EditorView.findFromDOM(container.querySelector<HTMLElement>('.cm-editor')!);
    expect(view).not.toBeNull();

    view!.dispatch({ changes: { from: 0, to: 0, insert: '// ' } });

    await waitFor(() => {
      expect(vi.mocked(applyEditorChange)).toHaveBeenCalledWith(
        'b1',
        1,
        [
          {
            start: { line: 0, utf16_column: 0 },
            end: { line: 0, utf16_column: 0 },
            replacement: '// ',
          },
        ],
        { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
      );
    });
  });

  it('preserves unacknowledged edits across buffer switches and keeps versions monotonic', async () => {
    vi.mocked(applyEditorChange).mockClear();
    const { container, rerender } = render(<CodeMirrorPane buffer={buffer} />);
    await waitFor(() => {
      expect(container.querySelector('.cm-editor')).not.toBeNull();
    });
    const view = EditorView.findFromDOM(container.querySelector<HTMLElement>('.cm-editor')!);
    view!.dispatch({ changes: { from: 0, to: 0, insert: '// ' } });

    await waitFor(() => {
      expect(vi.mocked(applyEditorChange)).toHaveBeenCalledWith(
        'b1',
        1,
        expect.any(Array),
        expect.any(Object),
      );
    });
    expect(view!.state.doc.toString()).toContain('// fn main');

    rerender(
      <CodeMirrorPane
        buffer={{
          ...buffer,
          buffer_id: 'b2',
          path: 'C:/workspace/src/lib.rs',
          text: 'fn lib() {}\n',
        }}
      />,
    );
    await waitFor(() => {
      expect(view!.state.doc.toString()).toBe('fn lib() {}\n');
    });

    // The echo for b1 never arrived, so its snapshot is still version 1 with
    // the original text. Switching back must restore the cached local doc.
    rerender(<CodeMirrorPane buffer={buffer} />);
    await waitFor(() => {
      expect(view!.state.doc.toString()).toContain('// fn main');
    });

    vi.mocked(applyEditorChange).mockClear();
    view!.dispatch({ changes: { from: 0, to: 0, insert: '/* x */ ' } });
    await waitFor(() => {
      expect(vi.mocked(applyEditorChange)).toHaveBeenCalledWith(
        'b1',
        2,
        expect.any(Array),
        expect.any(Object),
      );
    });
    expect(view!.state.doc.toString()).toContain('/* x */ // fn main');
  });

  it('ignores equal-version stale echoes but applies newer remote snapshots', async () => {
    const { container, rerender } = render(<CodeMirrorPane buffer={buffer} />);
    await waitFor(() => {
      expect(container.querySelector('.cm-editor')).not.toBeNull();
    });
    const view = EditorView.findFromDOM(container.querySelector<HTMLElement>('.cm-editor')!);
    view!.dispatch({ changes: { from: 0, to: 0, insert: '// ' } });

    rerender(<CodeMirrorPane buffer={{ ...buffer, version: 2, dirty: true }} />);
    expect(view!.state.doc.toString()).toContain('// fn main');

    rerender(<CodeMirrorPane buffer={{ ...buffer, version: 3, text: 'remote update\n' }} />);
    await waitFor(() => {
      expect(view!.state.doc.toString()).toBe('remote update\n');
    });
  });

  it('binds Ctrl+S to save and Ctrl+F to the search panel', async () => {
    const saveSpy = vi.spyOn(editorStore, 'saveActive').mockImplementation(() => undefined);
    const { container } = render(<CodeMirrorPane buffer={buffer} />);
    await waitFor(() => {
      expect(container.querySelector('.cm-editor')).not.toBeNull();
    });
    const view = EditorView.findFromDOM(container.querySelector<HTMLElement>('.cm-editor')!);
    const saveEvent = new KeyboardEvent('keydown', {
      key: 's',
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    view!.contentDOM.dispatchEvent(saveEvent);
    expect(saveEvent.defaultPrevented).toBe(true);
    expect(saveSpy).toHaveBeenCalledTimes(1);

    view!.contentDOM.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true, cancelable: true }),
    );
    await waitFor(() => {
      expect(container.querySelector('.cm-search')).not.toBeNull();
    });
    saveSpy.mockRestore();
  });

  it('reports the debounced selection with the current version', async () => {
    vi.mocked(setSelection).mockClear();
    const { container } = render(<CodeMirrorPane buffer={buffer} />);
    await waitFor(() => {
      expect(container.querySelector('.cm-editor')).not.toBeNull();
    });
    const view = EditorView.findFromDOM(container.querySelector<HTMLElement>('.cm-editor')!);
    view!.dispatch({ selection: { anchor: view!.state.doc.line(2).from } });

    await waitFor(
      () => {
        expect(vi.mocked(setSelection)).toHaveBeenCalledWith(
          'b1',
          {
            anchor: { line: 1, utf16_column: 0 },
            active: { line: 1, utf16_column: 0 },
            version: 1,
          },
          { core_instance_id: 'core-0001', workspace_id: 'ws-1', session_id: null },
        );
      },
      { timeout: 1000 },
    );
  });

  it('swaps files without echoing edits or leaking undo history', async () => {
    const { container, rerender } = render(<CodeMirrorPane buffer={buffer} />);
    await waitFor(() => {
      expect(container.querySelector('.cm-editor')).not.toBeNull();
    });
    const view = EditorView.findFromDOM(container.querySelector<HTMLElement>('.cm-editor')!);
    view!.dispatch({ changes: { from: 0, to: 0, insert: '// ' } });
    await waitFor(() => {
      expect(vi.mocked(applyEditorChange)).toHaveBeenCalled();
    });
    vi.mocked(applyEditorChange).mockClear();

    rerender(
      <CodeMirrorPane
        buffer={{
          ...buffer,
          buffer_id: 'b2',
          path: 'C:/workspace/src/lib.rs',
          text: 'fn lib() {}\n',
        }}
      />,
    );

    await waitFor(() => {
      expect(view!.state.doc.toString()).toBe('fn lib() {}\n');
    });
    expect(vi.mocked(applyEditorChange)).not.toHaveBeenCalled();
    expect(undo(view!)).toBe(false);
    expect(view!.state.doc.toString()).toBe('fn lib() {}\n');
  });

  it('opens a matching definition answer once and ignores notification loops', async () => {
    const openPathAtLine = vi
      .spyOn(editorStore, 'openPathAtLine')
      .mockImplementation(() => Promise.resolve());
    const { container } = render(<CodeMirrorPane buffer={buffer} />);
    await waitFor(() => {
      expect(container.querySelector('.cm-editor')).not.toBeNull();
    });

    const definitionResult = {
      buffer_id: 'b1',
      locations: [{ path: 'C:/workspace/src/lib.rs', line: 7, utf16_column: 4 }],
    };
    act(() => {
      appHarness.setSnapshot({
        phase: 'ready',
        bootstrap: null,
        error: null,
        definitionResult,
      });
    });

    await waitFor(() => {
      expect(openPathAtLine).toHaveBeenCalledWith('C:/workspace/src/lib.rs', 8);
    });
    act(() => {
      appHarness.setSnapshot({
        phase: 'ready',
        bootstrap: null,
        error: null,
        definitionResult,
      });
    });
    expect(openPathAtLine).toHaveBeenCalledTimes(1);
    openPathAtLine.mockRestore();
  });
});
