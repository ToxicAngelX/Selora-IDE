// Selora IDE — AgentSessionRail accessibility tests
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render } from '@testing-library/react';
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
    tokenCount: 1200,
    costUsd: 0.01,
    branchName: 'main',
  },
  {
    id: 'sess-older',
    title: 'Older session',
    createdAt: 3,
    updatedAt: 4,
    tokenCount: 2400,
    costUsd: 0.02,
    branchName: 'feature/a11y',
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

  it('switches sessions with both Enter and Space', () => {
    const switchSpy = vi.spyOn(agentStore, 'switchSession').mockImplementation(() => {});
    const onClose = vi.fn();
    const { getByText } = render(
      <AgentSessionRail
        sessions={sessions}
        activeSessionId="sess-active"
        onClose={onClose}
      />
    );
    const otherRow = getByText('Older session').closest('button');
    expect(otherRow).toBeTruthy();

    fireEvent.keyDown(otherRow!, { key: 'Enter' });
    expect(switchSpy).toHaveBeenLastCalledWith('sess-older');

    expect(fireEvent.keyDown(otherRow!, { key: ' ' })).toBe(false);
    expect(switchSpy).toHaveBeenCalledTimes(2);
    expect(onClose).toHaveBeenCalledTimes(2);

    switchSpy.mockRestore();
  });
});
