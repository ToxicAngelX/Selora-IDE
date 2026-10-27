// Dirty-tab close guard: closing never discards edits without an explicit
// Save / Discard / Cancel choice, from the close button or Ctrl+W.
import { describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react';
import { EditorTabBar, type EditorTabView } from './EditorTabBar';

function tab(overrides: Partial<EditorTabView> = {}): EditorTabView {
  return {
    id: 'b1',
    filename: 'main.rs',
    path: 'C:/workspace/src/main.rs',
    language: 'rust',
    isDirty: false,
    ...overrides,
  };
}

function renderBar(overrides: {
  tabs?: EditorTabView[];
  activeTabId?: string | null;
  onSelectTab?: ReturnType<typeof vi.fn>;
  onCloseTab?: ReturnType<typeof vi.fn>;
  onSaveTab?: ReturnType<typeof vi.fn>;
} = {}) {
  const onSelectTab = overrides.onSelectTab ?? vi.fn();
  const onCloseTab = overrides.onCloseTab ?? vi.fn();
  const onSaveTab = overrides.onSaveTab ?? vi.fn(() => Promise.resolve(true));
  const view = render(
    <EditorTabBar
      tabs={overrides.tabs ?? [tab()]}
      activeTabId={overrides.activeTabId === undefined ? 'b1' : overrides.activeTabId}
      onSelectTab={onSelectTab}
      onCloseTab={onCloseTab}
      onSaveTab={onSaveTab}
    />,
  );
  return { ...view, onSelectTab, onCloseTab, onSaveTab };
}

describe('EditorTabBar', () => {
  it('closes a clean tab immediately without asking', () => {
    const { getByRole, queryByRole, onCloseTab, onSaveTab } = renderBar();

    fireEvent.click(getByRole('button', { name: 'Close main.rs' }));

    expect(onCloseTab).toHaveBeenCalledWith('b1');
    expect(onSaveTab).not.toHaveBeenCalled();
    expect(queryByRole('dialog')).toBeNull();
  });

  it('shows Save, Discard, and Cancel for a dirty tab and never closes before a choice', () => {
    const { getByRole, onCloseTab, onSaveTab } = renderBar({
      tabs: [tab({ isDirty: true })],
    });

    fireEvent.click(getByRole('button', { name: 'Close main.rs' }));

    const dialog = getByRole('dialog', { name: 'Unsaved changes in main.rs' });
    expect(within(dialog).getByRole('button', { name: 'Save' })).toBeTruthy();
    expect(within(dialog).getByRole('button', { name: 'Discard' })).toBeTruthy();
    expect(within(dialog).getByRole('button', { name: 'Cancel' })).toBeTruthy();
    expect(onCloseTab).not.toHaveBeenCalled();
    expect(onSaveTab).not.toHaveBeenCalled();
  });

  it('saves first and closes only after the save resolves successfully', async () => {
    let resolveSave: (value: boolean) => void = () => undefined;
    const onSaveTab = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          resolveSave = resolve;
        }),
    );
    const { getByRole, onCloseTab } = renderBar({
      tabs: [tab({ isDirty: true })],
      onSaveTab,
    });

    fireEvent.click(getByRole('button', { name: 'Close main.rs' }));
    const dialog = getByRole('dialog', { name: 'Unsaved changes in main.rs' });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Save' }));

    expect(onSaveTab).toHaveBeenCalledWith('b1');
    expect(onCloseTab).not.toHaveBeenCalled();

    await act(async () => {
      resolveSave(true);
    });

    await waitFor(() => {
      expect(onCloseTab).toHaveBeenCalledWith('b1');
    });
  });

  it('keeps the tab open when the save fails', async () => {
    const onSaveTab = vi.fn(() => Promise.resolve(false));
    const { getByRole, queryByRole, onCloseTab } = renderBar({
      tabs: [tab({ isDirty: true })],
      onSaveTab,
    });

    fireEvent.click(getByRole('button', { name: 'Close main.rs' }));
    fireEvent.click(getByRole('button', { name: 'Save' }));

    await waitFor(() => {
      expect(getByRole('dialog', { name: 'Unsaved changes in main.rs' })).toBeTruthy();
    });
    expect(onCloseTab).not.toHaveBeenCalled();
    expect(queryByRole('button', { name: 'Save' })).not.toBeNull();
  });

  it('discards and closes the tab when Discard is chosen', () => {
    const { getByRole, onCloseTab, onSaveTab } = renderBar({
      tabs: [tab({ isDirty: true })],
    });

    fireEvent.click(getByRole('button', { name: 'Close main.rs' }));
    fireEvent.click(getByRole('button', { name: 'Discard' }));

    expect(onCloseTab).toHaveBeenCalledWith('b1');
    expect(onSaveTab).not.toHaveBeenCalled();
  });

  it('leaves and focuses the tab when Cancel is chosen', () => {
    const { getByRole, queryByRole, onSelectTab, onCloseTab } = renderBar({
      tabs: [tab({ isDirty: true })],
      activeTabId: 'b1',
    });
    const tabButton = getByRole('tab', { name: 'main.rs' });

    fireEvent.click(getByRole('button', { name: 'Close main.rs' }));
    fireEvent.click(getByRole('button', { name: 'Cancel' }));

    expect(onCloseTab).not.toHaveBeenCalled();
    expect(onSelectTab).toHaveBeenCalledWith('b1');
    expect(queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(tabButton);
  });

  it('opens the confirmation from Ctrl+W on a dirty tab', () => {
    const { getByRole, onCloseTab } = renderBar({
      tabs: [tab({ isDirty: true })],
    });

    fireEvent.keyDown(getByRole('tab', { name: 'main.rs' }), {
      key: 'w',
      ctrlKey: true,
    });

    expect(getByRole('dialog', { name: 'Unsaved changes in main.rs' })).toBeTruthy();
    expect(onCloseTab).not.toHaveBeenCalled();
  });

  it('closes a clean tab immediately from Ctrl+W', () => {
    const { getByRole, queryByRole, onCloseTab } = renderBar();

    fireEvent.keyDown(getByRole('tab', { name: 'main.rs' }), {
      key: 'w',
      ctrlKey: true,
    });

    expect(onCloseTab).toHaveBeenCalledWith('b1');
    expect(queryByRole('dialog')).toBeNull();
  });

  it('treats Escape as Cancel', () => {
    const { getByRole, queryByRole, onSelectTab, onCloseTab } = renderBar({
      tabs: [tab({ isDirty: true })],
      activeTabId: 'b1',
    });

    fireEvent.click(getByRole('button', { name: 'Close main.rs' }));
    fireEvent.keyDown(getByRole('dialog'), { key: 'Escape' });

    expect(onCloseTab).not.toHaveBeenCalled();
    expect(onSelectTab).toHaveBeenCalledWith('b1');
    expect(queryByRole('dialog')).toBeNull();
  });
});