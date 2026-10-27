// Selora IDE — Phase 9b plan approval card.
//
// The plan mode contract on screen: the model's ordered steps, with exactly
// two answers. Approve flips the session to execution (writes still card);
// Revise keeps planning and asks the model for a new plan. No blanket
// approval is offered — a plan is a reviewed shape, not a write grant.
import React from 'react';
import { Check, X } from '../../../components/icons';

export type PlanDecision = 'approve' | 'revise';

export type PlanCardProps = {
  summary?: string | null;
  steps: string[];
  /** `null` while the card is pending. */
  decision: PlanDecision | null;
  approvalId?: string;
  onDecision?: (decision: PlanDecision) => void;
};

export const PlanCard: React.FC<PlanCardProps> = ({
  summary,
  steps,
  decision,
  approvalId,
  onDecision,
}) => {
  const pending = decision === null;
  const label = decision === 'approve' ? 'Approved' : 'Revision requested';

  return (
    <div className="plan-card" data-testid="plan-card">
      <div className="plan-card-header">
        <span className="plan-card-title">Plan approval</span>
        <span className={`plan-card-status ${pending ? 'is-pending' : 'is-decided'}`}>
          {pending ? 'Needs your approval' : label}
        </span>
      </div>
      {summary && <p className="plan-card-summary">{summary}</p>}
      <ol className="plan-card-steps">
        {steps.map((step, index) => (
          <li key={`${index}-${step}`} className="plan-card-step">
            <span className="plan-card-step-num">{index + 1}</span>
            <span className="plan-card-step-text">{step}</span>
          </li>
        ))}
      </ol>
      {pending && (
        <div className="plan-card-actions">
          <button
            type="button"
            className="btn-deny"
            data-testid="plan-revise"
            onClick={() => onDecision?.('revise')}
            disabled={!approvalId}
            aria-label="Revise plan"
          >
            <X size={12} className="btn-icon" />
            <span>Revise</span>
          </button>
          <button
            type="button"
            className="btn-approve btn-approve-glow"
            data-testid="plan-approve"
            onClick={() => onDecision?.('approve')}
            disabled={!approvalId}
            aria-label="Approve plan"
          >
            <Check size={12} className="btn-icon" />
            <span>Approve plan</span>
          </button>
        </div>
      )}
      <p className="plan-card-note">
        {pending
          ? 'Approving unlocks writes, deletes, and commands for this session. Each one still needs its own approval card.'
          : decision === 'approve'
            ? 'The session is executing. Writes and commands still card individually.'
            : 'The session stays in plan mode until you approve a revised plan.'}
      </p>
    </div>
  );
};