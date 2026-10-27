import { useState } from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ABOUT_EVENT,
  AboutDialog,
  checkForUpdate,
  isNewerVersion,
  LATEST_RELEASE_URL,
  REPOSITORY_URL,
  repositorySlug,
  useAboutTrigger,
} from './AboutDialog';
import { toastStore } from '../features/toasts/toast-store';

const bridgeMock = vi.hoisted(() => ({ openExternal: vi.fn(async () => undefined) }));
const contextMock = vi.hoisted(() =>
  vi.fn(() => ({ core_instance_id: 'core-1', workspace_id: 'ws-1', session_id: null })),
);

vi.mock('../bridge/bridge', () => ({ openExternal: bridgeMock.openExternal }));
vi.mock('../app/app-store', () => ({ currentEnvelopeContext: contextMock }));

function releaseResponse(tag: string) {
  return {
    ok: true,
    status: 200,
    json: async () => ({
      tag_name: tag,
      html_url: `https://github.com/ToxicAngelX/Selora-IDE/releases/tag/${tag}`,
    }),
  };
}

describe('AboutDialog', () => {
  it('derives the release endpoint from the package repository URL', () => {
    expect(REPOSITORY_URL).toMatch(/^https:\/\/github\.com\/[^/]+\/[^/]+$/);
    expect(repositorySlug()).toBe(REPOSITORY_URL.replace(/^https:\/\/github\.com\//, ''));
    expect(LATEST_RELEASE_URL).toBe(
      `https://api.github.com/repos/${repositorySlug(REPOSITORY_URL)}/releases/latest`,
    );
    expect(repositorySlug('https://github.com/a/b.git')).toBe('a/b');
  });

  beforeEach(() => {
    bridgeMock.openExternal.mockClear();
    contextMock.mockClear();
    toastStore.clearAll();
    toastStore.clearHistory();
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    toastStore.clearAll();
    toastStore.clearHistory();
  });

  it('opens from the global selora:about event', () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      useAboutTrigger(() => setOpen(true));
      return <AboutDialog open={open} onClose={() => setOpen(false)} />;
    }

    render(<Harness />);
    expect(screen.queryByRole('dialog')).toBeNull();

    act(() => {
      window.dispatchEvent(new CustomEvent(ABOUT_EVENT));
    });

    expect(screen.getByRole('dialog', { name: 'Selora IDE' })).toBeTruthy();
  });

  it('Escape closes the dialog and restores focus to the opener', () => {
    const launcher = document.createElement('button');
    launcher.textContent = 'About launcher';
    document.body.appendChild(launcher);
    launcher.focus();

    const onClose = vi.fn();
    function Harness() {
      const [open, setOpen] = useState(true);
      return (
        <AboutDialog
          open={open}
          onClose={() => {
            onClose();
            setOpen(false);
          }}
        />
      );
    }

    render(<Harness />);
    const dialog = screen.getByRole('dialog', { name: 'Selora IDE' });
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Close' }));
    expect(dialog.getAttribute('aria-modal')).toBe('true');

    fireEvent.keyDown(dialog, { key: 'Escape' });

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(launcher);
    launcher.remove();
  });

  it('renders the package version, license, and repository', () => {
    render(<AboutDialog open onClose={vi.fn()} />);

    expect(screen.getByText('Lite Edition · v0.1.0')).toBeTruthy();
    expect(screen.getByText('Open source · MIT')).toBeTruthy();
    expect(screen.getByRole('link', { name: 'github.com/ToxicAngelX/Selora-IDE' })).toBeTruthy();
  });

  it('opens the repository through the allowlist instead of navigating the webview', () => {
    render(<AboutDialog open onClose={() => {}} />);
    const link = screen.getByRole('link', { name: /github\.com\/ToxicAngelX\/Selora-IDE/i });
    const navigated = fireEvent.click(link);
    expect(navigated).toBe(false);
    expect(bridgeMock.openExternal).toHaveBeenCalledWith(REPOSITORY_URL, {
      core_instance_id: 'core-1',
      workspace_id: 'ws-1',
      session_id: null,
    });
  });

  it('reports a newer release from the real Releases path', async () => {
    const fetchMock = vi.fn().mockResolvedValue(releaseResponse('v9.9.9'));
    vi.stubGlobal('fetch', fetchMock);
    render(<AboutDialog open onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));

    expect(await screen.findByText('Version v9.9.9 is available.')).toBeTruthy();
    expect(fetchMock).toHaveBeenCalledWith(LATEST_RELEASE_URL, {
      headers: { Accept: 'application/vnd.github+json' },
    });
    expect(toastStore.getSnapshot().history[0]?.message).toBe('Selora IDE v9.9.9 is available.');
  });

  it('reports the current release as up to date', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(releaseResponse('v0.1.0')));
    render(<AboutDialog open onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));

    expect(await screen.findByText('You are on the latest release (v0.1.0).')).toBeTruthy();
    expect(toastStore.getSnapshot().history[0]?.severity).toBe('success');
  });

  it('shows an explicit error state and error toast on failure', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: false, status: 500, json: async () => ({}) }),
    );
    render(<AboutDialog open onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));

    expect(await screen.findByRole('alert')).toHaveProperty(
      'textContent',
      'Update check failed: GitHub Releases answered 500',
    );
    expect(toastStore.getSnapshot().history[0]?.severity).toBe('error');
  });
});

describe('update version comparison', () => {
  it('compares numeric triples with and without a v prefix', () => {
    expect(isNewerVersion('v0.2.0', '0.1.0')).toBe(true);
    expect(isNewerVersion('v1.0.0', '0.9.9')).toBe(true);
    expect(isNewerVersion('v0.1.0', '0.1.0')).toBe(false);
    expect(isNewerVersion('v0.0.9', '0.1.0')).toBe(false);
    expect(isNewerVersion('nightly', '0.1.0')).toBe(false);
  });

  it('rejects an unreadable payload', async () => {
    await expect(
      checkForUpdate('0.1.0', vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({}) })),
    ).rejects.toThrow('Release payload is missing tag_name or html_url');
  });
});
