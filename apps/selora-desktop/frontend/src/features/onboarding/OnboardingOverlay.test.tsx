import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { OnboardingOverlay } from './OnboardingOverlay';
import { onboardingStore } from './onboarding-store';

function mockMatchMedia(matches: boolean) {
  vi.stubGlobal(
    'matchMedia',
    vi.fn().mockImplementation((query: string) => ({
      matches,
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(() => false),
    })),
  );
}

describe('OnboardingOverlay', () => {
  beforeEach(() => {
    window.localStorage.clear();
    onboardingStore.resetForTests();
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    window.localStorage.clear();
    onboardingStore.resetForTests();
  });

  it('walks three steps and finishes into a persisted dismissal', () => {
    render(<OnboardingOverlay />);

    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Selora IDE' })).toBeTruthy();
    expect(screen.getByText('STEP 1 / 3')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Next' }));
    expect(screen.getByRole('heading', { name: 'Your whole workspace, in view.' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Next' }));
    expect(screen.getByRole('heading', { name: 'Agent on your terms.' })).toBeTruthy();
    expect(screen.getByText('STEP 3 / 3')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Finish' }));

    expect(screen.queryByRole('dialog')).toBeNull();
    expect(onboardingStore.shouldShow()).toBe(false);
    expect(window.localStorage.getItem('selora:onboarding_seen')).toBe('true');
  });

  it('skips from any step', () => {
    render(<OnboardingOverlay />);
    fireEvent.click(screen.getByRole('button', { name: 'Next' }));
    fireEvent.click(screen.getByRole('button', { name: 'Skip' }));

    expect(screen.queryByRole('dialog')).toBeNull();
    expect(onboardingStore.shouldShow()).toBe(false);
  });

  it('closes on Escape', () => {
    render(<OnboardingOverlay />);
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });

    expect(screen.queryByRole('dialog')).toBeNull();
    expect(onboardingStore.shouldShow()).toBe(false);
  });

  it('Escape restores focus to the opener', () => {
    const opener = document.createElement('button');
    opener.textContent = 'Start tour';
    document.body.appendChild(opener);
    opener.focus();

    render(<OnboardingOverlay />);
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Next' }));

    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });

    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it('traps Tab focus inside the dialog', () => {
    render(<OnboardingOverlay />);
    const dialog = screen.getByRole('dialog');
    const next = screen.getByRole('button', { name: 'Next' });
    const skip = screen.getByRole('button', { name: 'Skip' });

    expect(document.activeElement).toBe(next);
    fireEvent.keyDown(dialog, { key: 'Tab' });
    expect(document.activeElement).toBe(skip);

    fireEvent.keyDown(dialog, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(next);
  });

  it('restores focus to the opener when dismissed', () => {
    const opener = document.createElement('button');
    opener.textContent = 'Open';
    document.body.appendChild(opener);
    opener.focus();

    render(<OnboardingOverlay />);
    fireEvent.click(screen.getByRole('button', { name: 'Skip' }));

    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it('marks the dialog reduced-motion when the OS preference is active', () => {
    mockMatchMedia(true);
    render(<OnboardingOverlay />);

    const dialog = screen.getByRole('dialog');
    expect(dialog.getAttribute('data-reduced-motion')).toBe('true');
    expect(dialog.classList.contains('is-reduced-motion')).toBe(true);
  });

  it('renders the CSS brand fallback when WebGL is unavailable', () => {
    render(<OnboardingOverlay />);

    expect(document.querySelector('[data-visual="fallback"]')).toBeTruthy();
  });

  it('renders nothing when the tour was already seen', () => {
    onboardingStore.complete();
    render(<OnboardingOverlay />);

    expect(screen.queryByRole('dialog')).toBeNull();
  });
});
