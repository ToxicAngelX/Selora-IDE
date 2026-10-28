// Selora IDE — xterm theme derived from the live design tokens.
//
// The terminal is a monochrome surface: chrome and ANSI slots read the same
// CSS custom properties as the rest of the shell, so `data-theme="light"`
// swaps the terminal palette with no second hardcoded light/dark map. The
// literal fallbacks only cover a document without the token sheet (tests,
// first paint before CSS). Saturated color stays on status semantics.
import type { ITheme } from '@xterm/xterm';

const DARK_FALLBACK = {
  background: '#000000',
  foreground: '#e5e5e5',
  muted: '#8a8a8a',
  faint: '#5a5a5a',
  cursor: '#ffffff',
  selection: '#2a2a2a',
  ok: '#22c55e',
  warn: '#d4a017',
  err: '#dc2626',
  info: '#6b7280',
} as const;

const LIGHT_FALLBACK = {
  background: '#f7f8fa',
  foreground: '#171a20',
  muted: '#4b5563',
  faint: '#5c6675',
  cursor: '#0b0d10',
  selection: '#cfd9e6',
  ok: '#15803d',
  warn: '#8a5300',
  err: '#b91c1c',
  info: '#4b5563',
} as const;

function token(styles: CSSStyleDeclaration, name: string, fallback: string): string {
  const value = styles.getPropertyValue(name).trim();
  return value === '' ? fallback : value;
}

/** True when the shell is currently on the light palette. */
export function isLightTerminalTheme(root: HTMLElement = document.documentElement): boolean {
  return root.dataset.theme === 'light';
}

/**
 * Build the xterm ITheme from the current tokens. ANSI slots use the mono
 * syntax ramp; red/green/yellow keep their status meaning because they carry
 * diagnostics and process failure in real shell output.
 */
export function readTerminalTheme(root: HTMLElement = document.documentElement): ITheme {
  const light = isLightTerminalTheme(root);
  const fallback = light ? LIGHT_FALLBACK : DARK_FALLBACK;
  const styles = getComputedStyle(root);
  const foreground = token(styles, '--fg-default', fallback.foreground);
  const muted = token(styles, '--fg-muted', fallback.muted);
  const faint = token(styles, '--fg-faint', fallback.faint);
  const ok = token(styles, '--ok', fallback.ok);
  const warn = token(styles, '--warn', fallback.warn);
  const err = token(styles, '--err', fallback.err);
  const info = token(styles, '--info', fallback.info);
  return {
    background: token(styles, '--ink-canvas', fallback.background),
    foreground,
    cursor: token(styles, '--fg-bright', fallback.cursor),
    cursorAccent: token(styles, '--ink-canvas', fallback.background),
    selectionBackground: token(styles, '--selection-bg', fallback.selection),
    selectionInactiveBackground: token(styles, '--ink-hi', fallback.selection),
    black: token(styles, '--ink-canvas', fallback.background),
    red: err,
    green: ok,
    yellow: warn,
    blue: info,
    magenta: token(styles, '--syn-attr', muted),
    cyan: token(styles, '--syn-type', faint),
    white: foreground,
    brightBlack: faint,
    brightRed: err,
    brightGreen: ok,
    brightYellow: warn,
    brightBlue: info,
    brightMagenta: token(styles, '--syn-keyword', muted),
    brightCyan: token(styles, '--syn-string', muted),
    brightWhite: token(styles, '--fg-bright', fallback.cursor),
  };
}

/** DESIGN_TOKENS §7.4: the in-app flag is `<html data-reduced-motion="true">`. */
export function prefersReducedMotion(root: HTMLElement = document.documentElement): boolean {
  return root.dataset.reducedMotion === 'true';
}

/** Terminal font size in px from the type scale, clamped to a readable range. */
export function terminalFontSize(root: HTMLElement = document.documentElement): number {
  const raw = getComputedStyle(root).getPropertyValue('--type-sm').trim();
  const parsed = Number.parseFloat(raw);
  return Number.isFinite(parsed) && parsed >= 11 ? parsed : 12;
}

/** xterm wants a concrete family; the CSS stack is already font-mono. */
export function terminalFontFamily(root: HTMLElement = document.documentElement): string {
  return token(getComputedStyle(root), '--font-mono', 'Consolas, monospace');
}