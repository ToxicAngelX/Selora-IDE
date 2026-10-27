// ConnectModal dialog contract and accessibility coverage (Z6 follow-up).
import { useState } from 'react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ApiKeyStatus } from '../../bridge/bridge';
import { makeBootstrap } from '../../testing/fixtures';

const bridgeMocks = vi.hoisted(() => ({
  getApiKeyStatus: vi.fn(),
  connectApiKey: vi.fn(),
  disconnectApiKey: vi.fn(),
  openExternal: vi.fn(),
}));

const appStoreMocks = vi.hoisted(() => ({
  currentEnvelopeContext: vi.fn(() => ({
    core_instance_id: 'core-1',
    workspace_id: 'ws-1',
    session_id: null,
  })),
}));

vi.mock('../../bridge/bridge', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../bridge/bridge')>();
  return {
    ...actual,
    getApiKeyStatus: bridgeMocks.getApiKeyStatus,
    connectApiKey: bridgeMocks.connectApiKey,
    disconnectApiKey: bridgeMocks.disconnectApiKey,
    openExternal: bridgeMocks.openExternal,
  };
});

vi.mock('../../app/app-store', () => ({
  refreshBootstrap: vi.fn().mockResolvedValue(null),
  currentEnvelopeContext: appStoreMocks.currentEnvelopeContext,
}));

import { ConnectModal, PROVIDER_PRESETS } from './ConnectModal';

const STATUS: ApiKeyStatus = {
  connected: false,
  has_key: false,
  source: 'none',
  base_url: '',
  key_hint: null,
  active_model_id: null,
};

const FOCUSABLE_SELECTOR =
  'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

async function flushStatusEffect() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe('ConnectModal accessibility', () => {
  beforeEach(() => {
    bridgeMocks.getApiKeyStatus.mockReset().mockResolvedValue(STATUS);
    bridgeMocks.connectApiKey.mockReset();
    bridgeMocks.disconnectApiKey.mockReset().mockResolvedValue(undefined);
    bridgeMocks.openExternal.mockReset().mockResolvedValue(undefined);
    appStoreMocks.currentEnvelopeContext.mockClear();
  });

  it('opens provider documentation through the native opener', async () => {
    render(<ConnectModal open onClose={vi.fn()} bootstrap={makeBootstrap()} />);
    await flushStatusEffect();

    fireEvent.click(screen.getByRole('button', { name: 'Anthropic Claude (Direct)' }));
    const link = screen.getByRole('link', {
      name: 'Anthropic Claude documentation (opens in a new tab)',
    });
    const navigated = fireEvent.click(link);

    expect(navigated).toBe(false);
    expect(bridgeMocks.openExternal).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.openExternal).toHaveBeenCalledWith(
      'https://console.anthropic.com/settings/keys',
      {
        core_instance_id: 'core-1',
        workspace_id: 'ws-1',
        session_id: null,
      },
    );
    expect(appStoreMocks.currentEnvelopeContext).toHaveBeenCalledTimes(1);
  });

  it('renders a disabled docs link for a non-allowlisted host', async () => {
    const openRouter = PROVIDER_PRESETS.find((preset) => preset.id === 'openrouter');
    if (!openRouter) throw new Error('OpenRouter preset is missing');
    const originalDocsUrl = openRouter.docsUrl;
    openRouter.docsUrl = 'https://docs.example.com/openrouter';

    try {
      render(<ConnectModal open onClose={vi.fn()} bootstrap={makeBootstrap()} />);
      await flushStatusEffect();

      fireEvent.click(screen.getByRole('button', { name: 'OpenRouter (Gateway)' }));
      const link = screen.getByRole('link', {
        name: 'OpenRouter documentation (opens in a new tab)',
      });

      expect(link.getAttribute('aria-disabled')).toBe('true');
      expect(link.getAttribute('tabindex')).toBe('-1');
      expect(link.getAttribute('title')).toContain('docs.example.com');
      expect(link.getAttribute('title')).toContain('allowlist');

      const navigated = fireEvent.click(link);
      expect(navigated).toBe(false);
      expect(bridgeMocks.openExternal).not.toHaveBeenCalled();
    } finally {
      openRouter.docsUrl = originalDocsUrl;
    }
  });

  it('renders a labelled modal, focuses the close control, and exposes provider cards as pressed buttons', async () => {
    render(<ConnectModal open onClose={vi.fn()} bootstrap={makeBootstrap()} />);
    await flushStatusEffect();

    const dialog = screen.getByRole('dialog', {
      name: 'Model Provider & Gateway Connection',
    });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.getAttribute('aria-describedby')).toBe('connect-modal-description');

    const close = within(dialog).getByRole('button', { name: 'Close dialog' });
    expect(document.activeElement).toBe(close);

    const selora = within(dialog).getByRole('button', {
      name: 'Selora API Gateway (Recommended)',
    });
    expect(selora.tagName).toBe('BUTTON');
    expect(selora.getAttribute('aria-pressed')).toBe('true');
    selora.focus();
    expect(document.activeElement).toBe(selora);

    fireEvent.click(within(dialog).getByRole('button', { name: 'OpenAI Direct (Direct)' }));
    expect(
      within(dialog)
        .getByRole('button', { name: 'OpenAI Direct (Direct)' })
        .getAttribute('aria-pressed'),
    ).toBe('true');
    expect(
      within(dialog)
        .getByRole('button', { name: 'Selora API Gateway (Recommended)' })
        .getAttribute('aria-pressed'),
    ).toBe('false');
  });

  it('wraps Tab and Shift+Tab within the dialog', async () => {
    render(<ConnectModal open onClose={vi.fn()} bootstrap={makeBootstrap()} />);
    await flushStatusEffect();

    const dialog = screen.getByRole('dialog', {
      name: 'Model Provider & Gateway Connection',
    });
    const close = within(dialog).getByRole('button', { name: 'Close dialog' });
    const focusables = Array.from(dialog.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
    const last = focusables[focusables.length - 1];

    close.focus();
    fireEvent.keyDown(close, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(last);

    last.focus();
    fireEvent.keyDown(last, { key: 'Tab' });
    expect(document.activeElement).toBe(close);
  });

  it('closes on Escape and restores focus to the opener', async () => {
    const launcher = document.createElement('button');
    launcher.textContent = 'Open connection settings';
    document.body.appendChild(launcher);
    launcher.focus();

    function Harness() {
      const [open, setOpen] = useState(true);
      return (
        <ConnectModal
          open={open}
          onClose={() => setOpen(false)}
          bootstrap={makeBootstrap()}
        />
      );
    }

    render(<Harness />);
    await flushStatusEffect();

    const close = screen.getByRole('button', { name: 'Close dialog' });
    expect(document.activeElement).toBe(close);

    fireEvent.keyDown(close, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(launcher);

    launcher.remove();
  });

  it('restores focus when unmounted while the dialog owns it', async () => {
    const launcher = document.createElement('button');
    launcher.textContent = 'Open connection settings';
    document.body.appendChild(launcher);
    launcher.focus();

    const { unmount } = render(
      <ConnectModal open onClose={vi.fn()} bootstrap={makeBootstrap()} />,
    );
    await flushStatusEffect();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Close dialog' }));

    unmount();
    expect(document.activeElement).toBe(launcher);

    launcher.remove();
  });

  it('associates the API key label, validation text, and privacy hint with the input', async () => {
    render(<ConnectModal open onClose={vi.fn()} bootstrap={makeBootstrap()} />);
    await flushStatusEffect();

    const input = screen.getByLabelText('API Key') as HTMLInputElement;
    expect(input.id).toBe('api-key-input');
    expect(input.getAttribute('aria-describedby')).toBe('api-key-validation api-key-privacy-hint');

    const badge = document.getElementById('api-key-validation');
    expect(badge?.textContent).toContain('Format required');

    fireEvent.change(input, { target: { value: 'not-a-selora-key' } });
    expect(badge?.textContent).toContain('Format hint: sk-gw-... or sk-selora-...');
    expect(input.getAttribute('aria-invalid')).toBe('true');
  });

  it('marks Test & Connect busy and publishes a text status while connecting', async () => {
    bridgeMocks.connectApiKey.mockReturnValue(new Promise(() => {}));
    render(<ConnectModal open onClose={vi.fn()} bootstrap={makeBootstrap()} />);
    await flushStatusEffect();

    fireEvent.change(screen.getByLabelText('API Key'), {
      target: { value: 'sk-gw-test-key' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Test & Connect' }));

    const busyButton = screen.getByRole('button', { name: 'Testing Gateway…' });
    expect(busyButton.getAttribute('aria-busy')).toBe('true');
    expect(screen.getByTestId('connect-test-status').textContent).toBe('Testing connection…');
  });
});
