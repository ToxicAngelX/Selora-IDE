// Selora IDE — Phase Z2 command registry.
//
// The canonical keyboard + palette command set (ARCHITECTURE §7.1). The
// palette lists exactly the eleven commands (openCommandPalette is the
// chord that opens the palette, not an entry inside it). The rebindable
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
  | 'selora.saveLayout';

export type CommandAction =
  | { kind: 'toggle-surface'; surface: SurfaceId }
  /** Ctrl+B: ToggleSurface{surface: current activity_view} (ARCHITECTURE §7.1). */
  | { kind: 'toggle-activity-view' }
  /** Ctrl+J: ToggleSurface{surface: current bottom_view}. */
  | { kind: 'toggle-bottom-view' }
  | { kind: 'save-layout' }
  | { kind: 'open-palette' };

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

type CommandEntry = {
  id: CommandId;
  title: string;
  action: CommandAction;
  /** null = no shortcut (the palette-only commands). */
  fixedBinding: string | null;
  rebindable: 'command_palette' | 'toggle_chat' | null;
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
  { id: 'selora.saveLayout', title: 'Save Layout', action: { kind: 'save-layout' }, fixedBinding: null, rebindable: null },
];

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
  for (const entry of COMMANDS) {
    let keybind: Keybind | null = null;
    if (entry.rebindable === 'command_palette') {
      keybind = palette;
    } else if (entry.rebindable === 'toggle_chat') {
      keybind = toggleChat;
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
    case 'save-layout':
      deps.saveLayout();
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
