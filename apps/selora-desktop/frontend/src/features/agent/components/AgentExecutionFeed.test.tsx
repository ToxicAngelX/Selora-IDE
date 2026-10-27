// Selora IDE — AgentExecutionFeed Component Tests
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { AgentExecutionFeed } from './AgentExecutionFeed';
import { agentStore } from '../agent-store';
import type { ChatMessage, AgentMetrics, QueuedPrompt } from '../types';

const scrollIntoView = vi.fn();

beforeEach(() => {
  scrollIntoView.mockClear();
  window.HTMLElement.prototype.scrollIntoView = scrollIntoView;
  agentStore.reset();
});

const defaultMetrics: AgentMetrics = {
  activeToolsCount: 0,
  approvedCount: 0,
  deniedCount: 0,
  totalTokens: 0,
  costUsd: 0,
  latencyMs: 0,
};

const createTestUserMessage = (
  id: string,
  content: string,
  extra: Partial<ChatMessage> = {}
): ChatMessage => ({
  id,
  role: 'user',
  content,
  timestamp: Date.now(),
  status: 'complete',
  ...extra,
});

describe('AgentExecutionFeed Component', () => {
  it('renders the empty hero state when there are no messages', () => {
    const { getByText } = render(
      <AgentExecutionFeed
        messages={[]}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    expect(getByText('Ask anything')).toBeTruthy();
    expect(getByText(/Ask about the code, or tell Selora what to change/i)).toBeTruthy();
    expect(getByText('Explain this workspace architecture')).toBeTruthy();
  });

  it('exposes the transcript as a polite log and follows output smoothly', () => {
    const messages: ChatMessage[] = [createTestUserMessage('msg-u1', 'Streaming reply')];

    const { getByRole } = render(
      <AgentExecutionFeed
        messages={messages}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    const log = getByRole('log', { name: 'Conversation Feed' });
    expect(log.getAttribute('aria-live')).toBe('polite');
    expect(log.getAttribute('aria-relevant')).toBe('additions text');
    expect(scrollIntoView).toHaveBeenCalledWith({ behavior: 'smooth' });
  });

  it('uses instant scrolling when reduced motion is requested', () => {
    document.documentElement.setAttribute('data-reduced-motion', 'true');
    try {
      const messages: ChatMessage[] = [createTestUserMessage('msg-u1', 'Reduced motion reply')];

      render(
        <AgentExecutionFeed
          messages={messages}
          inFlight={false}
          thinkingState="breathing"
          metrics={defaultMetrics}
        />
      );

      expect(scrollIntoView).toHaveBeenCalledWith({ behavior: 'auto' });
    } finally {
      document.documentElement.removeAttribute('data-reduced-motion');
    }
  });

  it('does not force-scroll when the reader is away from the bottom', () => {
    const first: ChatMessage[] = [createTestUserMessage('msg-u1', 'First message')];
    const { container, rerender } = render(
      <AgentExecutionFeed
        messages={first}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    const list = container.querySelector('.feed-messages-list') as HTMLElement;
    Object.defineProperty(list, 'scrollHeight', { configurable: true, value: 1000 });
    Object.defineProperty(list, 'clientHeight', { configurable: true, value: 200 });
    Object.defineProperty(list, 'scrollTop', { configurable: true, value: 100 });
    fireEvent.scroll(list);
    scrollIntoView.mockClear();

    rerender(
      <AgentExecutionFeed
        messages={[...first, createTestUserMessage('msg-u2', 'Second message')]}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    expect(scrollIntoView).not.toHaveBeenCalled();

    // Scrolling back to the bottom resumes following the stream.
    Object.defineProperty(list, 'scrollTop', { configurable: true, value: 800 });
    fireEvent.scroll(list);
    rerender(
      <AgentExecutionFeed
        messages={[
          ...first,
          createTestUserMessage('msg-u2', 'Second message'),
          createTestUserMessage('msg-u3', 'Third message'),
        ]}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    expect(scrollIntoView).toHaveBeenCalledWith({ behavior: 'smooth' });
  });

  it('renders user prompt and the micro-action toolbar (Copy, Rewind, Edit, Fork)', () => {
    const messages: ChatMessage[] = [
      createTestUserMessage('msg-u1', 'Refactor auth service'),
    ];

    const { getByText, getByRole, getByLabelText } = render(
      <AgentExecutionFeed
        messages={messages}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    expect(getByText('Refactor auth service')).toBeTruthy();
    expect(getByRole('toolbar', { name: 'Prompt actions' })).toBeTruthy();
    expect(getByLabelText('Copy prompt')).toBeTruthy();
    expect(getByLabelText('Rewind here')).toBeTruthy();
    expect(getByLabelText('Edit prompt')).toBeTruthy();
    expect(getByLabelText('Retry from this prompt')).toBeTruthy();
  });

  it('triggers copyPrompt on copy button click', async () => {
    const copySpy = vi.spyOn(agentStore, 'copyPrompt').mockResolvedValue(true);

    const messages: ChatMessage[] = [
      createTestUserMessage('msg-u1', 'Build test matrix'),
    ];

    const { getByLabelText } = render(
      <AgentExecutionFeed
        messages={messages}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    const copyBtn = getByLabelText('Copy prompt');
    fireEvent.click(copyBtn);

    expect(copySpy).toHaveBeenCalledWith('Build test matrix');
    copySpy.mockRestore();
  });

  it('triggers rewindToPrompt on rewind button click', () => {
    const rewindSpy = vi.spyOn(agentStore, 'rewindToPrompt');

    const messages: ChatMessage[] = [
      createTestUserMessage('msg-u1', 'First step in pipeline'),
    ];

    const { getByLabelText } = render(
      <AgentExecutionFeed
        messages={messages}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    const rewindBtn = getByLabelText('Rewind here');
    fireEvent.click(rewindBtn);

    expect(rewindSpy).toHaveBeenCalledWith('msg-u1', true);
    rewindSpy.mockRestore();
  });

  it('supports inline editing mode and resubmission', () => {
    const resubmitSpy = vi.spyOn(agentStore, 'resubmitEditedPrompt');

    const messages: ChatMessage[] = [
      createTestUserMessage('msg-u1', 'Original prompt text'),
    ];

    const { getByLabelText, getByRole, getByDisplayValue } = render(
      <AgentExecutionFeed
        messages={messages}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    // Click Edit button
    const editBtn = getByLabelText('Edit prompt');
    fireEvent.click(editBtn);

    // Textarea is shown with original prompt
    const textarea = getByDisplayValue('Original prompt text');
    expect(textarea).toBeTruthy();

    // Type new text
    fireEvent.change(textarea, { target: { value: 'Updated prompt text' } });

    // Click Save & Resubmit
    const submitBtn = getByRole('button', { name: /Save & Resubmit/i });
    fireEvent.click(submitBtn);

    expect(resubmitSpy).toHaveBeenCalledWith('msg-u1', 'Updated prompt text');
    resubmitSpy.mockRestore();
  });

  it('triggers retryMessage on retry button click', () => {
    const retrySpy = vi.spyOn(agentStore, 'retryMessage').mockResolvedValue(true);

    const messages: ChatMessage[] = [
      createTestUserMessage('msg-u1', 'Retry from here'),
    ];

    const { getByLabelText } = render(
      <AgentExecutionFeed
        messages={messages}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    const retryBtn = getByLabelText('Retry from this prompt');
    fireEvent.click(retryBtn);

    expect(retrySpy).toHaveBeenCalledWith('msg-u1');
    retrySpy.mockRestore();
  });

  it('displays steer directive badge for steered prompts', () => {
    const messages: ChatMessage[] = [
      createTestUserMessage('msg-s1', 'Stop and switch to CSS only', { isSteer: true }),
    ];

    const { getByText } = render(
      <AgentExecutionFeed
        messages={messages}
        inFlight={false}
        thinkingState="breathing"
        metrics={defaultMetrics}
      />
    );

    expect(getByText('Steer Directive')).toBeTruthy();
  });

  it('renders prompt queue tray and handles Steer Now, Edit, and Remove actions', () => {
    const removeSpy = vi.spyOn(agentStore, 'removeQueuedPrompt');
    const steerSpy = vi.spyOn(agentStore, 'steerActiveTurn');
    const clearSpy = vi.spyOn(agentStore, 'clearPromptQueue');

    const queuedPrompts: QueuedPrompt[] = [
      {
        id: 'q-1',
        text: 'Run e2e suite after build',
        createdAt: Date.now(),
      },
      {
        id: 'q-2',
        text: 'Check memory footprint',
        createdAt: Date.now(),
      },
    ];

    const messages: ChatMessage[] = [
      createTestUserMessage('msg-u1', 'Build project'),
    ];

    const { getByText, getByTitle, getAllByLabelText } = render(
      <AgentExecutionFeed
        messages={messages}
        inFlight={true}
        thinkingState="working"
        metrics={defaultMetrics}
        promptQueue={queuedPrompts}
      />
    );

    expect(getByText('Prompt Queue')).toBeTruthy();
    expect(getByText('#1')).toBeTruthy();
    expect(getByText('Run e2e suite after build')).toBeTruthy();
    expect(getByText('#2')).toBeTruthy();
    expect(getByText('Check memory footprint')).toBeTruthy();

    // Test Steer now button on first item
    const steerNowBtns = getAllByLabelText('Steer now');
    expect(steerNowBtns).toHaveLength(2);
    fireEvent.click(steerNowBtns[0]);
    expect(removeSpy).toHaveBeenCalledWith('q-1');
    expect(steerSpy).toHaveBeenCalledWith('Run e2e suite after build');

    // Test Clear queue button
    const clearBtn = getByTitle('Clear all queued prompts');
    fireEvent.click(clearBtn);
    expect(clearSpy).toHaveBeenCalled();

    removeSpy.mockRestore();
    steerSpy.mockRestore();
    clearSpy.mockRestore();
  });
});
