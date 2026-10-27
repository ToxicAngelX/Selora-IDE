// Appearance runtime tests: theme resolution, switching, persistence of the
// stored spelling, and reduced-motion independence.
import { describe, expect, it } from 'vitest';
import {
  DEFAULT_THEME,
  THEME_OPTIONS,
  THEME_VALUES,
  applyAppearance,
  applyReducedMotion,
  applyTheme,
  isThemeValue,
  normalizeThemeValue,
  resolveTheme,
} from './theme';

function root(): HTMLElement {
  const element = document.createElement('div');
  document.body.append(element);
  return element;
}

describe('theme resolution', () => {
  it('accepts exactly the values Rust persists', () => {
    expect([...THEME_VALUES]).toEqual(['selora-dark', 'dark', 'space-black', 'light']);
    expect(THEME_OPTIONS.map((option) => option.value)).toEqual(['selora-dark', 'dark', 'light']);
    expect(THEME_OPTIONS.some((option) => /OLED/.test(option.label))).toBe(false);
    for (const value of THEME_VALUES) expect(isThemeValue(value)).toBe(true);
    expect(isThemeValue('not-a-theme')).toBe(false);
  });

  it('maps both dark aliases and space-black to the dark palette', () => {
    expect(resolveTheme('selora-dark')).toBe('dark');
    expect(resolveTheme('dark')).toBe('dark');
    expect(resolveTheme('space-black')).toBe('dark');
    expect(resolveTheme('light')).toBe('light');
    expect(resolveTheme(undefined)).toBe('dark');
  });

  it('keeps an unknown stored value out of the select and falls back safely', () => {
    expect(normalizeThemeValue('light')).toBe('light');
    expect(normalizeThemeValue('unknown')).toBe(DEFAULT_THEME);
    expect(normalizeThemeValue(null)).toBe(DEFAULT_THEME);
  });
});

describe('theme switching', () => {
  it('writes data-theme and the selected palette on the root', () => {
    const element = root();
    expect(applyTheme('light', element)).toBe('light');
    expect(element.dataset.theme).toBe('light');
    expect(applyTheme('space-black', element)).toBe('dark');
    expect(element.dataset.theme).toBe('dark');
    expect(applyTheme('selora-dark', element)).toBe('dark');
  });

  it('applies theme and reduced motion together without coupling them', () => {
    const element = root();

    applyAppearance({ theme: 'light', reduced_motion: true }, element);
    expect(element.dataset.theme).toBe('light');
    expect(element.dataset.reducedMotion).toBe('true');

    // Switching the palette must not clear the motion preference.
    applyTheme('dark', element);
    expect(element.dataset.theme).toBe('dark');
    expect(element.dataset.reducedMotion).toBe('true');

    applyReducedMotion(false, element);
    expect(element.dataset.reducedMotion).toBeUndefined();
    expect(element.dataset.theme).toBe('dark');

    applyAppearance({ theme: 'light', reduced_motion: false }, element);
    expect(element.dataset.theme).toBe('light');
    expect(element.dataset.reducedMotion).toBeUndefined();
  });

  it('defaults the document root to dark before any bootstrap lands', () => {
    // The app never writes data-theme until Rust reports settings; the CSS
    // base palette is the dark ramp, so a root without the attribute is dark.
    expect(resolveTheme(undefined)).toBe('dark');
  });
});
