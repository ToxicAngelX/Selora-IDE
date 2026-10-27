// Selora IDE — Phase 8 wave 2: protocol → workflows-store wiring.
//
// Two responsibilities, both narrow:
//
//   toWorkflowStoreEvent   translation of one `FrontendEvent` into the
//                          panel store's `WorkflowEvent`, or `null` for every
//                          non-workflow protocol event.
//                          Run starts bind the backend's originating workspace identity.
//   installWorkflowHandlers  registers the two outbound transports (Stop and
//                          Approve/Deny) against the live envelope context.
//
// The app store owns the event gate and calls the mapper from `reduceEvent`;
// this module never subscribes to Tauri and never invents progress.
import {
  resumeWorkflow,
  stopWorkflow,
  workflowDecision,
  type FrontendEvent,
  type WorkflowStatus as ProtocolWorkflowStatus,
} from '../../bridge/bridge';
import { currentEnvelopeContext } from '../../app/app-store';
import {
  workflowsStore,
  type WorkflowEvent as WorkflowStoreEvent,
  type WorkflowPhaseStatus,
  type WorkflowStepStatus,
} from './workflows-store';

export type { WorkflowStoreEvent };

/**
 * Protocol `WorkflowStatus` → phase status.
 *
 * The protocol status enum is shared by runs, phases and steps, so it carries
 * two values the phase graph has no home for: `awaiting_approval` is still a
 * running phase, and `stopped` becomes `skipped` (the run-level `finished`
 * event carries the stop).
 */
const PHASE_STATUS: Record<ProtocolWorkflowStatus, WorkflowPhaseStatus> = {
  pending: 'pending',
  running: 'running',
  awaiting_approval: 'running',
  completed: 'completed',
  failed: 'failed',
  stopped: 'skipped',
};

/** Steps do have an `awaiting_approval` state; only `stopped` has no home. */
const STEP_STATUS: Record<ProtocolWorkflowStatus, WorkflowStepStatus> = {
  pending: 'pending',
  running: 'running',
  awaiting_approval: 'awaiting_approval',
  completed: 'completed',
  failed: 'failed',
  stopped: 'skipped',
};

type TerminalRunStatus = 'completed' | 'failed' | 'stopped';

function toTerminalRunStatus(status: ProtocolWorkflowStatus): TerminalRunStatus | undefined {
  return status === 'completed' || status === 'failed' || status === 'stopped'
    ? status
    : undefined;
}

/**
 * Translate the seven `workflow_*` protocol events into store events.
 *
 * Contract (snake_case, no envelope fields):
 *
 *   workflow_started        → { kind: 'started', run_id, name, max_cost_usd, origin_context }
 *   workflow_phase          → { kind: 'phase', run_id, phase: payload.phase.name,
 *                               status: PHASE_STATUS[payload.phase.status] }
 *   workflow_step           → { kind: 'step', run_id, step_id: step.id, phase,
 *                               label, agent, status, detail }
 *   workflow_report         → { kind: 'report', run_id, report_id: report.id,
 *                               text: report.item, phase: report.phase ?? null,
 *                               artifact_id: report.artifact_id ?? null }
 *   workflow_artifact       → { kind: 'artifact', run_id, artifact_id: artifact.id,
 *                               artifact_kind: 'markdown' | 'file',
 *                               title, content, path, phase: artifact.phase ?? null }
 *   workflow_approval_needed→ { kind: 'approval', run_id, approval_id,
 *                               request_id, state: 'pending', summary, path_or_command, diff_or_input,
 *                               step_id: null, tool_name: null }
 *   workflow_finished       → { kind: 'finished', run_id, status?, tokens,
 *                               cost_usd, error }
 *
 * Field gaps in the protocol (documented rather than invented): `started`
 * carries no phase plan, approvals carry no `step_id`/`tool_name`, and
 * `finished.max_cost_usd` is not surfaced by the store's finished reducer.
 * Report and artifact phases are additive: absent means unknown, not "".
 */
export function toWorkflowStoreEvent(event: FrontendEvent): WorkflowStoreEvent | null {
  switch (event.type) {
    case 'workflow_started': {
      const ctx = currentEnvelopeContext();
      return {
        kind: 'started',
        run_id: event.payload.run_id,
        request_id: event.payload.request_id,
        origin_context: event.payload.workspace_id && ctx ? {
          workspace_id: event.payload.workspace_id,
          core_instance_id: ctx.core_instance_id,
        } : null,
        name: event.payload.name,
        max_cost_usd: event.payload.max_cost_usd,
      };
    }

    case 'workflow_phase':
      return {
        kind: 'phase',
        run_id: event.payload.run_id,
        phase: event.payload.phase.name,
        status: PHASE_STATUS[event.payload.phase.status],
      };

    case 'workflow_step': {
      const step = event.payload.step;
      return {
        kind: 'step',
        run_id: event.payload.run_id,
        step_id: step.id,
        phase: step.phase ?? null,
        label: step.label ?? null,
        agent: step.agent ?? null,
        status: STEP_STATUS[step.status],
        detail: step.detail ?? null,
      };
    }

    case 'workflow_report': {
      const report = event.payload.report;
      return {
        kind: 'report',
        run_id: event.payload.run_id,
        report_id: report.id,
        text: report.item,
        phase: report.phase && report.phase.trim() !== '' ? report.phase : null,
        artifact_id: report.artifact_id ?? null,
      };
    }

    case 'workflow_artifact': {
      const artifact = event.payload.artifact;
      const markdown = artifact.kind === 'markdown';
      return {
        kind: 'artifact',
        run_id: event.payload.run_id,
        artifact_id: artifact.id,
        artifact_kind: markdown ? 'markdown' : 'file',
        title: artifact.title || null,
        content: markdown ? artifact.body : null,
        // The protocol has no separate path field: for file artifacts the
        // body is the workspace-relative path.
        path: markdown ? null : artifact.body,
        phase: artifact.phase && artifact.phase.trim() !== '' ? artifact.phase : null,
      };
    }

    case 'workflow_approval_needed':
      return {
        kind: 'approval',
        run_id: event.payload.run_id,
        approval_id: event.payload.approval_id,
        request_id: event.payload.request_id,
        state: 'pending',
        summary: event.payload.summary,
        path_or_command: event.payload.path_or_command,
        diff_or_input: event.payload.diff_or_input ?? null,
        step_id: null,
        tool_name: null,
      };

    case 'workflow_finished': {
      const status = toTerminalRunStatus(event.payload.status);
      return {
        kind: 'finished',
        run_id: event.payload.run_id,
        ...(status ? { status } : {}),
        tokens: event.payload.tokens,
        cost_usd: event.payload.cost_usd,
        error: event.payload.error ?? null,
      };
    }

    default:
      return null;
  }
}

function runContext(runId: string) {
  const ctx = currentEnvelopeContext();
  if (!ctx?.workspace_id) throw new Error('Open the workflow workspace before sending this request.');
  const run = workflowsStore.getSnapshot().runs.find((item) => item.id === runId);
  if (!run) throw new Error('Workflow run is no longer available.');
  const origin = run.origin_context;
  if (!origin?.workspace_id) throw new Error('Workflow origin workspace is missing. Wait for an authoritative workflow_started event.');
  if (origin.workspace_id !== ctx.workspace_id || origin.core_instance_id !== ctx.core_instance_id) {
    throw new Error('Switch back to the workspace that started this workflow.');
  }
  return { run, ctx: { ...ctx, session_id: null } };
}

/**
 * Register the store's outbound transports once, from `startApp`.
 *
 * Stop: the protocol's `workflow_stop` command has no `run_id` (the envelope
 * targets the active run), so the store's run id stops here; the request still
 * rides the non-session envelope context (`session_id: null`).
 *
 * Approve/Deny: `workflow_decision` echoes the `request_id` recorded on the
 * approval event; a missing id rejects the request and keeps the card pending.
 */
export function installWorkflowHandlers(): void {
  workflowsStore.setStopHandler((runId) => {
    const { run, ctx } = runContext(runId);
    if (!run.request_id) throw new Error('Workflow stop request id is missing.');
    return stopWorkflow(run.request_id, ctx).promise;
  });

  workflowsStore.setApprovalHandler((runId, approvalId, decision) => {
    const { run, ctx } = runContext(runId);
    const requestId = run.approvals.find((item) => item.approval_id === approvalId)?.request_id;
    if (!requestId) throw new Error('Workflow approval request id is missing.');
    return workflowDecision(requestId, approvalId, decision, ctx);
  });

  workflowsStore.setResumeHandler((runId, allowIncompleteAsks, acknowledgePartialEffects) => {
    // A journal-recovered run has no live origin yet. The backend resolves the
    // run id only inside the currently open workspace, so binding here cannot
    // resume a same-id run from a different project.
    const ctx = currentEnvelopeContext();
    if (!ctx?.workspace_id) throw new Error('Open the workflow workspace before resuming.');
    const run = workflowsStore.getSnapshot().runs.find((item) => item.id === runId);
    if (!run) throw new Error('Workflow run is no longer available.');
    if (run.origin_context && (
      run.origin_context.workspace_id !== ctx.workspace_id ||
      run.origin_context.core_instance_id !== ctx.core_instance_id
    )) {
      throw new Error('Switch back to the workspace that started this workflow.');
    }
    return resumeWorkflow(runId, { ...ctx, session_id: null }, allowIncompleteAsks, acknowledgePartialEffects).promise;
  });
}
