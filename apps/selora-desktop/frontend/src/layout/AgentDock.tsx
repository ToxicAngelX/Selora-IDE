// Selora IDE — Phase Z3 Agent Dock / Agentic Window (DESIGN §3.5, REVAMP_SPEC §2/§8, ZCode benchmark)
//
// The flagship agentic surface: multi-session rail, execution feed with tool cards,
// plan spine with live todos, subagents tracking, and hero composer card.
import React, { useState, useSyncExternalStore } from 'react';
import { ChevronDown } from '../components/icons';
import { agentStore } from '../features/agent/agent-store';
import { AgentSessionRail } from '../features/agent/components/AgentSessionRail';
import { AgentExecutionFeed } from '../features/agent/components/AgentExecutionFeed';
import { AgentPlanSpine } from '../features/agent/components/AgentPlanSpine';
import { AgentComposer } from '../features/agent/components/AgentComposer';
import { ThinkingOrb } from '../features/agent/presence/ThinkingOrb';
import { EmptyState } from '../components/EmptyState';
import type { BootstrapState } from '../bridge/bridge';

const SUGGESTIONS = [
  'Explain this file',
  'Find a bug in the selection',
  'Summarize the workspace',
];

export type AgentDockProps = {
  bootstrap?: BootstrapState;
};

export function AgentDock({ bootstrap }: AgentDockProps = {}) {
  const state = useSyncExternalStore(
    agentStore.subscribe,
    agentStore.getSnapshot
  );

  const [activeTab, setActiveTab] = useState<'feed' | 'plan'>('feed');
  const [showSessionRail, setShowSessionRail] = useState(false);
  const [showFloatingPlan, setShowFloatingPlan] = useState(false);
  const [infeedPlanExpanded, setInfeedPlanExpanded] = useState(false);

  const activeSession =
    state.sessions.find((s) => s.id === state.activeSessionId) ||
    state.sessions[0];

  const hasMessages = state.messages.length > 0;

  return (
    <div className="agent-dock agent-workbench-root" aria-label="Agentic Workbench">
      {/* 1. Header Bar */}
      <header className="agent-header agent-workbench-header">
        <div className="header-left-group">
          {/* Presence Orb slot */}
          <span className="agent-presence-slot" aria-hidden="true">
            <ThinkingOrb state={state.thinkingState} size={20} />
            <span className="agent-presence-dot" style={{ display: 'none' }} />
          </span>

          <span className="agent-session-label">Agent</span>

          {/* Session Switcher toggle */}
          <button
            type="button"
            className="header-session-title-btn"
            onClick={() => setShowSessionRail(!showSessionRail)}
            title="Toggle session history drawer"
            style={{
              background: 'transparent',
              border: 'none',
              padding: '0 6px',
              cursor: 'pointer',
              display: 'flex',
              alignItems: 'center',
              gap: '4px',
            }}
          >
            <span className="header-session-title">
              {activeSession ? activeSession.title : 'New Session'}
            </span>
            <ChevronDown
              size={12}
              style={{
                color: 'var(--fg-faint)',
                transform: showSessionRail ? 'rotate(180deg)' : 'none',
                transition: 'transform 150ms ease',
              }}
            />
          </button>
        </div>

        <div className="header-right-group">
          {/* View Mode switcher (Chat vs Plan) */}
          <button
            type="button"
            className={`agent-view-tab ${activeTab === 'feed' ? 'is-active' : ''}`}
            onClick={() => setActiveTab('feed')}
          >
            Chat
          </button>
          <button
            type="button"
            className={`agent-view-tab ${activeTab === 'plan' ? 'is-active' : ''}`}
            onClick={() => setActiveTab('plan')}
          >
            Plan ({state.todos.filter((t) => t.status === 'completed').length}/
            {state.todos.length})
          </button>

          {/* Quick Floating Plan Toggle for ZCode parity */}
          {activeTab === 'feed' && (
            <button
              type="button"
              className={`agent-floating-plan-toggle ${showFloatingPlan ? 'is-active' : ''}`}
              onClick={() => setShowFloatingPlan(!showFloatingPlan)}
              title={showFloatingPlan ? 'Hide floating plan card' : 'Show floating plan card'}
              aria-label="Toggle floating plan"
            >
              <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
                <rect x="2" y="2" width="12" height="12" rx="2" />
                <path d="M5 8h6M5 5h6M5 11h4" />
              </svg>
            </button>
          )}
        </div>
      </header>

      {/* 2. Main Workbench Body */}
      <div className="agent-workbench-body">
        {/* Optional Collapsible Session Drawer Overlay */}
        {showSessionRail && (
          <div className="agent-session-drawer-overlay">
            <AgentSessionRail
              sessions={state.sessions}
              activeSessionId={state.activeSessionId}
              onClose={() => setShowSessionRail(false)}
            />
            <div
              className="agent-session-drawer-backdrop"
              onClick={() => setShowSessionRail(false)}
              aria-hidden="true"
            />
          </div>
        )}

        {/* Center: Execution Feed or Empty State */}
        {activeTab === 'feed' ? (
          <div
            style={{
              position: 'relative',
              flex: 1,
              minWidth: 0,
              display: 'flex',
              flexDirection: 'column',
              overflow: 'hidden',
            }}
          >
            {showFloatingPlan && (
              <div className="zcode-floating-plan-popover">
                <AgentPlanSpine
                  todos={state.todos}
                  subagents={state.subagents}
                  artifacts={state.artifacts}
                  gitStatus={bootstrap?.git}
                  isFloatingPopover={true}
                  onClosePopover={() => setShowFloatingPlan(false)}
                />
              </div>
            )}

            {/* Quick in-feed progress card whenever todos are present */}
            {state.todos.length > 0 && (
              <div className="zcode-infeed-progress-wrap">
                <div
                  className="zcode-infeed-progress-toggle"
                  onClick={() => setInfeedPlanExpanded(!infeedPlanExpanded)}
                  role="button"
                  tabIndex={0}
                >
                  <div className="infeed-left">
                    <span className="infeed-label">Plan &amp; Progress</span>
                    <span className="infeed-pill">
                      {state.todos.filter((t) => t.status === 'completed').length}/{state.todos.length}
                    </span>
                  </div>
                  <div className="infeed-right">
                    <span className="infeed-action">
                      {infeedPlanExpanded ? 'Collapse' : 'View tasks'}
                    </span>
                    <ChevronDown
                      size={12}
                      style={{
                        transform: infeedPlanExpanded ? 'rotate(180deg)' : 'none',
                        transition: 'transform 150ms ease',
                      }}
                    />
                  </div>
                </div>
                {infeedPlanExpanded && (
                  <div className="zcode-infeed-expanded-body">
                    <AgentPlanSpine
                      todos={state.todos}
                      subagents={state.subagents}
                      artifacts={state.artifacts}
                      gitStatus={bootstrap?.git}
                      isCompactInfeed={true}
                    />
                  </div>
                )}
              </div>
            )}

            {hasMessages ? (
              <AgentExecutionFeed
                messages={state.messages}
                inFlight={state.inFlight}
                thinkingState={state.thinkingState}
                metrics={state.metrics}
                promptQueue={state.promptQueue}
              />
            ) : (
              <div className="agent-body">
                <EmptyState
                  heading="Ask anything"
                  hint={
                    <>
                      <em>Ask about the code, or tell Selora what to change.</em>
                      <span className="agent-status-line">Agent conversations arrive in a later phase.</span>
                    </>
                  }
                >
                  <div className="agent-suggestions" aria-label="Suggested prompts">
                    {SUGGESTIONS.map((suggestion) => (
                      <button
                        key={suggestion}
                        type="button"
                        className="agent-chip"
                        disabled
                        onClick={() => agentStore.sendUserMessage(suggestion)}
                        title="The composer arrives in a later slice"
                      >
                        {suggestion}
                      </button>
                    ))}
                  </div>
                </EmptyState>
              </div>
            )}

            {/* Sacred Composer Card */}
            <AgentComposer
              inFlight={state.inFlight}
              streamingActivityLevel={state.streamingActivityLevel}
              permissionMode={state.permissionMode}
              reasoningEffort={state.reasoningEffort}
              selectedModel={state.selectedModel}
              promptHistory={state.promptHistory}
            />
          </div>
        ) : (
          /* Plan & Subagents Tab */
          <div
            style={{
              position: 'relative',
              flex: 1,
              minWidth: 0,
              display: 'flex',
              flexDirection: 'column',
              overflow: 'hidden',
            }}
          >
            <div style={{ flex: 1, minWidth: 0, overflowY: 'auto' }}>
              <AgentPlanSpine
                todos={state.todos}
                subagents={state.subagents}
                artifacts={state.artifacts}
                gitStatus={bootstrap?.git}
              />
            </div>

            {/* Sacred Composer Card also available in Plan view */}
            <AgentComposer
              inFlight={state.inFlight}
              streamingActivityLevel={state.streamingActivityLevel}
              permissionMode={state.permissionMode}
              reasoningEffort={state.reasoningEffort}
              selectedModel={state.selectedModel}
              promptHistory={state.promptHistory}
            />
          </div>
        )}
      </div>
    </div>
  );
}
