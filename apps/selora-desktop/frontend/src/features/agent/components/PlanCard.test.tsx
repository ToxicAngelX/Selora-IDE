// Selora IDE — PlanCard unit tests (Phase 9b).
//
// The plan card is the plan-mode decision moment: the ordered steps, and
// exactly two answers (Approve, Revise). It never offers a blanket grant.
import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { PlanCard } from './PlanCard';

const props = {
  summary: 'Make the change',
  steps: ['Inspect the router', 'Write the plan', 'Verify the gates'],
  approvalId: 'appr-plan-1',
};

describe('PlanCard', () => {
  it('renders every step in order with the approval prompt', () => {
    const { getByTestId, getByText, container } = render(<PlanCard {...props} decision={null} />);

    expect(container.querySelectorAll('.agent-plan-card')).toHaveLength(1);
    expect(getByText('Plan review')).toBeTruthy();
    expect(getByText('Make the change')).toBeTruthy();

    const steps = Array.from(container.querySelectorAll('.agent-plan-step-title')).map(
      (node) => node.textContent,
    );
    expect(steps).toEqual(['Inspect the router', 'Write the plan', 'Verify the gates']);
    expect(getByText('Approve plan')).toBeTruthy();
    expect(getByText('Request changes')).toBeTruthy();
    expect(getByText('Deny plan')).toBeTruthy();
    // No blanket approval: a plan is a shape, not a write grant.
    expect(container.textContent).not.toContain('Approve All');
  });

  it('answers Approve and Revise through the decision callback', () => {
    const onDecision = vi.fn();
    const { getByText, getByLabelText, unmount } = render(
      <PlanCard {...props} decision={null} onDecision={onDecision} />,
    );

    fireEvent.click(getByText('Approve plan'));
    expect(onDecision).toHaveBeenLastCalledWith('approve');

    fireEvent.click(getByText('Request changes'));
    fireEvent.change(getByLabelText('What should change?'), { target: { value: 'Keep scope small' } });
    fireEvent.click(getByText('Send changes'));
    expect(onDecision).toHaveBeenLastCalledWith('revise');
    expect(onDecision).toHaveBeenCalledTimes(2);
    unmount();
  });

  it('records a decided plan instead of re-offering the buttons', () => {
    const { queryByTestId, getByText, rerender } = render(
      <PlanCard {...props} decision="approve" />,
    );
    expect(queryByTestId('plan-approve')).toBeNull();
    expect(getByText('Plan approved')).toBeTruthy();

    rerender(<PlanCard {...props} decision="revise" />);
    expect(queryByTestId('plan-approve')).toBeNull();
    expect(getByText('Changes requested: Revision requested')).toBeTruthy();
  });
});
