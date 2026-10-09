// Reads a numeric value (px or ms) out of the designer-owned token layer.
// A missing or unparsable token yields null — the caller reports the failure
// rather than substituting a hardcoded literal (DESIGN §13.4).
export function readTokenNumber(name: string): number | null {
  if (typeof document === 'undefined') return null;
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name);
  const value = Number.parseFloat(raw);
  return Number.isFinite(value) ? value : null;
}
