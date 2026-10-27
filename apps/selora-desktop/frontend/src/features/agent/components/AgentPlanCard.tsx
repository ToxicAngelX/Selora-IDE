// Selora IDE — Phase 9a plan review card (ROADMAP §"Phase 9", task 9a).
//
// Plan mode's structural gate lives in the runtime (9b); this card is the
// approval surface in front of it. The steps are the backend's
// `AgentPlanPrompt` verbatim. Approve sends the existing AgentDecision permit;
// Request changes sends Deny plus the additive plan resolution note. A build
// without 9b still receives a valid decision, so the parked turn continues.
import React, { useId, useState } from 'react';
import { Check, Edit, X } from '../../../components/icons';
import type { AgentDecisionResolution, AgentPlanPrompt } from '../../../bridge/bridge';

export type AgentPlanCardProps = {
  prompt: AgentPlanPrompt;
  submitting?: boolean;
  error?: string | null;
  resolution?: AgentDecisionResolution;
  onDecide: (decision: 'approve' | 'request_changes', feedback?: string) => void;
  onDeny?: () => void;
};

export const AgentPlanCard: React.FC<AgentPlanCardProps> = ({
  prompt,
  submitting = false,
  error,
  resolution,
  onDecide,
  onDeny,
}) => {
  const feedbackId = useId();
  const [requestingChanges, setRequestingChanges] = useState(false);
  const [feedback, setFeedback] = useState('');
  const planResolution = resolution?.kind === 'plan' ? resolution : null;
  const canRequestChanges = feedback.trim() !== '' && !submitting;

  if (planResolution?.decision === 'approve') {
    return (
      <section className="agent-decision-card agent-plan-card" aria-label="Plan review">
        <header className="agent-decision-header">
          <span className="agent-decision-kind">Plan</span>
          {prompt.title ? <span className="agent-decision-context">{prompt.title}</span> : null}
        </header>
        <div className="agent-decision-answered" role="status">
          <Check size={12} aria-hidden="true" />
          <span>Plan approved</span>
        </div>
      </section>
    );
  }

  if (planResolution?.decision === 'request_changes') {
    return (
      <section className="agent-decision-card agent-plan-card" aria-label="Plan review">
        <header className="agent-decision-header">
          <span className="agent-decision-kind">Plan</span>
          {prompt.title ? <span className="agent-decision-context">{prompt.title}</span> : null}
        </header>
        <div className="agent-decision-answered is-changes" role="status">
          <Edit size={12} aria-hidden="true" />
          <span>Changes requested: {planResolution.feedback}</span>
        </div>
      </section>
    );
  }

  return (
    <section className="agent-decision-card agent-plan-card" aria-label="Plan review">
      <header className="agent-decision-header">
        <span className="agent-decision-kind">Plan review</span>
        {prompt.title ? <span className="agent-decision-context">{prompt.title}</span> : null}
      </header>

      {prompt.summary ? <p className="agent-decision-summary">{prompt.summary}</p> : null}

      {prompt.steps.length === 0 ? (
        <p className="agent-decision-empty">No plan steps were provided.</p>
      ) : (
        <ol className="agent-plan-steps">
          {prompt.steps.map((step, index) => (
            <li key={`${index}-${step.title}`} className="agent-plan-step">
              <span className="agent-plan-step-index" aria-hidden="true">
                {index + 1}
              </span>
              <span className="agent-plan-step-body">
                <span className="agent-plan-step-title">{step.title}</span>
                {step.detail ? <span className="agent-plan-step-detail">{step.detail}</span> : null}
              </span>
            </li>
          ))}
        </ol>
      )}

      {requestingChanges ? (
        <label className="agent-plan-feedback" htmlFor={feedbackId}>
          <span className="agent-plan-feedback-label">What should change?</span>
          <textarea
            id={feedbackId}
            className="agent-question-textarea"
            rows={3}
            value={feedback}
            disabled={submitting}
            placeholder="Describe the revision the agent should make…"
            onChange={(event) => setFeedback(event.target.value)}
          />
        </label>
      ) : null}

      {error ? (
        <p className="agent-decision-error" role="alert">
          {error}
        </p>
      ) : null}

      <div className="agent-decision-actions">
        {onDeny && <button type="button" className="agent-decision-danger" disabled={submitting} onClick={onDeny}>Deny plan</button>}
        {requestingChanges ? (
          <>
            <button
              type="button"
              className="agent-decision-secondary"
              disabled={submitting}
              onClick={() => {
                setRequestingChanges(false);
                setFeedback('');
              }}
            >
              Cancel
            </button>
            <button
              type="button"
              className="agent-decision-danger"
              disabled={!canRequestChanges}
              aria-busy={submitting}
              onClick={() => onDecide('request_changes', feedback)}
            >
              <X size={12} aria-hidden="true" />
              <span>{submitting ? 'Sending…' : 'Send changes'}</span>
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              className="agent-decision-secondary"
              disabled={submitting}
              onClick={() => setRequestingChanges(true)}
            >
              <Edit size={12} aria-hidden="true" />
              <span>Request changes</span>
            </button>
            <button
              type="button"
              className="agent-decision-primary"
              disabled={submitting}
              aria-busy={submitting}
              onClick={() => onDecide('approve')}
            >
              <Check size={12} aria-hidden="true" />
              <span>{submitting ? 'Sending…' : 'Approve plan'}</span>
            </button>
          </>
        )}
      </div>
    </section>
  );
};
