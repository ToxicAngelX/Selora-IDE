// Phase 8 wave 2 — protocol → Workflows store bridge tests.
//
// Two contracts are pinned here:
//   1. `toWorkflowStoreEvent` translates exactly the seven `workflow_*`
//      protocol events (including the protocol/store status gaps) and returns
//      null for everything else.
//   2. `installWorkflowHandlers` connects the panel's Stop / Approve / Deny to
//      the bridge helpers with the non-session envelope context.
//
// The panel render test drives the same mapper the app store uses, so a full
// protocol sequence is proven to land in the DOM without any fixture run.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import type { FrontendEvent } from '../../bridge/bridge';

const appStoreMocks = vi.hoisted(() => {
  const context = { core_instance_id: 'core-1', workspace_id: 'ws-1', session_id: null };
  return {
    context,
    currentEnvelopeContext: vi.fn(
      (): { core_instance_id: string; workspace_id: string; session_id: string | null } | null =>
        context,
    ),
  };
});

const bridgeMocks = vi.hoisted(() => ({
  stopWorkflow: vi.fn((..._args: unknown[]) => ({
    requestId: 'req-stop',
    promise: Promise.resolve(),
  })),
  resumeWorkflow: vi.fn((..._args: unknown[]) => ({
    requestId: 'req-resume',
    promise: Promise.resolve(),
  })),
  workflowDecision: vi.fn((..._args: unknown[]) => Promise.resolve()),
  newRequestId: vi.fn(() => 'req-generated'),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('../../bridge/bridge', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../bridge/bridge')>();
  return { ...actual, ...bridgeMocks };
});
vi.mock('../../app/app-store', () => appStoreMocks);

import {
  installWorkflowHandlers,
  toWorkflowStoreEvent,
  type WorkflowStoreEvent,
} from './workflows-bridge';
import { workflowsStore } from './workflows-store';
import { WorkflowsPanel } from './WorkflowsPanel';

const CONTEXT = appStoreMocks.context;

const STARTED: FrontendEvent = {
  type: 'workflow_started',
  payload: { run_id: 'run_1', request_id: 'req_1', workspace_id: 'ws-1', name: 'Review Rust files', max_cost_usd: 5 },
};

const PHASE: FrontendEvent = {
  type: 'workflow_phase',
  payload: { run_id: 'run_1', phase: { name: 'Scan', status: 'running' } },
};

const STEP: FrontendEvent = {
  type: 'workflow_step',
  payload: {
    run_id: 'run_1',
    step: {
      id: 'step_glob',
      phase: 'Scan',
      label: 'Glob src/**/*.rs',
      status: 'running',
      agent: 'scout',
      detail: '14 files found',
    },
  },
};

const REPORT: FrontendEvent = {
  type: 'workflow_report',
  payload: {
    run_id: 'run_1',
    report: { id: 'report_1', item: 'Found 12 unwrap() calls.', artifact_id: 'artifact_md' },
  },
};

const ARTIFACT: FrontendEvent = {
  type: 'workflow_artifact',
  payload: {
    run_id: 'run_1',
    artifact: { id: 'artifact_md', title: 'Review summary', kind: 'markdown', body: '## Findings' },
  },
};

const APPROVAL: FrontendEvent = {
  type: 'workflow_approval_needed',
  payload: {
    run_id: 'run_1',
    request_id: 'req_2',
    approval_id: 'approval_1',
    summary: 'Write reports/review.md',
    path_or_command: 'reports/review.md',
    diff_or_input: '--- reports/review.md\n+++ reports/review.md\n@@ -1 +1 @@\n-old\n+new\n',
    can_approve_all_this_turn: false,
  },
};

const FINISHED: FrontendEvent = {
  type: 'workflow_finished',
  payload: {
    run_id: 'run_1',
    request_id: 'req_1',
    status: 'completed',
    tokens: 1234,
    cost_usd: 0.42,
    max_cost_usd: 5,
  },
};

function map(event: FrontendEvent): WorkflowStoreEvent {
  const mapped = toWorkflowStoreEvent(event);
  if (!mapped) throw new Error(`expected a store event for ${event.type}`);
  return mapped;
}

/** Apply protocol events exactly as `app-store.reduceEvent` does. */
function apply(...events: FrontendEvent[]): void {
  act(() => {
    for (const event of events) workflowsStore.applyEvent(map(event));
  });
}

function deferred() {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<void>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flush() {
  await act(async () => { await Promise.resolve(); });
}

beforeEach(() => {
  vi.clearAllMocks();
  appStoreMocks.currentEnvelopeContext.mockReturnValue(CONTEXT);
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

describe('toWorkflowStoreEvent', () => {
  it('maps workflow_started without inventing a phase plan', () => {
    expect(map(STARTED)).toEqual({
      kind: 'started',
      run_id: 'run_1',
      request_id: 'req_1',
      origin_context: { workspace_id: 'ws-1', core_instance_id: 'core-1' },
      name: 'Review Rust files',
      max_cost_usd: 5,
    });
  });

  it('maps workflow_phase and clamps the protocol-only statuses', () => {
    expect(map(PHASE)).toEqual({
      kind: 'phase',
      run_id: 'run_1',
      phase: 'Scan',
      status: 'running',
    });
    expect(
      map({
        type: 'workflow_phase',
        payload: { run_id: 'run_1', phase: { name: 'Confirm', status: 'awaiting_approval' } },
      }),
    ).toMatchObject({ status: 'running' });
    expect(
      map({
        type: 'workflow_phase',
        payload: { run_id: 'run_1', phase: { name: 'Review', status: 'stopped' } },
      }),
    ).toMatchObject({ status: 'skipped' });
  });

  it('maps workflow_step from the nested step DTO', () => {
    expect(map(STEP)).toEqual({
      kind: 'step',
      run_id: 'run_1',
      step_id: 'step_glob',
      phase: 'Scan',
      label: 'Glob src/**/*.rs',
      agent: 'scout',
      status: 'running',
      detail: '14 files found',
    });
    expect(
      map({
        type: 'workflow_step',
        payload: {
          run_id: 'run_1',
          step: { id: 'step_write', phase: 'Scan', label: 'Write notes', status: 'awaiting_approval' },
        },
      }),
    ).toMatchObject({ status: 'awaiting_approval', agent: null, detail: null });
    expect(
      map({
        type: 'workflow_step',
        payload: {
          run_id: 'run_1',
          step: { id: 'step_stop', phase: 'Scan', label: 'Interrupted', status: 'stopped' },
        },
      }),
    ).toMatchObject({ status: 'skipped' });
  });

  it('maps workflow_report item → text and keeps the artifact reference', () => {
    expect(map(REPORT)).toEqual({
      kind: 'report',
      run_id: 'run_1',
      report_id: 'report_1',
      text: 'Found 12 unwrap() calls.',
      phase: null,
      artifact_id: 'artifact_md',
    });
    expect(
      map({
        type: 'workflow_report',
        payload: { run_id: 'run_1', report: { id: 'report_2', item: 'No findings.' } },
      }),
    ).toMatchObject({ artifact_id: null });
  });

  it('maps an additive report phase and artifact phase without inventing one when absent', () => {
    expect(
      map({
        type: 'workflow_report',
        payload: {
          run_id: 'run_1',
          report: { id: 'report_2', item: 'Scoped finding.', phase: 'Scan' },
        },
      }),
    ).toMatchObject({ phase: 'Scan' });
    expect(
      map({
        type: 'workflow_artifact',
        payload: {
          run_id: 'run_1',
          artifact: {
            id: 'artifact_md',
            title: 'Summary',
            kind: 'markdown',
            body: '## Findings',
            phase: 'Review',
          },
        },
      }),
    ).toMatchObject({ phase: 'Review' });
    expect(
      map({
        type: 'workflow_report',
        payload: { run_id: 'run_1', report: { id: 'report_3', item: 'No phase.', phase: '' } },
      }),
    ).toMatchObject({ phase: null });
  });

  it('maps workflow_artifact markdown bodies to content and file bodies to paths', () => {
    expect(map(ARTIFACT)).toEqual({
      kind: 'artifact',
      run_id: 'run_1',
      artifact_id: 'artifact_md',
      artifact_kind: 'markdown',
      title: 'Review summary',
      content: '## Findings',
      path: null,
      phase: null,
    });
    expect(
      map({
        type: 'workflow_artifact',
        payload: {
          run_id: 'run_1',
          artifact: {
            id: 'artifact_file',
            title: 'Saved report',
            kind: 'file',
            body: 'reports/review.md',
          },
        },
      }),
    ).toEqual({
      kind: 'artifact',
      run_id: 'run_1',
      artifact_id: 'artifact_file',
      artifact_kind: 'file',
      title: 'Saved report',
      content: null,
      path: 'reports/review.md',
      phase: null,
    });
  });

  it('maps workflow_approval_needed to a pending approval with its request id', () => {
    expect(map(APPROVAL)).toEqual({
      kind: 'approval',
      run_id: 'run_1',
      approval_id: 'approval_1',
      request_id: 'req_2',
      state: 'pending',
      summary: 'Write reports/review.md',
      path_or_command: 'reports/review.md',
      diff_or_input: '--- reports/review.md\n+++ reports/review.md\n@@ -1 +1 @@\n-old\n+new\n',
      step_id: null,
      tool_name: null,
    });
  });

  it('maps workflow_finished and leaves non-terminal statuses to the store default', () => {
    expect(map(FINISHED)).toEqual({
      kind: 'finished',
      run_id: 'run_1',
      status: 'completed',
      tokens: 1234,
      cost_usd: 0.42,
      error: null,
    });
    expect(
      map({
        type: 'workflow_finished',
        payload: {
          run_id: 'run_1',
          request_id: 'req_1',
          status: 'failed',
          tokens: 3,
          cost_usd: 0.01,
          max_cost_usd: 5,
          error: 'provider timeout',
        },
      }),
    ).toMatchObject({ status: 'failed', error: 'provider timeout' });
    expect(
      map({
        type: 'workflow_finished',
        payload: {
          run_id: 'run_1',
          request_id: 'req_1',
          status: 'running',
          tokens: 0,
          cost_usd: 0,
          max_cost_usd: 5,
        },
      }),
    ).toEqual({ kind: 'finished', run_id: 'run_1', tokens: 0, cost_usd: 0, error: null });
  });

  it('returns null for every non-workflow protocol event', () => {
    expect(
      toWorkflowStoreEvent({ type: 'toast', payload: { severity: 'info', message: 'hello' } }),
    ).toBeNull();
    expect(
      toWorkflowStoreEvent({
        type: 'chat_delta',
        payload: {
          request_id: 'r',
          session_id: 's',
          turn_id: 't',
          message_id: 'm',
          sequence: 1,
          text: 'x',
        },
      }),
    ).toBeNull();
    // Agent approvals are a different surface entirely.
    expect(
      toWorkflowStoreEvent({
        type: 'approval_needed',
        payload: {
          request_id: 'r',
          session_id: 's',
          turn_id: 't',
          approval_id: 'a',
          tool_name: 'write_file',
          path_or_command: 'x',
        },
      }),
    ).toBeNull();
    expect(
      toWorkflowStoreEvent({ type: 'error', payload: { code: 'x', message: 'y', retryable: false } }),
    ).toBeNull();
  });
});

describe('workflows-bridge wiring', () => {
  it('binds a delayed workflow_started to its authoritative workspace rather than the current workspace', async () => {
    installWorkflowHandlers();
    appStoreMocks.currentEnvelopeContext.mockReturnValue({ ...CONTEXT, workspace_id: 'ws-other' });
    render(<WorkflowsPanel />);
    apply(STARTED, APPROVAL);
    expect(workflowsStore.getSnapshot().runs[0]?.origin_context).toEqual({ workspace_id: 'ws-1', core_instance_id: 'core-1' });
    fireEvent.click(screen.getByRole('button', { name: 'Approve Write reports/review.md' }));
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(bridgeMocks.workflowDecision).not.toHaveBeenCalled();
    expect(bridgeMocks.stopWorkflow).not.toHaveBeenCalled();
    expect(screen.getAllByRole('alert').every((alert) => alert.textContent?.includes('workspace that started'))).toBe(true);
    appStoreMocks.currentEnvelopeContext.mockReturnValue(CONTEXT);
    fireEvent.click(screen.getByRole('button', { name: 'Approve Write reports/review.md' }));
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await flush();
    expect(bridgeMocks.workflowDecision).toHaveBeenCalledWith('req_2', 'approval_1', 'approve', CONTEXT);
    expect(bridgeMocks.stopWorkflow).toHaveBeenCalledWith('req_1', CONTEXT);
  });

  it('fails closed for legacy workflow_started events without an originating workspace id', async () => {
    installWorkflowHandlers();
    render(<WorkflowsPanel />);
    apply({ type: 'workflow_started', payload: {
      run_id: 'run_1', request_id: 'req_1', name: 'Legacy run', max_cost_usd: 5,
    } }, APPROVAL);
    expect(workflowsStore.getSnapshot().runs[0]?.origin_context).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Approve Write reports/review.md' }));
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await flush();
    expect(bridgeMocks.workflowDecision).not.toHaveBeenCalled();
    expect(bridgeMocks.stopWorkflow).not.toHaveBeenCalled();
    expect(screen.getAllByRole('alert').every((alert) => alert.textContent?.includes('origin workspace is missing'))).toBe(true);
    expect(workflowsStore.getSnapshot().runs[0]?.approvals[0]?.state).toBe('pending');
    expect(workflowsStore.getSnapshot().runs[0]?.stop_requested).toBe(false);
  });

  it('preserves and renders exact approval diff and command text without inventing a step', () => {
    render(<WorkflowsPanel />);
    apply(STARTED, APPROVAL);
    expect(screen.getByLabelText('Path or command').textContent).toBe('reports/review.md');
    expect(screen.getByLabelText('Diff or input').textContent).toBe(
      '--- reports/review.md\n+++ reports/review.md\n@@ -1 +1 @@\n-old\n+new\n');
    expect(workflowsStore.getSnapshot().runs[0]?.approvals[0]?.step_id).toBeNull();
    expect(workflowsStore.getSnapshot().runs[0]?.steps).toEqual([]);

    const command = 'bash -lc "printf \'hello  world\\n\'"';
    apply({ type: 'workflow_approval_needed', payload: {
      run_id: 'run_1', request_id: 'req-bash', approval_id: 'approval-bash', summary: 'Run shell command',
      path_or_command: command, diff_or_input: 'stdin\n  exact spaces\n', can_approve_all_this_turn: false,
    } });
    expect(screen.getAllByLabelText('Path or command')[1]?.textContent).toBe(command);
    expect(screen.getAllByLabelText('Diff or input')[1]?.textContent).toBe('stdin\n  exact spaces\n');
  });

  it('dedupes approval clicks, keeps the card pending on rejection, and allows retry', async () => {
    installWorkflowHandlers();
    render(<WorkflowsPanel />);
    apply(STARTED, APPROVAL);
    const dispatch = deferred();
    bridgeMocks.workflowDecision.mockReturnValueOnce(dispatch.promise);
    const approve = screen.getByRole('button', { name: 'Approve Write reports/review.md' });
    const deny = screen.getByRole('button', { name: 'Deny Write reports/review.md' });
    fireEvent.click(approve);
    fireEvent.click(approve);
    fireEvent.click(deny);
    expect(bridgeMocks.workflowDecision).toHaveBeenCalledTimes(1);
    expect((approve as HTMLButtonElement).disabled).toBe(true);
    expect((deny as HTMLButtonElement).disabled).toBe(true);
    expect(workflowsStore.getSnapshot().runs[0]?.approvals[0]?.state).toBe('pending');

    await act(async () => { dispatch.reject({ message: 'Approval service unavailable' }); });
    expect(screen.getByRole('alert').textContent).toContain('Approval service unavailable');
    expect((approve as HTMLButtonElement).disabled).toBe(false);
    expect(workflowsStore.getSnapshot().runs[0]?.approvals[0]?.state).toBe('pending');
    fireEvent.click(deny);
    await flush();
    expect(bridgeMocks.workflowDecision).toHaveBeenCalledTimes(2);
    expect(workflowsStore.getSnapshot().runs[0]?.approvals[0]?.state).toBe('denied');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('dedupes Stop and re-enables it after a rejected transport', async () => {
    installWorkflowHandlers();
    render(<WorkflowsPanel />);
    apply(STARTED);
    const dispatch = deferred();
    bridgeMocks.stopWorkflow.mockReturnValueOnce({ requestId: 'req_1', promise: dispatch.promise });
    const stop = screen.getByRole('button', { name: 'Stop' });
    fireEvent.click(stop);
    fireEvent.click(stop);
    expect(bridgeMocks.stopWorkflow).toHaveBeenCalledTimes(1);
    expect(workflowsStore.getSnapshot().runs[0]?.status).toBe('running');
    await act(async () => { dispatch.reject(new Error('Stop unavailable')); });
    expect(screen.getByRole('alert').textContent).toContain('Stop unavailable');
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await flush();
    expect(bridgeMocks.stopWorkflow).toHaveBeenCalledTimes(2);
    expect(workflowsStore.getSnapshot().runs[0]?.stop_requested).toBe(true);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('refreshes context at click time and refuses a different originating workspace', async () => {
    installWorkflowHandlers();
    render(<WorkflowsPanel />);
    apply(STARTED, APPROVAL);
    appStoreMocks.currentEnvelopeContext.mockReturnValue({ ...CONTEXT, workspace_id: 'ws-other' });
    fireEvent.click(screen.getByRole('button', { name: 'Approve Write reports/review.md' }));
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(bridgeMocks.workflowDecision).not.toHaveBeenCalled();
    expect(bridgeMocks.stopWorkflow).not.toHaveBeenCalled();
    expect(screen.getAllByRole('alert').every((alert) => alert.textContent?.includes('workspace that started'))).toBe(true);
    appStoreMocks.currentEnvelopeContext.mockReturnValue(CONTEXT);
    fireEvent.click(screen.getByRole('button', { name: 'Approve Write reports/review.md' }));
    await flush();
    expect(bridgeMocks.workflowDecision).toHaveBeenCalledWith('req_2', 'approval_1', 'approve', CONTEXT);
  });

  it('refuses missing request ids instead of creating fresh requests', async () => {
    installWorkflowHandlers();
    render(<WorkflowsPanel />);
    act(() => {
      workflowsStore.applyEvent({ kind: 'started', run_id: 'run_1', name: 'Unknown request' });
      workflowsStore.applyEvent({ kind: 'approval', run_id: 'run_1', approval_id: 'approval_1',
        state: 'pending', summary: 'Unknown approval' });
    });
    fireEvent.click(screen.getByRole('button', { name: 'Approve Unknown approval' }));
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await flush();
    expect(bridgeMocks.workflowDecision).not.toHaveBeenCalled();
    expect(bridgeMocks.stopWorkflow).not.toHaveBeenCalled();
    expect(bridgeMocks.newRequestId).not.toHaveBeenCalled();
    expect(workflowsStore.getSnapshot().runs[0]?.approvals[0]?.state).toBe('pending');
    expect(workflowsStore.getSnapshot().runs[0]?.stop_requested).toBe(false);
    expect(screen.getAllByRole('alert')).toHaveLength(2);
  });

  it.each(['completed', 'failed', 'stopped'] as const)('ignores late approvals after a %s terminal event', async (status) => {
    installWorkflowHandlers();
    render(<WorkflowsPanel />);
    apply(STARTED, { type: 'workflow_finished', payload: {
      run_id: 'run_1', request_id: 'req_1', status, tokens: 0, cost_usd: 0, max_cost_usd: 5,
    } }, APPROVAL);
    expect(workflowsStore.getSnapshot().runs[0]?.approvals).toEqual([]);
    expect(screen.queryByRole('button', { name: 'Approve Write reports/review.md' })).toBeNull();
    expect(workflowsStore.getSnapshot().runs[0]?.status).toBe(status);
    await flush();
  });

  it('does not reopen a decided approval when the pending event is replayed', async () => {
    installWorkflowHandlers();
    render(<WorkflowsPanel />);
    apply(STARTED, APPROVAL);
    fireEvent.click(screen.getByRole('button', { name: 'Approve Write reports/review.md' }));
    await flush();
    apply(APPROVAL);
    expect(workflowsStore.getSnapshot().runs[0]?.approvals[0]?.state).toBe('approved');
    expect(workflowsStore.getSnapshot().runs[0]?.status).toBe('running');
  });

  it('renders a full started/phase/step/report/artifact/approval/finished sequence', () => {
    render(<WorkflowsPanel />);
    apply(STARTED, PHASE, STEP, REPORT, ARTIFACT, APPROVAL, FINISHED);

    expect(screen.getAllByText('Review Rust files').length).toBeGreaterThan(0);
    expect(screen.getByText('Glob src/**/*.rs')).toBeTruthy();
    expect(screen.getByText('Found 12 unwrap() calls.')).toBeTruthy();
    expect(screen.getByText('Review summary')).toBeTruthy();
    expect(screen.getByText(/## Findings/)).toBeTruthy();
    expect(screen.getByText('Write reports/review.md')).toBeTruthy();
    expect(screen.getByText('1,234')).toBeTruthy();
    expect(screen.getAllByText('$0.42').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Completed').length).toBeGreaterThan(0);

    const log = screen.getByRole('log', { name: 'Run log' });
    expect(within(log).getByText('Run started: Review Rust files')).toBeTruthy();
    expect(within(log).getByText('Run completed')).toBeTruthy();
    expect(within(log).getByText('Approval needed: Write reports/review.md')).toBeTruthy();
  });

  it('forwards Stop to stopWorkflow with the session-less envelope context', () => {
    installWorkflowHandlers();
    expect(workflowsStore.getSnapshot().stop_wired).toBe(true);
    render(<WorkflowsPanel />);
    apply(STARTED);

    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));

    expect(bridgeMocks.stopWorkflow).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.stopWorkflow).toHaveBeenCalledWith('req_1', CONTEXT);
    expect(CONTEXT.session_id).toBeNull();
  });

  it.each(['approve', 'deny'] as const)('forwards %s to workflowDecision with the approval request id', (decision) => {
    installWorkflowHandlers();
    expect(workflowsStore.getSnapshot().approval_wired).toBe(true);
    render(<WorkflowsPanel />);
    apply(STARTED, APPROVAL);

    fireEvent.click(screen.getByRole('button', { name: `${decision === 'approve' ? 'Approve' : 'Deny'} Write reports/review.md` }));
    expect(bridgeMocks.workflowDecision).toHaveBeenNthCalledWith(
      1,
      'req_2',
      'approval_1',
      decision,
      CONTEXT,
    );
  });

  it('forwards the explicit incomplete-asks confirmation through resumeWorkflow', async () => {
    const message =
      'Resume would dispatch 1 unfinished ask step(s) again: step 2 (reviewer). Tools in those steps may already have applied effects. Confirm to resume anyway.';
    bridgeMocks.resumeWorkflow.mockImplementationOnce(() => ({
      requestId: 'req-resume-refused',
      promise: Promise.reject({
        code: 'workflow_incomplete_asks',
        message,
        retryable: false,
      }),
    }));
    installWorkflowHandlers();
    act(() => workflowsStore.hydratePersisted([{
      run_id: 'run_journal',
      name: 'Recovered review',
      status: 'failed',
      resumable: true,
    }]));
    render(<WorkflowsPanel />);

    fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
    await screen.findByTestId('wf-resume-warning');
    expect(bridgeMocks.resumeWorkflow).toHaveBeenNthCalledWith(
      1,
      'run_journal',
      { ...CONTEXT, session_id: null },
      false,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Resume anyway' }));
    await flush();
    expect(bridgeMocks.resumeWorkflow).toHaveBeenNthCalledWith(
      2,
      'run_journal',
      { ...CONTEXT, session_id: null },
      true,
    );
  });

  it('does not dispatch when no envelope context is available', () => {
    appStoreMocks.currentEnvelopeContext.mockReturnValue(null);
    installWorkflowHandlers();
    render(<WorkflowsPanel />);
    apply(STARTED, APPROVAL);

    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    fireEvent.click(screen.getByRole('button', { name: 'Approve Write reports/review.md' }));

    expect(bridgeMocks.stopWorkflow).not.toHaveBeenCalled();
    expect(bridgeMocks.workflowDecision).not.toHaveBeenCalled();
  });
});
