// Selora IDE — Editor Minimap (DESIGN §3.4)
//
// Code silhouette preview rendering micro-bars colored according to syntax
// tokens. The stream scales to the whole file so long documents stay an
// overview; a semi-transparent viewport box tracks scrolling. Hovering shows
// the section the click would reveal, and pointer drag scrubs the editor.
import {
  memo,
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import type { CodeLineData } from './editor-types';

export type EditorMinimapProps = {
  lines: CodeLineData[];
  /** Real document line count (may exceed the sampled `lines` rows). */
  totalLines?: number;
  scrollTop?: number;
  scrollHeight?: number;
  clientHeight?: number;
  onNavigateLine?: (lineNum: number) => void;
};

// The silhouette is memoized separately so hover/pointer state does not
// re-render thousands of line elements on every mouse move.
const MinimapCodeStream = memo(function MinimapCodeStream({ lines }: { lines: CodeLineData[] }) {
  return (
    <div className="minimap-code-stream" aria-hidden="true">
      {lines.map((line) => (
        <div key={line.lineNum} className="minimap-line">
          {line.tokens.map((token, idx) => {
            const widthCh = Math.min(Math.max(token.text.trim().length, 1), 24);
            if (token.text.trim().length === 0) {
              return (
                <span
                  key={idx}
                  className="minimap-indent"
                  style={{ width: `${Math.min(token.text.length * 2, 24)}px` }}
                />
              );
            }
            return (
              <span
                key={idx}
                className={`minimap-token ${token.type}`}
                style={{ width: `${widthCh * 2.2}px` }}
              />
            );
          })}
        </div>
      ))}
    </div>
  );
});

export function EditorMinimap({
  lines,
  totalLines: totalLinesProp,
  scrollTop = 0,
  scrollHeight = 100,
  clientHeight = 100,
  onNavigateLine,
}: EditorMinimapProps) {
  const minimapRef = useRef<HTMLDivElement>(null);
  const pointerIdRef = useRef<number | null>(null);
  const [hoverPct, setHoverPct] = useState<number | null>(null);
  const [dragging, setDragging] = useState(false);

  const totalLines = Math.max(totalLinesProp ?? lines.length, 1);
  const scrollRange = Math.max(scrollHeight - clientHeight, 0);
  const scrollRatio = scrollRange > 0 ? Math.min(Math.max(scrollTop / scrollRange, 0), 1) : 0;
  // Keep the box usable on very long files without losing the true position.
  const viewportHeightPct = Math.min(
    Math.max((clientHeight / Math.max(scrollHeight, 1)) * 100, 12),
    100,
  );
  const viewportTopPct = scrollRatio * (100 - viewportHeightPct);

  const currentLine = Math.min(totalLines, Math.round(scrollRatio * (totalLines - 1)) + 1);
  const lineRef = useRef(currentLine);

  useEffect(() => {
    lineRef.current = currentLine;
  }, [currentLine]);

  const navigate = useCallback(
    (line: number) => {
      const next = Math.max(1, Math.min(totalLines, line));
      lineRef.current = next;
      onNavigateLine?.(next);
    },
    [onNavigateLine, totalLines],
  );

  const fractionAt = useCallback((clientY: number): number => {
    const rect = minimapRef.current?.getBoundingClientRect();
    if (!rect || rect.height <= 0) return 0;
    return Math.max(0, Math.min(1, (clientY - rect.top) / rect.height));
  }, []);

  const lineAtFraction = useCallback(
    (fraction: number): number => Math.min(totalLines, Math.floor(fraction * totalLines) + 1),
    [totalLines],
  );

  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    pointerIdRef.current = event.pointerId;
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture is unavailable in some embedded/test environments.
    }
    setDragging(true);
    setHoverPct(null);
    navigate(lineAtFraction(fractionAt(event.clientY)));
  };

  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const fraction = fractionAt(event.clientY);
    if (pointerIdRef.current === event.pointerId) {
      navigate(lineAtFraction(fraction));
      return;
    }
    setHoverPct(fraction);
  };

  const endDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (pointerIdRef.current !== event.pointerId) return;
    pointerIdRef.current = null;
    try {
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    } catch {
      // Ignore capture cleanup failures; dragging state still clears below.
    }
    setDragging(false);
    setHoverPct(fractionAt(event.clientY));
  };

  const handlePointerLeave = () => {
    if (pointerIdRef.current === null) setHoverPct(null);
  };

  const handleLostPointerCapture = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (pointerIdRef.current !== event.pointerId) return;
    pointerIdRef.current = null;
    setDragging(false);
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    let target: number | null = null;
    if (e.key === 'ArrowDown') target = lineRef.current + 1;
    else if (e.key === 'ArrowUp') target = lineRef.current - 1;
    else if (e.key === 'PageDown') target = lineRef.current + 10;
    else if (e.key === 'PageUp') target = lineRef.current - 10;
    else if (e.key === 'Home') target = 1;
    else if (e.key === 'End') target = totalLines;
    if (target === null) return;
    e.preventDefault();
    navigate(target);
  };

  const hoverTopPct =
    hoverPct === null ? 0 : Math.min(hoverPct * 100, 100 - viewportHeightPct);

  return (
    <aside
      className={`editor-minimap${dragging ? ' is-dragging' : ''}`}
      ref={minimapRef}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onLostPointerCapture={handleLostPointerCapture}
      onPointerLeave={handlePointerLeave}
      onKeyDown={handleKeyDown}
      aria-label="Code minimap"
      aria-orientation="vertical"
      aria-valuemin={1}
      aria-valuemax={totalLines}
      aria-valuenow={currentLine}
      aria-valuetext={`Line ${currentLine} of ${totalLines}`}
      role="slider"
      tabIndex={0}
    >
      <MinimapCodeStream lines={lines} />

      {hoverPct !== null && !dragging ? (
        <div
          className="minimap-hover-preview"
          style={{ top: `${hoverTopPct}%`, height: `${viewportHeightPct}%` }}
          aria-hidden="true"
        />
      ) : null}

      {/* Floating Viewport Indicator Box */}
      <div
        className="minimap-viewport-slider"
        style={{
          top: `${viewportTopPct}%`,
          height: `${viewportHeightPct}%`,
        }}
        aria-hidden="true"
      />
    </aside>
  );
}
