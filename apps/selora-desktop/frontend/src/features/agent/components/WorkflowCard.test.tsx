// Selora IDE — WorkflowCard unit tests (Phase 8c, ARCHITECTURE.md §18.1)
//
// The card is the workflow grant's confirmation moment, so these tests pin
// what the user actually sees before approving: the whole script, the
// extracted command set, and the cost cap — plus the decision callback and
// the collapsed/expanded states.
import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { WorkflowCard } from './WorkflowCard';

const SCRIPT = `phase('scan');
const reviewer = agent('reviewer');
await reviewer.ask({ instructions: 'review src/' });
await world.run('cargo', ['test'], { timeoutMs: 120000 });
report({ ok: true });
`;

const baseProps = {
  name: 'repo-audit',
  script: SCRIPT,
  commands: ['cargo test', 'git status'],
  maxCostUsd: 5,
  approvalId: 'appr-workflow-1',
};

describe('WorkflowCard', () => {
  it('renders the script body, the extracted command set, and the cost cap', () => {
    const { getByTestId, getByText, getAllByTestId } = render(
      <WorkflowCard {...baseProps} />,
    );

    const card = getByTestId('workflow-card');
    expect(card.getAttribute('aria-label')).toBe('Review workflow repo-audit');
    expect(card.getAttribute('data-decision')).toBe('pending');

    // The script is shown verbatim, line for line.
    expect(getByTestId('workflow-card-script').textContent).toBe(SCRIPT);

    // Header telemetry: 5 script lines, 2 confirmed commands, cap.
    expect(getByText('repo-audit')).toBeTruthy();
    expect(getByText('5 lines')).toBeTruthy();
    expect(getByText('2 commands')).toBeTruthy();
    expect(getByTestId('workflow-card-cost').textContent).toContain('$5.00');

    // The command set is itemized, in the confirmed order.
    const commands = getAllByTestId('workflow-card-command');
    expect(commands).toHaveLength(2);
    expect(commands[0].textContent).toContain('cargo test');
    expect(commands[1].textContent).toContain('git status');

    // The cost cap is restated in the decision row.
    expect(getByText('Cost cap')).toBeTruthy();
    expect(getByText('checked between every step')).toBeTruthy();

    // Approve and Deny are the only decisions this card offers.
    expect(getByTestId('workflow-approve').textContent).toContain('Approve');
    expect(getByTestId('workflow-deny').textContent).toContain('Deny');
  });

  it('calls the decision callback with the decision and the parked approval id', () => {
    const onDecision = vi.fn();
    const { getByTestId } = render(
      <WorkflowCard {...baseProps} onDecision={onDecision} />,
    );

    fireEvent.click(getByTestId('workflow-approve'));
    expect(onDecision).toHaveBeenCalledTimes(1);
    expect(onDecision).toHaveBeenCalledWith('approve', 'appr-workflow-1');

    fireEvent.click(getByTestId('workflow-deny'));
    expect(onDecision).toHaveBeenCalledTimes(2);
    expect(onDecision).toHaveBeenLastCalledWith('deny', 'appr-workflow-1');
  });

  it('shows the recorded decision and never decides twice', () => {
    const onDecision = vi.fn();
    const { getByTestId, queryByTestId } = render(
      <WorkflowCard
        {...baseProps}
        decision="deny"
        onDecision={onDecision}
      />,
    );

    expect(getByTestId('workflow-card').getAttribute('data-decision')).toBe('deny');
    expect(getByTestId('workflow-card-decision').textContent).toContain('Denied');
    expect(queryByTestId('workflow-approve')).toBeNull();
    expect(queryByTestId('workflow-deny')).toBeNull();
    expect(onDecision).not.toHaveBeenCalled();
  });

  it('starts expanded and collapses to the grant summary and back', () => {
    const { getByTestId, queryByTestId } = render(<WorkflowCard {...baseProps} />);

    const toggle = getByTestId('workflow-card-toggle');
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(getByTestId('workflow-card-script')).toBeTruthy();

    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(queryByTestId('workflow-card-script')).toBeNull();
    // The summary stays visible while collapsed — name, command count, cap.
    expect(getByTestId('workflow-card-cost')).toBeTruthy();
    expect(queryByTestId('workflow-approve')).toBeNull();

    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(getByTestId('workflow-card-script').textContent).toBe(SCRIPT);
  });

  it('can start collapsed and explains a gate-free workflow', () => {
    const { getByTestId, queryByTestId, getByText } = render(
      <WorkflowCard {...baseProps} commands={[]} defaultExpanded={false} />,
    );

    expect(queryByTestId('workflow-card-script')).toBeNull();
    expect(getByText('0 commands')).toBeTruthy();

    fireEvent.click(getByTestId('workflow-card-toggle'));
    expect(getByTestId('workflow-card-script')).toBeTruthy();
    expect(getByTestId('workflow-card-commands').textContent).toContain(
      'No shell commands',
    );
  });
});