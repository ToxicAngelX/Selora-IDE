// Selora IDE — Plan & Progress card (UI_SPEC §4.2, REVAMP_SPEC §8)
//
// Phase 9 session slice: this is a renderer over backend-authoritative data.
// The todos are the session's `manage_todo` list (the only writer), and the
// Git rows come from the bootstrap's real status. Nothing here invents a
// task, a subagent roster, a branch name, or a "commit or push" action —
// those surfaces were removed rather than faked.
import React, { useState } from 'react';
import {
  ChevronRight,
  GitBranch,
  FileText,
  Sparkles,
  CheckCircle,
} from '../../../components/icons';
import type { AgentTodo } from '../types';

export type GitStatus = {
  branch?: string | null;
  changed_paths?: string[];
  staged_paths?: string[];
};

export type AgentPlanSpineProps = {
  todos: AgentTodo[];
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
      <circle cx="8" cy="8" r="6.5" stroke="#22c55e" strokeWidth="1.2" className="check-ring-circle" />
      <path
        d="M5 8.2L7 10.2L11 5.8"
        stroke="#22c55e"
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
      <circle cx="8" cy="8" r="7.5" fill="rgba(59, 130, 246, 0.12)" className="active-arrow-halo" />
      <path
        d="M3 8H13M13 8L9 4M13 8L9 12"
        stroke="#60a5fa"
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
      <circle cx="8" cy="8" r="6.5" stroke="#52525b" strokeWidth="1.2" strokeDasharray="3 3" />
    </svg>
  );
}

function parseTodoTitle(rawTitle: string): { main: string; subtitle?: string } {
  const match = rawTitle.match(/^(.*?)\s*(\(.*?\))$/);
  if (match) {
    return { main: match[1], subtitle: match[2] };
  }
  return { main: rawTitle };
}

function GitFacts({ gitStatus }: { gitStatus?: GitStatus }) {
  const branch = gitStatus?.branch ?? null;
  const changedFilesCount =
    (gitStatus?.changed_paths?.length ?? 0) + (gitStatus?.staged_paths?.length ?? 0);
  return (
    <div className="zcode-card-section zcode-git-section">
      <div className="zcode-git-header">
        <span className="zcode-git-title">Git</span>
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
          <span>{branch ?? 'No branch'}</span>
        </div>
      </div>
    </div>
  );
}

function TodoRows({ todos, completed }: { todos: AgentTodo[]; completed: boolean }) {
  return (
    <>
      {todos.map((todo) => {
        const isActive = todo.status === 'in_progress';
        const parsed = parseTodoTitle(todo.text);
        return (
          <div
            key={todo.id}
            className={`zcode-todo-row ${
              completed ? 'is-completed' : isActive ? 'is-active is-active-shimmer' : 'is-pending'
            }`}
          >
            {completed ? <ZCodeCheckRing /> : isActive ? <ZCodeActiveArrow /> : <ZCodePendingRing />}
            <span className="zcode-todo-body">
              <span className="zcode-todo-text">{parsed.main}</span>
              {parsed.subtitle && <span className="zcode-todo-sub">{parsed.subtitle}</span>}
            </span>
          </div>
        );
      })}
    </>
  );
}

export const AgentPlanSpine: React.FC<AgentPlanSpineProps> = ({
  todos,
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

// --- Compact View (in-feed progress bar and floating popover) ---
  if (isFloatingPopover || isCompactInfeed) {
    return (
      <div
        className={`agent-plan-spine ${
          isFloatingPopover ? 'is-floating-card' : 'is-compact-infeed'
        }`}
        aria-label="Agent plan"
      >
        <div className="zcode-plan-card">
          <GitFacts gitStatus={gitStatus} />

          <div className="zcode-card-divider" />

          <div className="zcode-card-section zcode-progress-section">
            <div className="zcode-progress-header">
              <span className="zcode-progress-label">Progress</span>
              <div className="zcode-progress-indicators">
                <span className="zcode-progress-pct-badge">{progressPct}%</span>
                <span className="zcode-progress-count">
                  {completedCount}/{todos.length}
                </span>
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

            <div
              className="plan-live-progress-container"
              role="progressbar"
              aria-label={`Progress: ${progressPct}%`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={progressPct}
            >
              <div className="plan-live-progress-track">
                <div className="plan-live-progress-fill" style={{ width: `${progressPct}%` }}>
                  <div className="plan-progress-shimmer-sheen" />
                </div>
              </div>
            </div>

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
                    <TodoRows todos={completedTodos} completed />
                  </div>
                )}
              </div>
            )}

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
                <TodoRows todos={activeAndPendingTodos} completed={false} />
              </div>
            )}
          </div>
        </div>
      </div>
    );
  }

  // --- Full View (the Plan tab) ---
  return (
    <div className="agent-plan-spine is-full-view" aria-label="Agent plan">
      <div className="zcode-plan-workspace">
        <div className="zcode-plan-overview-banner">
          <div className="zcode-plan-overview-top">
            <div className="zcode-plan-overview-left">
              <div className="zcode-plan-icon-badge">
                <Sparkles size={18} />
              </div>
              <div>
                <h2 className="zcode-plan-title">Plan Execution</h2>
                <p className="zcode-plan-desc">
                  The session's stored plan, progress, and real Git change counts.
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

          <div
            className="zcode-plan-progress-wrap"
            role="progressbar"
            aria-label={`Plan progress: ${progressPct}%`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={progressPct}
          >
            <div className="zcode-plan-progress-track">
              <div className="zcode-plan-progress-fill" style={{ width: `${progressPct}%` }}>
                <div className="plan-progress-shimmer-sheen" />
              </div>
            </div>
            <span className="zcode-plan-progress-pct">{progressPct}%</span>
          </div>
        </div>

        <div className="zcode-plan-grid">
          <div className="zcode-plan-col-tasks">
            {activeTodo ? (
              <div className="zcode-dashboard-card zcode-active-task-hero-card is-active-shimmer">
                <div className="zcode-card-header">
                  <div className="zcode-card-title">
                    <span className="zcode-pulse-dot" style={{ background: '#3b82f6' }} />
                    <span>CURRENT OBJECTIVE</span>
                  </div>
                  <span className="zcode-card-badge" style={{ color: '#60a5fa' }}>Active Step</span>
                </div>
                <div className="zcode-hero-body">
                  <div className="zcode-hero-title-row">
                    <ZCodeActiveArrow />
                    <div>
                      <div className="zcode-hero-title">{parseTodoTitle(activeTodo.text).main}</div>
                      {parseTodoTitle(activeTodo.text).subtitle && (
                        <div style={{ color: 'var(--fg-faint)', fontSize: '12px', marginTop: '2px' }}>
                          {parseTodoTitle(activeTodo.text).subtitle}
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              </div>
            ) : nextPendingTodo ? (
              <div className="zcode-dashboard-card" style={{ borderColor: 'rgba(255, 255, 255, 0.1)' }}>
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
                      <div className="zcode-hero-title">{parseTodoTitle(nextPendingTodo.text).main}</div>
                      {parseTodoTitle(nextPendingTodo.text).subtitle && (
                        <div style={{ color: 'var(--fg-faint)', fontSize: '12px', marginTop: '2px' }}>
                          {parseTodoTitle(nextPendingTodo.text).subtitle}
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              </div>
            ) : todos.length > 0 && completedCount === todos.length ? (
              <div className="zcode-dashboard-card" style={{ borderColor: 'rgba(34, 197, 94, 0.25)' }}>
                <div className="zcode-card-header">
                  <div className="zcode-card-title" style={{ color: '#4ade80' }}>
                    <ZCodeCheckRing />
                    <span>PLAN COMPLETED</span>
                  </div>
                  <span className="zcode-card-badge" style={{ color: '#4ade80' }}>100%</span>
                </div>
                <div className="zcode-hero-body">
                  <div style={{ color: 'var(--fg-muted)', fontSize: '13px' }}>
                    All {todos.length} stored objectives are completed.
                  </div>
                </div>
              </div>
            ) : null}

            <div className="zcode-dashboard-card zcode-tasks-breakdown-card">
              <div className="zcode-card-header">
                <div className="zcode-card-title">
                  <FileText size={15} style={{ color: '#71717a' }} />
                  <span>Tasks &amp; Execution Breakdown</span>
                </div>
                <span className="zcode-card-badge">
                  {completedCount} / {todos.length}
                </span>
              </div>

              <div style={{ padding: '14px 16px' }}>
                {completedCount > 0 && (
                  <div className="zcode-completed-group" style={{ marginBottom: '12px' }}>
                    <button
                      type="button"
                      className="zcode-completed-toggle-btn"
                      onClick={() => setShowCompleted(!showCompleted)}
                      title={showCompleted ? 'Collapse completed tasks' : 'Expand completed tasks'}
                    >
                      <span className={`zcode-toggle-chevron ${showCompleted ? 'is-open' : ''}`}>
                        <ChevronRight size={11} />
                      </span>
                      <span className="zcode-toggle-label">{completedCount} completed</span>
                    </button>
                    {showCompleted && (
                      <div className="zcode-completed-nested-list">
                        <TodoRows todos={completedTodos} completed />
                      </div>
                    )}
                  </div>
                )}

                {todos.length === 0 ? (
                  <div className="zcode-todos-empty-full">
                    <div style={{ fontSize: '13px', fontWeight: 500, color: 'var(--fg-default)' }}>
                      No todos yet
                    </div>
<div style={{ fontSize: '12px', color: 'var(--fg-faint)', marginTop: '4px', lineHeight: 1.5 }}>
                      The session's real todo list appears here when the agent reports it.
                    </div>
                  </div>
                ) : (
                  <div className="zcode-todos-list">
                    <TodoRows todos={activeAndPendingTodos} completed={false} />
                  </div>
                )}
              </div>
            </div>
          </div>

          <div className="zcode-plan-col-sidebar">
            <div className="zcode-dashboard-card zcode-git-card">
              <div className="zcode-card-header">
                <div className="zcode-card-title">
                  <GitBranch size={15} style={{ color: '#71717a' }} />
                  <span>Git Workspace</span>
                </div>
              </div>
              <div style={{ padding: '12px 16px', display: 'flex', flexDirection: 'column', gap: '8px' }}>
                <GitFacts gitStatus={gitStatus} />
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};