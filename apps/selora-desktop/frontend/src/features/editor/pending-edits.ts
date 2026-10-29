import type { BootstrapState } from '../../bridge/bridge';

const pending = new Map<string, { workspaceId: string; path: string; version: number }>();
const normalize = (path: string) => path.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();

/** Mark local edits before IPC so destructive Explorer actions cannot beat their echo. */
export function markPendingEditorEdit(bufferId: string, workspaceId: string, path: string, version: number): void {
  pending.set(bufferId, { workspaceId, path, version });
}

export function hasPendingEditorEdits(bootstrap: BootstrapState, path: string, directory: boolean): boolean {
  const target = normalize(path);
  for (const [id, entry] of pending) {
    if (entry.workspaceId !== bootstrap.workspace.id) continue;
    const buffer = bootstrap.buffers.find((candidate) => candidate.buffer_id === id);
    if (!buffer || buffer.version >= entry.version) {
      pending.delete(id);
      continue;
    }
    const candidate = normalize(entry.path);
    if (candidate === target || (directory && candidate.startsWith(target + '/'))) return true;
  }
  return false;
}
