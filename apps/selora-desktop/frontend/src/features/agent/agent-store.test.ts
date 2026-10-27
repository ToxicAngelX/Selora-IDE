// Selora IDE — Phase Z3 Agent Store Tests
import { beforeEach, describe, it, expect, vi } from 'vitest';
import type { Approval, BootstrapState, EnvelopeContext } from '../../bridge/bridge';

const bridgeMocks = vi.hoisted(() => {
  let requestSeq = 0;
  const nextRequestId = (prefix: string) => {
    requestSeq += 1;
    return `${prefix}-${requestSeq}`;
  };
  return {
    sendAgent: vi.fn((..._args: unknown[]) => ({ requestId: nextRequestId('req-agent'), promise: Promise.resolve() })),
    sendChat: vi.fn((..._args: unknown[]) => ({ requestId: nextRequestId('req-chat'), promise: Promise.resolve() })),
    stopChat: vi.fn((..._args: unknown[]) => Promise.resolve()),
    agentDecision: vi.fn((..._args: unknown[]) => Promise.resolve()),
    runWorkflow: vi.fn((..._args: unknown[]) => ({
      requestId: nextRequestId('req-workflow'),
      promise: Promise.resolve(),
    })),
    fetchModels: vi.fn((..._args: unknown[]) => Promise.resolve([])),
  };
});

vi.mock('../../bridge/bridge', () => bridgeMocks);

import { agentStore, getComposerGate, toAgentMode, type WorkflowToolCall } from './agent-store';

function deferred() {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<void>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const WORKFLOW = { name: 'audit', script: "phase('scan');\n", commands: ['cargo test'], max_cost_usd: 5 };

function parkWorkflow() {
  agentStore.sendUserMessage('Audit this workspace');
  const requestId = latestAgentRequestId();
  agentStore.handleApprovalNeeded({
    request_id: requestId, session_id: 'sess-hydrated', turn_id: 'turn-wf', approval_id: 'appr-wf',
    tool_name: 'run_workflow', path_or_command: 'audit', workflow: WORKFLOW,
  });
  return requestId;
}

function workflowCard(): WorkflowToolCall | undefined {
  return agentStore.getSnapshot().messages.flatMap((message) => message.toolCalls ?? [])
    .find((call) => call.approvalId === 'appr-wf');
}

function hydratePendingWorkflow(requestId = 'req-parked', summary = 'run_workflow audit') {
  const bootstrap = makeAgentBootstrap();
  bootstrap.chat.active_request_id = requestId;
  bootstrap.chat.pending_approval = {
    request_id: requestId, session_id: 'sess-hydrated', turn_id: 'turn-wf', approval_id: 'appr-wf',
    summary, can_approve_all_this_turn: true,
  };
  agentStore.hydrate(bootstrap);
}

function deliverWorkflowApproval(requestId: string) {
  agentStore.handleApprovalNeeded({
    request_id: requestId, session_id: 'sess-hydrated', turn_id: 'turn-wf', approval_id: 'appr-wf',
    tool_name: 'run_workflow', path_or_command: 'audit', workflow: WORKFLOW,
  });
}

function latestAgentRequestId(): string {
  const results = bridgeMocks.sendAgent.mock.results;
  const value = results[results.length - 1]?.value as { requestId?: string } | undefined;
  return value?.requestId ?? '';
}

/** Fresh conversation, but still connected to a real workspace + catalog. */
function resetAndHydrate(sessionId = 'sess-hydrated') {
  agentStore.reset();
  agentStore.hydrate(makeAgentBootstrap(sessionId));
}

beforeEach(() => {
  vi.clearAllMocks();
  agentStore.reset();
  // A hydrated store: real workspace + real catalog model, no custom override.
  agentStore.updateModelSettings({ customModelId: '' });
  agentStore.hydrate(makeAgentBootstrap());
});

describe('workflow confirmation handoff', () => {
  it('rejects Deny after a run is accepted and only retries recording approval', async () => {
    const requestId = parkWorkflow();
    bridgeMocks.agentDecision.mockRejectedValueOnce(new Error('Decision unavailable'));
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    await agentStore.handleApprovalDecision('appr-wf', 'deny');
    expect(workflowCard()?.status).toBe('needs_approval');
    expect(workflowCard()?.resultSummary).toContain('already started and cannot be denied');
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(1);
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.agentDecision).toHaveBeenLastCalledWith(requestId, 'appr-wf', 'approve',
      expect.objectContaining({ session_id: 'sess-hydrated' }));
    expect(workflowCard()?.status).toBe('approved');
  });

  it('hydrates workflow identity but blocks decisions until the full approval arrives', async () => {
    agentStore.reset();
    hydratePendingWorkflow();
    expect(workflowCard()?.toolName).toBe('run_workflow');
    expect(workflowCard()?.workflow).toBeUndefined();
    expect(workflowCard()?.resultSummary).toContain('full approval_needed payload');
    for (const decision of ['approve', 'deny', 'approve_all'] as const) {
      await agentStore.handleApprovalDecision('appr-wf', decision);
    }
    expect(bridgeMocks.agentDecision).not.toHaveBeenCalled();
    expect(bridgeMocks.runWorkflow).not.toHaveBeenCalled();
    expect(workflowCard()?.status).toBe('needs_approval');

    deliverWorkflowApproval('req-parked');
    expect(workflowCard()?.workflow).toEqual(WORKFLOW);
    expect(agentStore.getSnapshot().messages.flatMap((message) => message.toolCalls ?? [])).toHaveLength(1);
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.agentDecision).toHaveBeenCalledWith('req-parked', 'appr-wf', 'approve',
      expect.objectContaining({ session_id: 'sess-hydrated' }));
  });

  it('preserves known workflow identity and accepted-run history across hydration without trusting the summary', async () => {
    const requestId = parkWorkflow();
    bridgeMocks.agentDecision.mockRejectedValueOnce(new Error('Decision unavailable'));
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    hydratePendingWorkflow(requestId, 'Audit pending');
    expect(workflowCard()?.toolName).toBe('run_workflow');
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(1);
    expect(workflowCard()?.resultSummary).toContain('authoritative workflow approval is missing');
    deliverWorkflowApproval(requestId);
    await agentStore.handleApprovalDecision('appr-wf', 'deny');
    expect(workflowCard()?.resultSummary).toContain('already started and cannot be denied');
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(2);
    expect(workflowCard()?.status).toBe('approved');
  });

  it('keeps pending workflow confirmation intact when an unrelated chat failure arrives', async () => {
    agentStore.sendUserMessage('Old turn');
    const oldRequestId = latestAgentRequestId();
    agentStore.completeActiveTurn();
    const requestId = parkWorkflow();
    const snapshot = agentStore.getSnapshot();
    agentStore.handleChatFailed({ request_id: oldRequestId, session_id: 'sess-hydrated', error: { message: 'Late failure' } });
    agentStore.handleChatFailed({ request_id: requestId, session_id: 'sess-other', error: { message: 'Other session failure' } });
    expect(agentStore.getSnapshot()).toBe(snapshot);
    expect(agentStore.getSnapshot().inFlight).toBe(true);
    expect(workflowCard()?.status).toBe('needs_approval');
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(1);
    expect(workflowCard()?.status).toBe('approved');
  });

  it('still ends the matching turn on its own chat failure', () => {
    const requestId = parkWorkflow();
    agentStore.handleChatFailed({ request_id: requestId, session_id: 'sess-hydrated', error: { message: 'Current failure' } });
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().messages.at(-1)?.content).toContain('Current failure');
  });

  it('dedupes double-clicks and waits for workflow_run acceptance before the agent decision', async () => {
    parkWorkflow();
    const run = deferred();
    const decision = deferred();
    bridgeMocks.runWorkflow.mockReturnValueOnce({ requestId: 'req-run', promise: run.promise });
    bridgeMocks.agentDecision.mockReturnValueOnce(decision.promise);

    const first = agentStore.handleApprovalDecision('appr-wf', 'approve');
    const second = agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.agentDecision).not.toHaveBeenCalled();
    expect(workflowCard()?.status).toBe('needs_approval');
    expect(workflowCard()?.approvalSubmitting).toBe(true);

    run.resolve();
    await Promise.resolve();
    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(1);
    expect(workflowCard()?.status).toBe('needs_approval');
    decision.resolve();
    await Promise.all([first, second]);
    expect(workflowCard()?.status).toBe('approved');
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(1);
  });

  it('denies without starting a workflow', async () => {
    const requestId = parkWorkflow();
    await agentStore.handleApprovalDecision('appr-wf', 'deny');
    expect(bridgeMocks.runWorkflow).not.toHaveBeenCalled();
    expect(bridgeMocks.agentDecision).toHaveBeenCalledWith(requestId, 'appr-wf', 'deny',
      expect.objectContaining({ session_id: 'sess-hydrated' }));
    expect(workflowCard()?.status).toBe('denied');
  });

  it('preserves the pending card on failed workflow dispatch and allows a retry', async () => {
    parkWorkflow();
    bridgeMocks.runWorkflow.mockReturnValueOnce({ requestId: 'req-run', promise: Promise.reject({ message: 'Host unavailable' }) });
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.agentDecision).not.toHaveBeenCalled();
    expect(workflowCard()?.status).toBe('needs_approval');
    expect(workflowCard()?.approvalSubmitting).toBe(false);
    expect(workflowCard()?.resultSummary).toContain('Host unavailable');

    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(2);
    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(1);
    expect(workflowCard()?.status).toBe('approved');
    expect(workflowCard()?.resultSummary).toBeUndefined();
  });

  it('retries a failed agent decision without launching the accepted workflow again', async () => {
    parkWorkflow();
    bridgeMocks.agentDecision.mockRejectedValueOnce(new Error('Decision unavailable'));
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(workflowCard()?.status).toBe('needs_approval');
    expect(workflowCard()?.resultSummary).toContain('Decision unavailable');

    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(2);
    expect(workflowCard()?.status).toBe('approved');
  });

  it('does not treat tool arguments as an authoritative workflow approval', async () => {
    agentStore.sendUserMessage('Audit');
    const requestId = latestAgentRequestId();
    agentStore.handleToolCallStarted({ request_id: requestId, session_id: 'sess-hydrated', turn_id: 'turn-wf',
      call_id: 'call-wf', tool_name: 'run_workflow', args_json: JSON.stringify(WORKFLOW) });
    const startedCard = agentStore.getSnapshot().messages.at(-1)?.toolCalls?.[0] as WorkflowToolCall;
    expect(startedCard.workflow).toEqual(WORKFLOW);
    agentStore.handleApprovalNeeded({ request_id: requestId, session_id: 'sess-hydrated', turn_id: 'turn-wf',
      approval_id: 'appr-wf', tool_name: 'run_workflow', path_or_command: 'audit' });

    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).not.toHaveBeenCalled();
    expect(bridgeMocks.agentDecision).not.toHaveBeenCalled();
    expect(workflowCard()?.status).toBe('needs_approval');
    expect(workflowCard()?.resultSummary).toContain('authoritative workflow approval is missing');
  });

  it('rejects approve_all for workflow scripts', async () => {
    parkWorkflow();
    await agentStore.handleApprovalDecision('appr-wf', 'approve_all');
    expect(bridgeMocks.runWorkflow).not.toHaveBeenCalled();
    expect(bridgeMocks.agentDecision).not.toHaveBeenCalled();
    expect(workflowCard()?.status).toBe('needs_approval');
    expect(workflowCard()?.resultSummary).toContain('explicit Approve');
  });

  it('requires the same pending request before launching a workflow', async () => {
    parkWorkflow();
    agentStore.sendUserMessage('A newer turn');
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    expect(bridgeMocks.runWorkflow).not.toHaveBeenCalled();
    expect(bridgeMocks.agentDecision).not.toHaveBeenCalled();
    expect(workflowCard()?.resultSummary).toContain('no longer matches the pending request');
  });

  it('does not reopen a workflow card on replayed approval events', async () => {
    const requestId = parkWorkflow();
    await agentStore.handleApprovalDecision('appr-wf', 'approve');
    agentStore.handleApprovalNeeded({ request_id: requestId, session_id: 'sess-hydrated', turn_id: 'turn-wf',
      approval_id: 'appr-wf', tool_name: 'run_workflow', path_or_command: 'audit', workflow: WORKFLOW });
    expect(workflowCard()?.status).toBe('approved');
    expect(agentStore.getSnapshot().messages.flatMap((message) => message.toolCalls ?? [])).toHaveLength(1);
  });

  it('does not send an agent decision after the workspace changes during run acceptance', async () => {
    parkWorkflow();
    const run = deferred();
    bridgeMocks.runWorkflow.mockReturnValueOnce({ requestId: 'req-run', promise: run.promise });
    const submission = agentStore.handleApprovalDecision('appr-wf', 'approve');
    agentStore.hydrate(makeAgentBootstrap('sess-hydrated', {
      workspace: { id: 'ws-other', core_instance_id: 'core-01', root: '/other', display_name: 'Other' },
    }));
    run.resolve();
    await submission;
    expect(bridgeMocks.agentDecision).not.toHaveBeenCalled();
  });
});

function makeAgentBootstrap(
  activeSessionId: string | null = 'sess-hydrated',
  overrides: Partial<BootstrapState> = {},
): BootstrapState {
  return {
    protocol_version: 1,
    core_instance_id: 'core-01',
    snapshot_sequence: 1,
    workspace: { id: 'ws-1', core_instance_id: 'core-01', root: '/repo', display_name: 'test' },
    buffers: [],
    active_buffer_id: null,
    file_tree: { root: null },
    models: [{ id: 'model-test', display_name: 'Test Model', available: true }],
    active_model_id: 'model-test',
    connection: 'connected',
    chat: {
      sessions: [],
      messages: [],
      active_session_id: activeSessionId,
      active_request_id: null,
      active_turn_id: null,
      pending_approval: null,
    },
    settings: {
      reduced_motion: false,
      theme: 'selora-dark',
      keybind_command_palette: 'Ctrl+Shift+P',
      keybind_toggle_chat: 'Ctrl+L',
    },
    layout: {} as BootstrapState['layout'],
    layout_constraints: {} as BootstrapState['layout_constraints'],
    git: { branch: 'main', changed_paths: [], staged_paths: [] },
    diagnostics: [],
    feature_flags: { agent: true, git: true, search: true, editor: true, later_slices: [] },
    ...overrides,
  };
}

describe('agentStore', () => {
  it('creates, switches, and deletes sessions', () => {
    const s1 = agentStore.createSession('Test Session 1');
    expect(agentStore.getSnapshot().activeSessionId).toBe(s1);

    const s2 = agentStore.createSession('Test Session 2');
    expect(agentStore.getSnapshot().activeSessionId).toBe(s2);

    agentStore.switchSession(s1);
    expect(agentStore.getSnapshot().activeSessionId).toBe(s1);

    agentStore.deleteSession(s2);
    expect(agentStore.getSnapshot().sessions.some((s) => s.id === s2)).toBe(false);
  });

  it('manages conversation turns, deltas, and in-flight states', () => {
    agentStore.sendUserMessage('Refactor the editor buffer');
    const state = agentStore.getSnapshot();
    expect(state.inFlight).toBe(true);
    expect(state.thinkingState).toBe('working');
    expect(state.messages.length).toBeGreaterThanOrEqual(2);

    const userMsg = state.messages[state.messages.length - 2];
    const assistantMsg = state.messages[state.messages.length - 1];

    expect(userMsg.role).toBe('user');
    expect(userMsg.content).toBe('Refactor the editor buffer');
    expect(assistantMsg.role).toBe('assistant');

    agentStore.appendAssistantDelta('Working on the rope structure...');
    const updatedState = agentStore.getSnapshot();
    const updatedAssistant = updatedState.messages[updatedState.messages.length - 1];
    expect(updatedAssistant.content).toContain('Working on the rope structure...');
    expect(updatedState.thinkingState).toBe('connecting');

    agentStore.completeActiveTurn();
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().thinkingState).toBe('breathing');
  });

  it('handles tool calls and side-effect approval gates', () => {
    agentStore.reset();
    agentStore.hydrate(makeAgentBootstrap('sess-tools'));
    agentStore.sendUserMessage('Run test command');
    const callId = agentStore.addToolCall({
      toolName: 'bash',
      argsSummary: 'cargo test -p selora-core',
      status: 'needs_approval',
      approvalId: 'appr-123',
      diff: {
        added: 2,
        removed: 1,
        hunks: ['- old line', '+ new line 1', '+ new line 2'],
      },
    });

    expect(callId).toBeTruthy();
    let state = agentStore.getSnapshot();
    let assistantMsg = state.messages[state.messages.length - 1];
    let tool = assistantMsg.toolCalls?.find((t) => t.id === callId);
    expect(tool?.status).toBe('needs_approval');

    agentStore.handleApprovalDecision('appr-123', 'approve');
    state = agentStore.getSnapshot();
    assistantMsg = state.messages[state.messages.length - 1];
    tool = assistantMsg.toolCalls?.find((t) => t.id === callId);
    expect(tool?.status).toBe('approved');

    agentStore.updateToolResult(callId, 'test result: ok. 4 passed', false, 0);
    state = agentStore.getSnapshot();
    assistantMsg = state.messages[state.messages.length - 1];
    tool = assistantMsg.toolCalls?.find((t) => t.id === callId);
    expect(tool?.status).toBe('completed');
    expect(tool?.resultSummary).toContain('4 passed');
    expect(tool?.exitCode).toBe(0);
  });

  it('updates todos and manages prompt recall history', () => {
    agentStore.addTodo('Verify border-beam animation', 'frontend-developer');
    const todos = agentStore.getSnapshot().todos;
    const lastTodo = todos[todos.length - 1];
    expect(lastTodo.title).toBe('Verify border-beam animation');
    expect(lastTodo.status).toBe('pending');

    agentStore.updateTodoStatus(lastTodo.id, 'completed');
    expect(
      agentStore.getSnapshot().todos.find((t) => t.id === lastTodo.id)?.status
    ).toBe('completed');

    agentStore.addPromptToHistory('Prompt A');
    agentStore.addPromptToHistory('Prompt B');
    expect(agentStore.getSnapshot().promptHistory[0]).toBe('Prompt B');
  });

  it('hydrates from bootstrap snapshot and handles reset', () => {
    const bootstrap = makeAgentBootstrap('sess-hydrated', {
      models: [{ id: 'glm-5', display_name: 'GLM-5.3 CodeFast', available: true }],
      active_model_id: 'glm-5',
    });

    agentStore.hydrate(bootstrap);
    expect(agentStore.getSnapshot().activeSessionId).toBe('sess-hydrated');
    expect(agentStore.getSnapshot().selectedModel).toBe('glm-5');

    agentStore.reset();
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().thinkingState).toBe('breathing');
  });

  it('handles live Z3 events: tool_call_started, approval_needed, and agent_turn_done', () => {
    agentStore.sendUserMessage('Execute full build and test');
    const requestId = latestAgentRequestId();

    // Tool call started
    agentStore.handleToolCallStarted({
      request_id: requestId,
      session_id: 'sess-hydrated',
      turn_id: 'turn-1',
      call_id: 'call-bash-1',
      tool_name: 'bash',
      args_json: JSON.stringify({ command: 'cargo test' }),
    });

    let state = agentStore.getSnapshot();
    let assistantMsg = state.messages[state.messages.length - 1];
    let tool = assistantMsg.toolCalls?.find((t) => t.id === 'call-bash-1');
    expect(tool?.status).toBe('running');
    expect(tool?.argsSummary).toBe('cargo test');
    expect(state.thinkingState).toBe('solving');

    // Tool result
    agentStore.handleToolResult({
      request_id: requestId,
      session_id: 'sess-hydrated',
      turn_id: 'turn-1',
      call_id: 'call-bash-1',
      result_summary: 'test result: ok. 12 passed',
      is_err: false,
    });

    state = agentStore.getSnapshot();
    assistantMsg = state.messages[state.messages.length - 1];
    tool = assistantMsg.toolCalls?.find((t) => t.id === 'call-bash-1');
    expect(tool?.status).toBe('completed');
    expect(tool?.resultSummary).toContain('12 passed');

    // Approval needed
    agentStore.handleApprovalNeeded({
      request_id: requestId,
      session_id: 'sess-hydrated',
      turn_id: 'turn-1',
      approval_id: 'appr-write-1',
      tool_name: 'write_file',
      path_or_command: 'src/main.rs',
      diff_or_input: '+ fn main() {}',
    });

    state = agentStore.getSnapshot();
    assistantMsg = state.messages[state.messages.length - 1];
    const apprTool = assistantMsg.toolCalls?.find((t) => t.approvalId === 'appr-write-1');
    expect(apprTool?.status).toBe('needs_approval');
    expect(apprTool?.argsSummary).toBe('src/main.rs');

    // Agent turn done
    agentStore.handleAgentTurnDone({
      request_id: requestId,
      session_id: 'sess-hydrated',
      turn_id: 'turn-1',
      total_tokens: 18500,
      cost_usd: 0.055,
      num_tools: 2,
      approved: 1,
      denied: 0,
      completed: true,
      end_message: 'Turn complete',
    });

    state = agentStore.getSnapshot();
    expect(state.inFlight).toBe(false);
    expect(state.thinkingState).toBe('breathing');
    expect(state.metrics.totalTokens).toBe(18500);
    expect(state.metrics.costUsd).toBe(0.055);
    expect(state.metrics.activeToolsCount).toBe(2);
  });

  it('carries the workflow confirmation payload onto the run_workflow card', async () => {
    agentStore.sendUserMessage('Run the repo audit workflow');
    const requestId = latestAgentRequestId();

    // `tool_call_started` already carries the four fields the card needs.
    agentStore.handleToolCallStarted({
      request_id: requestId,
      session_id: 'sess-hydrated',
      turn_id: 'turn-1',
      call_id: 'call-wf-1',
      tool_name: 'run_workflow',
      args_json: JSON.stringify({
        name: 'repo-audit',
        script: "phase('scan');\n",
        commands: ['cargo test', 'git status'],
        max_cost_usd: 2.5,
      }),
    });

    let state = agentStore.getSnapshot();
    let assistant = state.messages[state.messages.length - 1]!;
    let card = assistant.toolCalls?.find((call) => call.id === 'call-wf-1') as
      | WorkflowToolCall
      | undefined;
    expect(card?.status).toBe('running');
    // The header is the workflow name, not the whole argument JSON.
    expect(card?.argsSummary).toBe('repo-audit');
    expect(card?.workflow).toEqual({
      name: 'repo-audit',
      script: "phase('scan');\n",
      commands: ['cargo test', 'git status'],
      max_cost_usd: 2.5,
    });

    // `approval_needed` upgrades that same card (one grant, not two) with the
    // authoritative payload and the parked approval id. The script is not
    // carried as a diff.
    agentStore.handleApprovalNeeded({
      request_id: requestId,
      session_id: 'sess-hydrated',
      turn_id: 'turn-1',
      approval_id: 'appr-wf-1',
      tool_name: 'run_workflow',
      path_or_command: 'repo-audit',
      workflow: {
        name: 'repo-audit',
        script: "phase('scan');\nreport({ ok: true });\n",
        commands: ['cargo test'],
        max_cost_usd: 5,
      },
    });

    state = agentStore.getSnapshot();
    assistant = state.messages[state.messages.length - 1]!;
    const grants = (assistant.toolCalls ?? []).filter(
      (call) => call.toolName === 'run_workflow',
    ) as WorkflowToolCall[];
    expect(grants).toHaveLength(1);
    expect(grants[0]?.id).toBe('call-wf-1');
    expect(grants[0]?.status).toBe('needs_approval');
    expect(grants[0]?.approvalId).toBe('appr-wf-1');
    expect(grants[0]?.diff).toBeUndefined();
    expect(grants[0]?.workflow).toEqual({
      name: 'repo-audit',
      script: "phase('scan');\nreport({ ok: true });\n",
      commands: ['cargo test'],
      max_cost_usd: 5,
    });

    // The existing decision route still targets the request that raised it.
    await agentStore.handleApprovalDecision('appr-wf-1', 'approve');
    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.agentDecision).toHaveBeenCalledWith(
      requestId,
      'appr-wf-1',
      'approve',
      expect.objectContaining({ session_id: 'sess-hydrated' }),
    );
    // Approving the card starts the confirmed run with the displayed spec.
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.runWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'repo-audit',
        script: "phase('scan');\nreport({ ok: true });\n",
        commands: ['cargo test'],
        max_cost_usd: 5,
      }),
      expect.objectContaining({ session_id: null }),
    );
    state = agentStore.getSnapshot();
    assistant = state.messages[state.messages.length - 1]!;
    expect(
      assistant.toolCalls?.find((call) => call.id === 'call-wf-1')?.status,
    ).toBe('approved');
  });

  it('appends a run_workflow grant when no started card exists, leaving other cards alone', () => {
    agentStore.sendUserMessage('Approve two things');
    const requestId = latestAgentRequestId();

    agentStore.handleApprovalNeeded({
      request_id: requestId,
      session_id: 'sess-hydrated',
      turn_id: 'turn-1',
      approval_id: 'appr-wf-2',
      tool_name: 'run_workflow',
      path_or_command: 'nightly-audit',
      workflow: {
        name: 'nightly-audit',
        script: 'report({ ok: true });\n',
        commands: [],
        max_cost_usd: 1,
      },
    });
    agentStore.handleApprovalNeeded({
      request_id: requestId,
      session_id: 'sess-hydrated',
      turn_id: 'turn-1',
      approval_id: 'appr-write-2',
      tool_name: 'write_file',
      path_or_command: 'src/main.rs',
      diff_or_input: '+fn main() {}\n',
    });

    const assistant = agentStore.getSnapshot().messages[
      agentStore.getSnapshot().messages.length - 1
    ]!;
    const grant = assistant.toolCalls?.find(
      (call) => call.approvalId === 'appr-wf-2',
    ) as WorkflowToolCall | undefined;
    expect(grant?.workflow?.name).toBe('nightly-audit');
    expect(grant?.workflow?.commands).toEqual([]);

    // A non-workflow card keeps the generic diff card (and no payload).
    const write = assistant.toolCalls?.find(
      (call) => call.approvalId === 'appr-write-2',
    ) as WorkflowToolCall | undefined;
    expect(write?.workflow).toBeUndefined();
    expect(write?.diff?.hunks).toEqual(['+fn main() {}\n']);
  });

  it('configures reasoning depth, temperature, system instructions, and custom model ID', () => {
    agentStore.setReasoningEffort('quick');
    expect(agentStore.getSnapshot().reasoningEffort).toBe('quick');

    agentStore.updateModelSettings({
      temperature: 0.25,
      systemInstructions: 'Always verify before write',
      customModelId: 'deepseek-reasoner',
    });

    const snapshot = agentStore.getSnapshot();
    expect(snapshot.modelSettings.temperature).toBe(0.25);
    expect(snapshot.modelSettings.systemInstructions).toBe('Always verify before write');
    expect(snapshot.modelSettings.customModelId).toBe('deepseek-reasoner');

    agentStore.setAvailableModels([
      { id: 'claude-3-7-sonnet', display_name: 'Claude 3.7 Sonnet', available: true },
      { id: 'gpt-4o', display_name: 'GPT-4o', available: true },
    ]);

    expect(agentStore.getSnapshot().availableModels).toHaveLength(2);
    agentStore.setSelectedModel('claude-3-7-sonnet');
    expect(agentStore.getSnapshot().selectedModel).toBe('claude-3-7-sonnet');
  });

  it('copies prompt and rewinds conversation to checkpoint restoring draft', async () => {
    resetAndHydrate();
    agentStore.sendUserMessage('First task prompt');
    agentStore.completeActiveTurn();
    agentStore.sendUserMessage('Second task prompt');
    agentStore.completeActiveTurn();

    const snapshot1 = agentStore.getSnapshot();
    expect(snapshot1.messages.length).toBe(4); // 2 user + 2 assistant
    const firstUserMsgId = snapshot1.messages[0].id;

    // Test copy
    const copied = await agentStore.copyPrompt('First task prompt');
    expect(typeof copied).toBe('boolean');

    // Test rewind
    const result = agentStore.rewindToPrompt(firstUserMsgId, false);
    expect(result).not.toBeNull();
    expect(result?.restoredText).toBe('First task prompt');

    const snapshot2 = agentStore.getSnapshot();
    // Messages rewound to before first prompt
    expect(snapshot2.messages.length).toBe(0);
    expect(snapshot2.inFlight).toBe(false);
  });

  it('edits prompt in place and resubmits edited prompt', () => {
    resetAndHydrate();
    agentStore.sendUserMessage('Initial draft message');
    agentStore.completeActiveTurn();

    const initialMsgId = agentStore.getSnapshot().messages[0].id;

    // Edit in place
    agentStore.editPromptInPlace(initialMsgId, 'Corrected prompt text');
    expect(agentStore.getSnapshot().messages[0].content).toBe('Corrected prompt text');
    expect(agentStore.getSnapshot().messages[0].isEdited).toBe(true);

    // Resubmit edited prompt
    agentStore.resubmitEditedPrompt(initialMsgId, 'Resubmitted prompt with fixes');
    const updated = agentStore.getSnapshot();
    expect(updated.messages[0].content).toBe('Resubmitted prompt with fixes');
    expect(updated.inFlight).toBe(true);
  });

  it('forks new session from prompt checkpoint', () => {
    resetAndHydrate();
    agentStore.sendUserMessage('Step 1: Explore files');
    agentStore.completeActiveTurn();
    agentStore.sendUserMessage('Step 2: Modify code');
    agentStore.completeActiveTurn();

    const firstMsgId = agentStore.getSnapshot().messages[0].id;
    const forkedId = agentStore.forkSessionFromPrompt(firstMsgId);

    expect(forkedId).toBeTruthy();
    expect(agentStore.getSnapshot().activeSessionId).toBe(forkedId);
    expect(agentStore.getSnapshot().messages.length).toBe(1);
    expect(agentStore.getSnapshot().messages[0].content).toBe('Step 1: Explore files');
  });

  it('manages prompt queue: enqueue, reorder, promote, and auto-execution on turn done', () => {
    resetAndHydrate();
    // Mark agent in-flight
    agentStore.sendUserMessage('Active working turn');
    expect(agentStore.getSnapshot().inFlight).toBe(true);

    // Queue prompts while in-flight
    const q1 = agentStore.queuePrompt('Queued instruction 1');
    const q2 = agentStore.queuePrompt('Queued instruction 2');
    const q3 = agentStore.queuePrompt('Queued instruction 3');

    expect(agentStore.getSnapshot().promptQueue).toHaveLength(3);
    expect(agentStore.getSnapshot().promptQueue[0].text).toBe('Queued instruction 1');

    // Promote q3 to front
    agentStore.promoteQueuedPrompt(q3);
    expect(agentStore.getSnapshot().promptQueue[0].id).toBe(q3);

    // Remove q2
    agentStore.removeQueuedPrompt(q2);
    expect(agentStore.getSnapshot().promptQueue).toHaveLength(2);

    // Update q1 text
    agentStore.updateQueuedPrompt(q1, 'Updated instruction 1');
    const foundQ1 = agentStore.getSnapshot().promptQueue.find((q) => q.id === q1);
    expect(foundQ1?.text).toBe('Updated instruction 1');

    // Clear queue test
    agentStore.clearPromptQueue();
    expect(agentStore.getSnapshot().promptQueue).toHaveLength(0);
  });

  it('handles mid-flight steering course correction with isSteer priority flag', async () => {
    resetAndHydrate();
    agentStore.sendUserMessage('Initial plan');
    expect(agentStore.getSnapshot().inFlight).toBe(true);

    // Steer active turn (stopChat is awaited before the new turn starts)
    await agentStore.steerActiveTurn('Stop editing database, focus on frontend types only');

    const snapshot = agentStore.getSnapshot();
    expect(snapshot.inFlight).toBe(true);
    const lastUserMsg = snapshot.messages.find((m) => m.content.includes('focus on frontend'));
    expect(lastUserMsg).toBeTruthy();
    expect(lastUserMsg?.isSteer).toBe(true);
  });

  it('sends the live session id on SendAgent, StopChat, and AgentDecision envelopes', () => {
    agentStore.reset();
    agentStore.hydrate(makeAgentBootstrap('sess-live'));

    agentStore.sendUserMessage('Plan the migration');
    expect(bridgeMocks.sendAgent).toHaveBeenCalledTimes(1);
    const sendAgentContext = bridgeMocks.sendAgent.mock.calls[0]?.[3] as EnvelopeContext;
    expect(sendAgentContext.session_id).toBe('sess-live');
    // The workspace identity still rides along on the session envelope.
    expect(sendAgentContext.core_instance_id).toBe('core-01');
    expect(sendAgentContext.workspace_id).toBe('ws-1');

    agentStore.stopActiveTurn();
    expect(bridgeMocks.stopChat).toHaveBeenCalledTimes(1);
    const stopContext = bridgeMocks.stopChat.mock.calls[0]?.[1] as EnvelopeContext;
    expect(stopContext.session_id).toBe('sess-live');

    agentStore.sendUserMessage('Second turn');
    agentStore.handleApprovalDecision('appr-1', 'approve');
    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(1);
    const decisionContext = bridgeMocks.agentDecision.mock.calls[0]?.[3] as EnvelopeContext;
    expect(decisionContext.session_id).toBe('sess-live');
  });

  // --- FIX 2: bootstrap chat hydration ------------------------------------

  it('hydrates the Rust session rail and transcript instead of a synthetic session', () => {
    agentStore.reset();
    agentStore.hydrate(
      makeAgentBootstrap(null, {
        chat: {
          sessions: [
            { session_id: 'sess-a', title: 'First session', message_ids: ['m1', 'm2'], active: true },
            { session_id: 'sess-b', title: 'Second session', message_ids: [], active: false },
          ],
          messages: [
            { message_id: 'm1', session_id: 'sess-a', role: 'user', text: 'hello' },
            { message_id: 'm2', session_id: 'sess-a', role: 'assistant', text: 'hi there' },
            { message_id: 'm3', session_id: 'sess-a', role: 'tool', text: 'tool output' },
          ],
          active_session_id: 'sess-a',
          active_request_id: null,
          active_turn_id: null,
          pending_approval: null,
        },
      }),
    );

    const snapshot = agentStore.getSnapshot();
    expect(snapshot.sessions.map((session) => session.id)).toEqual(['sess-a', 'sess-b']);
    expect(snapshot.activeSessionId).toBe('sess-a');
    expect(
      snapshot.messages.map((message) => [message.id, message.role, message.content, message.status]),
    ).toEqual([
      ['m1', 'user', 'hello', 'complete'],
      ['m2', 'assistant', 'hi there', 'complete'],
    ]);
  });

  it('hydrates a pending approval as a parked needs_approval tool card', () => {
    agentStore.reset();
    const pending: Approval = {
      approval_id: 'appr-parked',
      request_id: 'req-parked',
      turn_id: 'turn-parked',
      session_id: 'sess-approval',
      summary: 'write src/lib.rs',
      can_approve_all_this_turn: true,
    };
    agentStore.hydrate(
      makeAgentBootstrap('sess-approval', {
        chat: {
          sessions: [{ session_id: 'sess-approval', title: 'Parked', message_ids: ['m1'], active: true }],
          messages: [
            { message_id: 'm1', session_id: 'sess-approval', role: 'assistant', text: 'May I write?' },
          ],
          active_session_id: 'sess-approval',
          active_request_id: 'req-parked',
          active_turn_id: 'turn-parked',
          pending_approval: pending,
        },
      }),
    );

    const snapshot = agentStore.getSnapshot();
    const assistant = snapshot.messages[snapshot.messages.length - 1]!;
    const card = assistant.toolCalls?.find((tool) => tool.approvalId === 'appr-parked');
    expect(card?.status).toBe('needs_approval');
    expect(card?.argsSummary).toBe('write src/lib.rs');
  });

  it('does not invent sessions when Rust sends an empty session list', () => {
    agentStore.reset();
    agentStore.hydrate(makeAgentBootstrap('sess-hydrated'));
    expect(agentStore.getSnapshot().sessions).toEqual([]);
  });

  // --- FIX 3: stale turn routing ------------------------------------------

  it('drops late events from a superseded turn and keeps matching events', () => {
    agentStore.reset();
    agentStore.hydrate(makeAgentBootstrap('sess-route'));
    agentStore.sendUserMessage('first turn');
    const firstRequestId = latestAgentRequestId();
    agentStore.completeActiveTurn();
    agentStore.sendUserMessage('second turn');
    const secondRequestId = latestAgentRequestId();
    expect(secondRequestId).not.toBe(firstRequestId);

    // Late events from the stopped/superseded request must not mutate the UI.
    agentStore.handleChatDelta({ request_id: firstRequestId, text: 'LATE-DELTA' });
    agentStore.handleToolResult({
      request_id: firstRequestId,
      session_id: 'sess-route',
      turn_id: 'turn-1',
      call_id: 'call-stale',
      result_summary: 'stale result',
      is_err: false,
    });
    agentStore.handleAgentTurnDone({
      request_id: firstRequestId,
      session_id: 'sess-route',
      turn_id: 'turn-1',
      total_tokens: 111,
      cost_usd: 1,
      num_tools: 3,
      approved: 1,
      denied: 0,
      completed: true,
      end_message: 'LATE-DONE',
    });
    // A different session is equally stale.
    agentStore.handleChatDelta({ request_id: secondRequestId, session_id: 'sess-other', text: 'OTHER-SESSION' });

    let snapshot = agentStore.getSnapshot();
    let assistant = snapshot.messages[snapshot.messages.length - 1]!;
    expect(assistant.content).not.toContain('LATE-DELTA');
    expect(assistant.content).not.toContain('LATE-DONE');
    expect(assistant.content).not.toContain('OTHER-SESSION');
    expect(assistant.status).toBe('streaming');
    expect(snapshot.inFlight).toBe(true);
    expect(snapshot.metrics.totalTokens).not.toBe(111);
    expect(snapshot.subagents[0]?.activeStep).not.toContain('stale result');

    // Matching events still apply.
    agentStore.handleChatDelta({ request_id: secondRequestId, text: 'CURRENT-DELTA' });
    agentStore.handleToolCallStarted({
      request_id: secondRequestId,
      session_id: 'sess-route',
      turn_id: 'turn-2',
      call_id: 'call-current',
      tool_name: 'bash',
      args_json: JSON.stringify({ command: 'ls' }),
    });
    agentStore.handleToolResult({
      request_id: secondRequestId,
      session_id: 'sess-route',
      turn_id: 'turn-2',
      call_id: 'call-current',
      result_summary: 'current result',
      is_err: false,
    });

    snapshot = agentStore.getSnapshot();
    assistant = snapshot.messages[snapshot.messages.length - 1]!;
    expect(assistant.content).toContain('CURRENT-DELTA');
    const currentTool = assistant.toolCalls?.find((tool) => tool.id === 'call-current');
    expect(currentTool?.status).toBe('completed');
  });

  // --- FIX 4: approval request routing + steer ordering --------------------

  it('routes an approval decision to the request that raised it, not the newest request', () => {
    agentStore.reset();
    agentStore.hydrate(
      makeAgentBootstrap('sess-approval', {
        chat: {
          sessions: [{ session_id: 'sess-approval', title: 'Parked', message_ids: ['m1'], active: true }],
          messages: [
            { message_id: 'm1', session_id: 'sess-approval', role: 'assistant', text: 'May I write?' },
          ],
          active_session_id: 'sess-approval',
          active_request_id: 'req-newer',
          active_turn_id: 'turn-newer',
          pending_approval: {
            approval_id: 'appr-parked',
            request_id: 'req-parked',
            turn_id: 'turn-parked',
            session_id: 'sess-approval',
            summary: 'write src/lib.rs',
            can_approve_all_this_turn: true,
          },
        },
      }),
    );

    agentStore.handleApprovalDecision('appr-parked', 'approve');

    expect(bridgeMocks.agentDecision).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.agentDecision.mock.calls[0]?.[0]).toBe('req-parked');
    expect(bridgeMocks.agentDecision.mock.calls[0]?.[1]).toBe('appr-parked');
    const decisionContext = bridgeMocks.agentDecision.mock.calls[0]?.[3] as EnvelopeContext;
    expect(decisionContext.session_id).toBe('sess-approval');
    const snapshot = agentStore.getSnapshot();
    const assistant = snapshot.messages[snapshot.messages.length - 1]!;
    const card = assistant.toolCalls?.find((tool) => tool.approvalId === 'appr-parked');
    expect(card?.status).toBe('approved');
  });

  it('does not flip an approval card when no request id can be resolved', () => {
    agentStore.reset();
    agentStore.hydrate(makeAgentBootstrap('sess-orphan'));
    agentStore.sendUserMessage('offline turn');
    // The turn is over: no live request is tracked any more, so a parked card
    // has no request to route its decision to.
    agentStore.completeActiveTurn();
    const callId = agentStore.addToolCall({
      toolName: 'bash',
      argsSummary: 'rm -rf build',
      status: 'needs_approval',
      approvalId: 'appr-orphan',
    });

    agentStore.handleApprovalDecision('appr-orphan', 'approve');

    expect(bridgeMocks.agentDecision).not.toHaveBeenCalled();
    const snapshot = agentStore.getSnapshot();
    const assistant = snapshot.messages[snapshot.messages.length - 1]!;
    const card = assistant.toolCalls?.find((tool) => tool.id === callId);
    expect(card?.status).toBe('needs_approval');
  });

  it('awaits stopChat before sending the steered turn', async () => {
    agentStore.reset();
    agentStore.hydrate(makeAgentBootstrap('sess-steer'));
    agentStore.sendUserMessage('initial direction');
    const stoppedRequestId = latestAgentRequestId();
    const sendsBefore = bridgeMocks.sendAgent.mock.calls.length;

    await agentStore.steerActiveTurn('new direction');

    expect(bridgeMocks.stopChat).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.stopChat.mock.calls[0]?.[0]).toBe(stoppedRequestId);
    expect(bridgeMocks.sendAgent).toHaveBeenCalledTimes(sendsBefore + 1);
    const stopOrder = bridgeMocks.stopChat.mock.invocationCallOrder[0];
    const steerOrder = bridgeMocks.sendAgent.mock.invocationCallOrder[sendsBefore];
    expect(stopOrder).toBeLessThan(steerOrder!);
    const snapshot = agentStore.getSnapshot();
    const lastMessage = snapshot.messages[snapshot.messages.length - 1]!;
    expect(lastMessage.isSteer).toBeUndefined();
    expect(snapshot.messages[snapshot.messages.length - 2]?.isSteer).toBe(true);
    expect(snapshot.inFlight).toBe(true);
  });

  it('still sends the steered turn when stopChat fails', async () => {
    agentStore.reset();
    agentStore.hydrate(makeAgentBootstrap('sess-steer'));
    agentStore.sendUserMessage('initial direction');
    bridgeMocks.stopChat.mockRejectedValueOnce(new Error('request_not_found'));

    await expect(agentStore.steerActiveTurn('redirect now')).resolves.toBeUndefined();

    expect(bridgeMocks.sendAgent).toHaveBeenCalledTimes(2);
    const snapshot = agentStore.getSnapshot();
    const lastMessage = snapshot.messages[snapshot.messages.length - 2]!;
    expect(lastMessage.content).toContain('redirect now');
    expect(lastMessage.isSteer).toBe(true);
    expect(snapshot.inFlight).toBe(true);
  });

  // --- FIX: no fabricated model, workspace guard, honest mode mapping ------

  it('does not fabricate a default model when the backend catalog is empty', () => {
    agentStore.reset();
    agentStore.hydrate(makeAgentBootstrap('sess-empty', { models: [], active_model_id: null }));

    const snapshot = agentStore.getSnapshot();
    expect(snapshot.selectedModel).toBe('');
    expect(snapshot.availableModels).toEqual([]);
    expect(JSON.stringify(snapshot)).not.toContain('GLM-5.3');
    expect(JSON.stringify(snapshot)).not.toContain('Claude 3.7 Sonnet');
  });

  it('blocks send with no model instead of sending an unknown model id', () => {
    agentStore.reset();
    agentStore.hydrate(makeAgentBootstrap('sess-nomodel', { models: [], active_model_id: null }));
    bridgeMocks.sendAgent.mockClear();

    const sent = agentStore.sendUserMessage('write the feature');

    expect(sent).toBe(false);
    expect(bridgeMocks.sendAgent).not.toHaveBeenCalled();
    expect(agentStore.getSnapshot().messages).toEqual([]);
    const gate = getComposerGate(agentStore.getSnapshot());
    expect(gate.ready).toBe(false);
    expect(gate.reason).toBe('no_model');
    expect(gate.message).toContain('Connect and refresh models');
  });

  it('blocks send when the workspace is absent instead of inventing one', () => {
    agentStore.reset();
    agentStore.hydrate(
      makeAgentBootstrap('sess-nows', {
        workspace: null as unknown as BootstrapState['workspace'],
      }),
    );
    bridgeMocks.sendAgent.mockClear();

    expect(agentStore.getSnapshot().workspace).toBeNull();
    const sent = agentStore.sendUserMessage('summarize the workspace');

    expect(sent).toBe(false);
    expect(bridgeMocks.sendAgent).not.toHaveBeenCalled();
    expect(agentStore.getSnapshot().messages).toEqual([]);
    const gate = getComposerGate(agentStore.getSnapshot());
    expect(gate.reason).toBe('no_workspace');
    expect(gate.message).toContain('open folder');
  });

  it('clears the model selection when models_changed reports an empty catalog', () => {
    agentStore.setAvailableModels([]);
    expect(agentStore.getSnapshot().selectedModel).toBe('');
    expect(agentStore.getSnapshot().availableModels).toEqual([]);
    expect(getComposerGate(agentStore.getSnapshot()).reason).toBe('no_model');
  });

  it('picks the first available backend model when none is selected', () => {
    agentStore.reset();
    agentStore.hydrate(
      makeAgentBootstrap('sess-catalog', {
        models: [
          { id: 'offline-model', display_name: 'Offline Model', available: false },
          { id: 'ready-model', display_name: 'Ready Model', available: true },
        ],
        active_model_id: null,
      }),
    );
    expect(agentStore.getSnapshot().selectedModel).toBe('ready-model');

    agentStore.setAvailableModels([
      { id: 'offline-model', display_name: 'Offline Model', available: false },
      { id: 'ready-model', display_name: 'Ready Model', available: true },
      { id: 'new-model', display_name: 'New Model', available: true },
    ]);
    // An existing selection is preserved across models_changed.
    expect(agentStore.getSnapshot().selectedModel).toBe('ready-model');
  });

  it('maps full/ask_before_change/plan_only to auto/execute/explore on send', () => {
    bridgeMocks.sendAgent.mockClear();

    const modes = [
      ['full', 'auto'],
      ['ask_before_change', 'execute'],
      ['plan_only', 'explore'],
    ] as const;
    for (const [permissionMode] of modes) {
      agentStore.setPermissionMode(permissionMode);
      expect(agentStore.sendUserMessage(`run in ${permissionMode}`)).toBe(true);
      agentStore.completeActiveTurn();
    }

    const sentModes = bridgeMocks.sendAgent.mock.calls.map((call) => call[1]);
    expect(sentModes).toEqual(['auto', 'execute', 'explore']);
    expect(bridgeMocks.sendAgent.mock.calls[0]?.[2]).toBe('model-test');
  });

  it('exposes the AgentMode mapping without guessing permissions', () => {
    expect(toAgentMode('full')).toBe('auto');
    expect(toAgentMode('ask_before_change')).toBe('execute');
    expect(toAgentMode('plan_only')).toBe('explore');
  });
});
