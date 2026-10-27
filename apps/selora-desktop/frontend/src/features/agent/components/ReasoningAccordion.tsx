// Selora IDE — Chain of Thought Reasoning Accordion (Cursor/Windsurf parity)
//
// Features:
// - Collapsible "Thinking Process" section with smooth spring height animation
// - Live elapsed stopwatch counter (e.g. "Thought for 4.2s")
// - Pulsating shimmer dot with live status indicator
// - Step-by-step thinking breakdown with bullet checkpoints
// - Copy reasoning trace action with tooltip & checkmark transition
import React, { useState, useEffect, useRef, useMemo } from 'react';
import {
  BrainCircuit,
  ChevronDown,
  Copy,
  Check,
  CheckCircle,
  Sparkles,
} from '../../../components/icons';

export type ReasoningStep = {
  id: string;
  title: string;
  detail?: string;
  status: 'done' | 'current' | 'pending';
};

export type ReasoningAccordionProps = {
  thought: string;
  durationSeconds?: number;
  isStreaming?: boolean;
  defaultExpanded?: boolean;
  className?: string;
  onToggle?: (expanded: boolean) => void;
};

/**
 * Intelligent parser that extracts step-by-step checkpoints from raw AI reasoning text.
 * Recognizes bullets (- / * / •), numbered items (1. / Step 1:), headers, and action clauses.
 */
export function extractReasoningSteps(rawThought: string, isStreaming = false): ReasoningStep[] {
  if (!rawThought || !rawThought.trim()) return [];

  const normalized = rawThought.replace(/\\n/g, '\n');
  const lines = normalized
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);

  const steps: ReasoningStep[] = [];
  let currentStep: ReasoningStep | null = null;

  const bulletRegex = /^(?:[-*•+]|\d+\.|\bstep\s*\d+:?|\bphase\s*\d+:?|#{1,4})\s*(.*)$/i;
  const actionPrefixRegex = /^(analyzed|inspecting|inspected|checking|checked|verifying|verified|formulating|formulated|reading|read|scanning|scanned|locating|located|updating|updated|generating|generated|planning|planned|reviewing|reviewed|synthesizing|synthesized)\b/i;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const bulletMatch = line.match(bulletRegex);
    const actionMatch = !bulletMatch && line.match(actionPrefixRegex);

    if (bulletMatch || actionMatch) {
      if (currentStep) {
        steps.push(currentStep);
      }
      const rawTitle = bulletMatch ? bulletMatch[1].trim() : line;
      // Strip markdown bold / italic markers from checkpoint title
      const cleanTitle = rawTitle.replace(/[*_`]/g, '').trim();

      currentStep = {
        id: `step-${steps.length + 1}`,
        title: cleanTitle || 'Thinking step',
        detail: '',
        status: 'done',
      };
    } else if (currentStep) {
      // Append line as detail/elaboration of the current step
      currentStep.detail = currentStep.detail
        ? `${currentStep.detail} ${line}`
        : line;
    }
  }

  if (currentStep) {
    steps.push(currentStep);
  }

  // Fallback: If no bullet points were found, synthesize steps from non-empty paragraphs
  if (steps.length === 0) {
    const paragraphs = rawThought
      .split(/\n\s*\n/)
      .map((p) => p.trim())
      .filter((p) => p.length > 0);

    paragraphs.slice(0, 6).forEach((para, idx) => {
      // First sentence or first 80 characters as title
      const firstSentenceMatch = para.match(/^([^.?!]+[.?!])/);
      const title = firstSentenceMatch
        ? firstSentenceMatch[1].trim().replace(/[*_`]/g, '')
        : para.slice(0, 75).trim() + (para.length > 75 ? '…' : '');
      const detail = para.length > title.length ? para.slice(title.length).trim() : undefined;

      steps.push({
        id: `synth-${idx + 1}`,
        title,
        detail,
        status: 'done',
      });
    });
  }

  // If streaming and we have steps, mark the last one as current
  if (isStreaming && steps.length > 0) {
    steps[steps.length - 1].status = 'current';
  }

  return steps;
}

export const ReasoningAccordion: React.FC<ReasoningAccordionProps> = ({
  thought,
  durationSeconds,
  isStreaming = false,
  defaultExpanded = false,
  className = '',
  onToggle,
}) => {
  const [isExpanded, setIsExpanded] = useState<boolean>(defaultExpanded);
  const [copied, setCopied] = useState<boolean>(false);
  const [elapsed, setElapsed] = useState<number>(durationSeconds ?? 0);
  const [viewMode, setViewMode] = useState<'steps' | 'raw'>('steps');
  const startTimeRef = useRef<number>(Date.now());
  const drawerId = React.useId();

  // Live stopwatch counter when streaming
  useEffect(() => {
    if (!isStreaming) {
      if (durationSeconds !== undefined) {
        setElapsed(durationSeconds);
      }
      return;
    }

    startTimeRef.current = Date.now() - (durationSeconds ? durationSeconds * 1000 : 0);
    const interval = setInterval(() => {
      const currentSeconds = (Date.now() - startTimeRef.current) / 1000;
      setElapsed(parseFloat(currentSeconds.toFixed(1)));
    }, 100);

    return () => clearInterval(interval);
  }, [isStreaming, durationSeconds]);

  const steps = useMemo(() => extractReasoningSteps(thought, isStreaming), [thought, isStreaming]);

  const handleToggle = () => {
    const nextState = !isExpanded;
    setIsExpanded(nextState);
    onToggle?.(nextState);
  };

  const handleCopy = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(thought);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      // Fallback
      setCopied(false);
    }
  };

  const formattedDuration = elapsed > 0 ? `${elapsed.toFixed(1)}s` : `${(durationSeconds || 2).toFixed(1)}s`;
  const thoughtLabel = isStreaming
    ? `Thinking… ${formattedDuration}`
    : `Thought for ${formattedDuration}`;

  return (
    <div
      className={`reasoning-accordion-root ${isExpanded ? 'is-expanded' : 'is-collapsed'} ${
        isStreaming ? 'is-streaming' : ''
      } ${className}`}
      data-testid="reasoning-accordion"
    >
      {/* Header Bar */}
      <div
        className="reasoning-accordion-header"
        onClick={handleToggle}
      >
        <button
          type="button"
          className="reasoning-accordion-toggle"
          onClick={(event) => {
            event.stopPropagation();
            handleToggle();
          }}
          aria-expanded={isExpanded}
          aria-controls={drawerId}
          aria-label={thoughtLabel}
        >
          <div className="reasoning-header-left">
            {/* Pulsating Shimmer Dot */}
            <span className={`reasoning-shimmer-dot ${isStreaming ? 'is-active-pulse' : ''}`} />

            <BrainCircuit size={13} className="reasoning-brain-icon" />

            <span className="reasoning-header-title">{thoughtLabel}</span>

            {steps.length > 0 && (
              <span className="reasoning-checkpoint-chip" title={`${steps.length} thinking checkpoints`}>
                {steps.length} {steps.length === 1 ? 'step' : 'steps'}
              </span>
            )}
          </div>
        </button>

        <div className="reasoning-header-right">
          {/* Quick Copy Action */}
          <button
            type="button"
            className={`reasoning-action-btn ${copied ? 'is-copied' : ''}`}
            onClick={handleCopy}
            title={copied ? 'Copied thought to clipboard!' : 'Copy thinking process'}
            aria-label={copied ? 'Copied reasoning' : 'Copy reasoning'}
          >
            {copied ? <Check size={11} className="text-ok" /> : <Copy size={11} />}
            <span className="action-tooltip">{copied ? 'Copied' : 'Copy'}</span>
          </button>

          {/* Smooth Spring Chevron */}
          <ChevronDown
            size={13}
            className={`reasoning-chevron ${isExpanded ? 'is-rotated' : ''}`}
            aria-hidden="true"
          />
        </div>
      </div>

      {/* Collapsible Accordion Drawer */}
      {isExpanded && (
        <div id={drawerId} className="reasoning-accordion-drawer">
          <div className="reasoning-inner-content">
            {/* View Mode Bar (if both checkpoints and detailed narrative exist) */}
            {steps.length > 0 && (
              <div className="reasoning-mode-selector">
                <button
                  type="button"
                  className={`reasoning-tab-btn ${viewMode === 'steps' ? 'is-active' : ''}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    setViewMode('steps');
                  }}
                >
                  <Sparkles size={11} />
                  <span>Checkpoints ({steps.length})</span>
                </button>
                <button
                  type="button"
                  className={`reasoning-tab-btn ${viewMode === 'raw' ? 'is-active' : ''}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    setViewMode('raw');
                  }}
                >
                  <span>Raw Trace</span>
                </button>
              </div>
            )}

            {viewMode === 'steps' && steps.length > 0 ? (
              /* Step-by-Step Thinking Breakdown */
              <div className="reasoning-steps-timeline">
                {steps.map((step, idx) => {
                  const isCurrent = step.status === 'current';
                  const isLast = idx === steps.length - 1;

                  return (
                    <div
                      key={step.id}
                      className={`reasoning-timeline-item ${isCurrent ? 'is-current' : 'is-done'} ${
                        isLast ? 'is-last' : ''
                      }`}
                    >
                      {/* Spine & Node */}
                      <div className="timeline-node-col">
                        <span className="timeline-node-icon">
                          {isCurrent ? (
                            <span className="node-active-ring">
                              <span className="node-active-center" />
                            </span>
                          ) : (
                            <CheckCircle size={12} className="node-check-icon" />
                          )}
                        </span>
                        {!isLast && <span className="timeline-spine-line" />}
                      </div>

                      {/* Content */}
                      <div className="timeline-content-col">
                        <div className="timeline-step-title">{step.title}</div>
                        {step.detail && <div className="timeline-step-detail">{step.detail}</div>}
                      </div>
                    </div>
                  );
                })}
              </div>
            ) : (
              /* Raw Monospace Thought Trace */
              <div className="reasoning-raw-trace-box">
                <pre className="reasoning-raw-pre">{thought}</pre>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
