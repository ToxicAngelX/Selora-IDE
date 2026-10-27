// Selora IDE — Phase Z5b first-run onboarding store.
//
// A deliberately small external store (the same shape as toast-store): the
// overlay subscribes with useSyncExternalStore, while the seen flag persists
// across launches. Only a completed or skipped tour marks the flag; a storage
// failure leaves the tour available in memory instead of blocking the app.

export const ONBOARDING_SEEN_KEY = 'selora:onboarding_seen';

export type OnboardingState = {
  seen: boolean;
};

type Listener = () => void;

const listeners = new Set<Listener>();

function getStorage(): Storage | null {
  if (typeof window === 'undefined') return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function readSeen(): boolean {
  try {
    return getStorage()?.getItem(ONBOARDING_SEEN_KEY) === 'true';
  } catch {
    return false;
  }
}

function persistSeen(): void {
  try {
    getStorage()?.setItem(ONBOARDING_SEEN_KEY, 'true');
  } catch {
    // Storage can be unavailable or full; the in-memory dismissal still works.
  }
}

let snapshot: OnboardingState = { seen: readSeen() };

function notify(): void {
  for (const listener of listeners) listener();
}

function markSeen(): void {
  if (snapshot.seen) return;
  persistSeen();
  snapshot = { seen: true };
  notify();
}

export const onboardingStore = {
  subscribe(listener: Listener): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },

  getSnapshot(): OnboardingState {
    return snapshot;
  },

  shouldShow(): boolean {
    return !snapshot.seen;
  },

  complete(): void {
    markSeen();
  },

  skip(): void {
    markSeen();
  },

  /** Test seam: clears the persisted flag and re-arms the overlay. */
  resetForTests(): void {
    try {
      getStorage()?.removeItem(ONBOARDING_SEEN_KEY);
    } catch {
      // Ignore unavailable storage; the in-memory state still resets.
    }
    snapshot = { seen: false };
    notify();
  },
};
