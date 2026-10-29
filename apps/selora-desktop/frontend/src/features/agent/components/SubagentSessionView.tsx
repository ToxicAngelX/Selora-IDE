import React, { useEffect, useRef } from 'react';
import { ChevronRight } from '../../../components/icons';
import type { ChatMessage, SubagentInfo } from '../types';
import { MarkdownMessage } from './MarkdownMessage';
import { ToolCard } from './ToolCard';
import './subagent-session-view.css';

export type SubagentSessionViewProps = {
  child: SubagentInfo;
  /** The parent session title, for the "Back to parent" affordance. */
  parentTitle?: string;
  onBack: () => void;
};

function liveStatus(child: SubagentInfo): string {
  if (child.status === 'working') {
    if (child.state === 'awaiting_approval') return 'Live · awaiting approval';
    if (child.state === 'running_tool') return 'Live · running tool';
    if (child.state === 'done') return 'Live · finishing';
    return 'Live · thinking';
  }
  if (child.status === 'idle') return 'Idle';
  if (child.status === 'failed') return 'Failed';
  if (child.status === 'cancelled') return 'Cancelled';
  return 'Finished';
}

function hasStreamedOutput(messages: ChatMessage[] | undefined): boolean {
  return Boolean(messages?.some((message) => message.role === 'assistant' &&
    (message.content.trim() !== '' || (message.toolCalls?.length ?? 0) > 0)));
}

/**
 * Read-only transcript for one background child. It reuses the parent feed's
 * bubble/Markdown/ToolCard CSS, but never renders a composer: children are
 * task-scoped and do not accept follow-up prompts.
 */
export function SubagentSessionView({ child, parentTitle, onBack }: SubagentSessionViewProps) {
  const backRef = useRef<HTMLButtonElement>(null);
  const messages = child.messages ?? [];
  const running = child.status === 'working' || child.status === 'idle';
  const status = liveStatus(child);
  const hasOutput = hasStreamedOutput(child.messages);
  const displayName = child.name?.trim() || child.id;

  // Keyboard users land on the way back first, not inside the transcript.
  useEffect(() => {
    backRef.current?.focus();
  }, [child.id]);

  return (
    <section
      className="subagent-session-view"
      aria-label={`Subagent session ${displayName}`}
      data-testid="subagent-session-view"
    >
      <header className="subagent-session-header">
        <button
          ref={backRef}
          type="button"
          className="subagent-session-back"
          onClick={onBack}
          aria-label={parentTitle ? `Back to parent (${parentTitle})` : 'Back to parent'}
        >
          <ChevronRight size={13} aria-hidden="true" style={{ transform: 'scaleX(-1)' }} />
          <span>Back to parent</span>
        </button>
        <div className="subagent-session-identity">
          <strong className="subagent-session-name">{displayName}</strong>
          <code className="subagent-session-id">{child.id}</code>
        </div>
        <span className="subagent-session-policy">{child.policy ?? 'Policy unknown'}</span>
        <span
          className={`subagent-session-status${running ? ' is-live' : ''}${child.status === 'failed' ? ' is-failed' : ''}`}
          role="status"
        >
          {running && <span className="subagent-session-status-dot" aria-hidden="true" />}
          {status}
        </span>
      </header>

      {child.activeTool && (
        <p className="subagent-session-active-tool" role="status">
          Active tool: <code>{child.activeTool}</code>
        </p>
      )}

      {child.status === 'failed' && (
        <p className="subagent-session-error" role="alert">
          {child.error || 'Subagent failed without a message.'}
        </p>
      )}
      {child.status === 'cancelled' && (
        <p className="subagent-session-terminal-note">Subagent was cancelled.</p>
      )}
      {child.status === 'done' && !hasOutput && (
        <p className="subagent-session-terminal-note">
          {child.result || 'Subagent finished without a streamed transcript.'}
        </p>
      )}

      <div
        className="agent-execution-feed subagent-session-feed"
        role="log"
        aria-live="polite"
        aria-relevant="additions text"
        aria-label="Subagent conversation transcript"
      >
        <div className="feed-messages-list">
          {messages.length === 0 && (
            <p className="subagent-session-empty">
              {running ? 'Waiting for the child to produce output.' : 'No transcript was streamed for this child.'}
            </p>
          )}
          {messages.map((message) => (
            <div
              key={message.id}
              className={`feed-message-row ${message.role === 'user' ? 'row-user' : 'row-assistant'}`}
            >
              {message.role === 'user' ? (
                <div className="user-bubble-wrapper">
                  <div className="message-bubble bubble-user">
                    <div className="message-text-content">
                      <p className="message-paragraph">{message.content}</p>
                    </div>
                  </div>
                </div>
              ) : (
                <div className="message-bubble bubble-assistant">
                  <div className="assistant-bubble-meta">
                    <span className="assistant-meta-label">{displayName}</span>
                  </div>
                  {message.toolCalls && message.toolCalls.length > 0 && (
                    <div className="message-tool-calls-stack">
                      {message.toolCalls.map((tool) => (
                        <ToolCard key={tool.id} tool={tool} />
                      ))}
                    </div>
                  )}
                  {message.content.trim() !== '' && (
                    <div className="assistant-content-wrap">
                      <MarkdownMessage content={message.content} />
                      {message.status === 'streaming' && <span className="streaming-caret" aria-hidden="true" />}
                    </div>
                  )}
                </div>
              )}
            </div>
          ))}

          {running && !hasOutput && (
            <div className="subagent-session-thinking" role="status" aria-label="Subagent is working">
              <span className="subagent-session-thinking-dot" aria-hidden="true" />
              <span>{child.state === 'awaiting_approval' ? 'Waiting for approval…' : 'Thinking…'}</span>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}