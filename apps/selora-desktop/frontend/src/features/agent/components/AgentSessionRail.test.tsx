// Selora IDE — AgentSessionRail accessibility tests
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render } from '@testing-library/react';
import React from 'react';
import { AgentSessionRail } from './AgentSessionRail';
import { agentStore } from '../agent-store';
import type { AgentSession, ResumableAgentSession } from '../types';

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

  it('lists durable sessions under a truthful Resume session group', () => {
    const resumable: ResumableAgentSession[] = [
      {
        id: 'sess-disk',
        title: 'Yesterday work',
        updatedAt: 10,
        model: 'test-model',
        mode: 'execute',
        pending: { kind: 'approval', summary: 'bash cargo test' },
        compacted: true,
        messageCount: 12,
      },
    ];
    const resumeSpy = vi.spyOn(agentStore, 'resumeSession').mockImplementation(() => true);
    const { getByRole, getByText } = render(
      <AgentSessionRail
        sessions={sessions}
        activeSessionId="sess-active"
        resumableSessions={resumable}
      />,
    );

    expect(getByRole('group', { name: 'Resume session' })).toBeTruthy();
    expect(getByText('Yesterday work')).toBeTruthy();
    expect(getByText('approval')).toBeTruthy();
    expect(getByText('compacted')).toBeTruthy();

    fireEvent.click(getByText('Yesterday work').closest('button')!);
    expect(resumeSpy).toHaveBeenLastCalledWith('sess-disk');
    resumeSpy.mockRestore();
  });
});
