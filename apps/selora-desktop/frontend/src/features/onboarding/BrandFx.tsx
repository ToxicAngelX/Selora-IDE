// Selora IDE — Phase Z5b brand surfaces.
//
// The shipped brand visuals are pure CSS. The earlier optional `img-fx` /
// `metal-fx` WebGL enhancements are intentionally not loaded: on WebView2 the
// hero shader failed to compile, lost its context, and blocked the renderer
// thread (the first-run overlay became unresponsive). A capability probe is not
// enough because context creation succeeds before the shader hangs the page.
// The CSS orbital mark is the supported implementation on every platform.
import type { ReactNode } from 'react';

export function BrandMark({ className = '' }: { className?: string }): ReactNode {
  return (
    <span className={`onboarding-brand-mark ${className}`.trim()} aria-hidden="true">
      <span className="onboarding-brand-core" />
      <span className="onboarding-brand-satellite" />
    </span>
  );
}

function HeroFallback() {
  return (
    <div className="onboarding-hero-fallback" data-visual="fallback" aria-hidden="true">
      <span className="onboarding-hero-grid" />
      <BrandMark className="onboarding-hero-fallback-mark" />
      <span className="onboarding-hero-fallback-label">Workspace preview</span>
    </div>
  );
}

export function OnboardingHero() {
  return <HeroFallback />;
}

export function MetalBrand({ size = 'about' }: { size?: 'about' | 'onboarding' }) {
  return <BrandMark className={`onboarding-metal-fallback onboarding-metal-${size}`} />;
}