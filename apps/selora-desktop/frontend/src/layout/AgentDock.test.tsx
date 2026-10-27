// AgentDock interaction parity: the session drawer is dismissible from the
// keyboard and returns focus to its toggle (UI_SPEC §17).
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render } from '@testing-library/react';
import { AgentDock } from './AgentDock';
import { agentStore } from '../features/agent/agent-store';

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

describe('AgentDock keyboard drawer', () => {
  it('closes the session drawer on Escape and restores focus to its toggle', () => {
    const { getByRole, queryByLabelText } = render(<AgentDock />);
    const toggle = getByRole('button', { name: /Agent Session 1/ });
    fireEvent.click(toggle);
    expect(queryByLabelText('Agent Session History')).toBeTruthy();
    const dialog = getByRole('dialog', { name: 'Agent sessions' });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.contains(document.activeElement)).toBe(true);

    const focusables = Array.from(
      dialog.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    );
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    first.focus();
    fireEvent.keyDown(first, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(last);
    last.focus();
    fireEvent.keyDown(last, { key: 'Tab' });
    expect(document.activeElement).toBe(first);

    fireEvent.keyDown(window, { key: 'Escape' });

    expect(queryByLabelText('Agent Session History')).toBeNull();
    expect(document.activeElement).toBe(toggle);
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