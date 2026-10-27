// Selora IDE — Live workspace file explorer (DESIGN §3.3)
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type FC,
  type KeyboardEvent,
} from 'react';
import {
  currentEnvelopeContext,
  getAppSnapshot,
  subscribeApp,
} from '../../app/app-store';
import {
  openFolderAt,
  pickFolder,
  refreshTree,
  type BootstrapState,
  type FileTreeNode,
} from '../../bridge/bridge';
import { ChevronDown, ChevronRight, FileText, RefreshCw } from '../../components/icons';
import { editorStore } from '../editor/editor-store';

const pathKey = (path: string): string => path.replace(/\\/g, '/').replace(/\/+$/, '');
const fileName = (path: string): string => path.split(/[\\/]/).pop() || path;
const parentPath = (path: string): string => {
  const parts = path.split(/[\\/]/);
  parts.pop();
  return parts.join('/');
};

function countFiles(node: FileTreeNode): number {
  if (node.kind === 'file') return 1;
  return node.children.reduce((total, child) => total + countFiles(child), 0);
}

type FileType = 'rust' | 'typescript' | 'react' | 'css' | 'json' | 'markdown' | 'other';

function detectFileType(name: string): FileType {
  if (name.endsWith('.rs')) return 'rust';
  if (name.endsWith('.tsx') || name.endsWith('.jsx')) return 'react';
  if (name.endsWith('.ts') || name.endsWith('.js')) return 'typescript';
  if (name.endsWith('.css') || name.endsWith('.scss')) return 'css';
  if (name.endsWith('.json') || name.endsWith('.toml') || name.endsWith('.yaml') || name.endsWith('.yml')) {
    return 'json';
  }
  if (name.endsWith('.md')) return 'markdown';
  return 'other';
}

/** File and folder glyphs shared by Files, Search, and Git. */
export const FileTypeIcon: FC<{
  type?: string;
  name: string;
  isFolder?: boolean;
  isOpen?: boolean;
}> = ({ type, name, isFolder, isOpen }) => {
  if (isFolder) {
    return (
      <svg
        className={'tree-icon folder-icon' + (isOpen ? ' is-open' : '')}
        viewBox="0 0 16 16"
        width="14"
        height="14"
        fill="currentColor"
        aria-hidden="true"
        focusable="false"
      >
        {isOpen ? (
          <path d="M1.5 2.5A1.5 1.5 0 0 1 3 1h3.086a1.5 1.5 0 0 1 1.06.44L8.207 2.5H13A1.5 1.5 0 0 1 14.5 4v1.2H2.176l1.248 7.488A1 1 0 0 0 4.41 13.5h8.92a1 1 0 0 0 .984-.818l1.15-6.902A.75.75 0 0 0 14.725 5H1.5V2.5z" />
        ) : (
          <path d="M1.5 2.5A1.5 1.5 0 0 1 3 1h3.086a1.5 1.5 0 0 1 1.06.44L8.207 2.5H13A1.5 1.5 0 0 1 14.5 4v8A1.5 1.5 0 0 1 13 13.5H3A1.5 1.5 0 0 1 1.5 12v-9.5z" />
        )}
      </svg>
    );
  }

  const detected = (type as FileType | undefined) || detectFileType(name);
  switch (detected) {
    case 'rust':
      return <span className="file-badge rust-badge" title="Rust file">rs</span>;
    case 'react':
      return <span className="file-badge react-badge" title="React TSX file">tsx</span>;
    case 'typescript':
      return <span className="file-badge ts-badge" title="TypeScript file">ts</span>;
    case 'css':
      return <span className="file-badge css-badge" title="CSS Style file">#</span>;
    case 'json':
      return <span className="file-badge json-badge" title="JSON configuration">{'{}'}</span>;
    case 'markdown':
      return <span className="file-badge md-badge" title="Markdown documentation">M</span>;
    default:
      return <FileText size={13} className="tree-icon default-file-icon" />;
  }
};

export interface FileTreeViewProps {
  bootstrap?: BootstrapState;
}

export function FileTreeView({ bootstrap }: FileTreeViewProps) {
  const app = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const liveBootstrap = app.bootstrap ?? bootstrap;
  const root = liveBootstrap?.file_tree.root ?? null;

  const [expanded, setExpanded] = useState<Set<string>>(
    () => (root ? new Set([pathKey(root.path)]) : new Set()),
  );
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  // Roving tabindex: the tree is a single Tab stop, arrow keys move the
  // active row. `focusPath` is the row that owns the tab stop.
  const [focusPath, setFocusPath] = useState<string | null>(null);
  const rowRefs = useRef(new Map<string, HTMLDivElement>());
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isOpening, setIsOpening] = useState(false);
  const [status, setStatus] = useState<{ kind: 'error' | 'info'; text: string } | null>(null);

  const activePath = useMemo(() => {
    if (!liveBootstrap?.active_buffer_id) return null;
    return (
      liveBootstrap.buffers.find(
        (buffer) => buffer.buffer_id === liveBootstrap.active_buffer_id,
      )?.path ?? null
    );
  }, [liveBootstrap?.active_buffer_id, liveBootstrap?.buffers]);

  const dirtyPaths = useMemo(
    () =>
      new Set(
        (liveBootstrap?.buffers ?? [])
          .filter((buffer) => buffer.dirty)
          .map((buffer) => pathKey(buffer.path)),
      ),
    [liveBootstrap?.buffers],
  );

  const fileCount = useMemo(() => (root ? countFiles(root) : 0), [root]);

  // Flatten the tree the same way it is rendered: only expanded directories
  // contribute their children, so arrow-key movement follows visual order.
  const visibleNodes = useMemo(() => {
    const rows: Array<{ node: FileTreeNode; depth: number }> = [];
    const visit = (node: FileTreeNode, depth: number) => {
      rows.push({ node, depth });
      if (node.kind !== 'directory' || !expanded.has(pathKey(node.path))) return;
      for (const child of node.children) visit(child, depth + 1);
    };
    if (root) visit(root, 0);
    return rows;
  }, [root, expanded]);

  const visibleKeys = useMemo(
    () => visibleNodes.map((row) => pathKey(row.node.path)),
    [visibleNodes],
  );

  const tabStopKey =
    focusPath && visibleKeys.includes(focusPath) ? focusPath : (visibleKeys[0] ?? null);

  const focusNodeAt = (index: number) => {
    if (visibleNodes.length === 0) return;
    const target = visibleNodes[Math.max(0, Math.min(visibleNodes.length - 1, index))];
    const key = pathKey(target.node.path);
    setFocusPath(key);
    rowRefs.current.get(key)?.focus();
  };

  useEffect(() => {
    setExpanded(root ? new Set([pathKey(root.path)]) : new Set());
    if (!root) setSelectedPath(null);
  }, [root?.path]);

  useEffect(() => {
    if (activePath) setSelectedPath(activePath);
  }, [activePath]);

  const toggleDirectory = (path: string) => {
    const key = pathKey(path);
    setExpanded((previous) => {
      const next = new Set(previous);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const openNode = (node: FileTreeNode) => {
    setSelectedPath(node.path);
    void editorStore.openPath(node.path);
  };

  const activateNode = (node: FileTreeNode) => {
    if (node.kind === 'directory') toggleDirectory(node.path);
    else openNode(node);
  };

  const handleNodeKeyDown = (event: KeyboardEvent<HTMLDivElement>, node: FileTreeNode) => {
    const key = pathKey(node.path);
    const index = visibleNodes.findIndex((row) => pathKey(row.node.path) === key);

    if (event.key === 'ArrowDown') {
      event.preventDefault();
      focusNodeAt(index + 1);
      return;
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      focusNodeAt(index - 1);
      return;
    }
    if (event.key === 'Home') {
      event.preventDefault();
      focusNodeAt(0);
      return;
    }
    if (event.key === 'End') {
      event.preventDefault();
      focusNodeAt(visibleNodes.length - 1);
      return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      activateNode(node);
      return;
    }
    if (node.kind !== 'directory') return;
    if (event.key === 'ArrowRight') {
      event.preventDefault();
      setExpanded((previous) => new Set(previous).add(key));
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      if (expanded.has(key)) {
        setExpanded((previous) => {
          const next = new Set(previous);
          next.delete(key);
          return next;
        });
      } else {
        const parentKey = pathKey(parentPath(node.path));
        const parentIndex = visibleKeys.indexOf(parentKey);
        if (parentIndex >= 0) focusNodeAt(parentIndex);
      }
    }
  };

  const handleOpenFolder = async () => {
    setStatus(null);
    setIsOpening(true);
    try {
      const picked = await pickFolder();
      if (!picked) return;
      const context = currentEnvelopeContext();
      if (!context) {
        throw new Error('Selora is still connecting. Try Open Folder again in a moment.');
      }
      await openFolderAt(picked, context);
    } catch (cause) {
      setStatus({
        kind: 'error',
        text: cause instanceof Error ? cause.message : String(cause),
      });
    } finally {
      setIsOpening(false);
    }
  };

  const handleRefresh = async () => {
    const context = currentEnvelopeContext();
    if (!context) {
      setStatus({ kind: 'error', text: 'Selora is still connecting. Try Refresh again in a moment.' });
      return;
    }
    setStatus(null);
    setIsRefreshing(true);
    try {
      await refreshTree(context);
    } catch (cause) {
      setStatus({
        kind: 'error',
        text: cause instanceof Error ? cause.message : String(cause),
      });
    } finally {
      setIsRefreshing(false);
    }
  };

  const renderNode = (node: FileTreeNode, depth: number) => {
    const isDirectory = node.kind === 'directory';
    const key = pathKey(node.path);
    const isExpanded = expanded.has(key);
    const isSelected = selectedPath !== null && pathKey(selectedPath) === key;
    const isDirty = dirtyPaths.has(key);

    return (
      <div className="tree-node-wrapper" key={node.path || node.name}>
        <div
          className={'tree-node-row' + (isSelected ? ' is-selected' : '')}
          style={{ paddingLeft: String(depth * 14 + 10) + 'px' }}
          role="treeitem"
          aria-level={depth + 1}
          aria-expanded={isDirectory ? isExpanded : undefined}
          aria-selected={isSelected}
          tabIndex={key === tabStopKey ? 0 : -1}
          title={node.path || node.name}
          ref={(element) => {
            if (element) rowRefs.current.set(key, element);
            else rowRefs.current.delete(key);
          }}
          onFocus={() => setFocusPath(key)}
          onClick={() => {
            setFocusPath(key);
            activateNode(node);
          }}
          onKeyDown={(event) => handleNodeKeyDown(event, node)}
        >
          <span className="tree-chevron-slot" aria-hidden="true">
            {isDirectory ? (
              isExpanded ? (
                <ChevronDown size={12} className="chevron-icon" />
              ) : (
                <ChevronRight size={12} className="chevron-icon" />
              )
            ) : (
              <span className="chevron-spacer" />
            )}
          </span>

          <FileTypeIcon
            type={isDirectory ? undefined : detectFileType(node.name)}
            name={node.name}
            isFolder={isDirectory}
            isOpen={isExpanded}
          />

          <span className={'tree-item-name' + (isDirty ? ' is-dirty-text' : '')}>
            {node.name}
          </span>

          {isDirty ? (
            <span className="tree-dirty-dot" title="Modified" aria-label="Modified">
              M
            </span>
          ) : null}
        </div>

        {isDirectory && isExpanded ? (
          <div className="tree-children-container" role="group">
            {node.children.length > 0 ? (
              node.children.map((child) => renderNode(child, depth + 1))
            ) : (
              <div
                className="tree-empty-folder"
                style={{ paddingLeft: String((depth + 1) * 14 + 34) + 'px' }}
              >
                Folder is empty
              </div>
            )}
          </div>
        ) : null}
      </div>
    );
  };

  if (!root) {
    return (
      <div className="file-tree-view" data-testid="file-tree-view">
        <div className="file-tree-empty">
          <div className="file-tree-empty-icon" aria-hidden="true">
            <FileText size={22} />
          </div>
          <h3>No folder open</h3>
          <p>Open a workspace folder to browse its files and start editing.</p>
          <button
            type="button"
            className="empty-state-btn file-tree-open-btn"
            title="Open Folder"
            aria-label="Open Folder"
            aria-busy={isOpening}
            onClick={() => void handleOpenFolder()}
          >
            {isOpening ? 'Opening…' : 'Open Folder'}
          </button>
          {status ? (
            <p className={'file-tree-status is-' + status.kind} role="status">
              {status.text}
            </p>
          ) : null}
        </div>
      </div>
    );
  }

  const displayName = root.name || liveBootstrap?.workspace.display_name || 'Workspace';
  const countLabel = String(fileCount) + (fileCount === 1 ? ' file' : ' files');

  return (
    <div className="file-tree-view" data-testid="file-tree-view">
      <div className="file-tree-action-bar">
        <div className="explorer-title-area">
          <span className="explorer-section-title" title={root.path}>
            {displayName}
          </span>
          <span className="tree-file-count" title={countLabel}>
            {countLabel}
          </span>
        </div>
        <div className="tree-actions-group">
          <button
            type="button"
            className={'tree-action-btn' + (isRefreshing ? ' is-spinning' : '')}
            title="Refresh Explorer"
            aria-label="Refresh Explorer"
            onClick={() => void handleRefresh()}
          >
            <RefreshCw size={13} />
          </button>
          <button
            type="button"
            className="tree-action-btn"
            title="Collapse All Folders"
            aria-label="Collapse All Folders"
            onClick={() => setExpanded(new Set())}
          >
            <ChevronRight size={13} />
          </button>
        </div>
      </div>

      {status ? (
        <div className={'file-tree-status is-' + status.kind} role="status">
          {status.text}
        </div>
      ) : null}

      <div className="file-tree-list" role="tree" aria-label="Workspace files">
        {renderNode(root, 0)}
      </div>
    </div>
  );
}

