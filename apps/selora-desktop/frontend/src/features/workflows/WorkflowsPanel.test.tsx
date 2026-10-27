// Phase 8d — Workflows panel tests.
//
// The panel is store-driven: every assertion below starts from an empty store
// and only renders what an explicit `applyEvent` sequence reports. No fixture
// run exists in the module.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { WorkflowsPanel } from './WorkflowsPanel';
import {
  workflowsStore,
  type WorkflowEvent,
  type WorkflowFinishedEvent,
  type WorkflowReportEvent,
  type WorkflowStartedEvent,
} from './workflows-store';

function apply(...events: WorkflowEvent[]): void {
  act(() => {
    for (const event of events) workflowsStore.applyEvent(event);
  });
}

const started: WorkflowStartedEvent = {
  kind: 'started',
  run_id: 'run_1',
  name: 'Review Rust files',
  phases: ['Scan', 'Review'],
  max_cost_usd: 5,
};

beforeEach(() => {
  workflowsStore.setStopHandler(null);
  workflowsStore.setApprovalHandler(null);
  workflowsStore.setResumeHandler(null);
  workflowsStore.reset();
});

afterEach(() => {
  workflowsStore.setStopHandler(null);
  workflowsStore.setApprovalHandler(null);
  workflowsStore.setResumeHandler(null);
  workflowsStore.reset();
});

describe('WorkflowsPanel', () => {
  it('renders an empty state and no fabricated run data', () => {
    render(<WorkflowsPanel />);

    expect(screen.getByTestId('workflows-empty')).toBeTruthy();
    expect(screen.getByText('No workflow runs')).toBeTruthy();
    expect(workflowsStore.getSnapshot().runs).toEqual([]);
    expect(screen.queryByRole('button', { name: /Stop/ })).toBeNull();
    expect(screen.queryByRole('log')).toBeNull();
    expect(screen.queryByText(/Review Rust files/)).toBeNull();
  });

  it('ignores events for unknown runs instead of creating one', () => {
    render(<WorkflowsPanel />);

    apply(
      {
        kind: 'phase',
        run_id: 'run_missing',
        phase: 'Scan',
        status: 'running',
      },
      {
        kind: 'step',
        run_id: 'run_missing',
        step_id: 'step_missing',
        phase: 'Scan',
        label: 'Invented step',
        status: 'running',
      },
      {
        kind: 'finished',
        run_id: 'run_missing',
        status: 'completed',
      },
    );

    expect(workflowsStore.getSnapshot().runs).toEqual([]);
    expect(screen.getByTestId('workflows-empty')).toBeTruthy();
    expect(screen.queryByText('Invented step')).toBeNull();
  });

  it('renders a started/phase/step/report/artifact/finished sequence', () => {
    render(<WorkflowsPanel />);

    apply(started);
    expect(screen.getAllByText('Review Rust files').length).toBeGreaterThan(0);
    expect(screen.getByRole('button', { name: 'Stop' })).toBeTruthy();

    apply(
      { kind: 'phase', run_id: 'run_1', phase: 'Scan', status: 'running' },
      {
        kind: 'step',
        run_id: 'run_1',
        step_id: 'step_glob',
        phase: 'Scan',
        label: 'Glob src/**/*.rs',
        agent: 'scout',
        status: 'running',
      },
    );

    const graph = screen.getByRole('list', { name: 'Workflow phase graph' });
    expect(within(graph).getByText('Scan')).toBeTruthy();
    expect(within(graph).getByText('Running: Glob src/**/*.rs')).toBeTruthy();

    apply(
      {
        kind: 'step',
        run_id: 'run_1',
        step_id: 'step_glob',
        status: 'completed',
        detail: '14 files found',
      },
      { kind: 'phase', run_id: 'run_1', phase: 'Scan', status: 'completed' },
      { kind: 'phase', run_id: 'run_1', phase: 'Review', status: 'running' },
      {
        kind: 'step',
        run_id: 'run_1',
        step_id: 'step_review',
        phase: 'Review',
        label: 'Ask reviewer for findings',
        agent: 'reviewer',
        status: 'awaiting_approval',
      },
    );

    const steps = screen.getByRole('list', { name: 'Workflow steps' });
    expect(within(steps).getByText('Glob src/**/*.rs')).toBeTruthy();
    expect(within(steps).getByText('Ask reviewer for findings')).toBeTruthy();
    expect(within(steps).getByText('Awaiting approval')).toBeTruthy();
    expect(within(graph).getByText('1/1 steps complete')).toBeTruthy();

    apply(
      {
        kind: 'report',
        run_id: 'run_1',
        report_id: 'report_1',
        phase: 'Review',
        text: 'Found 12 unwrap() calls in 8 files.',
        artifact_id: 'artifact_md',
      } satisfies WorkflowReportEvent,
      {
        kind: 'artifact',
        run_id: 'run_1',
        artifact_id: 'artifact_md',
        artifact_kind: 'markdown',
        title: 'Review summary',
        content: '## Findings\n\n12 unwrap() calls.',
        phase: 'Review',
      },
      {
        kind: 'artifact',
        run_id: 'run_1',
        artifact_id: 'artifact_file',
        artifact_kind: 'file',
        title: 'Saved report',
        path: 'reports/review.md',
      },
      {
        kind: 'finished',
        run_id: 'run_1',
        status: 'completed',
        tokens: 1234,
        cost_usd: 0.42,
      } satisfies WorkflowFinishedEvent,
    );

    expect(screen.getByText('Found 12 unwrap() calls in 8 files.')).toBeTruthy();
    expect(screen.getByText('Review summary')).toBeTruthy();
    expect(screen.getByText(/## Findings/)).toBeTruthy();
    expect(screen.getByText('Saved report')).toBeTruthy();
    expect(screen.getByText('reports/review.md')).toBeTruthy();
    expect(screen.getByText('1,234')).toBeTruthy();
    expect(screen.getAllByText('$0.42').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Completed').length).toBeGreaterThan(0);

    const log = screen.getByRole('log', { name: 'Run log' });
    expect(within(log).getByText('Run started: Review Rust files')).toBeTruthy();
    expect(within(log).getByText('Run completed')).toBeTruthy();
    expect(within(log).getByText('Artifact (file): Saved report')).toBeTruthy();
  });

  it('emits one approval decision from the inline approval card', async () => {
    const onDecision = vi.fn();
    workflowsStore.setApprovalHandler(onDecision);
    render(<WorkflowsPanel />);

    apply(
      { kind: 'started', run_id: 'run_appr', name: 'Confirm findings' },
      { kind: 'phase', run_id: 'run_appr', phase: 'Confirm', status: 'running' },
      {
        kind: 'step',
        run_id: 'run_appr',
        step_id: 'step_write',
        phase: 'Confirm',
        label: 'Write review notes',
        status: 'awaiting_approval',
      },
      {
        kind: 'approval',
        run_id: 'run_appr',
        approval_id: 'approval_1',
        state: 'pending',
        summary: 'Write reports/review.md',
        step_id: 'step_write',
        tool_name: 'write_file',
      },
    );

    const approve = screen.getByRole('button', { name: 'Approve Write reports/review.md' });
    const deny = screen.getByRole('button', { name: 'Deny Write reports/review.md' });

    fireEvent.click(approve);
    expect(onDecision).toHaveBeenCalledWith('run_appr', 'approval_1', 'approve');

    fireEvent.click(deny);
    expect(onDecision).toHaveBeenCalledTimes(1);
    expect((approve as HTMLButtonElement).disabled).toBe(true);
    await act(async () => { await Promise.resolve(); });
    expect(workflowsStore.getSnapshot().runs[0]?.approvals[0]?.state).toBe('approved');
  });

  it('forwards Stop for the active run and marks the request as stopping', () => {
    const onStop = vi.fn();
    workflowsStore.setStopHandler(onStop);
    render(<WorkflowsPanel />);

    apply({ kind: 'started', run_id: 'run_stop', name: 'Long review' });

    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));

    expect(onStop).toHaveBeenCalledWith('run_stop');
    expect(workflowsStore.getSnapshot().runs[0]?.stop_requested).toBe(true);
    expect(screen.getByRole('button', { name: 'Stopping' })).toBeTruthy();
  });

  it('offers resume only for a durable run and restores the button on failure', async () => {
    const onResume = vi.fn((_runId: string, _allow: boolean) =>
      Promise.reject(new Error('journal changed')));
    workflowsStore.setResumeHandler(onResume);
    act(() => workflowsStore.hydratePersisted([{
      run_id: 'run_durable',
      name: 'Interrupted review',
      status: 'failed',
      resumable: true,
      detail: 'Interrupted before completion.',
    }]));
    render(<WorkflowsPanel />);

    fireEvent.click(screen.getByRole('button', { name: 'Resume' }));

    expect(onResume).toHaveBeenCalledWith('run_durable', false);
    expect(await screen.findByRole('alert')).toHaveProperty(
      'textContent',
      'journal changed',
    );
    expect(screen.getByRole('button', { name: 'Resume' })).toHaveProperty('disabled', false);
    expect(workflowsStore.getSnapshot().runs[0]?.resumable).toBe(true);
  });

  it('shows an explicit confirmation instead of silently retrying incomplete asks', async () => {
    const message =
      'Resume would dispatch 2 unfinished ask step(s) again: step 3 (reviewer: inspect), step 4 (writer). Tools in those steps may already have applied effects. Confirm to resume anyway.';
    const onResume = vi.fn((_runId: string, allow: boolean) =>
      allow
        ? Promise.resolve()
        : Promise.reject({ code: 'workflow_incomplete_asks', message, retryable: false }));
    workflowsStore.setResumeHandler(onResume);
    act(() => workflowsStore.hydratePersisted([{
      run_id: 'run_asks',
      name: 'Interrupted ask run',
      status: 'failed',
      resumable: true,
    }]));
    render(<WorkflowsPanel />);

    fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
    const warning = await screen.findByTestId('wf-resume-warning');

    expect(onResume).toHaveBeenCalledTimes(1);
    expect(onResume).toHaveBeenNthCalledWith(1, 'run_asks', false);
    expect(warning.textContent).toContain('2 unfinished steps may run again');
    expect(warning.textContent).toContain('may already have applied effects');
    expect(screen.queryByRole('button', { name: 'Resume' })).toBeNull();
    expect(workflowsStore.getSnapshot().runs[0]?.resume_confirmation).toEqual({
      count: 2,
      summary: message,
    });

    // Nothing retries on its own.
    await act(async () => { await Promise.resolve(); });
    expect(onResume).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole('button', { name: 'Resume anyway' }));
    await act(async () => { await Promise.resolve(); });

    expect(onResume).toHaveBeenCalledTimes(2);
    expect(onResume).toHaveBeenNthCalledWith(2, 'run_asks', true);
    expect(workflowsStore.getSnapshot().runs[0]?.resume_confirmation).toBeNull();
    expect(workflowsStore.getSnapshot().runs[0]?.resumable).toBe(false);
    expect(screen.queryByTestId('wf-resume-warning')).toBeNull();
  });

  it('clears the confirmation on cancel without a second resume call', async () => {
    const onResume = vi.fn(() =>
      Promise.reject({
        code: 'workflow_incomplete_asks',
        message: 'Resume would dispatch 1 unfinished ask step(s) again.',
        retryable: false,
      }));
    workflowsStore.setResumeHandler(onResume);
    act(() => workflowsStore.hydratePersisted([{
      run_id: 'run_asks_cancel',
      name: 'Interrupted ask run',
      status: 'failed',
      resumable: true,
    }]));
    render(<WorkflowsPanel />);

    fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
    await screen.findByTestId('wf-resume-warning');
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onResume).toHaveBeenCalledTimes(1);
    expect(workflowsStore.getSnapshot().runs[0]?.resume_confirmation).toBeNull();
    expect(workflowsStore.getSnapshot().runs[0]?.resumable).toBe(true);
    expect(screen.getByRole('button', { name: 'Resume' })).toBeTruthy();
  });

  it('maps recovered journal history into the existing run model', () => {
    act(() => workflowsStore.hydratePersisted([{
      run_id: 'run_history',
      name: 'Recovered completed review',
      status: 'completed',
      resumable: false,
      detail: null,
      phases: ['Scan'],
      steps: [{
        id: 'step-1',
        phase: '',
        label: 'inspect src/lib.rs',
        status: 'completed',
        agent: 'reviewer',
        detail: 'step-1',
      }],
      reports: [{ id: 'report-1', item: 'kept', artifact_id: 'summary' }],
      artifacts: [{ id: 'summary', title: 'Review summary', kind: 'markdown', body: '## Findings' }],
      tokens: 1234,
      cost_usd: 0.42,
    }]));
    render(<WorkflowsPanel />);

    const run = workflowsStore.getSnapshot().runs[0];
    expect(run?.status).toBe('completed');
    expect(run?.resumable).toBe(false);
    expect(run?.phases).toEqual([{ name: 'Scan', status: 'completed' }]);
    expect(run?.steps[0]).toMatchObject({
      id: 'step-1',
      phase: null,
      label: 'inspect src/lib.rs',
      agent: 'reviewer',
      status: 'completed',
    });
    expect(run?.reports[0]).toEqual({
      id: 'report-1',
      phase: null,
      text: 'kept',
      artifact_id: 'summary',
      at: null,
    });
    expect(run?.artifacts[0]).toEqual({
      id: 'summary',
      kind: 'markdown',
      title: 'Review summary',
      content: '## Findings',
      path: null,
      phase: null,
    });
    expect(run?.tokens).toBe(1234);
    expect(run?.cost_usd).toBe(0.42);
    expect(screen.queryByRole('button', { name: 'Resume' })).toBeNull();
    expect(screen.getByText('kept')).toBeTruthy();
    expect(screen.getByText(/## Findings/)).toBeTruthy();
    const log = screen.getByRole('log', { name: 'Run log' });
    expect(within(log).getByText('Recovered from journal: Recovered completed review')).toBeTruthy();
  });

  it('never offers resume for an ambiguous durable run', () => {
    workflowsStore.setResumeHandler(() => undefined);
    act(() => workflowsStore.hydratePersisted([{
      run_id: 'run_ambiguous',
      name: 'Ambiguous command run',
      status: 'failed',
      resumable: false,
      detail: 'An interrupted command has ambiguous side effects and cannot resume.',
    }]));
    render(<WorkflowsPanel />);

    expect(screen.queryByRole('button', { name: 'Resume' })).toBeNull();
  });
});
