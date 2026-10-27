// The shipped brand surfaces must stay CSS-only. A WebGL hero previously
// failed shader compilation under WebView2 and blocked the renderer thread.
import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { BrandMark, MetalBrand, OnboardingHero } from './BrandFx';

describe('BrandFx', () => {
  it('renders the onboarding hero without a WebGL canvas', () => {
    const { container } = render(<OnboardingHero />);

    expect(container.querySelector('[data-visual="fallback"]')).toBeTruthy();
    expect(container.querySelector('canvas')).toBeNull();
    expect(container.textContent).toContain('Workspace preview');
  });

  it('renders the metal brand as the CSS orbital mark', () => {
    const { container } = render(<MetalBrand size="about" />);

    expect(container.querySelector('canvas')).toBeNull();
    expect(container.querySelector('.onboarding-metal-fallback')).toBeTruthy();
  });

  it('keeps the shared brand mark aria-hidden', () => {
    const { container } = render(<BrandMark className="probe" />);
    const mark = container.querySelector('.onboarding-brand-mark.probe');

    expect(mark).toBeTruthy();
    expect(mark?.getAttribute('aria-hidden')).toBe('true');
  });
});