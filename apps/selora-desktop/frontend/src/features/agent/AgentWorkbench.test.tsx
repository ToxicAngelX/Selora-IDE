// Selora IDE — Phase 9a Agent Workbench tests.
//
// Covers the acceptance seams for the dedicated agent surface: backend session
// tabs + real NewSession/SwitchSession commands, truthful empty/live todos,
// structured question/plan cards through AgentDecision, the header Stop, the
// composer mode row, and session switching through the keyboard.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, waitFor, within } from '@testing-library/react';
import React from 'react';
import { AgentDock } from '../../layout/AgentDock';
import { agentStore } from './agent-store';
import { makeBootstrap } from '../../testing/fixtures';
import type { BootstrapState } from '../../bridge/bridge';

const bridgeMocks = vi.hoisted(() => ({
  newSession: vi.fn(() => Promise.resolve()),
  switchSession: vi.fn(() => Promise.resolve()),
  sendAgent: vi.fn(() => ({ requestId: 'req-agent', promise: Promise.resolve() })),
  sendChat: vi.fn(() => ({ requestId: 'req-chat', promise: Promise.resolve() })),
  stopChat: vi.fn(() => Promise.resolve()),
  agentDecision: vi.fn(() => Promise.resolve()),
  runWorkflow: vi.fn(() => ({ requestId: 'req-workflow', promise: Promise.resolve() })),
  fetchModels: vi.fn(() => Promise.resolve([])),
}));

vi.mock('../../bridge/bridge', () => bridgeMocks);

type ChatState = BootstrapState['chat'];

function hydrate(chat: ChatState) {
  agentStore.reset();
  agentStore.hydrate(
    makeBootstrap({
      models: [{ id: 'model-test', display_name: 'Test Model', available: true }],
      active_model_id: 'model-test',
      workspace: {
        id: 'ws-1',
        core_instance_id: 'core-0001',
        root: '/repo',
        display_name: 'repo',
      },
      chat,
    }),
  );
}

const SESSIONS = [
  { session_id: 'sess-a', title: 'First session', message_ids: ['a1'], active: false },
  { session_id: 'sess-b', title: 'Second session', message_ids: ['b1'], active: false },
];

const MESSAGES = [
  { message_id: 'a1', session_id: 'sess-a', role: 'assistant' as const, text: 'first transcript' },
  { message_id: 'b1', session_id: 'sess-b', role: 'assistant' as const, text: 'second transcript' },
];

function baseChat(overrides: Partial<ChatState> = {}): ChatState {
  return {
    sessions: SESSIONS,
    messages: MESSAGES,
    active_session_id: 'sess-a',
    active_request_id: null,
    active_turn_id: null,
    pending_approval: null,
    ...overrides,
  };
}

function pendingQuestion(sessionId = 'sess-a', requestId = 'req-q') {
  return {
    approval_id: 'appr-q',
    request_id: requestId,
    turn_id: 'turn-q',
    session_id: sessionId,
    summary: 'ask_user_question',
    can_approve_all_this_turn: false,
    decision_prompt: {
      kind: 'question' as const,
      header: 'Need context',
      question: 'Which file should change?',
      options: [
        { id: 'opt-main', label: 'src/main.rs' },
        { id: 'opt-lib', label: 'src/lib.rs' },
      ],
      allow_free_text: true,
      free_text_placeholder: 'Other file',
    },
  };
}

function pendingPlan(sessionId = 'sess-a', requestId = 'req-p') {
  return {
    approval_id: 'appr-p',
    request_id: requestId,
    turn_id: 'turn-p',
    session_id: sessionId,
    summary: 'enter_plan_mode',
    can_approve_all_this_turn: false,
    decision_prompt: {
      kind: 'plan' as const,
      title: 'Fix the TODO',
      summary: 'Replace the marker and verify.',
      steps: [
        { title: 'Read src/main.rs', detail: 'Locate the TODO' },
        { title: 'Apply the fix' },
      ],
    },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  bridgeMocks.newSession.mockImplementation(() => Promise.resolve());
  bridgeMocks.switchSession.mockImplementation(() => Promise.resolve());
  bridgeMocks.stopChat.mockImplementation(() => Promise.resolve());
  bridgeMocks.agentDecision.mockImplementation(() => Promise.resolve());
});

describe('Agent workbench session tabs', () => {
  it('reads sessions from backend chat state and switches with the real command', async () => {
    hydrate(baseChat());
    const { getByRole, getByText } = render(<AgentDock />);

    const first = getByRole('tab', { name: /First session/ });
    const second = getByRole('tab', { name: /Second session/ });
    expect(first.getAttribute('aria-selected')).toBe('true');
    expect(second.getAttribute('aria-selected')).toBe('false');
    expect(getByText('first transcript')).toBeTruthy();

    fireEvent.click(second);
    await waitFor(() =>
      expect(bridgeMocks.switchSession).toHaveBeenCalledWith(
        'sess-b',
        expect.objectContaining({ core_instance_id: 'core-0001', workspace_id: 'ws-1' }),
      ),
    );
    expect(agentStore.getSnapshot().activeSessionId).toBe('sess-b');
    expect(second.getAttribute('aria-selected')).toBe('true');
    expect(getByText('second transcript')).toBeTruthy();
  });

  it('moves between tabs with Left/Right and Home/End (roving tabindex)', async () => {
    hydrate(baseChat());
    const { getByRole } = render(<AgentDock />);

    const first = getByRole('tab', { name: /First session/ });
    const second = getByRole('tab', { name: /Second session/ });
    expect(first.getAttribute('tabindex')).toBe('0');
    expect(second.getAttribute('tabindex')).toBe('-1');

    fireEvent.keyDown(first, { key: 'ArrowRight' });
    await waitFor(() => expect(agentStore.getSnapshot().activeSessionId).toBe('sess-b'));
    expect(second.getAttribute('tabindex')).toBe('0');
    expect(bridgeMocks.switchSession).toHaveBeenCalledWith('sess-b', expect.anything());

    fireEvent.keyDown(second, { key: 'Home' });
    await waitFor(() => expect(agentStore.getSnapshot().activeSessionId).toBe('sess-a'));
    expect(bridgeMocks.switchSession).toHaveBeenCalledWith('sess-a', expect.anything());
  });

  it('creates a session only through NewSession and never fabricates one', async () => {
    hydrate(baseChat());
    const { getByRole } = render(<AgentDock />);

    fireEvent.click(getByRole('button', { name: 'New session' }));
    await waitFor(() => expect(bridgeMocks.newSession).toHaveBeenCalledTimes(1));
    expect(bridgeMocks.newSession).toHaveBeenCalledWith(
      expect.objectContaining({ core_instance_id: 'core-0001', session_id: null }),
    );
    // No local placeholder: the authoritative bootstrap event owns the rail.
    expect(agentStore.getSnapshot().sessions).toHaveLength(2);
  });

  it('surfaces an unsupported backend command and keeps the session list truthful', async () => {
    bridgeMocks.newSession.mockRejectedValueOnce({
      code: 'unsupported_phase_slice',
      message: 'NewSession is not part of this slice',
      retryable: false,
    });
    hydrate(baseChat({ sessions: [], messages: [], active_session_id: null }));
    const { getByRole, findByRole, getByText } = render(<AgentDock />);

    expect(getByText('No sessions yet')).toBeTruthy();
    fireEvent.click(getByRole('button', { name: 'New session' }));

    const alert = await findByRole('alert');
    expect(alert.textContent).toContain('not supported by this backend build');
    expect(agentStore.getSnapshot().sessions).toEqual([]);
    expect(getByText('No sessions yet')).toBeTruthy();
  });
});

describe('Agent workbench todos', () => {
  it('says "No todos yet" when the session has no authoritative list', () => {
    hydrate(baseChat());
    const { getByRole, getAllByText } = render(<AgentDock />);
    const spine = getByRole('region', { name: 'Session todos' });
    expect(within(spine).getByText('No todos yet')).toBeTruthy();
    expect(getAllByText('No todos yet')).toHaveLength(1);
  });

  it('renders the backend todo list and progress without inventing steps', () => {
    hydrate(
      baseChat({
        todos: [
          { id: 't1', session_id: 'sess-a', title: 'Read the roadmap', status: 'completed' },
          { id: 't2', session_id: 'sess-a', title: 'Wire the workbench', status: 'in_progress' },
        ],
      }),
    );
    const { getByRole, getByText, queryByText } = render(<AgentDock />);
    const spine = getByRole('region', { name: 'Session todos' });
    expect(within(spine).getByText('Read the roadmap')).toBeTruthy();
    expect(within(spine).getByText('Wire the workbench')).toBeTruthy();
    expect(within(spine).getByRole('progressbar', { name: 'Todo progress: 50%' })).toBeTruthy();
    expect(queryByText('No todos yet')).toBeNull();
    expect(within(spine).getByText('1/2')).toBeTruthy();
  });
});

describe('Agent workbench question card', () => {
  it('renders options + free text and sends the chosen answer through AgentDecision', async () => {
    hydrate(
      baseChat({
        active_request_id: 'req-q',
        pending_approval: pendingQuestion(),
      }),
    );
    const { getByRole, getByText } = render(<AgentDock />);

    expect(getByText('Which file should change?')).toBeTruthy();
    fireEvent.click(getByRole('radio', { name: /src\/main.rs/ }));
    fireEvent.change(getByRole('textbox', { name: /Or add detail/ }), {
      target: { value: 'keep the header' },
    });
    fireEvent.click(getByRole('button', { name: 'Send answer' }));

    await waitFor(() =>
      expect(bridgeMocks.agentDecision).toHaveBeenCalledWith(
        'req-q',
        'appr-q',
        'answer',
        expect.objectContaining({ session_id: 'sess-a' }),
        { kind: 'question', option_id: 'opt-main', text: 'keep the header' },
      ),
    );
    expect(await getByText(/src\/main\.rs — keep the header/)).toBeTruthy();
  });

  it('accepts a free-text-only question and blocks an empty answer', async () => {
    hydrate(
      baseChat({
        active_request_id: 'req-q',
        pending_approval: {
          ...pendingQuestion(),
          decision_prompt: {
            kind: 'question' as const,
            question: 'Describe the fix',
            options: [],
            allow_free_text: true,
          },
        },
      }),
    );
    const { getByRole, getByText } = render(<AgentDock />);

    const send = getByRole('button', { name: 'Send answer' }) as HTMLButtonElement;
    expect(send.disabled).toBe(true);

    fireEvent.change(getByRole('textbox', { name: 'Your answer' }), {
      target: { value: 'Render the plan inline' },
    });
    fireEvent.click(send);
    await waitFor(() => expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(1));
    expect(bridgeMocks.agentDecision).toHaveBeenCalledWith(
      'req-q',
      'appr-q',
      'answer',
      expect.anything(),
      { kind: 'question', text: 'Render the plan inline' },
    );
    expect(getByText(/Render the plan inline/)).toBeTruthy();
  });
});

describe('Agent workbench plan card', () => {
  it('renders real plan steps and approves through AgentDecision', async () => {
    hydrate(
      baseChat({
        active_request_id: 'req-p',
        pending_approval: pendingPlan(),
      }),
    );
    const { getByRole, getByText } = render(<AgentDock />);

    expect(getByText('Read src/main.rs')).toBeTruthy();
    expect(getByText('Apply the fix')).toBeTruthy();
    fireEvent.click(getByRole('button', { name: 'Approve plan' }));

    await waitFor(() =>
      expect(bridgeMocks.agentDecision).toHaveBeenCalledWith(
        'req-p',
        'appr-p',
        'approve',
        expect.objectContaining({ session_id: 'sess-a' }),
        { kind: 'plan', decision: 'approve' },
      ),
    );
    expect(getByText('Plan approved')).toBeTruthy();
  });

  it('requests changes with a typed revision + feedback payload', async () => {
    hydrate(
      baseChat({
        active_request_id: 'req-p',
        pending_approval: pendingPlan(),
      }),
    );
    const { getByRole, getByText } = render(<AgentDock />);

    fireEvent.click(getByRole('button', { name: 'Request changes' }));
    const send = getByRole('button', { name: 'Send changes' }) as HTMLButtonElement;
    expect(send.disabled).toBe(true);

    fireEvent.change(getByRole('textbox', { name: 'What should change?' }), {
      target: { value: 'Split the edit into two steps' },
    });
    fireEvent.click(send);

    await waitFor(() =>
      expect(bridgeMocks.agentDecision).toHaveBeenCalledWith(
        'req-p',
        'appr-p',
        'revise',
        expect.anything(),
        {
          kind: 'plan',
          decision: 'request_changes',
          feedback: 'Split the edit into two steps',
        },
      ),
    );
    expect(getByText(/Changes requested: Split the edit into two steps/)).toBeTruthy();
  });
});

describe('Agent workbench run controls', () => {
  it('stops the live turn from the header with the real StopChat command', async () => {
    hydrate(baseChat({ active_request_id: 'req-live' }));
    const { getByRole } = render(<AgentDock />);

    const stop = getByRole('button', { name: 'Stop agent run' }) as HTMLButtonElement;
    expect(stop.disabled).toBe(false);
    fireEvent.click(stop);

    await waitFor(() =>
      expect(bridgeMocks.stopChat).toHaveBeenCalledWith(
        'req-live',
        expect.objectContaining({ session_id: 'sess-a' }),
      ),
    );
    expect(agentStore.getSnapshot().inFlight).toBe(false);
  });

  it('maps the composer Ask / Agent / Plan row to real session policies', () => {
    hydrate(baseChat());
    const { getByRole } = render(<AgentDock />);
    const row = getByRole('group', { name: 'Composer mode' });

    fireEvent.click(within(row).getByRole('button', { name: 'Plan' }));
    expect(agentStore.getSnapshot().permissionMode).toBe('plan_only');
    expect(within(row).getByRole('button', { name: 'Plan' }).getAttribute('aria-pressed')).toBe('true');

    fireEvent.click(within(row).getByRole('button', { name: 'Ask' }));
    expect(agentStore.getSnapshot().permissionMode).toBe('ask_before_change');

    fireEvent.click(within(row).getByRole('button', { name: 'Agent' }));
    expect(agentStore.getSnapshot().permissionMode).toBe('full');
  });
});
