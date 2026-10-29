// Selora IDE — end-of-turn report card.
//
// The transcript stays complete; this card is the digest a reader scans after
// a turn: what changed, what ran, how much the plan advanced, and the cost.
// It renders only reported facts, and collapses to one line once reviewed.
import React, { useMemo, useState } from 'react';
import {
  Check,
  ChevronDown,
  Copy,
  FileEdit,
  FilePlus,
  FileText,
  Terminal,
  XCircle,
} from '../../../components/icons';
import type { TurnReport } from '../turn-report';
import { reportToText } from '../turn-report';

export type TurnReportCardProps = {
  report: TurnReport;
};

const MAX_ROWS = 6;

function fileDelta(file: TurnReport['files'][number]): string {
  if (file.added === undefined && file.removed === undefined) return '';
  return `+${file.added ?? 0} −${file.removed ?? 0}`;
}

export const TurnReportCard: React.FC<TurnReportCardProps> = ({ report }) => {
  const [collapsed, setCollapsed] = useState(false);
  const [copied, setCopied] = useState(false);

  const visibleFiles = report.files.slice(0, MAX_ROWS);
  const visibleCommands = report.commands.slice(0, MAX_ROWS);
  const hiddenFiles = report.files.length - visibleFiles.length;
  const hiddenCommands = report.commands.length - visibleCommands.length;

  const headline = useMemo(() => {
    const parts: string[] = [];
    if (report.files.length > 0) {
      parts.push(`${report.files.length} ${report.files.length === 1 ? 'file' : 'files'} changed`);
    }
    if (report.commands.length > 0) {
      parts.push(`${report.commands.length} ${report.commands.length === 1 ? 'command' : 'commands'}`);
    }
    if (report.plan) parts.push(`${report.plan.completed}/${report.plan.total} steps`);
    parts.push(`${(report.tokens / 1000).toFixed(1)}k tokens`);
    return parts.join(' · ');
  }, [report]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(reportToText(report));
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };

  return (
    <section
      className={`turn-report-card ${collapsed ? 'is-collapsed' : ''}`}
      aria-label="Turn report"
      data-testid="turn-report-card"
    >
      <header className="turn-report-header">
        <button
          type="button"
          className="turn-report-toggle"
          onClick={() => setCollapsed((value) => !value)}
          aria-expanded={!collapsed}
        >
          <span className="turn-report-badge">Turn report</span>
          <span className="turn-report-headline">{headline}</span>
        </button>

        <div className="turn-report-actions">
          {report.denied > 0 && (
            <span className="turn-report-denied">{report.denied} denied</span>
          )}
          <button
            type="button"
            className={`turn-report-copy ${copied ? 'is-copied' : ''}`}
            onClick={copy}
            title={copied ? 'Copied report' : 'Copy report'}
            aria-label={copied ? 'Copied report' : 'Copy report'}
          >
            {copied ? <Check size={11} /> : <Copy size={11} />}
          </button>
          <ChevronDown
            size={13}
            className={`turn-report-chevron ${collapsed ? '' : 'is-open'}`}
            aria-hidden="true"
          />
        </div>
      </header>

      {!collapsed && (
        <div className="turn-report-body">
          {report.files.length > 0 && (
            <div className="turn-report-group">
              <div className="turn-report-group-title">Changed</div>
              <ul className="turn-report-rows">
                {visibleFiles.map((file, index) => (
                  <li key={`${file.path}-${index}`} className="turn-report-row">
                    <span className={`turn-report-file-icon is-${file.kind}`} aria-hidden="true">
                      {file.kind === 'wrote' ? (
                        <FilePlus size={12} />
                      ) : file.kind === 'edited' ? (
                        <FileEdit size={12} />
                      ) : (
                        <FileText size={12} />
                      )}
                    </span>
                    <span className="turn-report-path" title={file.path}>
                      {file.path}
                    </span>
                    <span className="turn-report-kind">{file.kind}</span>
                    {fileDelta(file) && (
                      <span className="turn-report-delta">{fileDelta(file)}</span>
                    )}
                  </li>
                ))}
              </ul>
              {hiddenFiles > 0 && (
                <div className="turn-report-more">+{hiddenFiles} more</div>
              )}
            </div>
          )}

          {report.commands.length > 0 && (
            <div className="turn-report-group">
              <div className="turn-report-group-title">Ran</div>
              <ul className="turn-report-rows">
                {visibleCommands.map((command, index) => (
                  <li key={`${command.command}-${index}`} className="turn-report-row">
                    <span className="turn-report-file-icon" aria-hidden="true">
                      <Terminal size={12} />
                    </span>
                    <code className="turn-report-command" title={command.command}>
                      {command.command}
                    </code>
                    {command.ok === false ? (
                      <span className="turn-report-failed">
                        <XCircle size={11} /> failed
                      </span>
                    ) : command.ok === true ? (
                      <span className="turn-report-ok">
                        <Check size={11} /> ok
                      </span>
                    ) : null}
                  </li>
                ))}
              </ul>
              {hiddenCommands > 0 && (
                <div className="turn-report-more">+{hiddenCommands} more</div>
              )}
            </div>
          )}

          <div className="turn-report-footline">
            {report.plan && (
              <span>
                Plan {report.plan.completed}/{report.plan.total}
              </span>
            )}
            {report.reads > 0 && <span>{report.reads} read</span>}
            {report.searches > 0 && <span>{report.searches} web</span>}
            {report.other.length > 0 && <span>{report.other.length} other</span>}
            <span>{report.tools} tools</span>
            <span>${report.costUsd.toFixed(3)}</span>
          </div>
        </div>
      )}
    </section>
  );
};