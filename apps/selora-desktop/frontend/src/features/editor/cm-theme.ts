// Selora IDE — Phase Z4 CodeMirror theme.
//
// The editor reads the design-token layer at paint time by referencing CSS
// custom properties directly. That makes the theme a real token remap: when
// the root flips to `data-theme="light"`, CodeMirror repaints from the light
// palette with no duplicated color table and no filter.
import type { Extension } from '@codemirror/state';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { EditorView } from '@codemirror/view';
import { tags } from '@lezer/highlight';
import { resolveTheme, type ResolvedTheme } from '../theme/theme';

const token = (name: string): string => `var(${name})`;

/** The palette currently reflected on the document root. */
export function currentTheme(): ResolvedTheme {
  if (typeof document === 'undefined') return 'dark';
  return resolveTheme(document.documentElement.dataset.theme);
}

export function createSeloraTheme(theme: ResolvedTheme = currentTheme()): Extension {
  const baseTheme = EditorView.theme(
    {
      '&': {
        height: '100%',
        minWidth: '0',
        backgroundColor: token('--ink-canvas'),
        color: token('--fg-default'),
      },
      '&.cm-focused': {
        outline: 'none',
      },
      '.cm-scroller': {
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--type-md)',
        lineHeight: 'var(--type-md-lh)',
        overflow: 'auto',
      },
      '.cm-content': {
        caretColor: token('--fg-default'),
        padding: '12px 0 60px',
      },
      '.cm-line': {
        paddingLeft: '8px',
      },
      '.cm-gutters': {
        backgroundColor: token('--ink-canvas'),
        color: token('--fg-faint'),
        border: 'none',
        borderRight: `1px solid ${token('--ink-hi')}`,
      },
      '.cm-lineNumbers .cm-gutterElement': {
        padding: '0 12px 0 8px',
        minWidth: '42px',
      },
      '.cm-activeLine': {
        backgroundColor: token('--hairline-faint'),
      },
      '.cm-activeLineGutter': {
        backgroundColor: 'transparent',
        color: token('--fg-default'),
      },
      '.cm-foldGutter .cm-gutterElement': {
        color: token('--fg-faint'),
        padding: '0 2px',
      },
      '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': {
        backgroundColor: token('--hairline-active'),
      },
      '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground': {
        backgroundColor: token('--hairline-active'),
      },
      '.cm-cursor, .cm-dropCursor': {
        borderLeftColor: token('--fg-default'),
        borderLeftWidth: '2px',
      },
      '.cm-panels': {
        backgroundColor: token('--ink-low'),
        color: token('--fg-default'),
        borderTop: `1px solid ${token('--ink-hi')}`,
      },
      '.cm-panels.cm-panels-top': {
        borderTop: 'none',
        borderBottom: `1px solid ${token('--ink-hi')}`,
      },
      '.cm-panel.cm-search': {
        fontFamily: 'var(--font-sans)',
        fontSize: 'var(--type-sm)',
        padding: '6px 8px',
      },
      '.cm-panel.cm-search input, .cm-panel.cm-search button, .cm-panel.cm-search label': {
        fontFamily: 'var(--font-sans)',
        fontSize: 'var(--type-sm)',
        color: token('--fg-default'),
      },
      '.cm-textfield': {
        backgroundColor: token('--ink-mid'),
        border: `1px solid ${token('--ink-hi')}`,
        borderRadius: 'var(--radius-1)',
        color: token('--fg-default'),
        padding: '3px 6px',
      },
      '.cm-button': {
        backgroundColor: token('--ink-mid'),
        backgroundImage: 'none',
        border: `1px solid ${token('--ink-hi')}`,
        borderRadius: 'var(--radius-1)',
        color: token('--fg-default'),
        padding: '3px 8px',
      },
      '.cm-button:hover': {
        backgroundColor: token('--ink-hi'),
      },
      '.cm-searchMatch': {
        backgroundColor: token('--glow-accent'),
        outline: `1px solid ${token('--ink-rule')}`,
      },
      '.cm-searchMatch.cm-searchMatch-selected': {
        backgroundColor: token('--hairline-active'),
        outline: `1px solid ${token('--fg-faint')}`,
      },
      '.cm-tooltip': {
        backgroundColor: token('--ink-mid'),
        border: `1px solid ${token('--ink-hi')}`,
        borderRadius: 'var(--radius-2)',
        color: token('--fg-default'),
      },
      '.cm-tooltip-autocomplete > ul': {
        fontFamily: 'var(--font-mono)',
      },
      '.cm-tooltip-autocomplete > ul > li[aria-selected]': {
        backgroundColor: token('--ink-hi'),
        color: token('--fg-default'),
      },
      '.cm-foldPlaceholder': {
        backgroundColor: token('--ink-mid'),
        border: `1px solid ${token('--ink-hi')}`,
        color: token('--fg-muted'),
      },
      '.cm-matchingBracket, &.cm-focused .cm-matchingBracket': {
        backgroundColor: token('--hairline-active'),
        outline: `1px solid ${token('--ink-rule')}`,
      },
      '.cm-nonmatchingBracket': {
        color: token('--err'),
      },
      '.cm-placeholder': {
        color: token('--fg-faint'),
      },
      '.cm-gutterElement': {
        fontFamily: 'var(--font-mono)',
      },
      '.cm-scroller::-webkit-scrollbar': {
        width: '10px',
        height: '10px',
      },
      '.cm-scroller::-webkit-scrollbar-track': {
        background: token('--ink-faint'),
      },
      '.cm-scroller::-webkit-scrollbar-thumb': {
        background: token('--ink-hi'),
        border: `2px solid ${token('--ink-canvas')}`,
      },
    },
    { dark: theme === 'dark' },
  );

  const highlightStyle = HighlightStyle.define([
    {
      tag: [
        tags.keyword,
        tags.modifier,
        tags.controlKeyword,
        tags.operatorKeyword,
        tags.definitionKeyword,
      ],
      color: token('--syn-keyword'),
      fontWeight: '500',
    },
    {
      tag: [tags.function(tags.variableName), tags.function(tags.propertyName), tags.macroName],
      color: token('--syn-fn'),
      fontWeight: '500',
    },
    {
      tag: [tags.typeName, tags.className, tags.namespace, tags.standard(tags.name)],
      color: token('--syn-type'),
    },
    {
      tag: [tags.string, tags.special(tags.string), tags.regexp, tags.character, tags.escape],
      color: token('--syn-string'),
    },
    {
      tag: [tags.number, tags.bool, tags.null, tags.integer, tags.float],
      color: token('--syn-number'),
    },
    {
      tag: [tags.comment, tags.lineComment, tags.blockComment, tags.docComment],
      color: token('--syn-comment'),
      fontStyle: 'italic',
    },
    {
      tag: [tags.operator, tags.punctuation, tags.bracket, tags.separator],
      color: token('--syn-punct'),
    },
    {
      tag: [tags.attributeName, tags.annotation, tags.meta],
      color: token('--syn-attr'),
    },
    {
      tag: tags.variableName,
      color: token('--syn-var'),
    },
    {
      tag: tags.propertyName,
      color: token('--syn-default'),
    },
    {
      tag: tags.heading,
      color: token('--syn-keyword'),
      fontWeight: '600',
    },
    {
      tag: tags.link,
      color: token('--syn-string'),
      textDecoration: 'underline',
    },
    {
      tag: tags.invalid,
      color: token('--err'),
      textDecoration: 'underline wavy',
    },
  ]);

  return [baseTheme, syntaxHighlighting(highlightStyle)];
}