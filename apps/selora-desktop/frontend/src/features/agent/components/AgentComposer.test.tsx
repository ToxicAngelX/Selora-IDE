// Selora IDE — AgentComposer Component Tests
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, fireEvent, act } from '@testing-library/react';
import React from 'react';
import { AgentComposer } from './AgentComposer';
import { agentStore } from '../agent-store';

beforeEach(() => {
  agentStore.reset();
});

describe('AgentComposer Component', () => {
  it('submits user message on Enter when not in-flight', () => {
    const sendSpy = vi.spyOn(agentStore, 'sendUserMessage');

    const { getByLabelText } = render(
      <AgentComposer
        inFlight={false}
        streamingActivityLevel={0}
        permissionMode="full"
        reasoningEffort="standard"
        selectedModel="GLM-5.3 CodeFast"
        promptHistory={[]}
      />
    );

    const textarea = getByLabelText('Composer prompt input');
    fireEvent.change(textarea, { target: { value: 'Fix compiler errors in main.rs' } });
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });

    expect(sendSpy).toHaveBeenCalledWith('Fix compiler errors in main.rs');
    sendSpy.mockRestore();
  });

  it('exposes Steer and Queue buttons when typing while in-flight', () => {
    const queueSpy = vi.spyOn(agentStore, 'queuePrompt');
    const steerSpy = vi.spyOn(agentStore, 'steerActiveTurn');

    const { getByLabelText, getByText } = render(
      <AgentComposer
        inFlight={true}
        streamingActivityLevel={0.4}
        permissionMode="full"
        reasoningEffort="standard"
        selectedModel="GLM-5.3 CodeFast"
        promptHistory={[]}
      />
    );

    const textarea = getByLabelText('Composer prompt input');
    fireEvent.change(textarea, { target: { value: 'Only modify frontend files' } });

    // Both Steer and Queue buttons are now exposed in the toolbar
    const steerBtn = getByLabelText('Steer active run');
    const queueBtn = getByLabelText('Queue prompt');
    expect(steerBtn).toBeTruthy();
    expect(queueBtn).toBeTruthy();

    // Verify token pill shows words and estimated tokens
    expect(getByText(/4w · ~/)).toBeTruthy();

    // Clicking Queue invokes queuePrompt
    fireEvent.click(queueBtn);
    expect(queueSpy).toHaveBeenCalledWith('Only modify frontend files', 'full');

    // Re-type and re-query Steer button (re-rendered with non-empty text)
    fireEvent.change(textarea, { target: { value: 'Urgent stop directive' } });
    const steerBtnAfterReType = getByLabelText('Steer active run');
    fireEvent.click(steerBtnAfterReType);
    expect(steerSpy).toHaveBeenCalledWith('Urgent stop directive');

    queueSpy.mockRestore();
    steerSpy.mockRestore();
  });

  it('steers immediately on Ctrl+Enter while typing in-flight', () => {
    const steerSpy = vi.spyOn(agentStore, 'steerActiveTurn');

    const { getByLabelText } = render(
      <AgentComposer
        inFlight={true}
        streamingActivityLevel={0.2}
        permissionMode="full"
        reasoningEffort="standard"
        selectedModel="GLM-5.3 CodeFast"
        promptHistory={[]}
      />
    );

    const textarea = getByLabelText('Composer prompt input');
    fireEvent.change(textarea, { target: { value: 'Skip unit test generation' } });
    fireEvent.keyDown(textarea, { key: 'Enter', ctrlKey: true });

    expect(steerSpy).toHaveBeenCalledWith('Skip unit test generation');
    steerSpy.mockRestore();
  });

  it('restores draft into composer when selora:set-composer-draft event is dispatched', () => {
    const { getByDisplayValue } = render(
      <AgentComposer
        inFlight={false}
        streamingActivityLevel={0}
        permissionMode="full"
        reasoningEffort="standard"
        selectedModel="GLM-5.3 CodeFast"
        promptHistory={[]}
      />
    );

    act(() => {
      window.dispatchEvent(
        new CustomEvent('selora:set-composer-draft', {
          detail: { text: 'Restored prompt from rewind' },
        })
      );
    });

    expect(getByDisplayValue('Restored prompt from rewind')).toBeTruthy();
  });
});
