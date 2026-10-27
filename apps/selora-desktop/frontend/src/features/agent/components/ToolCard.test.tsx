// Selora IDE — ToolCard Unit Tests
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { ToolCard } from './ToolCard';
import { agentStore, type WorkflowToolCall } from '../agent-store';
import type { AgentToolCall } from '../types';

const makePendingTool = (overrides: Partial<AgentToolCall> = {}): AgentToolCall => ({
  id: 'tool-pending',
  toolName: 'write_file',
  argsSummary: 'src/dangerous.rs',
  status: 'needs_approval',
  approvalId: 'appr-pending',
  startedAt: Date.now(),
  ...overrides,
});

describe('ToolCard Component', () => {
  it('renders one plan decision surface when old and new payloads both arrive', () => {
    const revise = vi.spyOn(agentStore, 'resolvePlan').mockResolvedValue(true);
    const deny = vi.spyOn(agentStore, 'handleApprovalDecision').mockImplementation(() => {});
    const tool: WorkflowToolCall = {
      ...makePendingTool({ toolName: 'exit_plan_mode' }),
      plan: { summary: 'Review', steps: ['Inspect'] },
      decisionPrompt: { kind: 'plan', summary: 'Review', steps: [{ title: 'Inspect' }] },
    };
    const { getByRole, queryAllByRole } = render(<ToolCard tool={tool} />);
    expect(queryAllByRole('button', { name: 'Approve plan' })).toHaveLength(1);
    fireEvent.click(getByRole('button', { name: 'Approve plan' }));
    expect(revise).toHaveBeenCalledTimes(1);
    expect(deny).not.toHaveBeenCalled();
    fireEvent.click(getByRole('button', { name: 'Deny plan' }));
    expect(deny).toHaveBeenCalledTimes(1);
    expect(deny).toHaveBeenCalledWith('appr-pending', 'deny');
  });
  beforeEach(() => {
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
      writable: true,
      configurable: true,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders tool header with name, target path chip, and duration badge', () => {
    const tool: AgentToolCall = {
      id: 'tool-1',
      toolName: 'write_file',
      argsSummary: 'src/main.rs',
      status: 'completed',
      durationMs: 420,
      startedAt: Date.now() - 500,
    };

    const { getByText, getByLabelText } = render(<ToolCard tool={tool} />);

    expect(getByText('write_file')).toBeTruthy();
    expect(getByText('src/main.rs')).toBeTruthy();
    expect(getByText('420ms')).toBeTruthy();
    expect(getByLabelText('Copy path')).toBeTruthy();
  });

  it('renders diff viewer with split and unified diff toggle when hunks are present', () => {
    const tool: AgentToolCall = {
      id: 'tool-2',
      toolName: 'edit_file',
      argsSummary: 'Cargo.toml',
      status: 'needs_approval',
      approvalId: 'appr-1',
      diff: {
        added: 2,
        removed: 1,
        hunks: [
          '@@ -10,3 +10,4 @@',
          ' [dependencies]',
          '-serde = "1.0"',
          '+serde = "1.0.210"',
          '+tokio = "1.36"',
        ],
      },
      startedAt: Date.now(),
    };

    const { getByText, getByRole } = render(<ToolCard tool={tool} />);

    expect(getByText('Needs Approval')).toBeTruthy();
    expect(getByText('+2')).toBeTruthy();
    expect(getByText('-1')).toBeTruthy();
    expect(getByText('serde = "1.0.210"')).toBeTruthy();

    // Toggle to split diff mode
    const splitBtn = getByRole('button', { name: /Split/i });
    fireEvent.click(splitBtn);

    expect(getByText('ORIGINAL')).toBeTruthy();
    expect(getByText('PROPOSED')).toBeTruthy();
  });

  it('triggers approval decisions on Accept, Reject, and Approve All buttons', () => {
    const decisionSpy = vi.spyOn(agentStore, 'handleApprovalDecision').mockImplementation(() => {});

    const tool: AgentToolCall = {
      id: 'tool-3',
      toolName: 'write_file',
      argsSummary: 'src/config.rs',
      status: 'needs_approval',
      approvalId: 'appr-99',
      startedAt: Date.now(),
    };

    const { getByRole } = render(<ToolCard tool={tool} />);

    // Click Accept (Enter)
    const acceptBtn = getByRole('button', { name: /Accept \(Enter\)/i });
    fireEvent.click(acceptBtn);
    expect(decisionSpy).toHaveBeenCalledWith('appr-99', 'approve');

    // Click Reject (Esc)
    const rejectBtn = getByRole('button', { name: /Reject \(Esc\)/i });
    fireEvent.click(rejectBtn);
    expect(decisionSpy).toHaveBeenCalledWith('appr-99', 'deny');

    // Click Approve All
    const approveAllBtn = getByRole('button', { name: /Approve All/i });
    fireEvent.click(approveAllBtn);
    expect(decisionSpy).toHaveBeenCalledWith('appr-99', 'approve_all');

    decisionSpy.mockRestore();
  });

  it('renders a run_workflow approval as the WorkflowCard grant and routes its decisions', () => {
    const decisionSpy = vi.spyOn(agentStore, 'handleApprovalDecision').mockImplementation(() => {});

    const script = "phase('scan');\nawait world.run('cargo', ['test']);\n";
    const tool: WorkflowToolCall = {
      id: 'tool-wf',
      toolName: 'run_workflow',
      argsSummary: 'repo-audit',
      status: 'needs_approval',
      approvalId: 'appr-wf',
      workflow: {
        name: 'repo-audit',
        script,
        commands: ['cargo test', 'git status'],
        max_cost_usd: 5,
      },
      startedAt: Date.now(),
    };

    const { getByTestId, getAllByTestId, getByText, queryByText } = render(
      <ToolCard tool={tool} />,
    );

    // The grant replaces the generic diff/command body entirely.
    expect(queryByText('Review proposed file changes before applying to workspace.')).toBeNull();
    expect(getByTestId('workflow-card-script').textContent).toBe(script);
    expect(getByTestId('workflow-card-cost').textContent).toContain('$5.00');
    expect(getByTestId('workflow-card').getAttribute('aria-label')).toBe(
      'Review workflow repo-audit',
    );
    expect(getByText('2 commands')).toBeTruthy();
    expect(getAllByTestId('workflow-card-command')).toHaveLength(2);

    // Approve/Deny ride the existing single-use decision callback.
    fireEvent.click(getByTestId('workflow-approve'));
    expect(decisionSpy).toHaveBeenCalledWith('appr-wf', 'approve');
    fireEvent.click(getByTestId('workflow-deny'));
    expect(decisionSpy).toHaveBeenCalledWith('appr-wf', 'deny');

    decisionSpy.mockRestore();
  });

  it('opens a card that becomes a pending approval after mounting as running', () => {
    const script = "phase('scan');\n";
    const running: WorkflowToolCall = {
      id: 'tool-wf-upgrade',
      toolName: 'run_workflow',
      argsSummary: 'repo-audit',
      status: 'running',
      workflow: {
        name: 'repo-audit',
        script,
        commands: ['cargo test'],
        max_cost_usd: 5,
      },
      startedAt: Date.now(),
    };

    const { rerender, getByTestId, queryByTestId } = render(<ToolCard tool={running} />);
    // While running there is no grant body (the card starts collapsed).
    expect(queryByTestId('workflow-card')).toBeNull();

    // The store upgrades the same card in place to `needs_approval`; it must
    // open so the user actually sees what they are approving.
    rerender(<ToolCard tool={{ ...running, status: 'needs_approval', approvalId: 'appr-wf' }} />);
    expect(getByTestId('workflow-card-script').textContent).toBe(script);
    expect(getByTestId('workflow-approve')).toBeTruthy();
  });

  it('records an already-decided run_workflow card instead of offering buttons', () => {
    const tool: WorkflowToolCall = {
      id: 'tool-wf-done',
      toolName: 'run_workflow',
      argsSummary: 'repo-audit',
      status: 'approved',
      approvalId: 'appr-wf',
      workflow: {
        name: 'repo-audit',
        script: 'report({ ok: true });\n',
        commands: [],
        max_cost_usd: 5,
      },
      startedAt: Date.now(),
    };

    const { getByTestId, getByText, queryByTestId } = render(<ToolCard tool={tool} />);

    // A decided card starts collapsed; the grant is one click away.
    expect(queryByTestId('workflow-card')).toBeNull();
    fireEvent.click(getByText('run_workflow'));

    expect(getByTestId('workflow-card').getAttribute('data-decision')).toBe('approve');
    expect(getByTestId('workflow-card-decision').textContent).toContain('Approved');
    expect(queryByTestId('workflow-approve')).toBeNull();
    expect(queryByTestId('workflow-deny')).toBeNull();
  });

  it('does not decide when Enter/Escape are pressed outside the expanded approval card', () => {
    const decisionSpy = vi.spyOn(agentStore, 'handleApprovalDecision').mockImplementation(() => {});

    const { getByTestId } = render(
      <div>
        <ToolCard tool={makePendingTool()} />
        <textarea data-testid="composer" aria-label="Composer" />
      </div>,
    );

    const card = getByTestId('tool-card-write_file');
    const composer = getByTestId('composer') as HTMLTextAreaElement;

    // The approval card is expanded by default (needs_approval), which used to
    // bypass the focus guard entirely.
    expect(card.className).toContain('is-expanded');

    composer.focus();
    expect(document.activeElement).toBe(composer);

    fireEvent.keyDown(composer, { key: 'Enter' });
    fireEvent.keyDown(composer, { key: 'Escape' });

    // Events dispatched globally (no card focus) must not decide either.
    fireEvent.keyDown(document.body, { key: 'Enter' });
    fireEvent.keyDown(document.body, { key: 'Escape' });

    expect(decisionSpy).not.toHaveBeenCalled();
  });

  it('approves on Enter and denies on Escape when the card itself is focused', () => {
    const decisionSpy = vi.spyOn(agentStore, 'handleApprovalDecision').mockImplementation(() => {});

    const { getByTestId } = render(<ToolCard tool={makePendingTool()} />);

    const card = getByTestId('tool-card-write_file');
    card.focus();
    expect(document.activeElement).toBe(card);

    fireEvent.keyDown(card, { key: 'Enter' });
    expect(decisionSpy).toHaveBeenCalledWith('appr-pending', 'approve');

    fireEvent.keyDown(card, { key: 'Escape' });
    expect(decisionSpy).toHaveBeenCalledWith('appr-pending', 'deny');

    expect(decisionSpy).toHaveBeenCalledTimes(2);
  });

  it('ignores modified or shifted Enter so global shortcuts never approve', () => {
    const decisionSpy = vi.spyOn(agentStore, 'handleApprovalDecision').mockImplementation(() => {});

    const { getByTestId } = render(<ToolCard tool={makePendingTool()} />);

    const card = getByTestId('tool-card-write_file');
    card.focus();

    fireEvent.keyDown(card, { key: 'Enter', ctrlKey: true });
    fireEvent.keyDown(card, { key: 'Enter', altKey: true });
    fireEvent.keyDown(card, { key: 'Enter', metaKey: true });
    fireEvent.keyDown(card, { key: 'Enter', shiftKey: true });

    expect(decisionSpy).not.toHaveBeenCalled();
  });
});
