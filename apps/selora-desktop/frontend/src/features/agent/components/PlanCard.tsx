// Compatibility adapter. All plan review rendering lives in AgentPlanCard.
import React from 'react';
import { AgentPlanCard } from './AgentPlanCard';

export type PlanDecision = 'approve' | 'revise' | 'deny';
export type PlanCardProps = {
  summary?: string | null;
  steps: string[];
  decision: PlanDecision | null;
  approvalId?: string;
  onDecision?: (decision: PlanDecision) => void;
};

export const PlanCard: React.FC<PlanCardProps> = ({ summary, steps, decision, approvalId, onDecision }) => (
  <AgentPlanCard
    prompt={{ kind: 'plan', summary: summary ?? undefined, steps: steps.map((title) => ({ title })) }}
    submitting={!approvalId}
    resolution={decision === null ? undefined : {
      kind: 'plan', decision: decision === 'approve' ? 'approve' : 'request_changes',
      feedback: decision === 'deny' ? 'Plan denied' : 'Revision requested',
    }}
    onDecide={(answer) => onDecision?.(answer === 'approve' ? 'approve' : 'revise')}
    onDeny={decision === null ? () => onDecision?.('deny') : undefined}
  />
);
