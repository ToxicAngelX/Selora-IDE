// Selora IDE — Phase Z4b editor-intelligence adapters.
//
// The protocol speaks 0-based lines and UTF-16 columns, while CodeMirror uses
// UTF-16 document offsets. These helpers keep that translation (and the small
// amount of result shaping) pure so the editor pane only deals with wiring.
import type { Completion } from '@codemirror/autocomplete';
import type {
  CompletionResult as ProtocolCompletionResult,
  DefinitionLocation,
  DefinitionResult,
  Diagnostic,
} from '../../bridge/bridge';
import { positionToOffset } from './text-offsets';

export type CmDiagnostic = {
  from: number;
  to: number;
  severity: Diagnostic['severity'];
  message: string;
};

type LineSlice = {
  from: number;
  to: number;
};

const COMPLETION_TYPES: Readonly<Record<string, string>> = {
  class: 'class',
  constructor: 'function',
  constant: 'constant',
  enum: 'enum',
  enummember: 'enum',
  event: 'property',
  field: 'property',
  fn: 'function',
  function: 'function',
  getter: 'function',
  interface: 'interface',
  keyword: 'keyword',
  local: 'variable',
  macro: 'function',
  method: 'function',
  module: 'namespace',
  namespace: 'namespace',
  operator: 'keyword',
  package: 'namespace',
  parameter: 'variable',
  property: 'property',
  setter: 'function',
  snippet: 'text',
  static: 'constant',
  struct: 'class',
  text: 'text',
  trait: 'interface',
  type: 'type',
  typeparameter: 'type',
  variable: 'variable',
};

function pathsEqual(left: string, right: string): boolean {
  return left.replace(/\\/g, '/').toLowerCase() === right.replace(/\\/g, '/').toLowerCase();
}

function lineSlices(text: string): LineSlice[] {
  const lines: LineSlice[] = [];
  let lineStart = 0;
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    if (char !== '\r' && char !== '\n') continue;
    lines.push({ from: lineStart, to: index });
    if (char === '\r' && text[index + 1] === '\n') index += 1;
    lineStart = index + 1;
  }
  lines.push({ from: lineStart, to: text.length });
  return lines;
}

function clampInteger(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(Math.max(Math.trunc(value), min), max);
}

/**
 * A protocol diagnostic carries only a start position. Underline the token
 * that starts there, or fall back to the rest of the line at whitespace or EOL.
 */
function diagnosticEnd(text: string, from: number, lineEnd: number): number {
  let end = from;
  while (end < lineEnd) {
    const char = text[end];
    if (char === ' ' || char === '\t' || char === '\v' || char === '\f' || char === '\u00a0') {
      break;
    }
    end += 1;
  }
  return end > from ? end : lineEnd;
}

export function toCmDiagnostics(
  activePath: string,
  diagnostics: readonly Diagnostic[],
  docText: string,
): CmDiagnostic[] {
  const lines = lineSlices(docText);
  const result: CmDiagnostic[] = [];

  for (const diagnostic of diagnostics) {
    if (!pathsEqual(activePath, diagnostic.path)) continue;
    const lineIndex = Math.trunc(diagnostic.line);
    if (!Number.isFinite(lineIndex) || lineIndex < 0 || lineIndex >= lines.length) continue;

    const line = lines[lineIndex];
    const column = clampInteger(diagnostic.column, 0, line.to - line.from);
    const from = positionToOffset(docText, { line: lineIndex, utf16_column: column });
    const to = diagnosticEnd(docText, from, line.to);
    result.push({
      from,
      to: Math.max(from, Math.min(to, docText.length)),
      severity: diagnostic.severity,
      message: diagnostic.message,
    });
  }

  return result;
}

function completionType(kind: string | null): string | undefined {
  if (!kind) return undefined;
  return COMPLETION_TYPES[kind.trim().toLowerCase()];
}

export function completionOptions(result: ProtocolCompletionResult | null): Completion[] {
  if (!result) return [];
  return result.items.map((item) => {
    const option: Completion = {
      label: item.label,
      apply: item.insert_text ?? item.label,
    };
    if (item.detail) option.detail = item.detail;
    const type = completionType(item.kind);
    if (type) option.type = type;
    return option;
  });
}

export function definitionTarget(
  result: DefinitionResult | null,
  activeBufferId: string | null,
): DefinitionLocation | null {
  if (!result || !activeBufferId || result.buffer_id !== activeBufferId) return null;
  return result.locations[0] ?? null;
}

export type DefinitionNavigationDeps = {
  getDefinitionResult: () => DefinitionResult | null;
  getActiveBufferId: () => string | null;
  subscribe: (listener: () => void) => () => void;
  openPathAtLine: (path: string, line: number) => void | Promise<void>;
};

/**
 * Consume each fresh definition answer exactly once. The consumed result object
 * is remembered locally because opening a location notifies app-store listeners
 * again with the same answer object.
 */
export function watchDefinitionNavigation(deps: DefinitionNavigationDeps): () => void {
  let consumed = deps.getDefinitionResult();
  return deps.subscribe(() => {
    const result = deps.getDefinitionResult();
    if (!result || result === consumed) return;
    consumed = result;

    const target = definitionTarget(result, deps.getActiveBufferId());
    if (!target) return;
    void deps.openPathAtLine(target.path, target.line + 1);
  });
}
