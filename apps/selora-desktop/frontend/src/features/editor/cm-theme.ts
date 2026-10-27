// Selora IDE — Phase Z4 CodeMirror theme.
//
// Every color is read from the design-token layer at runtime (`tokens.css`),
// with the same values mirrored in a fallback map for tests and for the first
// paint before the stylesheet resolves. Syntax colors use the existing
// monochrome `--syn-*` ramp; this file introduces no palette of its own.
import type { Extension } from '@codemirror/state';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { EditorView } from '@codemirror/view';
import { tags } from '@lezer/highlight';

export const TOKEN_FALLBACKS: Readonly<Record<string, string>> = {
  '--ink-canvas': '#000000',
  '--ink-faint': '#0a0a0a',
  '--ink-low': '#141414',
  '--ink-mid': '#1f1f1f',
  '--ink-hi': '#2a2a2a',
  '--ink-rule': '#3a3a3a',
  '--fg-default': '#e5e5e5',
  '--fg-muted': '#8a8a8a',
  '--fg-faint': '#5a5a5a',
  '--fg-disabled': '#3a3a3a',
  '--syn-default': '#e5e5e5',
  '--syn-keyword': '#cfcfcf',
  '--syn-fn': '#ffffff',
  '--syn-type': '#b8b8b8',
  '--syn-string': '#9a9a9a',
  '--syn-number': '#d0d0d0',
  '--syn-comment': '#5a5a5a',
  '--syn-punct': '#8a8a8a',
  '--syn-var': '#e5e5e5',
  '--syn-attr': '#a8a8a8',
  '--err': '#dc2626',
  '--hairline-faint': 'rgba(255, 255, 255, 0.07)',
  '--hairline-active': 'rgba(255, 255, 255, 0.16)',
  '--glow-accent': 'rgba(255, 255, 255, 0.08)',
  '--surface-hover': '#1d1d27',
};

export function tokenValue(name: string): string {
  const fallback = TOKEN_FALLBACKS[name] ?? '';
  if (typeof document === 'undefined') return fallback;
  const computed = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return computed || fallback;
}

export function createSeloraTheme(): Extension {
  const inkCanvas = tokenValue('--ink-canvas');
  const inkFaint = tokenValue('--ink-faint');
  const inkLow = tokenValue('--ink-low');
  const inkMid = tokenValue('--ink-mid');
  const inkHi = tokenValue('--ink-hi');
  const inkRule = tokenValue('--ink-rule');
  const fgDefault = tokenValue('--fg-default');
  const fgMuted = tokenValue('--fg-muted');
  const fgFaint = tokenValue('--fg-faint');
  const synDefault = tokenValue('--syn-default');
  const synKeyword = tokenValue('--syn-keyword');
  const synFn = tokenValue('--syn-fn');
  const synType = tokenValue('--syn-type');
  const synString = tokenValue('--syn-string');
  const synNumber = tokenValue('--syn-number');
  const synComment = tokenValue('--syn-comment');
  const synPunct = tokenValue('--syn-punct');
  const synAttr = tokenValue('--syn-attr');
  const synVar = tokenValue('--syn-var');
  const err = tokenValue('--err');
  const hairlineFaint = tokenValue('--hairline-faint');
  const hairlineActive = tokenValue('--hairline-active');
  const glowAccent = tokenValue('--glow-accent');

  const baseTheme = EditorView.theme(
    {
      '&': {
        height: '100%',
        minWidth: '0',
        backgroundColor: inkCanvas,
        color: fgDefault,
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
        caretColor: fgDefault,
        padding: '12px 0 60px',
      },
      '.cm-line': {
        paddingLeft: '8px',
      },
      '.cm-gutters': {
        backgroundColor: inkCanvas,
        color: fgFaint,
        border: 'none',
        borderRight: `1px solid ${inkHi}`,
      },
      '.cm-lineNumbers .cm-gutterElement': {
        padding: '0 12px 0 8px',
        minWidth: '42px',
      },
      '.cm-activeLine': {
        backgroundColor: hairlineFaint,
      },
      '.cm-activeLineGutter': {
        backgroundColor: 'transparent',
        color: fgDefault,
      },
      '.cm-foldGutter .cm-gutterElement': {
        color: fgFaint,
        padding: '0 2px',
      },
      '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': {
        backgroundColor: hairlineActive,
      },
      '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground': {
        backgroundColor: hairlineActive,
      },
      '.cm-cursor, .cm-dropCursor': {
        borderLeftColor: fgDefault,
        borderLeftWidth: '2px',
      },
      '.cm-panels': {
        backgroundColor: inkLow,
        color: fgDefault,
        borderTop: `1px solid ${inkHi}`,
      },
      '.cm-panels.cm-panels-top': {
        borderTop: 'none',
        borderBottom: `1px solid ${inkHi}`,
      },
      '.cm-panel.cm-search': {
        fontFamily: 'var(--font-sans)',
        fontSize: 'var(--type-sm)',
        padding: '6px 8px',
      },
      '.cm-panel.cm-search input, .cm-panel.cm-search button, .cm-panel.cm-search label': {
        fontFamily: 'var(--font-sans)',
        fontSize: 'var(--type-sm)',
        color: fgDefault,
      },
      '.cm-textfield': {
        backgroundColor: inkMid,
        border: `1px solid ${inkHi}`,
        borderRadius: 'var(--radius-1)',
        color: fgDefault,
        padding: '3px 6px',
      },
      '.cm-button': {
        backgroundColor: inkMid,
        backgroundImage: 'none',
        border: `1px solid ${inkHi}`,
        borderRadius: 'var(--radius-1)',
        color: fgDefault,
        padding: '3px 8px',
      },
      '.cm-button:hover': {
        backgroundColor: inkHi,
      },
      '.cm-searchMatch': {
        backgroundColor: glowAccent,
        outline: `1px solid ${inkRule}`,
      },
      '.cm-searchMatch.cm-searchMatch-selected': {
        backgroundColor: hairlineActive,
        outline: `1px solid ${fgFaint}`,
      },
      '.cm-tooltip': {
        backgroundColor: inkMid,
        border: `1px solid ${inkHi}`,
        borderRadius: 'var(--radius-2)',
        color: fgDefault,
      },
      '.cm-tooltip-autocomplete > ul': {
        fontFamily: 'var(--font-mono)',
      },
      '.cm-tooltip-autocomplete > ul > li[aria-selected]': {
        backgroundColor: inkHi,
        color: fgDefault,
      },
      '.cm-foldPlaceholder': {
        backgroundColor: inkMid,
        border: `1px solid ${inkHi}`,
        color: fgMuted,
      },
      '.cm-matchingBracket, &.cm-focused .cm-matchingBracket': {
        backgroundColor: hairlineActive,
        outline: `1px solid ${inkRule}`,
      },
      '.cm-nonmatchingBracket': {
        color: err,
      },
      '.cm-placeholder': {
        color: fgFaint,
      },
      '.cm-gutterElement': {
        fontFamily: 'var(--font-mono)',
      },
      '.cm-scroller::-webkit-scrollbar': {
        width: '10px',
        height: '10px',
      },
      '.cm-scroller::-webkit-scrollbar-track': {
        background: inkFaint,
      },
      '.cm-scroller::-webkit-scrollbar-thumb': {
        background: inkHi,
        border: `2px solid ${inkCanvas}`,
      },
    },
    { dark: true },
  );

  const highlightStyle = HighlightStyle.define([
    {
      tag: [tags.keyword, tags.modifier, tags.controlKeyword, tags.operatorKeyword, tags.definitionKeyword],
      color: synKeyword,
      fontWeight: '500',
    },
    {
      tag: [tags.function(tags.variableName), tags.function(tags.propertyName), tags.macroName],
      color: synFn,
      fontWeight: '500',
    },
    {
      tag: [tags.typeName, tags.className, tags.namespace, tags.standard(tags.name)],
      color: synType,
    },
    {
      tag: [tags.string, tags.special(tags.string), tags.regexp, tags.character, tags.escape],
      color: synString,
    },
    {
      tag: [tags.number, tags.bool, tags.null, tags.integer, tags.float],
      color: synNumber,
    },
    {
      tag: [tags.comment, tags.lineComment, tags.blockComment, tags.docComment],
      color: synComment,
      fontStyle: 'italic',
    },
    {
      tag: [tags.operator, tags.punctuation, tags.bracket, tags.separator],
      color: synPunct,
    },
    {
      tag: [tags.attributeName, tags.annotation, tags.meta],
      color: synAttr,
    },
    {
      tag: tags.variableName,
      color: synVar,
    },
    {
      tag: tags.propertyName,
      color: synDefault,
    },
    {
      tag: tags.heading,
      color: synKeyword,
      fontWeight: '600',
    },
    {
      tag: tags.link,
      color: synString,
      textDecoration: 'underline',
    },
    {
      tag: tags.invalid,
      color: err,
      textDecoration: 'underline wavy',
    },
  ]);

  return [baseTheme, syntaxHighlighting(highlightStyle)];
}
