// Selora IDE — tab-strip clipboard helpers.
//
// The protocol addresses buffers with workspace-relative, forward-slash paths
// (`WorkspacePath`). These helpers turn one into the three path flavours the
// tab context menu offers and keep the clipboard/toast plumbing out of the
// tab bar component.
import { toastStore } from '../toasts/toast-store';

function normalized(path: string): string {
  return path.replace(/\\/g, '/').replace(/\/+$/, '');
}

function isAbsolute(path: string): boolean {
  return /^[a-zA-Z]:\//.test(path) || path.startsWith('//');
}

/** Join the workspace root and a relative buffer path. */
export function absolutePathFor(root: string | null, path: string): string {
  const target = normalized(path);
  if (isAbsolute(target)) return target;
  if (!root) return target;
  return `${normalized(root)}/${target}`;
}

/** Workspace-relative path in the platform's separator style. */
export function relativePathFor(path: string): string {
  const target = normalized(path);
  if (usesBackslashes()) return target.replace(/\//g, '\\');
  return target;
}

/** The path Selora's breadcrumbs show: workspace label plus relative path. */
export function breadcrumbsPathFor(
  workspaceName: string | null,
  root: string | null,
  path: string,
): string {
  const target = normalized(path);
  const label =
    (workspaceName && workspaceName.trim()) ||
    (root ? normalized(root).split('/').pop() : null) ||
    '';
  const joined = label && !target.startsWith(`${normalized(label)}/`) ? `${label}/${target}` : target;
  return usesBackslashes() ? joined.replace(/\//g, '\\') : joined;
}

function usesBackslashes(): boolean {
  if (typeof navigator === 'undefined') return false;
  return /windows/i.test(navigator.userAgent);
}

/**
 * Copy text and announce the result. Clipboard failures are surfaced instead of
 * thrown so a menu click never leaves a dangling unhandled rejection.
 */
export async function copyTextToClipboard(text: string, label: string): Promise<void> {
  try {
    if (!navigator.clipboard?.writeText) throw new Error('Clipboard unavailable');
    await navigator.clipboard.writeText(text);
    toastStore.pushLocal('info', `Copied ${label}`);
  } catch {
    toastStore.pushLocal('error', `Could not copy ${label}`);
  }
}