// Theme contract tests.
//
// Vitest runs with `css: false`, so these tests read the stylesheets as text
// (Vite's `?raw` import) and enforce the same rules the CSS layer promises:
// a real light palette for every surface token, WCAG contrast, no hardcoded
// component colors, no filter-based theme, and unchanged motion tokens.
import { describe, expect, it } from 'vitest';

// @ts-ignore -- Node built-ins are available in Vitest's node runtime.
import { readdirSync, readFileSync } from 'node:fs';
// @ts-ignore -- Node built-ins are available in Vitest's node runtime.
import { join, relative } from 'node:path';

type SourceFile = { path: string; text: string };

function walk(root: string, extension: string): SourceFile[] {
  const files: SourceFile[] = [];
  const visit = (directory: string): void => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) visit(path);
      else if (entry.name.endsWith(extension)) {
        files.push({ path: relative(root, path).replaceAll('\\', '/'), text: readFileSync(path, 'utf8') });
      }
    }
  };
  visit(root);
  return files;
}

const SRC = join((globalThis as unknown as { process: { cwd(): string } }).process.cwd(), 'src');
const styleFiles = walk(SRC, '.css');
const tsxFiles = walk(SRC, '.tsx').filter((file) => !file.path.endsWith('.test.tsx'));

const style = (path: string): string => {
  const file = styleFiles.find((candidate) => candidate.path === path);
  if (!file) throw new Error(`missing stylesheet ${path}`);
  return file.text;
};

const TOKENS = style('styles/tokens.css');
const PALETTE = style('styles/palette.css');

function block(css: string, selector: string): string {
  const index = css.indexOf(selector);
  if (index < 0) throw new Error(`missing selector ${selector}`);
  const start = css.indexOf('{', index);
  let depth = 0;
  for (let i = start; i < css.length; i += 1) {
    if (css[i] === '{') depth += 1;
    else if (css[i] === '}') {
      depth -= 1;
      if (depth === 0) return css.slice(start + 1, i);
    }
  }
  throw new Error(`unbalanced block ${selector}`);
}

function declarations(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const match of text.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    out[match[1]] = match[2].trim();
  }
  return out;
}

const darkPalette = { ...declarations(block(TOKENS, ':root {')), ...declarations(block(PALETTE, ':root {')) };
const lightPalette = {
  ...darkPalette,
  ...declarations(block(PALETTE, ':root[data-theme="light"]')),
};

function rgb(value: string): [number, number, number] {
  const text = value.trim();
  if (text.startsWith('#')) {
    let hex = text.slice(1);
    if (hex.length === 3) hex = hex.split('').map((part) => part + part).join('');
    return [
      Number.parseInt(hex.slice(0, 2), 16),
      Number.parseInt(hex.slice(2, 4), 16),
      Number.parseInt(hex.slice(4, 6), 16),
    ];
  }
  const match = text.match(/rgba?\(([^)]+)\)/);
  if (!match) throw new Error(`unsupported color ${value}`);
  const parts = match[1].split(/[\s,/]+/).filter(Boolean).map(Number);
  return [parts[0], parts[1], parts[2]];
}

function luminance(value: string): number {
  const [r, g, b] = rgb(value).map((channel) => {
    const c = channel / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(foreground: string, background: string): number {
  const a = luminance(foreground);
  const b = luminance(background);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

describe('light theme palette', () => {
  it('defines a light value for every revamp surface token', () => {
    const surfaceTokens = Object.keys(darkPalette).filter((name) => name.startsWith('--c-'));
    expect(surfaceTokens.length).toBeGreaterThan(100);
    const missing = surfaceTokens.filter((name) => !(name in lightPalette));
    expect(missing).toEqual([]);
  });

  it('replaces every core color token in the light theme', () => {
    for (const name of [
      '--ink-canvas',
      '--ink-faint',
      '--ink-low',
      '--ink-mid',
      '--ink-hi',
      '--ink-rule',
      '--fg-default',
      '--fg-muted',
      '--fg-faint',
      '--fg-disabled',
      '--fg-bright',
      '--ok',
      '--warn',
      '--err',
      '--info',
      '--syn-default',
      '--syn-keyword',
      '--syn-fn',
      '--syn-comment',
      '--scrim',
      '--surface-1',
      '--glass-bg',
      '--hairline-faint',
    ]) {
      expect(lightPalette[name], name).toBeDefined();
      expect(lightPalette[name], name).not.toBe(darkPalette[name]);
    }
  });

  it('keeps motion tokens identical across themes', () => {
    for (const name of ['--dur-instant', '--dur-quick', '--dur-base', '--dur-loop', '--dur-long', '--dur-lap', '--motion-loops']) {
      expect(lightPalette[name], name).toBe(darkPalette[name]);
    }
  });
});

describe('contrast (WCAG AA)', () => {
  const pairs: Array<[string, string, number]> = [
    ['--fg-default', '--ink-canvas', 4.5],
    ['--fg-default', '--ink-low', 4.5],
    ['--fg-default', '--ink-mid', 4.5],
    ['--fg-muted', '--ink-low', 4.5],
    ['--fg-bright', '--ink-canvas', 4.5],
    ['--syn-default', '--ink-canvas', 4.5],
    ['--fg-faint', '--ink-canvas', 3],
    ['--syn-comment', '--ink-canvas', 3],
    ['--ok', '--ink-canvas', 3],
    ['--warn', '--ink-canvas', 3],
    ['--err', '--ink-canvas', 3],
    ['--info', '--ink-canvas', 3],
  ];

  for (const [theme, palette] of [
    ['dark', darkPalette],
    ['light', lightPalette],
  ] as const) {
    it(`${theme} theme text and status colors stay readable`, () => {
      for (const [foreground, background, minimum] of pairs) {
        const ratio = contrast(palette[foreground], palette[background]);
        expect(ratio, `${theme}: ${foreground} on ${background}`).toBeGreaterThanOrEqual(minimum);
      }
    });
  }
});

describe('theme implementation', () => {
  it('has no hardcoded colors in component stylesheets', () => {
    const literal = /#[0-9a-f]{3,8}\b|rgba?\(|\bhsla?\(/i;
    for (const file of styleFiles) {
      if (file.path === 'styles/tokens.css' || file.path === 'styles/palette.css') continue;
      expect(file.text, file.path).not.toMatch(literal);
    }
  });

  it('has no hardcoded colors in component TSX inline styles', () => {
    const literal = /#[0-9a-f]{3,8}\b|rgba?\(|\bhsla?\(/i;
    for (const file of tsxFiles) {
      expect(file.text, file.path).not.toMatch(literal);
    }
  });

  it('implements the light theme by token remap, never a CSS filter', () => {
    const filters = [...(PALETTE.match(/filter\s*:[^;]+;/g) ?? [])].join(' ');
    expect(filters).not.toMatch(/invert|hue-rotate|grayscale|sepia/i);
    expect(PALETTE).toContain(':root[data-theme="light"]');
    expect(style('styles/base.css')).toContain('@import "./palette.css"');
  });

  it('honors both reduced-motion entry points and freezes loops only', () => {
    expect(TOKENS).toMatch(/@media \(prefers-reduced-motion: reduce\)[\s\S]*?--motion-loops:\s*0;/);
    expect(TOKENS).toMatch(/:root\[data-reduced-motion="true"\][\s\S]*?--motion-loops:\s*0;/);
    const base = style('styles/base.css');
    expect(base).toMatch(/@media \(prefers-reduced-motion: reduce\)[\s\S]*?animation-iteration-count:\s*1 !important/);
    expect(base).toMatch(/:root\[data-reduced-motion="true"\][\s\S]*?animation-iteration-count:\s*1 !important/);
  });
});