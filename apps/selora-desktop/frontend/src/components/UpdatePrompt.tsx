// Selora IDE — startup update prompt (UI_SPEC §11).
//
// The shell checks GitHub Releases once per launch when the user has not
// turned the automatic check off in Settings. A newer release opens this
// centered dialog: "Yes" downloads, verifies and runs the published installer
// (the Inno Setup AppId upgrades the per-user install in place), "No" closes
// it until the next launch. A failed check is silent - a launch must never
// start with a network error dialog.
import { useEffect, useRef, useState } from 'react';
import {
  checkForUpdate,
  installUpdate,
  listenUpdateProgress,
  openExternal,
  type UpdateCheck,
  type UpdateProgress,
} from '../bridge/bridge';
import { currentEnvelopeContext } from '../app/app-store';
import { toastStore } from '../features/toasts/toast-store';

export type UpdatePromptProps = {
  /** `settings.check_for_updates`; false skips the automatic check. */
  autoCheck: boolean;
};

export function UpdatePrompt({ autoCheck }: UpdatePromptProps) {
  const [release, setRelease] = useState<UpdateCheck | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const checkedRef = useRef(false);
  const primaryRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (!autoCheck || checkedRef.current) return;
    checkedRef.current = true;
    let cancelled = false;
    void checkForUpdate()
      .then((result) => {
        if (!cancelled && result.newer) setRelease(result);
      })
      .catch(() => {
        // Silent by design: the About dialog still offers a manual check.
      });
    return () => {
      cancelled = true;
    };
  }, [autoCheck]);

  useEffect(() => {
    if (!release) return;
    primaryRef.current?.focus();
  }, [release]);

  useEffect(
    () => () => {
      unlistenRef.current?.();
    },
    [],
  );

  if (!release) return null;

  const dismiss = (): void => {
    if (installing) return;
    setRelease(null);
    setError(null);
  };

  const startInstall = async (): Promise<void> => {
    setInstalling(true);
    setError(null);
    setProgress(null);
    try {
      unlistenRef.current = await listenUpdateProgress(setProgress);
      await installUpdate();
      // The shell exits here so the installer can replace it.
    } catch (cause) {
      setInstalling(false);
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message);
      toastStore.pushLocal('error', `Update failed: ${message}`);
    }
  };

  const openReleasePage = (): void => {
    const context = currentEnvelopeContext();
    if (!context) return;
    void openExternal(release.release_url, context).catch(() => undefined);
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>): void => {
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      dismiss();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusables = dialogRef.current?.querySelectorAll<HTMLElement>(
      'button:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])',
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

  const percent =
    progress && progress.total > 0
      ? Math.min(100, Math.floor((progress.downloaded / progress.total) * 100))
      : null;

  return (
    <div className="update-backdrop" data-testid="update-backdrop">
      <div
        ref={dialogRef}
        className="update-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="update-title"
        aria-describedby="update-body"
        onKeyDown={handleKeyDown}
      >
        <p className="update-eyebrow">Update available</p>
        <h2 id="update-title" className="update-title">
          Selora IDE {release.latest_tag}
        </h2>
        <p id="update-body" className="update-body">
          You are on v{release.current_version}.{' '}
          {release.installer_available
            ? 'Download the latest version now? It installs over the current one — your settings, workspace and sessions stay.'
            : 'This release has no in-app installer attached; open its release page to download it.'}
        </p>

        <div className="update-feedback" aria-live="polite">
          {error ? (
            <p className="update-error" role="alert">
              {error}
            </p>
          ) : installing ? (
            <p className="update-progress" data-testid="update-progress">
              {percent === null ? 'Downloading and verifying…' : `Downloading ${percent}%…`}
            </p>
          ) : null}
        </div>

        <div className="update-actions">
          {release.installer_available ? (
            <button
              ref={primaryRef}
              type="button"
              className="about-button about-button-primary"
              onClick={() => void startInstall()}
              disabled={installing}
            >
              {installing ? 'Installing…' : 'Yes, update'}
            </button>
          ) : (
            <button
              ref={primaryRef}
              type="button"
              className="about-button about-button-primary"
              onClick={openReleasePage}
            >
              Open release page
            </button>
          )}
          <button type="button" className="about-button" onClick={dismiss} disabled={installing}>
            No, later
          </button>
        </div>
      </div>
    </div>
  );
}