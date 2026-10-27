// Selora IDE — Code Canvas (DESIGN §1.5, §3.4)
//
// High-fidelity code editor canvas:
// - Gutter with line numbers, git diffs (added/modified/deleted), and fold indicators
// - Active line highlight band and smooth blinking cursor
// - Syntax highlighting using strict monochrome design tokens (--syn-*)
// - Inline diagnostic squiggles with hover popovers and quick fix actions
// - Interactive line selection and block folding
import { useState, useRef, useEffect, type UIEvent } from 'react';
import type { CodeLineData, DiagnosticItem } from './editor-types';
import { AlertCircle, ChevronDown, ChevronRight, Lightbulb } from '../icons';

export type CodeCanvasProps = {
  lines: CodeLineData[];
  activeLine?: number;
  onSelectLine?: (lineNum: number) => void;
  onScroll?: (scrollTop: number, scrollHeight: number, clientHeight: number) => void;
};

export function CodeCanvas({
  lines,
  activeLine = 14,
  onSelectLine,
  onScroll,
}: CodeCanvasProps) {
  const [currentLine, setCurrentLine] = useState(activeLine);
  const [foldedLines, setFoldedLines] = useState<Record<number, boolean>>({});
  const [hoveredDiagnostic, setHoveredDiagnostic] = useState<{
    diagnostic: DiagnosticItem;
    top: number;
    left: number;
  } | null>(null);

  const canvasScrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setCurrentLine(activeLine);
  }, [activeLine]);

  const toggleFold = (lineNum: number) => {
    setFoldedLines((prev) => ({
      ...prev,
      [lineNum]: !prev[lineNum],
    }));
  };

  const handleScroll = (e: UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    onScroll?.(el.scrollTop, el.scrollHeight, el.clientHeight);
  };

  const handleLineClick = (lineNum: number) => {
    setCurrentLine(lineNum);
    onSelectLine?.(lineNum);
  };

  return (
    <div className="code-canvas-container" ref={canvasScrollRef} onScroll={handleScroll}>
      <div className="code-canvas-inner">
        {lines.map((line) => {
          const isCurrent = line.lineNum === currentLine;
          const isFolded = !!foldedLines[line.lineNum];
          const hasDiagnostic = !!line.diagnostic;

          return (
            <div
              key={line.lineNum}
              className={`code-line-row${isCurrent ? ' is-active-line' : ''}${
                line.diff ? ` has-diff diff-${line.diff}` : ''
              }`}
              onClick={() => handleLineClick(line.lineNum)}
            >
              {/* Gutter: Line Number + Diff Marker + Fold Toggle */}
              <div className="code-gutter">
                <span className={`diff-marker diff-${line.diff ?? 'none'}`} aria-hidden="true" />
                <span className="line-number">{line.lineNum}</span>
                <span className="fold-slot">
                  {line.foldable ? (
                    <button
                      type="button"
                      className="fold-btn"
                      aria-label={isFolded ? `Unfold line ${line.lineNum}` : `Fold line ${line.lineNum}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        toggleFold(line.lineNum);
                      }}
                    >
                      {isFolded ? <ChevronRight size={11} /> : <ChevronDown size={11} />}
                    </button>
                  ) : null}
                </span>
              </div>

              {/* Code Text with Syntax Highlight Tokens */}
              <div className="code-text-content">
                {line.tokens.map((token, tIdx) => {
                  const isSquiggle =
                    hasDiagnostic &&
                    line.diagnostic?.word &&
                    token.text.includes(line.diagnostic.word);

                  return isSquiggle ? (
                    <span
                      key={tIdx}
                      className={`syn-token ${token.type} diag-squiggle diag-squiggle-${line.diagnostic!.severity}`}
                      onMouseEnter={(e) => {
                        const rect = e.currentTarget.getBoundingClientRect();
                        setHoveredDiagnostic({
                          diagnostic: line.diagnostic!,
                          top: rect.bottom + 4,
                          left: rect.left,
                        });
                      }}
                      onMouseLeave={() => setHoveredDiagnostic(null)}
                      title={line.diagnostic!.message}
                    >
                      {token.text}
                    </span>
                  ) : (
                    <span key={tIdx} className={`syn-token ${token.type}`}>
                      {token.text}
                    </span>
                  );
                })}

                {/* Blinking Cursor on Active Line */}
                {isCurrent && <span className="code-cursor" aria-hidden="true" />}

                {/* Collapsed block indicator if folded */}
                {isFolded && (
                  <span className="code-folded-ellipsis" title="Collapsed code block">
                    {` ... `}
                  </span>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {/* Floating Diagnostic Card on Squiggle Hover */}
      {hoveredDiagnostic && (
        <div
          className="diagnostic-popover"
          style={{ top: `${hoveredDiagnostic.top}px`, left: `${hoveredDiagnostic.left}px` }}
          role="tooltip"
        >
          <div className="diagnostic-popover-header">
            <span className={`diagnostic-badge diagnostic-badge-${hoveredDiagnostic.diagnostic.severity}`}>
              <AlertCircle size={12} />
              <span>{hoveredDiagnostic.diagnostic.severity.toUpperCase()}</span>
            </span>
            {hoveredDiagnostic.diagnostic.source && (
              <span className="diagnostic-source">{hoveredDiagnostic.diagnostic.source}</span>
            )}
          </div>
          <div className="diagnostic-message">{hoveredDiagnostic.diagnostic.message}</div>
          {hoveredDiagnostic.diagnostic.quickFix && (
            <div className="diagnostic-actions">
              <button
                type="button"
                className="diagnostic-quick-fix-btn"
                onClick={() => setHoveredDiagnostic(null)}
              >
                <Lightbulb size={12} />
                <span>Quick Fix: {hoveredDiagnostic.diagnostic.quickFix}</span>
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
