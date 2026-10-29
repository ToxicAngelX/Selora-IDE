// Selora IDE — Phase 9a structured question card (ROADMAP §"Phase 9", task 9a).
//
// A question parks the same turn an approval does; it is not a second waiting
// mechanism. The card renders the backend's `AgentQuestionPrompt` verbatim and
// routes the answer through `agentStore.answerQuestion` (the existing
// AgentDecision command plus the additive `resolution` payload).
import React, { useId, useState } from 'react';
import { Check, DirectSend } from '../../../components/icons';
import type { AgentQuestionPrompt, AgentDecisionResolution } from '../../../bridge/bridge';

export type AgentQuestionCardProps = {
  prompt: AgentQuestionPrompt;
  submitting?: boolean;
  error?: string | null;
  resolution?: AgentDecisionResolution;
  onAnswer: (optionId: string | null, freeText: string) => void;
};

function answeredLabel(
  prompt: AgentQuestionPrompt,
  resolution: AgentDecisionResolution | undefined,
): string | null {
  if (!resolution || resolution.kind !== 'question') return null;
  const option = resolution.option_id
    ? prompt.options.find((candidate) => candidate.id === resolution.option_id)
    : undefined;
  if (option && resolution.text) return `${option.label} — ${resolution.text}`;
  if (option) return option.label;
  return resolution.text ?? null;
}

export const AgentQuestionCard: React.FC<AgentQuestionCardProps> = ({
  prompt,
  submitting = false,
  error,
  resolution,
  onAnswer,
}) => {
  const groupName = useId();
  const [selected, setSelected] = useState<string | null>(null);
  const [freeText, setFreeText] = useState('');
  const answered = answeredLabel(prompt, resolution);
  const freeTextAllowed = prompt.allow_free_text === true || prompt.options.length === 0;
  const canSend = !submitting && (selected !== null || freeText.trim() !== '');

  return (
    <section
      className="agent-decision-card agent-question-card agent-card-question"
      aria-label="Agent question"
    >
      <header className="agent-decision-header agent-card-question-header">
        <span className="agent-decision-kind agent-card-kind">Question</span>
        {prompt.header ? (
          <span className="agent-decision-context agent-card-context">{prompt.header}</span>
        ) : null}
      </header>

      <p className="agent-decision-question agent-card-question-text">{prompt.question}</p>

      {answered ? (
        <div className="agent-decision-answered agent-card-answered" role="status">
          <Check size={12} aria-hidden="true" />
          <span>{answered}</span>
        </div>
      ) : (
        <>
          {prompt.options.length > 0 ? (
            <fieldset className="agent-question-options agent-card-options">
              <legend className="agent-question-legend agent-card-legend">Choose an answer</legend>
              {prompt.options.map((option, index) => (
                <label key={option.id} className="agent-question-option agent-card-option">
                  <input
                    type="radio"
                    className="agent-card-option-input"
                    name={groupName}
                    value={option.id}
                    checked={selected === option.id}
                    disabled={submitting}
                    onChange={() => setSelected(option.id)}
                  />
                  <span className="agent-question-option-body agent-card-option-body">
                    <span className="agent-question-option-label agent-card-option-label">
                      {option.label}
                      {index === 0 ? <span className="visually-hidden"> (first option)</span> : null}
                    </span>
                    {option.description ? (
                      <span className="agent-question-option-desc agent-card-option-desc">
                        {option.description}
                      </span>
                    ) : null}
                  </span>
                </label>
              ))}
            </fieldset>
          ) : null}

          {freeTextAllowed ? (
            <label className="agent-question-free agent-card-free">
              <span className="agent-question-free-label agent-card-free-label">
                {prompt.options.length > 0 ? 'Or add detail (optional)' : 'Your answer'}
              </span>
              <textarea
                className="agent-question-textarea agent-card-textarea"
                value={freeText}
                rows={2}
                disabled={submitting}
                placeholder={prompt.free_text_placeholder ?? 'Type an answer…'}
                onChange={(event) => setFreeText(event.target.value)}
              />
            </label>
          ) : null}

          {error ? (
            <p className="agent-decision-error agent-card-error" role="alert">
              {error}
            </p>
          ) : null}

          <div className="agent-decision-actions agent-card-actions-row">
            <button
              type="button"
              className="agent-decision-primary agent-card-primary-btn"
              disabled={!canSend}
              aria-busy={submitting}
              onClick={() => onAnswer(selected, freeText)}
            >
              <DirectSend size={12} aria-hidden="true" />
              <span>{submitting ? 'Sending…' : 'Send answer'}</span>
            </button>
          </div>
        </>
      )}
    </section>
  );
};