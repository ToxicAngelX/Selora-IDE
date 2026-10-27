// Selora IDE — Phase Z5b WebGL capability probe.
//
// The visual packages are optional enhancements: if a canvas cannot create a
// context (happy-dom, a GPU-disabled WebView, a driver blocklist), the caller
// renders the token-only fallback instead of mounting the effect.

function canGetContext(kind: 'webgl' | 'webgl2'): boolean {
  if (typeof document === 'undefined') return false;
  try {
    const canvas = document.createElement('canvas');
    return canvas.getContext(kind) !== null;
  } catch {
    return false;
  }
}

export function hasWebGL(): boolean {
  return canGetContext('webgl') || canGetContext('webgl2');
}

export function hasWebGL2(): boolean {
  return canGetContext('webgl2');
}
