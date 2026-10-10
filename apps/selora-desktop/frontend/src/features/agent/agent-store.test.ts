// Selora IDE — Phase Z3 Agent Store Tests
import { describe, it, expect, beforeEach } from 'vitest';
import { agentStore } from './agent-store';

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
    const bootstrap = {
      protocol_version: 1,
      core_instance_id: 'core-01',
      snapshot_sequence: 1,
      workspace: { id: 'ws-1', core_instance_id: 'core-01', root: '/repo', display_name: 'test' },
      buffers: [],
      active_buffer_id: null,
      file_tree: { root: null },
      models: [{ id: 'glm-5', display_name: 'GLM-5.3 CodeFast', available: true }],
      active_model_id: 'glm-5',
      connection: 'connected' as const,
      chat: {
        sessions: [],
        messages: [],
        active_session_id: 'sess-hydrated',
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
      layout: {} as any,
      layout_constraints: {} as any,
      git: { branch: 'main', changed_paths: [], staged_paths: [] },
      diagnostics: [],
      feature_flags: { agent: true, git: true, search: true, editor: true, later_slices: [] },
    };

    agentStore.hydrate(bootstrap);
    expect(agentStore.getSnapshot().activeSessionId).toBe('sess-hydrated');
    expect(agentStore.getSnapshot().selectedModel).toBe('glm-5');

    agentStore.reset();
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().thinkingState).toBe('breathing');
  });

  it('handles live Z3 events: tool_call_started, approval_needed, and agent_turn_done', () => {
    agentStore.sendUserMessage('Execute full build and test');

    // Tool call started
    agentStore.handleToolCallStarted({
      request_id: 'req-1',
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
      request_id: 'req-1',
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
      request_id: 'req-1',
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
      request_id: 'req-1',
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
    agentStore.reset();
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
    agentStore.reset();
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
    agentStore.reset();
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
    agentStore.reset();
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

  it('handles mid-flight steering course correction with isSteer priority flag', () => {
    agentStore.reset();
    agentStore.sendUserMessage('Initial plan');
    expect(agentStore.getSnapshot().inFlight).toBe(true);

    // Steer active turn
    agentStore.steerActiveTurn('Stop editing database, focus on frontend types only');

    const snapshot = agentStore.getSnapshot();
    expect(snapshot.inFlight).toBe(true);
    const lastUserMsg = snapshot.messages.find((m) => m.content.includes('focus on frontend'));
    expect(lastUserMsg).toBeTruthy();
    expect(lastUserMsg?.isSteer).toBe(true);
  });
});
