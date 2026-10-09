// Selora IDE — Phase Z2 command palette.
//
// ARCHITECTURE §7.2 / UI_SPEC §5: a --ink-mid, 1px --ink-hi, --radius-2
// overlay with the `bloom` entrance, listing the eleven shell commands with
// their shortcut labels. Fuzzy filter, fully keyboard navigable
// (↑/↓/Home/End/Enter; Esc closes via the global shortcut hook), proper
// dialog/combobox/listbox roles, and focus returns to the invoking control
// on close (DESIGN §7).
import { useEffect, useMemo, useRef, useState } from 'react';
import { filterCommands, paletteCommands, type CommandSpec } from '../features/keymap/commands';
import { Icon } from './Icon';

export type CommandPaletteProps = {
  open: boolean;
  commands: CommandSpec[];
  onRun: (id: CommandSpec['id']) => void;
  onClose: () => void;
};

export function CommandPalette({ open, commands, onRun, onClose }: CommandPaletteProps) {
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  const list = useMemo(() => filterCommands(paletteCommands(commands), query), [commands, query]);

  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setQuery('');
    setActiveIndex(0);
    inputRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (open) return;
    // Focus returns to the invoking control when the dialog closes (DESIGN §7)
    // — but never steal focus the user already moved elsewhere.
    const restore = restoreFocusRef.current;
    restoreFocusRef.current = null;
    if (!restore || !restore.isConnected) return;
    const current = document.activeElement;
    const paletteHadFocus = current === document.body || (rootRef.current?.contains(current) ?? false);
    if (paletteHadFocus) restore.focus();
  }, [open]);

  useEffect(() => {
    const count = list.length;
    setActiveIndex((index) => (index >= count ? Math.max(0, count - 1) : index));
  }, [list.length]);

  useEffect(() => {
    if (!open) return;
    const option = rootRef.current?.querySelector<HTMLElement>(`[data-option-index="${activeIndex}"]`);
    option?.scrollIntoView?.({ block: 'nearest' });
  }, [open, activeIndex]);

  if (!open) return null;

  const runActive = () => {
    const command = list[activeIndex];
    if (!command) return;
    onRun(command.id);
    onClose();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        setActiveIndex((index) => (list.length === 0 ? 0 : (index + 1) % list.length));
        return;
      case 'ArrowUp':
        event.preventDefault();
        setActiveIndex((index) => (list.length === 0 ? 0 : (index - 1 + list.length) % list.length));
        return;
      case 'Home':
        event.preventDefault();
        setActiveIndex(0);
        return;
      case 'End':
        event.preventDefault();
        setActiveIndex(Math.max(0, list.length - 1));
        return;
      case 'Enter':
        event.preventDefault();
        runActive();
        return;
      case 'Tab':
        // The combobox is the only tab stop; keep focus trapped here.
        event.preventDefault();
        return;
      default:
        return;
    }
  };

  return (
    <div className="palette-backdrop" onPointerDown={onClose} data-testid="palette-backdrop">
      <div
        ref={rootRef}
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <div className="palette-input-row">
          <span className="palette-prefix" aria-hidden="true">
            <Icon name="command" />
          </span>
          <input
            ref={inputRef}
            className="palette-input"
            type="text"
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-listbox"
            aria-activedescendant={list.length > 0 ? `palette-option-${activeIndex}` : undefined}
            aria-autocomplete="list"
            aria-label="Search commands"
            placeholder="Type a command…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={onKeyDown}
            spellCheck={false}
          />
        </div>
        {list.length === 0 ? (
          <p className="palette-empty" role="status">
            No matching commands.
          </p>
        ) : (
          <ul className="palette-list" id="palette-listbox" role="listbox" aria-label="Commands">
            {list.map((command, index) => (
              <li
                key={command.id}
                id={`palette-option-${index}`}
                data-option-index={index}
                role="option"
                aria-selected={index === activeIndex}
                className={`palette-option${index === activeIndex ? ' is-active' : ''}`}
                onPointerDown={(event) => {
                  event.preventDefault();
                  setActiveIndex(index);
                }}
                onClick={runActive}
              >
                <span className="palette-option-label">{command.title}</span>
                {command.shortcut ? <span className="palette-option-key">{command.shortcut}</span> : null}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
