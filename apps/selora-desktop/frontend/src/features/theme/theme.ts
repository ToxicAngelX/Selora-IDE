// Selora IDE — appearance (theme + reduced motion) applied at runtime.
//
// Rust owns the persisted values (`ui.theme`, `ui.reduce_motion`); this module
// maps them onto the document root. A theme is a token remap, never a CSS
// filter: `data-theme="light"` swaps the palette in palette.css and
// `data-theme="dark"` keeps the default ink ramp.
export const THEME_VALUES = ['selora-dark', 'dark', 'space-black', 'light'] as const;

export type ThemeValue = (typeof THEME_VALUES)[number];
export type ResolvedTheme = 'dark' | 'light';

export const DEFAULT_THEME: ThemeValue = 'selora-dark';

export const THEME_OPTIONS: ReadonlyArray<{ value: ThemeValue; label: string }> = [
  { value: 'selora-dark', label: 'Selora Cosmic Dark' },
  { value: 'dark', label: 'Dark (alias)' },
  { value: 'light', label: 'Selora Moonlight (Light)' },
];

/** True when Rust's stored spelling is one the backend accepts. */
export function isThemeValue(value: string): value is ThemeValue {
  return (THEME_VALUES as readonly string[]).includes(value);
}

/** Keep a stored accepted value, otherwise fall back to the documented default. */
export function normalizeThemeValue(value: string | null | undefined): ThemeValue {
  return value && isThemeValue(value) ? value : DEFAULT_THEME;
}

/**
 * The two real palettes. `dark`, `selora-dark` and `space-black` are aliases
 * for the ink ramp; `light` is Selora Moonlight.
 */
export function resolveTheme(value: string | null | undefined): ResolvedTheme {
  return value === 'light' ? 'light' : 'dark';
}

/** Apply a stored theme value to the document root and return the palette it selected. */
export function applyTheme(
  value: string | null | undefined,
  root: HTMLElement = document.documentElement,
): ResolvedTheme {
  const resolved = resolveTheme(value);
  root.dataset.theme = resolved;
  return resolved;
}

/**
 * DESIGN_TOKENS §7.4: the in-app flag is `<html data-reduced-motion="true">`.
 * It only freezes loops (`--motion-loops: 0`); it never rewrites durations.
 */
export function applyReducedMotion(
  reduced: boolean,
  root: HTMLElement = document.documentElement,
): void {
  if (reduced) root.dataset.reducedMotion = 'true';
  else delete root.dataset.reducedMotion;
}

/** Apply both appearance settings from a bootstrap `SettingsView`. */
export function applyAppearance(
  settings: { theme: string; reduced_motion: boolean },
  root: HTMLElement = document.documentElement,
): ResolvedTheme {
  const resolved = applyTheme(settings.theme, root);
  applyReducedMotion(settings.reduced_motion, root);
  return resolved;
}
