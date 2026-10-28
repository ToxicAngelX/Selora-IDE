import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { UpdatePrompt } from './UpdatePrompt';

const bridgeMock = vi.hoisted(() => ({
  checkForUpdate: vi.fn(),
  installUpdate: vi.fn(async () => undefined),
  listenUpdateProgress: vi.fn(async () => () => undefined),
  openExternal: vi.fn(async () => undefined),
}));

vi.mock('../bridge/bridge', () => bridgeMock);
vi.mock('../app/app-store', () => ({
  currentEnvelopeContext: () => ({ core_instance_id: 'core-1', workspace_id: 'ws-1', session_id: null }),
}));

const UPDATE = {
  current_version: '0.1.0',
  latest_tag: 'v0.2.0',
  newer: true,
  release_url: 'https://github.com/ToxicAngelX/Selora-IDE/releases/tag/v0.2.0',
  installer_available: true,
};

describe('UpdatePrompt', () => {
  beforeEach(() => {
    bridgeMock.checkForUpdate.mockReset();
    bridgeMock.installUpdate.mockClear();
    bridgeMock.listenUpdateProgress.mockClear();
  });

  afterEach(() => {
    cleanup();
  });

  it('asks before downloading when a newer release exists', async () => {
    bridgeMock.checkForUpdate.mockResolvedValue(UPDATE);
    render(<UpdatePrompt autoCheck />);

    const dialog = await screen.findByRole('dialog');
    expect(dialog).toBeTruthy();
    expect(screen.getByText('Selora IDE v0.2.0')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Yes, update' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'No, later' })).toBeTruthy();
    expect(bridgeMock.installUpdate).not.toHaveBeenCalled();
  });

  it('downloads only after Yes', async () => {
    bridgeMock.checkForUpdate.mockResolvedValue(UPDATE);
    render(<UpdatePrompt autoCheck />);

    const yes = await screen.findByRole('button', { name: 'Yes, update' });
    await act(async () => {
      fireEvent.click(yes);
    });

    await waitFor(() => expect(bridgeMock.installUpdate).toHaveBeenCalledTimes(1));
    expect(bridgeMock.listenUpdateProgress).toHaveBeenCalledTimes(1);
  });

  it('No dismisses the prompt without downloading', async () => {
    bridgeMock.checkForUpdate.mockResolvedValue(UPDATE);
    render(<UpdatePrompt autoCheck />);

    const no = await screen.findByRole('button', { name: 'No, later' });
    await act(async () => {
      fireEvent.click(no);
    });

    expect(screen.queryByRole('dialog')).toBeNull();
    expect(bridgeMock.installUpdate).not.toHaveBeenCalled();
  });

  it('stays closed when the installed build is the latest', async () => {
    bridgeMock.checkForUpdate.mockResolvedValue({ ...UPDATE, newer: false, latest_tag: 'v0.1.0' });
    render(<UpdatePrompt autoCheck />);

    await waitFor(() => expect(bridgeMock.checkForUpdate).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('skips the automatic check when the setting is off', async () => {
    render(<UpdatePrompt autoCheck={false} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(bridgeMock.checkForUpdate).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('a failed check never opens a dialog', async () => {
    bridgeMock.checkForUpdate.mockRejectedValue(new Error('offline'));
    render(<UpdatePrompt autoCheck />);

    await waitFor(() => expect(bridgeMock.checkForUpdate).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});