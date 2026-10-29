// Selora IDE — ThinkingStatus (Phase Z / readability pass)
//
// A single slim, stable row that says what the agent is doing right now.
// It replaces the old behaviour where raw reasoning text streamed straight
// into the transcript and pushed the reader's scroll position around.
//
// Layout: Selora moon (ThinkingOrb) · rotating verb · elapsed seconds.
// The verb rotates slowly (4s) so the row stays calm instead of twitching.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { ThinkingOrb } from './ThinkingOrb';
import type { ThinkingState } from '../types';

const VERB_ROTATION_MS = 4000;

const VERBS: Record<ThinkingState, readonly string[]> = {
  breathing: ['Selora is breathing', 'Selora is here', 'Ready'],
  working: ['Thinking', 'Cooking', 'Brewing', 'Reasoning', 'Working it out'],
  searching: ['Searching the codebase', 'Scanning files', 'Hunting references'],
  solving: ['Cooking changes', 'Applying the plan', 'Editing files'],
  connecting: ['Writing response', 'Streaming'],
  listening: ['Waiting on your approval', 'Paused for review'],
  weaving: ['Weaving symbols', 'Linking references'],
  composing: ['Composing', 'Drafting the reply'],
  shaping: ['Shaping the plan', 'Sketching steps'],
};

export type ThinkingStatusProps = {
  state: ThinkingState;
  /** Live elapsed seconds since the turn started. Omit to hide the timer. */
  elapsedSeconds?: number;
  /** Compact mode drops the timer and shrinks the orb (used inline in a message). */
  compact?: boolean;
  /** Show the "Esc to stop" hint. */
  showStopHint?: boolean;
  className?: string;
};

function formatElapsed(totalSeconds: number): string {
  if (totalSeconds < 60) return `${Math.floor(totalSeconds)}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.floor(totalSeconds % 60);
  return `${minutes}m ${seconds.toString().padStart(2, '0')}s`;
}

export const ThinkingStatus: React.FC<ThinkingStatusProps> = ({
  state,
  elapsedSeconds,
  compact = false,
  showStopHint = false,
  className = '',
}) => {
  const verbs = useMemo(() => VERBS[state] ?? VERBS.breathing, [state]);
  const [verbIndex, setVerbIndex] = useState(0);
  const [autoElapsed, setAutoElapsed] = useState(0);
  const startedAtRef = useRef<number>(Date.now());

  // Rotate the verb slowly; reset when the semantic state changes.
  useEffect(() => {
    setVerbIndex(0);
    if (verbs.length < 2) return;
    const timer = setInterval(
      () => setVerbIndex((index) => (index + 1) % verbs.length),
      VERB_ROTATION_MS,
    );
    return () => clearInterval(timer);
  }, [verbs]);

  // Internal stopwatch, only when the caller does not supply one.
  useEffect(() => {
    if (compact || elapsedSeconds !== undefined) return;
    startedAtRef.current = Date.now();
    const timer = setInterval(() => {
      setAutoElapsed((Date.now() - startedAtRef.current) / 1000);
    }, 1000);
    return () => clearInterval(timer);
  }, [compact, elapsedSeconds, state]);

  const shownSeconds = elapsedSeconds ?? (compact ? undefined : autoElapsed);
  const label = verbs[verbIndex] ?? verbs[0];

  return (
    <div
      className={`selora-thinking-status ${compact ? 'is-compact' : ''} ${className}`}
      data-testid="thinking-status"
      data-state={state}
    >
      <span className="selora-thinking-orb" aria-hidden="true">
        <ThinkingOrb state={state} size={compact ? 16 : 20} />
      </span>

      <span className="selora-thinking-label" role="status" aria-live="off" aria-atomic="true">
        <span className="selora-thinking-verb">{label}</span>
        <span className="selora-thinking-dots" aria-hidden="true">
          <span />
          <span />
          <span />
        </span>
      </span>

      {shownSeconds !== undefined && shownSeconds >= 1 && (
        <span className="selora-thinking-elapsed" aria-hidden="true">
          {formatElapsed(shownSeconds)}
        </span>
      )}

      {showStopHint && (
        <span className="selora-thinking-hint" aria-hidden="true">
          esc to stop
        </span>
      )}
    </div>
  );
};