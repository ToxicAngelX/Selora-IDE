// Selora IDE — Phase Z5c keyboard shortcut cheat sheet.
//
// A modal dialog listing every registry command grouped by category. It
// reuses the palette's dialog contract: --scrim + --blur-surface backdrop,
// role="dialog" + aria-modal, a search input focused on open, Esc/backdrop
// close, and focus returned to the invoking control (PHASE_Z2_DESIGN §7).
// Rows read the resolved CommandSpecs, so the rebindable pair shows the
// live settings strings. Filtering matches titles (fuzzy, via the registry)
// and literal key-chord text.
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  COMMAND_CATEGORY_ORDER,
  commandCategory,
  filterCommands,
  type CommandSpec,
} from '../features/keymap/commands';
import { Icon } from './Icon';
import '../styles/cheatsheet.css';

/** Window event any surface may dispatch to open the sheet (Z5 contract). */
export const OPEN_CHEATSHEET_EVENT = 'selora:open-cheatsheet';

export type CheatSheetProps = {
  open: boolean;
  commands: CommandSpec[];
  onClose: () => void;
};

function filterRows(commands: CommandSpec[], query: string): CommandSpec[] {
  const q = query.trim().toLowerCase();
  if (!q) return commands;
  const titleMatches = new Set(filterCommands(commands, q).map((command) => command.id));
  return commands.filter(
    (command) => titleMatches.has(command.id) || (command.shortcut?.toLowerCase().includes(q) ?? false),
  );
}

/** Subscribe to the sheet's window event without owning the open state. */
export function useCheatSheetTrigger(onOpen: () => void): void {
  useEffect(() => {
    const handler = () => onOpen();
    window.addEventListener(OPEN_CHEATSHEET_EVENT, handler);
    return () => window.removeEventListener(OPEN_CHEATSHEET_EVENT, handler);
  }, [onOpen]);
}

function categoryDomId(category: string): string {
  return `cheatsheet-category-${category.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`;
}

export function CheatSheet({ open, commands, onClose }: CheatSheetProps) {
  const [query, setQuery] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setQuery('');
    inputRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (open) return;
    const restore = restoreFocusRef.current;
    restoreFocusRef.current = null;
    if (!restore || !restore.isConnected) return;
    const current = document.activeElement;
    const sheetHadFocus = current === document.body || (dialogRef.current?.contains(current) ?? false);
    if (sheetHadFocus) restore.focus();
  }, [open]);

  const groups = useMemo(() => {
    const filtered = filterRows(commands, query);
    return COMMAND_CATEGORY_ORDER.map((category) => ({
      category,
      commands: filtered.filter((command) => commandCategory(command.id) === category),
    })).filter((group) => group.commands.length > 0);
  }, [commands, query]);

  const visibleCount = groups.reduce((count, group) => count + group.commands.length, 0);

  if (!open) return null;

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    // Modal: keep every chord (including the palette chord) inside the
    // dialog and away from the global shortcut listener.
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key === 'Tab') {
      const focusables = dialogRef.current?.querySelectorAll<HTMLElement>('input, button');
      if (!focusables || focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
  };

  return (
    <div className="cheatsheet-backdrop" data-testid="cheatsheet-backdrop" onPointerDown={onClose}>
      <div
        ref={dialogRef}
        className="cheatsheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="cheatsheet-title"
        aria-describedby="cheatsheet-description"
        onPointerDown={(event) => event.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        <header className="cheatsheet-header">
          <div className="cheatsheet-heading">
            <h2 id="cheatsheet-title" className="cheatsheet-title">
              Keyboard Shortcuts
            </h2>
            <p id="cheatsheet-description" className="cheatsheet-subtitle">
              Every Selora command, grouped by area. Type to filter.
            </p>
          </div>
          <button
            type="button"
            className="cheatsheet-close"
            aria-label="Close keyboard shortcuts"
            onClick={onClose}
          >
            <Icon name="close" />
          </button>
        </header>

        <div className="cheatsheet-search-row">
          <Icon name="search" />
          <input
            ref={inputRef}
            className="cheatsheet-search"
            type="search"
            aria-label="Filter shortcuts"
            placeholder="Filter commands or keys…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
          <span className="cheatsheet-esc" aria-hidden="true">
            Esc
          </span>
        </div>

        <div className="cheatsheet-body">
          {groups.length === 0 ? (
            <p className="cheatsheet-empty" role="status">
              No matching shortcuts.
            </p>
          ) : (
            groups.map((group) => (
              <section
                key={group.category}
                className="cheatsheet-group"
                aria-labelledby={categoryDomId(group.category)}
              >
                <h3 id={categoryDomId(group.category)} className="cheatsheet-group-title">
                  {group.category}
                </h3>
                <ul className="cheatsheet-list">
                  {group.commands.map((command) => (
                    <li
                      key={command.id}
                      className="cheatsheet-row"
                      data-command-id={command.id}
                      data-testid="cheatsheet-row"
                    >
                      <span className="cheatsheet-command">{command.title}</span>
                      {command.shortcut ? (
                        <span className="cheatsheet-keys" aria-label={`Shortcut: ${command.shortcut}`}>
                          {command.shortcut.split('+').map((keyPart, index) => (
                            <kbd key={`${command.id}-${keyPart}-${index}`} className="cheatsheet-key">
                              {keyPart}
                            </kbd>
                          ))}
                        </span>
                      ) : (
                        <span className="cheatsheet-unbound">Unassigned</span>
                      )}
                    </li>
                  ))}
                </ul>
              </section>
            ))
          )}
        </div>

        <footer className="cheatsheet-footer">
          <span aria-live="polite">
            {visibleCount} of {commands.length} commands
          </span>
          <span aria-hidden="true">Esc closes · focus returns to your last control</span>
        </footer>
      </div>
    </div>
  );
}
