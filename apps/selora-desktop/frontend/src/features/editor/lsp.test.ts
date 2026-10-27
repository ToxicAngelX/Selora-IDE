import { describe, expect, it, vi } from 'vitest';
import type {
  CompletionResult,
  DefinitionResult,
  Diagnostic,
} from '../../bridge/bridge';
import {
  completionOptions,
  definitionTarget,
  toCmDiagnostics,
  watchDefinitionNavigation,
} from './lsp';

const rustPath = 'C:\\workspace\\src\\main.rs';

function diagnostic(overrides: Partial<Diagnostic> = {}): Diagnostic {
  return {
    path: rustPath,
    line: 0,
    column: 0,
    severity: 'error',
    message: 'unexpected token',
    ...overrides,
  };
}

describe('toCmDiagnostics', () => {
  it('converts CRLF lines and UTF-16 columns to document offsets', () => {
    const text = 'alpha\r\nbeta\r\ngamma';
    expect(
      toCmDiagnostics('C:/workspace/src/main.rs', [diagnostic({ line: 1, column: 1 })], text),
    ).toEqual([
      {
        from: 8,
        to: 11,
        severity: 'error',
        message: 'unexpected token',
      },
    ]);
  });

  it('keeps astral characters in UTF-16 offsets when selecting a token', () => {
    const text = 'a😀b\nnext';
    expect(toCmDiagnostics(rustPath, [diagnostic({ line: 0, column: 1 })], text)).toEqual([
      {
        from: 1,
        to: 4,
        severity: 'error',
        message: 'unexpected token',
      },
    ]);
  });

  it('clamps columns to the line and falls back to the line end', () => {
    const text = 'ab\ncd';
    expect(toCmDiagnostics(rustPath, [diagnostic({ column: 99 })], text)).toEqual([
      {
        from: 2,
        to: 2,
        severity: 'error',
        message: 'unexpected token',
      },
    ]);
    expect(toCmDiagnostics(rustPath, [diagnostic({ line: 99, column: 4 })], text)).toEqual([]);
  });

  it('drops diagnostics for other paths and preserves severities', () => {
    const text = 'let value = 1;';
    const diagnostics = [
      diagnostic({ path: 'C:/workspace/src/lib.rs' }),
      diagnostic({ column: 4, severity: 'warning', message: 'unused variable' }),
    ];

    expect(toCmDiagnostics(rustPath, diagnostics, text)).toEqual([
      {
        from: 4,
        to: 9,
        severity: 'warning',
        message: 'unused variable',
      },
    ]);
  });
});

describe('completionOptions', () => {
  it('maps insert_text to apply, keeps details, and maps kinds', () => {
    const result: CompletionResult = {
      buffer_id: 'b1',
      items: [
        {
          label: 'println',
          detail: 'macro',
          insert_text: 'println!()',
          kind: 'function',
        },
        {
          label: 'value',
          detail: null,
          insert_text: null,
          kind: 'variable',
        },
      ],
    };

    expect(completionOptions(result)).toEqual([
      {
        label: 'println',
        detail: 'macro',
        apply: 'println!()',
        type: 'function',
      },
      {
        label: 'value',
        apply: 'value',
        type: 'variable',
      },
    ]);
  });

  it('returns no options for null or empty results', () => {
    expect(completionOptions(null)).toEqual([]);
    expect(completionOptions({ buffer_id: 'b1', items: [] })).toEqual([]);
  });
});

describe('definitionTarget', () => {
  const result: DefinitionResult = {
    buffer_id: 'b1',
    locations: [
      { path: 'C:/workspace/src/lib.rs', line: 7, utf16_column: 4 },
      { path: 'C:/workspace/src/other.rs', line: 1, utf16_column: 0 },
    ],
  };

  it('returns the first matching location', () => {
    expect(definitionTarget(result, 'b1')).toEqual({
      path: 'C:/workspace/src/lib.rs',
      line: 7,
      utf16_column: 4,
    });
  });

  it('rejects mismatched and empty results', () => {
    expect(definitionTarget(result, 'b2')).toBeNull();
    expect(definitionTarget(result, null)).toBeNull();
    expect(definitionTarget(null, 'b1')).toBeNull();
    expect(definitionTarget({ buffer_id: 'b1', locations: [] }, 'b1')).toBeNull();
  });
});

describe('watchDefinitionNavigation', () => {
  it('opens the first matching location as a 1-based line exactly once', () => {
    const listeners = new Set<() => void>();
    let result: DefinitionResult | null = null;
    const openPathAtLine = vi.fn(() => Promise.resolve());
    const stop = watchDefinitionNavigation({
      getDefinitionResult: () => result,
      getActiveBufferId: () => 'b1',
      subscribe: (listener) => {
        listeners.add(listener);
        return () => {
          listeners.delete(listener);
        };
      },
      openPathAtLine,
    });

    result = {
      buffer_id: 'b1',
      locations: [{ path: 'C:/workspace/src/lib.rs', line: 7, utf16_column: 4 }],
    };
    for (const listener of listeners) listener();
    for (const listener of listeners) listener();

    expect(openPathAtLine).toHaveBeenCalledTimes(1);
    expect(openPathAtLine).toHaveBeenCalledWith('C:/workspace/src/lib.rs', 8);
    stop();
  });

  it('ignores answers for another buffer and later accepts a matching one', () => {
    const listeners = new Set<() => void>();
    let result: DefinitionResult | null = null;
    const openPathAtLine = vi.fn(() => Promise.resolve());
    watchDefinitionNavigation({
      getDefinitionResult: () => result,
      getActiveBufferId: () => 'b1',
      subscribe: (listener) => {
        listeners.add(listener);
        return () => {
          listeners.delete(listener);
        };
      },
      openPathAtLine,
    });

    result = {
      buffer_id: 'b2',
      locations: [{ path: 'C:/workspace/src/other.rs', line: 2, utf16_column: 0 }],
    };
    for (const listener of listeners) listener();
    expect(openPathAtLine).not.toHaveBeenCalled();

    result = {
      buffer_id: 'b1',
      locations: [{ path: 'C:/workspace/src/main.rs', line: 0, utf16_column: 0 }],
    };
    for (const listener of listeners) listener();
    expect(openPathAtLine).toHaveBeenCalledWith('C:/workspace/src/main.rs', 1);
  });
});
