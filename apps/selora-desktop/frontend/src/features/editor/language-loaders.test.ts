import { describe, expect, it, vi } from 'vitest';

const { markdown } = vi.hoisted(() => ({
  markdown: vi.fn(() => []),
}));

vi.mock('@codemirror/lang-markdown', () => ({ markdown }));

import { languageFor, loadLanguage } from './language-loaders';

describe('languageFor', () => {
  it.each([
    ['src/main.rs', 'rust'],
    ['src/app.js', 'javascript'],
    ['src/app.jsx', 'jsx'],
    ['src/app.ts', 'typescript'],
    ['src/app.tsx', 'tsx'],
    ['styles/site.css', 'css'],
    ['config/settings.json', 'json'],
    ['docs/README.md', 'markdown'],
    ['docs/GUIDE.MARKDOWN', 'markdown'],
    ['src/main.py', 'plain'],
    ['LICENSE', 'plain'],
  ] as const)('maps %s to %s', (path, expected) => {
    expect(languageFor(path)).toBe(expected);
  });
});

describe('loadLanguage', () => {
  it('loads markdown for a .md buffer and leaves unknown extensions plain', async () => {
    markdown.mockClear();

    const markdownLanguage = languageFor('C:\\workspace\\docs\\README.md');
    expect(markdownLanguage).toBe('markdown');
    await loadLanguage(markdownLanguage);
    expect(markdown).toHaveBeenCalledTimes(1);

    const plainLanguage = languageFor('C:\\workspace\\LICENSE');
    expect(plainLanguage).toBe('plain');
    expect(await loadLanguage(plainLanguage)).toEqual([]);
    expect(markdown).toHaveBeenCalledTimes(1);
  });
});
