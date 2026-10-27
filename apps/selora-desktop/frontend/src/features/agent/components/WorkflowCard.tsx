// Selora IDE — Workflow confirmation card (ARCHITECTURE.md §18.1, Phase 8c)
//
// `run_workflow` never runs from the router: it parks the turn on this card.
// The card is the whole grant — the script body verbatim, the extracted
// `world.run` command set, and the workflow-level cost cap. Approve starts the
// run; Deny continues the loop like any other denied card.
//
// Styling rules: tokens only (`var(--…)` from tokens.css), no literal colors,
// no component stylesheet. The card is presentation + a decision callback; the
// session owns the pending approval id and the actual Effect::WorkflowRun.

import React, { useState } from 'react';
import {
  Check,
  ChevronDown,
  ChevronRight,
  Code2,
  ShieldAlert,
  Terminal,
  X,
} from '../../../components/icons';

/** The two answers this card offers. `ApproveAllThisTurn` is deliberately not
 *  available: a workflow grant is always reviewed on its own (§18.1). */
export type WorkflowDecision = 'approve' | 'deny';

export type WorkflowCardProps = {
  /** Workflow name from the `run_workflow` arguments (unique in the run). */
  name: string;
  /** The complete script body, rendered verbatim. */
  script: string;
  /** The extracted `world.run` command literals the user is confirming. */
  commands: string[];
  /** Workflow-level cost cap in USD (`workflow.max_cost_usd`). */
  maxCostUsd: number;
  /** The parked approval this card decides, echoed back to the caller. */
  approvalId?: string;
  /** Confirmation cards start expanded: the whole point is reading the grant. */
  defaultExpanded?: boolean;
  /** Once the session records a decision the buttons become a status line. */
  decision?: WorkflowDecision | null;
  /** The wiring seam: the session forwards this to `Effect::WorkflowDecision`. */
  onDecision?: (decision: WorkflowDecision, approvalId?: string) => void;
};

const chipStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 'var(--space-1)',
  padding: '1px var(--space-2)',
  border: '1px solid var(--hairline-faint)',
  borderRadius: 'var(--radius-1)',
  color: 'var(--fg-muted)',
  fontFamily: 'var(--font-mono)',
  fontSize: 'var(--type-xs)',
  lineHeight: 'var(--type-xs-lh)',
  whiteSpace: 'nowrap',
};

const sectionLabelStyle: React.CSSProperties = {
  marginBottom: 'var(--space-1)',
  color: 'var(--fg-muted)',
  fontFamily: 'var(--font-sans)',
  fontSize: 'var(--type-xs)',
  lineHeight: 'var(--type-xs-lh)',
  fontWeight: 'var(--weight-medium)',
  letterSpacing: 'var(--tracking-wide)',
  textTransform: 'uppercase',
};

const actionButtonStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 'var(--space-1)',
  padding: 'var(--space-1) var(--space-3)',
  borderRadius: 'var(--radius-1)',
  background: 'transparent',
  fontFamily: 'var(--font-sans)',
  fontSize: 'var(--type-xs)',
  lineHeight: 'var(--type-xs-lh)',
  fontWeight: 'var(--weight-medium)',
  cursor: 'pointer',
};

/** Format the cap honestly: a non-finite value is displayed as unknown rather
 *  than silently rounded to a number the user did not approve. */
function formatCost(maxCostUsd: number): string {
  return Number.isFinite(maxCostUsd) ? `$${maxCostUsd.toFixed(2)}` : 'unknown';
}

export const WorkflowCard: React.FC<WorkflowCardProps> = ({
  name,
  script,
  commands,
  maxCostUsd,
  approvalId,
  defaultExpanded = true,
  decision = null,
  onDecision,
}) => {
  const [expanded, setExpanded] = useState(defaultExpanded);

  const lineCount = script
    .replace(/\r\n/g, '\n')
    .replace(/\n$/, '')
    .split('\n').length;
  const costLabel = formatCost(maxCostUsd);
  const commandLabel = `${commands.length} ${commands.length === 1 ? 'command' : 'commands'}`;

  const decide = (next: WorkflowDecision) => {
    if (decision) return;
    onDecision?.(next, approvalId);
  };

  return (
    <div
      data-testid="workflow-card"
      data-decision={decision ?? 'pending'}
      role="group"
      aria-label={`Review workflow ${name}`}
      style={{
        border: '1px solid var(--hairline-active)',
        borderRadius: 'var(--radius-2)',
        background: 'var(--surface-2)',
        color: 'var(--fg-default)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-2)',
          padding: 'var(--space-2) var(--space-3)',
          background: 'var(--surface-3)',
          borderBottom: expanded ? '1px solid var(--hairline-faint)' : 'none',
        }}
      >
        <button
          type="button"
          data-testid="workflow-card-toggle"
          aria-expanded={expanded}
          aria-label={`Toggle workflow ${name} details`}
          onClick={() => setExpanded((value) => !value)}
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            padding: 'var(--space-0)',
            border: 'none',
            background: 'transparent',
            color: 'var(--fg-muted)',
            cursor: 'pointer',
          }}
        >
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </button>

        <ShieldAlert size={13} aria-hidden="true" style={{ color: 'var(--warn)' }} />

        <span
          style={{
            minWidth: 0,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            fontFamily: 'var(--font-sans)',
            fontSize: 'var(--type-sm)',
            lineHeight: 'var(--type-sm-lh)',
            fontWeight: 'var(--weight-medium)',
          }}
        >
          {name}
        </span>

        <span style={{ ...chipStyle, color: 'var(--fg-faint)' }}>workflow</span>

        <span
          style={{
            marginLeft: 'auto',
            display: 'inline-flex',
            alignItems: 'center',
            gap: 'var(--space-2)',
          }}
        >
          <span style={chipStyle} title="Script size">
            <Code2 size={10} /> {lineCount} {lineCount === 1 ? 'line' : 'lines'}
          </span>
          <span style={chipStyle} title="Confirmed world.run command set">
            <Terminal size={10} /> {commandLabel}
          </span>
          <span
            data-testid="workflow-card-cost"
            style={{ ...chipStyle, color: 'var(--warn)' }}
            title="Workflow-level cost cap, checked between steps"
          >
            cap {costLabel}
          </span>
        </span>
      </div>

      {expanded && (
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 'var(--space-3)',
            padding: 'var(--space-3)',
          }}
        >
          <div
            style={{
              display: 'flex',
              alignItems: 'baseline',
              gap: 'var(--space-2)',
              paddingBottom: 'var(--space-2)',
              borderBottom: '1px solid var(--hairline-faint)',
            }}
          >
            <span
              style={{
                ...chipStyle,
                color: 'var(--warn)',
                borderColor: 'var(--warn)',
                fontFamily: 'var(--font-sans)',
                textTransform: 'uppercase',
                letterSpacing: 'var(--tracking-wide)',
              }}
            >
              Confirmation required
            </span>
            <span
              style={{
                color: 'var(--fg-muted)',
                fontFamily: 'var(--font-sans)',
                fontSize: 'var(--type-xs)',
                lineHeight: 'var(--type-xs-lh)',
              }}
            >
              Approving starts the run. Subagent writes and commands still ask
              individually — this approval never pre-approves them.
            </span>
          </div>

          <section aria-label="Workflow script">
            <div style={sectionLabelStyle}>Script</div>
            <pre
              data-testid="workflow-card-script"
              style={{
                margin: 'var(--space-0)',
                padding: 'var(--space-2)',
                maxHeight: '280px',
                overflow: 'auto',
                whiteSpace: 'pre',
                background: 'var(--ink-canvas)',
                border: '1px solid var(--hairline-faint)',
                borderRadius: 'var(--radius-1)',
                color: 'var(--fg-default)',
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--type-xs)',
                lineHeight: 'var(--type-xs-lh)',
              }}
            >
              {script}
            </pre>
          </section>

          <section aria-label="Confirmed command set">
            <div style={sectionLabelStyle}>
              world.run command set ({commands.length})
            </div>
            <div data-testid="workflow-card-commands">
              {commands.length === 0 ? (
                <span
                  style={{
                    color: 'var(--fg-faint)',
                    fontFamily: 'var(--font-sans)',
                    fontSize: 'var(--type-xs)',
                    lineHeight: 'var(--type-xs-lh)',
                  }}
                >
                  No shell commands — this workflow uses agents and read-only
                  world access only.
                </span>
              ) : (
                <ul
                  style={{
                    margin: 'var(--space-0)',
                    padding: 'var(--space-0)',
                    listStyle: 'none',
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 'var(--space-1)',
                  }}
                >
                  {commands.map((command, index) => (
                    <li
                      key={`${index}:${command}`}
                      data-testid="workflow-card-command"
                      style={{
                        display: 'flex',
                        alignItems: 'baseline',
                        gap: 'var(--space-2)',
                        padding: 'var(--space-1) var(--space-2)',
                        background: 'var(--surface-1)',
                        border: '1px solid var(--hairline-faint)',
                        borderRadius: 'var(--radius-1)',
                      }}
                    >
                      <span
                        aria-hidden="true"
                        style={{
                          color: 'var(--fg-faint)',
                          fontFamily: 'var(--font-mono)',
                          fontSize: 'var(--type-xs)',
                          lineHeight: 'var(--type-xs-lh)',
                        }}
                      >
                        {index + 1}
                      </span>
                      <code
                        style={{
                          color: 'var(--fg-default)',
                          fontFamily: 'var(--font-mono)',
                          fontSize: 'var(--type-sm)',
                          lineHeight: 'var(--type-sm-lh)',
                          overflowWrap: 'anywhere',
                        }}
                      >
                        {command}
                      </code>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </section>

          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 'var(--space-2)',
              paddingTop: 'var(--space-2)',
              borderTop: '1px solid var(--hairline-faint)',
            }}
          >
            <span
              style={{
                color: 'var(--fg-muted)',
                fontFamily: 'var(--font-sans)',
                fontSize: 'var(--type-xs)',
                lineHeight: 'var(--type-xs-lh)',
              }}
            >
              Cost cap
            </span>
            <strong
              style={{
                color: 'var(--warn)',
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--type-sm)',
                lineHeight: 'var(--type-sm-lh)',
                fontWeight: 'var(--weight-semibold)',
              }}
            >
              {costLabel}
            </strong>
            <span
              style={{
                color: 'var(--fg-faint)',
                fontFamily: 'var(--font-sans)',
                fontSize: 'var(--type-xs)',
                lineHeight: 'var(--type-xs-lh)',
              }}
            >
              checked between every step
            </span>

            <span
              style={{
                marginLeft: 'auto',
                display: 'inline-flex',
                alignItems: 'center',
                gap: 'var(--space-2)',
              }}
            >
              {decision ? (
                <span
                  data-testid="workflow-card-decision"
                  style={{
                    ...chipStyle,
                    color: decision === 'approve' ? 'var(--ok)' : 'var(--err)',
                    borderColor: decision === 'approve' ? 'var(--ok)' : 'var(--err)',
                    fontFamily: 'var(--font-sans)',
                  }}
                >
                  {decision === 'approve' ? (
                    <>
                      <Check size={11} /> Approved
                    </>
                  ) : (
                    <>
                      <X size={11} /> Denied
                    </>
                  )}
                </span>
              ) : (
                <>
                  <button
                    type="button"
                    data-testid="workflow-deny"
                    onClick={() => decide('deny')}
                    aria-label={`Deny workflow ${name}`}
                    style={{ ...actionButtonStyle, color: 'var(--fg-muted)' }}
                  >
                    <X size={11} />
                    Deny
                  </button>
                  <button
                    type="button"
                    data-testid="workflow-approve"
                    onClick={() => decide('approve')}
                    aria-label={`Approve workflow ${name}`}
                    style={{
                      ...actionButtonStyle,
                      color: 'var(--ok)',
                      border: '1px solid var(--ok)',
                    }}
                  >
                    <Check size={11} />
                    Approve &amp; run
                  </button>
                </>
              )}
            </span>
          </div>
        </div>
      )}
    </div>
  );
};