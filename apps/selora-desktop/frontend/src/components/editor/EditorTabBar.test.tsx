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
  onTabMenuAction?: ReturnType<typeof vi.fn>;
} = {}) {
  const onSelectTab = overrides.onSelectTab ?? vi.fn();
  const onCloseTab = overrides.onCloseTab ?? vi.fn();
  const onSaveTab = overrides.onSaveTab ?? vi.fn(() => Promise.resolve(true));
  const onTabMenuAction = overrides.onTabMenuAction ?? vi.fn();
  const view = render(
    <EditorTabBar
      tabs={overrides.tabs ?? [tab()]}
      activeTabId={overrides.activeTabId === undefined ? 'b1' : overrides.activeTabId}
      onSelectTab={onSelectTab}
      onCloseTab={onCloseTab}
      onSaveTab={onSaveTab}
      onTabMenuAction={onTabMenuAction}
    />,
  );
  return { ...view, onSelectTab, onCloseTab, onSaveTab, onTabMenuAction };
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

  it('opens the tab menu on right-click with the supported actions', () => {
    const { getByRole } = renderBar();

    fireEvent.contextMenu(getByRole('tab', { name: 'main.rs' }), { clientX: 40, clientY: 60 });

    const menu = getByRole('menu', { name: 'Actions for main.rs' });
    const rendered = within(menu)
      .getAllByRole('menuitem')
      .map((item) => item.textContent ?? '');
    for (const label of [
      'Close',
      'Close Others',
      'Close to the Right',
      'Close Saved',
      'Close All',
      'Copy Path',
      'Copy Relative Path',
      'Copy Breadcrumbs Path',
      'Reveal in File Explorer',
      'Reveal in Explorer View',
      'Keep Open',
      'Find File References',
    ]) {
      expect(rendered.some((text) => text.startsWith(label))).toBe(true);
    }
  });

  it('sends menu actions for the right-clicked tab, not the active one', () => {
    const { getByRole, onTabMenuAction, queryByRole } = renderBar({
      tabs: [tab(), tab({ id: 'b2', filename: 'lib.rs' })],
      activeTabId: 'b1',
    });

    fireEvent.contextMenu(getByRole('tab', { name: 'lib.rs' }), { clientX: 40, clientY: 60 });
    fireEvent.click(within(getByRole('menu')).getByText('Copy Path'));

    expect(onTabMenuAction).toHaveBeenCalledWith('copy-path', expect.objectContaining({ id: 'b2' }));
    expect(queryByRole('menu')).toBeNull();
  });

  it('routes the menu Close item through the dirty-tab guard', () => {
    const { getByRole, onCloseTab, onTabMenuAction } = renderBar({
      tabs: [tab({ isDirty: true })],
    });

    fireEvent.contextMenu(getByRole('tab', { name: 'main.rs' }), { clientX: 40, clientY: 60 });
    fireEvent.click(within(getByRole('menu')).getByText('Close'));

    expect(getByRole('dialog', { name: 'Unsaved changes in main.rs' })).toBeTruthy();
    expect(onCloseTab).not.toHaveBeenCalled();
    expect(onTabMenuAction).not.toHaveBeenCalled();
  });

  it('shows Unpin for a kept-open tab', () => {
    const { getByRole } = renderBar({ tabs: [tab({ isPinned: true })] });

    fireEvent.contextMenu(getByRole('tab', { name: 'main.rs' }), { clientX: 40, clientY: 60 });

    expect(within(getByRole('menu')).getByText('Unpin')).toBeTruthy();
  });

  it('closes the tab menu on Escape', () => {
    const { getByRole, queryByRole } = renderBar();

    fireEvent.contextMenu(getByRole('tab', { name: 'main.rs' }), { clientX: 40, clientY: 60 });
    fireEvent.keyDown(getByRole('menu'), { key: 'Escape' });

    expect(queryByRole('menu')).toBeNull();
  });
});