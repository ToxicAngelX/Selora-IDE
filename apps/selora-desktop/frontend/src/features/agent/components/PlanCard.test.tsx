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

    expect(getByTestId('plan-card')).toBeTruthy();
    expect(getByText('Plan approval')).toBeTruthy();
    expect(getByText('Needs your approval')).toBeTruthy();
    expect(getByText('Make the change')).toBeTruthy();

    const steps = Array.from(container.querySelectorAll('.plan-card-step-text')).map(
      (node) => node.textContent,
    );
    expect(steps).toEqual(['Inspect the router', 'Write the plan', 'Verify the gates']);
    expect(getByText('Approve plan')).toBeTruthy();
    expect(getByText('Revise')).toBeTruthy();
    // No blanket approval: a plan is a shape, not a write grant.
    expect(container.textContent).not.toContain('Approve All');
  });

  it('answers Approve and Revise through the decision callback', () => {
    const onDecision = vi.fn();
    const { getByTestId, unmount } = render(
      <PlanCard {...props} decision={null} onDecision={onDecision} />,
    );

    fireEvent.click(getByTestId('plan-approve'));
    expect(onDecision).toHaveBeenLastCalledWith('approve');

    fireEvent.click(getByTestId('plan-revise'));
    expect(onDecision).toHaveBeenLastCalledWith('revise');
    expect(onDecision).toHaveBeenCalledTimes(2);
    unmount();
  });

  it('records a decided plan instead of re-offering the buttons', () => {
    const { queryByTestId, getByText, rerender } = render(
      <PlanCard {...props} decision="approve" />,
    );
    expect(queryByTestId('plan-approve')).toBeNull();
    expect(getByText('Approved')).toBeTruthy();

    rerender(<PlanCard {...props} decision="revise" />);
    expect(queryByTestId('plan-approve')).toBeNull();
    expect(getByText('Revision requested')).toBeTruthy();
  });
});