// Selora IDE — Phase Z3/9a AgentDock Component Tests
import { beforeEach, describe, it, expect } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { AgentDock } from '../../layout/AgentDock';
import { agentStore } from './agent-store';
import { makeBootstrap } from '../../testing/fixtures';

beforeEach(() => {
  agentStore.reset();
  agentStore.hydrate(
    makeBootstrap({
      workspace: { id: 'ws-1', core_instance_id: 'core-0001', root: '/repo', display_name: 'repo' },
      chat: {
        sessions: [
          { session_id: 'sess-a', title: 'Agent Session 1', message_ids: [], active: false },
        ],
        messages: [],
        active_session_id: 'sess-a',
        active_request_id: null,
        active_turn_id: null,
        pending_approval: null,
      },
    }),
  );
});

describe('AgentDock Component', () => {
  it('renders the header with ThinkingOrb, session tabs, and view toggles', () => {
    const { getByRole, getAllByText, getByText } = render(<AgentDock />);

    expect(getAllByText('Agent').length).toBeGreaterThan(0);
    expect(getByRole('tab', { name: /Agent Session 1/ })).toBeTruthy();
    expect(getByRole('button', { name: 'Chat' })).toBeTruthy();
    expect(getByText(/Plan \(/)).toBeTruthy();
  });

it('switches between Chat and Plan tabs to reveal the todo spine', () => {
    const { getByRole, getByText, getAllByText, queryByText } = render(<AgentDock />);

    const planTab = getByRole('button', { name: /Plan \(/ });
    fireEvent.click(planTab);

    expect(getByText('Plan Execution')).toBeTruthy();
// Backend-authoritative: no fabricated subagent roster.
    expect(queryByText('Subagents Swarm')).toBeNull();
    expect(getAllByText('No todos yet').length).toBeGreaterThan(0);

    const chatTab = getByRole('button', { name: 'Chat' });
    fireEvent.click(chatTab);

    expect(getByText('Ask anything')).toBeTruthy();
  });

  it('reveals a completed child result under its parent session', () => {
    act(() => {
      agentStore.handleSubagentCompleted({
        child_id: 'child-dock-1',
        parent_session_id: 'sess-dock-parent',
        policy: 'full',
        ok: true,
        final_message: 'the child found the bug',
        artifacts: [{ kind: 'diff', path: 'src/main.rs' }],
      });
      agentStore.revealSubagent('sess-dock-parent', 'child-dock-1');
    });

    const { getByTestId, getByText } = render(<AgentDock />);
    const result = getByTestId('subagent-result');
    expect(result.textContent).toContain('the child found the bug');
    expect(result.textContent).toContain('src/main.rs');
    expect(getByText('Plan Execution')).toBeTruthy();
  });
});

