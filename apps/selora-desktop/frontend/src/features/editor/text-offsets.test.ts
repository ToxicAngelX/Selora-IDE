import { describe, expect, it } from 'vitest';
import { offsetToPosition, positionToOffset } from './text-offsets';

describe('text offsets', () => {
  it('maps offsets across LF lines', () => {
    const text = 'alpha\nbeta\ngamma';
    expect(offsetToPosition(text, 0)).toEqual({ line: 0, utf16_column: 0 });
    expect(offsetToPosition(text, 5)).toEqual({ line: 0, utf16_column: 5 });
    expect(offsetToPosition(text, 6)).toEqual({ line: 1, utf16_column: 0 });
    expect(offsetToPosition(text, 10)).toEqual({ line: 1, utf16_column: 4 });
    expect(offsetToPosition(text, 11)).toEqual({ line: 2, utf16_column: 0 });
    expect(positionToOffset(text, { line: 2, utf16_column: 0 })).toBe(11);
  });

  it('treats CRLF as one line break', () => {
    const text = 'one\r\ntwo\r\nthree';
    expect(offsetToPosition(text, 5)).toEqual({ line: 1, utf16_column: 0 });
    expect(offsetToPosition(text, 10)).toEqual({ line: 2, utf16_column: 0 });
    expect(positionToOffset(text, { line: 1, utf16_column: 0 })).toBe(5);
    expect(positionToOffset(text, { line: 1, utf16_column: 3 })).toBe(8);
    expect(positionToOffset(text, { line: 2, utf16_column: 0 })).toBe(10);
  });

  it('counts UTF-16 columns, not code points, for astral characters', () => {
    const text = 'a😀b\nnext';
    expect(offsetToPosition(text, 1)).toEqual({ line: 0, utf16_column: 1 });
    expect(offsetToPosition(text, 3)).toEqual({ line: 0, utf16_column: 3 });
    expect(offsetToPosition(text, 4)).toEqual({ line: 0, utf16_column: 4 });
    expect(offsetToPosition(text, 5)).toEqual({ line: 1, utf16_column: 0 });
    expect(positionToOffset(text, { line: 0, utf16_column: 3 })).toBe(3);
    expect(positionToOffset(text, { line: 1, utf16_column: 0 })).toBe(5);
  });

  it('clamps offsets and positions to the document bounds', () => {
    const text = 'ab\ncd';
    expect(offsetToPosition(text, -4)).toEqual({ line: 0, utf16_column: 0 });
    expect(offsetToPosition(text, 99)).toEqual({ line: 1, utf16_column: 2 });
    expect(positionToOffset(text, { line: 99, utf16_column: 0 })).toBe(text.length);
    expect(positionToOffset(text, { line: 1, utf16_column: 99 })).toBe(text.length);
  });
});
