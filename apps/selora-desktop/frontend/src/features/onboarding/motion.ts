// Selora IDE — Phase Z5b reduced-motion reader for imperative surfaces.
//
// CSS already freezes loops through base.css; canvas/WebGL surfaces need the
// same answer in JavaScript so they can mount paused rather than animate.
// Both the OS preference and the shell's data-reduced-motion flag count.
import { useEffect, useState } from 'react';

export const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)';

function inAppReducedMotion(): boolean {
  if (typeof document === 'undefined') return false;
  return document.documentElement.getAttribute('data-reduced-motion') === 'true';
}

function readReducedMotion(): boolean {
  if (inAppReducedMotion()) return true;
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false;
  try {
    return window.matchMedia(REDUCED_MOTION_QUERY).matches;
  } catch {
    return false;
  }
}

export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(readReducedMotion);

  useEffect(() => {
    const sync = () => setReduced(readReducedMotion());
    sync();

    let media: MediaQueryList | null = null;
    try {
      media = window.matchMedia(REDUCED_MOTION_QUERY);
      media.addEventListener?.('change', sync);
    } catch {
      media = null;
    }

    const observer =
      typeof MutationObserver === 'undefined'
        ? null
        : new MutationObserver(sync);
    observer?.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-reduced-motion'],
    });

    return () => {
      media?.removeEventListener?.('change', sync);
      observer?.disconnect();
    };
  }, []);

  return reduced;
}
