import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ONBOARDING_SEEN_KEY, onboardingStore } from './onboarding-store';

describe('onboardingStore', () => {
  beforeEach(() => {
    window.localStorage.clear();
    onboardingStore.resetForTests();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it('shows the tour on a first run and persists completion', () => {
    expect(onboardingStore.shouldShow()).toBe(true);

    onboardingStore.complete();

    expect(onboardingStore.shouldShow()).toBe(false);
    expect(onboardingStore.getSnapshot()).toEqual({ seen: true });
    expect(window.localStorage.getItem(ONBOARDING_SEEN_KEY)).toBe('true');
  });

  it('treats skip as a permanent dismissal', () => {
    onboardingStore.skip();

    expect(onboardingStore.shouldShow()).toBe(false);
    expect(window.localStorage.getItem(ONBOARDING_SEEN_KEY)).toBe('true');
  });

  it('notifies subscribers once per dismissal', () => {
    const listener = vi.fn();
    const unsubscribe = onboardingStore.subscribe(listener);

    onboardingStore.complete();
    onboardingStore.complete();
    unsubscribe();

    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('still dismisses in memory when storage writes fail', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota exceeded');
    });

    expect(() => onboardingStore.complete()).not.toThrow();
    expect(onboardingStore.shouldShow()).toBe(false);
  });

  it('re-arms from the test seam', () => {
    onboardingStore.complete();
    onboardingStore.resetForTests();

    expect(onboardingStore.shouldShow()).toBe(true);
    expect(window.localStorage.getItem(ONBOARDING_SEEN_KEY)).toBeNull();
  });
});
