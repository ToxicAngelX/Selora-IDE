// Selora IDE — Phase Z4 editor text offsets.
//
// CodeMirror, the Rust buffer model, and the protocol all address text by
// UTF-16 code units (JavaScript's native string indexing). These helpers are
// deliberately pure so the conversion can be unit-tested without a DOM or an
// EditorView. Lines are 0-based; `utf16_column` is a 0-based code-unit column.

import type { TextPosition } from '../../bridge/bridge';

/**
 * Convert an absolute string offset into a line / UTF-16 column position.
 * Handles LF and CRLF documents; the column includes a CR when the offset
 * lands between the CR and LF of a CRLF pair, preserving exact round-trips.
 */
export function offsetToPosition(text: string, offset: number): TextPosition {
  const limit = Math.max(0, Math.min(offset, text.length));
  let line = 0;
  let lineStart = 0;
  for (let index = 0; index < limit; index += 1) {
    if (text.charCodeAt(index) === 10) {
      line += 1;
      lineStart = index + 1;
    }
  }
  return { line, utf16_column: limit - lineStart };
}

/**
 * Convert a line / UTF-16 column position back into an absolute string offset.
 * Out-of-range lines clamp to the end of the document and over-long columns
 * clamp to the end of their line, matching the Rust model's forgiving reads.
 */
export function positionToOffset(text: string, position: TextPosition): number {
  const targetLine = Math.max(0, position.line);
  let line = 0;
  let lineStart = 0;

  while (line < targetLine) {
    const nextBreak = text.indexOf('\n', lineStart);
    if (nextBreak === -1) return text.length;
    line += 1;
    lineStart = nextBreak + 1;
  }

  const nextBreak = text.indexOf('\n', lineStart);
  const lineEnd = nextBreak === -1 ? text.length : nextBreak;
  const column = Math.max(0, position.utf16_column);
  return Math.min(lineStart + column, lineEnd);
}
