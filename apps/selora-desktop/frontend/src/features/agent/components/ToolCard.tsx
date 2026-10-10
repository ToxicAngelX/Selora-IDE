// Selora IDE — ZCode Parity Tool & Approval Card (UI_SPEC §4.2, PHASE5D §A)
//
// Renders read, edit, write, bash, web tool steps with collapsible outputs,
// ANSI stream containers, and side-effect approval diffs.
import React, { useState } from 'react';
import {
  FileText,
  FileEdit,
  FilePlus,
  Terminal,
  Search,
  Globe,
  Code2,
  Check,
  X,
  ChevronDown,
  ChevronRight,
  ShieldAlert,
} from '../../../components/icons';
import type { AgentToolCall } from '../types';
import { agentStore } from '../agent-store';

export type ToolCardProps = {
  tool: AgentToolCall;
};

export const ToolCard: React.FC<ToolCardProps> = ({ tool }) => {
  const [expanded, setExpanded] = useState(tool.status === 'needs_approval');

  const renderToolIcon = () => {
    const iconProps = { size: 14, className: 'tool-icon-svg' };
    switch (tool.toolName) {
      case 'read_file':
        return <FileText {...iconProps} />;
      case 'edit_file':
        return <FileEdit {...iconProps} />;
      case 'write_file':
        return <FilePlus {...iconProps} />;
      case 'bash':
        return <Terminal {...iconProps} />;
      case 'web_search':
        return <Search {...iconProps} />;
      case 'web_fetch':
        return <Globe {...iconProps} />;
      default:
        return <Code2 {...iconProps} />;
    }
  };

  const isRunning = tool.status === 'running';
  const isApproval = tool.status === 'needs_approval';
  const isError = tool.status === 'error' || tool.status === 'denied';
  const isSuccess = tool.status === 'completed' || tool.status === 'approved';

  return (
    <div
      className={`agent-tool-card tool-${tool.toolName} status-${tool.status} ${expanded ? 'is-expanded' : ''}`}
    >
      {/* Header Row */}
      <div
        className="tool-card-header"
        onClick={() => setExpanded(!expanded)}
        role="button"
        tabIndex={0}
      >
        <span className="tool-chevron-icon" aria-hidden="true">
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </span>
        <span className="tool-icon">{renderToolIcon()}</span>
        <span className="tool-name">{tool.toolName}</span>
        <span className="tool-args-summary">{tool.argsSummary}</span>

        {/* Diff badge if present */}
        {tool.diff && (
          <span className="tool-diff-badge">
            <span className="diff-add">+{tool.diff.added}</span>
            <span className="diff-remove">-{tool.diff.removed}</span>
          </span>
        )}

        {/* Status Indicator */}
        <span className="tool-status-glyph">
          {isRunning && <span className="tool-spinner" />}
          {isApproval && (
            <span className="tool-approval-pill">
              <ShieldAlert size={11} style={{ marginRight: 3 }} />
              Review
            </span>
          )}
          {isSuccess && <span className="tool-glyph-ok"><Check size={12} /></span>}
          {isError && <span className="tool-glyph-err"><X size={12} /></span>}
        </span>
      </div>

      {/* Expanded Content Drawer */}
      {expanded && (
        <div className="tool-card-body">
          {/* Output / Result text */}
          {tool.resultSummary && (
            <div className="tool-result-box">
              <pre className="tool-result-pre">{tool.resultSummary}</pre>
            </div>
          )}

          {/* Interactive Approval Gate */}
          {isApproval && tool.approvalId && (
            <div className="tool-approval-gate">
              <div className="approval-banner">
                <span className="approval-badge">Needs Approval</span>
                <span className="approval-desc">
                  This action proposes changes to your project.
                </span>
              </div>

              {/* Diff Preview if edit_file / write_file */}
              {tool.diff?.hunks && (
                <div className="approval-diff-viewer">
                  {tool.diff.hunks.map((line, idx) => {
                    const isAdd = line.startsWith('+');
                    const isSub = line.startsWith('-');
                    return (
                      <div
                        key={idx}
                        className={`diff-line ${isAdd ? 'line-add' : isSub ? 'line-sub' : 'line-ctx'}`}
                      >
                        <span className="line-text">{line}</span>
                      </div>
                    );
                  })}
                </div>
              )}

              {/* Action Buttons */}
              <div className="approval-actions">
                <button
                  type="button"
                  className="btn-deny"
                  onClick={() =>
                    agentStore.handleApprovalDecision(tool.approvalId!, 'deny')
                  }
                >
                  Deny
                </button>
                <button
                  type="button"
                  className="btn-approve-all"
                  onClick={() =>
                    agentStore.handleApprovalDecision(tool.approvalId!, 'approve_all')
                  }
                >
                  Approve All
                </button>
                <button
                  type="button"
                  className="btn-approve"
                  onClick={() =>
                    agentStore.handleApprovalDecision(tool.approvalId!, 'approve')
                  }
                >
                  Approve
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
};
