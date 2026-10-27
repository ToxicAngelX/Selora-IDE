// Selora IDE — Phase Z3 Agent Store
//
// Manages agent sessions, messages, tool executions, approval gates,
// live todos, subagents, and hero composer interactions.
// Implemented via React 18 external store subscription pattern.
import type {
  AgentSession,
  AgentToolCall,
  AgentTodo,
  AgentWorkspace,
  ArtifactItem,
  ChatMessage,
  ComposerGate,
  PermissionMode,
  QueuedPrompt,
  ReasoningEffort,
  ResumableAgentSession,
  SubagentInfo,
  ThinkingState,
  TodoStatus,
  AgentMetrics,
} from './types';
import {
  agentDecision,
  fetchModels,
  listAgentSessions,
  newSession as bridgeNewSession,
  retryMessage as bridgeRetryMessage,
  runWorkflow,
  sendAgent,
  sendChat,
  setAgentAutoApprove,
  stopChat,
  switchSession as bridgeSwitchSession,
  type AgentMode,
  type AgentStateChangedPayload,
  type AgentTurnDonePayload,
  type ApprovalNeededPayload,
  type BootstrapState,
  type ChatCompletedPayload,
  type EditorContextDto,
  type EnvelopeContext,
  type Model,
  type PlanApproval,
  type PlanPhase,
  type PlanStateChangedPayload,
  type SubagentCompletedPayload,
  type TodoItem,
  type TodosChangedPayload,
  type ToolCallStartedPayload,
  type ToolResultPayload,
  type WorkflowApproval,
} from '../../bridge/bridge';

export type ModelSettings = {
  temperature: number;
  systemInstructions: string;
  customModelId: string;
};

export type AgentState = {
  sessions: AgentSession[];
  /** Phase 9d: durable sessions from Rust (the resume picker). */
  resumableSessions: ResumableAgentSession[];
  activeSessionId: string;
  messages: ChatMessage[];
  todos: AgentTodo[];
  /** Phase 9b: the session's plan phase (hydrated + live). */
  planPhase: PlanPhase;
  /** Phase 9b: the approved/proposed plan, when any. */
  plan: PlanApproval | null;
  subagents: SubagentInfo[];
  /** Phase 9e: completion notifications not yet opened (activity badge). */
  unreadSubagentCount: number;
  /** Phase 9e: the child whose result the UI should reveal (click hook). */
  revealedSubagentId: string | null;
  artifacts: ArtifactItem[];
  inFlight: boolean;
  thinkingState: ThinkingState;
  permissionMode: PermissionMode;
  reasoningEffort: ReasoningEffort;
  /** Backend-reported model id only — empty until bootstrap/models_changed arrives. */
  selectedModel: string;
  availableModels: Model[];
  /** Null until the Rust bootstrap reports an open workspace. Never invented. */
  workspace: AgentWorkspace | null;
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

/** Shown when Rust reports no open workspace. */
export const NO_WORKSPACE_MESSAGE =
  'The agent needs an open folder to read and write files. Open a folder, then connect and refresh models.';

/** Shown when the backend catalog has no usable model. */
export const NO_MODEL_MESSAGE =
  'Connect and refresh models: no model is available to send this prompt to.';

/**
 * The exact model id a turn may send, or `null` when nothing is known.
 * Only backend-reported catalog entries (or an explicit user-entered override)
 * qualify — we never guess a model id.
 */
export function resolveModelId(snapshot: AgentState): string | null {
  const custom = snapshot.modelSettings.customModelId.trim();
  if (custom) return custom;
  const selected = snapshot.selectedModel.trim();
  if (!selected) return null;
  const matched = snapshot.availableModels.find(
    (model) => model.id === selected || model.display_name === selected,
  );
  return matched ? matched.id : null;
}

/** Store permission mode → the AgentMode the backend understands. */
export function toAgentMode(mode: PermissionMode): AgentMode {
  if (mode === 'full') return 'auto';
  if (mode === 'ask_before_change') return 'execute';
  return 'explore';
}

/**
 * Gate a send on a real workspace + a real model. The composer renders the
 * message and disables send; the store refuses the turn for the same reason.
 */
export function getComposerGate(snapshot: AgentState): ComposerGate {
  if (!snapshot.workspace) {
    return { ready: false, reason: 'no_workspace', message: NO_WORKSPACE_MESSAGE };
  }
  if (!resolveModelId(snapshot)) {
    return { ready: false, reason: 'no_model', message: NO_MODEL_MESSAGE };
  }
  return { ready: true, reason: null, message: null };
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
  resumableSessions: [],
  activeSessionId: DEFAULT_SESSION_ID,
  messages: [],
  todos: INITIAL_TODOS,
  planPhase: 'executing',
  plan: null,
  subagents: INITIAL_SUBAGENTS,
  unreadSubagentCount: 0,
  revealedSubagentId: null,
  artifacts: INITIAL_ARTIFACTS,
  inFlight: false,
  thinkingState: 'breathing',
  permissionMode: 'full',
  reasoningEffort: 'max',
  selectedModel: '',
  availableModels: [],
  workspace: null,
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

/**
 * Phase 9e: the dedupe ledger. A child id reaches this store once, ever;
 * a replayed protocol event or a duplicate completion is ignored rather
 * than producing a second toast/badge/result.
 */
const completedSubagentIds = new Set<string>();

function countUnreadSubagents(subagents: SubagentInfo[]): number {
  return subagents.reduce((count, subagent) => (subagent.unread ? count + 1 : count), 0);
}

/**
 * A `run_workflow` card also carries the §18.1 confirmation payload (script,
 * confirmed command set, cost cap) so the chat renders the same grant the
 * approval carries. `types.ts` is outside this slice's edit set, so the field
 * rides an intersection rather than growing `AgentToolCall` itself.
 */
export type WorkflowToolCall = AgentToolCall & {
  workflow?: WorkflowApproval;
  /** Phase 9b: the plan payload on an `exit_plan_mode` card. */
  plan?: PlanApproval;
  approvalSubmitting?: boolean;
};

/** Map the bridge's snake_case todo items onto the dock's `AgentTodo`. */
export function todosFromSnapshot(items: TodoItem[] | undefined): AgentTodo[] {
  if (!items) return [];
  return items
    .filter((item) => item && typeof item.title === 'string' && item.title.trim())
    .map((item) => ({
      id: item.id || `todo_${item.title}`,
      title: item.title,
      status:
        item.status === 'completed'
          ? 'completed'
          : item.status === 'in_progress'
            ? 'in_progress'
            : 'pending',
    }));
}

/** Validate a plan payload before it drives the approval card. */
export function validPlanApproval(
  plan: PlanApproval | null | undefined,
): plan is PlanApproval {
  return Boolean(
    plan &&
      Array.isArray(plan.steps) &&
      plan.steps.length > 0 &&
      plan.steps.every((step) => typeof step === 'string' && step.trim()),
  );
}

/**
 * The plan payload from an `exit_plan_mode` call's arguments. The
 * authoritative repeat rides `approval_needed`; this lets the running card
 * render before that arrives, without inventing steps.
 */
export function planApprovalFromArgs(
  args: Record<string, unknown> | undefined,
): PlanApproval | undefined {
  if (!args || !Array.isArray(args.plan)) return undefined;
  const steps = args.plan.filter(
    (step): step is string => typeof step === 'string' && Boolean(step.trim()),
  );
  if (steps.length === 0) return undefined;
  return {
    summary: typeof args.summary === 'string' ? args.summary : undefined,
    steps,
  };
}

/** §18.1's workflow-level cap default, mirroring `WorkflowApprovalDto`. */
const DEFAULT_WORKFLOW_MAX_COST_USD = 5;

/**
 * The confirmation payload from a `run_workflow` call's arguments — the
 * `tool_call_started` event carries the same four fields the `approval_needed`
 * event then repeats authoritatively. A malformed/absent field falls to the
 * same defaults Rust applies (empty list, §18.1 cap) instead of inventing one.
 */
export function workflowApprovalFromArgs(
  args: Record<string, unknown> | undefined,
): WorkflowApproval | undefined {
  if (!args) return undefined;
  const name = typeof args.name === 'string' ? args.name : '';
  const script = typeof args.script === 'string' ? args.script : '';
  if (!name && !script) return undefined;
  const commands = Array.isArray(args.commands)
    ? args.commands.filter((command): command is string => typeof command === 'string')
    : [];
  const cap = args.max_cost_usd;
  return {
    name,
    script,
    commands,
    max_cost_usd:
      typeof cap === 'number' && Number.isFinite(cap) && cap > 0
        ? cap
        : DEFAULT_WORKFLOW_MAX_COST_USD,
  };
}

// Workspace/core-instance identity only — `session_id` stays null here. Rust
// dispatch rejects a session on the envelope for every command except the
// session-scoped ones, so only sessionEnvelopeContext() may add it.
let context: EnvelopeContext | null = null;
let activeRequestId: string | null = null;

/**
 * The request that raised each parked approval, keyed by `approval_id`.
 * `AgentDecision` must target *that* request — the global `activeRequestId`
 * may already belong to a newer turn (or be null after a reload), which
 * would silently lose the decision.
 */
const pendingApprovalRequests = new Map<string, string>();

type WorkflowConfirmation = {
  requestId: string;
  sessionId: string;
  context: EnvelopeContext | null;
  workflow: WorkflowApproval | null;
  submitting: boolean;
  accepted: boolean;
};
const workflowConfirmations = new Map<string, WorkflowConfirmation>();
const terminalRequests = new Set<string>();

function updateWorkflowConfirmationCard(approvalId: string, update: Partial<WorkflowToolCall>) {
  state = {
    ...state,
    messages: state.messages.map((message) => ({
      ...message,
      toolCalls: message.toolCalls?.map((call) =>
        call.approvalId === approvalId ? { ...call, ...update } : call,
      ),
    })),
  };
  notify();
}

function workflowConfirmationError(approvalId: string, error: unknown) {
  const message = error && typeof error === 'object' && 'message' in error
    ? String(error.message)
    : typeof error === 'string' ? error : 'Request failed. Try again.';
  const acceptedRunNote = workflowConfirmations.get(approvalId)?.accepted
    ? '\nThe workflow already started. Retry Approve to record the decision without starting another run.'
    : '';
  updateWorkflowConfirmationCard(approvalId, {
    resultSummary: `Workflow confirmation failed: ${message}${acceptedRunNote}`,
    approvalSubmitting: false,
  });
}

function validWorkflowApproval(workflow: WorkflowApproval | null | undefined): workflow is WorkflowApproval {
  return Boolean(
    workflow &&
    typeof workflow.name === 'string' &&
    typeof workflow.script === 'string' && workflow.script.trim() &&
    Array.isArray(workflow.commands) && workflow.commands.every((command) => typeof command === 'string') &&
    Number.isFinite(workflow.max_cost_usd) && workflow.max_cost_usd > 0,
  );
}

async function submitWorkflowConfirmation(
  approvalId: string,
  decision: 'approve' | 'deny' | 'approve_all',
): Promise<void> {
  const confirmation = workflowConfirmations.get(approvalId);
  if (confirmation?.submitting) return;
  const card = state.messages.flatMap((message) => message.toolCalls ?? [])
    .find((call) => call.approvalId === approvalId);
  if (!card || card.status !== 'needs_approval') return;
  if (decision !== 'approve' && decision !== 'deny') {
    workflowConfirmationError(approvalId, 'Workflow scripts require an explicit Approve, not Approve all.');
    return;
  }
  if (!confirmation?.workflow || pendingApprovalRequests.get(approvalId) !== confirmation.requestId) {
    workflowConfirmationError(approvalId, 'The authoritative workflow approval is missing. Wait for approval_needed.');
    return;
  }
  if (confirmation.accepted && decision !== 'approve') {
    workflowConfirmationError(approvalId, 'This workflow already started and cannot be denied. Only retry Approve to record the approval.');
    return;
  }
  const liveContext = () => {
    const pendingCard = state.messages.some((message) => message.toolCalls?.some(
      (call) => call.approvalId === approvalId && call.status === 'needs_approval',
    ));
    if (
      workflowConfirmations.get(approvalId) !== confirmation ||
      terminalRequests.has(confirmation.requestId) || !state.inFlight ||
      activeRequestId !== confirmation.requestId || state.activeSessionId !== confirmation.sessionId ||
      !pendingCard
    ) {
      throw new Error('This workflow approval no longer matches the pending request.');
    }
    if (!confirmation.workflow) {
      throw new Error('The authoritative workflow approval is missing. Wait for approval_needed.');
    }
    if (
      !context || !confirmation.context ||
      context.workspace_id !== confirmation.context.workspace_id ||
      context.core_instance_id !== confirmation.context.core_instance_id
    ) {
      throw new Error('Switch back to the workspace that requested this workflow.');
    }
    return { ...context, session_id: null };
  };
  confirmation.submitting = true;
  updateWorkflowConfirmationCard(approvalId, { approvalSubmitting: true, resultSummary: undefined });
  try {
    let currentContext = liveContext();
    if (decision === 'approve' && !confirmation.accepted) {
      await runWorkflow(confirmation.workflow, currentContext).promise;
      // If recording the agent decision fails, retry only that decision, never the accepted run.
      confirmation.accepted = true;
    }
    currentContext = liveContext();
    await agentDecision(confirmation.requestId, approvalId, decision, {
      ...currentContext,
      session_id: confirmation.sessionId,
    });
    if (workflowConfirmations.get(approvalId) !== confirmation) return;
    pendingApprovalRequests.delete(approvalId);
    updateWorkflowConfirmationCard(approvalId, {
      status: decision === 'deny' ? 'denied' : 'approved',
      approvalSubmitting: false,
      resultSummary: undefined,
    });
  } catch (error) {
    if (workflowConfirmations.get(approvalId) === confirmation) workflowConfirmationError(approvalId, error);
  } finally {
    confirmation.submitting = false;
  }
}

/**
 * Drop events that belong to a stopped/retried turn or another session.
 * Payloads without a `request_id` (some events) stay compatible; a
 * request-scoped event is accepted only when it matches the live request or,
 * if the id was never recorded, while a turn is in flight.
 */
function isActiveEvent(payload: {
  session_id?: string | null;
  request_id?: string | null;
}): boolean {
  if (payload.session_id && state.activeSessionId && payload.session_id !== state.activeSessionId) {
    return false;
  }
  const requestId = payload.request_id;
  if (requestId && terminalRequests.has(requestId)) return false;
  if (!requestId) return true;
  if (activeRequestId) return requestId === activeRequestId;
  // No tracked request: accept while a turn is in flight (the id was not
  // recorded, e.g. an offline/standalone turn); otherwise it is late.
  return state.inFlight;
}

/**
 * The envelope context for SendAgent/SendChat/StopChat/AgentDecision. These
 * are the only session-scoped commands; they carry the live chat session so
 * stop/decision still target the turn in flight.
 */
function sessionEnvelopeContext(): EnvelopeContext | null {
  if (!context) return null;
  return { ...context, session_id: state.activeSessionId || null };
}

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
    // Workspace identity comes only from Rust. A null workspace means no
    // folder is open — the agent has nowhere to read or write, so we keep
    // `workspace` null and never synthesize one.
    const bootstrapWorkspace = bootstrap.workspace ?? null;
    const workspace: AgentWorkspace | null = bootstrapWorkspace
      ? {
          id: bootstrapWorkspace.id,
          root: bootstrapWorkspace.root ?? null,
          displayName: bootstrapWorkspace.display_name,
        }
      : null;
    context = workspace
      ? {
          core_instance_id: bootstrap.core_instance_id,
          workspace_id: workspace.id,
          session_id: null,
        }
      : null;

    const models = bootstrap.models ?? [];
    let modelToSelect = '';
    if (bootstrap.active_model_id && models.some((model) => model.id === bootstrap.active_model_id)) {
      modelToSelect = bootstrap.active_model_id;
    } else if (models.length > 0) {
      const available = models.find((model) => model.available) ?? models[0];
      modelToSelect = available ? available.id : '';
    }

    // The Rust snapshot is the source of truth for the session rail and the
    // transcript after a reload — never invent a synthetic session.
    const sessions: AgentSession[] = bootstrap.chat.sessions.map((session) => ({
      id: session.session_id,
      title: session.title,
      createdAt: Date.now(),
      updatedAt: Date.now(),
      tokenCount: 0,
      costUsd: 0,
    }));

    // Durable sessions come from `.selora/agent-sessions` — a read-only
    // projection Rust owns. Never synthesized locally.
    const resumableSessions: ResumableAgentSession[] = (bootstrap.agent_sessions ?? []).map(
      (session) => ({
        id: session.session_id,
        title: session.title,
        updatedAt: session.updated_at,
        ...(session.model ? { model: session.model } : {}),
        mode: session.mode,
        ...(session.pending ? { pending: session.pending } : {}),
        compacted: session.compacted,
        messageCount: session.message_count,
      }),
    );

    const messages: ChatMessage[] = [];
    for (const message of bootstrap.chat.messages) {
      if (message.role !== 'user' && message.role !== 'assistant') continue;
      messages.push({
        id: message.message_id,
        role: message.role,
        content: message.text,
        status: 'complete',
        timestamp: Date.now(),
      });
    }

    const activeSessionId =
      bootstrap.chat.active_session_id ??
      bootstrap.chat.sessions.find((session) => session.active)?.session_id ??
      sessions[0]?.id ??
      state.activeSessionId;

    // Phase 9b: the active session's plan/todos hydrate the dock's spine
    // from the Rust snapshot (session-scoped state, not a turn artifact).
    const activeSession = bootstrap.chat.sessions.find(
      (session) => session.session_id === activeSessionId,
    );
    const hydratedTodos = todosFromSnapshot(activeSession?.todos);
    const hydratedPlanPhase: PlanPhase = activeSession?.plan_phase ?? 'executing';
    const hydratedPlan = validPlanApproval(activeSession?.plan)
      ? activeSession.plan
      : null;

    // Keep request routing aligned with the turn Rust says is live.
    activeRequestId = bootstrap.chat.active_request_id ?? null;

    // A pending approval survives the reload as a parked tool card so the
    // user can still decide it. Workflow summaries cannot authorize a run:
    // retain their identity, but wait for the full approval_needed payload.
    const previousConfirmations = new Map(workflowConfirmations);
    pendingApprovalRequests.clear();
    workflowConfirmations.clear();
    terminalRequests.clear();
    const pending = bootstrap.chat.pending_approval;
    if (pending) {
      pendingApprovalRequests.set(pending.approval_id, pending.request_id);
      const previousCall = state.messages.flatMap((message) => message.toolCalls ?? [])
        .find((call) => call.approvalId === pending.approval_id) as WorkflowToolCall | undefined;
      const previousConfirmation = previousConfirmations.get(pending.approval_id);
      const isWorkflow = /^run_workflow(?:\s|$)/.test(pending.summary) ||
        previousCall?.toolName === 'run_workflow' || Boolean(previousCall?.workflow) || Boolean(previousConfirmation);
      const isPlan = Boolean(pending.plan) || Boolean(previousCall?.plan) ||
        /^exit_plan_mode(?:\s|$)/.test(pending.summary) ||
        previousCall?.toolName === 'exit_plan_mode';
      if (isWorkflow) {
        const sameOrigin = previousConfirmation?.requestId === pending.request_id &&
          previousConfirmation.sessionId === pending.session_id &&
          previousConfirmation.context?.workspace_id === context?.workspace_id &&
          previousConfirmation.context?.core_instance_id === context?.core_instance_id;
        const confirmation: WorkflowConfirmation = sameOrigin ? previousConfirmation : {
          requestId: pending.request_id,
          sessionId: pending.session_id,
          context: context ? { ...context } : null,
          workflow: null,
          submitting: false,
          accepted: false,
        };
        confirmation.workflow = null;
        workflowConfirmations.set(pending.approval_id, confirmation);
      }
      const approvalCall: WorkflowToolCall = {
        id: `call_appr_${pending.approval_id}`,
        toolName: isPlan ? 'exit_plan_mode' : isWorkflow ? 'run_workflow' : 'operation',
        argsSummary: pending.summary,
        status: 'needs_approval',
        approvalId: pending.approval_id,
        plan: isPlan
          ? (pending.plan ?? previousCall?.plan ?? undefined)
          : undefined,
        resultSummary: isWorkflow
          ? 'Workflow confirmation is unavailable after refresh. Wait for the full approval_needed payload before deciding.'
          : undefined,
        startedAt: Date.now(),
      };
      const lastAssistantIndex = messages.reduce(
        (found, message, index) => (message.role === 'assistant' ? index : found),
        -1,
      );
      if (lastAssistantIndex >= 0) {
        const target = messages[lastAssistantIndex]!;
        messages[lastAssistantIndex] = {
          ...target,
          toolCalls: [...(target.toolCalls ?? []), approvalCall],
        };
      } else {
        messages.push({
          id: `msg_pending_${pending.approval_id}`,
          role: 'assistant',
          content: '',
          status: 'complete',
          timestamp: Date.now(),
          toolCalls: [approvalCall],
        });
      }
    }

    state = {
      ...state,
      availableModels: models,
      selectedModel: modelToSelect,
      workspace,
      sessions,
      resumableSessions,
      messages,
      activeSessionId,
      todos: hydratedTodos,
      planPhase: hydratedPlanPhase,
      plan: hydratedPlan,
      inFlight: Boolean(bootstrap.chat.active_request_id),
    };
    notify();
  },

  setAvailableModels(models: Model[]) {
    // Reconcile against the backend catalog only. An empty catalog clears the
    // selection instead of keeping a stale (fabricated) model id.
    let modelToSelect = '';
    if (models.length > 0) {
      const hasCurrent = models.some(
        (m) => m.id === state.selectedModel || m.display_name === state.selectedModel,
      );
      const firstAvail = models.find((m) => m.available) ?? models[0];
      modelToSelect = hasCurrent ? state.selectedModel : firstAvail?.id ?? '';
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
    pendingApprovalRequests.clear();
    workflowConfirmations.clear();
    terminalRequests.clear();
    completedSubagentIds.clear();
    state = {
      ...state,
      messages: [],
      todos: [],
      planPhase: 'executing',
      plan: null,
      subagents: INITIAL_SUBAGENTS,
      unreadSubagentCount: 0,
      revealedSubagentId: null,
      inFlight: false,
      thinkingState: 'breathing',
      streamingActivityLevel: 0,
      promptQueue: [],
      availableModels: [],
      selectedModel: '',
      workspace: null,
      resumableSessions: [],
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
      planPhase: 'executing',
      plan: null,
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

  /**
   * Phase 9e: one accepted background completion. Returns false when this
   * child id was already handled (dedupe) or when the payload claims success
   * without a result. A failed child stays failed: it is never painted as a
   * completed result.
   */
  handleSubagentCompleted(payload: SubagentCompletedPayload): boolean {
    if (completedSubagentIds.has(payload.child_id)) return false;
    completedSubagentIds.add(payload.child_id);

    const existing = state.subagents.find((subagent) => subagent.id === payload.child_id);
    const ok = payload.ok === true;
    const entry: SubagentInfo = {
      id: payload.child_id,
      name: existing?.name ?? `subagent-${payload.child_id}`,
      model: existing?.model ?? 'Auto',
      status: ok ? 'done' : 'failed',
      parentSessionId: payload.parent_session_id,
      policy: payload.policy,
      result: ok ? payload.final_message : '',
      error: ok ? undefined : payload.error ?? 'subagent failed without a message',
      artifacts: payload.artifacts ?? [],
      unread: true,
      activeStep: ok
        ? `Completed: ${payload.final_message || 'no output'}`
        : `Failed: ${payload.error ?? 'unknown error'}`,
    };
    const subagents = existing
      ? state.subagents.map((subagent) =>
          subagent.id === payload.child_id ? { ...subagent, ...entry } : subagent,
        )
      : [...state.subagents, entry];
    state = {
      ...state,
      subagents,
      unreadSubagentCount: countUnreadSubagents(subagents),
    };
    notify();
    return true;
  },

  /**
   * A notification click: select the parent session and reveal the child's
   * result under it. Marking read keeps the badge truthful.
   */
  revealSubagent(sessionId: string, childId: string) {
    const subagents = state.subagents.map((subagent) =>
      subagent.id === childId ? { ...subagent, unread: false } : subagent,
    );
    state = {
      ...state,
      activeSessionId: sessionId || state.activeSessionId,
      subagents,
      unreadSubagentCount: countUnreadSubagents(subagents),
      revealedSubagentId: childId,
      messages: sessionId && sessionId !== state.activeSessionId ? [] : state.messages,
    };
    notify();
  },

  /** Mark one child's result read without changing the selected session. */
  markSubagentRead(childId: string) {
    const subagents = state.subagents.map((subagent) =>
      subagent.id === childId ? { ...subagent, unread: false } : subagent,
    );
    state = {
      ...state,
      subagents,
      unreadSubagentCount: countUnreadSubagents(subagents),
    };
    notify();
  },

  clearSubagentReveal() {
    if (state.revealedSubagentId === null) return;
    state = { ...state, revealedSubagentId: null };
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

  /** Phase 9d — open a durable session restored by Rust. */
  resumeSession(sessionId: string): boolean {
    if (!state.resumableSessions.some((session) => session.id === sessionId)) return false;
    this.switchSession(sessionId);
    const ctx = sessionEnvelopeContext();
    if (ctx) {
      void bridgeSwitchSession(sessionId, ctx).catch((err: unknown) => {
        console.error('Failed to restore agent session:', err);
      });
    }
    return true;
  },

  /** Phase 9d — new session (Rust owns the id and the empty on-disk store). */
  newDurableSession(title = 'New Agent Session'): string {
    const localId = this.createSession(title);
    const ctx = sessionEnvelopeContext();
    if (ctx) {
      void bridgeNewSession(ctx).catch((err: unknown) => {
        console.error('Failed to create agent session:', err);
      });
    }
    return localId;
  },

  /** Phase 9d — refresh the resume picker from disk. */
  refreshResumableSessions(): void {
    const ctx = sessionEnvelopeContext();
    if (ctx) {
      void listAgentSessions(ctx).catch((err: unknown) => {
        console.error('Failed to list agent sessions:', err);
      });
    }
  },

  /**
   * Phase 9d — arm/disarm session full access. The flag is Rust session
   * memory only: it never reaches disk and resets on switch/new/restart.
   */
  setSessionAutoApprove(enabled: boolean): void {
    const ctx = sessionEnvelopeContext();
    if (ctx) {
      void setAgentAutoApprove(state.activeSessionId, enabled, ctx).catch((err: unknown) => {
        console.error('Failed to set session auto-approve:', err);
      });
    }
  },

  /** Phase 9d — retry a user message (Rust rewinds the durable transcript). */
  retryFromMessage(messageId: string): boolean {
    const ctx = sessionEnvelopeContext();
    const index = state.messages.findIndex((message) => message.id === messageId);
    if (!ctx || index < 0 || state.messages[index]?.role !== 'user') return false;
    state = {
      ...state,
      messages: state.messages.slice(0, index),
      inFlight: false,
      thinkingState: 'breathing',
      streamingActivityLevel: 0,
    };
    notify();
    void bridgeRetryMessage(state.activeSessionId, messageId, ctx).catch((err: unknown) => {
      console.error('Failed to retry message:', err);
    });
    return true;
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

  sendUserMessage(text: string, isSteer = false, editorContext?: EditorContextDto): boolean {
    const trimmed = text.trim();
    if (!trimmed) return false;

    // Refuse to start a turn without a real workspace and a real catalog
    // model. The composer surfaces the same message and disables send.
    const gate = getComposerGate(state);
    const sessionContext = sessionEnvelopeContext();
    if (!gate.ready || !sessionContext) return false;

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

    const mode = toAgentMode(state.permissionMode);
    const modelToSend = resolveModelId(state);
    if (!modelToSend) return false;

    const { requestId, promise } = sendAgent(
      promptToSend,
      mode,
      modelToSend,
      sessionContext,
      {
        reasoningEffort: state.reasoningEffort,
        systemInstructions: state.modelSettings.systemInstructions,
        temperature: state.modelSettings.temperature,
        ...(editorContext ? { editorContext } : {}),
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
    return true;
  },

  async steerActiveTurn(text: string): Promise<void> {
    const trimmed = text.trim();
    if (!trimmed) return;
    if (state.inFlight) {
      const sessionContext = sessionEnvelopeContext();
      if (sessionContext && activeRequestId) {
        const stoppedRequestId = activeRequestId;
        try {
          // The steer must not race the stop: wait for the old turn to be
          // told to stop before starting the replacement turn.
          await stopChat(stoppedRequestId, sessionContext);
        } catch {
          // Tolerate stop failures (already finished/unknown request) and
          // still deliver the steering directive.
        }
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

    // Todos are owned by the session store (`todos_changed`), never
    // derived from assistant prose.
    state = {
      ...state,
      messages: newMessages,
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

  handleApprovalDecision(
    approvalId: string,
    decision: 'approve' | 'deny' | 'approve_all' | 'revise',
  ): void | Promise<void> {
    const target = state.messages.flatMap((message) => message.toolCalls ?? [])
      .find((call) => call.approvalId === approvalId) as WorkflowToolCall | undefined;
    const planCard = Boolean(target?.plan) || target?.toolName === 'exit_plan_mode';
    // A plan card only offers Approve/Revise: a blanket approval is a
    // different grant and must never skip the plan decision.
    if (planCard && decision === 'approve_all') return;
    if (!planCard && (target?.toolName === 'run_workflow' || target?.workflow || workflowConfirmations.has(approvalId))) {
      if (decision === 'revise') return;
      return submitWorkflowConfirmation(approvalId, decision);
    }
    // Route to the request that raised this approval, not whatever request
    // happens to be newest globally. Without a resolvable request the card
    // stays parked (flipping it would lie about a decision never sent).
    const requestId = pendingApprovalRequests.get(approvalId) ?? activeRequestId;
    const sessionContext = requestId ? sessionEnvelopeContext() : null;
    if (!requestId || !sessionContext) return;

    const mappedDecision = decision === 'approve_all' ? 'approve_all_this_turn' : decision;

    // Phase 8: the agent's `run_workflow` card is the confirmation moment.
    // The workflow-specific route above waits for the exact script/command set
    // to be accepted before telling the agent turn that the run is starting.
    void agentDecision(requestId, approvalId, mappedDecision, sessionContext).catch(() => {});
    pendingApprovalRequests.delete(approvalId);

    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex < 0) return;
    const lastMsg = state.messages[lastMsgIndex];
    if (!lastMsg || !lastMsg.toolCalls) return;

    const updatedCalls = lastMsg.toolCalls.map((c) => {
      const isPlan = c.approvalId === approvalId && (Boolean((c as WorkflowToolCall).plan) || c.toolName === 'exit_plan_mode');
      if (c.approvalId === approvalId || (decision === 'approve_all' && c.toolName !== 'run_workflow' && !(c as WorkflowToolCall).workflow && !(c as WorkflowToolCall).plan)) {
        return {
          ...c,
          status: (decision === 'deny' || decision === 'revise' ? 'denied' : 'approved') as AgentToolCall['status'],
          resultSummary: isPlan && decision === 'revise'
            ? 'Revision requested — the session stays in plan mode.'
            : c.resultSummary,
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
      thinkingState: decision === 'deny' || decision === 'revise' ? 'breathing' : 'solving',
    };
    notify();
  },

  completeActiveTurn(metricsUpdate?: Partial<AgentMetrics>) {
    if (activeRequestId) terminalRequests.add(activeRequestId);
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
    const sessionContext = sessionEnvelopeContext();
    if (sessionContext && activeRequestId) {
      void stopChat(activeRequestId, sessionContext).catch(() => {});
    }
    this.completeActiveTurn();
  },

  handleChatDelta(payload: { request_id: string; session_id?: string; text: string }) {
    if (!isActiveEvent(payload)) return;
    this.appendAssistantDelta(payload.text);
  },

  handleChatCompleted(payload: ChatCompletedPayload) {
    if (!isActiveEvent(payload)) return;
    this.completeActiveTurn({
      totalTokens: payload.total_tokens,
      costUsd: payload.cost_usd,
    });
  },

  handleChatFailed(payload: { request_id?: string; session_id?: string; error: { message: string } }) {
    if (!isActiveEvent(payload)) return;
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
    if (!isActiveEvent(payload)) return;
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

    // `run_workflow` is the one tool whose card is a grant: parse the §18.1
    // payload from the same arguments the approval later repeats, and label
    // the header with the workflow name instead of the whole JSON blob.
    const workflow =
      payload.tool_name === 'run_workflow'
        ? workflowApprovalFromArgs(parsedArgs)
        : undefined;
    // Phase 9b: an `exit_plan_mode` call carries its steps before the card.
    const plan =
      payload.tool_name === 'exit_plan_mode'
        ? planApprovalFromArgs(parsedArgs)
        : undefined;

    // Phase 9b: todos are session state owned by Rust. The store is updated
    // only by the authoritative `todos_changed` event — never inferred from
    // a tool's arguments and never advanced by another tool's start.
    const updatedTodos = state.todos;

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

    const toolCall: WorkflowToolCall = {
      id: payload.call_id,
      toolName: payload.tool_name,
      argsSummary: workflow?.name
        ? workflow.name
        : plan?.summary || (plan ? `${plan.steps.length}-step plan` : argsSummary),
      argsRaw: parsedArgs,
      status: 'running',
      // A workflow grant renders from this payload even before its approval.
      workflow,
      plan,
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
    if (!isActiveEvent(payload)) return;
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
    if (!isActiveEvent(payload)) return;
    if (payload.tool_name === 'run_workflow' && !state.inFlight && activeRequestId === payload.request_id) return;
    const existingConfirmation = workflowConfirmations.get(payload.approval_id);
    if (existingConfirmation && (
      existingConfirmation.workflow || existingConfirmation.requestId !== payload.request_id ||
      existingConfirmation.sessionId !== payload.session_id || payload.tool_name !== 'run_workflow'
    )) return;
    if (payload.tool_name === 'run_workflow' && validWorkflowApproval(payload.workflow)) {
      const confirmation = existingConfirmation ?? {
        requestId: payload.request_id,
        sessionId: payload.session_id,
        context: context ? { ...context } : null,
        workflow: null,
        submitting: false,
        accepted: false,
      };
      confirmation.workflow = { ...payload.workflow, commands: [...payload.workflow.commands] };
      workflowConfirmations.set(payload.approval_id, confirmation);
    }
    // Remember which request owns this approval so the decision targets it.
    if (payload.request_id) {
      pendingApprovalRequests.set(payload.approval_id, payload.request_id);
    }
    const lastMsgIndex = state.messages.length - 1;
    if (lastMsgIndex < 0) return;
    const lastMsg = state.messages[lastMsgIndex];
    if (!lastMsg) return;

    // A workflow card is a grant, not a diff: the payload rides `workflow`
    // and its script is never counted as a diff hunk.
    const workflow = payload.workflow;
    const plan = payload.plan;
    const diffHunks =
      !workflow && !plan && payload.diff_or_input ? [payload.diff_or_input] : undefined;
    const approvalCall: WorkflowToolCall = {
      id: `call_appr_${payload.approval_id}`,
      toolName: payload.tool_name,
      argsSummary: plan?.summary || payload.path_or_command,
      status: 'needs_approval',
      approvalId: payload.approval_id,
      workflow,
      plan,
      resultSummary: undefined,
      diff: diffHunks
        ? {
            added: 1,
            removed: 0,
            hunks: diffHunks,
          }
        : undefined,
      startedAt: Date.now(),
    };

    const existingCalls = lastMsg.toolCalls ?? [];
    // The `run_workflow` started card and its approval are one grant: upgrade
    // the running card in place (keeping its id, so the turn's later
    // `tool_result` lands on it) instead of appending a second card. Every
    // other tool keeps the append-only behavior.
    let upgradeIndex = -1;
    if (payload.tool_name === 'run_workflow') {
      for (let index = existingCalls.length - 1; index >= 0; index -= 1) {
        const call = existingCalls[index]!;
        if (call.id === approvalCall.id) {
          upgradeIndex = index;
          break;
        }
        if (call.toolName === 'run_workflow' && !call.approvalId) {
          upgradeIndex = index;
          break;
        }
      }
    }

    const toolCalls =
      upgradeIndex >= 0
        ? existingCalls.map((call, index) =>
            index === upgradeIndex
              ? {
                  ...call,
                  ...approvalCall,
                  id: call.id,
                  startedAt: call.startedAt,
                }
              : call,
          )
        : [...existingCalls, approvalCall];

    const updated = {
      ...lastMsg,
      toolCalls,
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

  handlePlanStateChanged(payload: PlanStateChangedPayload) {
    if (payload.session_id && state.activeSessionId && payload.session_id !== state.activeSessionId) {
      return;
    }
    const plan = validPlanApproval(payload.plan) ? payload.plan : null;
    state = {
      ...state,
      planPhase: payload.phase ?? 'executing',
      plan,
    };
    notify();
  },

  handleTodosChanged(payload: TodosChangedPayload) {
    if (payload.session_id && state.activeSessionId && payload.session_id !== state.activeSessionId) {
      return;
    }
    state = { ...state, todos: todosFromSnapshot(payload.todos) };
    notify();
  },

  handleAgentStateChanged(payload: AgentStateChangedPayload) {
    if (!isActiveEvent(payload)) return;
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
    if (!isActiveEvent(payload)) return;
    terminalRequests.add(payload.request_id);
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

    const idleSubagents = state.subagents.map((s) =>
      s.status === 'done' || s.status === 'failed'
        ? s
        : {
            ...s,
            status: 'idle' as const,
            activeStep: 'Ready',
          },
    );

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
    if (!isActiveEvent(payload)) return;
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
