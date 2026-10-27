// Breadcrumb menus: WAI-ARIA menu keyboard support (roving focus, activation,
// Escape focus restore) layered over the existing pointer interactions.
import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import { EditorBreadcrumbs } from './EditorBreadcrumbs';
import type { BreadcrumbSegment } from './editor-types';

const segments: BreadcrumbSegment[] = [
  { id: 'b1', label: 'src', kind: 'dir', siblings: ['src', 'tests', 'benches'] },
  { id: 'b2', label: 'main.rs', kind: 'file' },
];

describe('EditorBreadcrumbs', () => {
  it('opens the menu on the current sibling and navigates with ArrowUp/ArrowDown', () => {
    const { getByRole, getAllByRole } = render(<EditorBreadcrumbs segments={segments} />);

    fireEvent.click(getByRole('button', { name: 'src' }));
    const menu = getByRole('menu', { name: 'src siblings' });
    const items = getAllByRole('menuitem');

    expect(items).toHaveLength(3);
    expect(document.activeElement).toBe(items[0]);
    expect(items[0].tabIndex).toBe(0);
    expect(items[1].tabIndex).toBe(-1);

    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[1]);
    expect(items[1].tabIndex).toBe(0);
    expect(items[0].tabIndex).toBe(-1);

    // Wraps forward to the first item, then backward to the last.
    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[0]);

    fireEvent.keyDown(menu, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(items[2]);
  });

  it('jumps to the first and last items with Home and End', () => {
    const { getByRole, getAllByRole } = render(<EditorBreadcrumbs segments={segments} />);

    fireEvent.click(getByRole('button', { name: 'src' }));
    const menu = getByRole('menu', { name: 'src siblings' });
    const items = getAllByRole('menuitem');

    fireEvent.keyDown(menu, { key: 'End' });
    expect(document.activeElement).toBe(items[2]);

    fireEvent.keyDown(menu, { key: 'Home' });
    expect(document.activeElement).toBe(items[0]);
  });

  it('activates a sibling with Enter and Space and returns focus to the trigger', () => {
    const onSelectSibling = vi.fn();
    const { getByRole, queryByRole } = render(
      <EditorBreadcrumbs segments={segments} onSelectSibling={onSelectSibling} />,
    );
    const trigger = getByRole('button', { name: 'src' });

    fireEvent.click(trigger);
    const menu = getByRole('menu', { name: 'src siblings' });
    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    fireEvent.keyDown(menu, { key: 'Enter' });

    expect(onSelectSibling).toHaveBeenCalledWith('b1', 'tests');
    expect(queryByRole('menu')).toBeNull();
    expect(document.activeElement).toBe(trigger);

    fireEvent.click(trigger);
    const secondMenu = getByRole('menu', { name: 'src siblings' });
    fireEvent.keyDown(secondMenu, { key: 'End' });
    fireEvent.keyDown(secondMenu, { key: ' ' });

    expect(onSelectSibling).toHaveBeenLastCalledWith('b1', 'benches');
  });

  it('closes with Escape and returns focus to the trigger', () => {
    const { getByRole, getAllByRole, queryByRole } = render(<EditorBreadcrumbs segments={segments} />);
    const trigger = getByRole('button', { name: 'src' });

    fireEvent.click(trigger);
    const menu = getByRole('menu', { name: 'src siblings' });
    const items = getAllByRole('menuitem');

    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[1]);

    fireEvent.keyDown(menu, { key: 'Escape' });
    expect(queryByRole('menu')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it('does not open a menu for segments without siblings', () => {
    const { getByRole, queryByRole } = render(<EditorBreadcrumbs segments={segments} />);

    fireEvent.click(getByRole('button', { name: 'main.rs' }));

    expect(queryByRole('menu')).toBeNull();
  });
});
