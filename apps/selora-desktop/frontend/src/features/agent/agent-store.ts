// Selora IDE — Phase Z3 Agent Store
//
// Manages agent sessions, messages, tool executions, approval gates,
// live todos, subagents, and hero composer interactions.
// Implemented via React 18 external store subscription pattern.
import type {
  AgentSession,
  ChatMessage,
  AgentToolCall,
  AgentTodo,
  SubagentInfo,
  ArtifactItem,
  ThinkingState,
  PermissionMode,
  ReasoningEffort,
  AgentMetrics,
  TodoStatus,
  QueuedPrompt,
} from './types';
import {
  agentDecision,
  fetchModels,
  sendAgent,
  sendChat,
  stopChat,
  type AgentStateChangedPayload,
  type AgentTurnDonePayload,
  type ApprovalNeededPayload,
  type BootstrapState,
  type ChatCompletedPayload,
  type EnvelopeContext,
  type Model,
  type ToolCallStartedPayload,
  type ToolResultPayload,
} from '../../bridge/bridge';

export type ModelSettings = {
  temperature: number;
  systemInstructions: string;
  customModelId: string;
};

export type AgentState = {
  sessions: AgentSession[];
  activeSessionId: string;
  messages: ChatMessage[];
  todos: AgentTodo[];
  subagents: SubagentInfo[];
  artifacts: ArtifactItem[];
  inFlight: boolean;
  thinkingState: ThinkingState;
  permissionMode: PermissionMode;
  reasoningEffort: ReasoningEffort;
  selectedModel: string;
  availableModels: Model[];
  modelSettings: ModelSettings;
  streamingActivityLevel: number;
  promptHistory: string[];
  metrics: AgentMetrics;
  promptQueue: QueuedPrompt[];
};

const DEFAULT_SESSION_ID = 'sess_default';
const STORED_SETTINGS_KEY = 'selora:model_settings';

function loadStoredSettings(): ModelSettings {
  try {
    const raw = localStorage.getItem(STORED_SETTINGS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      return {
        temperature: typeof parsed.temperature === 'number' ? parsed.temperature : 0.7,
        systemInstructions: typeof parsed.systemInstructions === 'string' ? parsed.systemInstructions : '',
        customModelId: typeof parsed.customModelId === 'string' ? parsed.customModelId : '',
      };
    }
  } catch {
    // ignore
  }
  return {
    temperature: 0.7,
    systemInstructions: '',
    customModelId: '',
  };
}

function saveStoredSettings(settings: ModelSettings) {
  try {
    localStorage.setItem(STORED_SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    // ignore
  }
}

const INITIAL_SESSIONS: AgentSession[] = [
  {
    id: DEFAULT_SESSION_ID,
    title: 'Agent Session 1',
    createdAt: Date.now(),
    updatedAt: Date.now(),
    tokenCount: 0,
    costUsd: 0,
    branchName: 'main',
  },
];

const INITIAL_TODOS: AgentTodo[] = [];

const INITIAL_SUBAGENTS: SubagentInfo[] = [
  {
    id: 'sub-1',
    name: 'frontend-developer',
    model: 'Auto',
    status: 'idle',
    activeStep: 'Ready for instructions',
  },
  {
    id: 'sub-2',
    name: 'rust-developer',
    model: 'Auto',
    status: 'idle',
    activeStep: 'Ready for instructions',
  },
];

const INITIAL_ARTIFACTS: ArtifactItem[] = [];

let state: AgentState = {
  sessions: INITIAL_SESSIONS,
  activeSessionId: DEFAULT_SESSION_ID,
  messages: [],
  todos: INITIAL_TODOS,
  subagents: INITIAL_SUBAGENTS,
  artifacts: INITIAL_ARTIFACTS,
  inFlight: false,
  thinkingState: 'breathing',
  permissionMode: 'full',
  reasoningEffort: 'max',
  selectedModel: 'GLM-5.3 CodeFast',
  availableModels: [],
  modelSettings: loadStoredSettings(),
  streamingActivityLevel: 0,
  promptHistory: [
    'Explain this file',
    'Find a bug in the selection',
    'Summarize the workspace',
  ],
  metrics: {
    totalTokens: 0,
    costUsd: 0,
    activeToolsCount: 0,
    approvedCount: 0,
    deniedCount: 0,
    latencyMs: 0,
  },
  promptQueue: [],
};

export function extractMarkdownChecklist(text: string): AgentTodo[] | null {
  const lines = text.split('\n');
  const todos: AgentTodo[] = [];
  let idx = 0;
  for (const line of lines) {
    const trimmed = line.trim();
    const match =
      trimmed.match(/^[-*]\s*\[([ xX/\\-])\]\s*(.+)$/) ||
      trimmed.match(/^\d+\.\s*\[([ xX/\\-])\]\s*(.+)$/);
    if (match) {
      const mark = match[1].toLowerCase();
      const title = match[2].trim();
      if (!title) continue;
      const status: TodoStatus =
        mark === 'x' ? 'completed' : mark === '/' ? 'in_progress' : 'pending';
      todos.push({
        id: `todo_md_${idx++}`,
        title,
        status,
      });
    }
  }
  return todos.length > 0 ? todos : null;
}

export function parseModelTodos(input: unknown): AgentTodo[] | null {
  if (!input) return null;

  if (Array.isArray(input)) {
    const todos: AgentTodo[] = [];
    input.forEach((item, idx) => {
      if (typeof item === 'string') {
        const trimmed = item.trim();
        if (trimmed) {
          todos.push({
            id: `todo_${Date.now()}_${idx}`,
            title: trimmed,
            status: 'pending',
          });
        }
      } else if (item && typeof item === 'object') {
        const itemObj = item as Record<string, unknown>;
        const title = String(itemObj.title || itemObj.task || itemObj.text || itemObj.name || '').trim();
        if (!title) return;
        const rawStatus = String(itemObj.status || '').toLowerCase();
        const status: TodoStatus =
          rawStatus === 'completed' || rawStatus === 'done' || itemObj.completed === true
            ? 'completed'
            : rawStatus === 'in_progress' || rawStatus === 'active' || rawStatus === 'working'
            ? 'in_progress'
            : 'pending';
        todos.push({
          id: String(itemObj.id || `todo_${Date.now()}_${idx}`),
          title,
          status,
          assignedSubagent: itemObj.assignedSubagent ? String(itemObj.assignedSubagent) : undefined,
        });
      }
    });
    return todos.length > 0 ? todos : null;
  }

  if (typeof input === 'string') {
    try {
      const parsed = JSON.parse(input);
      if (Array.isArray(parsed)) return parseModelTodos(parsed);
      if (parsed && typeof parsed === 'object') {
        const parsedObj = parsed as Record<string, unknown>;
        const list = parsedObj.todos || parsedObj.tasks || parsedObj.steps || parsedObj.items;
        if (Array.isArray(list)) return parseModelTodos(list);
      }
    } catch {
      // Not JSON, try checklist
    }
    return extractMarkdownChecklist(input);
  }

  return null;
}

type Listener = () => void;
const listeners = new Set<Listener>();

let context: EnvelopeContext | null = null;
let activeRequestId: string | null = null;

function notify() {
  for (const listener of listeners) {
    listener();
  }
}

export const agentStore = {
  subscribe(listener: Listener): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },

  getSnapshot(): AgentState {
    return state;
  },

  hydrate(bootstrap: BootstrapState) {
    context = {
      core_instance_id: bootstrap.core_instance_id,
      workspace_id: bootstrap.workspace.id,
      session_id: bootstrap.chat.active_session_id ?? state.activeSessionId,
    };
    const models = bootstrap.models || [];
    let modelToSelect = state.selectedModel;
    if (bootstrap.active_model_id) {
      modelToSelect = bootstrap.active_model_id;
    } else if (models.length > 0) {
      const available = models.find((m) => m.available) ?? models[0];
      if (available) {
        modelToSelect = available.id;
      }
    }
    state = {
      ...state,
      availableModels: models,
      selectedModel: modelToSelect,
      activeSessionId: bootstrap.chat.active_session_id ?? state.activeSessionId,
    };
    notify();
  },

  setAvailableModels(models: Model[]) {
    let modelToSelect = state.selectedModel;
    if (models.length > 0) {
      const hasCurrent = models.some(
        (m) => m.id === state.selectedModel || m.display_name === state.selectedModel
      );
      if (!hasCurrent) {
        const firstAvail = models.find((m) => m.available) ?? models[0];
        if (firstAvail) {
          modelToSelect = firstAvail.id;
        }
      }
    }
    state = {
      ...state,
      availableModels: models,
      selectedModel: modelToSelect,
    };
    notify();
  },

  reset() {
    context = null;
    activeRequestId = null;
    state = {
      ...state,
      messages: [],
      todos: [],
      inFlight: false,
      thinkingState: 'breathing',
      streamingActivityLevel: 0,
      promptQueue: [],
    };
    notify();
  },

  createSession(title = 'New Agent Session'): string {
    const newId = `sess_${Date.now().toString(36)}`;
    const newSession: AgentSession = {
      id: newId,
      title,
      createdAt: Date.now(),
      updatedAt: Date.now(),
      tokenCount: 0,
      costUsd: 0,
    };
    state = {
      ...state,
      sessions: [newSession, ...state.sessions],
      activeSessionId: newId,
      messages: [],
      todos: [],
      inFlight: false,
      thinkingState: 'breathing',
      promptQueue: [],
    };
    notify();
    return newId;
  },

  switchSession(sessionId: string) {
    if (state.activeSessionId === sessionId) return;
    state = {
      ...state,
      activeSessionId: sessionId,
      messages: sessionId === DEFAULT_SESSION_ID ? state.messages : [],
    };
    notify();
  },

  deleteSession(sessionId: string) {
    const remaining = state.sessions.filter((s) => s.id !== sessionId);
    const nextActive = remaining[0]?.id || '';
    state = {
      ...state,
      sessions: remaining,
      activeSessionId: nextActive,
    };
    notify();
  },

  setPermissionMode(mode: PermissionMode) {
    state = { ...state, permissionMode: mode };
    notify();
  },

  setReasoningEffort(effort: ReasoningEffort) {
    state = { ...state, reasoningEffort: effort };
    notify();
  },

  setSelectedModel(model: string) {
    state = { ...state, selectedModel: model };
    notify();
  },

  updateModelSettings(settings: Partial<ModelSettings>) {
    const updated = {
      ...state.modelSettings,
      ...settings,
    };
    state = {
      ...state,
      modelSettings: updated,
    };
    saveStoredSettings(updated);
    notify();
  },

  async refreshModels(): Promise<Model[]> {
    try {
      const models = await fetchModels();
      this.setAvailableModels(models);
      return models;
    } catch (err) {
      console.error('Failed to fetch models from provider:', err);
      throw err;
    }
  },

  setThinkingState(thinkingState: ThinkingState) {
    state = { ...state, thinkingState };
    notify();
  },

  addPromptToHistory(prompt: string) {
    const trimmed = prompt.trim();
    if (!trimmed) return;
    const filtered = state.promptHistory.filter((p) => p !== trimmed);
    const updated = [trimmed, ...filtered].slice(0, 10);
    state = { ...state, promptHistory: updated };
    notify();
  },

  sendUserMessage(text: string, isSteer = false) {
    const trimmed = text.trim();
    if (!trimmed) return;

    this.addPromptToHistory(trimmed);

    const userMsg: ChatMessage = {
      id: `msg_${Date.now()}_u`,
      role: 'user',
      content: trimmed,
      isSteer: isSteer || undefined,
      status: 'complete',
      timestamp: Date.now(),
    };

    const assistantMsg: ChatMessage = {
      id: `msg_${Date.now()}_a`,
      role: 'assistant',
      content: '',
      thought: isSteer
        ? 'Course correcting and steering execution based on immediate directive...'
        : 'Analyzing request and inspecting relevant codebase context...',
      thoughtDurationSeconds: isSteer ? 1 : 2,
      status: 'streaming',
      timestamp: Date.now(),
      toolCalls: [],
    };

    // If existing todos are all completed or empty, initialize a new plan starting with this user task
    const hasPending = state.todos.some((t) => t.status !== 'completed');
    const initialTodo: AgentTodo = {
      id: `todo_${Date.now()}`,
      title: isSteer ? `Steer: ${trimmed.slice(0, 45)}` : trimmed,
      status: 'in_progress',
      assignedSubagent: 'coding-agent',
    };
    const updatedTodos = hasPending
      ? [...state.todos, initialTodo]
      : [initialTodo];

    const updatedSubagents = state.subagents.map((sub, i) =>
      i === 0
        ? {
            ...sub,
            status: 'working' as const,
            activeStep: isSteer
              ? `Steering: ${trimmed.slice(0, 40)}`
              : `Planning: ${trimmed.slice(0, 45)}`,
          }
        : sub
    );

    state = {
      ...state,
      messages: [...state.messages, userMsg, assistantMsg],
      todos: updatedTodos,
      subagents: updatedSubagents,
      inFlight: true,
      thinkingState: 'working',
      streamingActivityLevel: isSteer ? 0.9 : 0.7,
    };
    notify();

    const promptToSend = isSteer
      ? `[STEERING DIRECTIVE - IMMEDIATE COURSE CORRECTION]: ${trimmed}`
      : trimmed;

    if (context) {
      const mode = state.permissionMode === 'plan_only' ? 'explore' : 'execute';
      let modelToSend = state.modelSettings.customModelId.trim() || state.selectedModel;
      if (!state.modelSettings.customModelId.trim() && state.availableModels.length > 0) {
        const matched = state.availableModels.find(
          (m) => m.id === state.selectedModel || m.display_name === state.selectedModel
        );
        if (matched) {
          modelToSend = matched.id;
        } else if (state.availableModels[0]) {
          modelToSend = state.availableModels[0].id;
        }
      }

      const { requestId, promise } = sendAgent(
        promptToSend,
        mode,
        modelToSend,
        {
          ...context,
          session_id: state.activeSessionId,
        },
        {
          reasoningEffort: state.reasoningEffort,
          systemInstructions: state.modelSettings.systemInstructions,
          temperature: state.modelSettings.temperature,
        },
      );
      activeRequestId = requestId;
      promise.catch((err: unknown) => {
        const errObj = err as { code?: string; message?: string } | undefined;
        const lastIdx = state.messages.length - 1;
        if (lastIdx >= 0 && state.messages[lastIdx].role === 'assistant') {
          const content =
            errObj?.code === 'not_connected'
              ? 'No AI provider is connected yet. Click the "connect" button in the bottom status bar or open Settings to configure your API key.'
              : errObj?.code === 'no_workspace'
              ? 'No workspace is currently open. Open a folder to start using the autonomous coding agent.'
              : `Could not reach the assistant: ${errObj?.message || 'Connection unavailable.'}`;

          const updated = {
            ...state.messages[lastIdx],
            content,
            status: 'complete' as const,
          };
          const newMsgs = [...state.messages];
          newMsgs[lastIdx] = updated;
          state = {
            ...state,
            messages: newMsgs,
            inFlight: false,
            thinkingState: 'breathing',
            streamingActivityLevel: 0,
          };
          notify();
          this.checkAndExecuteNextQueued();
        }
      });
    } else {
      // Offline / standalone browser preview fallback
      setTimeout(() => {
        const lastIdx = state.messages.length - 1;
        if (lastIdx >= 0 && state.messages[lastIdx].role === 'assistant') {
          const updated = {
            ...state.messages[lastIdx],
            content: isSteer
              ? `Steering directive acknowledged: "${trimmed}". Adjusting agent trajectory.`
              : `I received: "${trimmed}". Connect your API key in the bottom status bar to enable live AI inference and autonomous coding tools.`,
            status: 'complete' as const,
          };
          const newMsgs = [...state.messages];
          newMsgs[lastIdx] = updated;
          state = {
            ...state,
            messages: newMsgs,
            inFlight: false,
            thinkingState: 'breathing',
            streamingActivityLevel: 0,
          };
          notify();
          this.checkAndExecuteNextQueued();
        }
      }, 500);
    }
  },

  steerActiveTurn(text: string) {
    const trimmed = text.trim();
    if (!trimmed) return;
    if (state.inFlight) {
      if (context && activeRequestId) {
        void stopChat(activeRequestId, context).catch(() => {});
      }
      activeRequestId = null;
    }
    this.sendUserMessage(trimmed, true);
  },

  async copyPrompt(text: string): Promise<boolean> {
    try {
      if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
        return true;
      }
    } catch (err) {
      console.warn('Clipboard copy error:', err);
    }
    return false;
  },

  rewindToPrompt(messageId: string, restoreDraft = true): { restoredText: string } | null {
    const idx = state.messages.findIndex((m) => m.id === messageId);
    if (idx === -1) return null;
    const targetMsg = state.messages[idx];
    const restoredText = targetMsg.content;

    if (state.inFlight) {
      this.stopActiveTurn();
    }

    // Rewind rolls back messages to before this prompt
    const kept = state.messages.slice(0, idx);

    state = {
      ...state,
      messages: kept,
      inFlight: false,
      thinkingState: 'breathing',
      streamingActivityLevel: 0,
    };
    notify();

    if (restoreDraft && typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent('selora:set-composer-draft', { detail: { text: restoredText } })
      );
    }

    return { restoredText };
  },

  editPromptInPlace(messageId: string, newText: string) {
    const trimmed = newText.trim();
    if (!trimmed) return;
    const updated = state.messages.map((m) =>
      m.id === messageId ? { ...m, content: trimmed, isEdited: true } : m
    );
    state = { ...state, messages: updated };
    notify();
  },

  resubmitEditedPrompt(messageId: string, newText: string) {
    const trimmed = newText.trim();
    if (!trimmed) return;
    const idx = state.messages.findIndex((m) => m.id === messageId);
    if (idx === -1) return;
    if (state.inFlight) {
      this.stopActiveTurn();
    }
    const kept = state.messages.slice(0, idx);
    state = { ...state, messages: kept };
    notify();
    this.sendUserMessage(trimmed);
  },

  forkSessionFromPrompt(messageId: string): string {
    const idx = state.messages.findIndex((m) => m.id === messageId);
    if (idx === -1) return '';
    const targetMsg = state.messages[idx];
    const sliced = state.messages.slice(0, idx + 1);
    const titleSnippet = targetMsg.content.slice(0, 24).trim();
    const newSessionId = this.createSession(`Fork: ${titleSnippet || 'Checkpoint'}`);
    state = {
      ...state,
      messages: sliced.map((m) => ({ ...m })),
    };
    notify();
    return newSessionId;
  },

  queuePrompt(text: string, mode?: PermissionMode): string {
    const trimmed = text.trim();
    if (!trimmed) return '';
    if (!state.inFlight) {
      this.sendUserMessage(trimmed);
      return '';
    }
    const id = `q_${Date.now()}_${Math.random().toString(36).slice(2, 6)}`;
    const queuedItem: QueuedPrompt = {
      id,
      text: trimmed,
      createdAt: Date.now(),
      mode: mode || state.permissionMode,
    };
    state = {
      ...state,
      promptQueue: [...state.promptQueue, queuedItem],
    };
    notify();
    return id;
  },

  removeQueuedPrompt(id: string) {
    state = {
      ...state,
      promptQueue: state.promptQueue.filter((q) => q.id !== id),
    };
    notify();
  },

  updateQueuedPrompt(id: string, newText: string) {
    const trimmed = newText.trim();
    if (!trimmed) return;
    state = {
      ...state,
      promptQueue: state.promptQueue.map((q) =>
        q.id === id ? { ...q, text: trimmed } : q
      ),
    };
    notify();
  },

  clearPromptQueue() {
    state = {
      ...state,
      promptQueue: [],
    };
    notify();
  },

  promoteQueuedPrompt(id: string) {
    const item = state.promptQueue.find((q) => q.id === id);
    if (!item) return;
    const remaining = state.promptQueue.filter((q) => q.id !== id);
    state = {
      ...state,
      promptQueue: [item, ...remaining],
    };
    notify();
  },

  reorderPromptQueue(fromIndex: number, toIndex: number) {
    if (fromIndex < 0 || fromIndex >= state.promptQueue.length) return;
    if (toIndex < 0 || toIndex >= state.promptQueue.length) return;
    const q = [...state.promptQueue];
    const [moved] = q.splice(fromIndex, 1);
    q.splice(toIndex, 0, moved);
    state = { ...state, promptQueue: q };
    notify();
  },

  checkAndExecuteNextQueued() {
    if (state.inFlight || state.promptQueue.length === 0) return;
    const nextItem = state.promptQueue[0];
    state = {
      ...state,
      promptQueue: state.promptQueue.slice(1),
    };
    notify();
    setTimeout(() => {
      this.sendUserMessage(nextItem.text);
    }, 80);
  },

  appendAssistantDelta(delta: string) {
    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex < 0) return;
    const lastMsg = state.messages[lastMsgIndex];
    if (!lastMsg || lastMsg.role !== 'assistant') return;

    const newContent = lastMsg.content + delta;
    const updated = {
      ...lastMsg,
      content: newContent,
    };

    const newMessages = [...state.messages];
    newMessages[lastMsgIndex] = updated;

    let updatedTodos = state.todos;
    const mdTodos = extractMarkdownChecklist(newContent);
    if (mdTodos && mdTodos.length > 0) {
      updatedTodos = mdTodos;
    }

    state = {
      ...state,
      messages: newMessages,
      todos: updatedTodos,
      thinkingState: 'connecting',
      streamingActivityLevel: Math.min(1.0, state.streamingActivityLevel + 0.1),
    };
    notify();
  },

  addToolCall(toolCall: Omit<AgentToolCall, 'id' | 'startedAt'>): string {
    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex < 0) return '';
    const lastMsg = state.messages[lastMsgIndex];
    if (!lastMsg || lastMsg.role !== 'assistant') return '';

    const callId = `call_${Date.now()}_${Math.random().toString(36).slice(2, 6)}`;
    const fullCall: AgentToolCall = {
      ...toolCall,
      id: callId,
      startedAt: Date.now(),
    };

    const updated = {
      ...lastMsg,
      toolCalls: [...(lastMsg.toolCalls || []), fullCall],
    };

    const newMessages = [...state.messages];
    newMessages[lastMsgIndex] = updated;

    state = {
      ...state,
      messages: newMessages,
      thinkingState: fullCall.toolName === 'read_file' ? 'searching' : 'solving',
    };
    notify();
    return callId;
  },

  updateToolResult(callId: string, resultSummary: string, isErr = false, exitCode?: number) {
    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex < 0) return;
    const lastMsg = state.messages[lastMsgIndex];
    if (!lastMsg || !lastMsg.toolCalls) return;

    const updatedCalls = lastMsg.toolCalls.map((c) => {
      if (c.id === callId) {
        return {
          ...c,
          status: (isErr ? 'error' : 'completed') as AgentToolCall['status'],
          resultSummary,
          exitCode: exitCode ?? (isErr ? 1 : 0),
          durationMs: Date.now() - c.startedAt,
        };
      }
      return c;
    });

    const updated = { ...lastMsg, toolCalls: updatedCalls };
    const newMessages = [...state.messages];
    newMessages[lastMsgIndex] = updated;

    state = { ...state, messages: newMessages, thinkingState: 'working' };
    notify();
  },

  handleApprovalDecision(approvalId: string, decision: 'approve' | 'deny' | 'approve_all') {
    if (context && activeRequestId) {
      const mappedDecision = decision === 'approve_all' ? 'approve_all_this_turn' : decision;
      void agentDecision(activeRequestId, approvalId, mappedDecision, context).catch(() => {});
    }

    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex < 0) return;
    const lastMsg = state.messages[lastMsgIndex];
    if (!lastMsg || !lastMsg.toolCalls) return;

    const updatedCalls = lastMsg.toolCalls.map((c) => {
      if (c.approvalId === approvalId || decision === 'approve_all') {
        return {
          ...c,
          status: (decision === 'deny' ? 'denied' : 'approved') as AgentToolCall['status'],
        };
      }
      return c;
    });

    const updated = { ...lastMsg, toolCalls: updatedCalls };
    const newMessages = [...state.messages];
    newMessages[lastMsgIndex] = updated;

    state = {
      ...state,
      messages: newMessages,
      thinkingState: decision === 'deny' ? 'breathing' : 'solving',
    };
    notify();
  },

  completeActiveTurn(metricsUpdate?: Partial<AgentMetrics>) {
    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex >= 0) {
      const lastMsg = state.messages[lastMsgIndex];
      if (lastMsg && lastMsg.role === 'assistant') {
        const updated = { ...lastMsg, status: 'complete' as const };
        const newMessages = [...state.messages];
        newMessages[lastMsgIndex] = updated;
        state = { ...state, messages: newMessages };
      }
    }

    state = {
      ...state,
      inFlight: false,
      thinkingState: 'breathing',
      streamingActivityLevel: 0,
      metrics: {
        ...state.metrics,
        ...metricsUpdate,
      },
    };
    activeRequestId = null;
    notify();
    this.checkAndExecuteNextQueued();
  },

  stopActiveTurn() {
    if (context && activeRequestId) {
      void stopChat(activeRequestId, context).catch(() => {});
    }
    this.completeActiveTurn();
  },

  handleChatDelta(payload: { request_id: string; text: string }) {
    this.appendAssistantDelta(payload.text);
  },

  handleChatCompleted(payload: ChatCompletedPayload) {
    this.completeActiveTurn({
      totalTokens: payload.total_tokens,
      costUsd: payload.cost_usd,
    });
  },

  handleChatFailed(payload: { error: { message: string } }) {
    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex >= 0 && state.messages[lastMsgIndex]?.role === 'assistant') {
      const lastMsg = state.messages[lastMsgIndex]!;
      const updated = {
        ...lastMsg,
        content: lastMsg.content
          ? `${lastMsg.content}\n\n[Error: ${payload.error.message}]`
          : `[Error: ${payload.error.message}]`,
        status: 'error' as const,
      };
      const newMessages = [...state.messages];
      newMessages[lastMsgIndex] = updated;
      state = { ...state, messages: newMessages };
    }
    this.completeActiveTurn();
  },

  handleToolCallStarted(payload: ToolCallStartedPayload) {
    let parsedArgs: Record<string, unknown> | undefined;
    let argsSummary = payload.tool_name;
    try {
      parsedArgs = JSON.parse(payload.args_json);
      if (parsedArgs && typeof parsedArgs === 'object') {
        if ('path' in parsedArgs) argsSummary = String(parsedArgs.path);
        else if ('command' in parsedArgs) argsSummary = String(parsedArgs.command);
        else if ('query' in parsedArgs) argsSummary = String(parsedArgs.query);
        else argsSummary = JSON.stringify(parsedArgs);
      }
    } catch {
      argsSummary = payload.args_json;
    }

    // 1. If tool is manage_todo or update_plan, extract and update todos
    let updatedTodos = state.todos;
    if (payload.tool_name === 'manage_todo' || payload.tool_name === 'update_plan') {
      const raw = parsedArgs ? (parsedArgs.todos ?? parsedArgs.tasks ?? parsedArgs.steps ?? parsedArgs) : payload.args_json;
      const modelTodos = parseModelTodos(raw);
      if (modelTodos && modelTodos.length > 0) {
        updatedTodos = modelTodos;
      }
    } else {
      // If there are pending todos and one is in_progress, keep it; if none in_progress, make first pending in_progress
      const hasInProgress = updatedTodos.some((t) => t.status === 'in_progress');
      if (!hasInProgress && updatedTodos.length > 0) {
        const firstPending = updatedTodos.find((t) => t.status === 'pending');
        if (firstPending) {
          updatedTodos = updatedTodos.map((t) =>
            t.id === firstPending.id ? { ...t, status: 'in_progress' as const } : t
          );
        }
      }
    }

    // 2. Update subagents active step
    const updatedSubagents = state.subagents.map((sub, i) =>
      i === 0
        ? {
            ...sub,
            status: 'working' as const,
            activeStep: `${payload.tool_name}: ${argsSummary.slice(0, 45)}`,
          }
        : sub
    );

    const toolCall: AgentToolCall = {
      id: payload.call_id,
      toolName: payload.tool_name,
      argsSummary,
      argsRaw: parsedArgs,
      status: 'running',
      startedAt: Date.now(),
    };

    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex >= 0 && state.messages[lastMsgIndex]?.role === 'assistant') {
      const lastMsg = state.messages[lastMsgIndex]!;
      const updated = {
        ...lastMsg,
        toolCalls: [...(lastMsg.toolCalls || []), toolCall],
      };
      const newMessages = [...state.messages];
      newMessages[lastMsgIndex] = updated;
      state = {
        ...state,
        messages: newMessages,
        todos: updatedTodos,
        subagents: updatedSubagents,
        thinkingState: payload.tool_name === 'read_file' ? 'searching' : 'solving',
      };
      notify();
    } else {
      state = {
        ...state,
        todos: updatedTodos,
        subagents: updatedSubagents,
      };
      notify();
    }
  },

  handleToolResult(payload: ToolResultPayload) {
    this.updateToolResult(payload.call_id, payload.result_summary, payload.is_err);
    const updatedSubagents = state.subagents.map((sub, i) =>
      i === 0
        ? {
            ...sub,
            activeStep: payload.is_err
              ? `Error: ${payload.result_summary.slice(0, 40)}`
              : `Completed: ${payload.result_summary.slice(0, 40)}`,
          }
        : sub
    );
    state = { ...state, subagents: updatedSubagents };
    notify();
  },

  handleApprovalNeeded(payload: ApprovalNeededPayload) {
    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex < 0) return;
    const lastMsg = state.messages[lastMsgIndex];
    if (!lastMsg) return;

    const diffHunks = payload.diff_or_input ? [payload.diff_or_input] : undefined;
    const approvalCall: AgentToolCall = {
      id: `call_appr_${payload.approval_id}`,
      toolName: payload.tool_name,
      argsSummary: payload.path_or_command,
      status: 'needs_approval',
      approvalId: payload.approval_id,
      diff: diffHunks
        ? {
            added: 1,
            removed: 0,
            hunks: diffHunks,
          }
        : undefined,
      startedAt: Date.now(),
    };

    const updated = {
      ...lastMsg,
      toolCalls: [...(lastMsg.toolCalls || []), approvalCall],
    };
    const newMessages = [...state.messages];
    newMessages[lastMsgIndex] = updated;

    state = {
      ...state,
      messages: newMessages,
      thinkingState: 'breathing',
    };
    notify();
  },

  handleAgentStateChanged(payload: AgentStateChangedPayload) {
    let mappedState: ThinkingState = 'breathing';
    switch (payload.state) {
      case 'thinking':
        mappedState = 'working';
        break;
      case 'running_tool':
        mappedState = payload.tool_name === 'read_file' ? 'searching' : 'solving';
        break;
      case 'awaiting_approval':
      case 'done':
        mappedState = 'breathing';
        break;
    }
    state = {
      ...state,
      thinkingState: mappedState,
      inFlight: payload.state !== 'done',
    };
    notify();
  },

  handleAgentTurnDone(payload: AgentTurnDonePayload) {
    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex >= 0 && state.messages[lastMsgIndex]?.role === 'assistant') {
      const lastMsg = state.messages[lastMsgIndex]!;
      let content = lastMsg.content;
      if (!content.trim()) {
        if (payload.end_message && payload.end_message.trim()) {
          content = payload.end_message;
        } else if (payload.completed) {
          content = 'I have completed your request.';
        } else {
          content = 'The assistant completed without output. Please check your model settings and connection.';
        }
      }
      const updated: ChatMessage = {
        ...lastMsg,
        content,
        status: payload.completed ? 'complete' : 'error',
      };
      const newMessages = [...state.messages];
      newMessages[lastMsgIndex] = updated;
      state = { ...state, messages: newMessages };
    }

    // Mark in-progress todos as completed on successful turn completion
    const finishedTodos = state.todos.map((t) =>
      t.status === 'in_progress' ? { ...t, status: 'completed' as const } : t
    );

    const idleSubagents = state.subagents.map((s) => ({
      ...s,
      status: 'idle' as const,
      activeStep: 'Ready',
    }));

    state = {
      ...state,
      todos: finishedTodos,
      subagents: idleSubagents,
      inFlight: false,
      thinkingState: 'breathing',
      streamingActivityLevel: 0,
      metrics: {
        ...state.metrics,
        totalTokens: payload.total_tokens,
        costUsd: payload.cost_usd,
        activeToolsCount: payload.num_tools,
        approvedCount: payload.approved,
        deniedCount: payload.denied,
      },
    };
    activeRequestId = null;
    notify();
    this.checkAndExecuteNextQueued();
  },

  handleAgentEvent(payload: { request_id: string; approval: { approval_id: string; summary: string } | null; message: string }) {
    if (payload.approval) {
      this.handleApprovalNeeded({
        request_id: payload.request_id,
        session_id: state.activeSessionId,
        turn_id: 'turn_live',
        approval_id: payload.approval.approval_id,
        tool_name: 'operation',
        path_or_command: payload.approval.summary,
      });
    }
    if (payload.message) {
      this.appendAssistantDelta(payload.message);
    }
  },

  updateTodoStatus(todoId: string, status: TodoStatus) {
    const updated = state.todos.map((todo) => {
      if (todo.id === todoId) {
        return { ...todo, status };
      }
      return todo;
    });
    state = { ...state, todos: updated };
    notify();
  },

  addTodo(title: string, assignedSubagent?: string) {
    const newTodo: AgentTodo = {
      id: `todo-${Date.now().toString(36)}`,
      title,
      status: 'pending',
      assignedSubagent,
    };
    state = { ...state, todos: [...state.todos, newTodo] };
    notify();
  },

  removeTodo(todoId: string) {
    state = { ...state, todos: state.todos.filter((t) => t.id !== todoId) };
    notify();
  },

  addArtifact(artifact: Omit<ArtifactItem, 'id' | 'createdAt'>) {
    const newArt: ArtifactItem = {
      ...artifact,
      id: `art-${Date.now().toString(36)}`,
      createdAt: Date.now(),
    };
    state = { ...state, artifacts: [newArt, ...state.artifacts] };
    notify();
  },
};
