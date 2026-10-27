// Selora IDE — Phase Z2 command palette (Raycast / Spotlight Elevated).
//
// ARCHITECTURE §7.2 / UI_SPEC §5: Sleek backdrop blur, hairline border with
// subtle accent glow, categorized sections (Recent, View & Layout, File Actions,
// Agent Tools), fuzzy match letter highlighting, shortcut pills, keyboard
// navigation (↑/↓/Home/End/Enter; Esc closes), proper dialog/combobox/listbox
// ARIA roles, and focus restoration to the invoking control on close (DESIGN §7).
import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { filterCommands, paletteCommands, type CommandSpec } from '../features/keymap/commands';
import { Icon, type IconName } from './Icon';

export type CommandPaletteProps = {
  open: boolean;
  commands: CommandSpec[];
  onRun: (id: CommandSpec['id']) => void;
  onClose: () => void;
};

export type CategoryKey = 'all' | 'recent' | 'view' | 'files' | 'agent';

type CategoryMeta = {
  id: 'view' | 'files' | 'agent';
  label: string;
};

const COMMAND_CATEGORIES: Record<string, CategoryMeta> = {
  'selora.togglePrimarySidebar': { id: 'view', label: 'View & Layout' },
  'selora.toggleAgentDock': { id: 'agent', label: 'Agent Tools' },
  'selora.toggleBottomDock': { id: 'view', label: 'View & Layout' },
  'selora.showFiles': { id: 'files', label: 'File Actions' },
  'selora.showSearch': { id: 'files', label: 'File Actions' },
  'selora.showGit': { id: 'files', label: 'File Actions' },
  'selora.showProblems': { id: 'agent', label: 'Agent Tools' },
  'selora.showSettings': { id: 'view', label: 'View & Layout' },
  'selora.showOutput': { id: 'view', label: 'View & Layout' },
  'selora.showTerminal': { id: 'view', label: 'View & Layout' },
  'selora.saveLayout': { id: 'view', label: 'View & Layout' },
};

const COMMAND_ICONS: Record<string, IconName> = {
  'selora.togglePrimarySidebar': 'file',
  'selora.toggleAgentDock': 'chat',
  'selora.toggleBottomDock': 'command',
  'selora.showFiles': 'file',
  'selora.showSearch': 'search',
  'selora.showGit': 'git-branch',
  'selora.showProblems': 'error',
  'selora.showSettings': 'settings',
  'selora.showOutput': 'command',
  'selora.showTerminal': 'command',
  'selora.saveLayout': 'check',
};

const CATEGORY_TABS: Array<{ key: CategoryKey; label: string; icon: IconName }> = [
  { key: 'all', label: 'All', icon: 'command' },
  { key: 'recent', label: 'Recent', icon: 'command' },
  { key: 'view', label: 'View & Layout', icon: 'win-restore' },
  { key: 'files', label: 'File Actions', icon: 'file' },
  { key: 'agent', label: 'Agent Tools', icon: 'chat' },
];

/** Highlights characters matching query (substring first, then subsequence). */
function highlightFuzzyMatch(title: string, query: string): ReactNode {
  const q = query.trim().toLowerCase();
  if (!q) return title;

  const lowerTitle = title.toLowerCase();
  const directIdx = lowerTitle.indexOf(q);
  if (directIdx !== -1) {
    const before = title.slice(0, directIdx);
    const match = title.slice(directIdx, directIdx + q.length);
    const after = title.slice(directIdx + q.length);
    return (
      <>
        {before}
        <mark className="palette-highlight">{match}</mark>
        {after}
      </>
    );
  }

  // Fuzzy subsequence match
  const parts: ReactNode[] = [];
  let qIdx = 0;
  for (let i = 0; i < title.length; i++) {
    const char = title[i];
    if (qIdx < q.length && char.toLowerCase() === q[qIdx]) {
      parts.push(
        <mark key={i} className="palette-highlight">
          {char}
        </mark>,
      );
      qIdx++;
    } else {
      parts.push(char);
    }
  }
  return <>{parts}</>;
}

export function CommandPalette({ open, commands, onRun, onClose }: CommandPaletteProps) {
  const [query, setQuery] = useState('');
  const [activeCategory, setActiveCategory] = useState<CategoryKey>('all');
  const [activeIndex, setActiveIndex] = useState(0);
  const [recentIds, setRecentIds] = useState<string[]>(() => {
    try {
      const stored = localStorage.getItem('selora:recent-commands');
      return stored ? JSON.parse(stored) : [];
    } catch {
      return [];
    }
  });

  const inputRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  // Filter commands by active category tab if user explicitly selected one
  const baseCommands = useMemo(() => {
    const all = paletteCommands(commands);
    if (activeCategory === 'recent') {
      const recentList = all.filter((cmd) => recentIds.includes(cmd.id));
      return recentList.length > 0 ? recentList : all;
    }
    if (activeCategory === 'view' || activeCategory === 'files' || activeCategory === 'agent') {
      return all.filter((cmd) => COMMAND_CATEGORIES[cmd.id]?.id === activeCategory);
    }
    return all;
  }, [commands, activeCategory, recentIds]);

  // Apply fuzzy filter
  const list = useMemo(() => filterCommands(baseCommands, query), [baseCommands, query]);

  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setQuery('');
    setActiveCategory('all');
    setActiveIndex(0);
    inputRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (open) return;
    // Focus returns to invoking control when dialog closes (DESIGN §7)
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

  const runCommand = (command: CommandSpec) => {
    // Track recently executed command
    setRecentIds((prev) => {
      const updated = [command.id, ...prev.filter((id) => id !== command.id)].slice(0, 8);
      try {
        localStorage.setItem('selora:recent-commands', JSON.stringify(updated));
      } catch {
        // storage disabled/unavailable
      }
      return updated;
    });

    onRun(command.id);
    onClose();
  };

  const runActive = () => {
    const command = list[activeIndex];
    if (!command) return;
    runCommand(command);
  };

  // Pointer and assistive-technology activation run the option that was
  // actually clicked, not whichever option happened to be active before.
  const runOption = (index: number) => {
    const command = list[index];
    if (!command) return;
    runCommand(command);
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
      default:
        return;
    }
  };

  const onDialogKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusables = Array.from(
      rootRef.current?.querySelectorAll<HTMLElement>('input, button:not([disabled])') ?? [],
    );
    if (focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div className="palette-backdrop" onPointerDown={onClose} data-testid="palette-backdrop">
      <div
        ref={rootRef}
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-labelledby="palette-title"
        onPointerDown={(event) => event.stopPropagation()}
        onKeyDown={onDialogKeyDown}
      >
        <h2 id="palette-title" className="visually-hidden">Command palette</h2>
        {/* Top search input row */}
        <div className="palette-input-row">
          <span className="palette-prefix" aria-hidden="true">
            <Icon name="command" />
          </span>
          <input
            ref={inputRef}
            className="palette-input"
            type="text"
            role="combobox"
            aria-expanded={list.length > 0}
            aria-controls="palette-listbox"
            aria-activedescendant={list.length > 0 ? `palette-option-${activeIndex}` : undefined}
            aria-autocomplete="list"
            aria-label="Search commands"
            placeholder="Type a command or search…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={onKeyDown}
            spellCheck={false}
          />
          <span className="palette-esc-badge" aria-hidden="true">
            ESC
          </span>
        </div>

        {/* Category Tabs */}
        <div className="palette-categories" role="presentation">
          {CATEGORY_TABS.map((cat) => {
            const isSelected = activeCategory === cat.key;
            return (
              <button
                key={cat.key}
                type="button"
                className={`palette-cat-pill${isSelected ? ' is-active' : ''}`}
                aria-pressed={isSelected}
                onClick={() => {
                  setActiveCategory(cat.key);
                  setActiveIndex(0);
                  inputRef.current?.focus();
                }}
              >
                <span>{cat.label}</span>
              </button>
            );
          })}
        </div>

        {/* Results List or Empty State */}
        {list.length === 0 ? (
          <div className="palette-empty-container">
            <div className="palette-empty-illustration" aria-hidden="true">
              <svg width="44" height="44" viewBox="0 0 44 44" fill="none">
                <circle className="palette-empty-orbit" cx="22" cy="22" r="20" strokeWidth="1.5" strokeDasharray="4 4" />
                <path className="palette-empty-x" d="M16 16L28 28M28 16L16 28" strokeWidth="1.5" strokeLinecap="round" opacity="0.6" />
                <circle className="palette-empty-x-ring" cx="22" cy="22" r="10" strokeWidth="1.2" />
              </svg>
            </div>
            <p className="palette-empty" role="status">
              No matching commands.
            </p>
            <span className="palette-empty-hint" aria-hidden="true">
              Try searching for layout, files, git, or agent actions
            </span>
          </div>
        ) : (
          <ul className="palette-list" id="palette-listbox" role="listbox" aria-label="Commands">
            {list.map((command, index) => {
              const category = COMMAND_CATEGORIES[command.id];
              const iconName = COMMAND_ICONS[command.id] ?? 'command';
              const isRecent = recentIds.includes(command.id);

              return (
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
                  onClick={() => runOption(index)}
                >
                  <div className="palette-option-left">
                    <span className="palette-item-icon" aria-hidden="true">
                      <Icon name={iconName} />
                    </span>
                    <span className="palette-option-label">
                      {highlightFuzzyMatch(command.title, query)}
                    </span>
                    {category && (
                      <span className={`palette-cat-tag cat-${category.id}`} aria-hidden="true">
                        {category.label}
                      </span>
                    )}
                    {isRecent && activeCategory !== 'recent' && (
                      <span className="palette-recent-tag" aria-hidden="true">
                        Recent
                      </span>
                    )}
                  </div>
                  {command.shortcut ? (
                    <span className="palette-option-key" aria-label={`Shortcut: ${command.shortcut}`}>
                      {command.shortcut.split('+').map((keyPart, kIdx) => (
                        <kbd key={kIdx} className="palette-kbd">
                          {keyPart}
                        </kbd>
                      ))}
                    </span>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}

        {/* Raycast / Spotlight Style Keyboard Hints Footer */}
        <div className="palette-footer" aria-hidden="true">
          <div className="palette-footer-left">
            <span className="palette-footer-brand">Selora IDE</span>
          </div>
          <div className="palette-footer-shortcuts">
            <span className="palette-footer-item">
              <kbd className="palette-kbd">↑</kbd>
              <kbd className="palette-kbd">↓</kbd>
              <span>navigate</span>
            </span>
            <span className="palette-footer-divider">·</span>
            <span className="palette-footer-item">
              <kbd className="palette-kbd">↵</kbd>
              <span>run</span>
            </span>
            <span className="palette-footer-divider">·</span>
            <span className="palette-footer-item">
              <kbd className="palette-kbd">esc</kbd>
              <span>close</span>
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
