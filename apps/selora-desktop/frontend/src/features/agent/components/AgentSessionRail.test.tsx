// Selora IDE — AgentSessionRail accessibility tests (Phase 9a).
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, waitFor } from '@testing-library/react';
import React from 'react';
import { AgentSessionRail } from './AgentSessionRail';
import { agentStore } from '../agent-store';
import type { AgentSession } from '../types';

const sessions: AgentSession[] = [
  {
    id: 'sess-active',
    title: 'Current session',
    createdAt: 1,
    updatedAt: 2,
    tokenCount: 0,
    costUsd: 0,
    isRunning: true,
  },
  {
    id: 'sess-older',
    title: 'Older session',
    createdAt: 3,
    updatedAt: 4,
    tokenCount: 0,
    costUsd: 0,
  },
];

describe('AgentSessionRail', () => {
  beforeEach(() => {
    agentStore.reset();
  });

  it('renders session rows as buttons and announces the current session', () => {
    const { getByText } = render(
      <AgentSessionRail sessions={sessions} activeSessionId="sess-active" />
    );

    const currentRow = getByText('Current session').closest('button');
    const otherRow = getByText('Older session').closest('button');

    expect(currentRow).toBeTruthy();
    expect(otherRow).toBeTruthy();
    expect(currentRow?.tagName).toBe('BUTTON');
    expect(otherRow?.tagName).toBe('BUTTON');
    expect(currentRow?.getAttribute('aria-current')).toBe('true');
    expect(otherRow?.hasAttribute('aria-current')).toBe(false);
  });

it('routes New and select through the backend session commands', async () => {
    const switchSpy = vi.spyOn(agentStore, 'requestSwitchSession').mockResolvedValue(true);
    const newSpy = vi.spyOn(agentStore, 'requestNewSession').mockResolvedValue(true);
    const onClose = vi.fn();
    const { getByText, getByRole } = render(
      <AgentSessionRail
        sessions={sessions}
        activeSessionId="sess-active"
        onClose={onClose}
      />
    );
    const otherRow = getByText('Older session').closest('button');
    expect(otherRow).toBeTruthy();

fireEvent.click(otherRow!);
    await waitFor(() => expect(switchSpy).toHaveBeenCalledWith('sess-older'));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));

    expect(fireEvent.keyDown(otherRow!, { key: ' ' })).toBe(false);
    await waitFor(() => expect(switchSpy).toHaveBeenCalledTimes(2));

    fireEvent.click(getByRole('button', { name: /New/ }));
    await waitFor(() => expect(newSpy).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(3));

    switchSpy.mockRestore();
    newSpy.mockRestore();
  });

  it('shows the empty session state without fabricating a row', () => {
    const { getByText, queryByRole } = render(
      <AgentSessionRail sessions={[]} activeSessionId="" />
    );
    expect(getByText('No sessions yet')).toBeTruthy();
    expect(queryByRole('button', { name: /Current session/ })).toBeNull();
  });
});