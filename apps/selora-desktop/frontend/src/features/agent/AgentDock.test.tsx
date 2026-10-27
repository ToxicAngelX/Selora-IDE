// Selora IDE — Phase Z3 AgentDock Component Tests
import { describe, it, expect } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { AgentDock } from '../../layout/AgentDock';

describe('AgentDock Component', () => {
  it('renders the header with ThinkingOrb, session label, and tab toggles', () => {
    const { getByRole, getByText, queryByRole } = render(<AgentDock />);

    expect(getByText('Agent')).toBeTruthy();
    expect(getByRole('button', { name: 'Chat' })).toBeTruthy();
    expect(getByText(/Plan \(/)).toBeTruthy();
    // No active buffer in this render: only the workspace chip is offered.
    expect(queryByRole('button', { name: 'Explain this file' })).toBeNull();
    expect(getByRole('button', { name: 'Summarize the workspace' })).toBeTruthy();
  });

  it('switches between Chat and Plan tabs to reveal live plan execution', () => {
    const { getByRole, getByText, getAllByText } = render(<AgentDock />);

    const planTab = getByRole('button', { name: /Plan \(/ });
    fireEvent.click(planTab);

    expect(getByText('Subagents Swarm')).toBeTruthy();
    expect(getByText('Plan Execution')).toBeTruthy();
    expect(getAllByText('frontend-developer').length).toBeGreaterThan(0);

    const chatTab = getByRole('button', { name: 'Chat' });
    fireEvent.click(chatTab);

    expect(getByText('Ask anything')).toBeTruthy();
  });
});

