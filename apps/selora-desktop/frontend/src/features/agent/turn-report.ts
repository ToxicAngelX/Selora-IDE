// Selora IDE — turn report digest.
//
// A finished turn can run dozens of tools. The transcript keeps every card,
// but the reader wants one deterministic summary at the end: what changed,
// what ran, and what it cost. Everything here is derived from data the
// backend already reported — no step is invented.
import type { AgentMetrics, AgentTodo, AgentToolCall, ChatMessage } from './types';

export type ChangedFile = {
  path: string;
  kind: 'wrote' | 'edited' | 'deleted';
  added?: number;
  removed?: number;
};

export type CommandRun = {
  command: string;
  /** `null` while the command never reported a status. */
  ok: boolean | null;
  exitCode?: number;
};

export type TurnReport = {
  files: ChangedFile[];
  commands: CommandRun[];
  reads: number;
  searches: number;
  other: string[];
  tools: number;
  denied: number;
  plan: { completed: number; total: number } | null;
  tokens: number;
  costUsd: number;
  hasWork: boolean;
};

const FILE_TOOLS: Record<string, ChangedFile['kind']> = {
  write_file: 'wrote',
  edit_file: 'edited',
  delete_file: 'deleted',
};

/** Pull the path or command out of a call without guessing a workspace path. */
export function callTarget(tool: AgentToolCall): string {
  const raw = tool.argsRaw ?? {};
  for (const key of ['path', 'filePath', 'file', 'target']) {
    const value = raw[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  for (const key of ['command', 'cmd']) {
    const value = raw[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  const summary = (tool.argsSummary ?? '').trim();
  if (!summary) return '';
  if (summary.startsWith('{') && summary.endsWith('}')) {
    try {
      const parsed = JSON.parse(summary) as Record<string, unknown>;
      for (const key of ['path', 'filePath', 'file', 'command', 'cmd']) {
        const value = parsed[key];
        if (typeof value === 'string' && value.trim()) return value.trim();
      }
    } catch {
      // not JSON; fall through to the raw summary
    }
  }
  return summary;
}

function isFinished(tool: AgentToolCall): boolean {
  return tool.status === 'completed' || tool.status === 'error';
}

/**
 * Summarize the last turn: every tool call after the most recent user prompt.
 * A call that is still running, parked for approval, or denied is not counted
 * as work done.
 */
export function buildTurnReport(
  messages: ChatMessage[],
  metrics: AgentMetrics,
  todos: AgentTodo[] = [],
): TurnReport {
  let lastUserIndex = -1;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index]?.role === 'user') {
      lastUserIndex = index;
      break;
    }
  }
  const turnMessages = messages.slice(lastUserIndex + 1);

  const files: ChangedFile[] = [];
  const commands: CommandRun[] = [];
  const other: string[] = [];
  let reads = 0;
  let searches = 0;
  let tools = 0;

  for (const message of turnMessages) {
    for (const tool of message.toolCalls ?? []) {
      if (!isFinished(tool)) continue;
      tools += 1;

      const fileKind = FILE_TOOLS[tool.toolName];
      if (fileKind) {
        const path = callTarget(tool);
        if (path) {
          files.push({
            path,
            kind: fileKind,
            added: tool.diff?.added,
            removed: tool.diff?.removed,
          });
        }
        continue;
      }

      if (tool.toolName === 'bash' || tool.toolName === 'run_command') {
        const command = callTarget(tool);
        if (command) {
          commands.push({
            command,
            ok:
              tool.status === 'error'
                ? false
                : typeof tool.exitCode === 'number'
                  ? tool.exitCode === 0
                  : null,
            exitCode: tool.exitCode,
          });
        }
        continue;
      }

      if (tool.toolName === 'read_file' || tool.toolName === 'list_dir') {
        reads += 1;
        continue;
      }

      if (tool.toolName === 'web_search' || tool.toolName === 'web_fetch') {
        searches += 1;
        continue;
      }

      if (tool.toolName === 'todo_write' || tool.toolName === 'manage_todo') {
        continue;
      }

      const target = callTarget(tool);
      other.push(target ? `${tool.toolName}: ${target}` : tool.toolName);
    }
  }

  const plan =
    todos.length > 0
      ? {
          completed: todos.filter((todo) => todo.status === 'completed').length,
          total: todos.length,
        }
      : null;

  return {
    files,
    commands,
    reads,
    searches,
    other,
    tools,
    denied: metrics.deniedCount,
    plan,
    tokens: metrics.totalTokens,
    costUsd: metrics.costUsd,
    hasWork: tools > 0,
  };
}

/** Plain-text version, for the report card's Copy action. */
export function reportToText(report: TurnReport): string {
  const lines: string[] = ['Turn report'];
  if (report.files.length > 0) {
    lines.push('', 'Changed files:');
    for (const file of report.files) {
      const delta =
        file.added !== undefined || file.removed !== undefined
          ? ` (+${file.added ?? 0} -${file.removed ?? 0})`
          : '';
      lines.push(`- ${file.kind} ${file.path}${delta}`);
    }
  }
  if (report.commands.length > 0) {
    lines.push('', 'Commands:');
    for (const command of report.commands) {
      const status = command.ok === null ? '' : command.ok ? ' (ok)' : ' (failed)';
      lines.push(`- ${command.command}${status}`);
    }
  }
  if (report.plan) {
    lines.push('', `Plan: ${report.plan.completed}/${report.plan.total} steps complete`);
  }
  const extras: string[] = [];
  if (report.reads > 0) extras.push(`${report.reads} files read`);
  if (report.searches > 0) extras.push(`${report.searches} web lookups`);
  if (report.other.length > 0) extras.push(...report.other);
  if (extras.length > 0) lines.push('', `Other: ${extras.join(', ')}`);
  lines.push(
    '',
    `Usage: ${(report.tokens / 1000).toFixed(1)}k tokens · $${report.costUsd.toFixed(3)} · ${report.tools} tools` +
      (report.denied > 0 ? ` · ${report.denied} denied` : ''),
  );
  return lines.join('\n');
}