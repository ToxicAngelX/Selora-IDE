// Selora IDE — ZCode Parity Tool & Approval Card (UI_SPEC §4.2, PHASE5D §A)
//
// Elevated with top-tier AI IDE styling (Cursor/Windsurf parity):
// - Glowing status border (amber = pending, green = approved/done, blue = running, red = error)
// - Header with tool icon, tool name, target path chip with copy button, elapsed duration badge
// - Diff View overhaul: split / unified toggle, crisp line-by-line diff styling, syntax highlight
// - Approval buttons: "Accept (Enter)" with functional green glow, "Reject (Esc)" with subtle red hover
import React, { useState, useEffect } from 'react';
import {
  FileText,
  FileEdit,
  FilePlus,
  Terminal,
  Search,
  Globe,
  Code2,
  Check,
  X,
  ChevronDown,
  ChevronRight,
  ShieldAlert,
  Copy,
  Clock,
  Columns,
} from '../../../components/icons';
import type { AgentToolCall } from '../types';
import { agentStore, type WorkflowToolCall } from '../agent-store';
import { WorkflowCard } from './WorkflowCard';
import { AgentQuestionCard } from './AgentQuestionCard';
import { AgentPlanCard } from './AgentPlanCard';
import './agent-card-polish.css';

export type ToolCardProps = {
  /** A workflow card carries the §18.1 confirmation payload on the call. */
  tool: WorkflowToolCall;
};

type DiffLine = {
  type: 'add' | 'remove' | 'context' | 'header';
  oldLineNumber?: number;
  newLineNumber?: number;
  text: string;
};

type SplitRow = {
  left?: { lineNumber?: number; text: string; type: 'remove' | 'context' };
  right?: { lineNumber?: number; text: string; type: 'add' | 'context' };
};

/**
 * Parses raw diff hunk lines into structured lines with line numbering.
 */
function parseDiffHunks(hunks: string[]): { unified: DiffLine[]; split: SplitRow[] } {
  const unified: DiffLine[] = [];
  const split: SplitRow[] = [];

  let oldCounter = 1;
  let newCounter = 1;

  for (const line of hunks) {
    if (line.startsWith('@@')) {
      // Parse hunk header: @@ -start,len +start,len @@
      const match = line.match(/@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      if (match) {
        oldCounter = parseInt(match[1], 10);
        newCounter = parseInt(match[2], 10);
      }
      unified.push({ type: 'header', text: line });
      split.push({
        left: { text: line, type: 'context' },
        right: { text: line, type: 'context' },
      });
      continue;
    }

    if (line.startsWith('+')) {
      unified.push({
        type: 'add',
        newLineNumber: newCounter++,
        text: line.slice(1),
      });
      split.push({
        right: {
          lineNumber: newCounter - 1,
          text: line.slice(1),
          type: 'add',
        },
      });
    } else if (line.startsWith('-')) {
      unified.push({
        type: 'remove',
        oldLineNumber: oldCounter++,
        text: line.slice(1),
      });
      split.push({
        left: {
          lineNumber: oldCounter - 1,
          text: line.slice(1),
          type: 'remove',
        },
      });
    } else {
      const content = line.startsWith(' ') ? line.slice(1) : line;
      unified.push({
        type: 'context',
        oldLineNumber: oldCounter++,
        newLineNumber: newCounter++,
        text: content,
      });
      split.push({
        left: {
          lineNumber: oldCounter - 1,
          text: content,
          type: 'context',
        },
        right: {
          lineNumber: newCounter - 1,
          text: content,
          type: 'context',
        },
      });
    }
  }

  return { unified, split };
}

/**
 * Elements that own their own keyboard semantics. A keystroke that starts in one
 * of these (composer, search field, a button inside the card, ...) must never be
 * interpreted as an approval decision — otherwise typing anywhere in the app can
 * approve a pending destructive tool call.
 */
function isKeyboardOwnedTarget(target: EventTarget | null): boolean {
  if (!target || !(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  return Boolean(
    target.closest(
      'input, textarea, select, button, a[href], [contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"], [role="textbox"], [role="searchbox"], [role="combobox"]',
    ),
  );
}

/**
 * Extracts target file path or shell command from tool call args.
 */
function extractTargetInfo(tool: AgentToolCall): { path: string; isCommand: boolean } | null {
  if (tool.argsRaw) {
    if (typeof tool.argsRaw.path === 'string' && tool.argsRaw.path) {
      return { path: tool.argsRaw.path, isCommand: false };
    }
    if (typeof tool.argsRaw.filePath === 'string' && tool.argsRaw.filePath) {
      return { path: tool.argsRaw.filePath, isCommand: false };
    }
    if (typeof tool.argsRaw.command === 'string' && tool.argsRaw.command) {
      return { path: tool.argsRaw.command, isCommand: true };
    }
    if (typeof tool.argsRaw.cmd === 'string' && tool.argsRaw.cmd) {
      return { path: tool.argsRaw.cmd, isCommand: true };
    }
  }

  const summary = tool.argsSummary?.trim();
  if (!summary) return null;

  if (summary.startsWith('{') && summary.endsWith('}')) {
    try {
      const parsed = JSON.parse(summary);
      if (parsed.path) return { path: parsed.path, isCommand: false };
      if (parsed.filePath) return { path: parsed.filePath, isCommand: false };
      if (parsed.command) return { path: parsed.command, isCommand: true };
    } catch {
      // not JSON
    }
  }

  if (tool.toolName === 'bash' || tool.toolName === 'run_command') {
    return { path: summary, isCommand: true };
  }

  const pathMatch = summary.match(/(?:(?:path|file|target)\s*[:=]\s*)?["']?([a-zA-Z0-9_\-./\\]+\.[a-zA-Z0-9]+)["']?/i);
  if (pathMatch) {
    return { path: pathMatch[1], isCommand: false };
  }

  return { path: summary, isCommand: false };
}

export const ToolCard: React.FC<ToolCardProps> = ({ tool }) => {
  const [expanded, setExpanded] = useState(tool.status === 'needs_approval');
  const [diffViewMode, setDiffViewMode] = useState<'unified' | 'split'>('unified');
  const [pathCopied, setPathCopied] = useState(false);
  const [runningElapsed, setRunningElapsed] = useState<number>(0);

  const isRunning = tool.status === 'running';
  const isApproval = tool.status === 'needs_approval';
  const isError = tool.status === 'error' || tool.status === 'denied';
  const isSuccess = tool.status === 'completed' || tool.status === 'approved';

  // A `run_workflow` card is a grant, not a diff: once the confirmation
  // payload is attached (tool_call_started's args or approval_needed), the
  // whole body is the WorkflowCard. It stays visible after the decision so the
  // card records Approve/Deny instead of reverting to a generic tool body.
  const workflow = tool.workflow;
  const showWorkflowGrant =
    Boolean(workflow) &&
    tool.toolName === 'run_workflow' &&
    (isApproval ||
      tool.status === 'approved' ||
      tool.status === 'denied' ||
      tool.status === 'completed');
  const workflowDecision: 'approve' | 'deny' | null = isApproval
    ? null
    : tool.status === 'denied'
    ? 'deny'
    : 'approve';

  // Phase 9a: a structured question / plan is the body of the same parked
  // card. It stays expanded after the decision so the answer remains visible.
   const decisionPrompt = tool.decisionPrompt ?? (tool.plan ? {
     kind: 'plan' as const,
     summary: tool.plan.summary ?? undefined,
     steps: tool.plan.steps.map((title) => ({ title })),
   } : undefined);
  const showDecisionPrompt = Boolean(decisionPrompt);
  // Phase 9b: an `exit_plan_mode` card owns the body with its steps and the
  // Approve/Revise pair. It stays visible after the decision, like the
  // workflow grant, so the transcript records what was approved.

  // Live timer for running tools
  useEffect(() => {
    if (!isRunning) return;
    const interval = setInterval(() => {
      const elapsedMs = Date.now() - (tool.startedAt || Date.now());
      setRunningElapsed(elapsedMs);
    }, 100);
    return () => clearInterval(interval);
  }, [isRunning, tool.startedAt]);

  // A parked approval must be readable the moment it lands: a card that was
  // already on screen while `running` (the `run_workflow` grant is upgraded in
  // place) has to open when it becomes `needs_approval`.
  useEffect(() => {
    if (tool.status === 'needs_approval' || tool.decisionPrompt) setExpanded(true);
  }, [tool.status, tool.decisionPrompt]);

  // Keyboard decisions are handled on the card itself (see handleCardKeyDown),
  // never through a global listener: only keystrokes routed to this card — and
  // not owned by a form/button/contentEditable descendant — may decide.
  const handleCardKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!isApproval || !tool.approvalId || showDecisionPrompt) return;

    // Modified keystrokes belong to global shortcuts (Ctrl+Enter, Cmd+K, ...).
    if (e.ctrlKey || e.altKey || e.metaKey) return;

    // Typing in the composer/search (or activating a nested control) is not consent.
    if (isKeyboardOwnedTarget(e.target)) return;

    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      agentStore.handleApprovalDecision(tool.approvalId, 'approve');
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      agentStore.handleApprovalDecision(tool.approvalId, 'deny');
    }
  };

  const renderToolIcon = () => {
    const iconProps = { size: 13, className: 'tool-icon-svg' };
    switch (tool.toolName) {
      case 'read_file':
        return <FileText {...iconProps} />;
      case 'edit_file':
        return <FileEdit {...iconProps} />;
      case 'write_file':
        return <FilePlus {...iconProps} />;
      case 'bash':
      case 'run_command':
        return <Terminal {...iconProps} />;
      case 'web_search':
        return <Search {...iconProps} />;
      case 'web_fetch':
        return <Globe {...iconProps} />;
      default:
        return <Code2 {...iconProps} />;
    }
  };

  const targetInfo = extractTargetInfo(tool);

  const handleCopyPath = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!targetInfo) return;
    try {
      await navigator.clipboard.writeText(targetInfo.path);
      setPathCopied(true);
      setTimeout(() => setPathCopied(false), 1600);
    } catch {
      setPathCopied(false);
    }
  };

  const formatDuration = () => {
    if (isRunning) {
      return `${(runningElapsed / 1000).toFixed(1)}s`;
    }
    if (tool.durationMs !== undefined) {
      if (tool.durationMs < 1000) return `${tool.durationMs}ms`;
      return `${(tool.durationMs / 1000).toFixed(1)}s`;
    }
    return null;
  };

  const durationText = formatDuration();
  const parsedDiff = tool.diff?.hunks ? parseDiffHunks(tool.diff.hunks) : null;

  return (
    <div
      className={`agent-tool-card agent-card-root tool-${tool.toolName} status-${tool.status} ${
        expanded ? 'is-expanded' : ''
      } ${isApproval ? 'is-approval-pending' : ''} ${isRunning ? 'is-running-glow' : ''}`}
      tabIndex={isApproval ? 0 : undefined}
      onKeyDown={handleCardKeyDown}
      role={isApproval ? 'group' : undefined}
      aria-label={isApproval ? `Review pending ${tool.toolName} tool call` : undefined}
      data-testid={`tool-card-${tool.toolName}`}
    >
      {/* Header Row */}
      <div className="tool-card-header agent-card-header" onClick={() => setExpanded(!expanded)}>
        <div className="tool-header-primary agent-card-primary">
          <button
            type="button"
            className="tool-header-toggle agent-card-toggle"
            onClick={(event) => {
              event.stopPropagation();
              setExpanded(!expanded);
            }}
            aria-expanded={expanded}
            aria-label={`Toggle ${tool.toolName} tool details`}
            style={{ display: 'inline-flex', alignItems: 'center', gap: '7px', minWidth: 0 }}
          >
            <span className="tool-chevron-icon agent-card-chevron" aria-hidden="true">
              {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            </span>

            <span className="tool-icon agent-card-icon">{renderToolIcon()}</span>

            <span className="tool-name agent-card-name">{tool.toolName}</span>
          </button>

          {/* Target Path Chip with Copy Button */}
          {targetInfo && (
            <span
              className={`tool-target-chip agent-card-target ${
                targetInfo.isCommand ? 'is-command' : 'is-file'
              }`}
              title={targetInfo.path}
            >
              <span className="target-chip-text agent-card-target-text">{targetInfo.path}</span>
              <button
                type="button"
                className={`target-chip-copy-btn agent-card-copy ${pathCopied ? 'is-copied' : ''}`}
                onClick={handleCopyPath}
                title={pathCopied ? 'Copied!' : 'Copy path'}
                aria-label={pathCopied ? 'Copied path' : 'Copy path'}
              >
                {pathCopied ? <Check size={10} className="text-ok" /> : <Copy size={10} />}
              </button>
            </span>
          )}

          {!targetInfo && tool.argsSummary && (
            <span className="tool-args-summary agent-card-args">{tool.argsSummary}</span>
          )}
        </div>

        <div className="tool-header-telemetry agent-card-telemetry">
          {/* Diff badge if present (+X -Y) */}
          {tool.diff && (
            <span className="tool-diff-badge agent-card-diff" title={`+${tool.diff.added} / -${tool.diff.removed}`}>
              <span className="diff-add agent-card-diff-add">+{tool.diff.added}</span>
              <span className="diff-remove agent-card-diff-remove">-{tool.diff.removed}</span>
            </span>
          )}

          {/* Elapsed Duration Badge */}
          {durationText && (
            <span
              className={`tool-duration-badge agent-card-duration ${isRunning ? 'is-running' : ''}`}
              title="Execution duration"
            >
              <Clock size={10} className="duration-icon" />
              <span>{durationText}</span>
            </span>
          )}

          {/* Exit code reported by the backend, when it sent one */}
          {tool.exitCode !== undefined && (
            <span
              className={`agent-card-exit ${tool.exitCode === 0 ? 'is-ok' : 'is-error'}`}
              title="Exit code"
            >
              exit {tool.exitCode}
            </span>
          )}

          {/* Status Indicator */}
          <span className="tool-status-glyph agent-card-status">
            {isRunning && <span className="tool-spinner agent-card-spinner" aria-label="Running" />}
            {isApproval && (
              <span className="tool-approval-pill agent-card-pill">
                <ShieldAlert size={11} style={{ marginRight: 3 }} />
                Review
              </span>
            )}
            {isSuccess && (
              <span className="tool-glyph-ok agent-card-glyph-ok" aria-label="Completed">
                <Check size={12} />
              </span>
            )}
            {isError && (
              <span className="tool-glyph-err agent-card-glyph-err" aria-label="Error">
                <X size={12} />
              </span>
            )}
          </span>
        </div>
      </div>

      {/* Expanded Content Drawer */}
      {expanded && (
        <div className="tool-card-body agent-card-body">
          {/* Output / Result text */}
          {tool.resultSummary && (
            <div className="tool-result-box agent-card-result">
              <pre className="tool-result-pre agent-card-result-pre">{tool.resultSummary}</pre>
            </div>
          )}

          {/* Phase 9a: structured question / plan review on the park channel. */}
          {decisionPrompt?.kind === 'question' && tool.approvalId ? (
            <div className="tool-decision-prompt agent-card-prompt" data-testid="tool-question-card">
              <AgentQuestionCard
                prompt={decisionPrompt}
                submitting={tool.decisionSubmitting}
                error={tool.decisionError}
                resolution={tool.resolution}
                onAnswer={(optionId, text) =>
                  void agentStore.answerQuestion(tool.approvalId!, optionId, text)
                }
              />
            </div>
          ) : null}

          {decisionPrompt?.kind === 'plan' && tool.approvalId ? (
            <div className="tool-decision-prompt agent-card-prompt" data-testid="tool-plan-card">
              <AgentPlanCard
                prompt={decisionPrompt}
                submitting={tool.decisionSubmitting}
                error={tool.decisionError}
                resolution={tool.resolution ?? (isApproval ? undefined : {
                  kind: 'plan', decision: tool.status === 'denied' ? 'request_changes' : 'approve',
                  feedback: tool.status === 'denied' ? 'Plan denied' : undefined,
                })}
                onDecide={(decision, feedback) =>
                  void agentStore.resolvePlan(tool.approvalId!, decision, feedback)
                }
                onDeny={isApproval ? () => agentStore.handleApprovalDecision(tool.approvalId!, 'deny') : undefined}
              />
            </div>
          ) : null}

          {/* A workflow grant owns the body: script, command set, cap, and the
              Approve/Deny pair the session routes through the existing
              decision callback. */}
          {showWorkflowGrant && workflow && (
            <div className="tool-approval-gate agent-card-gate" data-testid="tool-workflow-grant">
              <WorkflowCard
                name={workflow.name || tool.argsSummary}
                script={workflow.script}
                commands={workflow.commands}
                maxCostUsd={workflow.max_cost_usd}
                approvalId={tool.approvalId}
                decision={workflowDecision}
                onDecision={(decision) => {
                  if (tool.approvalId) {
                    agentStore.handleApprovalDecision(tool.approvalId, decision);
                  }
                }}
              />
            </div>
          )}

          {/* Interactive Approval Gate & Diff View */}
          {isApproval && !showWorkflowGrant && !showDecisionPrompt && (
            <div className="tool-approval-gate agent-card-gate">
              <div className="approval-banner agent-card-banner">
                <div className="approval-banner-left agent-card-banner-left">
                  <span className="approval-badge agent-card-badge">Needs Approval</span>
                  <span className="approval-desc agent-card-desc">
                    Review proposed file changes before applying to workspace.
                  </span>
                </div>

                {/* Split vs Unified Diff Toggle */}
                {parsedDiff && parsedDiff.unified.length > 0 && (
                  <div className="diff-view-toggle-group agent-card-toggle-group">
                    <button
                      type="button"
                      className={`diff-toggle-btn agent-card-toggle-btn ${diffViewMode === 'unified' ? 'is-active' : ''}`}
                      onClick={() => setDiffViewMode('unified')}
                      title="Unified Diff View"
                    >
                      Unified
                    </button>
                    <button
                      type="button"
                      className={`diff-toggle-btn agent-card-toggle-btn ${diffViewMode === 'split' ? 'is-active' : ''}`}
                      onClick={() => setDiffViewMode('split')}
                      title="Split Diff View"
                    >
                      <Columns size={11} />
                      <span>Split</span>
                    </button>
                  </div>
                )}
              </div>

              {/* Overhauled Line-by-Line Diff Viewer */}
              {parsedDiff && parsedDiff.unified.length > 0 && (
                <div className={`approval-diff-viewer agent-card-diff-viewer mode-${diffViewMode}`}>
                  {diffViewMode === 'unified' ? (
                    /* Unified Diff Mode */
                    <div className="diff-unified-table">
                      {parsedDiff.unified.map((line, idx) => (
                        <div
                          key={idx}
                          className={`diff-line line-${line.type}`}
                        >
                          <span className="diff-line-num old-num">
                            {line.oldLineNumber ?? ''}
                          </span>
                          <span className="diff-line-num new-num">
                            {line.newLineNumber ?? ''}
                          </span>
                          <span className="diff-line-marker">
                            {line.type === 'add' ? '+' : line.type === 'remove' ? '-' : ' '}
                          </span>
                          <span className="diff-line-code">{line.text}</span>
                        </div>
                      ))}
                    </div>
                  ) : (
                    /* Split Diff Mode */
                    <div className="diff-split-table">
                      <div className="diff-split-header">
                        <div className="split-header-col">ORIGINAL</div>
                        <div className="split-header-col">PROPOSED</div>
                      </div>
                      <div className="diff-split-body">
                        {parsedDiff.split.map((row, idx) => (
                          <div key={idx} className="diff-split-row">
                            {/* Left (Original / Deletion) */}
                            <div
                              className={`split-col left-col ${
                                row.left ? `line-${row.left.type}` : 'line-empty'
                              }`}
                            >
                              <span className="split-num">{row.left?.lineNumber ?? ''}</span>
                              <span className="split-text">{row.left?.text ?? ''}</span>
                            </div>

                            {/* Right (Proposed / Addition) */}
                            <div
                              className={`split-col right-col ${
                                row.right ? `line-${row.right.type}` : 'line-empty'
                              }`}
                            >
                              <span className="split-num">{row.right?.lineNumber ?? ''}</span>
                              <span className="split-text">{row.right?.text ?? ''}</span>
                            </div>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              )}

              {/* Action Buttons */}
              <div className="approval-actions agent-card-actions">
                <button
                  type="button"
                  className="btn-deny agent-card-btn agent-card-btn-deny"
                  onClick={() =>
                    agentStore.handleApprovalDecision(tool.approvalId!, 'deny')
                  }
                  title="Reject this tool execution (Esc)"
                  aria-label="Reject (Esc)"
                >
                  <X size={12} className="btn-icon" />
                  <span>Reject (Esc)</span>
                </button>

                {!tool.backgroundChild && (
                  <button
                    type="button"
                    className="btn-approve-all agent-card-btn agent-card-btn-all"
                    onClick={() =>
                      agentStore.handleApprovalDecision(tool.approvalId!, 'approve_all')
                    }
                    title="Approve all remaining tools in this turn"
                    aria-label="Approve All"
                  >
                    Approve All
                  </button>
                )}

                <button
                  type="button"
                  className="btn-approve btn-approve-glow agent-card-btn agent-card-btn-accept"
                  onClick={() =>
                    agentStore.handleApprovalDecision(tool.approvalId!, 'approve')
                  }
                  title="Accept and apply changes (Enter)"
                  aria-label="Accept (Enter)"
                >
                  <Check size={12} className="btn-icon" />
                  <span>Accept (Enter)</span>
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
};
