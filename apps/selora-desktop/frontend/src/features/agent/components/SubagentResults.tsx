import React, { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, Clock, X } from '../../../components/icons';
import type { SubagentInfo } from '../types';
import './subagent-results.css';

type SubagentResultsProps = {
  sessionId: string;
  subagents: SubagentInfo[];
  revealedChildId: string | null;
  onRead: (childId: string) => void;
  onClearReveal: () => void;
  /** Open the child's own read-only transcript in the dock. */
  onOpenSession: (childId: string) => void;
};

function statusLabel(child: SubagentInfo): string {
  if (child.status === 'cancelled') return 'Cancelled';
  if (child.status === 'failed') return 'Failed';
  if (child.status === 'done') return 'Succeeded';
  if (child.status === 'idle') return 'Idle';
  return child.state === 'awaiting_approval' ? 'Awaiting approval' : 'Running';
}

function liveDetail(child: SubagentInfo): string | null {
  if (child.status !== 'working') return null;
  if (child.activeTool) return `Running ${child.activeTool}`;
  if (child.state === 'thinking') return 'Thinking';
  if (child.state === 'awaiting_approval') return 'Awaiting approval';
  if (child.state === 'done') return 'Finishing';
  return child.activeStep ?? null;
}

/** Running and finished children, grouped under the selected parent session. */
export function SubagentResults({
  sessionId,
  subagents,
  revealedChildId,
  onRead,
  onClearReveal,
  onOpenSession,
}: SubagentResultsProps) {
  const children = subagents.filter((child) => child.parentSessionId === sessionId);
  if (children.length === 0) return null;

  return (
    <section className="subagent-results" aria-label="Subagent sessions">
      <div className="subagent-results-heading">
        <strong>Subagent sessions</strong>
        <span>{children.length}</span>
      </div>
      <p className="subagent-results-note">Running and finished children. Open one to read its live transcript.</p>
      <div className="subagent-results-list">
        {children.map((child) => (
          <SubagentResult key={child.id} child={child} revealed={revealedChildId === child.id}
            onRead={onRead} onClearReveal={onClearReveal} onOpenSession={onOpenSession} />
        ))}
      </div>
    </section>
  );
}

function SubagentResult({ child, revealed, onRead, onClearReveal, onOpenSession }: {
  child: SubagentInfo;
  revealed: boolean;
  onRead: (childId: string) => void;
  onClearReveal: () => void;
  onOpenSession: (childId: string) => void;
}) {
  const [expanded, setExpanded] = useState(revealed);
  const toggle = useRef<HTMLButtonElement>(null);
  const failed = child.status === 'failed';
  const cancelled = child.status === 'cancelled';
  const running = child.status === 'working' || child.status === 'idle';
  const status = statusLabel(child);
  const detail = liveDetail(child);
  const bodyId = `subagent-result-body-${child.id}`;
  const displayName = child.name?.trim() || child.id;

  useEffect(() => {
    if (!revealed) return;
    setExpanded(true);
    toggle.current?.focus();
    toggle.current?.scrollIntoView?.({ block: 'nearest' });
  }, [revealed]);

  return (
    <article
      className={`subagent-result-card${failed ? ' is-failed' : cancelled ? ' is-cancelled' : running ? ' is-running' : ''}`}
      data-testid={expanded ? 'subagent-result' : undefined}
    >
      <button ref={toggle} type="button" className="subagent-result-toggle"
        aria-label={`${expanded ? 'Collapse' : 'Expand'} result for ${displayName}`}
        aria-describedby={`${bodyId}-summary`}
        aria-expanded={expanded} aria-controls={bodyId}
        onClick={() => {
          setExpanded(!expanded);
          if (!expanded) onRead(child.id);
          if (revealed) onClearReveal();
        }}>
        {failed || cancelled ? <X size={14} aria-hidden="true" /> : running
          ? <Clock size={14} aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}
        <span className="subagent-result-name">{displayName}</span>
        <code className="subagent-result-id">{child.id}</code>
        <span className="subagent-result-policy">{child.policy ?? 'Policy unknown'}</span>
        <span className={`subagent-result-status${running ? ' is-running' : ''}`}>{status}</span>
        {child.unread && <span className="subagent-result-unread" aria-label="Unread result" />}
        <ChevronDown size={13} aria-hidden="true" className={expanded ? 'is-expanded' : ''} />
      </button>
      <span id={`${bodyId}-summary`} className="visually-hidden">
        {status}.{detail ? ` ${detail}.` : ''} Policy: {child.policy ?? 'unknown'}.{child.unread ? ' Unread result.' : ''}
      </span>
      <div className="subagent-result-actions">
        {detail && <span className="subagent-result-detail">{detail}</span>}
        <button type="button" className="subagent-result-open"
          onClick={() => onOpenSession(child.id)}>
          Open session
        </button>
      </div>
      {expanded && (
        <div id={bodyId} className="subagent-result-content">
          {detail && <p className="subagent-result-live" role="status">{detail}</p>}
          {failed && <p className="subagent-result-error">{child.error || 'Subagent failed without a message.'}</p>}
          {cancelled && <p className="subagent-result-live">Subagent was cancelled.</p>}
          {!running && (
            <div className="subagent-result-message">
              {child.result || (failed ? 'No final message.' : 'Subagent finished without a final message.')}
            </div>
          )}
          {running && !detail && <p className="subagent-result-live">Waiting for the child's first output.</p>}
          {!!child.artifacts?.length && (
            <div className="subagent-result-artifacts">
              <strong>Artifacts ({child.artifacts.length})</strong>
              <ul>
                {child.artifacts.map((artifact, index) => (
                  <li key={`${artifact.kind}:${artifact.path}:${index}`}>
                    <span>{artifact.kind}</span> <code>{artifact.path}</code>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </article>
  );
}