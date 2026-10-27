// Selora IDE — Phase Z2 custom title bar (DESIGN §3.1, ARCHITECTURE §7.3).
//
// 32px, --ink-canvas background. Left: 16px orbital mark + "Selora IDE" + File/Edit/View/Help menus.
// Center: Command search trigger pill + workspace session / git branch pill + workbench view mode switcher.
// Right: Native-acting window controls with sleek micro-interactions.
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Bot, Columns, Code2, GitBranch, Search } from '../components/icons';
import { Icon } from '../components/Icon';
import { NotificationCenter } from '../components/NotificationCenter';
import { SeloraMark } from '../components/SeloraMark';
import {
  MENU_COMMAND_IDS,
  paletteCommands,
  type CommandId,
  type CommandSpec,
} from '../features/keymap/commands';
import type { BootstrapState } from '../bridge/bridge';
import { getAppSnapshot, subscribeApp } from '../app/app-store';

type MenuKind = 'file' | 'edit' | 'view' | 'help';

const MENU_LABEL: Record<MenuKind, string> = {
  file: 'File',
  edit: 'Edit',
  view: 'View',
  help: 'Help',
};

function menuCommands(commands: CommandSpec[], kind: MenuKind): CommandSpec[] {
  if (kind === 'view') return paletteCommands(commands);
  const ids: CommandId[] = MENU_COMMAND_IDS[kind];
  return ids
    .map((id) => commands.find((command) => command.id === id))
    .filter((command): command is CommandSpec => command !== undefined);
}

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
  const items = menuCommands(commands, kind);

  const onMenuKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    const items = Array.from(
      rootRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not([disabled])') ?? [],
    );
    if (items.length === 0) return;
    event.preventDefault();
    const current = items.indexOf(document.activeElement as HTMLElement);
    let next = current;
    if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = items.length - 1;
    else if (event.key === 'ArrowDown') next = current < 0 ? 0 : (current + 1) % items.length;
    else if (event.key === 'ArrowUp') next = current < 0 ? items.length - 1 : (current - 1 + items.length) % items.length;
    items[next]?.focus();
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
          {items.map((command) => (
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
          ))}
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
        className="window-control window-control-min"
        aria-label="Minimize window"
        title="Minimize"
        onClick={() => withCurrentWindow((win) => void win.minimize())}
      >
        <Icon name="win-min" />
      </button>
      <button
        type="button"
        className="window-control window-control-max"
        aria-label={maximized ? 'Restore window' : 'Maximize window'}
        title={maximized ? 'Restore' : 'Maximize'}
        onClick={() => withCurrentWindow((win) => void win.toggleMaximize())}
      >
        <Icon name={maximized ? 'win-restore' : 'win-max'} />
      </button>
      <button
        type="button"
        className="window-control window-control-close is-close"
        aria-label="Close window"
        title="Close"
        onClick={() => withCurrentWindow((win) => void win.close())}
      >
        <Icon name="close" />
      </button>
    </div>
  );
}

/**
 * Run an action against the current Tauri window; outside the WebView
 * (browser preview, tests) the constructor throws and the control no-ops.
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
  bootstrap?: BootstrapState;
};

export function TitleBar({
  commands,
  onRunCommand,
  viewMode = 'split',
  onSelectViewMode,
  bootstrap: initialBootstrap,
}: TitleBarProps) {
  const appState = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const bootstrap = initialBootstrap ?? appState.bootstrap;

  const workspaceDisplay =
    bootstrap?.workspace?.display_name && bootstrap.workspace.display_name !== 'No workspace open'
      ? bootstrap.workspace.display_name
      : 'Agent Workspace';
  const gitBranch = bootstrap?.git?.branch || 'main';

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

      {/* Modern Center Bar with Search Pill, Workspace Breadcrumb & View Switcher */}
      <div className="titlebar-center" data-tauri-drag-region="">
        {/* Quick-action command search trigger pill */}
        <button
          type="button"
          className="titlebar-search-trigger"
          onClick={() => onRunCommand('selora.openCommandPalette')}
          title="Open Command Palette (Ctrl+Shift+P)"
          aria-label="Open Command Palette"
        >
          <Search size={12} className="titlebar-search-icon" aria-hidden="true" />
          <span className="titlebar-search-text">Search or type a command...</span>
          <kbd className="titlebar-search-kbd">Ctrl+Shift+P&#x200E;</kbd>
        </button>

        {/* Workspace Session badge */}
        <div className="titlebar-session-badge" title={`Active workspace: ${workspaceDisplay}`}>
          <span className="session-status-dot" aria-hidden="true" />
          <span className="session-title-text">{workspaceDisplay}</span>
        </div>

        {/* Git branch pill */}
        <div className="titlebar-git-pill" title={`Current git branch: ${gitBranch}`}>
          <GitBranch size={12} className="git-icon" aria-hidden="true" />
          <span className="git-branch-text">{gitBranch}</span>
        </div>

        {/* Workbench View Mode Switcher */}
        {onSelectViewMode && (
          <div className="titlebar-view-switcher" role="group" aria-label="Workbench View Mode">
            <button
              type="button"
              className={`view-mode-pill ${viewMode === 'agent' ? 'is-active' : ''}`}
              onClick={() => onSelectViewMode('agent')}
              title="Agent Workbench View"
              aria-label="Agent View"
              aria-pressed={viewMode === 'agent'}
            >
              <Bot size={13} aria-hidden="true" />
              <span>Agent</span>
            </button>
            <button
              type="button"
              className={`view-mode-pill ${viewMode === 'split' ? 'is-active' : ''}`}
              onClick={() => onSelectViewMode('split')}
              title="Split Agent and Editor View"
              aria-label="Split View"
              aria-pressed={viewMode === 'split'}
            >
              <Columns size={13} aria-hidden="true" />
              <span>Split</span>
            </button>
            <button
              type="button"
              className={`view-mode-pill ${viewMode === 'editor' ? 'is-active' : ''}`}
              onClick={() => onSelectViewMode('editor')}
              title="Editor View"
              aria-label="Editor View"
              aria-pressed={viewMode === 'editor'}
            >
              <Code2 size={13} aria-hidden="true" />
              <span>Editor</span>
            </button>
          </div>
        )}
      </div>

      <div className="titlebar-right">
        <NotificationCenter />
        <WindowControls />
      </div>
    </header>
  );
}
