// Selora IDE — Phase 9a Agent Workbench (ROADMAP §"Phase 9", task 9a).
// The dedicated agent surface: backend session tabs, transcript reuse
// (AgentExecutionFeed's cards/thinking states), a live todo spine, structured
// question / plan review cards inside ToolCard, the composer's Ask / Agent /
// Plan row, and an explicit way back to the editor or split view.
//
// Every session and todo shown here comes from Rust (`bootstrap.chat`) or a
// real protocol event. New/select go through the real NewSession /
// SwitchSession commands; a rejected command shows its message and creates
// nothing locally.
import React, { useState, useSyncExternalStore } from 'react';
import { ChevronDown, Code2, Columns, ShieldCheck, Square } from '../components/icons';
import { agentStore, getComposerGate } from '../features/agent/agent-store';
import { AgentSessionTabs } from '../features/agent/components/AgentSessionTabs';
import { AgentTodoSpine } from '../features/agent/components/AgentTodoSpine';
import { AgentExecutionFeed } from '../features/agent/components/AgentExecutionFeed';
import { AgentPlanSpine } from '../features/agent/components/AgentPlanSpine';
import { AgentComposer } from '../features/agent/components/AgentComposer';
import { ThinkingOrb } from '../features/agent/presence/ThinkingOrb';
import { EmptyState } from '../components/EmptyState';
import { activeEditorContext, editorStore } from '../features/editor/editor-store';
import type { BootstrapState } from '../bridge/bridge';

const SUGGESTIONS = [
  'Explain this file',
  'Find a bug in the selection',
  'Summarize the workspace',
];

export type AgentDockProps = {
  bootstrap?: BootstrapState;
  /** Leave the dedicated agent view for the editor or split shell. */
  onExit?: (mode: 'editor' | 'split') => void;
};

export function AgentDock({ bootstrap, onExit }: AgentDockProps = {}) {
  const state = useSyncExternalStore(
    agentStore.subscribe,
    agentStore.getSnapshot
  );
  const editor = useSyncExternalStore(
    editorStore.subscribe,
    editorStore.getSnapshot
  );

  const [activeTab, setActiveTab] = useState<'feed' | 'plan'>('feed');
  const [showFloatingPlan, setShowFloatingPlan] = useState(false);
  const [infeedPlanExpanded, setInfeedPlanExpanded] = useState(false);

  const activeSession =
    state.sessions.find((session) => session.id === state.activeSessionId) ?? null;

  const hasMessages = state.messages.length > 0;
  const gate = getComposerGate(state);
  const autoApplyArmed = state.permissionMode === 'full';
  const activeSessionTabId = activeSession
    ? `agent-session-tab-${activeSession.id}`
    : undefined;

  const exitWorkbench = (mode: 'editor' | 'split') => {
    if (onExit) {
      onExit(mode);
      return;
    }
    // The shell listens for this when AgentDock is rendered outside ShellGrid
    // (tests, isolated previews). One event, no local layout fiction.
    if (typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent('selora:set-view-mode', { detail: { mode } }),
      );
    }
  };

  const suggestions = SUGGESTIONS.filter((suggestion) => {
    if (suggestion === 'Explain this file') return editor.activeBuffer !== null;
    if (suggestion === 'Find a bug in the selection') return editor.activeSelection !== null;
    return true;
  });

  return (
    <div className="agent-dock agent-workbench-root" role="region" aria-label="Agent workbench">
      {/* 1. Workbench header: presence, active session badge, view actions. */}
      <header className="agent-header agent-workbench-header">
        <div className="header-left-group">
          <span className="agent-presence-slot" aria-hidden="true">
            <ThinkingOrb state={state.thinkingState} size={20} />
          </span>

          <span className="agent-session-label">Agent</span>

          <span
            className="agent-session-badge"
            role="status"
            title={
              activeSession
                ? `Active session: ${activeSession.title}`
                : 'The backend has not reported an agent session yet'
            }
          >
            <span
              className={`agent-session-badge-dot${state.inFlight ? ' is-live' : ''}`}
              aria-hidden="true"
            />
            <span className="agent-session-badge-title">
              {activeSession ? activeSession.title : 'No session'}
            </span>
          </span>
        </div>

        <div className="header-right-group">
          {autoApplyArmed && (
            <span
              className="agent-auto-apply-badge"
              role="status"
              title="Full auto-apply is active: file writes, edits and deletes apply immediately for this session. Bash commands still require approval."
            >
              <ShieldCheck size={11} aria-hidden="true" />
              Auto-apply armed
            </span>
          )}

          <button
            type="button"
            className={`agent-view-tab ${activeTab === 'feed' ? 'is-active' : ''}`}
            onClick={() => setActiveTab('feed')}
            aria-pressed={activeTab === 'feed'}
          >
            Chat
          </button>
          <button
            type="button"
            className={`agent-view-tab ${activeTab === 'plan' ? 'is-active' : ''}`}
            onClick={() => setActiveTab('plan')}
            aria-pressed={activeTab === 'plan'}
          >
            Plan ({state.todos.filter((t) => t.status === 'completed').length}/
            {state.todos.length})
          </button>

          {activeTab === 'feed' && (
            <button
              type="button"
              className={`agent-floating-plan-toggle ${showFloatingPlan ? 'is-active' : ''}`}
              onClick={() => setShowFloatingPlan(!showFloatingPlan)}
              title={showFloatingPlan ? 'Hide floating plan card' : 'Show floating plan card'}
              aria-label="Toggle floating plan"
              aria-pressed={showFloatingPlan}
            >
              <svg
                width="13"
                height="13"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                aria-hidden="true"
                focusable="false"
              >
                <rect x="2" y="2" width="12" height="12" rx="2" />
                <path d="M5 8h6M5 5h6M5 11h4" />
              </svg>
            </button>
          )}

          <button
            type="button"
            className="agent-header-stop"
            onClick={() => agentStore.stopActiveTurn()}
            disabled={!state.inFlight}
            title={state.inFlight ? 'Stop the running agent turn (Esc)' : 'No agent turn is running'}
            aria-label="Stop agent run"
          >
            <Square size={11} aria-hidden="true" />
            <span>Stop</span>
          </button>

          <div className="agent-exit-group" role="group" aria-label="Switch workspace view">
            <button
              type="button"
              className="agent-exit-btn"
              onClick={() => exitWorkbench('editor')}
              title="Return to the editor view"
              aria-label="Editor view"
            >
              <Code2 size={12} aria-hidden="true" />
              <span>Editor</span>
            </button>
            <button
              type="button"
              className="agent-exit-btn"
              onClick={() => exitWorkbench('split')}
              title="Show the agent beside the editor"
              aria-label="Split view"
            >
              <Columns size={12} aria-hidden="true" />
              <span>Split</span>
            </button>
          </div>
        </div>
      </header>

      {/* 2. Backend session tabs (new / select via real protocol commands). */}
      <AgentSessionTabs
        sessions={state.sessions}
        activeSessionId={state.activeSessionId}
        busy={state.sessionBusy}
        error={state.sessionError}
        onNew={() => void agentStore.requestNewSession()}
        onSelect={(sessionId) => void agentStore.requestSwitchSession(sessionId)}
      />

      {/* 3. Workbench body */}
      <div
        className="agent-workbench-body"
        id="agent-session-panel"
        role="tabpanel"
        aria-labelledby={activeSessionTabId}
      >
        {activeTab === 'feed' ? (
          <div className="agent-feed-column">
            {showFloatingPlan && (
              <div className="zcode-floating-plan-popover">
                <AgentPlanSpine
                  todos={state.todos}
                  gitStatus={bootstrap?.git}
                  isFloatingPopover={true}
                  onClosePopover={() => setShowFloatingPlan(false)}
                />
              </div>
            )}

            {/* The live spine is always present; an absent list says so. */}
            <AgentTodoSpine todos={state.todos} inFlight={state.inFlight} />

            {/* Quick in-feed progress card whenever todos are present. */}
            {state.todos.length > 0 && (
              <div className="zcode-infeed-progress-wrap">
                <button
                  type="button"
                  className="zcode-infeed-progress-toggle"
                  onClick={() => setInfeedPlanExpanded(!infeedPlanExpanded)}
                  aria-expanded={infeedPlanExpanded}
                  aria-controls="zcode-infeed-progress-body"
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
                </button>
                {infeedPlanExpanded && (
                  <div id="zcode-infeed-progress-body" className="zcode-infeed-expanded-body">
                    <AgentPlanSpine
                      todos={state.todos}
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
                    <em>Ask about the code, or tell Selora what to change.</em>
                  }
                >
                  <div className="agent-suggestions" role="group" aria-label="Suggested prompts">
                    {suggestions.map((suggestion) => (
                      <button
                        key={suggestion}
                        type="button"
                        className="agent-chip"
                        disabled={!gate.ready}
                        title={gate.ready ? undefined : gate.message ?? undefined}
                        onClick={() => {
                          const context = activeEditorContext();
                          if (context) agentStore.sendUserMessage(suggestion, false, context);
                          else agentStore.sendUserMessage(suggestion);
                        }}
                      >
                        {suggestion}
                      </button>
                    ))}
                  </div>
                </EmptyState>
              </div>
            )}

            <AgentComposer
              inFlight={state.inFlight}
              streamingActivityLevel={state.streamingActivityLevel}
              permissionMode={state.permissionMode}
              reasoningEffort={state.reasoningEffort}
              promptHistory={state.promptHistory}
            />
          </div>
        ) : (
<div className="agent-feed-column">
            <div className="agent-plan-scroll">
              <AgentPlanSpine
                todos={state.todos}
                gitStatus={bootstrap?.git}
              />
            </div>

            <AgentComposer
              inFlight={state.inFlight}
              streamingActivityLevel={state.streamingActivityLevel}
              permissionMode={state.permissionMode}
              reasoningEffort={state.reasoningEffort}
              promptHistory={state.promptHistory}
            />
          </div>
        )}
      </div>
    </div>
  );
}