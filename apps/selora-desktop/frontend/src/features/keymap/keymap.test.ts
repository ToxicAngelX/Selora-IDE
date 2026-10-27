// Keymap tests (ARCHITECTURE §10): the rebindable-string parser mirrors
// Keybind::parse's grammar incl. the garbage fallback; keymap resolution
// (fixed table + rebindable pair); the palette registry invariants (every
// SurfaceId covered, ids unique, shortcut labels match §7.1, exactly
// thirteen palette entries after Z5c's help commands); palette filtering.
import { describe, expect, it, vi, afterEach } from 'vitest';
import { canonicalKeybind, keybindMatches, parseKeybind } from './keybind';
import {
  SURFACE_IDS,
  buildCommandList,
  filterCommands,
  paletteCommands,
  resolveKeymap,
  runCommandAction,
  type CommandDeps,
} from './commands';
import { makeBootstrap } from '../../testing/fixtures';

const SETTINGS = makeBootstrap().settings;

describe('parseKeybind — Keybind::parse grammar (keybind.rs:36-67)', () => {
  it('parses case-insensitively with whitespace tolerance and modifier aliases', () => {
    expect(parseKeybind(' ctrl + shift + p ')).toEqual({ ctrl: true, alt: false, shift: true, key: 'p' });
    expect(parseKeybind('CTRL+P')).toEqual(parseKeybind('Ctrl+P'));
    expect(parseKeybind('Command+P')).toEqual(parseKeybind('Ctrl+P'));
    expect(parseKeybind('Option+S')).toEqual(parseKeybind('Alt+S'));
    expect(parseKeybind('Alt+Ctrl+X')).toEqual(parseKeybind('Ctrl+Alt+X'));
  });

  it('parses modifier-less bindings and punctuation keys', () => {
    expect(parseKeybind('Tab')).toEqual({ ctrl: false, alt: false, shift: false, key: 'tab' });
    expect(parseKeybind('Ctrl+,')).toEqual({ ctrl: true, alt: false, shift: false, key: ',' });
    expect(parseKeybind('ctrl+f5')?.key).toBe('f5');
  });

  it('returns null for the garbage set', () => {
    for (const garbage of ['', '   ', 'hello world', 'Ctrl+', 'Ctrl', 'Shift', 'abc', 'Ctrl+??', 'Ctrl+Shift+P+L', 'Ctrl++P']) {
      expect(parseKeybind(garbage)).toBeNull();
    }
  });

  it('canonical display form round-trips through parse', () => {
    for (const s of ['Ctrl+Shift+P', 'Ctrl+P', 'Ctrl+L', 'Ctrl+Comma', 'F12', 'Tab']) {
      const kb = parseKeybind(s)!;
      expect(parseKeybind(canonicalKeybind(kb))).toEqual(kb);
    }
  });
});

describe('keybindMatches (Windows-first: ctrlKey, no metaKey mapping)', () => {
  it('matches its own event and rejects wrong key or modifiers', () => {
    const kb = parseKeybind('Ctrl+Shift+P')!;
    expect(keybindMatches(kb, { ctrlKey: true, altKey: false, shiftKey: true, key: 'P' })).toBe(true);
    expect(keybindMatches(kb, { ctrlKey: true, altKey: false, shiftKey: false, key: 'P' })).toBe(false);
    expect(keybindMatches(kb, { ctrlKey: false, altKey: false, shiftKey: true, key: 'P' })).toBe(false);
    expect(keybindMatches(kb, { ctrlKey: true, altKey: true, shiftKey: true, key: 'P' })).toBe(false);
    // metaKey never maps to Ctrl (§7.1).
    expect(keybindMatches(kb, { ctrlKey: false, altKey: false, shiftKey: true, metaKey: true, key: 'P' } as never)).toBe(false);
    // Case-insensitive e.key.
    expect(keybindMatches(parseKeybind('Ctrl+L')!, { ctrlKey: true, altKey: false, shiftKey: false, key: 'l' })).toBe(true);
  });
});

describe('resolveKeymap (ARCHITECTURE §7.1)', () => {
  it('resolves the canonical table from default settings', () => {
    const keymap = resolveKeymap(SETTINGS);
    const shortcutOf = (id: string) => canonicalKeybind(keymap.get(id as never)!);
    expect(shortcutOf('selora.togglePrimarySidebar')).toBe('Ctrl+B');
    expect(shortcutOf('selora.toggleAgentDock')).toBe('Ctrl+L');
    expect(shortcutOf('selora.toggleBottomDock')).toBe('Ctrl+J');
    expect(shortcutOf('selora.showFiles')).toBe('Ctrl+Shift+E');
    expect(shortcutOf('selora.showSearch')).toBe('Ctrl+Shift+F');
    expect(shortcutOf('selora.showGit')).toBe('Ctrl+Shift+G');
    expect(shortcutOf('selora.showProblems')).toBe('Ctrl+Shift+M');
    expect(shortcutOf('selora.showSettings')).toBe('Ctrl+Comma');
    expect(shortcutOf('selora.openCommandPalette')).toBe('Ctrl+Shift+P');
    expect(shortcutOf('selora.fileSave')).toBe('Ctrl+S');
    expect(shortcutOf('selora.editFindInFiles')).toBe('Ctrl+P');
    expect(keymap.has('selora.showOutput')).toBe(false);
    expect(keymap.has('selora.showTerminal')).toBe(false);
    expect(keymap.has('selora.saveLayout')).toBe(false);
  });

  it('uses a stored rebind for all four settings keybinds', () => {
    const keymap = resolveKeymap({
      ...SETTINGS,
      keybind_toggle_chat: 'Alt+Q',
      keybind_command_palette: 'F2',
      keybind_save: 'Ctrl+Alt+S',
      keybind_fuzzy_find: 'Ctrl+Shift+O',
    });
    expect(canonicalKeybind(keymap.get('selora.toggleAgentDock')!)).toBe('Alt+Q');
    expect(canonicalKeybind(keymap.get('selora.openCommandPalette')!)).toBe('F2');
    expect(canonicalKeybind(keymap.get('selora.fileSave')!)).toBe('Ctrl+Alt+S');
    expect(canonicalKeybind(keymap.get('selora.editFindInFiles')!)).toBe('Ctrl+Shift+O');
  });

  it('falls back to the default with a console warning on an unparseable stored string', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const keymap = resolveKeymap({ ...SETTINGS, keybind_toggle_chat: 'not a keybind' });
    expect(canonicalKeybind(keymap.get('selora.toggleAgentDock')!)).toBe('Ctrl+L');
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0][0])).toContain('unparseable');
    warn.mockRestore();
  });
});

describe('palette registry (ARCHITECTURE §7.2/§10)', () => {
  const commands = buildCommandList(SETTINGS);

  it('lists every palette command (the palette chord is not an entry)', () => {
    expect(paletteCommands(commands)).toHaveLength(19);
    expect(commands).toHaveLength(21);
    expect(paletteCommands(commands).map((c) => c.id)).not.toContain('selora.openCommandPalette');
  });

  it('has unique ids', () => {
    const ids = commands.map((command) => command.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('covers every SurfaceId in at least one command', () => {
    const surfaces = new Set(
      commands.flatMap((command) =>
        command.action.kind === 'toggle-surface' ? [command.action.surface] : [],
      ),
    );
    for (const surface of SURFACE_IDS) {
      expect(surfaces.has(surface)).toBe(true);
    }
  });

  it('carries the §7.1 shortcut labels on the display list', () => {
    const byId = new Map(commands.map((command) => [command.id, command]));
    expect(byId.get('selora.togglePrimarySidebar')?.shortcut).toBe('Ctrl+B');
    expect(byId.get('selora.toggleAgentDock')?.shortcut).toBe('Ctrl+L');
    expect(byId.get('selora.toggleBottomDock')?.shortcut).toBe('Ctrl+J');
    expect(byId.get('selora.showSettings')?.shortcut).toBe('Ctrl+Comma');
    expect(byId.get('selora.fileSave')?.shortcut).toBe('Ctrl+S');
    expect(byId.get('selora.editFindInFiles')?.shortcut).toBe('Ctrl+P');
    expect(byId.get('selora.showOutput')?.shortcut).toBeNull();
    expect(byId.get('selora.showTerminal')?.shortcut).toBeNull();
    expect(byId.get('selora.showKeyboardShortcuts')?.shortcut).toBe('Ctrl+Slash');
    expect(byId.get('selora.about')?.shortcut).toBeNull();
    expect(byId.get('selora.saveLayout')?.shortcut).toBeNull();
  });

  it('keeps every chord unique', () => {
    const shortcuts = commands.map((command) => command.shortcut).filter((s): s is string => s !== null);
    expect(new Set(shortcuts).size).toBe(shortcuts.length);
  });
});

describe('filterCommands (UI_SPEC §5: fuzzy, no regex)', () => {
  const commands = buildCommandList(SETTINGS);
  const palette = paletteCommands(commands);

  it('an empty query returns all commands in registry order', () => {
    expect(filterCommands(palette, '')).toEqual(palette);
    expect(filterCommands(palette, '   ')).toEqual(palette);
  });

  it('ranks a prefix match above a word-start substring above a subsequence', () => {
    // 'save' prefixes both Save commands, in registry order.
    expect(filterCommands(palette, 'save').map((command) => command.title)).toEqual([
      'Save Layout',
      'Save',
    ]);
    // 'files' hits a word start inside both "Show Files" and "Find in Files".
    expect(filterCommands(palette, 'files').map((command) => command.title)).toEqual([
      'Show Files',
      'Find in Files',
    ]);
    // 'sf' is a subsequence of only "Show Files" (tier 40).
    expect(filterCommands(palette, 'sf').map((command) => command.title)).toEqual(['Show Files']);
  });

  it('equal scores keep registry order', () => {
    expect(filterCommands(palette, 'toggle').map((command) => command.title)).toEqual([
      'Toggle Primary Sidebar',
      'Toggle Agent Dock',
      'Toggle Bottom Dock',
    ]);
  });

  it('no match yields an empty list', () => {
    expect(filterCommands(palette, 'zzzz')).toEqual([]);
  });
});

describe('runCommandAction (the registry owns the id→action mapping)', () => {
  function makeDeps() {
    const state = {
      toggled: [] as string[],
      saved: 0,
      opened: 0,
      cheatsheet: 0,
      about: 0,
      layout: { activity_view: 'git', bottom_view: 'problems' } as never,
    };
    const deps: CommandDeps = {
      layout: () => state.layout,
      toggle: (surface) => {
        state.toggled.push(surface);
      },
      saveLayout: () => {
        state.saved += 1;
      },
      openPalette: () => {
        state.opened += 1;
      },
      openCheatsheet: () => {
        state.cheatsheet += 1;
      },
      showAbout: () => {
        state.about += 1;
      },
    };
    return { deps, state };
  }

  it('Ctrl+B routes through the current activity view; Ctrl+J through the current bottom view', () => {
    const { deps, state } = makeDeps();
    runCommandAction('selora.togglePrimarySidebar', deps);
    expect(state.toggled).toEqual(['git']);
    runCommandAction('selora.toggleBottomDock', deps);
    expect(state.toggled).toEqual(['git', 'problems']);
  });

  it('fixed-surface commands dispatch their surface; save and palette route too', () => {
    const { deps, state } = makeDeps();
    runCommandAction('selora.showOutput', deps);
    runCommandAction('selora.showTerminal', deps);
    runCommandAction('selora.saveLayout', deps);
    runCommandAction('selora.openCommandPalette', deps);
    expect(state.toggled).toEqual(['output', 'terminal_placeholder']);
    expect(state.saved).toBe(1);
    expect(state.opened).toBe(1);
  });

  it('routes the Z5 help commands and falls back to their window events', () => {
    const { deps, state } = makeDeps();
    runCommandAction('selora.showKeyboardShortcuts', deps);
    runCommandAction('selora.about', deps);
    expect(state.cheatsheet).toBe(1);
    expect(state.about).toBe(1);

    const sheetEvent = vi.fn();
    const aboutEvent = vi.fn();
    window.addEventListener('selora:open-cheatsheet', sheetEvent);
    window.addEventListener('selora:about', aboutEvent);
    const bare: CommandDeps = { ...deps, openCheatsheet: undefined, showAbout: undefined };
    runCommandAction('selora.showKeyboardShortcuts', bare);
    runCommandAction('selora.about', bare);
    expect(sheetEvent).toHaveBeenCalledTimes(1);
    expect(aboutEvent).toHaveBeenCalledTimes(1);
    window.removeEventListener('selora:open-cheatsheet', sheetEvent);
    window.removeEventListener('selora:about', aboutEvent);
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});
