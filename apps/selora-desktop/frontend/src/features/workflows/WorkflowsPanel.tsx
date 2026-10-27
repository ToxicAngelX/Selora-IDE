// Selora IDE — Phase 8d Workflows panel.
//
// Self-contained, store-driven surface: run list, phase graph (one node per
// phase with live step status), step rows, approvals, reports, artifacts
// (markdown + file), the run log, and Stop. Nothing is mounted here — the
// shell wiring slice owns the region/activity placement.
//
// Truthfulness rules: an empty store renders an empty state, section bodies
// state that nothing has been reported yet, and every action is forwarded to
// the store (which forwards to the shell). The panel never invents a run,
// step, approval, or artifact.
import { useId, useSyncExternalStore } from 'react';
import { StopIcon } from '../../components/icons';
import {
  isTerminalRunStatus,
  workflowsStore,
  type WorkflowApproval,
  type WorkflowApprovalDecision,
  type WorkflowArtifact,
  type WorkflowPhase,
  type WorkflowPhaseStatus,
  type WorkflowRun,
  type WorkflowRunStatus,
  type WorkflowStep,
  type WorkflowStepStatus,
  type WorkflowsState,
  type WorkflowsStore,
} from './workflows-store';
import './workflows.css';

const RUN_STATUS_LABEL: Record<WorkflowRunStatus, string> = {
  queued: 'Queued',
  running: 'Running',
  awaiting_approval: 'Awaiting approval',
  completed: 'Completed',
  failed: 'Failed',
  stopped: 'Stopped',
};

const PHASE_STATUS_LABEL: Record<WorkflowPhaseStatus, string> = {
  pending: 'Pending',
  running: 'Running',
  completed: 'Completed',
  failed: 'Failed',
  skipped: 'Skipped',
};

const STEP_STATUS_LABEL: Record<WorkflowStepStatus, string> = {
  pending: 'Pending',
  running: 'Running',
  completed: 'Completed',
  failed: 'Failed',
  skipped: 'Skipped',
  awaiting_approval: 'Awaiting approval',
};

const NUMBER_FORMAT = new Intl.NumberFormat('en-US');
const USD_FORMAT = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

function runStatusLabel(run: WorkflowRun): string {
  if (run.stop_requested && !isTerminalRunStatus(run.status)) return 'Stopping';
  return RUN_STATUS_LABEL[run.status];
}

function PhaseNode({
  phase,
  steps,
  current,
}: {
  phase: WorkflowPhase;
  steps: WorkflowStep[];
  current: boolean;
}) {
  const phaseSteps = steps.filter((step) => step.phase === phase.name);
  const runningStep = phaseSteps.find((step) => step.status === 'running');
  const done = phaseSteps.filter(
    (step) => step.status === 'completed' || step.status === 'skipped',
  ).length;
  const detail = runningStep
    ? `Running: ${runningStep.label}`
    : phaseSteps.length > 0
      ? `${done}/${phaseSteps.length} steps complete`
      : 'No steps reported yet';
  return (
    <li
      className="wf-graph-node"
      data-status={phase.status}
      aria-current={current ? 'step' : undefined}
      aria-label={`${phase.name}: ${PHASE_STATUS_LABEL[phase.status]}. ${detail}.`}
    >
      <span className="wf-graph-dot" aria-hidden="true" />
      <div className="wf-graph-card">
        <div className="wf-graph-title-row">
          <span className="wf-graph-name">{phase.name}</span>
          <span className="wf-graph-status">{PHASE_STATUS_LABEL[phase.status]}</span>
        </div>
        <span className="wf-graph-detail">{detail}</span>
      </div>
    </li>
  );
}

function StepRow({ step }: { step: WorkflowStep }) {
  return (
    <li className="wf-step" data-status={step.status}>
      <span className="wf-step-dot" aria-hidden="true" />
      <div className="wf-step-body">
        <div className="wf-step-title-row">
          <span className="wf-step-label">{step.label}</span>
          <span className="wf-step-status">{STEP_STATUS_LABEL[step.status]}</span>
        </div>
        <div className="wf-step-meta">
          {step.phase ? <span className="wf-chip">{step.phase}</span> : null}
          {step.agent ? <span className="wf-chip wf-chip-agent">{step.agent}</span> : null}
          {step.detail ? <span className="wf-step-detail">{step.detail}</span> : null}
        </div>
        {step.error ? <p className="wf-step-error">{step.error}</p> : null}
      </div>
    </li>
  );
}

function ArtifactCard({ artifact }: { artifact: WorkflowArtifact }) {
  const title = artifact.title ?? artifact.path ?? artifact.id;
  return (
    <li className="wf-artifact" data-kind={artifact.kind}>
      <div className="wf-artifact-head">
        <span className="wf-artifact-kind">
          {artifact.kind === 'markdown' ? 'Markdown' : 'File'}
        </span>
        <span className="wf-artifact-title">{title}</span>
      </div>
      {artifact.kind === 'markdown' ? (
        artifact.content ? (
          <pre className="wf-artifact-markdown">{artifact.content}</pre>
        ) : (
          <p className="wf-muted">Markdown artifact reported without content.</p>
        )
      ) : (
        <code className="wf-artifact-path">{artifact.path ?? 'No path reported.'}</code>
      )}
      {artifact.phase ? <span className="wf-chip">{artifact.phase}</span> : null}
    </li>
  );
}

function ApprovalCard({
  approval,
  runId,
  canDecide,
  onDecision,
}: {
  approval: WorkflowApproval;
  runId: string;
  canDecide: boolean;
  onDecision: (
    runId: string,
    approvalId: string,
    decision: WorkflowApprovalDecision,
  ) => void;
}) {
  const pending = approval.state === 'pending';
  return (
    <li className="wf-approval" data-state={approval.state}>
      <div className="wf-approval-head">
        <span className="wf-approval-label">Approval</span>
        <span className="wf-approval-state">{approval.state}</span>
      </div>
      <p className="wf-approval-summary">{approval.summary}</p>
      {approval.path_or_command !== null ? (
        <pre className="wf-approval-input" aria-label="Path or command">{approval.path_or_command}</pre>
      ) : null}
      {approval.diff_or_input !== null ? (
        <pre className="wf-approval-input" aria-label="Diff or input">{approval.diff_or_input}</pre>
      ) : null}
      {approval.error ? <p className="wf-step-error" role="alert">{approval.error}</p> : null}
      {approval.submitting ? <p className="wf-muted" role="status">Sending decision…</p> : null}
      {approval.tool_name ? (
        <span className="wf-chip">{approval.tool_name}</span>
      ) : null}
      {pending ? (
        <div className="wf-approval-actions">
          <button
            type="button"
            className="wf-btn wf-btn-approve"
            disabled={!canDecide || approval.submitting}
            aria-label={`Approve ${approval.summary}`}
            onClick={() => onDecision(runId, approval.approval_id, 'approve')}
          >
            Approve
          </button>
          <button
            type="button"
            className="wf-btn wf-btn-deny"
            disabled={!canDecide || approval.submitting}
            aria-label={`Deny ${approval.summary}`}
            onClick={() => onDecision(runId, approval.approval_id, 'deny')}
          >
            Deny
          </button>
        </div>
      ) : null}
    </li>
  );
}

export type WorkflowsPanelProps = {
  /** Defaults to the module singleton; tests may pass an isolated store. */
  store?: WorkflowsStore;
};

export function WorkflowsPanel({ store = workflowsStore }: WorkflowsPanelProps) {
  const state: WorkflowsState = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const headingId = useId();
  const run = state.runs.find((item) => item.id === state.active_run_id) ?? state.runs[0] ?? null;

  if (!run) {
    return (
      <div className="workflows-panel" data-testid="workflows-panel">
        <header className="wf-header">
          <h2 className="wf-title">Workflows</h2>
        </header>
        <div className="wf-empty" role="status" data-testid="workflows-empty">
          <h3 className="wf-empty-title">No workflow runs</h3>
          <p className="wf-empty-hint">
            Approve a workflow script in the chat and its phase graph, steps, artifacts, and
            cost will appear here.
          </p>
        </div>
      </div>
    );
  }

  const terminal = isTerminalRunStatus(run.status);
  const canStop = state.stop_wired && !terminal && !run.stop_requested;
  const canDecide = state.approval_wired && !terminal;
  const currentPhase =
    [...run.phases].reverse().find((phase) => phase.status === 'running')?.name ?? null;
  const budgetRatio =
    run.max_cost_usd !== null && run.max_cost_usd > 0
      ? Math.min(1, run.cost_usd / run.max_cost_usd)
      : 0;
  const budgetState = budgetRatio >= 1 ? 'over' : budgetRatio >= 0.8 ? 'near' : 'ok';

  return (
    <div className="workflows-panel" data-testid="workflows-panel">
      <header className="wf-header">
        <div className="wf-header-main">
          <h2 className="wf-title">Workflows</h2>
          <span className="wf-status-chip" data-status={run.status}>
            {runStatusLabel(run)}
          </span>
        </div>
        {run.resumable && !run.resume_confirmation ? (
          <button
            type="button"
            className="wf-btn wf-resume"
            disabled={!state.resume_wired || run.resume_requested}
            title={state.resume_wired
              ? 'Replay the journal, skipping completed work'
              : 'Resume is not available in this build'}
            onClick={() => store.resume(run.id)}
          >
            {run.resume_requested ? 'Resuming' : 'Resume'}
          </button>
        ) : null}
        <button
          type="button"
          className="wf-btn wf-stop"
          disabled={!canStop}
          title={state.stop_wired ? 'Stop this run and its in-flight commands' : 'Stop is not wired yet'}
          onClick={() => store.requestStop(run.id)}
        >
          <StopIcon size={12} color="currentColor" />
          <span>{run.stop_requested ? 'Stopping' : 'Stop'}</span>
        </button>
      </header>

      {run.partial_effects && !run.resume_confirmation ? (
        <div className="wf-resume-warning" role="alert" data-testid="wf-partial-effects">
          <p className="wf-resume-warning-title">Earlier tool effects may already exist</p>
          <p className="wf-resume-warning-text">
            The run&apos;s tool-effect ledger recorded a file write, edit, delete, or bash
            command that did not prove completion. Those effects are not undone by resume.
          </p>
          <div className="wf-resume-warning-actions">
            <button
              type="button"
              className="wf-btn wf-btn-approve"
              disabled={!state.resume_wired || run.resume_requested}
              onClick={() => store.acknowledgePartialEffects(run.id)}
            >
              {run.resume_requested ? 'Resuming' : 'Acknowledge and resume anyway'}
            </button>
          </div>
        </div>
      ) : null}
      {run.resume_error ? (
        <p className="wf-step-error" role="alert">{run.resume_error}</p>
      ) : null}
      {run.resume_confirmation ? (
        <div className="wf-resume-warning" role="alert" data-testid="wf-resume-warning">
          <p className="wf-resume-warning-title">
            {run.resume_confirmation.kind === 'partial_effects'
              ? 'Earlier tool effects may already exist'
              : run.resume_confirmation.count > 0
                ? `${run.resume_confirmation.count} unfinished ${
                    run.resume_confirmation.count === 1 ? 'step' : 'steps'
                  } may run again`
                : 'Unfinished steps may run again'}
          </p>
          <p className="wf-resume-warning-text">{run.resume_confirmation.summary}</p>
          <div className="wf-resume-warning-actions">
            <button
              type="button"
              className="wf-btn wf-btn-approve"
              disabled={!state.resume_wired || run.resume_requested}
              onClick={() => store.confirmResume(run.id)}
            >
              {run.resume_confirmation.kind === 'partial_effects'
                ? 'Acknowledge and resume anyway'
                : 'Resume anyway'}
            </button>
            <button
              type="button"
              className="wf-btn"
              disabled={run.resume_requested}
              onClick={() => store.dismissResumeConfirmation(run.id)}
            >
              Cancel
            </button>
          </div>
        </div>
      ) : null}
      <div className="wf-scroll">
        <nav className="wf-runs" aria-label="Workflow runs">
          <ul className="wf-run-list">
            {state.runs.map((item) => (
              <li key={item.id}>
                <button
                  type="button"
                  className="wf-run-button"
                  aria-current={item.id === run.id ? 'true' : undefined}
                  onClick={() => store.selectRun(item.id)}
                >
                  <span className="wf-run-name">{item.name}</span>
                  <span className="wf-run-meta">
                    <span className="wf-run-status" data-status={item.status}>
                      {runStatusLabel(item)}
                    </span>
                    <span className="wf-run-cost">{USD_FORMAT.format(item.cost_usd)}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </nav>

        <section className="wf-run-summary" aria-labelledby={`${headingId}-summary`}>
          <div className="wf-section-head">
            <h3 className="wf-section-title" id={`${headingId}-summary`}>
              {run.name}
            </h3>
            <span className="wf-section-meta">
              {run.started_at ? <span className="wf-mono">{run.started_at}</span> : null}
            </span>
          </div>
          <div className="wf-meter-row">
            <span className="wf-meter">
              <span className="wf-meter-value">{NUMBER_FORMAT.format(run.tokens)}</span> tokens
            </span>
            <span className="wf-meter">
              <span className="wf-meter-value">{USD_FORMAT.format(run.cost_usd)}</span>
              {run.max_cost_usd !== null
                ? ` of ${USD_FORMAT.format(run.max_cost_usd)} cap`
                : ' spent'}
            </span>
          </div>
          {run.max_cost_usd !== null ? (
            <div
              className="wf-budget"
              role="progressbar"
              aria-label="Workflow cost budget"
              aria-valuemin={0}
              aria-valuemax={run.max_cost_usd}
              aria-valuenow={run.cost_usd}
              aria-valuetext={`${USD_FORMAT.format(run.cost_usd)} of ${USD_FORMAT.format(
                run.max_cost_usd,
              )}`}
            >
              <div
                className="wf-budget-fill"
                data-state={budgetState}
                style={{ width: `${budgetRatio * 100}%` }}
              />
            </div>
          ) : null}
          {run.error ? <p className="wf-run-error">{run.error}</p> : null}
          {run.stop_error ? <p className="wf-run-error" role="alert">{run.stop_error}</p> : null}
        </section>

        <section className="wf-section" aria-labelledby={`${headingId}-phases`}>
          <h3 className="wf-section-title" id={`${headingId}-phases`}>
            Phases
          </h3>
          {run.phases.length > 0 ? (
            <ol className="wf-graph" role="list" aria-label="Workflow phase graph">
              {run.phases.map((phase) => (
                <PhaseNode
                  key={phase.name}
                  phase={phase}
                  steps={run.steps}
                  current={phase.name === currentPhase}
                />
              ))}
            </ol>
          ) : (
            <p className="wf-muted">No phases reported yet.</p>
          )}
        </section>

        <section className="wf-section" aria-labelledby={`${headingId}-steps`}>
          <h3 className="wf-section-title" id={`${headingId}-steps`}>
            Steps
          </h3>
          {run.steps.length > 0 ? (
            <ul className="wf-steps" aria-label="Workflow steps">
              {run.steps.map((step) => (
                <StepRow key={step.id} step={step} />
              ))}
            </ul>
          ) : (
            <p className="wf-muted">No steps reported yet.</p>
          )}
        </section>

        <section className="wf-section" aria-labelledby={`${headingId}-approvals`}>
          <h3 className="wf-section-title" id={`${headingId}-approvals`}>
            Approvals
          </h3>
          {run.approvals.length > 0 ? (
            <ul className="wf-approvals" aria-label="Run approvals">
              {run.approvals.map((approval) => (
                <ApprovalCard
                  key={approval.approval_id}
                  approval={approval}
                  runId={run.id}
                  canDecide={canDecide}
                  onDecision={store.decideApproval}
                />
              ))}
            </ul>
          ) : (
            <p className="wf-muted">No approvals requested.</p>
          )}
        </section>

        <section className="wf-section" aria-labelledby={`${headingId}-reports`}>
          <h3 className="wf-section-title" id={`${headingId}-reports`}>
            Reports
          </h3>
          {run.reports.length > 0 ? (
            <ul className="wf-reports" aria-label="Run reports">
              {run.reports.map((report) => (
                <li key={report.id} className="wf-report">
                  <p className="wf-report-text">{report.text}</p>
                  <div className="wf-report-meta">
                    {report.phase ? <span className="wf-chip">{report.phase}</span> : null}
                    {report.artifact_id ? (
                      <span className="wf-ref">Artifact {report.artifact_id}</span>
                    ) : null}
                  </div>
                </li>
              ))}
            </ul>
          ) : (
            <p className="wf-muted">No reports filed yet.</p>
          )}
        </section>

        <section className="wf-section" aria-labelledby={`${headingId}-artifacts`}>
          <h3 className="wf-section-title" id={`${headingId}-artifacts`}>
            Artifacts
          </h3>
          {run.artifacts.length > 0 ? (
            <ul className="wf-artifacts" aria-label="Run artifacts">
              {run.artifacts.map((artifact) => (
                <ArtifactCard key={artifact.id} artifact={artifact} />
              ))}
            </ul>
          ) : (
            <p className="wf-muted">No artifacts produced yet.</p>
          )}
        </section>

        <section className="wf-section" aria-labelledby={`${headingId}-log`}>
          <h3 className="wf-section-title" id={`${headingId}-log`}>
            Run log
          </h3>
          <div
            className="wf-log"
            role="log"
            aria-live="polite"
            aria-relevant="additions"
            aria-label="Run log"
            tabIndex={0}
          >
            {run.log.length > 0 ? (
              run.log.map((entry) => (
                <div key={entry.id} className="wf-log-line" data-level={entry.level}>
                  <span className="wf-log-message">{entry.message}</span>
                </div>
              ))
            ) : (
              <p className="wf-muted">No log entries yet.</p>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
