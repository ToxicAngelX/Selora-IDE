// Selora IDE — Editor Types and Mock Definitions
// Structured around DESIGN_TOKENS §1.5 syntax hierarchy, line numbers, git diffs,
// fold indicators, breadcrumbs, diagnostics, and minimap rendering.

export type GitDiffType = 'added' | 'modified' | 'deleted';

export type SyntaxTokenType =
  | 'syn-default'
  | 'syn-keyword'
  | 'syn-fn'
  | 'syn-type'
  | 'syn-string'
  | 'syn-number'
  | 'syn-comment'
  | 'syn-punct'
  | 'syn-attr'
  | 'syn-var';

export type SyntaxToken = {
  text: string;
  type: SyntaxTokenType;
};

export type DiagnosticSeverity = 'warning' | 'error' | 'info';

export type DiagnosticItem = {
  line: number;
  word: string;
  severity: DiagnosticSeverity;
  message: string;
  source?: string;
  quickFix?: string;
};

export type CodeLineData = {
  lineNum: number;
  tokens: SyntaxToken[];
  diff?: GitDiffType;
  foldable?: boolean;
  isFolded?: boolean;
  diagnostic?: DiagnosticItem;
};

export type BreadcrumbSegment = {
  id: string;
  label: string;
  kind: 'workspace' | 'dir' | 'file' | 'symbol';
  siblings?: string[];
};

export type EditorTabItem = {
  id: string;
  filename: string;
  path: string;
  language: 'rust' | 'typescript' | 'css';
  isDirty?: boolean;
  breadcrumbs: BreadcrumbSegment[];
  lines: CodeLineData[];
};

export const MOCK_TABS: EditorTabItem[] = [
  {
    id: 'tab-main-rs',
    filename: 'main.rs',
    path: 'crates/selora-core/src/main.rs',
    language: 'rust',
    isDirty: false,
    breadcrumbs: [
      { id: 'b1', label: 'selora-core', kind: 'workspace', siblings: ['selora-core', 'selora-desktop', 'selora-ui'] },
      { id: 'b2', label: 'src', kind: 'dir', siblings: ['src', 'benches', 'tests'] },
      { id: 'b3', label: 'main.rs', kind: 'file', siblings: ['main.rs', 'lib.rs', 'engine.rs', 'broker.rs'] },
      { id: 'b4', label: 'fn main', kind: 'symbol', siblings: ['struct CoreEngine', 'impl CoreEngine', 'fn main'] },
    ],
    lines: [
      {
        lineNum: 1,
        tokens: [{ text: '// Selora IDE Core Engine — Entrypoint', type: 'syn-comment' }],
      },
      {
        lineNum: 2,
        tokens: [
          { text: 'use', type: 'syn-keyword' },
          { text: ' std', type: 'syn-default' },
          { text: '::', type: 'syn-punct' },
          { text: 'sync', type: 'syn-default' },
          { text: '::', type: 'syn-punct' },
          { text: 'Arc', type: 'syn-type' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 3,
        tokens: [
          { text: 'use', type: 'syn-keyword' },
          { text: ' tokio', type: 'syn-default' },
          { text: '::', type: 'syn-punct' },
          { text: 'sync', type: 'syn-default' },
          { text: '::', type: 'syn-punct' },
          { text: 'RwLock', type: 'syn-type' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 4,
        tokens: [{ text: '', type: 'syn-default' }],
      },
      {
        lineNum: 5,
        tokens: [
          { text: 'pub struct', type: 'syn-keyword' },
          { text: ' CoreEngine', type: 'syn-type' },
          { text: ' {', type: 'syn-punct' },
        ],
        foldable: true,
      },
      {
        lineNum: 6,
        tokens: [
          { text: '    epoch', type: 'syn-default' },
          { text: ':', type: 'syn-punct' },
          { text: ' u64', type: 'syn-type' },
          { text: ',', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 7,
        tokens: [
          { text: '    instance_id', type: 'syn-default' },
          { text: ':', type: 'syn-punct' },
          { text: ' String', type: 'syn-type' },
          { text: ',', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 8,
        tokens: [
          { text: '    active_workspace', type: 'syn-default' },
          { text: ':', type: 'syn-punct' },
          { text: ' Option', type: 'syn-type' },
          { text: '<', type: 'syn-punct' },
          { text: 'Arc', type: 'syn-type' },
          { text: '<', type: 'syn-punct' },
          { text: 'RwLock', type: 'syn-type' },
          { text: '<', type: 'syn-punct' },
          { text: 'Workspace', type: 'syn-type' },
          { text: '>>>', type: 'syn-punct' },
          { text: ',', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 9,
        tokens: [{ text: '}', type: 'syn-punct' }],
      },
      {
        lineNum: 10,
        tokens: [{ text: '', type: 'syn-default' }],
      },
      {
        lineNum: 11,
        tokens: [
          { text: 'impl', type: 'syn-keyword' },
          { text: ' CoreEngine', type: 'syn-type' },
          { text: ' {', type: 'syn-punct' },
        ],
        foldable: true,
      },
      {
        lineNum: 12,
        diff: 'modified',
        tokens: [
          { text: '    pub async fn', type: 'syn-keyword' },
          { text: ' bootstrap', type: 'syn-fn' },
          { text: '() -> ', type: 'syn-punct' },
          { text: 'Result', type: 'syn-type' },
          { text: '<', type: 'syn-punct' },
          { text: 'Self', type: 'syn-type' },
          { text: ', ', type: 'syn-punct' },
          { text: 'CoreError', type: 'syn-type' },
          { text: '> {', type: 'syn-punct' },
        ],
        foldable: true,
      },
      {
        lineNum: 13,
        tokens: [
          { text: '        tracing', type: 'syn-default' },
          { text: '::', type: 'syn-punct' },
          { text: 'info', type: 'syn-fn' },
          { text: '!(', type: 'syn-punct' },
          { text: '"Initializing Selora Core kernel v0.1.0"', type: 'syn-string' },
          { text: ');', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 14,
        tokens: [
          { text: '        let mut', type: 'syn-keyword' },
          { text: ' epoch', type: 'syn-default' },
          { text: ' = ', type: 'syn-punct' },
          { text: '1042', type: 'syn-number' },
          { text: ';', type: 'syn-punct' },
        ],
        diagnostic: {
          line: 14,
          word: 'mut',
          severity: 'warning',
          message: "variable does not need to be mutable: `epoch`",
          source: 'rustc (unused_mut)',
          quickFix: 'remove `mut` keyword',
        },
      },
      {
        lineNum: 15,
        tokens: [
          { text: '        let', type: 'syn-keyword' },
          { text: ' instance_id = format', type: 'syn-default' },
          { text: '!(', type: 'syn-punct' },
          { text: '"core-{:04}"', type: 'syn-string' },
          { text: ', epoch % ', type: 'syn-punct' },
          { text: '10000', type: 'syn-number' },
          { text: ');', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 16,
        tokens: [{ text: '', type: 'syn-default' }],
      },
      {
        lineNum: 17,
        tokens: [
          { text: '        // Connect event broker with zero-overhead IPC channel', type: 'syn-comment' },
        ],
      },
      {
        lineNum: 18,
        diff: 'added',
        tokens: [
          { text: '        let', type: 'syn-keyword' },
          { text: ' broker = ', type: 'syn-default' },
          { text: 'EventBroker', type: 'syn-type' },
          { text: '::', type: 'syn-punct' },
          { text: 'connect', type: 'syn-fn' },
          { text: '(&instance_id).', type: 'syn-punct' },
          { text: 'await', type: 'syn-keyword' },
          { text: '?;', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 19,
        diff: 'added',
        tokens: [
          { text: '        broker.', type: 'syn-default' },
          { text: 'publish', type: 'syn-fn' },
          { text: '(', type: 'syn-punct' },
          { text: 'SystemEvent', type: 'syn-type' },
          { text: '::', type: 'syn-punct' },
          { text: 'Ready', type: 'syn-default' },
          { text: ').', type: 'syn-punct' },
          { text: 'await', type: 'syn-keyword' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 20,
        tokens: [{ text: '', type: 'syn-default' }],
      },
      {
        lineNum: 21,
        tokens: [
          { text: '        Ok', type: 'syn-type' },
          { text: '(', type: 'syn-punct' },
          { text: 'Self', type: 'syn-type' },
          { text: ' { epoch, instance_id, active_workspace: ', type: 'syn-default' },
          { text: 'None', type: 'syn-type' },
          { text: ' })', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 22,
        tokens: [{ text: '    }', type: 'syn-punct' }],
      },
      {
        lineNum: 23,
        tokens: [{ text: '', type: 'syn-default' }],
      },
      {
        lineNum: 24,
        tokens: [
          { text: '    pub fn', type: 'syn-keyword' },
          { text: ' instance_id', type: 'syn-fn' },
          { text: '(&', type: 'syn-punct' },
          { text: 'self', type: 'syn-keyword' },
          { text: ') -> &', type: 'syn-punct' },
          { text: 'str', type: 'syn-type' },
          { text: ' {', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 25,
        diff: 'deleted',
        tokens: [
          { text: '        &', type: 'syn-punct' },
          { text: 'self', type: 'syn-keyword' },
          { text: '.instance_id', type: 'syn-default' },
        ],
      },
      {
        lineNum: 26,
        tokens: [{ text: '    }', type: 'syn-punct' }],
      },
      {
        lineNum: 27,
        tokens: [{ text: '}', type: 'syn-punct' }],
      },
      {
        lineNum: 28,
        tokens: [{ text: '', type: 'syn-default' }],
      },
      {
        lineNum: 29,
        tokens: [{ text: '#[tokio::main]', type: 'syn-attr' }],
      },
      {
        lineNum: 30,
        tokens: [
          { text: 'async fn', type: 'syn-keyword' },
          { text: ' main', type: 'syn-fn' },
          { text: '() -> ', type: 'syn-punct' },
          { text: 'Result', type: 'syn-type' },
          { text: '<(), ', type: 'syn-punct' },
          { text: 'Box', type: 'syn-type' },
          { text: '<dyn std::error::Error>> {', type: 'syn-punct' },
        ],
        foldable: true,
      },
      {
        lineNum: 31,
        tokens: [
          { text: '    let', type: 'syn-keyword' },
          { text: ' engine = ', type: 'syn-default' },
          { text: 'CoreEngine', type: 'syn-type' },
          { text: '::', type: 'syn-punct' },
          { text: 'bootstrap', type: 'syn-fn' },
          { text: '().', type: 'syn-punct' },
          { text: 'await', type: 'syn-keyword' },
          { text: '?;', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 32,
        tokens: [
          { text: '    println', type: 'syn-fn' },
          { text: '!(', type: 'syn-punct' },
          { text: '"Selora IDE core initialized: {}"', type: 'syn-string' },
          { text: ', engine.', type: 'syn-punct' },
          { text: 'instance_id', type: 'syn-fn' },
          { text: '());', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 33,
        tokens: [
          { text: '    Ok', type: 'syn-type' },
          { text: '(())', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 34,
        tokens: [{ text: '}', type: 'syn-punct' }],
      },
    ],
  },
  {
    id: 'tab-shell-grid-tsx',
    filename: 'ShellGrid.tsx',
    path: 'apps/selora-desktop/src/layout/ShellGrid.tsx',
    language: 'typescript',
    isDirty: true,
    breadcrumbs: [
      { id: 'sb1', label: 'selora-desktop', kind: 'workspace', siblings: ['selora-desktop', 'selora-ui'] },
      { id: 'sb2', label: 'src', kind: 'dir', siblings: ['src', 'public', 'tests'] },
      { id: 'sb3', label: 'layout', kind: 'dir', siblings: ['layout', 'components', 'styles', 'app'] },
      { id: 'sb4', label: 'ShellGrid.tsx', kind: 'file', siblings: ['ShellGrid.tsx', 'TitleBar.tsx', 'EditorRegion.tsx'] },
      { id: 'sb5', label: 'function ShellGrid', kind: 'symbol', siblings: ['function ShellGrid', 'type ShellGridProps'] },
    ],
    lines: [
      {
        lineNum: 1,
        tokens: [{ text: '// Selora IDE — Seven-region shell grid (DESIGN §1)', type: 'syn-comment' }],
      },
      {
        lineNum: 2,
        tokens: [
          { text: 'import', type: 'syn-keyword' },
          { text: ' { useState, useSyncExternalStore, ', type: 'syn-default' },
          { text: 'type', type: 'syn-keyword' },
          { text: ' CSSProperties } ', type: 'syn-default' },
          { text: 'from', type: 'syn-keyword' },
          { text: " 'react'", type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 3,
        tokens: [
          { text: 'import', type: 'syn-keyword' },
          { text: ' { TitleBar } ', type: 'syn-default' },
          { text: 'from', type: 'syn-keyword' },
          { text: " './TitleBar'", type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 4,
        tokens: [
          { text: 'import', type: 'syn-keyword' },
          { text: ' { EditorRegion } ', type: 'syn-default' },
          { text: 'from', type: 'syn-keyword' },
          { text: " './EditorRegion'", type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 5,
        tokens: [{ text: '', type: 'syn-default' }],
      },
      {
        lineNum: 6,
        tokens: [
          { text: 'export function', type: 'syn-keyword' },
          { text: ' ShellGrid', type: 'syn-fn' },
          { text: '({ bootstrap }: ', type: 'syn-punct' },
          { text: 'ShellGridProps', type: 'syn-type' },
          { text: ') {', type: 'syn-punct' },
        ],
        foldable: true,
      },
      {
        lineNum: 7,
        tokens: [
          { text: '  const', type: 'syn-keyword' },
          { text: ' [viewMode, setViewMode] = ', type: 'syn-default' },
          { text: 'useState', type: 'syn-fn' },
          { text: "<'split' | 'editor'>('split');", type: 'syn-string' },
        ],
      },
      {
        lineNum: 8,
        tokens: [
          { text: '  const', type: 'syn-keyword' },
          { text: ' activeTab = ', type: 'syn-default' },
          { text: 'bootstrap.active_buffer_id;', type: 'syn-default' },
        ],
      },
      {
        lineNum: 9,
        tokens: [{ text: '', type: 'syn-default' }],
      },
      {
        lineNum: 10,
        tokens: [
          { text: '  return (', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 11,
        tokens: [
          { text: '    <div className="shell" data-view-mode={viewMode}>', type: 'syn-default' },
        ],
      },
      {
        lineNum: 12,
        tokens: [
          { text: '      <TitleBar viewMode={viewMode} onSelectViewMode={setViewMode} />', type: 'syn-default' },
        ],
      },
      {
        lineNum: 13,
        tokens: [
          { text: '      <main className="editor-region-slot" aria-label="Editor">', type: 'syn-default' },
        ],
      },
      {
        lineNum: 14,
        tokens: [
          { text: '        <EditorRegion />', type: 'syn-default' },
        ],
      },
      {
        lineNum: 15,
        tokens: [
          { text: '      </main>', type: 'syn-default' },
        ],
      },
      {
        lineNum: 16,
        tokens: [
          { text: '    </div>', type: 'syn-default' },
        ],
      },
      {
        lineNum: 17,
        tokens: [
          { text: '  );', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 18,
        tokens: [{ text: '}', type: 'syn-punct' }],
      },
    ],
  },
  {
    id: 'tab-tokens-css',
    filename: 'tokens.css',
    path: 'apps/selora-desktop/src/styles/tokens.css',
    language: 'css',
    isDirty: false,
    breadcrumbs: [
      { id: 'tb1', label: 'selora-desktop', kind: 'workspace', siblings: ['selora-desktop'] },
      { id: 'tb2', label: 'src', kind: 'dir', siblings: ['src', 'public'] },
      { id: 'tb3', label: 'styles', kind: 'dir', siblings: ['styles', 'layout', 'components'] },
      { id: 'tb4', label: 'tokens.css', kind: 'file', siblings: ['tokens.css', 'base.css', 'shell.css', 'editor.css'] },
      { id: 'tb5', label: ':root', kind: 'symbol', siblings: [':root', '@font-face'] },
    ],
    lines: [
      {
        lineNum: 1,
        tokens: [{ text: '/* Selora IDE — Phase Z2 Design Tokens */', type: 'syn-comment' }],
      },
      {
        lineNum: 2,
        tokens: [
          { text: ':root', type: 'syn-type' },
          { text: ' {', type: 'syn-punct' },
        ],
        foldable: true,
      },
      {
        lineNum: 3,
        tokens: [{ text: '  /* 1.1 The ink ramp */', type: 'syn-comment' }],
      },
      {
        lineNum: 4,
        tokens: [
          { text: '  --ink-canvas', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#000000', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 5,
        tokens: [
          { text: '  --ink-faint', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#0a0a0a', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 6,
        tokens: [
          { text: '  --ink-low', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#141414', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 7,
        tokens: [
          { text: '  --ink-mid', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#1f1f1f', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 8,
        tokens: [
          { text: '  --ink-hi', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#2a2a2a', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 9,
        tokens: [{ text: '', type: 'syn-default' }],
      },
      {
        lineNum: 10,
        tokens: [{ text: '  /* 1.5 Syntax palette */', type: 'syn-comment' }],
      },
      {
        lineNum: 11,
        tokens: [
          { text: '  --syn-fn', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#ffffff', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 12,
        tokens: [
          { text: '  --syn-keyword', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#cfcfcf', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 13,
        tokens: [
          { text: '  --syn-type', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#b8b8b8', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 14,
        tokens: [
          { text: '  --syn-string', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#9a9a9a', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 15,
        tokens: [
          { text: '  --syn-comment', type: 'syn-keyword' },
          { text: ': ', type: 'syn-punct' },
          { text: '#5a5a5a', type: 'syn-string' },
          { text: ';', type: 'syn-punct' },
        ],
      },
      {
        lineNum: 16,
        tokens: [{ text: '}', type: 'syn-punct' }],
      },
    ],
  },
];
