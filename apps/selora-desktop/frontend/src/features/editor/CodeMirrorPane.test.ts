import { describe, expect, it } from 'vitest';
import { EditorState } from '@codemirror/state';
import { computeTextChanges } from './CodeMirrorPane';

describe('computeTextChanges', () => {
  it('reports one replacement range in pre-edit coordinates', () => {
    const state = EditorState.create({ doc: 'let value = 1;\n' });
    const transaction = state.update({ changes: { from: 12, to: 13, insert: '42' } });

    expect(computeTextChanges(transaction.startState.doc, transaction.changes)).toEqual([
      {
        start: { line: 0, utf16_column: 12 },
        end: { line: 0, utf16_column: 13 },
        replacement: '42',
      },
    ]);
  });

  it('keeps multi-range edits separate and handles multi-line insertions', () => {
    const state = EditorState.create({ doc: 'alpha\nbeta\ngamma' });
    const transaction = state.update({
      changes: [
        { from: 0, to: 5, insert: 'one\ntwo' },
        { from: 11, to: 16, insert: 'omega' },
      ],
    });

    expect(computeTextChanges(transaction.startState.doc, transaction.changes)).toEqual([
      {
        start: { line: 0, utf16_column: 0 },
        end: { line: 0, utf16_column: 5 },
        replacement: 'one\ntwo',
      },
      {
        start: { line: 2, utf16_column: 0 },
        end: { line: 2, utf16_column: 5 },
        replacement: 'omega',
      },
    ]);
  });

  it('does not shift earlier ranges when an earlier edit changes length', () => {
    const state = EditorState.create({ doc: 'a😀b' });
    const transaction = state.update({ changes: { from: 1, to: 3, insert: 'x' } });

    expect(computeTextChanges(transaction.startState.doc, transaction.changes)).toEqual([
      {
        start: { line: 0, utf16_column: 1 },
        end: { line: 0, utf16_column: 3 },
        replacement: 'x',
      },
    ]);
  });
});
