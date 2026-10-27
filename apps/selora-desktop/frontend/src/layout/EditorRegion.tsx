// Selora IDE — Phase Z4 editor region.
//
// The tab strip, breadcrumbs, CodeMirror pane, and minimap are all projections
// of the protocol's open-buffer list. The region owns no mock documents: when
// no folder is open it offers a live folder picker, and when a workspace is
// open it points at the Files sidebar with top-level file chips.
import { lazy, Suspense, useCallback, useMemo, useState, useSyncExternalStore } from 'react';
import { EmptyState, EmptyStateButton, KeyChip } from '../components/EmptyState';
import { SeloraMark } from '../components/SeloraMark';
import { EditorTabBar, type EditorTabView } from '../components/editor/EditorTabBar';
import { EditorBreadcrumbs } from '../components/editor/EditorBreadcrumbs';
import { EditorMinimap } from '../components/editor/EditorMinimap';
import type {
  BreadcrumbSegment,
  CodeLineData,
  SyntaxTokenType,
} from '../components/editor/editor-types';
import type { ScrollGeometry } from '../features/editor/CodeMirrorPane';

// CodeMirror and its language modes are the heaviest dependency in the app.
// Load the pane as its own chunk so the shell paints first (Z6 bundle split);
// the editor still owns all buffer/selection authority in Rust.
const CodeMirrorPane = lazy(async () => {
  const module = await import('../features/editor/CodeMirrorPane');
  return { default: module.CodeMirrorPane };
});
import { currentEditorContext, editorStore } from '../features/editor/editor-store';
import {
  absolutePathFor,
  breadcrumbsPathFor,
  copyTextToClipboard,
  relativePathFor,
} from '../features/editor/tab-actions';
import { layoutStore } from '../features/layout/layout-store';
import { requestFileTreeReveal } from '../features/sidebar/file-tree-reveal';
import { toastStore } from '../features/toasts/toast-store';
import { openFolderAt, pickFolder, revealInExplorer, type FileTreeNode } from '../bridge/bridge';
import type { TabMenuAction } from '../components/editor/TabContextMenu';

const MINIMAP_MAX_LINES = 2000;

function filenameOf(path: string): string {
  const segments = path.split(/[\\/]/);
  return segments[segments.length - 1] || path;
}

function topLevelFiles(tree: FileTreeNode | null): FileTreeNode[] {
  if (!tree) return [];
  return tree.children.filter((child) => child.kind === 'file').slice(0, 8);
}

function breadcrumbSegments(
  root: string | null,
  displayName: string | null,
  path: string,
): BreadcrumbSegment[] {
  const normalizedRoot = root ? root.replace(/\\/g, '/').replace(/\/+$/, '') : '';
  const normalizedPath = path.replace(/\\/g, '/');
  const relative =
    normalizedRoot && normalizedPath.toLowerCase().startsWith(`${normalizedRoot.toLowerCase()}/`)
      ? normalizedPath.slice(normalizedRoot.length + 1)
      : normalizedPath;
  const parts = relative.split('/').filter(Boolean);
  const rootLabel =
    displayName || (normalizedRoot ? normalizedRoot.split('/').pop() : null) || 'Workspace';

  const segments: BreadcrumbSegment[] = [
    { id: 'breadcrumb-workspace', label: rootLabel, kind: 'workspace' },
  ];
  parts.forEach((part, index) => {
    segments.push({
      id: `breadcrumb-${index}-${part}`,
      label: part,
      kind: index === parts.length - 1 ? 'file' : 'dir',
    });
  });
  return segments;
}

function minimapTokenType(line: string): SyntaxTokenType {
  const trimmed = line.trimStart();
  if (trimmed.startsWith('//') || trimmed.startsWith('#') || trimmed.startsWith('/*')) {
    return 'syn-comment';
  }
  if (/^(['"]).*\1[,;)]?$/.test(trimmed)) return 'syn-string';
  if (/^(fn|let|const|pub|use|struct|impl|class|function|import|export)\b/.test(trimmed)) {
    return 'syn-keyword';
  }
  return 'syn-default';
}

function minimapLines(text: string): CodeLineData[] {
  return text
    .split(/\r?\n/)
    .slice(0, MINIMAP_MAX_LINES)
    .map((line, index) => ({
      lineNum: index + 1,
      tokens: [{ text: line.length === 0 ? ' ' : line, type: minimapTokenType(line) }],
    }));
}

export function EditorRegion() {
  const editor = useSyncExternalStore(editorStore.subscribe, editorStore.getSnapshot);
  const [scrollGeometry, setScrollGeometry] = useState<ScrollGeometry>({
    scrollTop: 0,
    scrollHeight: 100,
    clientHeight: 100,
  });

  const handleScroll = useCallback((geometry: ScrollGeometry) => {
    setScrollGeometry(geometry);
  }, []);

  const handleOpenFolder = useCallback(async () => {
    const context = currentEditorContext();
    if (!context) {
      toastStore.pushLocal('warning', 'Connect to a workspace before opening a folder.');
      return;
    }
    try {
      const root = await pickFolder();
      if (!root) return;
      await openFolderAt(root, context);
    } catch (error) {
      const detail =
        error instanceof Error && error.message.trim() ? error.message.trim() : 'Unknown error';
      toastStore.pushLocal('error', `Open folder failed: ${detail}`);
    }
  }, []);

  const handleTabMenuAction = useCallback((action: TabMenuAction, tab: EditorTabView) => {
    const current = editorStore.getSnapshot();
    const index = current.tabs.findIndex((candidate) => candidate.id === tab.id);

    const reportBulkClose = (result: { closed: number; kept: number }) => {
      if (result.kept === 0) return;
      const spared = `${result.kept} ${result.kept === 1 ? 'tab' : 'tabs'} stayed open (unsaved or kept open).`;
      toastStore.pushLocal('info', result.closed > 0 ? `Closed ${result.closed}. ${spared}` : spared);
    };

    switch (action) {
      case 'close-others':
        reportBulkClose(
          editorStore.closeMany(
            current.tabs.filter((candidate) => candidate.id !== tab.id).map((candidate) => candidate.id),
          ),
        );
        break;
      case 'close-right':
        reportBulkClose(
          editorStore.closeMany(
            (index >= 0 ? current.tabs.slice(index + 1) : []).map((candidate) => candidate.id),
          ),
        );
        break;
      case 'close-saved':
        reportBulkClose(
          editorStore.closeMany(
            current.tabs.filter((candidate) => !candidate.isDirty).map((candidate) => candidate.id),
          ),
        );
        break;
      case 'close-all':
        reportBulkClose(editorStore.closeMany(current.tabs.map((candidate) => candidate.id)));
        break;
      case 'copy-path':
        void copyTextToClipboard(absolutePathFor(current.workspaceRoot, tab.path), 'path');
        break;
      case 'copy-relative-path':
        void copyTextToClipboard(relativePathFor(tab.path), 'relative path');
        break;
      case 'copy-breadcrumbs-path':
        void copyTextToClipboard(
          breadcrumbsPathFor(current.workspaceName, current.workspaceRoot, tab.path),
          'breadcrumbs path',
        );
        break;
      case 'reveal-in-explorer': {
        const context = currentEditorContext();
        if (!context) {
          toastStore.pushLocal('warning', 'Connect to a workspace before revealing a file.');
          break;
        }
        // The bridge reports failures through the command-error toast path.
        void revealInExplorer(tab.path, context).catch(() => undefined);
        break;
      }
      case 'reveal-in-view': {
        const layout = layoutStore.getSnapshot().layout;
        if (!layout || !layout.primary_sidebar_open || layout.activity_view !== 'files') {
          void layoutStore.toggle('files');
        }
        requestFileTreeReveal(tab.path);
        break;
      }
      case 'toggle-pin':
        editorStore.togglePinned(tab.id);
        break;
      case 'find-references':
        void (async () => {
          const layout = layoutStore.getSnapshot().layout;
          if (!layout || !layout.primary_sidebar_open || layout.activity_view !== 'search') {
            await layoutStore.toggle('search');
          }
          window.dispatchEvent(
            new CustomEvent('selora:search-query', { detail: { query: tab.filename } }),
          );
        })();
        break;
      default:
        break;
    }
  }, []);

  const active = editor.activeBuffer;
  const activeTab = editor.tabs.find((tab) => tab.id === editor.activeBufferId) ?? null;
  const recentFiles = useMemo(() => topLevelFiles(editor.fileTree), [editor.fileTree]);
  const breadcrumbs = useMemo(
    () => (active ? breadcrumbSegments(editor.workspaceRoot, editor.workspaceName, active.path) : []),
    [active, editor.workspaceName, editor.workspaceRoot],
  );
  const minimap = useMemo(() => (active ? minimapLines(active.text) : []), [active]);
  const tabViews = useMemo(
    () =>
      editor.tabs.map((tab) => ({
        ...tab,
        isPinned: editor.pinnedIds.includes(tab.id),
      })),
    [editor.tabs, editor.pinnedIds],
  );

  if (!active || !activeTab) {
    const hasWorkspace = Boolean(editor.workspaceRoot);
    return (
      <div className="editor-region">
        <div className="editor-tabbar" aria-hidden="true" />
        <div className="editor-canvas">
          {hasWorkspace ? (
            <>
              <EmptyState
                heading="Open a file from the Files sidebar"
                hint={
                  <em>
                    {editor.workspaceName
                      ? `${editor.workspaceName} is open, but no buffer is active.`
                      : 'Choose a file to start editing.'}
                  </em>
                }
                mark={
                  <div className="glowing-selora-mark">
                    <SeloraMark size={64} />
                  </div>
                }
              />
              {recentFiles.length > 0 ? (
                <div className="empty-recent-files">
                  <span className="empty-recent-title">Top-level files</span>
                  <div className="empty-recent-list">
                    {recentFiles.map((file) => (
                      <button
                        key={file.path}
                        type="button"
                        className="empty-recent-btn"
                        onClick={() => void editorStore.openPath(file.path)}
                        title={`Open ${file.path}`}
                      >
                        <span>{file.name || filenameOf(file.path)}</span>
                        <span className="recent-path">{file.path}</span>
                      </button>
                    ))}
                  </div>
                </div>
              ) : null}
            </>
          ) : (
            <EmptyState
              heading="No folder open"
              hint={<em>Open a folder to start editing.</em>}
              mark={
                <div className="glowing-selora-mark">
                  <SeloraMark size={64} />
                </div>
              }
            >
              <EmptyStateButton onClick={() => void handleOpenFolder()} title="Choose a workspace folder">
                Open Folder
              </EmptyStateButton>
              <div className="empty-chips" role="group" aria-label="Shell shortcuts">
                <KeyChip>Ctrl+B</KeyChip>
                <KeyChip>Ctrl+J</KeyChip>
                <KeyChip>Ctrl+Shift+P</KeyChip>
              </div>
            </EmptyState>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="editor-region">
      <EditorTabBar
        tabs={tabViews}
        activeTabId={editor.activeBufferId}
        onSelectTab={(id) => editorStore.setActive(id)}
        onCloseTab={(id) => editorStore.closeBuffer(id)}
        onSaveTab={(id) => editorStore.saveBuffer(id)}
        onTabMenuAction={handleTabMenuAction}
      />

      <EditorBreadcrumbs segments={breadcrumbs} />

      <div
        className="editor-body"
        role="tabpanel"
        aria-labelledby={editor.activeBufferId ? `editor-tab-${editor.activeBufferId}` : undefined}
        id="editor-surface"
      >
        <Suspense
          fallback={<div className="editor-pane-loading" aria-busy="true" />}
        >
          <CodeMirrorPane
            buffer={active}
            onScroll={handleScroll}
            revealTarget={editor.revealTarget}
          />
        </Suspense>
        <EditorMinimap
          lines={minimap}
          scrollTop={scrollGeometry.scrollTop}
          scrollHeight={scrollGeometry.scrollHeight}
          clientHeight={scrollGeometry.clientHeight}
          onNavigateLine={(line) => void editorStore.openPathAtLine(active.path, line)}
        />
      </div>
    </div>
  );
}
