// Selora IDE — Phase Z3 Agentic Window Types
// Traceable to DESIGN_TOKENS.md, ROADMAP.md Phase 5/9, and ZCode benchmark.

export type ThinkingState =
  | 'breathing'
  | 'working'
  | 'searching'
  | 'solving'
  | 'listening'
  | 'connecting'
  | 'weaving'
  | 'composing'
  | 'shaping';

export type PermissionMode = 'full' | 'plan_only' | 'ask_before_change';

export type ReasoningEffort = 'max' | 'standard' | 'quick';

export type ToolType =
  | 'read_file'
  | 'edit_file'
  | 'write_file'
  | 'bash'
  | 'web_search'
  | 'web_fetch'
  | 'todo_write'
  | 'spawn_subagent';

export type ToolCallStatus =
  | 'running'
  | 'needs_approval'
  | 'approved'
  | 'denied'
  | 'completed'
  | 'error';

export type DiffStat = {
  added: number;
  removed: number;
  oldContent?: string;
  newContent?: string;
  hunks?: string[];
};

export type AgentToolCall = {
  id: string;
  toolName: ToolType | string;
  argsSummary: string;
  argsRaw?: Record<string, unknown>;
  status: ToolCallStatus;
  approvalId?: string;
  resultSummary?: string;
  diff?: DiffStat;
  exitCode?: number;
  durationMs?: number;
  startedAt: number;
};

export type MessageRole = 'user' | 'assistant' | 'system';

export type ChatMessage = {
  id: string;
  role: MessageRole;
  content: string;
  thought?: string;
  thoughtDurationSeconds?: number;
  toolCalls?: AgentToolCall[];
  status: 'complete' | 'streaming' | 'error';
  timestamp: number;
  isSteer?: boolean;
  isEdited?: boolean;
  forkedFromSessionId?: string;
};

export type QueuedPrompt = {
  id: string;
  text: string;
  createdAt: number;
  mode?: PermissionMode;
};

export type TodoStatus = 'pending' | 'in_progress' | 'completed';

export type AgentTodo = {
  id: string;
  title: string;
  status: TodoStatus;
  assignedSubagent?: string;
};

export type SubagentStatus = 'working' | 'idle' | 'done';

export type SubagentInfo = {
  id: string;
  name: string;
  model: string;
  status: SubagentStatus;
  activeStep?: string;
};

export type ArtifactKind = 'doc' | 'code' | 'diff' | 'diagram';

export type ArtifactItem = {
  id: string;
  title: string;
  kind: ArtifactKind;
  path: string;
  sizeBytes?: number;
  summary: string;
  createdAt: number;
};

export type AgentSession = {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  tokenCount: number;
  costUsd: number;
  branchName?: string;
  isCompacted?: boolean;
};

export type AgentMetrics = {
  totalTokens: number;
  costUsd: number;
  activeToolsCount: number;
  approvedCount: number;
  deniedCount: number;
  latencyMs: number;
};
