// Selora IDE — AgentPlanSpine Component Tests
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { AgentPlanSpine } from './AgentPlanSpine';
import { agentStore } from '../agent-store';
import type { AgentTodo, SubagentInfo, ArtifactItem } from '../types';

beforeEach(() => {
  agentStore.reset();
});

const sampleTodos: AgentTodo[] = [
  { id: 't-1', title: '1. Audit frontend architecture', status: 'completed' },
  { id: 't-2', title: '2. Implement responsive composer (Phase Z)', status: 'in_progress', assignedSubagent: 'frontend-developer' },
  { id: 't-3', title: '3. Run test verification', status: 'pending' },
];

const sampleSubagents: SubagentInfo[] = [
  { id: 'sub-1', name: 'frontend-developer', model: 'GLM-5.3 CodeFast', status: 'working', activeStep: 'Elevating AgentComposer' },
  { id: 'sub-2', name: 'rust-developer', model: 'Claude 3.7 Sonnet', status: 'idle' },
  { id: 'sub-3', name: 'qa-tester', model: 'GPT-4o', status: 'done', activeStep: 'All test assertions passed' },
];

const sampleArtifacts: ArtifactItem[] = [
  { id: 'art-1', title: 'COMPOSER_REVAMP.md', kind: 'doc', path: 'docs/composer.md', summary: 'UI revamp specs', createdAt: Date.now() },
];

describe('AgentPlanSpine Component', () => {
  it('renders live progress percentage bar and step counters in full view', () => {
    const { getByText, getAllByText } = render(
      <AgentPlanSpine
        todos={sampleTodos}
        subagents={sampleSubagents}
        artifacts={sampleArtifacts}
      />
    );

    // 1 completed out of 3 = 33%
    expect(getAllByText('33%').length).toBeGreaterThan(0);
    expect(getByText('1 of 3 steps complete')).toBeTruthy();
    expect(getByText('In Progress')).toBeTruthy();
  });

  it('renders subagents swarm roster with live status badges (Running, Idle, Completed)', () => {
    const { getByText, getAllByText } = render(
      <AgentPlanSpine
        todos={sampleTodos}
        subagents={sampleSubagents}
        artifacts={sampleArtifacts}
      />
    );

    expect(getByText('Subagents Swarm')).toBeTruthy();
    expect(getAllByText('frontend-developer').length).toBeGreaterThan(0);
    expect(getByText('Running')).toBeTruthy();
    expect(getByText('Idle')).toBeTruthy();
    expect(getByText('Completed')).toBeTruthy();
  });

  it('renders active task objective with animated indicator and subagent tag', () => {
    const { getByText, getAllByText } = render(
      <AgentPlanSpine
        todos={sampleTodos}
        subagents={sampleSubagents}
        artifacts={sampleArtifacts}
      />
    );

    expect(getByText('CURRENT OBJECTIVE')).toBeTruthy();
    expect(getAllByText('2. Implement responsive composer').length).toBeGreaterThan(0);
    expect(getAllByText('(Phase Z)').length).toBeGreaterThan(0);
  });

  it('renders compact infeed view with progress bar and git tools', () => {
    const { getByText } = render(
      <AgentPlanSpine
        todos={sampleTodos}
        subagents={sampleSubagents}
        artifacts={sampleArtifacts}
        isCompactInfeed={true}
      />
    );

    expect(getByText('Git tools')).toBeTruthy();
    expect(getByText('Progress')).toBeTruthy();
    expect(getByText('1/3')).toBeTruthy();
    expect(getByText('33%')).toBeTruthy();
  });

  it('activates plan rows through a real primary button and exposes their pressed state', () => {
    const updateSpy = vi.spyOn(agentStore, 'updateTodoStatus').mockImplementation(() => {});
    const { getByRole } = render(
      <AgentPlanSpine
        todos={sampleTodos}
        subagents={sampleSubagents}
        artifacts={sampleArtifacts}
      />
    );

    const pendingRow = getByRole('button', { name: 'Start 3. Run test verification' });
    expect(pendingRow.tagName).toBe('BUTTON');
    expect(pendingRow.getAttribute('type')).toBe('button');
    expect(pendingRow.getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(pendingRow);
    expect(updateSpy).toHaveBeenLastCalledWith('t-3', 'in_progress');

    updateSpy.mockRestore();
  });

  it('renders rows as sibling buttons instead of a button nested in role=button', () => {
    const updateSpy = vi.spyOn(agentStore, 'updateTodoStatus').mockImplementation(() => {});
    const removeSpy = vi.spyOn(agentStore, 'removeTodo').mockImplementation(() => {});
    const { container, getByRole } = render(
      <AgentPlanSpine
        todos={sampleTodos}
        subagents={sampleSubagents}
        artifacts={sampleArtifacts}
      />
    );

    // Reveal the completed section so every full-view row shape is covered.
    fireEvent.click(getByRole('button', { name: /1 completed/ }));

    expect(container.querySelectorAll('[role="button"]')).toHaveLength(0);
    expect(container.querySelectorAll('.zcode-todo-row button button')).toHaveLength(0);

    const rows = Array.from(container.querySelectorAll('.zcode-todo-row'));
    expect(rows.length).toBe(3);
    for (const row of rows) {
      expect(row.tagName).not.toBe('BUTTON');
      expect(row.getAttribute('role')).not.toBe('button');
      const directButtons = Array.from(row.children).filter((child) => child.tagName === 'BUTTON');
      expect(directButtons.length).toBe(2);
    }

    fireEvent.click(getByRole('button', { name: 'Remove 2. Implement responsive composer' }));
    expect(removeSpy).toHaveBeenCalledWith('t-2');
    expect(updateSpy).not.toHaveBeenCalled();

    removeSpy.mockRestore();
    updateSpy.mockRestore();
  });

  it('exposes plan progress as an ARIA progressbar with min, max, and now values', () => {
    const { getAllByRole } = render(
      <AgentPlanSpine
        todos={sampleTodos}
        subagents={sampleSubagents}
        artifacts={sampleArtifacts}
      />
    );

    const bars = getAllByRole('progressbar');
    expect(bars).toHaveLength(1);
    expect(bars[0].getAttribute('aria-valuemin')).toBe('0');
    expect(bars[0].getAttribute('aria-valuemax')).toBe('100');
    expect(bars[0].getAttribute('aria-valuenow')).toBe('33');
  });

  it('exposes compact progress as an ARIA progressbar', () => {
    const { getByRole } = render(
      <AgentPlanSpine
        todos={sampleTodos}
        subagents={sampleSubagents}
        artifacts={sampleArtifacts}
        isCompactInfeed={true}
      />
    );

    const bar = getByRole('progressbar', { name: 'Progress: 33%' });
    expect(bar.getAttribute('aria-valuemin')).toBe('0');
    expect(bar.getAttribute('aria-valuemax')).toBe('100');
    expect(bar.getAttribute('aria-valuenow')).toBe('33');
  });
});
