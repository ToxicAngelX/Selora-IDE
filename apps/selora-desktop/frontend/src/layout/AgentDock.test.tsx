// AgentDock workbench parity: honest empty session state, the header Stop
// switch, and the explicit way back to the editor/split view (UI_SPEC §17).
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render } from '@testing-library/react';
import { AgentDock } from './AgentDock';
import { agentStore } from '../features/agent/agent-store';
import { makeBootstrap } from '../testing/fixtures';

const editorHarness = vi.hoisted(() => {
  let snapshot: unknown = { activeBuffer: null, activeSelection: null };
  return {
    get: () => snapshot as { activeBuffer: unknown; activeSelection: unknown },
    set(next: { activeBuffer: unknown; activeSelection: unknown }) { snapshot = next; },
  };
});
vi.mock('../features/editor/editor-store', () => ({
  editorStore: {
    subscribe: () => () => undefined,
    getSnapshot: () => editorHarness.get(),
  },
  activeEditorContext: () => {
    const s = editorHarness.get() as { activeBuffer: { path: string; language: string } | null; activeSelection: unknown };
    return s.activeBuffer
      ? { path: s.activeBuffer.path, language: s.activeBuffer.language, ...(s.activeSelection ? { selection: s.activeSelection } : {}), dirty: false }
      : null;
  },
}));

beforeEach(() => {
  agentStore.reset();
  agentStore.setPermissionMode('full');
  editorHarness.set({ activeBuffer: null, activeSelection: null });
});

describe('AgentDock session truth', () => {
  it('shows an empty session strip rather than inventing a session', () => {
    const { getByText, queryAllByRole } = render(<AgentDock />);
    expect(getByText('No sessions yet')).toBeTruthy();
    expect(queryAllByRole('tab')).toHaveLength(0);
    expect(agentStore.getSnapshot().sessions).toEqual([]);
  });

  it('refreshes the tab strip after the backend deletes the active session', () => {
    act(() => {
      agentStore.hydrate(
        makeBootstrap({
          workspace: {
            id: 'ws-1',
            core_instance_id: 'core-0001',
            root: '/repo',
            display_name: 'repo',
          },
          chat: {
            sessions: [
              { session_id: 'sess-a', title: 'Agent Session 1', message_ids: [], active: false },
              { session_id: 'sess-b', title: 'Agent Session 2', message_ids: [], active: false },
            ],
            messages: [],
            active_session_id: 'sess-a',
            active_request_id: null,
            active_turn_id: null,
            pending_approval: null,
          },
        }),
      );
    });

    const { getByRole, queryByRole } = render(<AgentDock />);
    expect(getByRole('tab', { name: /Agent Session 1/ })).toBeTruthy();

    act(() => {
      agentStore.deleteSession('sess-a');
    });

    expect(queryByRole('tab', { name: /Agent Session 1/ })).toBeNull();
    const replacement = getByRole('tab', { name: /Agent Session 2/ });
    expect(replacement.getAttribute('aria-selected')).toBe('true');
  });

  it('offers editor and split exits that name the shell mode', () => {
    const onExit = vi.fn();
    const { getByRole } = render(<AgentDock onExit={onExit} />);

    fireEvent.click(getByRole('button', { name: 'Editor view' }));
    fireEvent.click(getByRole('button', { name: 'Split view' }));
    expect(onExit.mock.calls.map(([mode]) => mode)).toEqual(['editor', 'split']);
  });
});

describe('AgentDock auto-apply indicator', () => {
  it('shows "Auto-apply armed" in full mode and hides it otherwise', () => {
    const { getAllByText, queryByText } = render(<AgentDock />);

    expect(getAllByText('Auto-apply armed').length).toBeGreaterThan(0);

    act(() => {
      agentStore.setPermissionMode('plan_only');
    });
    expect(queryByText('Auto-apply armed')).toBeNull();

    act(() => {
      agentStore.setPermissionMode('ask_before_change');
    });
    expect(queryByText('Auto-apply armed')).toBeNull();

    act(() => {
      agentStore.setPermissionMode('full');
    });
    expect(getAllByText('Auto-apply armed').length).toBeGreaterThan(0);
  });

  it('does not claim an armed session without backend models or workspace', () => {
    const { getByRole, getByText } = render(<AgentDock />);

    const suggestion = getByRole('button', { name: 'Summarize the workspace' }) as HTMLButtonElement;
    expect(suggestion.disabled).toBe(true);
    expect(getByText(/needs an open folder to read and write files/i)).toBeTruthy();
  });
});

describe('AgentDock suggestion chips', () => {
  it('hides the buffer- and selection-dependent chips when no buffer is active', () => {
    const { queryByRole, getByRole } = render(<AgentDock />);

    expect(queryByRole('button', { name: 'Explain this file' })).toBeNull();
    expect(queryByRole('button', { name: 'Find a bug in the selection' })).toBeNull();
    expect(getByRole('button', { name: 'Summarize the workspace' })).toBeTruthy();
  });

  it('offers "Explain this file" for an active buffer without a selection', () => {
    act(() =>
      editorHarness.set({
        activeBuffer: { path: 'src/main.rs', language: 'rust' },
        activeSelection: null,
      }),
    );
    const { queryByRole, getByRole } = render(<AgentDock />);

    expect(getByRole('button', { name: 'Explain this file' })).toBeTruthy();
    expect(queryByRole('button', { name: 'Find a bug in the selection' })).toBeNull();
  });

  it('offers the selection chip when the active buffer has a selection', () => {
    act(() =>
      editorHarness.set({
        activeBuffer: { path: 'src/main.rs', language: 'rust' },
        activeSelection: {
          anchor: { line: 2, utf16_column: 0 },
          active: { line: 2, utf16_column: 5 },
          version: 1,
        },
      }),
    );
    const { getByRole } = render(<AgentDock />);

    expect(getByRole('button', { name: 'Explain this file' })).toBeTruthy();
    expect(getByRole('button', { name: 'Find a bug in the selection' })).toBeTruthy();
  });
});