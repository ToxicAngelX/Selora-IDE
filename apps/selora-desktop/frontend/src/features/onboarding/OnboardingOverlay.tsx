// Selora IDE — Phase Z5b first-run onboarding overlay.
//
// Three short steps introduce the real merged shell: the workspace tree, the
// CodeMirror editing surface, and the approval-gated agent. The dialog follows
// the same modal contract as the cheat sheet: scrim + blur backdrop, focus
// trap, Escape, and focus restoration. It disappears permanently after Finish
// or Skip (persisted by onboarding-store).
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { BrandMark, OnboardingHero } from './BrandFx';
import { useReducedMotion } from './motion';
import { onboardingStore } from './onboarding-store';

const STEPS = [
  {
    short: 'Welcome',
    eyebrow: 'WELCOME',
    title: 'Selora IDE',
    body: 'The editor born connected. Open a folder and work with a real file tree, editor, and language intelligence in one surface.',
  },
  {
    short: 'Workspace',
    eyebrow: 'WORKSPACE',
    title: 'Your whole workspace, in view.',
    body: 'Browse files, search the tree, review Git changes, and jump to diagnostics without leaving the editor.',
  },
  {
    short: 'Agent',
    eyebrow: 'AGENT',
    title: 'Agent on your terms.',
    body: 'Every agent tool call is visible. Writes wait for your approval, and you can stop a turn at any point.',
  },
] as const;

export function OnboardingOverlay() {
  const state = useSyncExternalStore(onboardingStore.subscribe, onboardingStore.getSnapshot);
  const [step, setStep] = useState(0);
  const reducedMotion = useReducedMotion();
  const dialogRef = useRef<HTMLElement>(null);
  const nextRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  const visible = !state.seen;

  useEffect(() => {
    if (!visible) return;
    restoreFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    nextRef.current?.focus();
    return () => {
      const restore = restoreFocusRef.current;
      restoreFocusRef.current = null;
      if (!restore || !restore.isConnected) return;
      const active = document.activeElement;
      const dialogHadFocus = active === document.body || (dialogRef.current?.contains(active) ?? false);
      if (dialogHadFocus) restore.focus();
    };
  }, [visible]);

  if (!visible) return null;

  const isLast = step === STEPS.length - 1;
  const current = STEPS[step];

  const close = () => onboardingStore.skip();
  const finish = () => onboardingStore.complete();

  const handleKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusables = dialogRef.current?.querySelectorAll<HTMLElement>(
      'button:not([disabled]), a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
    );
    if (!focusables || focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div className="onboarding-backdrop" data-testid="onboarding-backdrop">
      <section
        ref={dialogRef}
        className={`onboarding-dialog${reducedMotion ? ' is-reduced-motion' : ''}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby="onboarding-title"
        aria-describedby="onboarding-body"
        data-reduced-motion={reducedMotion ? 'true' : 'false'}
        tabIndex={-1}
        onKeyDown={handleKeyDown}
      >
        <div className="onboarding-visual">
          <OnboardingHero />
          <div className="onboarding-visual-meta">
            <span className="onboarding-visual-step">STEP {step + 1} / {STEPS.length}</span>
            <span className="onboarding-visual-rule" aria-hidden="true" />
          </div>
        </div>

        <div className="onboarding-content">
          <div className="onboarding-topline">
            <BrandMark className="onboarding-content-mark" />
            <button type="button" className="onboarding-skip" onClick={close}>
              Skip
            </button>
          </div>

          <p className="onboarding-eyebrow">{current.eyebrow}</p>
          <h1 id="onboarding-title" className="onboarding-title">
            {current.title}
          </h1>
          <p id="onboarding-body" className="onboarding-body">
            {current.body}
          </p>

          <div className="onboarding-dots" aria-label="Onboarding progress">
            {STEPS.map((item, index) => (
              <button
                key={item.short}
                type="button"
                className={`onboarding-dot${index === step ? ' is-active' : ''}`}
                aria-label={`Go to step ${index + 1}: ${item.short}`}
                aria-current={index === step ? 'step' : undefined}
                onClick={() => setStep(index)}
              >
                <span className="onboarding-dot-fill" />
              </button>
            ))}
          </div>

          <footer className="onboarding-actions">
            <button
              type="button"
              className="onboarding-button onboarding-button-quiet"
              onClick={() => setStep((value) => Math.max(0, value - 1))}
              disabled={step === 0}
            >
              Back
            </button>
            <button
              ref={nextRef}
              type="button"
              className="onboarding-button onboarding-button-primary"
              onClick={() => (isLast ? finish() : setStep((value) => value + 1))}
            >
              {isLast ? 'Finish' : 'Next'}
            </button>
          </footer>
          <p className="onboarding-hint">Press Esc to skip.</p>
        </div>
      </section>
    </div>
  );
}
