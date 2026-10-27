// Typing-latency harness (scripts/perf.ps1).
//
// Measures the synchronous frontend cost of one keystroke: CodeMirror's
// transaction (history, brackets, lint viewport) plus the protocol-change
// computation and the local buffer bookkeeping in CodeMirrorPane's update
// listener. `applyEditorChange` is mocked, so the IPC round trip and paint are
// NOT part of this number; the Rust harness measures the backend half.
//
// happy-dom has no layout engine, so measure-only (no assertions on absolute
// timings). Correctness assertions keep the measurement honest.
import { describe, expect, it, vi } from 'vitest';
import { render, waitFor } from '@testing-library/react';
import { EditorView } from '@codemirror/view';
import type { BufferSnapshot } from '../src/bridge/bridge';

vi.mock('../src/bridge/bridge', () => ({
  applyEditorChange: vi.fn(() => Promise.resolve(undefined)),
  setSelection: vi.fn(() => Promise.resolve(undefined)),
}));

const appHarness = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  const snapshot = { phase: 'ready', bootstrap: null, error: null };
  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
});

vi.mock('../src/app/app-store', () => ({
  getAppSnapshot: appHarness.getSnapshot,
  subscribeApp: appHarness.subscribe,
  currentEnvelopeContext: () => ({
    core_instance_id: 'core-0001',
    workspace_id: 'ws-1',
    session_id: null,
  }),
}));

import { CodeMirrorPane } from '../src/features/editor/CodeMirrorPane';

function documentOf(lines: number): string {
  return Array.from(
    { length: lines },
    (_, index) => `fn line_${index}() { let value = ${index}; }`,
  ).join('\n');
}

function percentile(sorted: number[], fraction: number): number {
  if (sorted.length === 0) return 0;
  return sorted[Math.round((sorted.length - 1) * fraction)]!;
}

function summary(samples: number[]) {
  const sorted = [...samples].sort((a, b) => a - b);
  return {
    n: sorted.length,
    p50_ms: percentile(sorted, 0.5),
    p95_ms: percentile(sorted, 0.95),
    max_ms: sorted[sorted.length - 1] ?? 0,
    mean_ms: sorted.reduce((sum, value) => sum + value, 0) / Math.max(sorted.length, 1),
  };
}

async function measure(lines: number, edits: number) {
  const buffer: BufferSnapshot = {
    buffer_id: `perf-${lines}`,
    path: `C:/workspace/perf_${lines}.rs`,
    language: 'rust',
    text: documentOf(lines),
    version: 1,
    dirty: false,
    selection: null,
  };

  const { container, unmount } = render(<CodeMirrorPane buffer={buffer} />);
  await waitFor(() => {
    expect(container.querySelector('.cm-editor')).not.toBeNull();
  });
  const view = EditorView.findFromDOM(container.querySelector<HTMLElement>('.cm-editor')!);
  expect(view).not.toBeNull();

  let position = view!.state.doc.line(Math.min(10, view!.state.doc.lines)).from;
  // Warm up JIT, the protocol path, and CodeMirror's first update.
  for (let index = 0; index < 20; index += 1) {
    view!.dispatch({ changes: { from: position, insert: 'w' }, selection: { anchor: position + 1 } });
    position += 1;
  }

  const samples: number[] = [];
  for (let index = 0; index < edits; index += 1) {
    const start = performance.now();
    view!.dispatch({ changes: { from: position, insert: 'x' }, selection: { anchor: position + 1 } });
    samples.push(performance.now() - start);
    position += 1;
  }

  const result = {
    metric: 'typing_latency_frontend',
    lines,
    edits,
    ...summary(samples),
    doc_chars: view!.state.doc.length,
  };
  unmount();
  return result;
}

describe('typing latency (frontend, no paint, mocked IPC)', () => {
  it('measures a 1k-line and a 10k-line document', async () => {
    const small = await measure(1_000, 300);
    const large = await measure(10_000, 300);
    // One PERF_JSON line per scenario for the PowerShell harness.
    console.log(`PERF_JSON ${JSON.stringify(small)}`);
    console.log(`PERF_JSON ${JSON.stringify(large)}`);
    expect(small.doc_chars).toBeGreaterThan(0);
    expect(large.doc_chars).toBeGreaterThan(small.doc_chars);
  });
});