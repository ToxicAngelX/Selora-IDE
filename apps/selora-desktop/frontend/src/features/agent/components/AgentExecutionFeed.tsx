// Selora IDE — ZCode Parity Execution Feed (UI_SPEC §4.2, REVAMP_SPEC §4/§5)
//
// Renders conversation messages, collapsible thought blocks,
// tool execution cards, status line, and turn summary footer.
import React, { useRef, useEffect, useState } from 'react';
import {
  BrainCircuit,
  ChevronDown,
  Copy,
  Check,
  Edit,
  RotateLeft,
  GitBranch,
  DirectSend,
  X,
} from '../../../components/icons';
import type { ChatMessage, ThinkingState, AgentMetrics, QueuedPrompt } from '../types';
import { ToolCard } from './ToolCard';
import { ThinkingOrb } from '../presence/ThinkingOrb';
import { BotAvatar } from '../presence/BotAvatar';
import { agentStore } from '../agent-store';

export type AgentExecutionFeedProps = {
  messages: ChatMessage[];
  inFlight: boolean;
  thinkingState: ThinkingState;
  metrics: AgentMetrics;
  promptQueue?: QueuedPrompt[];
};

const SUGGESTIONS = [
  'Explain this workspace architecture',
  'Find TODOs and unhandled edge cases',
  'Review the active changes for bugs',
];

export const AgentExecutionFeed: React.FC<AgentExecutionFeedProps> = ({
  messages,
  inFlight,
  thinkingState,
  metrics,
  promptQueue,
}) => {
  const bottomRef = useRef<HTMLDivElement>(null);
  const [expandedThoughts, setExpandedThoughts] = useState<Record<string, boolean>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editBuffer, setEditBuffer] = useState<string>('');

  const activeQueue = promptQueue ?? agentStore.getSnapshot().promptQueue;

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, inFlight, activeQueue.length]);

  const toggleThought = (msgId: string) => {
    setExpandedThoughts((prev) => ({
      ...prev,
      [msgId]: !prev[msgId],
    }));
  };

  const getStatusText = () => {
    switch (thinkingState) {
      case 'working':
        return 'Thinking…';
      case 'searching':
        return 'Searching codebase…';
      case 'solving':
        return 'Executing tool changes…';
      case 'listening':
        return 'Awaiting your approval…';
      case 'connecting':
        return 'Streaming response…';
      case 'weaving':
        return 'Weaving workspace symbols…';
      default:
        return 'Ready';
    }
  };

  // If transcript is empty, show the hero welcome state
  if (messages.length === 0) {
    return (
      <div className="agent-execution-feed is-empty">
        <div className="agent-welcome-hero">
          {/* 64px Breathing ThinkingOrb */}
          <ThinkingOrb state="breathing" size={64} isLive={true} />

          <h2 className="welcome-heading">Ask anything</h2>
          <p className="welcome-subline">
            <em>Ask about the code, or tell Selora what to change.</em>
          </p>

          <div className="welcome-suggestions-grid">
            {SUGGESTIONS.map((prompt) => (
              <button
                key={prompt}
                type="button"
                className="welcome-chip"
                onClick={() => agentStore.sendUserMessage(prompt)}
              >
                {prompt}
              </button>
            ))}
          </div>
        </div>
      </div>
    );
  }

  function parseMessageContent(rawContent: string): { thoughtText?: string; mainContent: string } {
    if (!rawContent) return { mainContent: '' };
    const thinkMatch = rawContent.match(/^\s*<think>([\s\S]*?)<\/think>\s*([\s\S]*)$/i);
    if (thinkMatch) {
      return {
        thoughtText: thinkMatch[1].trim(),
        mainContent: thinkMatch[2].trim(),
      };
    }
    const partialThinkMatch = rawContent.match(/^\s*<think>([\s\S]*)$/i);
    if (partialThinkMatch) {
      return {
        thoughtText: partialThinkMatch[1].trim(),
        mainContent: '',
      };
    }
    return { mainContent: rawContent };
  }

  return (
    <div className="agent-execution-feed" aria-label="Conversation Feed">
      <div className="feed-messages-list">
        {messages.map((msg) => {
          const isUser = msg.role === 'user';
          const parsed = !isUser ? parseMessageContent(msg.content) : { mainContent: msg.content };
          const thoughtToShow = parsed.thoughtText || msg.thought;
          const contentToShow = parsed.mainContent || (!parsed.thoughtText ? msg.content : '');
          const isThoughtExpanded = expandedThoughts[msg.id] ?? (!contentToShow && !!thoughtToShow);

          return (
            <div
              key={msg.id}
              className={`feed-message-row ${isUser ? 'row-user' : 'row-assistant'}`}
            >
              {isUser ? (
                /* --- USER MESSAGE CONTAINER WITH MICRO-ACTION TOOLBAR --- */
                <div className="user-bubble-wrapper">
                  <div
                    className={`message-bubble bubble-user ${msg.isSteer ? 'is-steer-prompt' : ''} ${
                      editingId === msg.id ? 'is-editing' : ''
                    }`}
                  >
                    {/* Steer Directive Indicator */}
                    {msg.isSteer && (
                      <div className="user-steer-badge" title="Steering directive sent mid-flight">
                        <DirectSend size={12} className="steer-badge-icon" />
                        <span>Steer Directive</span>
                      </div>
                    )}

                    {/* Inline Edit Mode vs Display Mode */}
                    {editingId === msg.id ? (
                      <div className="user-prompt-edit-container">
                        <textarea
                          className="user-prompt-edit-textarea"
                          value={editBuffer}
                          onChange={(e) => setEditBuffer(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter' && !e.shiftKey) {
                              e.preventDefault();
                              if (editBuffer.trim()) {
                                agentStore.resubmitEditedPrompt(msg.id, editBuffer);
                                setEditingId(null);
                              }
                            } else if (e.key === 'Escape') {
                              e.preventDefault();
                              setEditingId(null);
                            }
                          }}
                          rows={Math.max(2, Math.min(8, editBuffer.split('\n').length))}
                          autoFocus
                          aria-label="Edit prompt input"
                        />
                        <div className="user-prompt-edit-actions">
                          <span className="user-prompt-edit-hint">Enter to resubmit · Esc to cancel</span>
                          <div className="edit-btn-group">
                            <button
                              type="button"
                              className="user-prompt-cancel-btn"
                              onClick={() => setEditingId(null)}
                            >
                              Cancel
                            </button>
                            <button
                              type="button"
                              className="user-prompt-submit-btn"
                              disabled={!editBuffer.trim()}
                              onClick={() => {
                                if (editBuffer.trim()) {
                                  agentStore.resubmitEditedPrompt(msg.id, editBuffer);
                                  setEditingId(null);
                                }
                              }}
                            >
                              Save &amp; Resubmit
                            </button>
                          </div>
                        </div>
                      </div>
                    ) : (
                      <>
                        <div className="message-text-content">
                          <p className="message-paragraph">{contentToShow}</p>
                        </div>
                        {msg.isEdited && (
                          <span className="user-msg-edited-indicator" title="This prompt was edited">
                            (edited)
                          </span>
                        )}
                      </>
                    )}
                  </div>

                  {/* Micro-Action Hover Toolbar (Copy, Rewind, Edit, Fork) */}
                  {editingId !== msg.id && (
                    <div className="user-prompt-action-toolbar" role="toolbar" aria-label="Prompt actions">
                      {/* Copy Button */}
                      <button
                        type="button"
                        className={`user-action-btn ${copiedId === msg.id ? 'is-copied' : ''}`}
                        onClick={async () => {
                          const ok = await agentStore.copyPrompt(msg.content);
                          if (ok) {
                            setCopiedId(msg.id);
                            setTimeout(() => setCopiedId(null), 1800);
                          }
                        }}
                        title={copiedId === msg.id ? 'Copied to clipboard!' : 'Copy prompt'}
                        aria-label={copiedId === msg.id ? 'Copied' : 'Copy prompt'}
                      >
                        {copiedId === msg.id ? (
                          <Check size={12} className="text-ok" />
                        ) : (
                          <Copy size={12} />
                        )}
                        <span className="btn-tooltip-label">
                          {copiedId === msg.id ? 'Copied!' : 'Copy'}
                        </span>
                      </button>

                      {/* Rewind Button */}
                      <button
                        type="button"
                        className="user-action-btn rewind-btn"
                        onClick={() => {
                          agentStore.rewindToPrompt(msg.id, true);
                        }}
                        title="Rewind conversation to this checkpoint (restores draft &amp; rolls back turns)"
                        aria-label="Rewind here"
                      >
                        <RotateLeft size={12} />
                        <span className="btn-tooltip-label">Rewind</span>
                      </button>

                      {/* Edit Button */}
                      <button
                        type="button"
                        className="user-action-btn"
                        onClick={() => {
                          setEditingId(msg.id);
                          setEditBuffer(msg.content);
                        }}
                        title="Edit prompt &amp; rerun"
                        aria-label="Edit prompt"
                      >
                        <Edit size={12} />
                        <span className="btn-tooltip-label">Edit</span>
                      </button>

                      {/* Fork Session Button */}
                      <button
                        type="button"
                        className="user-action-btn"
                        onClick={() => {
                          agentStore.forkSessionFromPrompt(msg.id);
                        }}
                        title="Fork new branch session from this checkpoint"
                        aria-label="Fork session"
                      >
                        <GitBranch size={12} />
                        <span className="btn-tooltip-label">Fork</span>
                      </button>
                    </div>
                  )}
                </div>
              ) : (
                /* --- ASSISTANT MESSAGE CONTAINER --- */
                <div className="message-bubble bubble-assistant">
                  <div className="assistant-bubble-meta">
                    <BotAvatar
                      role="orchestrator"
                      status={msg.status === 'streaming' ? 'working' : 'default'}
                      size={16}
                      name="Selora Coding Agent"
                    />
                    <span className="assistant-meta-label">Selora Agent</span>
                  </div>

                  {/* 1. Collapsible Thought Block (ZCode style) */}
                  {thoughtToShow && (
                    <div className="message-thought-block">
                      <button
                        type="button"
                        className="thought-toggle-btn"
                        onClick={() => toggleThought(msg.id)}
                      >
                        <BrainCircuit size={13} className="thought-icon-svg" />
                        <span className="thought-label">
                          Thought · {msg.thoughtDurationSeconds || 2}s
                        </span>
                        <ChevronDown
                          size={13}
                          className={`thought-chevron-svg ${isThoughtExpanded ? 'is-expanded' : ''}`}
                          style={{
                            transform: isThoughtExpanded ? 'rotate(180deg)' : 'none',
                            transition: 'transform 150ms ease',
                          }}
                        />
                      </button>
                      {isThoughtExpanded && (
                        <div className="thought-content-box">
                          <pre className="thought-pre">{thoughtToShow}</pre>
                        </div>
                      )}
                    </div>
                  )}

                  {/* 2. Tool Execution Cards */}
                  {msg.toolCalls && msg.toolCalls.length > 0 && (
                    <div className="message-tool-calls-stack">
                      {msg.toolCalls.map((tool) => (
                        <ToolCard key={tool.id} tool={tool} />
                      ))}
                    </div>
                  )}

                  {/* 3. Message Text Content or Streaming Indicator */}
                  {contentToShow ? (
                    <div className="message-text-content">
                      <p className="message-paragraph">{contentToShow}</p>
                      {(contentToShow.toLowerCase().includes('api key') || msg.status === 'error') && (
                        <div style={{ marginTop: '10px' }}>
                          <button
                            type="button"
                            className="feed-connect-key-btn"
                            onClick={() => window.dispatchEvent(new CustomEvent('selora:open-connect'))}
                          >
                            Configure API Key &amp; Provider
                          </button>
                        </div>
                      )}
                    </div>
                  ) : msg.status === 'streaming' ? (
                    <div className="agent-thinking-stream-indicator">
                      <span className="typing-dot" />
                      <span className="typing-dot" />
                      <span className="typing-dot" />
                    </div>
                  ) : msg.status === 'error' ? (
                    <div className="message-text-content">
                      <p className="message-paragraph text-error">
                        Encountered an issue executing request. Check model provider or API key.
                      </p>
                      <div style={{ marginTop: '8px' }}>
                        <button
                          type="button"
                          className="feed-connect-key-btn"
                          onClick={() => window.dispatchEvent(new CustomEvent('selora:open-connect'))}
                        >
                          Configure API Key &amp; Provider
                        </button>
                      </div>
                    </div>
                  ) : !thoughtToShow ? (
                    <div className="message-text-content">
                      <p className="message-paragraph text-muted" style={{ fontStyle: 'italic', opacity: 0.8 }}>
                        {msg.toolCalls && msg.toolCalls.length > 0 ? 'Executed tools successfully.' : 'Ready for your next instruction.'}
                      </p>
                    </div>
                  ) : null}
                </div>
              )}
            </div>
          );
        })}

        {/* 4. Prompt Queue Tray (shows all queued instructions scheduled to run next) */}
        {activeQueue && activeQueue.length > 0 && (
          <div className="agent-prompt-queue-tray" aria-label="Prompt Queue">
            <div className="prompt-queue-header">
              <div className="queue-header-left">
                <span className="queue-pulse-dot" />
                <span className="queue-title">Prompt Queue</span>
                <span className="queue-count-badge">{activeQueue.length}</span>
              </div>
              <div className="queue-header-actions">
                <button
                  type="button"
                  className="queue-clear-btn"
                  onClick={() => agentStore.clearPromptQueue()}
                  title="Clear all queued prompts"
                >
                  Clear queue
                </button>
              </div>
            </div>
            <div className="prompt-queue-items-list">
              {activeQueue.map((item, index) => (
                <div key={item.id} className="prompt-queue-item-row">
                  <span className="queue-order-badge">#{index + 1}</span>
                  <span className="queue-item-text" title={item.text}>
                    {item.text}
                  </span>
                  <div className="queue-item-actions">
                    <button
                      type="button"
                      className="queue-action-btn steer-now-btn"
                      onClick={() => {
                        agentStore.removeQueuedPrompt(item.id);
                        agentStore.steerActiveTurn(item.text);
                      }}
                      title="Steer now (promote immediately as steering directive)"
                      aria-label="Steer now"
                    >
                      <DirectSend size={11} />
                      <span>Steer now</span>
                    </button>
                    <button
                      type="button"
                      className="queue-action-btn"
                      onClick={() => {
                        agentStore.removeQueuedPrompt(item.id);
                        if (typeof window !== 'undefined') {
                          window.dispatchEvent(
                            new CustomEvent('selora:set-composer-draft', {
                              detail: { text: item.text },
                            })
                          );
                        }
                      }}
                      title="Edit in composer"
                      aria-label="Edit in composer"
                    >
                      <Edit size={11} />
                    </button>
                    <button
                      type="button"
                      className="queue-action-btn remove-btn"
                      onClick={() => agentStore.removeQueuedPrompt(item.id)}
                      title="Remove from queue"
                      aria-label="Remove"
                    >
                      <X size={11} />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* 5. Active Status Line (when in flight) */}
        {inFlight && (
          <div className="agent-active-status-line">
            <div className="status-line-left">
              <ThinkingOrb state={thinkingState} size={20} />
              <BotAvatar role="orchestrator" status="working" size={20} name="Selora Orchestrator" />
              <span className="status-text">{getStatusText()}</span>
            </div>
            <button
              type="button"
              className="status-stop-btn"
              onClick={() => agentStore.stopActiveTurn()}
              title="Stop execution (Esc)"
            >
              Stop
            </button>
          </div>
        )}

        {/* 5. Turn Summary Footer (when done and tools ran) */}
        {!inFlight && metrics.activeToolsCount > 0 && (
          <div className="turn-summary-footer">
            <span>
              {metrics.activeToolsCount} tools · {metrics.approvedCount} approved ·{' '}
              {metrics.deniedCount} denied · {(metrics.totalTokens / 1000).toFixed(1)}k tokens · $
              {metrics.costUsd.toFixed(3)}
            </span>
          </div>
        )}

        <div ref={bottomRef} />
      </div>
    </div>
  );
};
