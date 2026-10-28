// Selora IDE — Phase 8d Workflows store.
//
// A small, protocol-ready store for the Workflows panel. It renders only what
// the engine reports: the seed is empty, an event for an unknown run is
// ignored rather than creating one, and Stop / Approve / Deny are forwarded
// through wiring callbacks instead of pretending the engine already acted.
//
// Wiring lives in `workflows-bridge.ts`: `toWorkflowStoreEvent` maps the
// protocol's `workflow_*` events onto `WorkflowEvent` and `installWorkflowHandlers`
// registers the two outbound callbacks:
//
//   workflowsStore.setStopHandler(() => stopWorkflow(ctx));
//   workflowsStore.setApprovalHandler((runId, approvalId, decision) =>
//     workflowDecision(requestId, approvalId, decision, ctx));
//
// The reducers are idempotent: replaying a journal event or a bootstrap
// snapshot does not duplicate runs, phases, steps, reports, artifacts,
// approvals, or log lines.

export type WorkflowRunStatus =
  | 'queued'
  | 'running'
  | 'awaiting_approval'
  | 'completed'
  | 'failed'
  | 'stopped';

export type WorkflowPhaseStatus = 'pending' | 'running' | 'completed' | 'failed' | 'skipped';

export type WorkflowStepStatus =
  | 'pending'
  | 'running'
  | 'completed'
  | 'failed'
  | 'skipped'
  | 'awaiting_approval';

export type WorkflowApprovalState = 'pending' | 'approved' | 'denied';

/** What the panel forwards to the shell for an approval card. */
export type WorkflowApprovalDecision = 'approve' | 'deny';

export type WorkflowLogLevel = 'info' | 'warn' | 'error';

export type WorkflowPhase = {
  name: string;
  status: WorkflowPhaseStatus;
};

/**
 * Receipt usage the engine attributed to one phase. Tokens and USD are
 * additive facts from the journal; a phase the engine reported as quiet is an
 * explicit 0, never an estimate.
 */
export type WorkflowPhaseUsage = {
  phase: string;
  tokens: number;
  cost_usd: number;
};

export type WorkflowStep = {
  id: string;
  /** Phase marker the step belongs to; null when the engine reported none. */
  phase: string | null;
  label: string;
  /** Named subagent identity from the facade's `agent(name?)`, when reported. */
  agent: string | null;
  status: WorkflowStepStatus;
  detail: string | null;
  error: string | null;
  started_at: string | null;
  finished_at: string | null;
};

export type WorkflowReport = {
  id: string;
  phase: string | null;
  text: string;
  /** Set when the report was filed against a markdown/file artifact. */
  artifact_id: string | null;
  at: string | null;
};

export type WorkflowArtifact = {
  id: string;
  kind: 'markdown' | 'file';
  title: string | null;
  /** Markdown body for `kind: 'markdown'`. */
  content: string | null;
  /** Workspace-relative path for `kind: 'file'`. */
  path: string | null;
  phase: string | null;
};

export type WorkflowApproval = {
  approval_id: string;
  /** Request that raised the card; the decision reply must echo it. */
  request_id: string | null;
  step_id: string | null;
  tool_name: string | null;
  summary: string;
  path_or_command: string | null;
  diff_or_input: string | null;
  state: WorkflowApprovalState;
  submitting: boolean;
  error: string | null;
};

export type WorkflowLogEntry = {
  id: string;
  at: string | null;
  level: WorkflowLogLevel;
  message: string;
};

export type WorkflowRun = {
  id: string;
  origin_context: { core_instance_id: string; workspace_id: string | null } | null;
  /** The request id the shell's `WorkflowStop`/`WorkflowDecision` address. */
  request_id: string | null;
  name: string;
  status: WorkflowRunStatus;
  phases: WorkflowPhase[];
  steps: WorkflowStep[];
  reports: WorkflowReport[];
  artifacts: WorkflowArtifact[];
  /**
   * Per-phase receipt usage. Empty when the engine reported no
   * phase-attributed spend; the panel then shows no per-phase numbers.
   */
  phase_usage: WorkflowPhaseUsage[];
  approvals: WorkflowApproval[];
  log: WorkflowLogEntry[];
  /** Tokens billed across every ask in the run. */
  tokens: number;
  /** USD billed across every ask in the run. */
  cost_usd: number;
  /** Workflow-level budget cap; null until the engine reports one. */
  max_cost_usd: number | null;
  error: string | null;
  started_at: string | null;
  finished_at: string | null;
  /** True between the Stop click and the terminal `finished` event. */
  stop_requested: boolean;
  stop_error: string | null;
  /** Durable, non-completed run found at bootstrap. */
  resumable: boolean;
  /**
   * Durable ledger evidence that a tool effect may already exist. Resume is
   * unavailable (`resumable: false`); the panel offers a separate explicit
   * acknowledgement action.
   */
  partial_effects: boolean;
  resume_requested: boolean;
  resume_error: string | null;
  /**
   * Set when the backend refused a resume because unfinished asks would be
   * dispatched again, or because partial tool effects may already exist. The
   * panel shows the warning and an explicit "Resume anyway" action; no retry
   * happens without it.
   */
  resume_confirmation: WorkflowResumeConfirmation | null;
};

export type WorkflowResumeConfirmation = {
  /** Which refusal this confirmation answers. */
  kind: 'incomplete_asks' | 'partial_effects';
  /** How many ask steps or effects the resume would replay. */
  count: number;
  /** The backend's refusal text, shown verbatim. */
  summary: string;
};

export type PersistedWorkflowRun = {
  run_id: string;
  name: string;
  status: 'pending' | 'running' | 'awaiting_approval' | 'completed' | 'failed' | 'stopped';
  resumable: boolean;
  /** Durable ledger evidence that a tool effect may already exist. */
  partial_effects?: boolean;
  detail?: string | null;
  /** Terminal-journal history (additive; absent on pre-history bootstraps). */
  phases?: string[];
  steps?: Array<{
    id: string;
    phase?: string;
    label?: string;
    status: string;
    agent?: string;
    detail?: string;
  }>;
  reports?: Array<{ id: string; item: string; artifact_id?: string; phase?: string | null }>;
  artifacts?: Array<{
    id: string;
    title: string;
    kind: string;
    body: string;
    phase?: string | null;
  }>;
  /** Additive per-phase usage; absent on pre-phase-usage bootstraps. */
  phase_usage?: Array<{ phase: string; tokens?: number; cost_usd?: number }>;
  tokens?: number;
  cost_usd?: number;
};

export type WorkflowsState = {
  /** Newest run first. Empty until the engine reports a run. */
  runs: WorkflowRun[];
  active_run_id: string | null;
  /** True once the shell registered the Stop transport. */
  stop_wired: boolean;
  /** True once the shell registered the approval transport. */
  approval_wired: boolean;
  /** True once the shell registered the durable-resume transport. */
  resume_wired: boolean;
};

// --- Event shapes -----------------------------------------------------------
//
// One discriminated union, snake_case payload fields, no envelope concerns.
// The wiring agent maps `AppMessage::Workflow*` onto these one-for-one.

export type WorkflowStartedEvent = {
  kind: 'started';
  run_id: string;
  request_id?: string | null;
  origin_context?: WorkflowRun['origin_context'];
  name: string;
  /** Phase names from the script's plan; the graph grows as markers arrive. */
  phases?: string[];
  max_cost_usd?: number | null;
  at?: string | null;
};

export type WorkflowPhaseEvent = {
  kind: 'phase';
  run_id: string;
  phase: string;
  status: WorkflowPhaseStatus;
  at?: string | null;
};

export type WorkflowStepEvent = {
  kind: 'step';
  run_id: string;
  /** Stable step identity; resume matches steps by this id. */
  step_id: string;
  phase?: string | null;
  label?: string | null;
  agent?: string | null;
  status: WorkflowStepStatus;
  detail?: string | null;
  error?: string | null;
  at?: string | null;
};

export type WorkflowReportEvent = {
  kind: 'report';
  run_id: string;
  report_id: string;
  text: string;
  phase?: string | null;
  artifact_id?: string | null;
  at?: string | null;
};

export type WorkflowArtifactEvent = {
  kind: 'artifact';
  run_id: string;
  artifact_id: string;
  artifact_kind: 'markdown' | 'file';
  title?: string | null;
  content?: string | null;
  path?: string | null;
  phase?: string | null;
  at?: string | null;
};

export type WorkflowApprovalEvent = {
  kind: 'approval';
  run_id: string;
  approval_id: string;
  /** Carried through from `workflow_approval_needed` so Deny/Approve can echo it. */
  request_id?: string | null;
  state: WorkflowApprovalState;
  summary: string;
  path_or_command?: string | null;
  diff_or_input?: string | null;
  step_id?: string | null;
  tool_name?: string | null;
  at?: string | null;
};

export type WorkflowFinishedEvent = {
  kind: 'finished';
  run_id: string;
  /** Defaults to 'completed', or 'failed' when an error is present. */
  status?: 'completed' | 'failed' | 'stopped';
  tokens?: number;
  cost_usd?: number;
  error?: string | null;
  at?: string | null;
};

export type WorkflowEvent =
  | WorkflowStartedEvent
  | WorkflowPhaseEvent
  | WorkflowStepEvent
  | WorkflowReportEvent
  | WorkflowArtifactEvent
  | WorkflowApprovalEvent
  | WorkflowFinishedEvent;

export type WorkflowStopHandler = (runId: string) => void | Promise<unknown>;
export type WorkflowResumeHandler = (
  runId: string,
  allowIncompleteAsks: boolean,
  acknowledgePartialEffects?: boolean,
) => void | Promise<unknown>;
export type WorkflowApprovalHandler = (
  runId: string,
  approvalId: string,
  decision: WorkflowApprovalDecision,
) => void | Promise<unknown>;

export type WorkflowsStore = {
  subscribe(listener: () => void): () => void;
  getSnapshot(): WorkflowsState;
  setStopHandler(handler: WorkflowStopHandler | null): void;
  setApprovalHandler(handler: WorkflowApprovalHandler | null): void;
  setResumeHandler(handler: WorkflowResumeHandler | null): void;
  /** Add durable runs discovered at bootstrap (including completed history). */
  hydratePersisted(runs: PersistedWorkflowRun[]): void;
  /** Resume a durable run; the live `started` event takes over on success. */
  resume(runId: string): void;
  /** Confirm a refused resume: send it again with `allow_incomplete_asks`. */
  confirmResume(runId: string): void;
  /**
   * Acknowledge durable partial-effect evidence: a SEPARATE action that sends
   * `acknowledge_partial_effects`. Never wired to ordinary Resume.
   */
  acknowledgePartialEffects(runId: string): void;
  /** Dismiss the incomplete-asks warning without resuming. */
  dismissResumeConfirmation(runId: string): void;
  applyEvent(event: WorkflowEvent): void;
  selectRun(runId: string): void;
  /** Request a stop; the run only becomes `stopped` on the terminal event. */
  stop(runId: string): void;
  /** Alias preferred by the panel; same semantics as `stop`. */
  requestStop(runId: string): void;
  decideApproval(
    runId: string,
    approvalId: string,
    decision: WorkflowApprovalDecision,
  ): void;
  reset(): void;
};

/** ARCHITECTURE §18.2 caps reports at 256; keep the newest if it ever overshoots. */
export const WORKFLOW_REPORT_MAX = 256;

/** Bounded in-memory log. The journal on disk remains the durable record. */
export const WORKFLOW_LOG_MAX = 500;

export function isTerminalRunStatus(status: WorkflowRunStatus): boolean {
  return status === 'completed' || status === 'failed' || status === 'stopped';
}

const EMPTY_STATE: WorkflowsState = {
  runs: [],
  active_run_id: null,
  stop_wired: false,
  approval_wired: false,
  resume_wired: false,
};

type Listener = () => void;
const listeners = new Set<Listener>();

let state: WorkflowsState = EMPTY_STATE;
let stopHandler: WorkflowStopHandler | null = null;
let approvalHandler: WorkflowApprovalHandler | null = null;
let resumeHandler: WorkflowResumeHandler | null = null;
let nextLogId = 0;
let generation = 0;

function transportError(error: unknown): string {
  if (error && typeof error === 'object' && 'message' in error) return String(error.message);
  return typeof error === 'string' ? error : 'Request failed. Try again.';
}

function notify(): void {
  for (const listener of listeners) listener();
}

function appendLog(
  run: WorkflowRun,
  message: string,
  level: WorkflowLogLevel,
  at: string | null,
): WorkflowRun {
  nextLogId += 1;
  const entry: WorkflowLogEntry = {
    id: `wf_log_${nextLogId}`,
    at,
    level,
    message,
  };
  return { ...run, log: [...run.log, entry].slice(-WORKFLOW_LOG_MAX) };
}

function upsertPhase(
  phases: WorkflowPhase[],
  name: string,
  status: WorkflowPhaseStatus,
): { phases: WorkflowPhase[]; changed: boolean } {
  const index = phases.findIndex((phase) => phase.name === name);
  if (index === -1) {
    return { phases: [...phases, { name, status }], changed: true };
  }
  if (phases[index]!.status === status) return { phases, changed: false };
  return {
    phases: phases.map((phase, i) => (i === index ? { ...phase, status } : phase)),
    changed: true,
  };
}

function planPhases(names: string[] | undefined): WorkflowPhase[] {
  const phases: WorkflowPhase[] = [];
  const seen = new Set<string>();
  for (const rawName of names ?? []) {
    const name = rawName.trim();
    if (!name || seen.has(name)) continue;
    seen.add(name);
    phases.push({ name, status: 'pending' });
  }
  return phases;
}

/**
 * Sanitise persisted per-phase usage. Only a non-empty phase name and
 * non-negative finite numbers are facts; anything else reads as 0 rather than
 * inventing spend or a phase.
 */
function phaseUsageRows(rows: PersistedWorkflowRun['phase_usage']): WorkflowPhaseUsage[] {
  const usage: WorkflowPhaseUsage[] = [];
  const seen = new Set<string>();
  for (const row of rows ?? []) {
    if (!row || typeof row.phase !== 'string') continue;
    const phase = row.phase.trim();
    if (!phase || seen.has(phase)) continue;
    seen.add(phase);
    const tokens =
      typeof row.tokens === 'number' && Number.isFinite(row.tokens) && row.tokens > 0
        ? Math.trunc(row.tokens)
        : 0;
    const cost_usd =
      typeof row.cost_usd === 'number' && Number.isFinite(row.cost_usd) && row.cost_usd > 0
        ? row.cost_usd
        : 0;
    usage.push({ phase, tokens, cost_usd });
  }
  return usage;
}

/** Protocol phase status → phase graph status (mirrors workflows-bridge). */
function toPhaseStatus(status: string): WorkflowPhaseStatus {
  switch (status) {
    case 'completed':
    case 'failed':
      return status;
    case 'stopped':
      return 'skipped';
    case 'running':
    case 'awaiting_approval':
      return 'running';
    default:
      return 'pending';
  }
}

/** Protocol step status → step row status (mirrors workflows-bridge). */
function toStepStatus(status: string): WorkflowStepStatus {
  switch (status) {
    case 'completed':
    case 'failed':
    case 'awaiting_approval':
      return status;
    case 'stopped':
      return 'skipped';
    case 'running':
      return 'running';
    default:
      return 'pending';
  }
}

/**
 * Recognise the backend's `workflow_incomplete_asks` refusal. The protocol
 * error has no structured detail slot, so the count is read from the message
 * the desktop formats; the message itself is the warning text.
 */
function incompleteAsksConfirmation(error: unknown): WorkflowResumeConfirmation | null {
  if (!error || typeof error !== 'object') return null;
  const candidate = error as { code?: unknown; message?: unknown };
  if (candidate.code !== 'workflow_incomplete_asks') return null;
  const summary =
    typeof candidate.message === 'string' && candidate.message.trim() !== ''
      ? candidate.message
      : 'Resume would dispatch unfinished ask steps again. Their tools may already have applied effects.';
  const match = /(\d+)\s+unfinished ask/i.exec(summary);
  return { kind: 'incomplete_asks', count: match ? Number(match[1]) : 0, summary };
}

/**
 * Recognise the backend's `workflow_partial_effects` refusal. This is the
 * separate acknowledgement: the ledger proved a tool effect may already have
 * landed, so ordinary Resume is refused.
 */
function partialEffectsConfirmation(error: unknown): WorkflowResumeConfirmation | null {
  if (!error || typeof error !== 'object') return null;
  const candidate = error as { code?: unknown; message?: unknown };
  if (candidate.code !== 'workflow_partial_effects') return null;
  const summary =
    typeof candidate.message === 'string' && candidate.message.trim() !== ''
      ? candidate.message
      : 'Earlier tool effects may already exist. Resume does not undo them.';
  return { kind: 'partial_effects', count: 0, summary };
}

function upsertStep(
  steps: WorkflowStep[],
  event: WorkflowStepEvent,
): { steps: WorkflowStep[]; changed: boolean } {
  const index = steps.findIndex((step) => step.id === event.step_id);
  const previous = index >= 0 ? steps[index]! : null;
  const terminal = event.status === 'completed' || event.status === 'failed' || event.status === 'skipped';
  const next: WorkflowStep = {
    id: event.step_id,
    phase: event.phase ?? previous?.phase ?? null,
    label: event.label ?? previous?.label ?? event.step_id,
    agent: event.agent ?? previous?.agent ?? null,
    status: event.status,
    detail: event.detail ?? previous?.detail ?? null,
    error: event.error ?? previous?.error ?? null,
    started_at: previous?.started_at ?? event.at ?? null,
    finished_at: terminal ? event.at ?? previous?.finished_at ?? null : previous?.finished_at ?? null,
  };
  if (!previous) return { steps: [...steps, next], changed: true };
  const changed =
    previous.phase !== next.phase ||
    previous.label !== next.label ||
    previous.agent !== next.agent ||
    previous.status !== next.status ||
    previous.detail !== next.detail ||
    previous.error !== next.error;
  if (!changed) return { steps, changed: false };
  return { steps: steps.map((step, i) => (i === index ? next : step)), changed: true };
}

function upsertApproval(
  approvals: WorkflowApproval[],
  event: WorkflowApprovalEvent,
): { approvals: WorkflowApproval[]; changed: boolean; hasPending: boolean } {
  const index = approvals.findIndex((item) => item.approval_id === event.approval_id);
  if (index === -1) {
    const created: WorkflowApproval = {
      approval_id: event.approval_id,
      request_id: event.request_id ?? null,
      step_id: event.step_id ?? null,
      tool_name: event.tool_name ?? null,
      summary: event.summary,
      path_or_command: event.path_or_command ?? null,
      diff_or_input: event.diff_or_input ?? null,
      state: event.state,
      submitting: false,
      error: null,
    };
    const next = [...approvals, created];
    return {
      approvals: next,
      changed: true,
      hasPending: next.some((item) => item.state === 'pending'),
    };
  }
  // Replayed pending events must not reopen a single-use decision.
  if (approvals[index]!.state !== 'pending' && event.state === 'pending') {
    return {
      approvals,
      changed: false,
      hasPending: approvals.some((item) => item.state === 'pending'),
    };
  }
  const next = approvals.map((item, i) =>
    i === index
      ? {
          ...item,
          request_id: event.request_id ?? item.request_id,
          step_id: event.step_id ?? item.step_id,
          tool_name: event.tool_name ?? item.tool_name,
          summary: event.summary,
          path_or_command: event.path_or_command ?? item.path_or_command,
          diff_or_input: event.diff_or_input ?? item.diff_or_input,
          state: event.state,
          submitting: event.state === 'pending' ? item.submitting : false,
          error: event.state === 'pending' ? item.error : null,
        }
      : item,
  );
  return {
    approvals: next,
    changed:
      approvals[index]!.state !== event.state ||
      approvals[index]!.summary !== event.summary ||
      (event.step_id != null && approvals[index]!.step_id !== event.step_id) ||
      (event.tool_name != null && approvals[index]!.tool_name !== event.tool_name) ||
      (event.path_or_command != null && approvals[index]!.path_or_command !== event.path_or_command) ||
      (event.diff_or_input != null && approvals[index]!.diff_or_input !== event.diff_or_input) ||
      (event.request_id != null && approvals[index]!.request_id !== event.request_id),
    hasPending: next.some((item) => item.state === 'pending'),
  };
}

function replaceRun(
  runId: string,
  update: (run: WorkflowRun) => WorkflowRun,
): void {
  state = {
    ...state,
    runs: state.runs.map((run) => (run.id === runId ? update(run) : run)),
  };
}

/**
 * Dispatch a resume with the user's confirmation flag.
 *
 * A refusal carrying `workflow_incomplete_asks` becomes a per-run warning
 * (count + backend message) instead of a plain error, and nothing retries until
 * the panel calls `confirmResume`. Any other failure keeps the existing
 * resumable/error behavior.
 */
function sendResume(
  runId: string,
  allowIncompleteAsks: boolean,
  acknowledgePartialEffects = false,
): void {
  if (!resumeHandler) return;
  replaceRun(runId, (current) => ({
    ...current,
    resume_requested: true,
    resume_error: null,
    resume_confirmation: null,
  }));
  notify();
  const currentGeneration = generation;
  const failed = (error: unknown) => {
    if (generation !== currentGeneration) return;
    const confirmation = acknowledgePartialEffects
      ? null
      : partialEffectsConfirmation(error) ?? incompleteAsksConfirmation(error);
    replaceRun(runId, (current) => {
      const partial = confirmation?.kind === 'partial_effects' ? true : current.partial_effects;
      return {
        ...current,
        resume_requested: false,
        resumable: partial ? false : true,
        partial_effects: partial,
        resume_error: confirmation ? null : transportError(error),
        resume_confirmation: confirmation ?? current.resume_confirmation,
      };
    });
    notify();
  };
  try {
    void Promise.resolve(
      resumeHandler(runId, allowIncompleteAsks, acknowledgePartialEffects),
    ).then(() => {
      if (generation !== currentGeneration) return;
      // The command was accepted: the run is live now, so clear the resume
      // offer. The `started` event still owns request/origin metadata.
      replaceRun(runId, (current) =>
        current.resumable || current.partial_effects
          ? { ...current, resumable: false, partial_effects: false }
          : current,
      );
      notify();
    }, failed);
  } catch (error) {
    failed(error);
  }
}

export const workflowsStore: WorkflowsStore = {
  subscribe(listener: Listener): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },

  getSnapshot(): WorkflowsState {
    return state;
  },

  setStopHandler(handler: WorkflowStopHandler | null): void {
    stopHandler = handler;
    const stop_wired = handler !== null;
    if (state.stop_wired !== stop_wired) {
      state = { ...state, stop_wired };
      notify();
    }
  },

  setApprovalHandler(handler: WorkflowApprovalHandler | null): void {
    approvalHandler = handler;
    const approval_wired = handler !== null;
    if (state.approval_wired !== approval_wired) {
      state = { ...state, approval_wired };
      notify();
    }
  },

  setResumeHandler(handler: WorkflowResumeHandler | null): void {
    resumeHandler = handler;
    const resume_wired = handler !== null;
    if (state.resume_wired !== resume_wired) {
      state = { ...state, resume_wired };
      notify();
    }
  },

  hydratePersisted(runs: PersistedWorkflowRun[]): void {
    const additions = runs
      .filter((run) => run.run_id && !state.runs.some((existing) => existing.id === run.run_id))
      .map((run): WorkflowRun => {
        const completed = run.status === 'completed';
        const status: WorkflowRunStatus =
          run.status === 'pending' ? 'queued' : run.status;
        const phases = planPhases(run.phases);
        const recoveredPhases = completed
          ? phases.map((phase) => ({ ...phase, status: 'completed' as const }))
          : phases;
        const steps: WorkflowStep[] = (run.steps ?? []).map((step) => ({
          id: step.id,
          phase: step.phase && step.phase.trim() !== '' ? step.phase : null,
          label: step.label && step.label !== '' ? step.label : step.id,
          agent: step.agent ?? null,
          status: toStepStatus(step.status),
          detail: step.detail ?? null,
          error: null,
          started_at: null,
          finished_at: null,
        }));
        const reports: WorkflowReport[] = (run.reports ?? []).map((report) => ({
          id: report.id,
          phase: report.phase && report.phase.trim() !== '' ? report.phase : null,
          text: report.item,
          artifact_id: report.artifact_id ?? null,
          at: null,
        }));
        const artifacts: WorkflowArtifact[] = (run.artifacts ?? []).map((artifact) => {
          const markdown = artifact.kind === 'markdown';
          return {
            id: artifact.id,
            kind: markdown ? 'markdown' : 'file',
            title: artifact.title || null,
            content: markdown ? artifact.body : null,
            path: markdown ? null : artifact.body,
            phase: artifact.phase && artifact.phase.trim() !== '' ? artifact.phase : null,
          };
        });
        const created = appendLog(
          {
            id: run.run_id,
            origin_context: null,
            request_id: null,
            name: run.name,
            status,
            phases: recoveredPhases,
            steps,
            reports,
            artifacts,
            phase_usage: phaseUsageRows(run.phase_usage),
            approvals: [],
            log: [],
            tokens: run.tokens ?? 0,
            cost_usd: run.cost_usd ?? 0,
            max_cost_usd: null,
            error: run.detail ?? null,
            started_at: null,
            finished_at: null,
            stop_requested: false,
            stop_error: null,
            // A completed journal is history: never offer it for replay.
            resumable: completed ? false : run.resumable,
            // Durable ledger evidence is additive; old bootstraps omit it.
            partial_effects: completed ? false : run.partial_effects === true,
            resume_requested: false,
            resume_error: null,
            resume_confirmation: null,
          },
          `Recovered from journal: ${run.name}`,
          'info',
          null,
        );
        return created;
      });
    if (additions.length === 0) return;
    state = {
      ...state,
      runs: [...state.runs, ...additions],
      active_run_id: state.active_run_id ?? additions[0]!.id,
    };
    notify();
  },

  resume(runId: string): void {
    const run = state.runs.find((item) => item.id === runId);
    if (!run || !run.resumable || run.resume_requested || !resumeHandler) return;
    // A refused resume is never retried silently: the panel must send the
    // explicit confirmation through `confirmResume`.
    if (run.resume_confirmation) return;
    sendResume(runId, false);
  },

  confirmResume(runId: string): void {
    const run = state.runs.find((item) => item.id === runId);
    if (!run || run.resume_requested || !resumeHandler) return;
    if (!run.resume_confirmation) return;
    // A partial-effect confirmation is shown while `resumable` is false (the
    // bootstrap marks the run non-resumable). Only that explicit confirmation
    // may proceed without it.
    if (!run.resumable && run.resume_confirmation.kind !== 'partial_effects') return;
    // The partial-effect acknowledgement also names the effects of an
    // unfinished ask, so it subsumes `allow_incomplete_asks`.
    sendResume(runId, true, run.partial_effects);
  },

  acknowledgePartialEffects(runId: string): void {
    const run = state.runs.find((item) => item.id === runId);
    if (!run || !run.partial_effects || run.resume_requested || !resumeHandler) return;
    sendResume(runId, true, true);
  },

  dismissResumeConfirmation(runId: string): void {
    const run = state.runs.find((item) => item.id === runId);
    if (!run?.resume_confirmation) return;
    replaceRun(runId, (current) => ({
      ...current,
      resume_confirmation: null,
      resume_error: null,
    }));
    notify();
  },

  applyEvent(event: WorkflowEvent): void {
    const at = 'at' in event ? event.at ?? null : null;

    switch (event.kind) {
      case 'started': {
        const existing = state.runs.find((run) => run.id === event.run_id);
        if (existing) {
          // A replayed `started` (bootstrap/resume) refreshes identity and the
          // planned phase list without resetting live progress.
          let phases = existing.phases;
          for (const phase of planPhases(event.phases)) {
            phases = upsertPhase(phases, phase.name, phase.status).phases;
          }
          state = {
            ...state,
            runs: state.runs.map((run) =>
              run.id === event.run_id
                ? {
                    ...run,
                    request_id: event.request_id ?? run.request_id,
                    origin_context: run.origin_context ?? event.origin_context ?? null,
                    name: event.name,
                    phases,
                    max_cost_usd: event.max_cost_usd ?? run.max_cost_usd,
                    started_at: run.started_at ?? at,
                    resumable: false,
                    partial_effects: false,
                    resume_requested: false,
                    resume_error: null,
                    resume_confirmation: null,
                  }
                : run,
            ),
            active_run_id: event.run_id,
          };
          break;
        }

        const created = appendLog(
          {
            id: event.run_id,
            origin_context: event.origin_context ? { ...event.origin_context } : null,
            request_id: event.request_id ?? null,
            name: event.name,
            status: 'running',
            phases: planPhases(event.phases),
            steps: [],
            reports: [],
            artifacts: [],
            phase_usage: [],
            approvals: [],
            log: [],
            tokens: 0,
            cost_usd: 0,
            max_cost_usd: event.max_cost_usd ?? null,
            error: null,
            started_at: at,
            finished_at: null,
            stop_requested: false,
            stop_error: null,
            resumable: false,
            partial_effects: false,
            resume_requested: false,
            resume_error: null,
            resume_confirmation: null,
          },
          `Run started: ${event.name}`,
          'info',
          at,
        );
        state = {
          ...state,
          runs: [created, ...state.runs],
          active_run_id: event.run_id,
        };
        break;
      }

      case 'phase': {
        const run = state.runs.find((item) => item.id === event.run_id);
        if (!run) return;
        const result = upsertPhase(run.phases, event.phase, event.status);
        if (!result.changed) return;
        const next = appendLog(
          {
            ...run,
            phases: result.phases,
            status: run.status === 'queued' ? 'running' : run.status,
          },
          `Phase ${event.phase}: ${event.status}`,
          event.status === 'failed' ? 'error' : 'info',
          at,
        );
        replaceRun(run.id, () => next);
        break;
      }

      case 'step': {
        const run = state.runs.find((item) => item.id === event.run_id);
        if (!run) return;
        let phases = run.phases;
        if (event.phase) {
          // A step can arrive before its phase marker; make the node exist so
          // the graph never hides live work. The marker still owns its status.
          const existingPhase = phases.find((phase) => phase.name === event.phase);
          if (!existingPhase) {
            phases = [...phases, { name: event.phase, status: 'pending' }];
          }
        }
        const result = upsertStep(run.steps, event);
        if (!result.changed && phases === run.phases) return;
        const label = result.steps.find((step) => step.id === event.step_id)?.label ?? event.step_id;
        const next = appendLog(
          {
            ...run,
            phases,
            steps: result.steps,
            status: run.status === 'queued' ? 'running' : run.status,
          },
          `${label}: ${event.status}`,
          event.status === 'failed' ? 'error' : 'info',
          at,
        );
        replaceRun(run.id, () => next);
        break;
      }

      case 'report': {
        const run = state.runs.find((item) => item.id === event.run_id);
        if (!run) return;
        if (run.reports.some((report) => report.id === event.report_id)) return;
        const report: WorkflowReport = {
          id: event.report_id,
          phase: event.phase ?? null,
          text: event.text,
          artifact_id: event.artifact_id ?? null,
          at,
        };
        const next = appendLog(
          {
            ...run,
            reports: [...run.reports, report].slice(-WORKFLOW_REPORT_MAX),
          },
          `Report: ${event.text}`,
          'info',
          at,
        );
        replaceRun(run.id, () => next);
        break;
      }

      case 'artifact': {
        const run = state.runs.find((item) => item.id === event.run_id);
        if (!run) return;
        const artifact: WorkflowArtifact = {
          id: event.artifact_id,
          kind: event.artifact_kind,
          title: event.title ?? null,
          content: event.content ?? null,
          path: event.path ?? null,
          phase: event.phase ?? null,
        };
        const index = run.artifacts.findIndex((item) => item.id === event.artifact_id);
        const artifacts =
          index === -1
            ? [...run.artifacts, artifact]
            : run.artifacts.map((item, i) => (i === index ? artifact : item));
        const changed =
          index === -1 ||
          run.artifacts[index]!.content !== artifact.content ||
          run.artifacts[index]!.path !== artifact.path ||
          run.artifacts[index]!.title !== artifact.title;
        if (!changed) return;
        const label = event.title ?? event.path ?? event.artifact_id;
        const next = appendLog(
          { ...run, artifacts },
          `Artifact (${event.artifact_kind}): ${label}`,
          'info',
          at,
        );
        replaceRun(run.id, () => next);
        break;
      }

      case 'approval': {
        const run = state.runs.find((item) => item.id === event.run_id);
        if (!run || isTerminalRunStatus(run.status)) return;
        const result = upsertApproval(run.approvals, event);
        if (!result.changed) return;
        let status = run.status;
        if (!isTerminalRunStatus(status)) {
          if (event.state === 'pending') status = 'awaiting_approval';
          else if (!result.hasPending && status === 'awaiting_approval') status = 'running';
        }
        const message =
          event.state === 'pending'
            ? `Approval needed: ${event.summary}`
            : `Approval ${event.state}: ${event.summary}`;
        const next = appendLog(
          { ...run, approvals: result.approvals, status },
          message,
          event.state === 'denied' ? 'warn' : 'info',
          at,
        );
        replaceRun(run.id, () => next);
        break;
      }

      case 'finished': {
        const run = state.runs.find((item) => item.id === event.run_id);
        if (!run) return;
        const status =
          event.status ?? (event.error ? 'failed' : 'completed');
        const next = appendLog(
          {
            ...run,
            status,
            tokens: event.tokens ?? run.tokens,
            cost_usd: event.cost_usd ?? run.cost_usd,
            error: event.error ?? run.error,
            finished_at: at ?? run.finished_at,
            stop_requested: false,
            resume_confirmation: null,
            approvals: run.approvals.map((approval) => ({ ...approval, submitting: false })),
          },
          `Run ${status}${event.error ? `: ${event.error}` : ''}`,
          status === 'completed' ? 'info' : status === 'stopped' ? 'warn' : 'error',
          at,
        );
        replaceRun(run.id, () => next);
        break;
      }
    }

    notify();
  },

  selectRun(runId: string): void {
    if (state.active_run_id === runId) return;
    if (!state.runs.some((run) => run.id === runId)) return;
    state = { ...state, active_run_id: runId };
    notify();
  },

  /**
   * Forward a Stop to the shell and mark the request locally. The run only
   * becomes `stopped` when the terminal `finished` event confirms it.
   */
  stop(runId: string): void {
    workflowsStore.requestStop(runId);
  },

  requestStop(runId: string): void {
    const run = state.runs.find((item) => item.id === runId);
    if (!run || isTerminalRunStatus(run.status) || run.stop_requested) return;
    if (!stopHandler) return;
    const next = appendLog(
      { ...run, stop_requested: true, stop_error: null },
      'Stop requested.',
      'warn',
      null,
    );
    replaceRun(runId, () => next);
    notify();
    const currentGeneration = generation;
    const failed = (error: unknown) => {
      if (generation !== currentGeneration) return;
      replaceRun(runId, (current) => isTerminalRunStatus(current.status) ? current : {
        ...current,
        stop_requested: false,
        stop_error: transportError(error),
      });
      notify();
    };
    try {
      void Promise.resolve(stopHandler(runId)).catch(failed);
    } catch (error) {
      failed(error);
    }
  },

  /**
   * Forward an approval decision. The card keeps its pending state until the
   * transport accepts it; rejection leaves it pending and allows a retry.
   */
  decideApproval(
    runId: string,
    approvalId: string,
    decision: WorkflowApprovalDecision,
  ): void {
    const run = state.runs.find((item) => item.id === runId);
    if (!run || isTerminalRunStatus(run.status)) return;
    if (!approvalHandler) return;
    const approval = run.approvals.find((item) => item.approval_id === approvalId);
    if (!approval || approval.state !== 'pending' || approval.submitting) return;
    if (decision !== 'approve' && decision !== 'deny') return;
    replaceRun(runId, (current) => ({
      ...current,
      approvals: current.approvals.map((item) =>
        item.approval_id === approvalId ? { ...item, submitting: true, error: null } : item,
      ),
    }));
    notify();
    const currentGeneration = generation;
    const settled = (error?: unknown) => {
      if (generation !== currentGeneration) return;
      replaceRun(runId, (current) => {
        if (isTerminalRunStatus(current.status)) return current;
        const approvals = current.approvals.map((item): WorkflowApproval => {
          if (item.approval_id !== approvalId || item.state !== 'pending') return item;
          return {
            ...item,
            submitting: false,
            error: error === undefined ? null : transportError(error),
            state: error === undefined ? (decision === 'approve' ? 'approved' : 'denied') : item.state,
          };
        });
        const hasPending = approvals.some((item) => item.state === 'pending');
        return {
          ...current,
          approvals,
          status: current.status === 'awaiting_approval' && !hasPending ? 'running' : current.status,
        };
      });
      notify();
    };
    try {
      void Promise.resolve(approvalHandler(runId, approvalId, decision)).then(
        () => settled(),
        (error: unknown) => settled(error ?? new Error('Request failed. Try again.')),
      );
    } catch (error) {
      settled(error ?? new Error('Request failed. Try again.'));
    }
  },

  reset(): void {
    generation += 1;
    nextLogId = 0;
    state = {
      ...EMPTY_STATE,
      stop_wired: stopHandler !== null,
      approval_wired: approvalHandler !== null,
      resume_wired: resumeHandler !== null,
    };
    notify();
  },
};

export function subscribeWorkflows(listener: Listener): () => void {
  return workflowsStore.subscribe(listener);
}

export function getWorkflowsSnapshot(): WorkflowsState {
  return workflowsStore.getSnapshot();
}
