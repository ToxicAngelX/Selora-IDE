// Selora IDE — QuestionCard tests (Phase 9c, ARCHITECTURE.md §19.4)
//
// The card must show every option, accept free text only when allowed, and
// call the answer route with the verbatim answer.
import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { QuestionCard } from './QuestionCard';
import type { QuestionApproval } from '../../../bridge/bridge';

const question: QuestionApproval = {
  questions: [
    {
      question: 'Which file should I change?',
      options: [
        { label: 'src/a.rs', description: 'the parser' },
        { label: 'src/b.rs', description: 'the router' },
      ],
      allow_free_text: true,
    },
  ],
};

describe('QuestionCard', () => {
  it('renders the options and submits a selected option', () => {
    const onAnswer = vi.fn();
    const { getByTestId, getByText } = render(
      <QuestionCard question={question} approvalId="appr-q1" onAnswer={onAnswer} />,
    );

    expect(getByTestId('question-card').getAttribute('data-decision')).toBe('pending');
    expect(getByText('Which file should I change?')).toBeTruthy();
    expect(getByText('the parser')).toBeTruthy();
    expect(getByText('the router')).toBeTruthy();

    fireEvent.click(getByText('src/b.rs'));
    fireEvent.click(getByTestId('question-submit'));

    expect(onAnswer).toHaveBeenCalledWith(
      [{ question: 'Which file should I change?', answer: 'src/b.rs' }],
      'appr-q1',
    );
  });

  it('accepts free text when allowed', () => {
    const onAnswer = vi.fn();
    const { getByLabelText, getByTestId } = render(
      <QuestionCard question={question} approvalId="appr-q2" onAnswer={onAnswer} />,
    );

    fireEvent.change(getByLabelText('Answer Which file should I change?'), {
      target: { value: 'src/other.rs' },
    });
    fireEvent.click(getByTestId('question-submit'));

    expect(onAnswer).toHaveBeenCalledWith(
      [{ question: 'Which file should I change?', answer: 'src/other.rs' }],
      'appr-q2',
    );
  });

  it('hides free text when the model disabled it', () => {
    const noFreeText: QuestionApproval = {
      questions: [{ ...question.questions[0]!, allow_free_text: false }],
    };
    const { queryByLabelText } = render(
      <QuestionCard question={noFreeText} approvalId="appr-q3" />,
    );
    expect(queryByLabelText('Answer Which file should I change?')).toBeNull();
  });

  it('never submits twice once answered', () => {
    const onAnswer = vi.fn();
    const { queryByTestId } = render(
      <QuestionCard question={question} approvalId="appr-q4" answered onAnswer={onAnswer} />,
    );
    expect(queryByTestId('question-submit')).toBeNull();
    expect(onAnswer).not.toHaveBeenCalled();
  });
});