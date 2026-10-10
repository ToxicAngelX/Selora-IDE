// Selora IDE — Phase Z2 custom title bar (DESIGN §3.1).
//
// 32px, --ink-canvas background, no bottom border. Left: the 16px orbital
// mark + "Selora IDE". Center-left: File / Edit / View / Help menus — only
// View opens a working dropdown (the eleven palette commands); the others
// show one disabled row, "Available in a later slice" (ARCHITECTURE §7.3 —
// no fake menu items). Right: the three window controls (44×32, no red
// close-hover — §12). The drag region covers the bar except interactive
// controls; double-click toggles maximize natively.
import { useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Bot, Columns, Code2, GitBranch } from '../components/icons';
import { Icon } from '../components/Icon';
import { SeloraMark } from '../components/SeloraMark';
import { paletteCommands, type CommandSpec } from '../features/keymap/commands';

type MenuKind = 'file' | 'edit' | 'view' | 'help';

const MENU_LABEL: Record<MenuKind, string> = {
  file: 'File',
  edit: 'Edit',
  view: 'View',
  help: 'Help',
};

const LATER_SLICE_LABEL = 'Available in a later slice';

function TitleBarMenu({
  kind,
  commands,
  onRunCommand,
}: {
  kind: MenuKind;
  commands: CommandSpec[];
  onRunCommand: (id: CommandSpec['id']) => void;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current && event.target instanceof Node && !rootRef.current.contains(event.target)) {
        setOpen(false);
      }
    };
    window.addEventListener('pointerdown', onPointerDown);
    return () => window.removeEventListener('pointerdown', onPointerDown);
  }, [open]);

  useEffect(() => {
    if (open) {
      const first = rootRef.current?.querySelector<HTMLElement>('[role="menuitem"]:not([disabled])');
      first?.focus();
    } else {
      // Focus returns to the invoking control when the popover closes (DESIGN §7).
      if (rootRef.current && document.activeElement instanceof Node && rootRef.current.contains(document.activeElement)) {
        buttonRef.current?.focus();
      }
    }
  }, [open]);

  const close = () => setOpen(false);
  const viewCommands = paletteCommands(commands);

  const onMenuKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close();
    }
  };

  return (
    <div ref={rootRef} className="titlebar-menu" onKeyDown={onMenuKeyDown}>
      <button
        ref={buttonRef}
        type="button"
        className={`titlebar-menu-button${open ? ' is-open' : ''}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {MENU_LABEL[kind]}
      </button>
      {open ? (
        <div className="titlebar-dropdown" role="menu" aria-label={`${MENU_LABEL[kind]} menu`}>
          {kind === 'view' ? (
            viewCommands.map((command) => (
              <button
                key={command.id}
                type="button"
                role="menuitem"
                className="titlebar-menu-item"
                onClick={() => {
                  close();
                  onRunCommand(command.id);
                }}
              >
                <span>{command.title}</span>
                {command.shortcut ? <span className="titlebar-menu-key">{command.shortcut}</span> : null}
              </button>
            ))
          ) : (
            <button type="button" role="menuitem" className="titlebar-menu-item" disabled>
              <span>{LATER_SLICE_LABEL}</span>
            </button>
          )}
        </div>
      ) : null}
    </div>
  );
}

function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    withCurrentWindow((win) => {
      const update = () => {
        void win
          .isMaximized()
          .then(setMaximized)
          .catch(() => undefined);
      };
      update();
      void win
        .onResized(update)
        .then((stop) => {
          unlisten = stop;
        })
        .catch(() => undefined);
    });
    return () => unlisten?.();
  }, []);

  return (
    <div className="titlebar-window-controls">
      <button
        type="button"
        className="window-control"
        aria-label="Minimize window"
        onClick={() => withCurrentWindow((win) => void win.minimize())}
      >
        <Icon name="win-min" />
      </button>
      <button
        type="button"
        className="window-control"
        aria-label={maximized ? 'Restore window' : 'Maximize window'}
        onClick={() => withCurrentWindow((win) => void win.toggleMaximize())}
      >
        <Icon name={maximized ? 'win-restore' : 'win-max'} />
      </button>
      <button
        type="button"
        className="window-control"
        aria-label="Close window"
        onClick={() => withCurrentWindow((win) => void win.close())}
      >
        <Icon name="close" />
      </button>
    </div>
  );
}

/**
 * Run an action against the current Tauri window; outside the WebView
 * (browser preview, tests) the constructor throws and the control no-ops —
 * the Z2 window controls exist only inside the shell.
 */
function withCurrentWindow(action: (win: ReturnType<typeof getCurrentWindow>) => void): void {
  try {
    action(getCurrentWindow());
  } catch {
    // Not running inside the Tauri WebView.
  }
}

export type TitleBarProps = {
  commands: CommandSpec[];
  onRunCommand: (id: CommandSpec['id']) => void;
  viewMode?: 'agent' | 'split' | 'editor';
  onSelectViewMode?: (mode: 'agent' | 'split' | 'editor') => void;
};

export function TitleBar({ commands, onRunCommand, viewMode = 'split', onSelectViewMode }: TitleBarProps) {
  return (
    <header className="titlebar" data-tauri-drag-region="">
      <div className="titlebar-identity">
        <span className="titlebar-mark" data-tauri-drag-region="">
          <SeloraMark size={16} />
        </span>
        <span className="titlebar-title" data-tauri-drag-region="">
          Selora IDE
        </span>
      </div>
      <nav className="titlebar-menus" aria-label="Window menus">
        <TitleBarMenu kind="file" commands={commands} onRunCommand={onRunCommand} />
        <TitleBarMenu kind="edit" commands={commands} onRunCommand={onRunCommand} />
        <TitleBarMenu kind="view" commands={commands} onRunCommand={onRunCommand} />
        <TitleBarMenu kind="help" commands={commands} onRunCommand={onRunCommand} />
      </nav>

      {/* Modern Agentic Center Bar (ZCode / Codex Benchmark) */}
      <div className="titlebar-center" data-tauri-drag-region="">
        <div className="titlebar-session-badge" title="Active agent workspace session">
          <span className="session-status-dot" />
          <span className="session-title-text">Agent Workspace</span>
        </div>

        <div className="titlebar-git-pill" title="Current git branch">
          <GitBranch size={12} className="git-icon" />
          <span className="git-branch-text">main</span>
        </div>

        {onSelectViewMode && (
          <div className="titlebar-view-switcher" role="radiogroup" aria-label="Workbench View Mode">
            <button
              type="button"
              className={`view-mode-pill ${viewMode === 'agent' ? 'is-active' : ''}`}
              onClick={() => onSelectViewMode('agent')}
              title="Agent Workbench View"
              aria-label="Agent View"
            >
              <Bot size={13} />
              <span>Agent</span>
            </button>
            <button
              type="button"
              className={`view-mode-pill ${viewMode === 'split' ? 'is-active' : ''}`}
              onClick={() => onSelectViewMode('split')}
              title="Split Agent and Editor View"
              aria-label="Split View"
            >
              <Columns size={13} />
              <span>Split</span>
            </button>
            <button
              type="button"
              className={`view-mode-pill ${viewMode === 'editor' ? 'is-active' : ''}`}
              onClick={() => onSelectViewMode('editor')}
              title="Editor View"
              aria-label="Editor View"
            >
              <Code2 size={13} />
              <span>Editor</span>
            </button>
          </div>
        )}
      </div>

      <WindowControls />
    </header>
  );
}
