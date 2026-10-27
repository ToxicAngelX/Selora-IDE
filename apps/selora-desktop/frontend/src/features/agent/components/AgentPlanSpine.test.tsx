// Selora IDE — AgentPlanSpine Component Tests (Phase 9: backend-authoritative)
import { describe, it, expect, beforeEach } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { AgentPlanSpine } from './AgentPlanSpine';
import { agentStore } from '../agent-store';
import type { AgentTodo } from '../types';

beforeEach(() => {
  agentStore.reset();
});

const sampleTodos: AgentTodo[] = [
  { id: 't-1', text: '1. Audit frontend architecture', status: 'completed' },
  { id: 't-2', text: '2. Implement responsive composer (Phase Z)', status: 'in_progress' },
  { id: 't-3', text: '3. Run test verification', status: 'pending' },
];

describe('AgentPlanSpine Component', () => {
  it('renders live progress and step counters in full view', () => {
    const { getByText, getAllByText } = render(<AgentPlanSpine todos={sampleTodos} />);

    // 1 completed out of 3 = 33%
    expect(getAllByText('33%').length).toBeGreaterThan(0);
    expect(getByText('1 of 3 steps complete')).toBeTruthy();
    expect(getByText('In Progress')).toBeTruthy();
  });

  it('renders only stored todos — no invented subagent roster or artifacts', () => {
    const { queryByText } = render(<AgentPlanSpine todos={sampleTodos} />);

    expect(queryByText('Subagents Swarm')).toBeNull();
    expect(queryByText('frontend-developer')).toBeNull();
    expect(queryByText('Artifacts')).toBeNull();
    // The old template actions would have fabricated local todos.
    expect(queryByText('+ Add step')).toBeNull();
    expect(queryByText(/Load Full Development Plan/)).toBeNull();
  });

  it('renders the active objective from the stored text', () => {
    const { getByText, getAllByText } = render(<AgentPlanSpine todos={sampleTodos} />);

    expect(getByText('CURRENT OBJECTIVE')).toBeTruthy();
    expect(getAllByText('2. Implement responsive composer').length).toBeGreaterThan(0);
    expect(getAllByText('(Phase Z)').length).toBeGreaterThan(0);
  });

  it('uses the real git branch and never a fabricated main/commit action', () => {
    const { getByText, queryByText } = render(
      <AgentPlanSpine
        todos={sampleTodos}
        gitStatus={{ branch: 'feature/phase-9', changed_paths: ['src/a.ts'] }}
        isCompactInfeed={true}
      />,
    );

    expect(getByText('feature/phase-9')).toBeTruthy();
    expect(queryByText('main')).toBeNull();
    expect(queryByText('Commit or push')).toBeNull();
  });

  it('says No branch when the backend reports none', () => {
    const { getByText } = render(
      <AgentPlanSpine todos={sampleTodos} isCompactInfeed={true} />,
    );
    expect(getByText('No branch')).toBeTruthy();
  });

  it('renders todos as read-only rows (no local status mutation controls)', () => {
    const { container, queryByRole } = render(<AgentPlanSpine todos={sampleTodos} />);

    expect(queryByRole('button', { name: /Start 3\. Run test verification/ })).toBeNull();
    expect(queryByRole('button', { name: /Remove/ })).toBeNull();

    // Reveal the completed section: still no row buttons.
    const toggle = queryByRole('button', { name: /1 completed/ });
    expect(toggle).toBeTruthy();
    fireEvent.click(toggle!);

    const rows = Array.from(container.querySelectorAll('.zcode-todo-row'));
    expect(rows.length).toBe(3);
    for (const row of rows) {
      expect(row.tagName).not.toBe('BUTTON');
      expect(row.querySelectorAll('button')).toHaveLength(0);
    }
  });

  it('exposes plan progress as an ARIA progressbar with min, max, and now values', () => {
    const { getAllByRole } = render(<AgentPlanSpine todos={sampleTodos} />);

    const bars = getAllByRole('progressbar');
    expect(bars).toHaveLength(1);
    expect(bars[0].getAttribute('aria-valuemin')).toBe('0');
    expect(bars[0].getAttribute('aria-valuemax')).toBe('100');
    expect(bars[0].getAttribute('aria-valuenow')).toBe('33');
  });

  it('exposes compact progress as an ARIA progressbar', () => {
    const { getByRole } = render(
      <AgentPlanSpine todos={sampleTodos} isCompactInfeed={true} />,
    );

    const bar = getByRole('progressbar', { name: 'Progress: 33%' });
    expect(bar.getAttribute('aria-valuemin')).toBe('0');
    expect(bar.getAttribute('aria-valuemax')).toBe('100');
    expect(bar.getAttribute('aria-valuenow')).toBe('33');
  });
});