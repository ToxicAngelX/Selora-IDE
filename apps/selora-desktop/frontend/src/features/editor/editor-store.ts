// Selora IDE — Phase Z4 editor store.
//
// The protocol owns open buffers; this store is the thin, observable view of
// `bootstrap.buffers` that the editor region renders. Two pieces of state are
// local by design: which buffer is active in the pane and a one-shot reveal
// request (path + 1-based line) issued by search, diagnostics, or the minimap.
import { currentEnvelopeContext, getAppSnapshot, subscribeApp } from '../../app/app-store';
import {
  closeBuffer as closeBufferCommand,
  keepBufferEdits as keepBufferEditsCommand,
  openFile as openFileCommand,
  reloadBufferFromDisk as reloadBufferFromDiskCommand,
  saveBuffer as saveBufferCommand,
  setActiveBuffer as setActiveBufferCommand,
  type BootstrapState,
  type BufferSnapshot,
  type EditorContextDto,
  type EnvelopeContext,
  type FileTreeNode,
  type Selection,
} from '../../bridge/bridge';
import { toastStore } from '../toasts/toast-store';

export type EditorTab = {
  id: string;
  path: string;
  filename: string;
  language: string;
  isDirty: boolean;
  /** P2 gap 3: the file also changed on disk (local edits were kept). */
  externallyChanged: boolean;
  version: number;
};

export type RevealTarget = {
  path: string;
  line: number;
  requestId: number;
};

export type EditorStoreState = {
  tabs: EditorTab[];
  activeBufferId: string | null;
  activeBuffer: BufferSnapshot | null;
  activeSelection: Selection | null;
  revealTarget: RevealTarget | null;
  /** Buffers the user marked "Keep Open"; bulk closes spare them. */
  pinnedIds: string[];
  workspaceRoot: string | null;
  workspaceName: string | null;
  fileTree: FileTreeNode | null;
  /** P2 gap 3: buffer id → path for tabs with an unresolved disk conflict. */
  conflicts: Record<string, string>;
};

type Listener = () => void;

const listeners = new Set<Listener>();
let localActiveId: string | null = null;
let revealTarget: RevealTarget | null = null;
let revealSequence = 0;
let appSnapshot = getAppSnapshot();
let protocolActiveId: string | null = appSnapshot.bootstrap?.active_buffer_id ?? null;
let pendingOpenPath: string | null = null;
let knownBufferIds = new Set(
  (appSnapshot.bootstrap?.buffers ?? []).map((buffer) => buffer.buffer_id),
);
// Selections typed in the pane, keyed by buffer, recorded before the protocol
// echo lands. Kept outside the snapshot so a keystroke does not rebuild the app.
let liveSelections = new Map<string, Selection>();
// "Keep Open" is a local tab affordance: the protocol has no pin concept, so
// the pins survive snapshots and are pruned whenever a buffer disappears.
let pinnedIds = new Set<string>();
let state: EditorStoreState = deriveState(appSnapshot, localActiveId, revealTarget);
let appUnsubscribe: (() => void) | null = null;

function filenameOf(path: string): string {
  const segments = path.split(/[\\/]/);
  return segments[segments.length - 1] || path;
}

function pathsEqual(left: string, right: string): boolean {
  return left.replace(/\\/g, '/').toLowerCase() === right.replace(/\\/g, '/').toLowerCase();
}

function isEmptySelection(selection: Selection): boolean {
  return (
    selection.anchor.line === selection.active.line &&
    selection.anchor.utf16_column === selection.active.utf16_column
  );
}

function deriveState(
  app: ReturnType<typeof getAppSnapshot>,
  preferredActiveId: string | null,
  reveal: RevealTarget | null,
): EditorStoreState {
  const bootstrap: BootstrapState | null = app.bootstrap;
  const buffers = bootstrap?.buffers ?? [];
  const conflicts = app.bufferConflicts ?? {};
  const tabs: EditorTab[] = buffers.map((buffer) => ({
    id: buffer.buffer_id,
    path: buffer.path,
    filename: filenameOf(buffer.path),
    language: buffer.language,
    isDirty: buffer.dirty,
    externallyChanged: Object.prototype.hasOwnProperty.call(conflicts, buffer.buffer_id),
    version: buffer.version,
  }));

  let activeBufferId: string | null = null;
  if (preferredActiveId && buffers.some((buffer) => buffer.buffer_id === preferredActiveId)) {
    activeBufferId = preferredActiveId;
  } else if (
    bootstrap?.active_buffer_id &&
    buffers.some((buffer) => buffer.buffer_id === bootstrap.active_buffer_id)
  ) {
    activeBufferId = bootstrap.active_buffer_id;
  } else {
    activeBufferId = tabs[0]?.id ?? null;
  }

  const activeBuffer = buffers.find((buffer) => buffer.buffer_id === activeBufferId) ?? null;

  // The pane knows the caret before the protocol echo arrives, so a live
  // selection wins over the snapshot. Empty selections stay out of state.
  const protocolSelection = activeBuffer?.selection ?? null;
  const liveSelection = activeBufferId ? liveSelections.get(activeBufferId) ?? null : null;
  const candidate = liveSelection ?? protocolSelection;
  const activeSelection = candidate && !isEmptySelection(candidate) ? candidate : null;

  return {
    tabs,
    activeBufferId,
    activeBuffer,
    activeSelection,
    revealTarget: reveal,
    pinnedIds: tabs.filter((tab) => pinnedIds.has(tab.id)).map((tab) => tab.id),
    workspaceRoot: bootstrap?.workspace.root ?? null,
    workspaceName: bootstrap?.workspace.display_name ?? null,
    fileTree: bootstrap?.file_tree.root ?? null,
    conflicts,
  };
}

function notify(): void {
  for (const listener of listeners) listener();
}

function rebuild(): void {
  focusPendingBuffer();
  state = deriveState(appSnapshot, localActiveId, revealTarget);
  localActiveId = state.activeBufferId;
  notify();
}

function focusPendingBuffer(): void {
  const pending = pendingOpenPath;
  const buffers = appSnapshot.bootstrap?.buffers ?? [];
  if (pending) {
    const opened = buffers.find((buffer) => pathsEqual(buffer.path, pending));
    if (opened) {
      localActiveId = opened.buffer_id;
      pendingOpenPath = null;
    }
  }

  const nextProtocolActiveId = appSnapshot.bootstrap?.active_buffer_id ?? null;
  if (
    nextProtocolActiveId &&
    nextProtocolActiveId !== protocolActiveId &&
    buffers.some((buffer) => buffer.buffer_id === nextProtocolActiveId)
  ) {
    localActiveId = nextProtocolActiveId;
  }

  const added = buffers.filter((buffer) => !knownBufferIds.has(buffer.buffer_id));
  if (!pending && added.length === 1 && nextProtocolActiveId === protocolActiveId) {
    localActiveId = added[0].buffer_id;
  }
  protocolActiveId = nextProtocolActiveId;
  knownBufferIds = new Set(buffers.map((buffer) => buffer.buffer_id));
}

function syncFromApp(): void {
  const next = getAppSnapshot();
  if (next === appSnapshot) return;
  appSnapshot = next;
  focusPendingBuffer();
  state = deriveState(appSnapshot, localActiveId, revealTarget);
  localActiveId = state.activeBufferId;
}

function ensureAppSubscription(): void {
  if (appUnsubscribe) return;
  appUnsubscribe = subscribeApp(() => {
    const next = getAppSnapshot();
    if (next === appSnapshot) return;
    appSnapshot = next;
    rebuild();
  });
}

function openFileAt(path: string, context: EnvelopeContext): void {
  pendingOpenPath = path;
  void openFileCommand(path, context).catch(() => {
    if (pendingOpenPath === path) pendingOpenPath = null;
  });
}

function reportMissingContext(): void {
  toastStore.pushLocal('warning', 'Connect to a workspace before editing files.');
}

/**
 * Tell Rust which tab the user is looking at, so the next bootstrap's
 * `active_buffer_id` is the real selection and not path order. Fire and
 * forget: the pane already applied the switch locally; a failed ack leaves
 * the previous (still valid) selection.
 */
function recordActiveBuffer(bufferId: string): void {
  const context = currentEnvelopeContext();
  if (!context) return;
  void setActiveBufferCommand(bufferId, context).catch(() => undefined);
}

export function currentEditorContext(): EnvelopeContext | null {
  return currentEnvelopeContext();
}

/** Record the editor pane's live selection (before the protocol echo lands). */
export function recordEditorSelection(bufferId: string, selection: Selection): void {
  const previous = state.activeSelection;
  if (isEmptySelection(selection)) {
    liveSelections.delete(bufferId);
  } else {
    liveSelections.set(bufferId, selection);
  }
  state = deriveState(appSnapshot, localActiveId, revealTarget);
  if (state.activeSelection !== previous) notify();
}

/**
 * The active editor reference for the next agent turn, or null when no
 * buffer is active. Never includes file text: the agent reads it with
 * `read_file`. Empty selections are omitted, not sent as 0:0-0:0.
 */
export function activeEditorContext(): EditorContextDto | null {
  syncFromApp();
  const buffer = state.activeBuffer;
  if (!buffer || !buffer.path) return null;
  return {
    path: buffer.path,
    language: buffer.language,
    ...(state.activeSelection ? { selection: state.activeSelection } : {}),
    dirty: buffer.dirty,
  };
}

export const editorStore = {
  subscribe(listener: Listener): () => void {
    ensureAppSubscription();
    listeners.add(listener);
    syncFromApp();
    return () => {
      listeners.delete(listener);
    };
  },

  getSnapshot(): EditorStoreState {
    syncFromApp();
    return state;
  },

  /** Rebuild from the current bootstrap (rebootstrap / test isolation). */
  reset(): void {
    localActiveId = null;
    revealTarget = null;
    pendingOpenPath = null;
    liveSelections.clear();
    pinnedIds = new Set();
    appSnapshot = getAppSnapshot();
    protocolActiveId = appSnapshot.bootstrap?.active_buffer_id ?? null;
    knownBufferIds = new Set(
      (appSnapshot.bootstrap?.buffers ?? []).map((buffer) => buffer.buffer_id),
    );
    state = deriveState(appSnapshot, localActiveId, revealTarget);
    localActiveId = state.activeBufferId;
    notify();
  },

  setActive(id: string): void {
    syncFromApp();
    if (!state.tabs.some((tab) => tab.id === id)) return;
    if (localActiveId === id) return;
    localActiveId = id;
    recordActiveBuffer(id);
    rebuild();
  },

  async openPath(path: string): Promise<void> {
    syncFromApp();
    const existing = state.tabs.find((tab) => pathsEqual(tab.path, path));
    if (existing) {
      editorStore.setActive(existing.id);
      return;
    }
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return;
    }
    pendingOpenPath = path;
    await openFileCommand(path, context).catch(() => {
      if (pendingOpenPath === path) pendingOpenPath = null;
    });
  },

  async openPathAtLine(path: string, line: number): Promise<void> {
    syncFromApp();
    const existing = state.tabs.find((tab) => pathsEqual(tab.path, path));
    if (existing) {
      localActiveId = existing.id;
      recordActiveBuffer(existing.id);
    } else {
      const context = currentEditorContext();
      if (!context) {
        reportMissingContext();
        return;
      }
      openFileAt(path, context);
    }
    revealSequence += 1;
    revealTarget = { path, line, requestId: revealSequence };
    rebuild();
  },

  clearReveal(requestId?: number): void {
    if (!revealTarget) return;
    if (requestId !== undefined && revealTarget.requestId !== requestId) return;
    revealTarget = null;
    rebuild();
  },

  closeBuffer(id: string): void {
    syncFromApp();
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return;
    }
    void closeBufferCommand(id, context).catch(() => undefined);
  },

  /**
   * Close several buffers at once, skipping dirty and "kept open" tabs so no
   * unsaved edit is ever discarded silently. Returns how many closes were
   * dispatched and how many requested tabs were spared.
   */
  closeMany(ids: string[]): { closed: number; kept: number } {
    syncFromApp();
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return { closed: 0, kept: 0 };
    }
    const requested = new Set(ids);
    const matches = state.tabs.filter((tab) => requested.has(tab.id));
    const targets = matches.filter((tab) => !tab.isDirty && !pinnedIds.has(tab.id));
    for (const tab of targets) {
      void closeBufferCommand(tab.id, context).catch(() => undefined);
    }
    return { closed: targets.length, kept: matches.length - targets.length };
  },

  /** Toggle the local "Keep Open" pin for one tab. */
  togglePinned(id: string): void {
    syncFromApp();
    if (!state.tabs.some((tab) => tab.id === id)) return;
    if (pinnedIds.has(id)) pinnedIds.delete(id);
    else pinnedIds.add(id);
    rebuild();
  },

  /**
   * Save one buffer (which may not be active) and resolve `true` only once the
   * protocol acknowledges the save. The tab bar uses this to gate closing a
   * dirty tab: a rejected save must leave the buffer open.
   */
  async saveBuffer(id: string): Promise<boolean> {
    syncFromApp();
    if (!state.tabs.some((tab) => tab.id === id)) return false;
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return false;
    }
    try {
      await saveBufferCommand(id, context);
      return true;
    } catch {
      return false;
    }
  },

  saveActive(): void {
    syncFromApp();
    const active = state.activeBuffer;
    if (!active) return;
    void editorStore.saveBuffer(active.buffer_id);
  },

  /** P2 gap 3: take the disk text, discarding the kept local edits. */
  reloadFromDisk(id: string): void {
    syncFromApp();
    if (!state.tabs.some((tab) => tab.id === id)) return;
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return;
    }
    void reloadBufferFromDiskCommand(id, context).catch(() => undefined);
  },

  /** P2 gap 3: keep the local text and clear the external-change marker. */
  keepMyEdits(id: string): void {
    syncFromApp();
    if (!state.tabs.some((tab) => tab.id === id)) return;
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return;
    }
    void keepBufferEditsCommand(id, context).catch(() => undefined);
  },
};
