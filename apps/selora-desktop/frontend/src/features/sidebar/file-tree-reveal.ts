// Selora IDE — one-shot request that the Files tree reveal a path.
// The tree only mounts while the Files activity view is active, so the
// request is queued and consumed on mount.

export type FileTreeRevealRequest = {
  path: string;
  requestId: number;
};

let pending: FileTreeRevealRequest | null = null;
let nextRequestId = 0;
const listeners = new Set<(request: FileTreeRevealRequest) => void>();

/** Queue a reveal and notify a mounted tree immediately. */
export function requestFileTreeReveal(path: string): void {
  nextRequestId += 1;
  const request = { path, requestId: nextRequestId };
  pending = request;
  for (const listener of listeners) listener(request);
}

/** Take the queued request, if any (called when the tree mounts). */
export function consumeFileTreeReveal(): FileTreeRevealRequest | null {
  const request = pending;
  pending = null;
  return request;
}

export function subscribeFileTreeReveal(
  listener: (request: FileTreeRevealRequest) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}