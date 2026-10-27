// Selora IDE — ZCode Parity Plan & Progress Card (UI_SPEC §4.2, REVAMP_SPEC §8)
//
// Matches ZCode's ultra-clean Git tools and collapsible Progress card.
// - Zero strikethrough slop: completed tasks use elegant green ring checkmarks with animated SVG draw strokes.
// - Active in-progress task highlighted with dynamic shimmer sheen, glowing indicator arrow, and moonlight-white text.
// - Collapsible '< X completed' accordion to keep the plan hyper-focused.
// - Git tools panel with branch selector, changes diff (+7632 -125), and commit action.
// - Full Subagent swarm roster with live status badges: Running (pulsing radar beacon), Idle, and Completed.
// - Live progress percentage bar with dual-gradient fill and traveling shimmer highlight.
// - Full-view workspace with structured task breakdown, live progress banner, hero objective, and swarm details.
import React, { useState } from 'react';
import {
  ChevronRight,
  ChevronDown,
  GitBranch,
  FileText,
  Code2,
  BarChart2,
  Sparkles,
  CheckCircle,
  Check,
} from '../../../components/icons';
import type { AgentTodo, SubagentInfo, ArtifactItem } from '../types';
import { BotAvatar } from '../presence/BotAvatar';
import { agentStore } from '../agent-store';

export type GitStatus = {
  branch?: string | null;
  changed_paths?: string[];
  staged_paths?: string[];
};

export type AgentPlanSpineProps = {
  todos: AgentTodo[];
  subagents: SubagentInfo[];
  artifacts: ArtifactItem[];
  gitStatus?: GitStatus;
  isFloatingPopover?: boolean;
  isCompactInfeed?: boolean;
  onClosePopover?: () => void;
};

// 1. Animated ZCode Green Ring Checkmark SVG
export function ZCodeCheckRing() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className="zcode-ring-check-svg is-animated"
      style={{ flexShrink: 0, marginTop: '2px' }}
      aria-hidden="true"
    >
      <circle cx="8" cy="8" r="6.5" stroke="var(--ok)" strokeWidth="1.2" className="check-ring-circle" />
      <path
        d="M5 8.2L7 10.2L11 5.8"
        stroke="var(--ok)"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="zcode-check-stroke"
      />
    </svg>
  );
}

// 2. Animated ZCode Active Task Arrow SVG
export function ZCodeActiveArrow() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className="zcode-active-arrow-svg is-pulsing"
      style={{ flexShrink: 0, marginTop: '2px' }}
      aria-hidden="true"
    >
      <circle cx="8" cy="8" r="7.5" fill="var(--c-rgba-59-130-246-0p12)" className="active-arrow-halo" />
      <path
        d="M3 8H13M13 8L9 4M13 8L9 12"
        stroke="var(--c-60a5fa)"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

// 3. ZCode Pending Task Ring SVG
export function ZCodePendingRing() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className="zcode-pending-ring-svg"
      style={{ flexShrink: 0, marginTop: '2px' }}
      aria-hidden="true"
    >
      <circle cx="8" cy="8" r="6.5" stroke="var(--c-52525b)" strokeWidth="1.2" strokeDasharray="3 3" />
    </svg>
  );
}

// Primary action control inside a todo row. Resets the native button chrome
// so the row keeps its flex layout, and lets the sibling remove button stay a
// separate, non-nested control.
const TODO_ROW_PRIMARY_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'flex-start',
  gap: '10px',
  flex: 1,
  minWidth: 0,
  padding: 0,
  margin: 0,
  border: 'none',
  background: 'transparent',
  color: 'inherit',
  font: 'inherit',
  textAlign: 'left',
  cursor: 'pointer',
};

function parseTodoTitle(rawTitle: string): { main: string; subtitle?: string } {
  const match = rawTitle.match(/^(.*?)\s*(\(.*?\))$/);
  if (match) {
    return { main: match[1], subtitle: match[2] };
  }
  return { main: rawTitle };
}

export const AgentPlanSpine: React.FC<AgentPlanSpineProps> = ({
  todos,
  subagents,
  artifacts,
  gitStatus,
  isFloatingPopover = false,
  isCompactInfeed = false,
  onClosePopover,
}) => {
  const [showCompleted, setShowCompleted] = useState(false);

  const completedTodos = todos.filter((t) => t.status === 'completed');
  const activeAndPendingTodos = todos.filter((t) => t.status !== 'completed');
  const activeTodo = todos.find((t) => t.status === 'in_progress');
  const nextPendingTodo = todos.find((t) => t.status === 'pending');
  const completedCount = completedTodos.length;
  const progressPct = todos.length > 0 ? Math.round((completedCount / todos.length) * 100) : 0;

  const branchName = gitStatus?.branch || 'main';
  const changedFilesCount = gitStatus?.changed_paths?.length || 0;

  // Subagents metrics
  const workingSubagents = subagents.filter((s) => s.status === 'working');
  const idleSubagents = subagents.filter((s) => s.status === 'idle');
  const doneSubagents = subagents.filter((s) => s.status === 'done');

  // --- Compact View (Used in In-feed progress bar and Floating popover) ---
  if (isFloatingPopover || isCompactInfeed) {
    return (
      <div
        className={`agent-plan-spine ${
          isFloatingPopover ? 'is-floating-card' : 'is-compact-infeed'
        }`}
        aria-label="Agent Plan & Subagents"
      >
        <div className="zcode-plan-card">
          {/* 1. Git Tools Header */}
          <div className="zcode-card-section zcode-git-section">
            <div className="zcode-git-header">
              <span className="zcode-git-title">Git tools</span>
              <div className="zcode-git-actions">
                <button
                  type="button"
                  className="zcode-action-btn"
                  title="Options"
                  aria-label="Git options"
                >
                  <span>···</span>
                </button>
                {isFloatingPopover && onClosePopover && (
                  <button
                    type="button"
                    className="zcode-action-btn"
                    title="Close popover"
                    onClick={onClosePopover}
                    aria-label="Close"
                  >
                    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6">
                      <path d="M4 4l8 8M12 4l-8 8" strokeLinecap="round" />
                    </svg>
                  </button>
                )}
              </div>
            </div>

            <div className="zcode-git-row">
              <div className="zcode-git-row-label">
                <FileText size={13} className="zcode-git-icon" />
                <span>Changes</span>
              </div>
              <div className="zcode-git-diff">
                {changedFilesCount > 0 ? (
                  <>
                    <span className="diff-add">+{changedFilesCount}</span>
                    <span style={{ color: 'var(--fg-faint)', fontSize: '11px' }}>
                      file{changedFilesCount === 1 ? '' : 's'}
                    </span>
                  </>
                ) : (
                  <span style={{ color: 'var(--fg-faint)', fontSize: '11px' }}>clean</span>
                )}
              </div>
            </div>

            <div className="zcode-git-row">
              <div className="zcode-git-row-label">
                <GitBranch size={13} className="zcode-git-icon" />
                <span>{branchName}</span>
              </div>
              <ChevronDown size={11} className="zcode-chevron-icon" />
            </div>

            <div className="zcode-git-row zcode-commit-row">
              <div className="zcode-git-row-label">
                <span className="zcode-commit-bullet">⊶</span>
                <span className="zcode-commit-text">Commit or push</span>
              </div>
            </div>
          </div>

          <div className="zcode-card-divider" />

          {/* 2. ZCode Progress Section with Live Percentage Bar */}
          <div className="zcode-card-section zcode-progress-section">
            <div className="zcode-progress-header">
              <span className="zcode-progress-label">Progress</span>
              <div className="zcode-progress-indicators">
                <span className="zcode-progress-pct-badge">{progressPct}%</span>
                <span className="zcode-progress-count">
                  {completedCount}/{todos.length}
                </span>
              </div>
            </div>

            {/* Live Progress Bar with Shimmer */}
            <div
              className="plan-live-progress-container"
              role="progressbar"
              aria-label={`Progress: ${progressPct}%`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={progressPct}
            >
              <div className="plan-live-progress-track">
                <div
                  className="plan-live-progress-fill"
                  style={{ width: `${progressPct}%` }}
                >
                  <div className="plan-progress-shimmer-sheen" />
                </div>
              </div>
            </div>

            {/* Collapsible Completed Section */}
            {completedCount > 0 && (
              <div className="zcode-completed-group">
                <button
                  type="button"
                  className="zcode-completed-toggle-btn"
                  onClick={() => setShowCompleted(!showCompleted)}
                  title={showCompleted ? 'Collapse completed tasks' : 'Expand completed tasks'}
                >
                  <span className={`zcode-toggle-chevron ${showCompleted ? 'is-open' : ''}`}>
                    &lt;
                  </span>
                  <span className="zcode-toggle-label">{completedCount} completed</span>
                </button>

                {showCompleted && (
                  <div className="zcode-completed-nested-list">
                    {completedTodos.map((todo) => {
                      const parsed = parseTodoTitle(todo.title);
                      return (
                        <button
                          key={todo.id}
                          type="button"
                          className="zcode-todo-row is-completed"
                          onClick={() => agentStore.updateTodoStatus(todo.id, 'in_progress')}
                          aria-label={`Reopen ${parsed.main}`}
                        >
                          <ZCodeCheckRing />
                          <div className="zcode-todo-body">
                            <span className="zcode-todo-text">{parsed.main}</span>
                            {parsed.subtitle && <span className="zcode-todo-sub">{parsed.subtitle}</span>}
                          </div>
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            )}

            {/* Active and Pending Tasks */}
            {todos.length === 0 ? (
              <div
                className="zcode-todos-empty"
                style={{
                  padding: '16px 12px',
                  color: 'var(--fg-faint)',
                  fontSize: '12px',
                  lineHeight: '1.5',
                  textAlign: 'center',
                }}
              >
                <div>No todos yet</div>
                <div style={{ fontSize: '11px', opacity: 0.75, marginTop: '4px' }}>
                  The session's todo list appears here when the agent reports one.
                </div>
              </div>
            ) : (
              <div className="zcode-todos-list">
                {activeAndPendingTodos.map((todo) => {
                  const isActive = todo.status === 'in_progress';
                  const parsed = parseTodoTitle(todo.title);
                  return (
                    <button
                      key={todo.id}
                      type="button"
                      className={`zcode-todo-row ${
                        isActive ? 'is-active is-active-shimmer' : 'is-pending'
                      }`}
                      onClick={() =>
                        agentStore.updateTodoStatus(
                          todo.id,
                          isActive ? 'completed' : 'in_progress'
                        )
                      }
                      aria-pressed={isActive}
                      aria-label={`${isActive ? 'Complete' : 'Start'} ${parsed.main}`}
                    >
                      {isActive ? <ZCodeActiveArrow /> : <ZCodePendingRing />}
                      <div className="zcode-todo-body">
                        <span className="zcode-todo-text">{parsed.main}</span>
                        {parsed.subtitle && <span className="zcode-todo-sub">{parsed.subtitle}</span>}
                        {todo.assignedSubagent && (
                          <span className="zcode-subagent-tag">
                            <BotAvatar
                              role={todo.assignedSubagent}
                              status={isActive ? 'working' : 'default'}
                              size={12}
                              name={todo.assignedSubagent}
                            />
                            <span>{todo.assignedSubagent}</span>
                          </span>
                        )}
                      </div>
                    </button>
                  );
                })}
              </div>
            )}

            </div>
        </div>
      </div>
    );
  }

  // --- Full View (Accessed via Plan Tab in AgentDock) ---
  return (
    <div className="agent-plan-spine is-full-view" aria-label="Agent Plan & Subagents">
      <div className="zcode-plan-workspace">
        {/* 1. Overview Banner with Live Progress & Metrics */}
        <div className="zcode-plan-overview-banner">
          <div className="zcode-plan-overview-top">
            <div className="zcode-plan-overview-left">
              <div className="zcode-plan-icon-badge">
                <Sparkles size={18} />
              </div>
              <div>
                <h2 className="zcode-plan-title">Plan Execution</h2>
                <p className="zcode-plan-desc">
                  Structured task breakdown, git change tracking, and subagent swarm operations.
                </p>
              </div>
            </div>

            <div className="zcode-plan-overview-pills">
              {activeTodo ? (
                <span className="zcode-pill zcode-pill-active">
                  <span className="zcode-pulse-dot" />
                  In Progress
                </span>
              ) : todos.length > 0 && completedCount === todos.length ? (
                <span className="zcode-pill zcode-pill-complete">
                  <CheckCircle size={12} />
                  All Complete
                </span>
              ) : (
                <span className="zcode-pill zcode-pill-idle">
                  {todos.length > 0 ? 'Paused' : 'Standby'}
                </span>
              )}

              <span className="zcode-pill zcode-pill-idle">
                {completedCount} of {todos.length} steps complete
              </span>

              <span className="zcode-pill zcode-pill-idle" style={{ fontFamily: 'var(--font-mono)' }}>
                {progressPct}%
              </span>
            </div>
          </div>

          {/* Elevated Live Progress Track with Gradient & Shimmer */}
          <div
            className="zcode-plan-progress-wrap"
            role="progressbar"
            aria-label={`Plan progress: ${progressPct}%`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={progressPct}
          >
            <div className="zcode-plan-progress-track">
              <div
                className="zcode-plan-progress-fill"
                style={{ width: `${progressPct}%` }}
              >
                <div className="plan-progress-shimmer-sheen" />
              </div>
            </div>
            <span className="zcode-plan-progress-pct">{progressPct}%</span>
          </div>
        </div>

        {/* 2. Structured Two-Column Grid */}
        <div className="zcode-plan-grid">
          {/* Column 1: Tasks & Active Execution */}
          <div className="zcode-plan-col-tasks">
            {/* Active or Upcoming Objective Card */}
            {activeTodo ? (
              <div className="zcode-dashboard-card zcode-active-task-hero-card is-active-shimmer">
                <div className="zcode-card-header">
                  <div className="zcode-card-title">
                    <span className="zcode-pulse-dot" style={{ background: 'var(--c-3b82f6)' }} />
                    <span>CURRENT OBJECTIVE</span>
                  </div>
                  <span className="zcode-card-badge" style={{ color: 'var(--c-60a5fa)' }}>Active Step</span>
                </div>
                <div className="zcode-hero-body">
                  <div className="zcode-hero-title-row">
                    <ZCodeActiveArrow />
                    <div>
                      <div className="zcode-hero-title">{parseTodoTitle(activeTodo.title).main}</div>
                      {parseTodoTitle(activeTodo.title).subtitle && (
                        <div style={{ color: 'var(--fg-faint)', fontSize: '12px', marginTop: '2px' }}>
                          {parseTodoTitle(activeTodo.title).subtitle}
                        </div>
                      )}
                    </div>
                  </div>
                  {activeTodo.assignedSubagent && (
                    <div className="zcode-hero-meta">
                      <span className="zcode-subagent-tag">
                        <BotAvatar
                          role={activeTodo.assignedSubagent}
                          status="working"
                          size={14}
                          name={activeTodo.assignedSubagent}
                        />
                        <span>{activeTodo.assignedSubagent}</span>
                      </span>
                    </div>
                  )}
                  <div className="zcode-hero-actions">
                    <button
                      type="button"
                      className="zcode-btn-sm primary"
                      onClick={() => agentStore.updateTodoStatus(activeTodo.id, 'completed')}
                    >
                      Complete Step
                    </button>
                    <button
                      type="button"
                      className="zcode-btn-sm"
                      onClick={() => agentStore.updateTodoStatus(activeTodo.id, 'pending')}
                    >
                      Pause Step
                    </button>
                  </div>
                </div>
              </div>
            ) : nextPendingTodo ? (
              <div className="zcode-dashboard-card" style={{ borderColor: 'var(--c-rgba-255-255-255-0p1)' }}>
                <div className="zcode-card-header">
                  <div className="zcode-card-title">
                    <ZCodePendingRing />
                    <span>NEXT OBJECTIVE</span>
                  </div>
                  <span className="zcode-card-badge">Up Next</span>
                </div>
                <div className="zcode-hero-body">
                  <div className="zcode-hero-title-row">
                    <div>
                      <div className="zcode-hero-title">{parseTodoTitle(nextPendingTodo.title).main}</div>
                      {parseTodoTitle(nextPendingTodo.title).subtitle && (
                        <div style={{ color: 'var(--fg-faint)', fontSize: '12px', marginTop: '2px' }}>
                          {parseTodoTitle(nextPendingTodo.title).subtitle}
                        </div>
                      )}
                    </div>
                  </div>
                  <div className="zcode-hero-actions">
                    <button
                      type="button"
                      className="zcode-btn-sm primary"
                      onClick={() => agentStore.updateTodoStatus(nextPendingTodo.id, 'in_progress')}
                    >
                      Start Step
                    </button>
                  </div>
                </div>
              </div>
            ) : todos.length > 0 && completedCount === todos.length ? (
              <div className="zcode-dashboard-card" style={{ borderColor: 'var(--c-rgba-34-197-94-0p25)' }}>
                <div className="zcode-card-header">
                  <div className="zcode-card-title" style={{ color: 'var(--c-4ade80)' }}>
                    <ZCodeCheckRing />
                    <span>PLAN COMPLETED</span>
                  </div>
                  <span className="zcode-card-badge" style={{ color: 'var(--c-4ade80)' }}>100%</span>
                </div>
                <div className="zcode-hero-body">
                  <div style={{ color: 'var(--fg-muted)', fontSize: '13px' }}>
                    All {todos.length} planned objectives have been successfully executed and verified.
                  </div>
                </div>
              </div>
            ) : null}

            {/* Tasks Breakdown Card */}
            <div className="zcode-dashboard-card zcode-tasks-breakdown-card">
              <div className="zcode-card-header">
                <div className="zcode-card-title">
                  <FileText size={15} style={{ color: 'var(--c-71717a)' }} />
                  <span>Tasks &amp; Execution Breakdown</span>
                </div>
                <span className="zcode-card-badge">
                  {completedCount} / {todos.length}
                </span>
              </div>

              <div style={{ padding: '14px 16px' }}>
                {/* Collapsible Completed Section */}
                {completedCount > 0 && (
                  <div className="zcode-completed-group" style={{ marginBottom: '12px' }}>
                    <button
                      type="button"
                      className="zcode-completed-toggle-btn"
                      onClick={() => setShowCompleted(!showCompleted)}
                      title={showCompleted ? 'Collapse completed tasks' : 'Expand completed tasks'}
                    >
                      <span className={`zcode-toggle-chevron ${showCompleted ? 'is-open' : ''}`}>
                        &lt;
                      </span>
                      <span className="zcode-toggle-label">{completedCount} completed</span>
                    </button>

                    {showCompleted && (
                      <div className="zcode-completed-nested-list">
                        {completedTodos.map((todo) => {
                          const parsed = parseTodoTitle(todo.title);
                          return (
                            <div key={todo.id} className="zcode-todo-row is-completed">
                              <button
                                type="button"
                                className="zcode-todo-row-primary"
                                style={TODO_ROW_PRIMARY_STYLE}
                                onClick={() => agentStore.updateTodoStatus(todo.id, 'in_progress')}
                                aria-label={`Reopen ${parsed.main}`}
                              >
                                <ZCodeCheckRing />
                                <span className="zcode-todo-body">
                                  <span className="zcode-todo-text">{parsed.main}</span>
                                  {parsed.subtitle && <span className="zcode-todo-sub">{parsed.subtitle}</span>}
                                </span>
                              </button>
                              <button
                                type="button"
                                className="zcode-todo-remove-btn"
                                title="Remove task"
                                aria-label={`Remove ${parsed.main}`}
                                onClick={() => agentStore.removeTodo(todo.id)}
                              >
                                ×
                              </button>
                            </div>
                          );
                        })}
                      </div>
                    )}
                  </div>
                )}

                {/* Active and Pending Tasks */}
                {todos.length === 0 ? (
                  <div className="zcode-todos-empty-full">
                    <div style={{ fontSize: '13px', fontWeight: 500, color: 'var(--fg-default)' }}>
                      No todos yet
                    </div>
                    <div style={{ fontSize: '12px', color: 'var(--fg-faint)', marginTop: '4px', lineHeight: 1.5 }}>
                      The session's real todo list appears here when the agent reports one.
                    </div>
                  </div>
                ) : (
                  <div className="zcode-todos-list">
                    {activeAndPendingTodos.map((todo) => {
                      const isActive = todo.status === 'in_progress';
                      const parsed = parseTodoTitle(todo.title);
                      return (
                        <div
                          key={todo.id}
                          className={`zcode-todo-row ${
                            isActive ? 'is-active is-active-shimmer' : 'is-pending'
                          }`}
                        >
                          <button
                            type="button"
                            className="zcode-todo-row-primary"
                            style={TODO_ROW_PRIMARY_STYLE}
                            onClick={() =>
                              agentStore.updateTodoStatus(
                                todo.id,
                                isActive ? 'completed' : 'in_progress'
                              )
                            }
                            aria-pressed={isActive}
                            aria-label={`${isActive ? 'Complete' : 'Start'} ${parsed.main}`}
                          >
                            {isActive ? <ZCodeActiveArrow /> : <ZCodePendingRing />}
                            <span className="zcode-todo-body">
                              <span className="zcode-todo-text">{parsed.main}</span>
                              {parsed.subtitle && <span className="zcode-todo-sub">{parsed.subtitle}</span>}
                              {todo.assignedSubagent && (
                                <span className="zcode-subagent-tag">
                                  <BotAvatar
                                    role={todo.assignedSubagent}
                                    status={isActive ? 'working' : 'default'}
                                    size={12}
                                    name={todo.assignedSubagent}
                                  />
                                  <span>{todo.assignedSubagent}</span>
                                </span>
                              )}
                            </span>
                          </button>
                          <button
                            type="button"
                            className="zcode-todo-remove-btn"
                            title="Remove task"
                            aria-label={`Remove ${parsed.main}`}
                            onClick={() => agentStore.removeTodo(todo.id)}
                          >
                            ×
                          </button>
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* Column 2: Git Workspace & Subagents Sidebar */}
          <div className="zcode-plan-col-sidebar">
            {/* Git Workspace Card */}
            <div className="zcode-dashboard-card zcode-git-card">
              <div className="zcode-card-header">
                <div className="zcode-card-title">
                  <GitBranch size={15} style={{ color: 'var(--c-71717a)' }} />
                  <span>Git Workspace</span>
                </div>
                <div className="zcode-git-actions">
                  <button
                    type="button"
                    className="zcode-action-btn"
                    title="Git options"
                    aria-label="Git options"
                  >
                    <span>···</span>
                  </button>
                </div>
              </div>

              <div style={{ padding: '12px 16px', display: 'flex', flexDirection: 'column', gap: '8px' }}>
                <div className="zcode-git-row">
                  <div className="zcode-git-row-label">
                    <FileText size={13} className="zcode-git-icon" />
                    <span>Changes</span>
                  </div>
                  <div className="zcode-git-diff">
                    {changedFilesCount > 0 ? (
                      <>
                        <span className="diff-add">+{changedFilesCount}</span>
                        <span style={{ color: 'var(--fg-faint)', fontSize: '11px' }}>
                          file{changedFilesCount === 1 ? '' : 's'}
                        </span>
                      </>
                    ) : (
                      <span style={{ color: 'var(--fg-faint)', fontSize: '11px' }}>clean</span>
                    )}
                  </div>
                </div>

                <div className="zcode-git-row">
                  <div className="zcode-git-row-label">
                    <GitBranch size={13} className="zcode-git-icon" />
                    <span>{branchName}</span>
                  </div>
                  <ChevronDown size={11} className="zcode-chevron-icon" />
                </div>

                <div className="zcode-git-row zcode-commit-row">
                  <div className="zcode-git-row-label">
                    <span className="zcode-commit-bullet">⊶</span>
                    <span className="zcode-commit-text">Commit or push</span>
                  </div>
                </div>
              </div>
            </div>

            {/* Subagents Swarm Roster with Idle, Running, Completed Badges */}
            <div className="zcode-dashboard-card zcode-swarm-card">
              <div className="zcode-card-header">
                <div className="zcode-card-title">
                  <Sparkles size={15} style={{ color: 'var(--c-a855f7)' }} />
                  <span>Subagents Swarm</span>
                </div>
                <span className="zcode-card-badge">
                  {workingSubagents.length > 0 ? (
                    <span style={{ color: 'var(--c-60a5fa)' }}>{workingSubagents.length} running</span>
                  ) : (
                    <span>{subagents.length} roster</span>
                  )}
                </span>
              </div>

              <div className="swarm-roster-full">
                {subagents.map((sub) => {
                  const isWorking = sub.status === 'working';
                  const isDone = sub.status === 'done';
                  const isIdle = sub.status === 'idle';

                  return (
                    <div
                      key={sub.id}
                      className={`swarm-card-item status-${sub.status} ${isWorking ? 'is-running-card' : ''}`}
                      title={`${sub.name} (${sub.model}): ${sub.activeStep || 'Idle'}`}
                    >
                      <BotAvatar
                        role={sub.name}
                        status={isWorking ? 'working' : isIdle ? 'sleeping' : 'default'}
                        size={24}
                        name={sub.name}
                      />
                      <div className="swarm-info">
                        <div className="swarm-header-row">
                          <span className="subagent-name">{sub.name}</span>
                          <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                            <span className="zcode-card-badge model-spec-badge" style={{ fontSize: '10px' }}>
                              {sub.model || 'Auto'}
                            </span>
                            {/* Rich Status Badges: Running, Idle, Completed */}
                            {isWorking ? (
                              <span className="zcode-subagent-status-badge status-working">
                                <span className="zcode-pulse-dot" style={{ width: '5px', height: '5px' }} />
                                Running
                              </span>
                            ) : isDone ? (
                              <span className="zcode-subagent-status-badge status-done">
                                <Check size={10} className="status-check-icon" />
                                Completed
                              </span>
                            ) : (
                              <span className="zcode-subagent-status-badge status-idle">
                                <span className="status-idle-dot" />
                                Idle
                              </span>
                            )}
                          </div>
                        </div>
                        <span className="subagent-active-step">
                          {sub.activeStep || (isWorking ? 'Processing task directives…' : isDone ? 'Finished all allocated steps' : 'Ready for instructions')}
                        </span>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>

            {/* Artifacts Drawer */}
            <div className="zcode-dashboard-card zcode-artifacts-card">
              <div className="zcode-card-header">
                <div className="zcode-card-title">
                  <Code2 size={15} style={{ color: 'var(--c-71717a)' }} />
                  <span>Artifacts</span>
                </div>
                <span className="zcode-card-badge">
                  {artifacts.length}
                </span>
              </div>

              <div style={{ padding: '12px 16px' }}>
                {artifacts.length === 0 ? (
                  <div style={{ fontSize: '12px', color: 'var(--c-71717a)', textAlign: 'center', padding: '12px 0' }}>
                    No artifacts generated yet
                  </div>
                ) : (
                  <div className="artifacts-grid">
                    {artifacts.map((art) => (
                      <div
                        key={art.id}
                        className="artifact-card"
                        title={`${art.title}\n${art.summary}`}
                      >
                        <div className="artifact-card-top">
                          <span className="artifact-icon">
                            {art.kind === 'doc' ? (
                              <FileText size={13} className="artifact-icon-svg" />
                            ) : art.kind === 'code' ? (
                              <Code2 size={13} className="artifact-icon-svg" />
                            ) : (
                              <BarChart2 size={13} className="artifact-icon-svg" />
                            )}
                          </span>
                          <span className="artifact-title">{art.title}</span>
                        </div>
                        <p className="artifact-summary">{art.summary}</p>
                        <div className="artifact-card-footer">
                          <span className="artifact-path">{art.path}</span>
                          {art.sizeBytes && (
                            <span className="artifact-size">
                              {(art.sizeBytes / 1024).toFixed(1)} KB
                            </span>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
