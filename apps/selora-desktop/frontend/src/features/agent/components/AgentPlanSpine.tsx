// Selora IDE — ZCode Parity Plan & Progress Card (UI_SPEC §4.2, REVAMP_SPEC §8)
//
// Matches ZCode's ultra-clean Git tools and collapsible Progress card.
// - Zero strikethrough slop: completed tasks use elegant green ring checkmarks with readable muted typography.
// - Active in-progress task highlighted with leading '→' arrow and moonlight-white text.
// - Collapsible '< X completed' accordion to keep the plan hyper-focused.
// - Git tools panel with branch selector, changes diff (+7632 -125), and commit action.
// - Full Subagent swarm roster and Artifacts tray.
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
  Plus,
  CheckCircle,
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

// 1. ZCode Green Ring Checkmark SVG
export function ZCodeCheckRing() {
  return (
    <svg
      width="15"
      height="15"
      viewBox="0 0 16 16"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className="zcode-ring-check-svg"
      style={{ flexShrink: 0, marginTop: '2px' }}
      aria-hidden="true"
    >
      <circle cx="8" cy="8" r="6.5" stroke="#22c55e" strokeWidth="1.2" />
      <path
        d="M5 8.2L7 10.2L11 5.8"
        stroke="#22c55e"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

// 2. ZCode Active Task Arrow SVG
export function ZCodeActiveArrow() {
  return (
    <svg
      width="15"
      height="15"
      viewBox="0 0 16 16"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className="zcode-active-arrow-svg"
      style={{ flexShrink: 0, marginTop: '2px' }}
      aria-hidden="true"
    >
      <path
        d="M3 8H13M13 8L9 4M13 8L9 12"
        stroke="#f4f4f5"
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
      width="15"
      height="15"
      viewBox="0 0 16 16"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className="zcode-pending-ring-svg"
      style={{ flexShrink: 0, marginTop: '2px' }}
      aria-hidden="true"
    >
      <circle cx="8" cy="8" r="6.5" stroke="#52525b" strokeWidth="1.2" />
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
  const [isAdding, setIsAdding] = useState(false);
  const [newStepText, setNewStepText] = useState('');

  const completedTodos = todos.filter((t) => t.status === 'completed');
  const activeAndPendingTodos = todos.filter((t) => t.status !== 'completed');
  const activeTodo = todos.find((t) => t.status === 'in_progress');
  const nextPendingTodo = todos.find((t) => t.status === 'pending');
  const completedCount = completedTodos.length;
  const progressPct = todos.length > 0 ? Math.round((completedCount / todos.length) * 100) : 0;

  const branchName = gitStatus?.branch || 'main';
  const changedFilesCount = gitStatus?.changed_paths?.length || 0;

  const handleAddStep = () => {
    const trimmed = newStepText.trim();
    if (trimmed) {
      agentStore.addTodo(trimmed);
      setNewStepText('');
      setIsAdding(false);
    }
  };

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

            <div className="zcode-git-row zcode-commit-row" role="button" tabIndex={0}>
              <div className="zcode-git-row-label">
                <span className="zcode-commit-bullet">⊶</span>
                <span className="zcode-commit-text">Commit or push</span>
              </div>
            </div>
          </div>

          <div className="zcode-card-divider" />

          {/* 2. ZCode Progress Section */}
          <div className="zcode-card-section zcode-progress-section">
            <div className="zcode-progress-header">
              <span className="zcode-progress-label">Progress</span>
              <span className="zcode-progress-count">
                {completedCount}/{todos.length}
              </span>
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
                        <div
                          key={todo.id}
                          className="zcode-todo-row is-completed"
                          onClick={() => agentStore.updateTodoStatus(todo.id, 'in_progress')}
                          role="button"
                          tabIndex={0}
                        >
                          <ZCodeCheckRing />
                          <div className="zcode-todo-body">
                            <span className="zcode-todo-text">{parsed.main}</span>
                            {parsed.subtitle && <span className="zcode-todo-sub">{parsed.subtitle}</span>}
                          </div>
                        </div>
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
                <div>No active tasks yet.</div>
                <div style={{ fontSize: '11px', opacity: 0.75, marginTop: '4px' }}>
                  Prompt the agent to begin work — tasks and progress will be created and updated here dynamically.
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
                      className={`zcode-todo-row ${isActive ? 'is-active' : 'is-pending'}`}
                      onClick={() =>
                        agentStore.updateTodoStatus(
                          todo.id,
                          isActive ? 'completed' : 'in_progress'
                        )
                      }
                      role="button"
                      tabIndex={0}
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
                    </div>
                  );
                })}
              </div>
            )}

            {/* Interactive Add Step */}
            <div className="zcode-add-step-container" style={{ marginTop: '10px', paddingTop: '6px', borderTop: '1px solid rgba(255, 255, 255, 0.05)' }}>
              {isAdding ? (
                <div style={{ display: 'flex', gap: '6px', alignItems: 'center' }}>
                  <input
                    type="text"
                    autoFocus
                    value={newStepText}
                    onChange={(e) => setNewStepText(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') handleAddStep();
                      if (e.key === 'Escape') {
                        setIsAdding(false);
                        setNewStepText('');
                      }
                    }}
                    placeholder="Add task step..."
                    style={{
                      flex: 1,
                      background: 'rgba(255, 255, 255, 0.06)',
                      border: '1px solid rgba(255, 255, 255, 0.12)',
                      borderRadius: '4px',
                      color: 'var(--fg-normal)',
                      fontSize: '11px',
                      padding: '4px 8px',
                      outline: 'none',
                    }}
                  />
                  <button
                    type="button"
                    onClick={handleAddStep}
                    style={{
                      background: 'var(--accent, #3b82f6)',
                      border: 'none',
                      borderRadius: '4px',
                      color: '#fff',
                      fontSize: '11px',
                      padding: '4px 10px',
                      cursor: 'pointer',
                    }}
                  >
                    Add
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setIsAdding(false);
                      setNewStepText('');
                    }}
                    style={{
                      background: 'transparent',
                      border: 'none',
                      color: 'var(--fg-faint)',
                      fontSize: '11px',
                      cursor: 'pointer',
                      padding: '4px 6px',
                    }}
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <button
                  type="button"
                  className="zcode-add-step-btn"
                  onClick={() => setIsAdding(true)}
                  style={{
                    background: 'transparent',
                    border: 'none',
                    color: 'var(--fg-faint)',
                    fontSize: '11px',
                    cursor: 'pointer',
                    display: 'flex',
                    alignItems: 'center',
                    gap: '4px',
                    padding: '2px 0',
                  }}
                >
                  <span>+ Add step</span>
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    );
  }

  // --- Full View (Accessed via Plan Tab in AgentDock) ---
  return (
    <div className="agent-plan-spine is-full-view" aria-label="Agent Plan & Subagents">
      <div className="zcode-plan-workspace">
        {/* 1. Overview Banner */}
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

          <div className="zcode-plan-progress-wrap">
            <div className="zcode-plan-progress-track">
              <div
                className="zcode-plan-progress-fill"
                style={{ width: `${progressPct}%` }}
              />
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
              <div className="zcode-dashboard-card zcode-active-task-hero-card">
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
                    All {todos.length} planned objectives have been successfully executed and verified.
                  </div>
                </div>
              </div>
            ) : null}

            {/* Tasks Breakdown Card */}
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
                            <div
                              key={todo.id}
                              className="zcode-todo-row is-completed"
                              onClick={() => agentStore.updateTodoStatus(todo.id, 'in_progress')}
                              role="button"
                              tabIndex={0}
                            >
                              <ZCodeCheckRing />
                              <div className="zcode-todo-body">
                                <span className="zcode-todo-text">{parsed.main}</span>
                                {parsed.subtitle && <span className="zcode-todo-sub">{parsed.subtitle}</span>}
                              </div>
                              <button
                                type="button"
                                className="zcode-todo-remove-btn"
                                title="Remove task"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  agentStore.removeTodo(todo.id);
                                }}
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
                    <div style={{ fontSize: '13px', fontWeight: 500, color: '#e4e4e7' }}>
                      No execution tasks currently planned
                    </div>
                    <div style={{ fontSize: '12px', color: '#71717a', marginTop: '4px', lineHeight: 1.5 }}>
                      When the agent tackles a task, structured steps will appear here in real time.
                      You can also initialize a plan using starter templates or add custom steps below.
                    </div>
                    <div className="zcode-starter-templates">
                      <button
                        type="button"
                        className="zcode-starter-btn"
                        onClick={() => {
                          agentStore.addTodo('1. Audit architecture and component boundaries', 'frontend-developer');
                          agentStore.addTodo('2. Implement features with full test coverage', 'frontend-developer');
                          agentStore.addTodo('3. Verify edge cases and performance', 'rust-developer');
                        }}
                      >
                        <span>+ Load Full Development Plan template</span>
                        <span style={{ fontSize: '11px', color: '#71717a' }}>3 steps</span>
                      </button>
                      <button
                        type="button"
                        className="zcode-starter-btn"
                        onClick={() => {
                          agentStore.addTodo('1. Reproduce issue and inspect logs', 'frontend-developer');
                          agentStore.addTodo('2. Implement regression fix', 'frontend-developer');
                          agentStore.addTodo('3. Run unit tests and verify fix', 'rust-developer');
                        }}
                      >
                        <span>+ Load Bug Investigation &amp; Fix template</span>
                        <span style={{ fontSize: '11px', color: '#71717a' }}>3 steps</span>
                      </button>
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
                          className={`zcode-todo-row ${isActive ? 'is-active' : 'is-pending'}`}
                          onClick={() =>
                            agentStore.updateTodoStatus(
                              todo.id,
                              isActive ? 'completed' : 'in_progress'
                            )
                          }
                          role="button"
                          tabIndex={0}
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
                          <button
                            type="button"
                            className="zcode-todo-remove-btn"
                            title="Remove task"
                            onClick={(e) => {
                              e.stopPropagation();
                              agentStore.removeTodo(todo.id);
                            }}
                          >
                            ×
                          </button>
                        </div>
                      );
                    })}
                  </div>
                )}

                {/* Interactive Add Step */}
                <div className="zcode-add-step-container" style={{ marginTop: '14px', paddingTop: '10px', borderTop: '1px solid rgba(255, 255, 255, 0.05)' }}>
                  {isAdding ? (
                    <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                      <input
                        type="text"
                        autoFocus
                        value={newStepText}
                        onChange={(e) => setNewStepText(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') handleAddStep();
                          if (e.key === 'Escape') {
                            setIsAdding(false);
                            setNewStepText('');
                          }
                        }}
                        placeholder="Add task step (e.g. Verify test suite)..."
                        style={{
                          flex: 1,
                          background: 'rgba(255, 255, 255, 0.06)',
                          border: '1px solid rgba(255, 255, 255, 0.12)',
                          borderRadius: '6px',
                          color: 'var(--fg-normal)',
                          fontSize: '12px',
                          padding: '6px 10px',
                          outline: 'none',
                        }}
                      />
                      <button
                        type="button"
                        onClick={handleAddStep}
                        className="zcode-btn-sm primary"
                      >
                        Add
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          setIsAdding(false);
                          setNewStepText('');
                        }}
                        className="zcode-btn-sm"
                      >
                        Cancel
                      </button>
                    </div>
                  ) : (
                    <button
                      type="button"
                      className="zcode-add-step-btn"
                      onClick={() => setIsAdding(true)}
                      style={{
                        background: 'transparent',
                        border: 'none',
                        color: 'var(--fg-faint)',
                        fontSize: '12px',
                        cursor: 'pointer',
                        display: 'flex',
                        alignItems: 'center',
                        gap: '6px',
                        padding: '2px 0',
                      }}
                    >
                      <Plus size={14} />
                      <span>Add step</span>
                    </button>
                  )}
                </div>
              </div>
            </div>
          </div>

          {/* Column 2: Git Workspace & Subagents Sidebar */}
          <div className="zcode-plan-col-sidebar">
            {/* Git Workspace Card */}
            <div className="zcode-dashboard-card zcode-git-card">
              <div className="zcode-card-header">
                <div className="zcode-card-title">
                  <GitBranch size={15} style={{ color: '#71717a' }} />
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

                <div className="zcode-git-row zcode-commit-row" role="button" tabIndex={0}>
                  <div className="zcode-git-row-label">
                    <span className="zcode-commit-bullet">⊶</span>
                    <span className="zcode-commit-text">Commit or push</span>
                  </div>
                </div>
              </div>
            </div>

            {/* Subagents Swarm Roster */}
            <div className="zcode-dashboard-card zcode-swarm-card">
              <div className="zcode-card-header">
                <div className="zcode-card-title">
                  <Sparkles size={15} style={{ color: '#a855f7' }} />
                  <span>Subagents Swarm</span>
                </div>
                <span className="zcode-card-badge">
                  {subagents.length} active
                </span>
              </div>

              <div className="swarm-roster-full">
                {subagents.map((sub) => (
                  <div
                    key={sub.id}
                    className={`swarm-card-item status-${sub.status}`}
                    title={`${sub.name} (${sub.model}): ${sub.activeStep || 'Idle'}`}
                  >
                    <BotAvatar
                      role={sub.name}
                      status={sub.status === 'working' ? 'working' : sub.status === 'idle' ? 'sleeping' : 'default'}
                      size={24}
                      name={sub.name}
                    />
                    <div className="swarm-info">
                      <div className="swarm-header-row">
                        <span className="subagent-name">{sub.name}</span>
                        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                          <span className="zcode-card-badge" style={{ fontSize: '10px' }}>
                            {sub.model || 'Auto'}
                          </span>
                          <span className={`zcode-subagent-status-badge status-${sub.status}`}>
                            {sub.status === 'working' && <span className="zcode-pulse-dot" style={{ width: '4px', height: '4px' }} />}
                            {sub.status}
                          </span>
                        </div>
                      </div>
                      <span className="subagent-active-step">
                        {sub.activeStep || 'Ready for instructions'}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            </div>

            {/* Artifacts Drawer */}
            <div className="zcode-dashboard-card zcode-artifacts-card">
              <div className="zcode-card-header">
                <div className="zcode-card-title">
                  <Code2 size={15} style={{ color: '#71717a' }} />
                  <span>Artifacts</span>
                </div>
                <span className="zcode-card-badge">
                  {artifacts.length}
                </span>
              </div>

              <div style={{ padding: '12px 16px' }}>
                {artifacts.length === 0 ? (
                  <div style={{ fontSize: '12px', color: '#71717a', textAlign: 'center', padding: '12px 0' }}>
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
