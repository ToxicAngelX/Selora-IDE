// Selora IDE — Phase Z3 Agent Store
//
// Manages agent sessions, messages, tool executions, approval gates,
// backend-authoritative todos, and hero composer interactions.
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
  SubagentLiveState,
  SubagentStatus,
  ThinkingState,
  TodoStatus,
  AgentMetrics,
} from './types';
import {
  agentDecision,
  fetchModels,
  newSession as requestNewSessionCommand,
  deleteSession as deleteSessionCommand,
  listAgentSessions,
  retryMessage as bridgeRetryMessage,
  runWorkflow,
  sendAgent,
  sendChat,
  setAgentAutoApprove,
  stopChat,
  switchSession as switchSessionCommand,
  type AgentDecisionResolution,
  type AgentMode,
  type AgentStateChangedPayload,
  type AgentTurnDonePayload,
  type Approval,
  type ApprovalNeededPayload,
  type BootstrapState,
  type ChatCompletedPayload,
  type ChatTodoDto,
  type EditorContextDto,
  type EnvelopeContext,
  type Model,
  type PlanApproval,
  type PlanPhase,
  type PlanStateChangedPayload,
  type SubagentActivityPayload,
  type SubagentCompletedPayload,
  type TodoItem,
  type TodosChangedPayload,
  type ToolCallStartedPayload,
  type ToolResultPayload,
  type TodoDto,
  type WorkflowApproval,
} from '../../bridge/bridge';
import { toastStore } from '../toasts/toast-store';

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
/** Authoritative `manage_todo` list for the active session (backend-owned). */
  /**
   * The active session's transcript / todos. The maps below keep every
   * backend-known session's copy so a tab switch never invents a blank one.
   */
  transcripts: Record<string, ChatMessage[]>;
  todosBySession: Record<string, AgentTodo[]>;
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
  /** Child transcript open in the dock; valid only under activeSessionId. */
  activeSubagentId: string | null;
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
  /** A NewSession / SwitchSession command is in flight. */
  sessionBusy: boolean;
  /** Truthful session-command failure (unsupported slice, transport, ...). */
  sessionError: string | null;
};

const STORED_SETTINGS_KEY = 'selora:model_settings';

const INITIAL_SESSIONS: AgentSession[] = [];
/**
 * Session-scoped permission default (Phase 9): auto-approve is never a
 * persisted trust grant. A restart, a reset, or a new session starts here.
 */
const DEFAULT_PERMISSION_MODE: PermissionMode = 'ask_before_change';

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

const INITIAL_TODOS: AgentTodo[] = [];

let state: AgentState = {
  sessions: INITIAL_SESSIONS,
  activeSessionId: '',
  resumableSessions: [],
  messages: [],
  transcripts: {},
  todosBySession: {},
  todos: INITIAL_TODOS,
  planPhase: 'executing',
  plan: null,
  subagents: [],
  unreadSubagentCount: 0,
  revealedSubagentId: null,
  activeSubagentId: null,
  artifacts: [],
  inFlight: false,
  thinkingState: 'breathing',
  permissionMode: DEFAULT_PERMISSION_MODE,
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
  sessionBusy: false,
  sessionError: null,
};

/** Protocol todo → the plan-spine row shape. Text is the task; no invented fields. */
function todoFromDto(todo: TodoDto): AgentTodo {
  return { id: todo.id, text: todo.text ?? todo.title ?? '', status: todo.status };
}

/**
 * A `manage_todo` payload may also arrive as a markdown checklist. Read it
 * without inventing rows; `null` means "no checklist in this text".
 */
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
        text: title,
        status,
      });
    }
  }
  return todos.length > 0 ? todos : null;
}

/**
 * Parse a `manage_todo` argument payload (array, object list, JSON string or
 * checklist) into plan rows. Malformed entries are dropped, never guessed.
 */
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
            text: trimmed,
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
          text: title,
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

const PENDING_APPROVAL_PREFIX = 'msg_pending_';

/**
 * Map the backend's authoritative session todos (ChatStateDto.todos once 9b
 * lands) without inventing titles. A malformed status falls back to `pending`
 * because that is the least-claiming state, never `completed`.
 */
export function parseSessionTodos(todos: ChatTodoDto[] | undefined): AgentTodo[] {
  if (!todos || todos.length === 0) return [];
  return todos
    .map((todo) => ({
      id: todo.id,
      // `text` is the authoritative Phase 9b field; `title` is the 9a spelling
      // older payloads and local plans carry.
      text: (todo.text ?? todo.title ?? '').trim(),
      status: todo.status,
      ...(todo.assigned_subagent ? { assignedSubagent: todo.assigned_subagent } : {}),
    }))
    .filter((todo) => todo.text !== '')
    .map((todo) => ({
      ...todo,
      status:
        todo.status === 'completed'
          ? 'completed'
          : todo.status === 'in_progress'
            ? 'in_progress'
            : 'pending',
    }));
}

function updateToolCallCard(
  approvalId: string,
  update: Partial<AgentToolCall>,
): void {
  const apply = (messages: ChatMessage[]) => messages.map((message) => ({
    ...message,
    toolCalls: message.toolCalls?.map((call) =>
      call.approvalId === approvalId ? { ...call, ...update } : call,
    ),
  }));
  // A child approval is mirrored in both transcripts: the parent's awareness
  // card and the child's own card share one decision/status.
  state = {
    ...state,
    messages: apply(state.messages),
    subagents: state.subagents.map((child) =>
      child.messages?.some((message) =>
        message.toolCalls?.some((call) => call.approvalId === approvalId),
      )
        ? { ...child, messages: apply(child.messages) }
        : child,
    ),
  };
  notify();
}

function decisionErrorMessage(error: unknown): string {
  const message =
    error && typeof error === 'object' && 'message' in error
      ? String((error as { message: unknown }).message)
      : typeof error === 'string'
        ? error
        : 'The decision could not be sent. Try again.';
  return message || 'The decision could not be sent. Try again.';
}

function sessionCommandError(
  action: 'create' | 'switch' | 'delete' | 'retry',
  error: unknown,
): string {
  const code =
    error && typeof error === 'object' && 'code' in error
      ? String((error as { code: unknown }).code)
      : '';
  if (code === 'unsupported_phase_slice') {
    return action === 'create'
      ? 'Creating agent sessions is not supported by this backend build yet.'
      : action === 'switch'
        ? 'Switching agent sessions is not supported by this backend build yet.'
        : action === 'delete'
          ? 'Deleting agent sessions is not supported by this backend build yet.'
          : 'Retrying a turn is not supported by this backend build yet.';
  }
  const prefix =
    action === 'create'
      ? 'Could not create an agent session: '
      : action === 'switch'
        ? 'Could not switch agent sessions: '
        : action === 'delete'
          ? 'Could not delete the agent session: '
          : 'Could not retry the turn: ';
  return `${prefix}${decisionErrorMessage(error)}`;
}

/**
 * Phase 9e: the dedupe ledger. A child id reaches this store once, ever;
 * a replayed protocol event or a duplicate completion is ignored rather
 * than producing a second toast/badge/result.
 */
const completedSubagentIds = new Set<string>();

function countUnreadSubagents(subagents: SubagentInfo[]): number {
  return subagents.reduce((count, subagent) => (subagent.unread ? count + 1 : count), 0);
}

const SUBAGENT_NAME_MAX = 48;

/**
 * A child's display name comes from the backend's `started.name`, else the
 * first non-empty mission line collapsed to one line, else a stable id suffix.
 */
export function subagentDisplayName(
  name: string | null | undefined,
  mission: string | null | undefined,
  childId: string,
): string {
  const explicit = name?.trim();
  if (explicit) return explicit;
  const firstLine = mission
    ?.split(/\r?\n/)
    .map((line) => line.replace(/\s+/g, ' ').trim())
    .find((line) => line.length > 0);
  if (firstLine) {
    return firstLine.length > SUBAGENT_NAME_MAX
      ? `${firstLine.slice(0, SUBAGENT_NAME_MAX - 1).trimEnd()}…`
      : firstLine;
  }
  return `Subagent ${childId.slice(-8)}`;
}

/** A completion must keep a started name, not downgrade to `<id>`. */
function meaningfulSubagentName(child: SubagentInfo | undefined): string | null {
  if (!child) return null;
  const name = child.name?.trim();
  if (!name || name === child.id || name === `subagent-${child.id}`) return null;
  return name;
}

function isTerminalChildStatus(status: SubagentStatus): boolean {
  return status === 'done' || status === 'failed' || status === 'cancelled';
}

function finalizeChildMessages(
  messages: ChatMessage[] | undefined,
  status: 'complete' | 'error',
): ChatMessage[] | undefined {
  return messages?.map((message) =>
    message.role === 'assistant' && message.status === 'streaming'
      ? { ...message, status }
      : message,
  );
}

function subagentFallback(childId: string, parentSessionId: string): SubagentInfo {
  return {
    id: childId,
    name: subagentDisplayName(undefined, undefined, childId),
    model: 'Auto',
    status: 'working',
    parentSessionId,
    startedAt: Date.now(),
  };
}

/** Child activity is kept in its own ledger; the parent transcript is untouched. */
function updateChildSubagent(
  childId: string,
  parentSessionId: string,
  update: (child: SubagentInfo) => SubagentInfo,
): void {
  const knownParent = parentSessionId === state.activeSessionId ||
    state.sessions.some((session) => session.id === parentSessionId);
  if (!parentSessionId || !knownParent) return;
  const existing = state.subagents.find((child) => child.id === childId);
  const base = existing ?? subagentFallback(childId, parentSessionId);
  const updated = update({ ...base, parentSessionId });
  const subagents = existing
    ? state.subagents.map((child) => (child.id === childId ? updated : child))
    : [...state.subagents, updated];
  state = { ...state, subagents, unreadSubagentCount: countUnreadSubagents(subagents) };
}

function activeChildStillBelongs(
  childId: string | null,
  activeSessionId: string,
  subagents: SubagentInfo[],
): string | null {
  if (!childId) return null;
  const child = subagents.find((entry) => entry.id === childId);
  return child && child.parentSessionId === activeSessionId ? childId : null;
}

/** Derive the same compact tool target the parent transcript shows. */
function summarizeToolArgs(argsJson: string, toolName: string): {
  argsSummary: string;
  argsRaw?: Record<string, unknown>;
} {
  let argsRaw: Record<string, unknown> | undefined;
  try {
    const parsed: unknown = JSON.parse(argsJson);
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      argsRaw = parsed as Record<string, unknown>;
      if ('path' in argsRaw) return { argsSummary: String(argsRaw.path), argsRaw };
      if ('command' in argsRaw) return { argsSummary: String(argsRaw.command), argsRaw };
      if ('query' in argsRaw) return { argsSummary: String(argsRaw.query), argsRaw };
      return { argsSummary: JSON.stringify(argsRaw), argsRaw };
    }
  } catch {
    // Not JSON: the raw argument text is the honest summary.
  }
  return { argsSummary: argsJson || toolName, ...(argsRaw ? { argsRaw } : {}) };
}

type ChildApprovalTarget = { childId: string; toolName: string };

/** HEAD's child approval carries `subagent_id`; older payloads tag the tool. */
function childApprovalTarget(payload: ApprovalNeededPayload): ChildApprovalTarget | null {
  if (payload.subagent_id?.trim()) {
    return {
      childId: payload.subagent_id.trim(),
      toolName: payload.tool_name.replace(/^Subagent\s*\[[^\]]+\]\s*:\s*/i, '').trim() || payload.tool_name,
    };
  }
  const match = payload.tool_name.match(/^Subagent\s*\[([^\]]+)\]\s*:\s*(.+)$/i);
  if (!match || !match[1]?.trim()) return null;
  return { childId: match[1].trim(), toolName: match[2]!.trim() || payload.tool_name };
}

/** Append to the child's last assistant message, or open one for this activity. */
function withChildAssistant(
  child: SubagentInfo,
  childId: string,
  update: (message: ChatMessage) => ChatMessage,
): ChatMessage[] {
  const messages = [...(child.messages ?? [])];
  const lastIndex = messages.length - 1;
  if (lastIndex >= 0 && messages[lastIndex]?.role === 'assistant') {
    messages[lastIndex] = update(messages[lastIndex]!);
    return messages;
  }
  messages.push(update({
    id: `subagent-${childId}-assistant`,
    role: 'assistant',
    content: '',
    status: 'streaming',
    timestamp: Date.now(),
    toolCalls: [],
  }));
  return messages;
}

/**
 * A child approval follows the child's `tool_call_started`. Upgrade that live
 * running card in place so its `call_id` still receives the later result; a
 * missing live card becomes a new approval card. The parent's awareness card
 * is appended by `handleApprovalNeeded` independently.
 */
function attachChildApprovalCard(
  payload: ApprovalNeededPayload,
  target: ChildApprovalTarget,
): void {
  childApprovalOwners.set(payload.approval_id, {
    childId: target.childId,
    parentSessionId: payload.session_id,
  });
  const approvalCall: WorkflowToolCall = {
    id: `call_appr_${payload.approval_id}`,
    toolName: target.toolName,
    argsSummary: payload.plan?.summary || payload.path_or_command,
    status: 'needs_approval',
    approvalId: payload.approval_id,
    backgroundChild: true,
    workflow: payload.workflow,
    decisionPrompt: payload.prompt,
    plan: payload.plan,
    diff: !payload.workflow && !payload.plan && payload.diff_or_input
      ? { added: 1, removed: 0, hunks: [payload.diff_or_input] }
      : undefined,
    startedAt: Date.now(),
  };
  updateChildSubagent(target.childId, payload.session_id, (child) => {
    const messages = [...(child.messages ?? [])];
    // A replayed approval updates the existing card, never duplicates it.
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index]!;
      if (!message.toolCalls?.some((call) => call.approvalId === payload.approval_id)) continue;
      messages[index] = {
        ...message,
        toolCalls: message.toolCalls.map((call) =>
          call.approvalId === payload.approval_id ? { ...call, ...approvalCall, id: call.id } : call,
        ),
      };
      return {
        ...child,
        messages,
        status: 'working',
        state: 'awaiting_approval',
        activeTool: target.toolName,
        activeStep: `Awaiting approval: ${target.toolName}`,
      };
    }
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index]!;
      if (message.role !== 'assistant') continue;
      const calls = message.toolCalls ?? [];
      const runningIndex = calls.reduce(
        (found, call, callIndex) =>
          call.toolName === target.toolName && call.status === 'running' && !call.approvalId
            ? callIndex
            : found,
        -1,
      );
      if (runningIndex < 0) continue;
      const running = calls[runningIndex]!;
      messages[index] = {
        ...message,
        toolCalls: calls.map((call, callIndex) =>
          callIndex === runningIndex
            ? { ...running, ...approvalCall, id: running.id, startedAt: running.startedAt }
            : call,
        ),
      };
      return {
        ...child,
        messages,
        status: 'working',
        state: 'awaiting_approval',
        activeTool: target.toolName,
        activeStep: `Awaiting approval: ${target.toolName}`,
      };
    }
    const lastIndex = messages.length - 1;
    if (lastIndex >= 0 && messages[lastIndex]?.role === 'assistant') {
      const message = messages[lastIndex]!;
      messages[lastIndex] = {
        ...message,
        toolCalls: [...(message.toolCalls ?? []), approvalCall],
      };
    } else {
      messages.push({
        id: `subagent-${target.childId}-approval`,
        role: 'assistant',
        content: '',
        status: 'streaming',
        timestamp: Date.now(),
        toolCalls: [approvalCall],
      });
    }
    return {
      ...child,
      messages,
      status: 'working',
      state: 'awaiting_approval',
      activeTool: target.toolName,
      activeStep: `Awaiting approval: ${target.toolName}`,
    };
  });
  notify();
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
export function todosFromSnapshot(items: Array<TodoItem | TodoDto> | undefined): AgentTodo[] {
  if (!items) return [];
  return items
    .flatMap((item) => {
      const title = item.title ?? ('text' in item ? item.text : undefined) ?? '';
      if (!title.trim()) return [];
      return [{
        id: item.id || `todo_${title}`,
        text: title,
        status:
          item.status === 'completed'
            ? 'completed' as const
            : item.status === 'in_progress'
              ? 'in_progress' as const
              : 'pending' as const,
      }];
    });
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
type SessionView = Pick<AgentState, 'messages' | 'todos' | 'plan' | 'planPhase' | 'inFlight' |
  'thinkingState' | 'metrics' | 'promptQueue' | 'streamingActivityLevel'> & { requestId: string | null };
const sessionViews = new Map<string, SessionView>();
let routingSessionEvent = false;

function selectSessionView(sessionId: string, clearChildSelection = false) {
  const view = sessionViews.get(sessionId);
  activeRequestId = view?.requestId ?? null;
  state = {
    ...state,
    activeSessionId: sessionId,
    activeSubagentId: clearChildSelection
      ? activeChildStillBelongs(state.activeSubagentId, sessionId, state.subagents)
      : state.activeSubagentId,
    messages: view?.messages ?? state.transcripts[sessionId] ?? [],
    todos: view?.todos ?? state.todosBySession[sessionId] ?? [],
    plan: view?.plan ?? null,
    planPhase: view?.planPhase ?? 'executing',
    inFlight: view?.inFlight ?? false,
    thinkingState: view?.thinkingState ?? 'breathing',
    metrics: view?.metrics ?? { totalTokens: 0, costUsd: 0, activeToolsCount: 0, approvedCount: 0, deniedCount: 0, latencyMs: 0 },
    promptQueue: view?.promptQueue ?? [],
    streamingActivityLevel: view?.streamingActivityLevel ?? 0,
    permissionMode: DEFAULT_PERMISSION_MODE,
  };
}

/**
 * The request that raised each parked approval, keyed by `approval_id`.
 * `AgentDecision` must target *that* request — the global `activeRequestId`
 * may already belong to a newer turn (or be null after a reload), which
 * would silently lose the decision.
 */
const pendingApprovalRequests = new Map<string, string>();
/** Which child (and its parent session) owns a parked child approval card. */
const childApprovalOwners = new Map<string, { childId: string; parentSessionId: string }>();

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

/**
 * A finished child can no longer answer its parked cards. Close the child
 * transcript's cards (and the parent awareness copies) truthfully.
 */
function closeChildApprovalCards(childId: string, sessionId: string, reason: string): void {
  terminalRequests.add(childId);
  const approvals = new Set([...pendingApprovalRequests]
    .filter(([, request]) => request === childId).map(([approval]) => approval));
  for (const [approval, owner] of childApprovalOwners) {
    if (owner.childId === childId) approvals.add(approval);
  }
  for (const approval of approvals) {
    pendingApprovalRequests.delete(approval);
    childApprovalOwners.delete(approval);
  }
  if (approvals.size === 0) return;
  const update = (messages: ChatMessage[]) => messages.map((message) => ({
    ...message,
    toolCalls: message.toolCalls?.map((call) =>
      call.approvalId && approvals.has(call.approvalId) && call.status === 'needs_approval'
        ? { ...call, status: 'denied' as const, decisionSubmitting: false, resultSummary: reason }
        : call),
  }));
  const view = sessionViews.get(sessionId);
  if (view) sessionViews.set(sessionId, { ...view, messages: update(view.messages) });
  const subagents = state.subagents.map((child) => {
    if (child.id !== childId || !child.messages?.some((message) =>
      message.toolCalls?.some((call) => call.approvalId && approvals.has(call.approvalId)),
    )) return child;
    return { ...child, messages: update(child.messages), activeTool: undefined };
  });
  state = {
    ...state,
    subagents,
    messages: state.activeSessionId === sessionId ? update(state.messages) : state.messages,
    transcripts: Object.prototype.hasOwnProperty.call(state.transcripts, sessionId)
      ? { ...state.transcripts, [sessionId]: update(state.transcripts[sessionId] ?? []) }
      : state.transcripts,
  };
}

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
  if (state.activeSessionId) {
    sessionViews.set(state.activeSessionId, {
      messages: state.messages, todos: state.todos, plan: state.plan, planPhase: state.planPhase,
      inFlight: state.inFlight, thinkingState: state.thinkingState, metrics: state.metrics,
      promptQueue: state.promptQueue, streamingActivityLevel: state.streamingActivityLevel,
      requestId: activeRequestId,
    });
    state = {
      ...state,
      transcripts: { ...state.transcripts, [state.activeSessionId]: state.messages },
      todosBySession: { ...state.todosBySession, [state.activeSessionId]: state.todos },
    };
  }
  if (routingSessionEvent) return;
  for (const listener of listeners) {
    listener();
  }
}

function findApprovalCard(approvalId: string): AgentToolCall | undefined {
  const parent = state.messages
    .flatMap((message) => message.toolCalls ?? [])
    .find((call) => call.approvalId === approvalId);
  if (parent) return parent;
  return state.subagents
    .flatMap((child) => child.messages ?? [])
    .flatMap((message) => message.toolCalls ?? [])
    .find((call) => call.approvalId === approvalId);
}

/**
 * Phase 9a question/plan decisions ride the existing `AgentDecision` command.
 * The optional `resolution` is additive: a pre-9b Rust build ignores the extra
 * field and still records the permit/deny, so the turn never wedges. The card
 * only flips once the command resolves; a transport failure leaves it parked
 * with the real message visible.
 */
async function submitPromptDecision(
  approvalId: string,
  decision: 'approve' | 'answer' | 'deny' | 'revise',
  resolution: AgentDecisionResolution,
): Promise<boolean> {
  const card = findApprovalCard(approvalId);
  if ((!card?.decisionPrompt && !(card as WorkflowToolCall | undefined)?.plan) || card?.status !== 'needs_approval') return false;
  if (card.decisionSubmitting) return false;
  const requestId = pendingApprovalRequests.get(approvalId) ?? activeRequestId;
  const ctx = requestId ? sessionEnvelopeContext() : null;
  if (!requestId || !ctx) {
    updateToolCallCard(approvalId, {
      decisionError: 'The request that asked this question is no longer live.',
    });
    return false;
  }
  updateToolCallCard(approvalId, {
    decisionSubmitting: true,
    decisionError: undefined,
  });
  try {
    await agentDecision(requestId, approvalId, decision, ctx, resolution);
    pendingApprovalRequests.delete(approvalId);
    updateToolCallCard(approvalId, {
      status: decision === 'approve' || decision === 'answer' ? 'approved' : 'denied',
      resolution,
      decisionSubmitting: false,
      decisionError: undefined,
    });
    return true;
  } catch (error) {
    updateToolCallCard(approvalId, {
      decisionSubmitting: false,
      decisionError: decisionErrorMessage(error),
    });
    return false;
  }
}

export const agentStore = {
  /** Apply a background turn's event to its own projection, not the selected tab. */
  routeSessionEvent(payload: { session_id?: string | null; request_id?: string | null }, apply: () => void): boolean {
    const sessionId = payload.session_id;
    if (!sessionId || sessionId === state.activeSessionId) return false;
    const selected = state.activeSessionId;
    const permissionMode = state.permissionMode;
    routingSessionEvent = true;
    selectSessionView(sessionId);
    if (!activeRequestId && payload.request_id && !terminalRequests.has(payload.request_id)) {
      activeRequestId = payload.request_id;
      state = { ...state, inFlight: true };
    }
    try { apply(); } finally {
      notify();
      selectSessionView(selected);
      state = { ...state, permissionMode };
      routingSessionEvent = false;
      notify();
    }
    return true;
  },
  subscribe(listener: Listener): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },

  getSnapshot(): AgentState {
    return state;
  },

  hydrate(bootstrap: BootstrapState) {
    const previousSessionId = state.activeSessionId;
    if (context && (context.workspace_id !== bootstrap.workspace.id || context.core_instance_id !== bootstrap.core_instance_id)) {
      sessionViews.clear();
      pendingApprovalRequests.clear();
      childApprovalOwners.clear();
      workflowConfirmations.clear();
      completedSubagentIds.clear();
      state = { ...state, transcripts: {}, todosBySession: {}, messages: [], subagents: [], artifacts: [] };
    }
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
      isRunning: session.active,
    }));

const activeSessionId =
      bootstrap.chat.active_session_id ??
      bootstrap.chat.sessions.find((session) => session.active)?.session_id ??
      sessions[0]?.id ??
      '';

    // Keep each backend session's transcript separate: switching a tab must
    // show that session's messages, not the flattened union of every session.
    const transcripts: Record<string, ChatMessage[]> = {};
    for (const session of sessions) transcripts[session.id] = [];
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

    for (const message of bootstrap.chat.messages) {
      if (!transcripts[message.session_id]) continue;
      if (message.role !== 'user' && message.role !== 'assistant') continue;
      transcripts[message.session_id]!.push({
        id: message.message_id,
        role: message.role,
        content: message.text,
        status: 'complete',
        timestamp: Date.now(),
      });
    }
    for (const session of sessions) {
      const previous = state.transcripts[session.id] ?? [];
      if (session.isRunning && previous.length > 0) {
        transcripts[session.id] = previous;
      } else {
        for (const message of previous.filter((message) => message.toolCalls?.length)) {
          const index = transcripts[session.id]!.findIndex((entry) => entry.id === message.id);
          if (index >= 0) transcripts[session.id]![index] = { ...transcripts[session.id]![index]!, toolCalls: message.toolCalls };
          else transcripts[session.id]!.push(message);
        }
      }
    }

    // Phase 9a additive seam: `ChatStateDto.todos` is authoritative once 9b
    // lands. Without it the list stays empty; no step is ever synthesized.
    const todosBySession: Record<string, AgentTodo[]> = {};
    for (const todo of bootstrap.chat.todos ?? []) {
      const sessionId = todo.session_id ?? activeSessionId;
      if (!sessionId) continue;
      (todosBySession[sessionId] ??= []).push(...parseSessionTodos([todo]));
    }

    // Phase 9b: the active session's plan/todos hydrate the dock's spine
    // from the Rust snapshot (session-scoped state, not a turn artifact).
    const activeSession = bootstrap.chat.sessions.find(
      (session) => session.session_id === activeSessionId,
    );
    for (const session of bootstrap.chat.sessions) {
      if (session.todos !== undefined) todosBySession[session.session_id] = todosFromSnapshot(session.todos);
    }
    const hydratedTodos = todosBySession[activeSessionId] ?? [];
    const hydratedPlanPhase: PlanPhase = activeSession?.plan_phase ?? 'executing';
    const hydratedPlan = validPlanApproval(activeSession?.plan)
      ? activeSession.plan
      : null;

    // Keep request routing aligned with the turn Rust says is live.
    activeRequestId = bootstrap.chat.active_request_id ?? null;
    for (const session of bootstrap.chat.sessions) {
      const previous = sessionViews.get(session.session_id);
      const isActive = session.session_id === activeSessionId;
      sessionViews.set(session.session_id, {
        messages: transcripts[session.session_id] ?? [],
        todos: todosBySession[session.session_id] ?? [],
        plan: validPlanApproval(session.plan) ? session.plan : null,
        planPhase: session.plan_phase ?? 'executing',
        inFlight: Boolean(previous?.inFlight || session.active || (isActive && activeRequestId)),
        thinkingState: previous?.thinkingState ?? 'breathing',
        metrics: previous?.metrics ?? { totalTokens: 0, costUsd: 0, activeToolsCount: 0, approvedCount: 0, deniedCount: 0, latencyMs: 0 },
        promptQueue: previous?.promptQueue ?? [],
        streamingActivityLevel: previous?.streamingActivityLevel ?? 0,
        requestId: previous?.requestId ?? (isActive ? activeRequestId : null),
      });
    }

    // A pending approval survives the reload as a parked tool card so the
    // user can still decide it. Workflow summaries cannot authorize a run:
    // retain their identity, but wait for the full approval_needed payload.
    const previousConfirmations = new Map(workflowConfirmations);
    pendingApprovalRequests.clear();
    const parks = bootstrap.chat.pending_approvals?.length
      ? bootstrap.chat.pending_approvals
      : bootstrap.chat.pending_approval ? [bootstrap.chat.pending_approval] : [];
    for (const pending of parks) {
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
        decisionPrompt: pending.decision_prompt,
        plan: isPlan
          ? (pending.plan ?? previousCall?.plan ?? undefined)
          : undefined,
        resultSummary: isWorkflow
          ? 'Workflow confirmation is unavailable after refresh. Wait for the full approval_needed payload before deciding.'
          : undefined,
        startedAt: Date.now(),
      };
      const pendingSessionId = pending.session_id || activeSessionId;
      const pendingMessages = pendingSessionId
        ? (transcripts[pendingSessionId] ??= [])
        : (transcripts[activeSessionId] ??= []);
      const lastAssistantIndex = pendingMessages.reduce(
        (found, message, index) => (message.role === 'assistant' ? index : found),
        -1,
      );
      if (lastAssistantIndex >= 0) {
        const target = pendingMessages[lastAssistantIndex]!;
        pendingMessages[lastAssistantIndex] = {
          ...target,
          toolCalls: [...(target.toolCalls ?? []).filter((call) => call.approvalId !== pending.approval_id), approvalCall],
        };
      } else {
        pendingMessages.push({
          id: `${PENDING_APPROVAL_PREFIX}${pending.approval_id}`,
          role: 'assistant',
          content: '',
          status: 'complete',
          timestamp: Date.now(),
          toolCalls: [approvalCall],
        });
      }
    }

    // A child is only meaningful under a session this snapshot still knows.
    // Their transcripts stay in `subagents[].messages`; they never merge into
    // `transcripts`/`messages`.
    const validParents = new Set(
      [...sessions.map((session) => session.id), activeSessionId].filter(Boolean),
    );
    const subagents = state.subagents.filter(
      (child) => Boolean(child.parentSessionId) && validParents.has(child.parentSessionId!),
    );
    const activeSubagentId = activeChildStillBelongs(
      state.activeSubagentId,
      activeSessionId,
      subagents,
    );

    state = {
      ...state,
      availableModels: models,
      selectedModel: modelToSelect,
      workspace,
      sessions,
      transcripts,
      todosBySession,
      messages: transcripts[activeSessionId] ?? [],
      resumableSessions,
      subagents,
      unreadSubagentCount: countUnreadSubagents(subagents),
      activeSubagentId,
      activeSessionId,
      permissionMode: activeSessionId === previousSessionId ? state.permissionMode : DEFAULT_PERMISSION_MODE,
      todos: hydratedTodos,
      planPhase: hydratedPlanPhase,
      plan: hydratedPlan,
      inFlight: Boolean(bootstrap.chat.active_request_id),
      sessionBusy: false,
      sessionError: null,
    };
    const previousView = sessionViews.get(activeSessionId);
    if (previousView && state.inFlight) {
      state = { ...state, thinkingState: previousView.thinkingState, metrics: previousView.metrics,
        streamingActivityLevel: previousView.streamingActivityLevel, promptQueue: previousView.promptQueue };
    }
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
    sessionViews.clear();
    context = null;
    activeRequestId = null;
    pendingApprovalRequests.clear();
    childApprovalOwners.clear();
    workflowConfirmations.clear();
    terminalRequests.clear();
    completedSubagentIds.clear();
    state = {
      ...state,
      sessions: [],
      activeSessionId: '',
      messages: [],
      transcripts: {},
      todosBySession: {},
      todos: [],
      planPhase: 'executing',
      plan: null,
      subagents: [],
      unreadSubagentCount: 0,
      revealedSubagentId: null,
      activeSubagentId: null,
      inFlight: false,
      thinkingState: 'breathing',
      permissionMode: DEFAULT_PERMISSION_MODE,
      streamingActivityLevel: 0,
      promptQueue: [],
      availableModels: [],
      selectedModel: '',
      workspace: null,
      sessionBusy: false,
      sessionError: null,
      resumableSessions: [],
    };
    notify();
  },

  /**
   * Ask the backend for a real session. Rust creates it, selects it, and
   * emits the bootstrap snapshot this store hydrates from — no local
   * placeholder row is ever fabricated.
   */
  async createSession(_title?: string): Promise<boolean> {
    state = { ...state, permissionMode: DEFAULT_PERMISSION_MODE };
    if (!context) {
      toastStore.pushLocal('warning', 'Open a folder and connect before starting a session.');
      return false;
    }
    try {
      await requestNewSessionCommand({ ...context, session_id: null });
      return true;
    } catch (error) {
      toastStore.pushLocal('error', sessionCommandError('create', error));
      return false;
    }
  },

  /** Select an existing backend session (unknown ids are refused by Rust). */
  async switchSession(sessionId: string): Promise<boolean> {
    if (!state.sessions.some((session) => session.id === sessionId)) return false;
    if (!context) return false;
    if (sessionId === state.activeSessionId) return true;
    try {
      await switchSessionCommand(sessionId, { ...context, session_id: null });
      return true;
    } catch (error) {
      toastStore.pushLocal('error', sessionCommandError('switch', error));
      return false;
    }
  },

  /** Retry a user turn: Rust truncates the failed tail and dispatches once. */
  async retryMessage(messageId: string): Promise<boolean> {
    const target = state.messages.find((message) => message.id === messageId);
    if (!target || target.role !== 'user') return false;
    if (!context || !state.activeSessionId) return false;
    try {
      await bridgeRetryMessage(state.activeSessionId, messageId, {
        ...context,
        session_id: state.activeSessionId,
      });
      return true;
    } catch (error) {
      toastStore.pushLocal('error', sessionCommandError('retry', error));
      return false;
    }
  },

  /**
   * Local fork helper (Phase Z3 `forkSessionFromPrompt`). Session tabs never
   * call this: their New session goes through `requestNewSession`, which is the
   * real `NewSession` command and never invents a session on failure.
   */
  createLocalSession(title = 'New Agent Session'): string {
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
      activeSubagentId: null,
      messages: [],
      transcripts: { ...state.transcripts, [newId]: [] },
      todosBySession: { ...state.todosBySession, [newId]: [] },
      todos: [],
      planPhase: 'executing',
      plan: null,
      inFlight: false,
      thinkingState: 'breathing',
      // The flag dies with the session that enabled it.
      permissionMode: DEFAULT_PERMISSION_MODE,
      promptQueue: [],
    };
    notify();
    return newId;
  },

  /**
   * Phase 9a — the real `NewSession` command. A failure (today the backend
   * answers `unsupported_phase_slice`) surfaces as `sessionError` and leaves
   * the session list untouched: no local placeholder session is ever created.
   * On success the authoritative `bootstrap_state` event rehydrates the rail.
   */
  async requestNewSession(): Promise<boolean> {
    // Section-scoped commands stay off the envelope (state.rs only allows the
    // envelope session for SendChat/StopChat/SendAgent/AgentDecision). NewSession
    // has no target at all.
    const ctx = context;
    if (!ctx) {
      state = {
        ...state,
        sessionError: 'Open a workspace before creating an agent session.',
      };
      notify();
      return false;
    }
    if (state.sessionBusy) return false;
    state = { ...state, sessionBusy: true, sessionError: null };
    notify();
    try {
      await requestNewSessionCommand(ctx);
      state = { ...state, permissionMode: DEFAULT_PERMISSION_MODE };
      return true;
    } catch (error) {
      state = { ...state, sessionError: sessionCommandError('create', error) };
      return false;
    } finally {
      state = { ...state, sessionBusy: false };
      notify();
    }
  },

  /**
   * Select a backend session locally without a command; only the fork/tests
   * use it, because session tabs dispatch the real `SwitchSession`.
   */
  selectSessionLocally(sessionId: string) {
    if (state.activeSessionId === sessionId) return;
    selectSessionView(sessionId, true);
    notify();
  },

  /**
   * Phase 9a — the real `SwitchSession` command. The target must already be a
   * backend-reported session; after the command succeeds the local view moves
   * to that session's backend transcript. A rejected command changes nothing.
   */
  async requestSwitchSession(sessionId: string): Promise<boolean> {
    if (sessionId === state.activeSessionId) return true;
    if (!state.sessions.some((session) => session.id === sessionId) &&
        !state.resumableSessions.some((session) => session.id === sessionId)) {
      state = {
        ...state,
        sessionError: 'That session is not in the backend session list.',
      };
      notify();
      return false;
    }
    const ctx = context;
    if (!ctx) {
      state = {
        ...state,
        sessionError: 'Open a workspace before switching agent sessions.',
      };
      notify();
      return false;
    }
    if (state.sessionBusy) return false;
    state = { ...state, sessionBusy: true, sessionError: null };
    notify();
    try {
      await switchSessionCommand(sessionId, ctx);
      selectSessionView(sessionId, true);
      state = {
        ...state,
        activeSessionId: sessionId,
        messages: state.transcripts[sessionId] ?? [],
        todos: state.todosBySession[sessionId] ?? [],
        sessionBusy: false,
      };
      notify();
      return true;
    } catch (error) {
      state = {
        ...state,
        sessionBusy: false,
        sessionError: sessionCommandError('switch', error),
      };
      notify();
      return false;
    }
  },

  /**
   * Delete a session through the backend. A durable session's on-disk store
   * is removed too, so it cannot be resumed later. The backend refuses while
   * a turn runs; that message is shown instead of hiding the tab.
   */
  async requestDeleteSession(sessionId: string): Promise<boolean> {
    if (
      !state.sessions.some((session) => session.id === sessionId) &&
      !state.resumableSessions.some((session) => session.id === sessionId)
    ) {
      state = {
        ...state,
        sessionError: 'That session is not in the backend session list.',
      };
      notify();
      return false;
    }
    const ctx = context;
    if (!ctx) {
      state = {
        ...state,
        sessionError: 'Open a workspace before deleting agent sessions.',
      };
      notify();
      return false;
    }
    if (state.sessionBusy) return false;
    state = { ...state, sessionBusy: true, sessionError: null };
    notify();
    try {
      await deleteSessionCommand(sessionId, ctx);
      this.deleteSession(sessionId);
      state = { ...state, sessionBusy: false };
      notify();
      return true;
    } catch (error) {
      state = {
        ...state,
        sessionBusy: false,
        sessionError: sessionCommandError('delete', error),
      };
      notify();
      return false;
    }
  },

  /**
   * Apply a `session_todos_changed` event. The list is authoritative and
   * session-scoped: a late event from another session is ignored (the next
   * bootstrap restores that session's own list on switch).
   */
  applySessionTodos(payload: { session_id: string; todos: TodoDto[] }): void {
    const todos = payload.todos.map(todoFromDto);
    const todosBySession = { ...state.todosBySession, [payload.session_id]: todos };
    if (payload.session_id !== state.activeSessionId) {
      state = { ...state, todosBySession };
      return;
    }
    state = { ...state, todos, todosBySession };
    notify();
  },

  /**
   * Live child activity. Every branch writes only the child ledger; the
   * parent's `messages`/`transcripts` are never touched. Late activity may
   * arrive before `started`, so the child is upserted onto its named parent.
   */
  handleSubagentActivity(payload: SubagentActivityPayload): void {
    const { child_id, parent_session_id } = payload;
    if (payload.kind !== 'started' && completedSubagentIds.has(child_id)) return;

    switch (payload.kind) {
      case 'started': {
        const existing = state.subagents.find((child) => child.id === child_id);
        const mission = payload.mission ?? '';
        const derivedName = subagentDisplayName(payload.name, mission, child_id);
        const hasFreshName = Boolean(payload.name?.trim() || mission.trim());
        const name = hasFreshName
          ? derivedName
          : meaningfulSubagentName(existing) ?? derivedName;
        const existingMessages = existing?.messages ?? [];
        const missionId = `subagent-${child_id}-mission`;
        const missionMessage: ChatMessage = {
          id: missionId,
          role: 'user',
          content: mission,
          status: 'complete',
          timestamp: existing?.startedAt ?? Date.now(),
        };
        const messages = mission
          ? existingMessages.some((message) => message.id === missionId)
            ? existingMessages.map((message) =>
                message.id === missionId ? { ...message, content: mission } : message)
            : [missionMessage, ...existingMessages]
          : existingMessages;
        updateChildSubagent(child_id, parent_session_id, (child) => ({
          ...child,
          name,
          mission: mission || child.mission,
          policy: payload.policy,
          model: child.model || 'Auto',
          status: isTerminalChildStatus(child.status) ? child.status : 'working',
          state: child.state ?? (isTerminalChildStatus(child.status) ? 'done' : 'thinking'),
          startedAt: child.startedAt ?? Date.now(),
          unread: isTerminalChildStatus(child.status) ? child.unread : false,
          messages,
        }));
        break;
      }
      case 'delta': {
        updateChildSubagent(child_id, parent_session_id, (child) => {
          if (isTerminalChildStatus(child.status)) return child;
          const messages = [...(child.messages ?? [])];
          const index = messages.findIndex((message) => message.id === payload.message_id);
          if (index >= 0) {
            const message = messages[index]!;
            messages[index] = {
              ...message,
              content: message.content + payload.text,
              // A streamed message stays streamed; completion closes it.
              status: message.status === 'error' ? 'error' : 'streaming',
            };
          } else {
            messages.push({
              id: payload.message_id,
              role: 'assistant',
              content: payload.text,
              status: 'streaming',
              timestamp: Date.now(),
            });
          }
          return {
            ...child,
            messages,
            status: 'working',
            state: child.state === 'running_tool' ? 'running_tool' : 'thinking',
            activeStep: 'Responding',
          };
        });
        break;
      }
      case 'tool_call_started': {
        const { argsSummary, argsRaw } = summarizeToolArgs(payload.args_json, payload.tool_name);
        updateChildSubagent(child_id, parent_session_id, (child) => {
          if (isTerminalChildStatus(child.status)) return child;
          const call: AgentToolCall = {
            id: payload.call_id,
            toolName: payload.tool_name,
            argsSummary,
            ...(argsRaw ? { argsRaw } : {}),
            status: 'running',
            startedAt: Date.now(),
          };
          const messages = withChildAssistant(child, child_id, (message) => ({
            ...message,
            toolCalls: [
              ...(message.toolCalls ?? []).filter((entry) => entry.id !== payload.call_id),
              call,
            ],
          }));
          return {
            ...child,
            messages,
            status: 'working',
            state: 'running_tool',
            activeTool: payload.tool_name,
            activeStep: `Running ${payload.tool_name}`,
          };
        });
        break;
      }
      case 'tool_result': {
        updateChildSubagent(child_id, parent_session_id, (child) => {
          if (isTerminalChildStatus(child.status)) return child;
          let found = false;
          const messages = (child.messages ?? []).map((message) => ({
            ...message,
            toolCalls: message.toolCalls?.map((call) => {
              if (call.id !== payload.call_id) return call;
              found = true;
              return {
                ...call,
                status: (payload.is_err ? 'error' : 'completed') as AgentToolCall['status'],
                resultSummary: payload.result_summary,
                exitCode: payload.is_err ? 1 : 0,
                durationMs: Date.now() - call.startedAt,
              };
            }),
          }));
          return {
            ...child,
            messages,
            status: 'working',
            state: 'thinking',
            activeTool: undefined,
            activeStep: found ? 'Thinking' : child.activeStep,
          };
        });
        break;
      }
      case 'state_changed': {
        const runState: SubagentLiveState = payload.state;
        updateChildSubagent(child_id, parent_session_id, (child) => {
          if (isTerminalChildStatus(child.status)) return child;
          const activeStep = runState === 'thinking'
            ? 'Thinking'
            : runState === 'running_tool'
              ? payload.tool_name ? `Running ${payload.tool_name}` : 'Running tool'
              : runState === 'awaiting_approval'
                ? payload.tool_name ? `Awaiting approval: ${payload.tool_name}` : 'Awaiting approval'
                : 'Done';
          return {
            ...child,
            status: 'working',
            state: runState,
            activeTool: runState === 'done' ? undefined : payload.tool_name,
            activeStep,
          };
        });
        break;
      }
    }
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
    closeChildApprovalCards(payload.child_id, payload.parent_session_id, 'This child conversation has ended.');

    const existing = state.subagents.find((subagent) => subagent.id === payload.child_id);
    const ok = payload.ok === true;
    const entry: SubagentInfo = {
      // Live transcript and started metadata survive the terminal event.
      ...existing,
      id: payload.child_id,
      name: meaningfulSubagentName(existing) ??
        subagentDisplayName(undefined, existing?.mission, payload.child_id),
      model: existing?.model ?? 'Auto',
      status: ok ? 'done' : 'failed',
      parentSessionId: payload.parent_session_id,
      policy: payload.policy,
      result: payload.final_message,
      error: ok ? undefined : payload.error ?? 'subagent failed without a message',
      artifacts: payload.artifacts ?? [],
      unread: true,
      activeStep: ok
        ? `Completed: ${payload.final_message || 'no output'}`
        : `Failed: ${payload.error ?? 'unknown error'}`,
      state: 'done',
      activeTool: undefined,
      completedAt: Date.now(),
      messages: finalizeChildMessages(existing?.messages, ok ? 'complete' : 'error'),
    };
    const subagents = existing
      ? state.subagents.map((subagent) =>
          subagent.id === payload.child_id ? entry : subagent,
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

  handleSubagentCancelled(payload: { child_id: string; parent_session_id: string }): void {
    if (completedSubagentIds.has(payload.child_id)) return;
    completedSubagentIds.add(payload.child_id);
    closeChildApprovalCards(payload.child_id, payload.parent_session_id, 'Subagent was cancelled.');
    const existing = state.subagents.find((child) => child.id === payload.child_id);
    const entry: SubagentInfo = {
      ...existing,
      id: payload.child_id,
      name: meaningfulSubagentName(existing) ??
        subagentDisplayName(undefined, existing?.mission, payload.child_id),
      model: existing?.model ?? 'Auto',
      parentSessionId: payload.parent_session_id,
      status: 'cancelled',
      result: 'Subagent was cancelled.',
      unread: false,
      activeStep: 'Cancelled',
      activeTool: undefined,
      state: 'done',
      completedAt: Date.now(),
      messages: finalizeChildMessages(existing?.messages, 'complete'),
    };
    const subagents = existing
      ? state.subagents.map((child) => (child.id === payload.child_id ? entry : child))
      : [...state.subagents, entry];
    state = { ...state, subagents, unreadSubagentCount: countUnreadSubagents(subagents) };
    notify();
  },

  /**
   * A notification click: select the parent session and reveal the child's
   * result under it. Marking read keeps the badge truthful.
   */
  async revealSubagent(sessionId: string, childId: string): Promise<boolean> {
    if (sessionId && sessionId !== state.activeSessionId && !await this.requestSwitchSession(sessionId)) return false;
    const subagents = state.subagents.map((subagent) =>
      subagent.id === childId ? { ...subagent, unread: false } : subagent,
    );
    state = {
      ...state,
      subagents,
      unreadSubagentCount: countUnreadSubagents(subagents),
      revealedSubagentId: childId,
    };
    notify();
    return true;
  },

  /**
   * Open a child transcript in the dock. Only a child that belongs to the
   * active parent session can open; callers (title-bar pet, results row) get
   * `false` rather than showing a transcript under the wrong parent.
   */
  openSubagentSession(childId: string): boolean {
    const child = state.subagents.find((entry) => entry.id === childId);
    if (!child || child.parentSessionId !== state.activeSessionId) return false;
    const subagents = state.subagents.map((entry) =>
      entry.id === childId ? { ...entry, unread: false } : entry,
    );
    state = {
      ...state,
      subagents,
      unreadSubagentCount: countUnreadSubagents(subagents),
      activeSubagentId: childId,
      revealedSubagentId: state.revealedSubagentId === childId ? null : state.revealedSubagentId,
    };
    notify();
    return true;
  },

  /** Return from the child transcript to the parent's normal feed. */
  closeSubagentSession() {
    if (state.activeSubagentId === null) return;
    state = { ...state, activeSubagentId: null };
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
    sessionViews.delete(sessionId);
    const wasActive = state.activeSessionId === sessionId;
    const remaining = state.sessions.filter((s) => s.id !== sessionId);
    // Deleting a background tab must not yank the user's selection. When the
    // active tab goes, the backend's recency order (oldest first) makes the
    // last remaining row the real replacement.
    const nextActive = wasActive
      ? remaining[remaining.length - 1]?.id || ''
      : state.activeSessionId;
    const { [sessionId]: _removedTranscript, ...transcripts } = state.transcripts;
    const { [sessionId]: _removedTodos, ...todosBySession } = state.todosBySession;
    // Children belong to their parent session: deleting the parent deletes
    // only their byte-for-byte child ledger, never a parent transcript entry.
    const subagents = state.subagents.filter((child) => child.parentSessionId !== sessionId);
    const activeSubagentId = activeChildStillBelongs(
      state.activeSubagentId,
      nextActive,
      subagents,
    );
    state = {
      ...state,
      sessions: remaining,
      resumableSessions: state.resumableSessions.filter((session) => session.id !== sessionId),
      transcripts,
      todosBySession,
      subagents,
      unreadSubagentCount: countUnreadSubagents(subagents),
      activeSubagentId,
      activeSessionId: nextActive,
      messages: transcripts[nextActive] ?? [],
      todos: todosBySession[nextActive] ?? [],
    };
    notify();
  },

  /** Phase 9d — open a durable session restored by Rust. */
  resumeSession(sessionId: string): boolean {
    if (!state.resumableSessions.some((session) => session.id === sessionId)) return false;
    void this.requestSwitchSession(sessionId);
    return true;
  },

  /** Phase 9d — new session (Rust owns the id and the empty on-disk store). */
  newDurableSession(title = 'New Agent Session'): Promise<boolean> {
    return this.createSession(title);
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

// The backend owns the transcript; these are the live render rows only.
    // Todos are NOT seeded from the prompt — `manage_todo` is the only writer.
    const updatedSubagents = state.subagents;

    state = {
      ...state,
      messages: [...state.messages, userMsg, assistantMsg],
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
    const target = findApprovalCard(approvalId) as WorkflowToolCall | undefined;
    const childOwner = childApprovalOwners.get(approvalId);
    // A child card never grants this turn's blanket approval.
    if (childOwner && decision === 'approve_all') return;
    const planCard = Boolean(target?.plan) || target?.toolName === 'exit_plan_mode';
    // A question is never approved or denied generically: it needs text or
    // one of the offered options (the answerQuestion route).
    if (target?.decisionPrompt?.kind === 'question') return;
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
    const sessionContext = requestId
      ? childOwner && context
        ? { ...context, session_id: childOwner.parentSessionId }
        : sessionEnvelopeContext()
      : null;
    if (!requestId || !sessionContext) return;

    const mappedDecision = decision === 'approve_all' ? 'approve_all_this_turn' : decision;

    // A child card stays parked until its decision route actually succeeds:
// the pet transcript and the parent awareness card both show the truth.
    if (childOwner) {
      updateToolCallCard(approvalId, { decisionSubmitting: true, decisionError: undefined });
      return agentDecision(requestId, approvalId, mappedDecision, sessionContext).then(() => {
        pendingApprovalRequests.delete(approvalId);
        childApprovalOwners.delete(approvalId);
        updateToolCallCard(approvalId, {
          status: decision === 'deny' || decision === 'revise' ? 'denied' : 'approved',
          decisionSubmitting: false,
          decisionError: undefined,
        });
      }).catch((error: unknown) => {
        updateToolCallCard(approvalId, {
          decisionSubmitting: false,
          decisionError: decisionErrorMessage(error),
        });
      });
    }

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
      if (c.approvalId === approvalId || (decision === 'approve_all' && !c.backgroundChild && c.toolName !== 'run_workflow' && !(c as WorkflowToolCall).workflow && !(c as WorkflowToolCall).plan)) {
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

    // The roster is backend-reported. A tool start never fabricates a child.
    const updatedSubagents = state.subagents;

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
        todosBySession: {
          ...state.todosBySession,
          ...(state.activeSessionId ? { [state.activeSessionId]: updatedTodos } : {}),
        },
        subagents: updatedSubagents,
        thinkingState: payload.tool_name === 'read_file' ? 'searching' : 'solving',
      };
      notify();
    } else {
      state = {
        ...state,
        todos: updatedTodos,
        todosBySession: {
          ...state.todosBySession,
          ...(state.activeSessionId ? { [state.activeSessionId]: updatedTodos } : {}),
        },
        subagents: updatedSubagents,
        thinkingState: payload.tool_name === 'read_file' ? 'searching' : 'solving',
      };
      notify();
    }
  },

  handleToolResult(payload: ToolResultPayload) {
    if (!isActiveEvent(payload)) return;
    this.updateToolResult(payload.call_id, payload.result_summary, payload.is_err);
  },

  handleApprovalNeeded(payload: ApprovalNeededPayload, backgroundChildEvent = false) {
    const childTarget = childApprovalTarget(payload);
    // A late card after completion/cancellation must never resurrect the child.
    if (childTarget && completedSubagentIds.has(childTarget.childId)) return;
    const parentVisible = !payload.session_id || payload.session_id === state.activeSessionId;
    // Remember which request owns this approval before any transcript write.
    if (payload.request_id) {
      pendingApprovalRequests.set(payload.approval_id, payload.request_id);
    }
    // A child card belongs to the child transcript even when another parent
    // session is on screen. Opening that pet/session later must show the park.
    if (childTarget) attachChildApprovalCard(payload, childTarget);
    if (!parentVisible) return;
    // A background child can ask after its parent turn ended. Its card is
    // still bound to the same session and is safe to render there.
    if (!isActiveEvent(payload) && !backgroundChildEvent && !childTarget) return;
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
    const questionPrompt = payload.prompt?.kind === 'question';
    const diffHunks =
      !workflow && !plan && !questionPrompt && payload.diff_or_input ? [payload.diff_or_input] : undefined;
    const approvalCall: WorkflowToolCall = {
      id: `call_appr_${payload.approval_id}`,
      toolName: payload.tool_name,
      argsSummary: plan?.summary || payload.path_or_command,
      status: 'needs_approval',
      approvalId: payload.approval_id,
      backgroundChild: Boolean(childTarget) || backgroundChildEvent,
      workflow,
      decisionPrompt: payload.prompt,
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
    this.applySessionTodos(payload);
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

// Todos are not rewritten here: the plan stays exactly what the backend
    // last stored. A turn ending is not a todo status change, and a child
    // that outlives the parent turn keeps its own backend-reported state.
    state = {
      ...state,
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

  handleAgentEvent(payload: { request_id: string; approval: Approval | null; message: string }) {
    if (!isActiveEvent(payload)) return;
    if (payload.approval) {
      this.handleApprovalNeeded({
        request_id: payload.request_id,
        session_id: state.activeSessionId,
        turn_id: 'turn_live',
        approval_id: payload.approval.approval_id,
        tool_name: 'operation',
        path_or_command: payload.approval.summary,
        prompt: payload.approval.decision_prompt,
      });
    }
    if (payload.message) {
      this.appendAssistantDelta(payload.message);
    }
  },

  /**
   * Answer a parked structured question. Exactly one answer path is required:
   * a known option id and/or non-empty free text. Nothing is sent on an empty
   * form, and the card shows the transport error if the decision fails.
   */
  answerQuestion(
    approvalId: string,
    optionId: string | null,
    freeText: string,
  ): Promise<boolean> {
    const card = findApprovalCard(approvalId);
    const prompt = card?.decisionPrompt;
    if (!prompt || prompt.kind !== 'question') return Promise.resolve(false);
    const text = freeText.trim();
    const ownsOption = optionId
      ? prompt.options.some((option) => option.id === optionId)
      : false;
    if (!ownsOption && !text) {
      updateToolCallCard(approvalId, {
        decisionError: 'Choose an option or type an answer before sending.',
      });
      return Promise.resolve(false);
    }
    return submitPromptDecision(approvalId, 'answer', {
      kind: 'question',
      ...(ownsOption ? { option_id: optionId! } : {}),
      ...(text ? { text } : {}),
    });
  },

  /**
   * Answer a parked plan review. `request_changes` maps to Deny (the loop
   * continues) and carries the revision note; approve maps to the normal
   * permit so the session can flip into execution.
   */
  resolvePlan(
    approvalId: string,
    decision: 'approve' | 'request_changes',
    feedback = '',
  ): Promise<boolean> {
    const card = findApprovalCard(approvalId);
    const prompt = card?.decisionPrompt;
    if (prompt?.kind !== 'plan' && !(card as WorkflowToolCall | undefined)?.plan) return Promise.resolve(false);
    const note = feedback.trim();
    if (decision === 'request_changes' && !note) {
      updateToolCallCard(approvalId, {
        decisionError: 'Describe the changes you want before sending.',
      });
      return Promise.resolve(false);
    }
    return submitPromptDecision(
      approvalId,
      decision === 'approve' ? 'approve' : 'revise',
      {
        kind: 'plan',
        decision,
        ...(note ? { feedback: note } : {}),
      },
    );
  },

  updateTodoStatus(todoId: string, status: TodoStatus) {
    const updated = state.todos.map((todo) => {
      if (todo.id === todoId) {
        return { ...todo, status };
      }
      return todo;
    });
    state = {
      ...state,
      todos: updated,
      todosBySession: {
        ...state.todosBySession,
        ...(state.activeSessionId ? { [state.activeSessionId]: updated } : {}),
      },
    };
    notify();
  },

  addTodo(title: string, assignedSubagent?: string) {
    const newTodo: AgentTodo = {
      id: `todo-${Date.now().toString(36)}`,
      text: title,
      status: 'pending',
      assignedSubagent,
    };
    const updated = [...state.todos, newTodo];
    state = {
      ...state,
      todos: updated,
      todosBySession: {
        ...state.todosBySession,
        ...(state.activeSessionId ? { [state.activeSessionId]: updated } : {}),
      },
    };
    notify();
  },

  removeTodo(todoId: string) {
    const updated = state.todos.filter((t) => t.id !== todoId);
    state = {
      ...state,
      todos: updated,
      todosBySession: {
        ...state.todosBySession,
        ...(state.activeSessionId ? { [state.activeSessionId]: updated } : {}),
      },
    };
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
