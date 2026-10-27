// Selora IDE — Live Git panel (DESIGN §3.3)
import {
  useMemo,
  useState,
  useSyncExternalStore,
  type FC,
  type FormEvent,
  type KeyboardEvent,
} from 'react';
import {
  currentEnvelopeContext,
  getAppSnapshot,
  subscribeApp,
} from '../../app/app-store';
import {
  gitBranches,
  gitCommit,
  gitConflictResolve,
  gitCreateBranch,
  gitDeleteBranch,
  gitDiscard,
  gitFetch,
  gitLog,
  gitMerge,
  gitPull,
  gitPush,
  gitShowCommit,
  gitStage,
  gitStageAll,
  gitStashApply,
  gitStashDrop,
  gitStashList,
  gitStashPop,
  gitStashSave,
  gitSwitchBranch,
  gitUnstage,
  gitUnstageAll,
  openFile,
  openGitDiff,
  type BootstrapState,
} from '../../bridge/bridge';
import {
  AlertCircle,
  ArchiveIcon,
  Check,
  ChevronDown,
  ChevronRight,
  CloseIcon,
  GitBranch,
  Plus,
  RefreshCw,
  RotateLeft,
  Trash2,
  ArrowUpIcon,
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

/** Truthful local-branch row; `current` is only true when the backend says so. */
export interface GitBranchEntry {
  name: string;
  current: boolean;
}

/** One `GitLog` entry, already validated. */
export interface GitLogEntry {
  sha: string;
  summary: string;
  author: string;
  timestamp: number;
}

/** `GitShowCommit`'s detail payload, already validated. */
export interface GitCommitDetail extends GitLogEntry {
  message: string;
  unified_diff: string;
}

/** One `GitStashList` entry, already validated. */
export interface GitStashEntry {
  index: number;
  message: string;
}

type StatusMessage = { kind: 'error' | 'info'; text: string };

type ConfirmRequest = {
  title: string;
  message: string;
  confirmLabel: string;
  action: () => void;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function listOf(result: unknown, key: string): unknown[] {
  if (Array.isArray(result)) return result;
  if (isRecord(result) && Array.isArray(result[key])) return result[key] as unknown[];
  return [];
}

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

/** Additive conflict list; absent on older payloads, so never invented. */
function conflictsOf(status: unknown): string[] {
  const conflicts = (status as { conflicts?: unknown } | null | undefined)?.conflicts;
  if (!Array.isArray(conflicts)) return [];
  return conflicts.filter((path): path is string => typeof path === 'string' && path.length > 0);
}

function branchesOf(result: unknown, checkedOut: string): GitBranchEntry[] {
  const entries: GitBranchEntry[] = [];
  for (const row of listOf(result, 'branches')) {
    if (typeof row === 'string' && row.length > 0) {
      entries.push({ name: row, current: row === checkedOut });
      continue;
    }
    if (!isRecord(row) || typeof row.name !== 'string' || row.name.length === 0) continue;
    entries.push({
      name: row.name,
      current: typeof row.current === 'boolean' ? row.current : row.name === checkedOut,
    });
  }
  return entries;
}

function logOf(result: unknown): GitLogEntry[] {
  const entries: GitLogEntry[] = [];
  for (const row of listOf(result, 'commits')) {
    if (!isRecord(row) || typeof row.sha !== 'string' || row.sha.length === 0) continue;
    if (typeof row.summary !== 'string') continue;
    entries.push({
      sha: row.sha,
      summary: row.summary,
      author: typeof row.author === 'string' ? row.author : '',
      timestamp: typeof row.timestamp === 'number' && Number.isFinite(row.timestamp) ? row.timestamp : 0,
    });
  }
  return entries;
}

function commitDetailOf(result: unknown): GitCommitDetail | null {
  if (!isRecord(result)) return null;
  if (typeof result.sha !== 'string' || result.sha.length === 0) return null;
  const message = typeof result.message === 'string' ? result.message : '';
  if (message.length === 0) return null;
  return {
    sha: result.sha,
    summary:
      typeof result.summary === 'string' && result.summary.length > 0
        ? result.summary
        : message.split('\n')[0],
    author: typeof result.author === 'string' ? result.author : '',
    timestamp: typeof result.timestamp === 'number' && Number.isFinite(result.timestamp) ? result.timestamp : 0,
    message,
    unified_diff: typeof result.unified_diff === 'string' ? result.unified_diff : '',
  };
}

function stashesOf(result: unknown): GitStashEntry[] {
  const entries: GitStashEntry[] = [];
  for (const row of listOf(result, 'stashes')) {
    if (!isRecord(row) || typeof row.index !== 'number' || !Number.isInteger(row.index)) continue;
    entries.push({
      index: row.index,
      message: typeof row.message === 'string' ? row.message : '',
    });
  }
  return entries;
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
  if (cause instanceof Error) return cause.message;
  if (isRecord(cause) && typeof cause.message === 'string') return cause.message;
  return String(cause);
}

function shortSha(sha: string): string {
  return sha.slice(0, 7);
}

/** Unix seconds (or milliseconds) to a locale-independent date, or nothing. */
function formatTimestamp(timestamp: number): string | null {
  if (!Number.isFinite(timestamp) || timestamp <= 0) return null;
  const millis = timestamp > 1e12 ? timestamp : timestamp * 1000;
  const date = new Date(millis);
  if (Number.isNaN(date.getTime())) return null;
  return date.toISOString().slice(0, 10);
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

function diffLines(text: string, label: string) {
  return (
    <div className="git-diff-lines git-diff-readonly" role="region" aria-label={label}>
      {text.split('\n').map((line, index) => (
        <div className={'git-diff-line ' + diffLineClass(line)} key={String(index)}>
          <span className="diff-line-content">{line || ' '}</span>
        </div>
      ))}
    </div>
  );
}

const CONNECTING = 'Selora is still connecting. Try again in a moment.';
const HISTORY_LIMIT = 50;

export interface GitPanelViewProps {
  bootstrap?: BootstrapState;
}

export const GitPanelView: FC<GitPanelViewProps> = ({ bootstrap }) => {
  const app = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const liveBootstrap = app.bootstrap ?? bootstrap;
  const git = liveBootstrap?.git ?? null;
  const branch = git?.branch ?? null;

  // --- Changes -------------------------------------------------------------
  const [stagedOpen, setStagedOpen] = useState(true);
  const [changedOpen, setChangedOpen] = useState(true);
  const [commitMessage, setCommitMessage] = useState('');
  const [busyPath, setBusyPath] = useState<string | null>(null);
  const [isCommitting, setIsCommitting] = useState(false);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [status, setStatus] = useState<StatusMessage | null>(null);

  // --- Branches ------------------------------------------------------------
  const [branchOpen, setBranchOpen] = useState(false);
  const [branches, setBranches] = useState<GitBranchEntry[]>([]);
  const [branchLoading, setBranchLoading] = useState(false);
  const [branchError, setBranchError] = useState<string | null>(null);
  const [newBranch, setNewBranch] = useState('');

  // --- History -------------------------------------------------------------
  const [historyOpen, setHistoryOpen] = useState(false);
  const [history, setHistory] = useState<GitLogEntry[]>([]);
  const [historyLoaded, setHistoryLoaded] = useState(false);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [detail, setDetail] = useState<{
    sha: string;
    loading: boolean;
    error: string | null;
    data: GitCommitDetail | null;
  } | null>(null);

  // --- Stash ---------------------------------------------------------------
  const [stashOpen, setStashOpen] = useState(false);
  const [stashes, setStashes] = useState<GitStashEntry[]>([]);
  const [stashLoaded, setStashLoaded] = useState(false);
  const [stashLoading, setStashLoading] = useState(false);
  const [stashError, setStashError] = useState<string | null>(null);
  const [stashMessage, setStashMessage] = useState('');

  // --- One in-flight command for the non-file surfaces ---------------------
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);

  const files = useMemo(() => preciseFilesOf(git), [git]);
  const conflicts = useMemo(() => conflictsOf(git), [git]);
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

  /**
   * Run one bridge command with a truthful busy state. Every failure becomes
   * a user-visible error (the Lite backend may not implement the additive
   * command yet); nothing is optimistically applied.
   */
  const run = async (
    key: string,
    action: (context: NonNullable<ReturnType<typeof currentEnvelopeContext>>) => Promise<unknown>,
    success?: string,
  ): Promise<boolean> => {
    const context = currentEnvelopeContext();
    if (!context) {
      setStatus({ kind: 'error', text: CONNECTING });
      return false;
    }
    setBusy(key);
    setStatus(null);
    try {
      await action(context);
      if (success) setStatus({ kind: 'info', text: success });
      return true;
    } catch (cause) {
      setStatus({ kind: 'error', text: formatError(cause) });
      return false;
    } finally {
      setBusy((current) => (current === key ? null : current));
    }
  };

  // --- Changes -------------------------------------------------------------

  const runPathAction = async (
    path: string,
    action: (target: string, context: NonNullable<ReturnType<typeof currentEnvelopeContext>>) => Promise<unknown>,
    success?: string,
  ) => {
    const context = currentEnvelopeContext();
    if (!context) {
      setStatus({ kind: 'error', text: CONNECTING });
      return;
    }
    setStatus(null);
    setBusyPath(path);
    try {
      await action(path, context);
      if (success) setStatus({ kind: 'info', text: success });
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
      setStatus({ kind: 'error', text: CONNECTING });
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
      setStatus({ kind: 'error', text: CONNECTING });
      return;
    }
    setStatus(null);
    setIsCommitting(true);
    try {
      await gitCommit(message, context);
      setCommitMessage('');
      setStatus({ kind: 'info', text: 'Commit created.' });
      if (historyOpen) void loadHistory();
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

  const requestDiscard = (path: string) => {
    setConfirm({
      title: 'Discard changes',
      message:
        'Discard all changes to "' + path + '"? This cannot be undone, and an untracked file is deleted.',
      confirmLabel: 'Discard',
      action: () => {
        void runPathAction(
          path,
          gitDiscard,
          'Discarded changes in ' + fileName(path) + '.',
        );
      },
    });
  };

  // --- Branches ------------------------------------------------------------

  const loadBranches = async () => {
    const context = currentEnvelopeContext();
    if (!context) {
      setBranchError(CONNECTING);
      return;
    }
    setBranchLoading(true);
    setBranchError(null);
    setBranches([]);
    try {
      setBranches(branchesOf(await gitBranches(context), branch ?? ''));
    } catch (cause) {
      setBranchError(formatError(cause));
    } finally {
      setBranchLoading(false);
    }
  };

  const toggleBranchMenu = () => {
    if (branchOpen) {
      setBranchOpen(false);
      return;
    }
    setBranchOpen(true);
    void loadBranches();
  };

  const handleSwitchBranch = async (name: string) => {
    const ok = await run('branch-switch', (context) => gitSwitchBranch(name, context), 'Switched to ' + name + '.');
    if (ok) setBranchOpen(false);
  };

  const handleCreateBranch = async (event?: FormEvent) => {
    event?.preventDefault();
    const name = newBranch.trim();
    if (!name) return;
    const ok = await run(
      'branch-create',
      (context) => gitCreateBranch(name, true, context),
      'Created and switched to ' + name + '.',
    );
    if (ok) {
      setNewBranch('');
      setBranchOpen(false);
      void loadHistory();
    }
  };

  const requestDeleteBranch = (name: string) => {
    setConfirm({
      title: 'Delete branch',
      message: 'Delete branch "' + name + '"? This cannot be undone.',
      confirmLabel: 'Delete branch',
      action: () => {
        void run(
          'branch-delete',
          (context) => gitDeleteBranch(name, context),
          'Deleted branch ' + name + '.',
        ).then((ok) => {
          if (ok) void loadBranches();
        });
      },
    });
  };

  const requestMergeBranch = (name: string) => {
    setConfirm({
      title: 'Merge branch',
      message: 'Merge branch "' + name + '" into "' + (branch ?? 'the current branch') + '"?',
      confirmLabel: 'Merge',
      action: () => {
        void run(
          'merge',
          (context) => gitMerge(name, context),
          'Merged ' + name + '.',
        ).then((ok) => {
          if (ok) {
            setBranchOpen(false);
            if (historyOpen) void loadHistory();
          }
        });
      },
    });
  };

  // --- History -------------------------------------------------------------

  const loadHistory = async () => {
    const context = currentEnvelopeContext();
    if (!context) {
      setHistoryError(CONNECTING);
      return;
    }
    setHistoryLoading(true);
    setHistoryError(null);
    try {
      setHistory(logOf(await gitLog(HISTORY_LIMIT, context)));
      setHistoryLoaded(true);
    } catch (cause) {
      setHistory([]);
      setHistoryError(formatError(cause));
    } finally {
      setHistoryLoading(false);
    }
  };

  const toggleHistory = () => {
    const next = !historyOpen;
    setHistoryOpen(next);
    if (next) void loadHistory();
  };

  const handleShowCommit = async (sha: string) => {
    const context = currentEnvelopeContext();
    if (!context) {
      setDetail({ sha, loading: false, error: CONNECTING, data: null });
      return;
    }
    setDetail({ sha, loading: true, error: null, data: null });
    try {
      const data = commitDetailOf(await gitShowCommit(sha, context));
      setDetail({
        sha,
        loading: false,
        error: data ? null : 'Commit details are not available for ' + shortSha(sha) + '.',
        data,
      });
    } catch (cause) {
      setDetail({ sha, loading: false, error: formatError(cause), data: null });
    }
  };

  // --- Stash ---------------------------------------------------------------

  const loadStashes = async () => {
    const context = currentEnvelopeContext();
    if (!context) {
      setStashError(CONNECTING);
      return;
    }
    setStashLoading(true);
    setStashError(null);
    try {
      setStashes(stashesOf(await gitStashList(context)));
      setStashLoaded(true);
    } catch (cause) {
      setStashes([]);
      setStashError(formatError(cause));
    } finally {
      setStashLoading(false);
    }
  };

  const toggleStashMenu = () => {
    if (stashOpen) {
      setStashOpen(false);
      return;
    }
    setStashOpen(true);
    void loadStashes();
  };

  const handleStashSave = async () => {
    const message = stashMessage.trim();
    const ok = await run(
      'stash-save',
      (context) => gitStashSave(message.length > 0 ? message : null, context),
      'Stashed changes.',
    );
    if (ok) {
      setStashMessage('');
      void loadStashes();
    }
  };

  const handleStashApply = async (entry: GitStashEntry) => {
    const ok = await run(
      'stash-apply-' + entry.index,
      (context) => gitStashApply(entry.index, context),
      'Applied stash@{' + entry.index + '}.',
    );
    if (ok) void loadStashes();
  };

  const handleStashPop = async (entry: GitStashEntry) => {
    const ok = await run(
      'stash-pop-' + entry.index,
      (context) => gitStashPop(entry.index, context),
      'Popped stash@{' + entry.index + '}.',
    );
    if (ok) void loadStashes();
  };

  const requestStashDrop = (entry: GitStashEntry) => {
    setConfirm({
      title: 'Drop stash',
      message: 'Drop stash@{' + entry.index + '}? This cannot be undone.',
      confirmLabel: 'Drop stash',
      action: () => {
        void run(
          'stash-drop-' + entry.index,
          (context) => gitStashDrop(entry.index, context),
          'Dropped stash@{' + entry.index + '}.',
        ).then((ok) => {
          if (ok) void loadStashes();
        });
      },
    });
  };

  // --- Conflicts -----------------------------------------------------------

  const requestConflictResolve = (path: string, resolution: 'ours' | 'theirs') => {
    const side = resolution === 'ours' ? 'ours' : 'theirs';
    setConfirm({
      title: 'Resolve conflict',
      message: 'Resolve "' + path + '" with the ' + side + ' version? The other side of the conflict is discarded.',
      confirmLabel: resolution === 'ours' ? 'Use ours' : 'Use theirs',
      action: () => {
        void run(
          'conflict-' + path,
          (context) => gitConflictResolve(path, resolution, context),
          'Resolved ' + fileName(path) + ' with ' + side + '.',
        );
      },
    });
  };

  const handleOpenConflict = (path: string) => {
    void run('open-conflict-' + path, (context) => openFile(path, context));
  };

  const handleConfirm = () => {
    const action = confirm?.action;
    setConfirm(null);
    if (action) action();
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
        {staged ? null : (
          <button
            type="button"
            className="git-row-action-btn is-visible"
            title={'Discard changes to ' + fileName(path)}
            aria-label={'Discard changes to ' + path}
            disabled={isBusy}
            onKeyDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation();
              requestDiscard(path);
            }}
          >
            <RotateLeft size={11} />
          </button>
        )}
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

  const headerBusy = busy !== null;

  return (
    <div className="git-panel-view" data-testid="git-panel-view">
      <div className="git-branch-header">
        <div className="git-branch-anchor">
          <button
            type="button"
            className="git-branch-trigger"
            aria-haspopup="menu"
            aria-expanded={branchOpen}
            aria-label={'Branch: ' + git.branch}
            onClick={toggleBranchMenu}
          >
            <GitBranch size={14} className="git-branch-icon" />
            <span className="git-branch-name" title={'Branch: ' + git.branch}>
              {git.branch}
            </span>
            <ChevronDown size={11} />
          </button>
          <span className="git-branch-count" title={String(stagedPaths.length) + ' staged files'}>
            {stagedPaths.length} staged
          </span>

          {branchOpen ? (
            <div className="git-branch-menu" role="region" aria-label="Branches">
              <div className="git-menu-title">Branches</div>
              {branchLoading ? <div className="git-menu-hint">Loading branches…</div> : null}
              {branchError ? (
                <div className="git-menu-error" role="alert">
                  Branch list unavailable: {branchError}
                </div>
              ) : null}
              {!branchLoading && !branchError ? (
                branches.length === 0 ? (
                  <div className="git-menu-hint">No branches reported.</div>
                ) : (
                  <ul className="git-branch-list" aria-label="Local branches">
                    {branches.map((entry) => (
                      <li className="git-branch-row" key={entry.name}>
                        <button
                          type="button"
                          className="git-branch-row-main"
                          aria-label={
                            entry.current
                              ? 'Current branch ' + entry.name
                              : 'Switch to ' + entry.name
                          }
                          aria-current={entry.current ? 'true' : undefined}
                          disabled={entry.current || headerBusy}
                          onClick={() => void handleSwitchBranch(entry.name)}
                        >
                          {entry.current ? <Check size={11} /> : <GitBranch size={11} />}
                          <span className="git-branch-row-name">{entry.name}</span>
                        </button>
                        {entry.current ? null : (
                          <>
                            <button
                              type="button"
                              className="git-branch-row-action"
                              aria-label={'Merge ' + entry.name + ' into ' + git.branch}
                              title={'Merge ' + entry.name + ' into ' + git.branch}
                              disabled={headerBusy}
                              onClick={() => requestMergeBranch(entry.name)}
                            >
                              Merge
                            </button>
                            <button
                              type="button"
                              className="git-branch-row-action"
                              aria-label={'Delete branch ' + entry.name}
                              title={'Delete branch ' + entry.name}
                              disabled={headerBusy}
                              onClick={() => requestDeleteBranch(entry.name)}
                            >
                              <Trash2 size={11} />
                            </button>
                          </>
                        )}
                      </li>
                    ))}
                  </ul>
                )
              ) : null}
              <form
                className="git-branch-create"
                onSubmit={(event) => {
                  event.preventDefault();
                  void handleCreateBranch();
                }}
              >
                <input
                  className="git-menu-input"
                  value={newBranch}
                  onChange={(event) => setNewBranch(event.target.value)}
                  placeholder="New branch name"
                  aria-label="New branch name"
                  spellCheck={false}
                />
                <button
                  type="submit"
                  className="git-menu-btn"
                  disabled={newBranch.trim().length === 0 || headerBusy}
                >
                  Create branch
                </button>
              </form>
            </div>
          ) : null}
        </div>

        <div className="git-header-actions">
          <button
            type="button"
            className="git-icon-btn"
            title="Fetch"
            aria-label="Fetch"
            aria-busy={busy === 'fetch'}
            disabled={headerBusy}
            onClick={() => void run('fetch', gitFetch, 'Fetched.')}
          >
            <RefreshCw size={13} />
          </button>
          <button
            type="button"
            className="git-icon-btn"
            title="Pull"
            aria-label="Pull"
            aria-busy={busy === 'pull'}
            disabled={headerBusy}
            onClick={() => void run('pull', gitPull, 'Pulled.')}
          >
            <RotateLeft size={13} />
          </button>
          <button
            type="button"
            className="git-icon-btn"
            title="Push"
            aria-label="Push"
            aria-busy={busy === 'push'}
            disabled={headerBusy}
            onClick={() => void run('push', gitPush, 'Pushed.')}
          >
            <ArrowUpIcon size={13} />
          </button>
          <button
            type="button"
            className="git-icon-btn"
            title="Stash"
            aria-label="Stash menu"
            aria-expanded={stashOpen}
            onClick={toggleStashMenu}
          >
            <ArchiveIcon size={13} />
          </button>
        </div>
      </div>

      {stashOpen ? (
        <div className="git-stash-menu" role="region" aria-label="Stashes">
          <div className="git-stash-save-row">
            <input
              className="git-menu-input"
              value={stashMessage}
              onChange={(event) => setStashMessage(event.target.value)}
              placeholder="Optional stash message"
              aria-label="Stash message"
              spellCheck={false}
            />
            <button
              type="button"
              className="git-menu-btn"
              disabled={changedPaths.length === 0 || busy === 'stash-save'}
              title={
                changedPaths.length === 0
                  ? 'No working-tree changes to stash'
                  : 'Stash the working-tree changes'
              }
              onClick={() => void handleStashSave()}
            >
              {busy === 'stash-save' ? 'Stashing…' : 'Stash changes'}
            </button>
          </div>
          <div className="git-menu-title">Stashes</div>
          {stashLoading ? <div className="git-menu-hint">Loading stashes…</div> : null}
          {stashError ? (
            <div className="git-menu-error" role="alert">
              Stash list unavailable: {stashError}
            </div>
          ) : null}
          {!stashLoading && !stashError ? (
            stashes.length === 0 ? (
              <div className="git-menu-hint">
                {stashLoaded ? 'No stashes.' : 'Open the menu to load stashes.'}
              </div>
            ) : (
              <ul className="git-stash-list">
                {stashes.map((entry) => (
                  <li className="git-stash-row" key={entry.index}>
                    <span className="git-stash-label">{'stash@{' + entry.index + '}'}</span>
                    <span className="git-stash-message" title={entry.message}>
                      {entry.message.length > 0 ? entry.message : 'No message'}
                    </span>
                    <button
                      type="button"
                      className="git-branch-row-action"
                      aria-label={'Apply stash ' + entry.index}
                      disabled={headerBusy}
                      onClick={() => void handleStashApply(entry)}
                    >
                      Apply
                    </button>
                    <button
                      type="button"
                      className="git-branch-row-action"
                      aria-label={'Pop stash ' + entry.index}
                      disabled={headerBusy}
                      onClick={() => void handleStashPop(entry)}
                    >
                      Pop
                    </button>
                    <button
                      type="button"
                      className="git-branch-row-action"
                      aria-label={'Drop stash ' + entry.index}
                      disabled={headerBusy}
                      onClick={() => requestStashDrop(entry)}
                    >
                      <Trash2 size={11} />
                    </button>
                  </li>
                ))}
              </ul>
            )
          ) : null}
        </div>
      ) : null}

      {conflicts.length > 0 ? (
        <div className="git-conflict-banner" role="alert" aria-label="Merge conflicts">
          <div className="git-conflict-title">
            <AlertCircle size={13} />
            <strong>Merge conflicts</strong>
            <span>{conflicts.length} file{conflicts.length === 1 ? '' : 's'} to resolve</span>
          </div>
          <ul className="git-conflict-list">
            {conflicts.map((path) => (
              <li className="git-conflict-row" key={path}>
                <span className="git-conflict-path" title={path}>
                  {path}
                </span>
                <button
                  type="button"
                  className="git-conflict-btn"
                  aria-label={'Use ours for ' + path}
                  disabled={headerBusy}
                  onClick={() => requestConflictResolve(path, 'ours')}
                >
                  Ours
                </button>
                <button
                  type="button"
                  className="git-conflict-btn"
                  aria-label={'Use theirs for ' + path}
                  disabled={headerBusy}
                  onClick={() => requestConflictResolve(path, 'theirs')}
                >
                  Theirs
                </button>
                <button
                  type="button"
                  className="git-conflict-btn"
                  aria-label={'Open ' + path}
                  disabled={headerBusy}
                  onClick={() => handleOpenConflict(path)}
                >
                  Open
                </button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

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
        <div className="git-section-header">
          <button
            type="button"
            className="git-section-toggle-btn"
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
          <div className="git-section-actions">
            <button
              type="button"
              className="git-section-btn"
              title="Unstage all changes"
              aria-label="Unstage all changes"
              disabled={stagedPaths.length === 0 || busy === 'unstage-all'}
              onClick={() => void run('unstage-all', gitUnstageAll, 'Unstaged all changes.')}
            >
              −
            </button>
          </div>
        </div>

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
        <div className="git-section-header">
          <button
            type="button"
            className="git-section-toggle-btn"
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
          <div className="git-section-actions">
            <button
              type="button"
              className="git-section-btn"
              title="Stage all changes"
              aria-label="Stage all changes"
              disabled={changedPaths.length === 0 || busy === 'stage-all'}
              onClick={() => void run('stage-all', gitStageAll, 'Staged all changes.')}
            >
              <Plus size={11} />
            </button>
          </div>
        </div>

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

      <div className="git-changes-section git-history-section">
        <div className="git-section-header">
          <button
            type="button"
            className="git-section-toggle-btn"
            onClick={toggleHistory}
            aria-expanded={historyOpen}
          >
            <span className="git-section-toggle" aria-hidden="true">
              {historyOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            </span>
            <span className="git-section-title">HISTORY</span>
            {historyLoaded ? (
              <span className="git-count-pill" title={String(history.length) + ' loaded commits'}>
                {history.length}
              </span>
            ) : null}
          </button>
          <div className="git-section-actions">
            <button
              type="button"
              className="git-section-btn"
              title="Refresh history"
              aria-label="Refresh history"
              disabled={historyLoading}
              onClick={() => void loadHistory()}
            >
              <RefreshCw size={11} />
            </button>
          </div>
        </div>

        {historyOpen ? (
          <div className="git-history-body">
            {historyLoading ? <div className="git-menu-hint">Loading history…</div> : null}
            {historyError ? (
              <div className="git-menu-error" role="alert">
                History unavailable: {historyError}
              </div>
            ) : null}
            {!historyLoading && !historyError ? (
              history.length === 0 ? (
                <div className="git-empty-hint">No commits yet</div>
              ) : (
                <ul className="git-history-list">
                  {history.map((entry) => (
                    <li key={entry.sha}>
                      <button
                        type="button"
                        className="git-history-row"
                        aria-label={'Show commit ' + shortSha(entry.sha)}
                        onClick={() => void handleShowCommit(entry.sha)}
                      >
                        <span className="git-history-sha">{shortSha(entry.sha)}</span>
                        <span className="git-history-summary" title={entry.summary}>
                          {entry.summary}
                        </span>
                        <span className="git-history-meta">
                          {[entry.author, formatTimestamp(entry.timestamp)]
                            .filter((part): part is string => Boolean(part))
                            .join(' · ')}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )
            ) : null}

            {detail ? (
              <div className="git-commit-detail">
                <div className="git-commit-detail-header">
                  <span className="git-commit-detail-title">
                    Commit <strong>{shortSha(detail.sha)}</strong>
                  </span>
                  <button
                    type="button"
                    className="diff-close-btn"
                    title="Close commit details"
                    aria-label="Close commit details"
                    onClick={() => setDetail(null)}
                  >
                    <CloseIcon size={12} />
                  </button>
                </div>
                {detail.loading ? (
                  <div className="git-menu-hint">Loading commit…</div>
                ) : detail.error ? (
                  <div className="git-menu-error" role="alert">
                    {detail.error}
                  </div>
                ) : detail.data ? (
                  <>
                    <div className="git-commit-detail-meta">
                      {[detail.data.author, formatTimestamp(detail.data.timestamp)]
                        .filter((part): part is string => Boolean(part))
                        .join(' · ')}
                    </div>
                    <pre className="git-commit-message">{detail.data.message}</pre>
                    {detail.data.unified_diff.length > 0 ? (
                      diffLines(detail.data.unified_diff, 'Commit diff')
                    ) : (
                      <div className="git-empty-hint">No diff recorded for this commit.</div>
                    )}
                  </>
                ) : null}
              </div>
            ) : null}
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
            diffLines(diffText, 'File diff')
          ) : (
            <div className="git-diff-loading">Loading diff…</div>
          )}
        </div>
      ) : null}

      {confirm ? (
        <div
          className="git-confirm-backdrop"
          onPointerDown={() => setConfirm(null)}
        >
          <div
            className="git-confirm-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="git-confirm-title"
            onPointerDown={(event) => event.stopPropagation()}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.stopPropagation();
                setConfirm(null);
              }
            }}
          >
            <h3 id="git-confirm-title" className="git-confirm-title">
              {confirm.title}
            </h3>
            <p className="git-confirm-message">{confirm.message}</p>
            <div className="git-confirm-actions">
              <button
                type="button"
                className="git-confirm-btn"
                autoFocus
                onClick={() => setConfirm(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="git-confirm-btn is-danger"
                onClick={handleConfirm}
              >
                {confirm.confirmLabel}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
};