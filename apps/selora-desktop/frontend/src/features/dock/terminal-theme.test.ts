import { afterEach, describe, expect, it } from 'vitest';
import {
  prefersReducedMotion,
  readTerminalTheme,
  terminalFontFamily,
  terminalFontSize,
} from './terminal-theme';

const root = document.documentElement;

afterEach(() => {
  root.removeAttribute('data-theme');
  root.removeAttribute('data-reduced-motion');
  root.removeAttribute('style');
});

describe('terminal tokens', () => {
  it('reads the dark token ramp from the document', () => {
    root.style.setProperty('--ink-canvas', '#010203');
    root.style.setProperty('--fg-default', '#0a0b0c');
    root.style.setProperty('--err', '#aa0000');
    root.style.setProperty('--type-sm', '13px');
    root.style.setProperty('--font-mono', 'Fira Code, monospace');

    const theme = readTerminalTheme(root);
    expect(theme.background).toBe('#010203');
    expect(theme.foreground).toBe('#0a0b0c');
    expect(theme.red).toBe('#aa0000');
    expect(terminalFontSize(root)).toBe(13);
    expect(terminalFontFamily(root)).toBe('Fira Code, monospace');
  });

  it('switches to the light token ramp via data-theme', () => {
    root.dataset.theme = 'light';
    root.style.setProperty('--ink-canvas', '#fafafa');
    root.style.setProperty('--fg-default', '#111111');

    const theme = readTerminalTheme(root);
    expect(theme.background).toBe('#fafafa');
    expect(theme.foreground).toBe('#111111');
  });

  it('keeps readable fallbacks when the token sheet is absent', () => {
    root.dataset.theme = 'light';
    const theme = readTerminalTheme(root);
    expect(theme.background).toBe('#f7f8fa');
    expect(theme.foreground).toBe('#171a20');
  });

  it('honours the reduced-motion flag', () => {
    expect(prefersReducedMotion(root)).toBe(false);
    root.dataset.reducedMotion = 'true';
    expect(prefersReducedMotion(root)).toBe(true);
  });
});