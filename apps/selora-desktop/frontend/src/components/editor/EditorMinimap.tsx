// Selora IDE — Editor Minimap (DESIGN §3.4)
//
// Code silhouette preview rendering micro-bars colored according to syntax tokens.
// Features a visible semi-transparent viewport indicator that tracks scrolling
// and allows direct click-to-navigate.
import { useEffect, useRef, type KeyboardEvent, type MouseEvent } from 'react';
import type { CodeLineData } from './editor-types';

export type EditorMinimapProps = {
  lines: CodeLineData[];
  scrollTop?: number;
  scrollHeight?: number;
  clientHeight?: number;
  onNavigateLine?: (lineNum: number) => void;
};

export function EditorMinimap({
  lines,
  scrollTop = 0,
  scrollHeight = 100,
  clientHeight = 100,
  onNavigateLine,
}: EditorMinimapProps) {
  const minimapRef = useRef<HTMLDivElement>(null);

  // Compute viewport indicator position and height percentage
  const totalLines = Math.max(lines.length, 1);
  const ratio = scrollHeight > 0 ? clientHeight / scrollHeight : 1;
  const viewportHeightPct = Math.min(Math.max(ratio * 100, 15), 100);
  const scrollPct = scrollHeight > clientHeight ? (scrollTop / (scrollHeight - clientHeight)) * (100 - viewportHeightPct) : 0;

  const handleMinimapClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!minimapRef.current) return;
    const rect = minimapRef.current.getBoundingClientRect();
    const clickY = e.clientY - rect.top;
    const clickPct = Math.max(0, Math.min(1, clickY / rect.height));
    const targetLine = Math.floor(clickPct * totalLines) + 1;
    onNavigateLine?.(targetLine);
  };

  const currentLine = Math.max(
    1,
    Math.min(totalLines, Math.round((scrollTop / Math.max(scrollHeight, 1)) * totalLines) + 1),
  );
  const lineRef = useRef(currentLine);

  useEffect(() => {
    lineRef.current = currentLine;
  }, [currentLine]);

  const navigate = (line: number) => {
    const next = Math.max(1, Math.min(totalLines, line));
    lineRef.current = next;
    onNavigateLine?.(next);
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

  return (
    <aside
      className="editor-minimap"
      ref={minimapRef}
      onClick={handleMinimapClick}
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
      <div className="minimap-code-stream" aria-hidden="true">
        {lines.map((line) => {
          return (
            <div key={line.lineNum} className="minimap-line">
              {line.tokens.map((token, idx) => {
                const widthCh = Math.min(Math.max(token.text.trim().length, 1), 24);
                if (token.text.trim().length === 0) {
                  return <span key={idx} className="minimap-indent" style={{ width: `${token.text.length * 2}px` }} />;
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
          );
        })}
      </div>

      {/* Floating Viewport Indicator Box */}
      <div
        className="minimap-viewport-slider"
        style={{
          top: `${scrollPct}%`,
          height: `${viewportHeightPct}%`,
        }}
        aria-hidden="true"
      />
    </aside>
  );
}
