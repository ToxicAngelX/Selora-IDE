// Selora IDE — editor tab context menu.
//
// Right-clicking a tab opens this fixed-position menu anchored at the pointer.
// Actions are target-scoped: every item applies to the tab it was opened on,
// not the active tab. Items mirror the VS Code tab menu for the actions Selora
// can perform; shortcut hints are display-only except Ctrl+F4 (global keymap).
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import type { EditorTabView } from './EditorTabBar';

export type TabMenuAction =
  | 'close'
  | 'close-others'
  | 'close-right'
  | 'close-saved'
  | 'close-all'
  | 'copy-path'
  | 'copy-relative-path'
  | 'copy-breadcrumbs-path'
  | 'reveal-in-explorer'
  | 'reveal-in-view'
  | 'toggle-pin'
  | 'find-references';

type MenuItem = {
  action: TabMenuAction;
  label: string;
  hint?: string;
};

export type TabContextMenuProps = {
  tab: EditorTabView;
  /** Viewport coordinates of the right-click. */
  x: number;
  y: number;
  isPinned: boolean;
  onAction: (action: TabMenuAction) => void;
  onClose: () => void;
};

const MENU_MARGIN = 8;

function menuItems(isPinned: boolean): Array<MenuItem | 'divider'> {
  return [
    { action: 'close', label: 'Close', hint: 'Ctrl+F4' },
    { action: 'close-others', label: 'Close Others' },
    { action: 'close-right', label: 'Close to the Right' },
    { action: 'close-saved', label: 'Close Saved', hint: 'Ctrl+K U' },
    { action: 'close-all', label: 'Close All', hint: 'Ctrl+K W' },
    'divider',
    { action: 'copy-path', label: 'Copy Path', hint: 'Shift+Alt+C' },
    { action: 'copy-relative-path', label: 'Copy Relative Path', hint: 'Ctrl+K Ctrl+Shift+C' },
    { action: 'copy-breadcrumbs-path', label: 'Copy Breadcrumbs Path' },
    'divider',
    { action: 'reveal-in-explorer', label: 'Reveal in File Explorer', hint: 'Shift+Alt+R' },
    { action: 'reveal-in-view', label: 'Reveal in Explorer View' },
    'divider',
    { action: 'toggle-pin', label: isPinned ? 'Unpin' : 'Keep Open', hint: 'Ctrl+K Enter' },
    'divider',
    { action: 'find-references', label: 'Find File References', hint: 'Shift+Alt+F12' },
  ];
}

export function TabContextMenu({ tab, x, y, isPinned, onAction, onClose }: TabContextMenuProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ top: y, left: x });

  // Measure after paint so the menu never hangs off the viewport edge.
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const rect = root.getBoundingClientRect();
    const viewportWidth = window.innerWidth || 1200;
    const viewportHeight = window.innerHeight || 800;
    setPosition((current) => {
      const left = Math.max(
        MENU_MARGIN,
        Math.min(current.left, viewportWidth - rect.width - MENU_MARGIN),
      );
      const top = Math.max(
        MENU_MARGIN,
        Math.min(current.top, viewportHeight - rect.height - MENU_MARGIN),
      );
      return left === current.left && top === current.top ? current : { left, top };
    });
    root.querySelector<HTMLButtonElement>('[role="menuitem"]:not([disabled])')?.focus();
  }, []);

  useEffect(() => {
    const handlePointer = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (!target || rootRef.current?.contains(target)) return;
      onClose();
    };
    // Keep the menu open while its own items are clicked.
    window.addEventListener('mousedown', handlePointer);
    return () => window.removeEventListener('mousedown', handlePointer);
  }, [onClose]);

  const moveFocus = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp' && event.key !== 'Home' && event.key !== 'End') {
      return;
    }
    event.preventDefault();
    const items = Array.from(
      rootRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not([disabled])') ?? [],
    );
    if (items.length === 0) return;
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    let next = 0;
    if (event.key === 'End') next = items.length - 1;
    else if (event.key !== 'Home') {
      const delta = event.key === 'ArrowDown' ? 1 : -1;
      next = index < 0 ? 0 : (index + delta + items.length) % items.length;
    }
    items[next]?.focus();
  };

  return (
    <div
      ref={rootRef}
      className="tree-context-menu tab-context-menu"
      role="menu"
      aria-label={`Actions for ${tab.filename}`}
      style={{ top: position.top, left: position.left }}
      onKeyDown={moveFocus}
      onContextMenu={(event) => event.preventDefault()}
    >
      {menuItems(isPinned).map((item, index) =>
        item === 'divider' ? (
          <div key={`divider-${index}`} className="context-menu-divider" role="separator" />
        ) : (
          <button
            key={item.action}
            type="button"
            role="menuitem"
            className="context-menu-item"
            onClick={() => {
              onAction(item.action);
              onClose();
            }}
          >
            <span>{item.label}</span>
            {item.hint ? <span className="context-kbd">{item.hint}</span> : null}
          </button>
        ),
      )}
    </div>
  );
}