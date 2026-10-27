// Selora IDE — Phase Z2 command registry.
//
// The canonical keyboard + palette command set (ARCHITECTURE §7.1). The
// palette lists every command except openCommandPalette (the chord that
// opens the palette, not an entry inside it). The rebindable
// pair resolves from `bootstrap.settings`; an unparseable stored string
// falls back to the default and logs a console warning — never throws.
import { canonicalKeybind, parseKeybind, type Keybind } from './keybind';
import type { Layout, SettingsView, SurfaceId } from '../../bridge/bridge';

export type CommandId =
  | 'selora.togglePrimarySidebar'
  | 'selora.toggleAgentDock'
  | 'selora.toggleBottomDock'
  | 'selora.showFiles'
  | 'selora.showSearch'
  | 'selora.showGit'
  | 'selora.showProblems'
  | 'selora.showSettings'
  | 'selora.showOutput'
  | 'selora.showTerminal'
  | 'selora.openCommandPalette'
  | 'selora.showKeyboardShortcuts'
  | 'selora.about'
  | 'selora.saveLayout'
  | 'selora.fileOpenFolder'
  | 'selora.fileSave'
  | 'selora.fileCloseEditor'
  | 'selora.fileCloseWindow'
  | 'selora.editFindInFiles'
  | 'selora.editCommandPalette'
  | 'selora.helpDocumentation';

export type CommandAction =
  | { kind: 'toggle-surface'; surface: SurfaceId }
  /** Ctrl+B: ToggleSurface{surface: current activity_view} (ARCHITECTURE §7.1). */
  | { kind: 'toggle-activity-view' }
  /** Ctrl+J: ToggleSurface{surface: current bottom_view}. */
  | { kind: 'toggle-bottom-view' }
  | { kind: 'save-layout' }
  | { kind: 'open-palette' }
  /** Z5c: Help → Keyboard Shortcuts (open the cheat-sheet dialog). */
  | { kind: 'open-cheatsheet' }
  /** Z5c: Help → About Selora (dispatch `selora:about`). */
  | { kind: 'about' }
  | { kind: 'open-folder' }
  | { kind: 'save-active' }
  | { kind: 'close-active' }
  | { kind: 'close-window' }
  /** Help → Documentation (an allowlisted external URL). */
  | { kind: 'open-documentation' };

export type CommandSpec = {
  id: CommandId;
  title: string;
  shortcut: string | null;
  action: CommandAction;
};

/** Every SurfaceId must appear in at least one command (ARCHITECTURE §10). */
export const SURFACE_IDS: SurfaceId[] = [
  'files', 'search', 'git', 'problems', 'settings', 'agent', 'output', 'terminal_placeholder',
];

const DEFAULT_COMMAND_PALETTE = 'Ctrl+Shift+P';
const DEFAULT_TOGGLE_CHAT = 'Ctrl+L';
const DEFAULT_FUZZY_FIND = 'Ctrl+P';
const DEFAULT_SAVE = 'Ctrl+S';

type CommandEntry = {
  id: CommandId;
  title: string;
  action: CommandAction;
  /** null = no shortcut (the palette-only commands). */
  fixedBinding: string | null;
  rebindable: 'command_palette' | 'toggle_chat' | 'fuzzy_find' | 'save' | null;
};

const COMMANDS: CommandEntry[] = [
  { id: 'selora.togglePrimarySidebar', title: 'Toggle Primary Sidebar', action: { kind: 'toggle-activity-view' }, fixedBinding: 'Ctrl+B', rebindable: null },
  { id: 'selora.toggleAgentDock', title: 'Toggle Agent Dock', action: { kind: 'toggle-surface', surface: 'agent' }, fixedBinding: null, rebindable: 'toggle_chat' },
  { id: 'selora.toggleBottomDock', title: 'Toggle Bottom Dock', action: { kind: 'toggle-bottom-view' }, fixedBinding: 'Ctrl+J', rebindable: null },
  { id: 'selora.showFiles', title: 'Show Files', action: { kind: 'toggle-surface', surface: 'files' }, fixedBinding: 'Ctrl+Shift+E', rebindable: null },
  { id: 'selora.showSearch', title: 'Show Search', action: { kind: 'toggle-surface', surface: 'search' }, fixedBinding: 'Ctrl+Shift+F', rebindable: null },
  { id: 'selora.showGit', title: 'Show Git', action: { kind: 'toggle-surface', surface: 'git' }, fixedBinding: 'Ctrl+Shift+G', rebindable: null },
  { id: 'selora.showProblems', title: 'Show Problems', action: { kind: 'toggle-surface', surface: 'problems' }, fixedBinding: 'Ctrl+Shift+M', rebindable: null },
  { id: 'selora.showSettings', title: 'Show Settings', action: { kind: 'toggle-surface', surface: 'settings' }, fixedBinding: 'Ctrl+Comma', rebindable: null },
  { id: 'selora.showOutput', title: 'Show Output', action: { kind: 'toggle-surface', surface: 'output' }, fixedBinding: null, rebindable: null },
  { id: 'selora.showTerminal', title: 'Show Terminal', action: { kind: 'toggle-surface', surface: 'terminal_placeholder' }, fixedBinding: null, rebindable: null },
  { id: 'selora.openCommandPalette', title: 'Open Command Palette', action: { kind: 'open-palette' }, fixedBinding: null, rebindable: 'command_palette' },
  { id: 'selora.showKeyboardShortcuts', title: 'Keyboard Shortcuts', action: { kind: 'open-cheatsheet' }, fixedBinding: 'Ctrl+/', rebindable: null },
  { id: 'selora.about', title: 'About Selora', action: { kind: 'about' }, fixedBinding: null, rebindable: null },
  { id: 'selora.saveLayout', title: 'Save Layout', action: { kind: 'save-layout' }, fixedBinding: null, rebindable: null },
  { id: 'selora.fileOpenFolder', title: 'Open Folder…', action: { kind: 'open-folder' }, fixedBinding: null, rebindable: null },
{ id: 'selora.fileSave', title: 'Save', action: { kind: 'save-active' }, fixedBinding: null, rebindable: 'save' },
  { id: 'selora.fileCloseEditor', title: 'Close Editor', action: { kind: 'close-active' }, fixedBinding: 'Ctrl+F4', rebindable: null },
  { id: 'selora.fileCloseWindow', title: 'Exit Selora IDE', action: { kind: 'close-window' }, fixedBinding: null, rebindable: null },
  { id: 'selora.editFindInFiles', title: 'Find in Files', action: { kind: 'toggle-surface', surface: 'search' }, fixedBinding: null, rebindable: 'fuzzy_find' },
  { id: 'selora.editCommandPalette', title: 'Command Palette', action: { kind: 'open-palette' }, fixedBinding: null, rebindable: null },
  { id: 'selora.helpDocumentation', title: 'Documentation', action: { kind: 'open-documentation' }, fixedBinding: null, rebindable: null },
];

/** Explicit File/Edit/Help menu grouping; View lists the palette's view commands. */
export const MENU_COMMAND_IDS: Record<'file' | 'edit' | 'help', CommandId[]> = {
  file: [
    'selora.fileOpenFolder',
    'selora.fileSave',
    'selora.fileCloseEditor',
    'selora.saveLayout',
    'selora.fileCloseWindow',
  ],
  edit: [
    'selora.editFindInFiles',
    'selora.editCommandPalette',
    'selora.showKeyboardShortcuts',
  ],
  help: [
    'selora.helpDocumentation',
    'selora.showKeyboardShortcuts',
    'selora.about',
  ],
};

/**
 * Z5c — the cheat sheet's grouping. Categories mirror the palette's tabs so
 * a command reads the same everywhere (UI_SPEC §5); Help is new for the two
 * Z5 entries. The map is exhaustive over CommandId by construction.
 */
export type CommandCategory = 'View & Layout' | 'File Actions' | 'Agent Tools' | 'Help';

export const COMMAND_CATEGORY_ORDER: CommandCategory[] = [
  'View & Layout',
  'File Actions',
  'Agent Tools',
  'Help',
];

const COMMAND_CATEGORIES: Record<CommandId, CommandCategory> = {
  'selora.togglePrimarySidebar': 'View & Layout',
  'selora.toggleAgentDock': 'Agent Tools',
  'selora.toggleBottomDock': 'View & Layout',
  'selora.showFiles': 'File Actions',
  'selora.showSearch': 'File Actions',
  'selora.showGit': 'File Actions',
  'selora.showProblems': 'Agent Tools',
  'selora.showSettings': 'View & Layout',
  'selora.showOutput': 'View & Layout',
  'selora.showTerminal': 'View & Layout',
  'selora.openCommandPalette': 'View & Layout',
  'selora.showKeyboardShortcuts': 'Help',
  'selora.about': 'Help',
  'selora.saveLayout': 'View & Layout',
  'selora.fileOpenFolder': 'File Actions',
  'selora.fileSave': 'File Actions',
  'selora.fileCloseEditor': 'File Actions',
  'selora.fileCloseWindow': 'File Actions',
  'selora.editFindInFiles': 'File Actions',
  'selora.editCommandPalette': 'View & Layout',
  'selora.helpDocumentation': 'Help',
};

export function commandCategory(id: CommandId): CommandCategory {
  return COMMAND_CATEGORIES[id];
}

/** The palette's entries: every command except the chord that opens it (§7.2). */
export function paletteCommands(commands: CommandSpec[]): CommandSpec[] {
  return commands.filter((command) => command.action.kind !== 'open-palette');
}

function rebindableKeybind(stored: string, fallback: string, label: string): Keybind {
  const parsed = parseKeybind(stored);
  if (parsed) return parsed;
  console.warn(`Selora: settings keybind "${stored}" for ${label} is unparseable; using ${fallback}`);
  return parseKeybind(fallback)!;
}

/**
 * Resolve the keymap: fixed bindings plus the rebindable pair from
 * `bootstrap.settings` (ARCHITECTURE §5.1/§7.1). Insertion order follows the
 * registry, so a user rebind that collides with a fixed chord resolves to the
 * first registry entry at dispatch time (the fixed table wins — §7.1's
 * no-conflict rule preserved).
 */
export function resolveKeymap(settings: SettingsView | null): Map<CommandId, Keybind> {
  const map = new Map<CommandId, Keybind>();
  const palette = rebindableKeybind(
    settings?.keybind_command_palette ?? DEFAULT_COMMAND_PALETTE,
    DEFAULT_COMMAND_PALETTE,
    'command_palette',
  );
  const toggleChat = rebindableKeybind(
    settings?.keybind_toggle_chat ?? DEFAULT_TOGGLE_CHAT,
    DEFAULT_TOGGLE_CHAT,
    'toggle_chat',
  );
  const fuzzyFind = rebindableKeybind(
    settings?.keybind_fuzzy_find ?? DEFAULT_FUZZY_FIND,
    DEFAULT_FUZZY_FIND,
    'fuzzy_find',
  );
  const save = rebindableKeybind(
    settings?.keybind_save ?? DEFAULT_SAVE,
    DEFAULT_SAVE,
    'save',
  );
  for (const entry of COMMANDS) {
    let keybind: Keybind | null = null;
    if (entry.rebindable === 'command_palette') {
      keybind = palette;
    } else if (entry.rebindable === 'toggle_chat') {
      keybind = toggleChat;
    } else if (entry.rebindable === 'fuzzy_find') {
      keybind = fuzzyFind;
    } else if (entry.rebindable === 'save') {
      keybind = save;
    } else if (entry.fixedBinding) {
      keybind = parseKeybind(entry.fixedBinding);
    }
    if (keybind) map.set(entry.id, keybind);
  }
  return map;
}

/**
 * Build the display list with shortcut labels — the rebindable pair
 * canonicalized so "ctrl + l" displays as "Ctrl+L".
 */
export function buildCommandList(settings: SettingsView | null): CommandSpec[] {
  const keymap = resolveKeymap(settings);
  return COMMANDS.map((entry) => ({
    id: entry.id,
    title: entry.title,
    shortcut: keymap.has(entry.id) ? canonicalKeybind(keymap.get(entry.id)!) : null,
    action: entry.action,
  }));
}

/** Fuzzy-filter the palette list (UI_SPEC §5: fuzzy, no regex). */
export function filterCommands(commands: CommandSpec[], query: string): CommandSpec[] {
  const q = query.trim().toLowerCase();
  if (!q) return commands;
  const scored: Array<{ command: CommandSpec; index: number; score: number }> = [];
  commands.forEach((command, index) => {
    const score = scoreTitle(command.title.toLowerCase(), q);
    if (score > 0) scored.push({ command, index, score });
  });
  return scored
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((entry) => entry.command);
}

// --- Execution ---------------------------------------------------------------

export type CommandDeps = {
  layout: () => Layout | null;
  toggle: (surface: SurfaceId) => void;
  saveLayout: () => void;
  openPalette: () => void;
  /** Z5c: open the keyboard cheat sheet (falls back to its window event). */
  openCheatsheet?: () => void;
  /** Z5c: show About (falls back to dispatching `selora:about`). */
  showAbout?: () => void;
  /** File menu: the shell owns the picker and workspace command. */
  openFolder?: () => void;
  saveActive?: () => void;
  closeActive?: () => void;
  closeWindow?: () => void;
  /** Help → Documentation; the shell validates the allowlisted URL. */
  openDocumentation?: () => void;
};

/**
 * Execute a command by id. The id→action mapping lives here (the registry),
 * never in a component. Region toggles deliberately route through the
 * protocol's toggle semantics (§2.2): Ctrl+B on an open sidebar showing Files
 * closes it; on a closed sidebar it opens the last active view.
 */
export function runCommandAction(id: CommandId, deps: CommandDeps): void {
  const entry = COMMANDS.find((command) => command.id === id);
  if (!entry) return;
  const action = entry.action;
  switch (action.kind) {
    case 'open-palette':
      deps.openPalette();
      return;
    case 'open-cheatsheet':
      if (deps.openCheatsheet) deps.openCheatsheet();
      else window.dispatchEvent(new CustomEvent('selora:open-cheatsheet'));
      return;
    case 'about':
      if (deps.showAbout) deps.showAbout();
      else window.dispatchEvent(new CustomEvent('selora:about'));
      return;
    case 'save-layout':
      deps.saveLayout();
      return;
    case 'open-folder':
      deps.openFolder?.();
      return;
    case 'save-active':
      deps.saveActive?.();
      return;
    case 'close-active':
      deps.closeActive?.();
      return;
    case 'close-window':
      deps.closeWindow?.();
      return;
    case 'open-documentation':
      deps.openDocumentation?.();
      return;
    case 'toggle-surface':
      deps.toggle(action.surface);
      return;
    case 'toggle-activity-view': {
      const layout = deps.layout();
      if (!layout) return;
      deps.toggle(layout.activity_view);
      return;
    }
    case 'toggle-bottom-view': {
      const layout = deps.layout();
      if (!layout) return;
      deps.toggle(layout.bottom_view);
      return;
    }
  }
}

function scoreTitle(title: string, q: string): number {
  const direct = title.indexOf(q);
  if (direct === 0) return 100;
  if (direct > 0) return title[direct - 1] === ' ' ? 80 : 60;
  // Subsequence: every query character in order.
  let cursor = 0;
  for (const ch of q) {
    const found = title.indexOf(ch, cursor);
    if (found === -1) return 0;
    cursor = found + 1;
  }
  return 40;
}
