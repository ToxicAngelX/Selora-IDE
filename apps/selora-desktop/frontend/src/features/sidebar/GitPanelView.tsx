// Selora IDE — Live Git panel (DESIGN §3.3)
import {
  useMemo,
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
  gitCommit,
  gitStage,
  gitUnstage,
  openGitDiff,
  type BootstrapState,
} from '../../bridge/bridge';
import {
  Check,
  ChevronDown,
  ChevronRight,
  CloseIcon,
  GitBranch,
  Plus,
} from '../../components/icons';
import { FileTypeIcon } from './FileTreeView';

const normalizePath = (path: string): string => path.replace(/\\/g, '/').replace(/\/+$/, '');
const fileName = (path: string): string => path.split(/[\\/]/).pop() || path;
const parentPath = (path: string): string => {
  const parts = path.split(/[\\/]/);
  parts.pop();
  return parts.join('/');
};

/** P2: one changed file's two independent git slots (index / worktree). */
export interface GitFileStatus {
  path: string;
  staged: boolean;
  worktree: boolean;
}

/** Stable empty list so the legacy-array fallbacks keep their identity. */
const NO_PATHS: string[] = [];

/**
 * P2: read the precise per-slot `files` list. `BootstrapState['git']` still
 * types only the two legacy arrays (bridge.ts is outside this slice), and
 * pre-P2 payloads omit `files` entirely, so the lookup is structural and
 * validated; anything unrecognised is ignored rather than trusted.
 */
function preciseFilesOf(status: unknown): GitFileStatus[] {
  const files = (status as { files?: unknown } | null | undefined)?.files;
  if (!Array.isArray(files)) return [];
  return (files as GitFileStatus[]).filter(
    (file) =>
      typeof file?.path === 'string' &&
      typeof file.staged === 'boolean' &&
      typeof file.worktree === 'boolean',
  );
}

/** First occurrence wins; the panel renders one row per path and section. */
function uniquePaths(paths: string[]): string[] {
  const seen = new Set<string>();
  const unique: string[] = [];
  for (const path of paths) {
    const key = normalizePath(path);
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(path);
  }
  return unique;
}

function formatError(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

function diffLineClass(line: string): string {
  if (line.startsWith('+++') || line.startsWith('---') || line.startsWith('diff --git') || line.startsWith('index ')) {
    return 'is-meta';
  }
  if (line.startsWith('@@')) return 'is-hunk';
  if (line.startsWith('+')) return 'is-add';
  if (line.startsWith('-')) return 'is-del';
  return 'is-same';
}

export interface GitPanelViewProps {
  bootstrap?: BootstrapState;
}

export const GitPanelView: FC<GitPanelViewProps> = ({ bootstrap }) => {
  const app = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const liveBootstrap = app.bootstrap ?? bootstrap;
  const git = liveBootstrap?.git ?? null;

  const [stagedOpen, setStagedOpen] = useState(true);
  const [changedOpen, setChangedOpen] = useState(true);
  const [commitMessage, setCommitMessage] = useState('');
  const [busyPath, setBusyPath] = useState<string | null>(null);
  const [isCommitting, setIsCommitting] = useState(false);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [status, setStatus] = useState<{ kind: 'error' | 'info'; text: string } | null>(null);

  const files = useMemo(() => preciseFilesOf(git), [git]);
  // Legacy fallback: the flattened arrays, which cannot express a file that
  // is staged *and* still has unstaged edits (that file used to vanish from
  // the unstaged section entirely).
  const legacyStagedPaths = git?.staged_paths ?? NO_PATHS;
  const legacyStagedSet = useMemo(
    () => new Set(legacyStagedPaths.map(normalizePath)),
    [legacyStagedPaths],
  );
  const legacyChangedPaths = useMemo(
    () => (git?.changed_paths ?? NO_PATHS).filter((path) => !legacyStagedSet.has(normalizePath(path))),
    [git?.changed_paths, legacyStagedSet],
  );

  // P2: `files` is the precise source whenever the payload carries it — a
  // file appears in the staged section, the unstaged section, or both,
  // according to its own slots. Old payloads fall back to the arrays.
  const stagedPaths = useMemo(
    () =>
      uniquePaths(
        files.length > 0
          ? files.filter((file) => file.staged).map((file) => file.path)
          : legacyStagedPaths,
      ),
    [files, legacyStagedPaths],
  );
  const changedPaths = useMemo(
    () =>
      uniquePaths(
        files.length > 0
          ? files.filter((file) => file.worktree).map((file) => file.path)
          : legacyChangedPaths,
      ),
    [files, legacyChangedPaths],
  );

  const diffText = useMemo(() => {
    if (!selectedPath || !app.gitDiff) return null;
    return normalizePath(app.gitDiff.path) === normalizePath(selectedPath)
      ? app.gitDiff.unified_diff
      : null;
  }, [app.gitDiff, selectedPath]);

  const runPathAction = async (
    path: string,
    action: (target: string, context: NonNullable<ReturnType<typeof currentEnvelopeContext>>) => Promise<unknown>,
  ) => {
    const context = currentEnvelopeContext();
    if (!context) {
      setStatus({ kind: 'error', text: 'Selora is still connecting. Try again in a moment.' });
      return;
    }
    setStatus(null);
    setBusyPath(path);
    try {
      await action(path, context);
    } catch (cause) {
      setStatus({ kind: 'error', text: formatError(cause) });
    } finally {
      setBusyPath(null);
    }
  };

  const handleOpenDiff = async (path: string) => {
    setSelectedPath(path);
    const context = currentEnvelopeContext();
    if (!context) {
      setStatus({ kind: 'error', text: 'Selora is still connecting. Try again in a moment.' });
      return;
    }
    setStatus(null);
    try {
      await openGitDiff(path, context);
    } catch (cause) {
      setStatus({ kind: 'error', text: formatError(cause) });
    }
  };

  const handleCommit = async () => {
    const message = commitMessage.trim();
    if (!message || stagedPaths.length === 0) return;
    const context = currentEnvelopeContext();
    if (!context) {
      setStatus({ kind: 'error', text: 'Selora is still connecting. Try again in a moment.' });
      return;
    }
    setStatus(null);
    setIsCommitting(true);
    try {
      await gitCommit(message, context);
      setCommitMessage('');
      setStatus({ kind: 'info', text: 'Commit created.' });
    } catch (cause) {
      setStatus({ kind: 'error', text: formatError(cause) });
    } finally {
      setIsCommitting(false);
    }
  };

  const handleCommitKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
      event.preventDefault();
      void handleCommit();
    }
  };

  const renderFileRow = (path: string, staged: boolean) => {
    const isBusy = busyPath === path;
    return (
      <div
        key={path}
        className={'git-file-row' + (selectedPath === path ? ' is-selected' : '')}
        role="button"
        tabIndex={0}
        title={path + ' (click to view diff)'}
        aria-label={'View diff for ' + path}
        onClick={() => void handleOpenDiff(path)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            void handleOpenDiff(path);
          }
        }}
      >
        <FileTypeIcon name={fileName(path)} />
        <span className="git-file-name">{fileName(path)}</span>
        <span className="git-file-dir">{parentPath(path)}</span>
        <button
          type="button"
          className="git-row-action-btn is-visible"
          title={staged ? 'Unstage ' + fileName(path) : 'Stage ' + fileName(path)}
          aria-label={staged ? 'Unstage ' + fileName(path) : 'Stage ' + fileName(path)}
          disabled={isBusy}
          onKeyDown={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation();
            void runPathAction(path, staged ? gitUnstage : gitStage);
          }}
        >
          {staged ? '−' : <Plus size={11} />}
        </button>
      </div>
    );
  };

  if (!git || !git.branch) {
    return (
      <div className="git-panel-view" data-testid="git-panel-view">
        <div className="git-no-repository-state">
          <div className="git-no-repository-icon" aria-hidden="true">
            <GitBranch size={24} />
          </div>
          <h3>No Git repository in this workspace</h3>
          <p>Open a folder backed by Git to stage files, review diffs, and commit.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="git-panel-view" data-testid="git-panel-view">
      <div className="git-branch-header">
        <div className="git-branch-info">
          <GitBranch size={14} className="git-branch-icon" />
          <span className="git-branch-name" title={'Branch: ' + git.branch}>
            {git.branch}
          </span>
          <span className="git-branch-count" title={String(stagedPaths.length) + ' staged files'}>
            {stagedPaths.length} staged
          </span>
        </div>
      </div>

      <div className="git-commit-box">
        <textarea
          className="git-commit-textarea"
          value={commitMessage}
          onChange={(event) => setCommitMessage(event.target.value)}
          onKeyDown={handleCommitKeyDown}
          placeholder="Commit message (Ctrl+Enter to commit)"
          aria-label="Commit message"
          rows={3}
          spellCheck={false}
        />

        <div className="git-commit-action-row">
          <span className="git-char-count">
            {commitMessage.length > 0 ? String(commitMessage.length) + ' chars' : ''}
          </span>
          <button
            type="button"
            className="git-commit-btn"
            onClick={() => void handleCommit()}
            disabled={!commitMessage.trim() || stagedPaths.length === 0 || isCommitting}
            title="Commit staged changes (Ctrl+Enter)"
          >
            <Check size={12} className="commit-check-icon" />
            <span>{isCommitting ? 'Committing…' : 'Commit'}</span>
            <kbd className="git-commit-kbd">Ctrl+↵</kbd>
          </button>
        </div>
      </div>

      {status ? (
        <div className={'git-status-message is-' + status.kind} role="status">
          {status.text}
        </div>
      ) : null}

      <div className="git-changes-section">
        <button
          type="button"
          className="git-section-header"
          onClick={() => setStagedOpen((previous) => !previous)}
          aria-expanded={stagedOpen}
        >
          <span className="git-section-toggle" aria-hidden="true">
            {stagedOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          </span>
          <span className="git-section-title">STAGED CHANGES</span>
          <span className="git-count-pill" title={String(stagedPaths.length) + ' staged files'}>
            {stagedPaths.length}
          </span>
        </button>

        {stagedOpen ? (
          <div className="git-file-list">
            {stagedPaths.length === 0 ? (
              <div className="git-empty-hint">No staged changes</div>
            ) : (
              stagedPaths.map((path) => renderFileRow(path, true))
            )}
          </div>
        ) : null}
      </div>

      <div className="git-changes-section">
        <button
          type="button"
          className="git-section-header"
          onClick={() => setChangedOpen((previous) => !previous)}
          aria-expanded={changedOpen}
        >
          <span className="git-section-toggle" aria-hidden="true">
            {changedOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          </span>
          <span className="git-section-title">CHANGES</span>
          <span className="git-count-pill" title={String(changedPaths.length) + ' changed files'}>
            {changedPaths.length}
          </span>
        </button>

        {changedOpen ? (
          <div className="git-file-list">
            {changedPaths.length === 0 ? (
              <div className="git-empty-hint">Working tree clean</div>
            ) : (
              changedPaths.map((path) => renderFileRow(path, false))
            )}
          </div>
        ) : null}
      </div>

      {selectedPath ? (
        <div className="git-diff-preview-card">
          <div className="git-diff-preview-header">
            <span className="diff-file-label" title={selectedPath}>
              Diff: <strong>{fileName(selectedPath)}</strong>
            </span>
            <button
              type="button"
              className="diff-close-btn"
              onClick={() => setSelectedPath(null)}
              title="Close Diff"
              aria-label="Close Diff"
            >
              <CloseIcon size={12} />
            </button>
          </div>
          {diffText ? (
            <div className="git-diff-lines git-diff-readonly" role="region" aria-label="File diff">
              {diffText.split('\n').map((line, index) => (
                <div className={'git-diff-line ' + diffLineClass(line)} key={String(index)}>
                  <span className="diff-line-content">{line || ' '}</span>
                </div>
              ))}
            </div>
          ) : (
            <div className="git-diff-loading">Loading diff…</div>
          )}
        </div>
      ) : null}
    </div>
  );
};

