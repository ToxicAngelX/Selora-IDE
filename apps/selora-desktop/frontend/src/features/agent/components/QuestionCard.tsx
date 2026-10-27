// Selora IDE — structured escalation card (ARCHITECTURE.md §19.4, Phase 9c).
//
// A question card parks the same §17.4 turn an approval card does. The answer
// is not an approval: it is returned to the model verbatim as the
// `ask_user_question` tool result. Options are buttons; free text is available
// only when `allow_free_text` is true.

import React, { useState } from 'react';
import { Check, ShieldAlert } from '../../../components/icons';
import type { QuestionApproval, QuestionResponse } from '../../../bridge/bridge';

export type QuestionCardProps = {
  question: QuestionApproval;
  approvalId?: string;
  /** `true` once the single-use answer was accepted by Rust. */
  answered?: boolean;
  /** While the decision is in flight the buttons are inert. */
  submitting?: boolean;
  /** A refusal from Rust; the card stays parked for a corrected answer. */
  error?: string;
  onAnswer?: (answers: QuestionResponse[], approvalId?: string) => void;
};

const actionButtonStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 'var(--space-1)',
  padding: 'var(--space-1) var(--space-3)',
  borderRadius: 'var(--radius-1)',
  fontFamily: 'var(--font-sans)',
  fontSize: 'var(--type-xs)',
  lineHeight: 'var(--type-xs-lh)',
  fontWeight: 'var(--weight-medium)',
  cursor: 'pointer',
};

export const QuestionCard: React.FC<QuestionCardProps> = ({
  question,
  approvalId,
  answered = false,
  submitting = false,
  error,
  onAnswer,
}) => {
  const [answers, setAnswers] = useState<string[]>(() =>
    question.questions.map(() => ''),
  );

  const setAnswer = (index: number, value: string) => {
    if (answered || submitting) return;
    setAnswers((current) =>
      current.map((answer, position) => (position === index ? value : answer)),
    );
  };

  const complete = question.questions.every(
    (item, index) => answers[index]?.trim().length > 0,
  );

  const submit = () => {
    if (!complete || answered || submitting) return;
    onAnswer?.(
      question.questions.map((item, index) => ({
        question: item.question,
        answer: answers[index]!.trim(),
      })),
      approvalId,
    );
  };

  return (
    <div
      data-testid="question-card"
      data-decision={answered ? 'answered' : 'pending'}
      role="group"
      aria-label="Agent question"
      style={{
        border: '1px solid var(--hairline-active)',
        borderRadius: 'var(--radius-2)',
        background: 'var(--surface-2)',
        color: 'var(--fg-default)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-2)',
          padding: 'var(--space-2) var(--space-3)',
          background: 'var(--surface-3)',
          borderBottom: '1px solid var(--hairline-faint)',
        }}
      >
        <ShieldAlert size={13} />
        <span
          style={{
            fontFamily: 'var(--font-sans)',
            fontSize: 'var(--type-xs)',
            lineHeight: 'var(--type-xs-lh)',
            fontWeight: 'var(--weight-medium)',
            letterSpacing: 'var(--tracking-wide)',
            textTransform: 'uppercase',
            color: 'var(--fg-muted)',
          }}
        >
          Question
        </span>
        {answered && <Check size={12} aria-label="Answered" />}
      </div>

      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 'var(--space-3)',
          padding: 'var(--space-3)',
        }}
      >
        {question.questions.map((item, index) => (
          <fieldset
            key={`${item.question}-${index}`}
            style={{ border: 'none', margin: 0, padding: 0, minWidth: 0 }}
          >
            <legend
              style={{
                marginBottom: 'var(--space-2)',
                fontFamily: 'var(--font-sans)',
                fontSize: 'var(--type-sm)',
                lineHeight: 'var(--type-sm-lh)',
                fontWeight: 'var(--weight-medium)',
              }}
            >
              {item.question}
            </legend>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-1)' }}>
              {item.options.map((option) => {
                const selected = answers[index] === option.label;
                return (
                  <button
                    key={option.label}
                    type="button"
                    role="radio"
                    aria-checked={selected}
                    disabled={answered || submitting}
                    onClick={() => setAnswer(index, option.label)}
                    style={{
                      ...actionButtonStyle,
                      justifyContent: 'flex-start',
                      textAlign: 'left',
                      border: `1px solid ${
                        selected ? 'var(--hairline-active)' : 'var(--hairline-faint)'
                      }`,
                      background: selected ? 'var(--surface-3)' : 'transparent',
                      cursor: answered || submitting ? 'default' : 'pointer',
                    }}
                  >
                    <span>{option.label}</span>
                    {option.description && (
                      <span
                        style={{
                          color: 'var(--fg-muted)',
                          fontWeight: 'var(--weight-regular)',
                        }}
                      >
                        {option.description}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
            {item.allow_free_text && !answered && (
              <input
                type="text"
                aria-label={`Answer ${item.question}`}
                placeholder="Or type an answer"
                value={answers[index]}
                disabled={submitting}
                onChange={(event) => setAnswer(index, event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' && !event.shiftKey) {
                    event.preventDefault();
                    submit();
                  }
                }}
                style={{
                  marginTop: 'var(--space-2)',
                  width: '100%',
                  boxSizing: 'border-box',
                  padding: 'var(--space-1) var(--space-2)',
                  border: '1px solid var(--hairline-faint)',
                  borderRadius: 'var(--radius-1)',
                  background: 'var(--surface-1)',
                  color: 'var(--fg-default)',
                  fontFamily: 'var(--font-sans)',
                  fontSize: 'var(--type-xs)',
                  lineHeight: 'var(--type-xs-lh)',
                }}
              />
            )}
            {answered && answers[index] && (
              <div
                style={{
                  marginTop: 'var(--space-1)',
                  color: 'var(--fg-muted)',
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--type-xs)',
                  lineHeight: 'var(--type-xs-lh)',
                }}
              >
                {answers[index]}
              </div>
            )}
          </fieldset>
        ))}

        {error && (
          <div role="alert" style={{ color: 'var(--err)', fontSize: 'var(--type-xs)' }}>
            {error}
          </div>
        )}

        {!answered && (
          <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
            <button
              type="button"
              className="btn-approve"
              data-testid="question-submit"
              disabled={!complete || submitting}
              onClick={submit}
              aria-label="Send answer"
              style={actionButtonStyle}
            >
              <Check size={12} />
              <span>{submitting ? 'Sending…' : 'Send answer'}</span>
            </button>
          </div>
        )}
      </div>
    </div>
  );
};