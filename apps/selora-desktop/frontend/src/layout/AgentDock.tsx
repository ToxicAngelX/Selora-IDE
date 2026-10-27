// Selora IDE — Phase Z3 Agent Dock / Agentic Window (DESIGN §3.5, REVAMP_SPEC §2/§8, ZCode benchmark)
//
// The flagship agentic surface: multi-session rail, execution feed with tool cards,
// plan spine with live todos, subagents tracking, and hero composer card.
import React, { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { ChevronDown, ShieldCheck } from '../components/icons';
import { agentStore, getComposerGate } from '../features/agent/agent-store';
import { AgentSessionRail } from '../features/agent/components/AgentSessionRail';
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
};

export function AgentDock({ bootstrap }: AgentDockProps = {}) {
  const state = useSyncExternalStore(
    agentStore.subscribe,
    agentStore.getSnapshot
  );
  const editor = useSyncExternalStore(
    editorStore.subscribe,
    editorStore.getSnapshot
  );

  const [activeTab, setActiveTab] = useState<'feed' | 'plan'>('feed');
  const [showSessionRail, setShowSessionRail] = useState(false);
  const [showFloatingPlan, setShowFloatingPlan] = useState(false);
  const [infeedPlanExpanded, setInfeedPlanExpanded] = useState(false);
  const sessionButtonRef = useRef<HTMLButtonElement>(null);
  const sessionDrawerRef = useRef<HTMLDivElement>(null);

  const closeSessionRail = () => {
    setShowSessionRail(false);
    sessionButtonRef.current?.focus();
  };

  useEffect(() => {
    if (!showSessionRail) return;
    const drawer = sessionDrawerRef.current;
    drawer
      ?.querySelector<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      )
      ?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeSessionRail();
        return;
      }
      if (event.key !== 'Tab') return;
      const focusables = Array.from(
        drawer?.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      );
      if (focusables.length === 0) {
        event.preventDefault();
        drawer?.focus();
        return;
      }
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (!drawer?.contains(document.activeElement)) {
        event.preventDefault();
        first.focus();
      } else if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [showSessionRail]);

  const activeSession =
    state.sessions.find((s) => s.id === state.activeSessionId) ||
    state.sessions[0];

  const hasMessages = state.messages.length > 0;
  const gate = getComposerGate(state);
  const autoApplyArmed = state.permissionMode === 'full';
  const revealedSubagent = state.revealedSubagentId
    ? state.subagents.find((subagent) => subagent.id === state.revealedSubagentId)
    : undefined;

  useEffect(() => {
    if (state.revealedSubagentId) setActiveTab('plan');
  }, [state.revealedSubagentId]);

  const suggestions = SUGGESTIONS.filter((suggestion) => {
    if (suggestion === 'Explain this file') return editor.activeBuffer !== null;
    if (suggestion === 'Find a bug in the selection') return editor.activeSelection !== null;
    return true;
  });

  return (
    <div className="agent-dock agent-workbench-root" role="region" aria-label="Agentic Workbench">
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
            ref={sessionButtonRef}
            type="button"
            className="header-session-title-btn"
            onClick={() => setShowSessionRail(!showSessionRail)}
            title="Toggle session history drawer"
            aria-expanded={showSessionRail}
            aria-controls={showSessionRail ? 'agent-session-drawer' : undefined}
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
          {/* Armed auto-apply indicator: session writes land without cards */}
          {autoApplyArmed && (
            <span
              className="agent-auto-apply-badge"
              role="status"
              title="Full auto-apply is active: file writes, edits and deletes apply immediately for this session. Bash commands still require approval."
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 4,
                padding: '2px 7px',
                marginRight: 4,
                fontSize: 10,
                fontWeight: 600,
                letterSpacing: '0.02em',
                color: '#34d399',
                background: 'rgba(16, 185, 129, 0.12)',
                border: '1px solid rgba(16, 185, 129, 0.35)',
                borderRadius: 4,
              }}
            >
              <ShieldCheck size={11} aria-hidden="true" />
              Auto-apply armed
            </span>
          )}

          {/* View Mode switcher (Chat vs Plan) */}
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

          {/* Quick Floating Plan Toggle for ZCode parity */}
          {activeTab === 'feed' && (
            <button
              type="button"
              className={`agent-floating-plan-toggle ${showFloatingPlan ? 'is-active' : ''}`}
              onClick={() => setShowFloatingPlan(!showFloatingPlan)}
              title={showFloatingPlan ? 'Hide floating plan card' : 'Show floating plan card'}
              aria-label="Toggle floating plan"
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
        </div>
      </header>

      {/* Phase 9e: notification click reveals the child result here, under
          the selected parent session. */}
      {revealedSubagent ? (
        <section className="agent-subagent-result" data-testid="subagent-result" aria-label="Subagent result">
          <div className="agent-subagent-result-head">
            <span className="agent-subagent-result-title">
              {revealedSubagent.name}
              <span className="agent-subagent-policy">{revealedSubagent.policy ?? 'read_only'}</span>
            </span>
            <button
              type="button"
              className="agent-subagent-result-close"
              aria-label="Dismiss subagent result"
              onClick={() => agentStore.clearSubagentReveal()}
            >
              Close
            </button>
          </div>
          <p className="agent-subagent-result-body">
            {revealedSubagent.status === 'failed'
              ? revealedSubagent.error ?? 'Subagent failed without a message.'
              : revealedSubagent.result || 'Subagent finished without a final message.'}
          </p>
          {revealedSubagent.artifacts && revealedSubagent.artifacts.length > 0 ? (
            <ul className="agent-subagent-artifacts">
              {revealedSubagent.artifacts.map((artifact) => (
                <li key={`${artifact.kind}:${artifact.path}`}>
                  <span>{artifact.kind}</span> {artifact.path}
                </li>
              ))}
            </ul>
          ) : null}
        </section>
      ) : null}

      {/* 2. Main Workbench Body */}
      <div className="agent-workbench-body">
        {/* Optional Collapsible Session Drawer Overlay */}
        {showSessionRail && (
          <div
            ref={sessionDrawerRef}
            id="agent-session-drawer"
            className="agent-session-drawer-overlay"
            role="dialog"
            aria-modal="true"
            aria-labelledby="agent-session-drawer-title"
            tabIndex={-1}
          >
            <h2 id="agent-session-drawer-title" className="visually-hidden">Agent sessions</h2>
            <AgentSessionRail
              sessions={state.sessions}
              activeSessionId={state.activeSessionId}
              onClose={closeSessionRail}
            />
            <div
              className="agent-session-drawer-backdrop"
              onClick={closeSessionRail}
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

            {/* Sacred Composer Card */}
            <AgentComposer
              inFlight={state.inFlight}
              streamingActivityLevel={state.streamingActivityLevel}
              permissionMode={state.permissionMode}
              reasoningEffort={state.reasoningEffort}
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
              promptHistory={state.promptHistory}
            />
          </div>
        )}
      </div>
    </div>
  );
}
