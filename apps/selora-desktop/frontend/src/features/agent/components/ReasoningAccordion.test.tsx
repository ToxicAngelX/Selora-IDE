// Selora IDE — ReasoningAccordion Unit Tests
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import { ReasoningAccordion, extractReasoningSteps } from './ReasoningAccordion';

describe('ReasoningAccordion Component', () => {
  beforeEach(() => {
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
      writable: true,
      configurable: true,
    });
  });

  it('renders thinking process header with elapsed stopwatch duration and shimmer dot', () => {
    const raw = `- Analyzed file requirements\n- Inspected Cargo.toml\n- Formulating patch`;
    const { getByText, getByTestId } = render(
      <ReasoningAccordion
        thought={raw}
        durationSeconds={4.2}
        isStreaming={false}
      />
    );

    const root = getByTestId('reasoning-accordion');
    expect(root).toBeTruthy();
    expect(getByText(/Thought for 4.2s/i)).toBeTruthy();
    expect(getByText(/3 steps/i)).toBeTruthy();
  });

  it('extracts step-by-step thinking breakdown with bullet checkpoints', () => {
    const rawThought = `- Analyzed file requirements\n- Inspected Cargo.toml\n- Formulating patch`;
    const steps = extractReasoningSteps(rawThought);

    expect(steps).toHaveLength(3);
    expect(steps[0].title).toBe('Analyzed file requirements');
    expect(steps[1].title).toBe('Inspected Cargo.toml');
    expect(steps[2].title).toBe('Formulating patch');
  });

  it('collapses and expands on header button click', () => {
    const raw = `- Analyzed file requirements\n- Inspected Cargo.toml\n- Formulating patch`;
    const { getByRole, getByText, queryByText } = render(
      <ReasoningAccordion
        thought={raw}
        durationSeconds={3.5}
        defaultExpanded={false}
      />
    );

    // Initially collapsed
    expect(queryByText('Analyzed file requirements')).toBeNull();

    // Click to expand
    const headerBtn = getByRole('button', { name: /Thought for 3.5s/i });
    fireEvent.click(headerBtn);

    // Checkpoints now visible
    expect(getByText('Analyzed file requirements')).toBeTruthy();
    expect(getByText('Inspected Cargo.toml')).toBeTruthy();
    expect(getByText('Formulating patch')).toBeTruthy();
  });

  it('copies thought trace to clipboard on copy action click', async () => {
    const thoughtText = 'Detailed thinking trace about AST and token streaming.';
    const { getByLabelText } = render(
      <ReasoningAccordion thought={thoughtText} durationSeconds={2.1} />
    );

    const copyBtn = getByLabelText('Copy reasoning');
    fireEvent.click(copyBtn);

    await waitFor(() => {
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith(thoughtText);
    });
  });
});
