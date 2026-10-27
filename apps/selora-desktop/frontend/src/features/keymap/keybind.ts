// Selora IDE — Phase Z2 keybind parsing.
//
// Mirrors `Keybind::parse` in crates/selora-settings/src/keybind.rs so a
// stored egui-side rebind parses identically in the web shell
// (ARCHITECTURE §7.1): case-insensitive, whitespace-tolerant, modifiers in
// any order, alias spellings for Ctrl/Alt, exactly one key, `null` for
// garbage. Matching is Windows-first: `ctrlKey` only, no `metaKey` mapping.
//
// A normalized key token is the lowercased DOM `e.key` spelling
// ("p", ",", "f5", "arrowup", "escape"…) restricted to the keys the Rust
// grammar knows.

export type Keybind = {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
};

const CTRL_ALIASES = new Set(['ctrl', 'control', 'cmd', 'command', 'win', 'super', 'meta']);
const ALT_ALIASES = new Set(['alt', 'opt', 'option']);
const SHIFT = 'shift';

// parse_key's key set (keybind.rs:137-210). Multi-word tokens accepted.
const KEY_ALIASES: Record<string, string> = {
  return: 'enter',
  escape: 'esc',
  arrowup: 'up',
  arrowdown: 'down',
  arrowleft: 'left',
  arrowright: 'right',
  del: 'delete',
  comma: ',',
  period: '.',
  slash: '/',
  minus: '-',
  backslash: '\\',
};
const SINGLE_KEYS = new Set([
  'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm',
  'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
  '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
  'f1', 'f2', 'f3', 'f4', 'f5', 'f6', 'f7', 'f8', 'f9', 'f10', 'f11', 'f12',
  'enter', 'esc', 'tab', 'space', 'backspace', 'delete', 'insert',
  'home', 'end', 'pageup', 'pagedown', 'up', 'down', 'left', 'right',
  ',', '.', '/', '-', '\\',
]);

function normalizeKeyToken(raw: string): string | null {
  const token = raw.trim().toLowerCase();
  if (!token) return null;
  const aliased = KEY_ALIASES[token] ?? token;
  return SINGLE_KEYS.has(aliased) ? aliased : null;
}

/**
 * Parse a keybind string such as "Ctrl+Shift+P".
 * Returns null for empty strings, unknown tokens, two keys, or a binding
 * with no key at all — the same garbage set as `Keybind::parse`.
 */
export function parseKeybind(s: string): Keybind | null {
  const keybind: Keybind = { ctrl: false, alt: false, shift: false, key: '' };
  let key: string | null = null;

  for (const rawToken of s.split('+')) {
    const token = rawToken.trim().toLowerCase();
    if (!token) return null;
    if (CTRL_ALIASES.has(token)) {
      keybind.ctrl = true;
    } else if (token === SHIFT) {
      keybind.shift = true;
    } else if (ALT_ALIASES.has(token)) {
      keybind.alt = true;
    } else {
      if (key !== null) return null; // e.g. "Ctrl+Shift+P+L"
      key = normalizeKeyToken(token);
      if (key === null) return null;
    }
  }

  if (key === null) return null; // modifiers only
  keybind.key = key;
  return keybind;
}

/** Canonical display form, e.g. "Ctrl+Shift+P" — the shape parseKeybind accepts. */
export function canonicalKeybind(keybind: Keybind): string {
  const parts: string[] = [];
  if (keybind.ctrl) parts.push('Ctrl');
  if (keybind.alt) parts.push('Alt');
  if (keybind.shift) parts.push('Shift');
  parts.push(keyLabel(keybind.key));
  return parts.join('+');
}

// key_label (keybind.rs:214-286): the display spelling of each key token.
const KEY_LABELS: Record<string, string> = {
  enter: 'Enter',
  esc: 'Esc',
  tab: 'Tab',
  space: 'Space',
  backspace: 'Backspace',
  delete: 'Delete',
  insert: 'Insert',
  home: 'Home',
  end: 'End',
  pageup: 'PageUp',
  pagedown: 'PageDown',
  up: 'Up',
  down: 'Down',
  left: 'Left',
  right: 'Right',
  ',': 'Comma',
  '.': 'Period',
  '/': 'Slash',
  '-': 'Minus',
  '\\': 'Backslash',
};

function keyLabel(key: string): string {
  if (KEY_LABELS[key]) return KEY_LABELS[key];
  return key.toUpperCase();
}

/** Normalize a DOM KeyboardEvent key into a known token, or null. */
export function normalizeEventKey(key: string): string | null {
  return normalizeKeyToken(key);
}

export type KeyboardEventLike = {
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey?: boolean;
  key: string;
};

/** Does this keybind match the given event? (keybind.rs:83-90 semantics.) */
export function keybindMatches(keybind: Keybind, event: KeyboardEventLike): boolean {
  if (!keybind.ctrl && (event.ctrlKey || event.metaKey)) return false;
  if (keybind.ctrl && !event.ctrlKey) return false;
  if (keybind.alt !== event.altKey) return false;
  if (keybind.shift !== event.shiftKey) return false;
  return normalizeEventKey(event.key) === keybind.key;
}
