// Selora IDE — Phase Z2 layout store.
//
// The single frontend source of truth for region sizes and open state
// (ARCHITECTURE §2.4/§3.1, DESIGN §1/§2). Hydration comes from
// `bootstrap.layout`, authoritative updates come only from validated
// `workspace_layout_changed` events, every resize proposal is clamped with
// `bootstrap.layout_constraints` (never literals), and the command cadence is
// the contract's: exactly one `ResizeRegion` per completed drag/step, awaited,
// then `SaveLayout`; `ToggleSurface` immediately, awaited, then `SaveLayout`.
import {
  resizeRegion,
  saveLayout,
  toggleSurface,
  type BootstrapState,
  type EnvelopeContext,
  type Layout,
  type LayoutConstraints,
  type RegionId,
  type SurfaceId,
} from '../../bridge/bridge';

export type PendingResize = { region: RegionId; sizePx: number };

export type LayoutState = {
  hydrated: boolean;
  layout: Layout | null;
  constraints: LayoutConstraints | null;
  /** The bootstrap-delivered LayoutDto — the double-click reset target (DESIGN §4). */
  defaults: Layout | null;
  /** A committed size awaiting its confirming event; drives CSS vars until then. */
  pending: PendingResize | null;
  /** Region currently being pointer-dragged (drives the shell's no-select class). */
  dragging: RegionId | null;
};

/** Non-reactive geometry used by the clamping math; measured from the DOM. */
type WorkbenchBox = { width: number; height: number; activityBarWidth: number };

export type ClampContext = {
  constraints: LayoutConstraints;
  layout: Layout;
  workbenchWidth: number;
  workbenchHeight: number;
  activityBarWidth: number;
};

const REGION_SIZE_FIELD: Record<RegionId, keyof Layout> = {
  primary_sidebar: 'primary_sidebar_width_px',
  agent_dock: 'agent_dock_width_px',
  bottom_dock: 'bottom_dock_height_px',
};

const REGION_OPEN_FIELD: Record<RegionId, keyof Layout> = {
  primary_sidebar: 'primary_sidebar_open',
  agent_dock: 'agent_dock_open',
  bottom_dock: 'bottom_dock_open',
};

export function regionSize(layout: Layout, region: RegionId): number {
  return layout[REGION_SIZE_FIELD[region]] as number;
}

export function regionOpen(layout: Layout, region: RegionId): boolean {
  return layout[REGION_OPEN_FIELD[region]] as boolean;
}

export function regionMin(constraints: LayoutConstraints, region: RegionId): number {
  switch (region) {
    case 'primary_sidebar':
      return constraints.primary_sidebar_min_px;
    case 'agent_dock':
      return constraints.agent_dock_min_px;
    case 'bottom_dock':
      return constraints.bottom_dock_min_px;
  }
}

/** The static Rust ceiling for ARIA bounds (DESIGN §4). */
export function regionMax(constraints: LayoutConstraints, region: RegionId): number {
  switch (region) {
    case 'primary_sidebar':
      return constraints.primary_sidebar_max_px;
    case 'agent_dock':
      return constraints.agent_dock_max_px;
    case 'bottom_dock':
      return constraints.bottom_dock_max_px;
  }
}

/**
 * Clamp a resize proposal into the region's bounds (ARCHITECTURE §3.1):
 * `clamp(px, region_min, min(region_max, bottom ? 0.5 × workbenchHeight : ∞))`,
 * never letting the editor drop below `editor_min_width_px`. Width-wise
 * proposals reserve both 6px handle tracks (ARCHITECTURE §1.1's arithmetic).
 * All constants come from `bootstrap.layout_constraints` — no literals.
 */
export function clampRegionSize(region: RegionId, sizePx: number, ctx: ClampContext): number {
  const { constraints, layout } = ctx;
  const min = regionMin(constraints, region);
  let max = regionMax(constraints, region);
  if (region === 'bottom_dock') {
    max = Math.min(max, constraints.bottom_dock_max_workbench_ratio * ctx.workbenchHeight);
  } else {
    const other: RegionId = region === 'primary_sidebar' ? 'agent_dock' : 'primary_sidebar';
    const otherTrack = regionOpen(layout, other) ? regionSize(layout, other) + constraints.resize_hit_zone_min_px : 0;
    const byEditor =
      ctx.workbenchWidth - ctx.activityBarWidth - otherTrack - constraints.resize_hit_zone_min_px - constraints.editor_min_width_px;
    max = Math.min(max, byEditor);
  }
  if (!Number.isFinite(sizePx)) return min;
  return Math.max(min, Math.min(max, sizePx));
}

/** Final sizes snap to whole CSS pixels (DESIGN §4). */
export function snapToWholePx(px: number): number {
  return Math.round(px);
}

/** Clamp every persisted size into bounds — applied to the bootstrap layout. */
function sanitizeLayout(layout: Layout, constraints: LayoutConstraints): Layout {
  return {
    ...layout,
    primary_sidebar_width_px: Math.max(
      constraints.primary_sidebar_min_px,
      Math.min(constraints.primary_sidebar_max_px, layout.primary_sidebar_width_px),
    ),
    agent_dock_width_px: Math.max(
      constraints.agent_dock_min_px,
      Math.min(constraints.agent_dock_max_px, layout.agent_dock_width_px),
    ),
    bottom_dock_height_px: Math.max(
      constraints.bottom_dock_min_px,
      Math.min(constraints.bottom_dock_max_px, layout.bottom_dock_height_px),
    ),
  };
}

type DragRecord = { region: RegionId; startSize: number; current: number };

// --- The store (module singleton; consumed via useSyncExternalStore) --------

type Listener = () => void;
const listeners = new Set<Listener>();

let state: LayoutState = {
  hydrated: false,
  layout: null,
  constraints: null,
  defaults: null,
  pending: null,
  dragging: null,
};
let snapshot = state;
const workbench: WorkbenchBox = { width: 0, height: 0, activityBarWidth: 0 };
let context: EnvelopeContext | null = null;
let drag: DragRecord | null = null;

function notify(): void {
  snapshot = {
    hydrated: state.hydrated,
    layout: state.layout,
    constraints: state.constraints,
    defaults: state.defaults,
    pending: state.pending,
    dragging: state.dragging,
  };
  for (const listener of listeners) listener();
}

export function subscribeLayout(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getLayoutSnapshot(): LayoutState {
  return snapshot;
}

function clampContextNow(): ClampContext | null {
  if (!state.constraints || !state.layout) return null;
  return {
    constraints: state.constraints,
    layout: state.layout,
    workbenchWidth: workbench.width,
    workbenchHeight: workbench.height,
    activityBarWidth: workbench.activityBarWidth,
  };
}

/**
 * ResizeRegion + SaveLayout, exactly once each, in order
 * (ARCHITECTURE §3.1: the save must await the resize — Tauri commands run
 * concurrently and a save landing first would persist the pre-drag layout).
 * The pending preview is applied immediately and cleared on failure
 * (rollback) or when the confirming event arrives.
 */
async function commitResizePair(region: RegionId, sizePx: number): Promise<void> {
  const ctx = context;
  const clampCtx = clampContextNow();
  if (!ctx || !clampCtx) return;
  const final = snapToWholePx(clampRegionSize(region, sizePx, clampCtx));
  state = { ...state, pending: { region, sizePx: final }, dragging: null };
  notify();
  try {
    await resizeRegion(region, final, ctx);
  } catch {
    state = { ...state, pending: null };
    notify();
    return; // the bridge already surfaced the error through its command-error path
  }
  try {
    await saveLayout(ctx);
  } catch {
    // surfaced by the bridge; the Rust layout is already applied
  }
}

export const layoutStore = {
  getSnapshot: getLayoutSnapshot,

  /** Hydrate from a bootstrap snapshot (ARCHITECTURE §3.4 step 1). */
  hydrate(bootstrap: BootstrapState): void {
    context = {
      core_instance_id: bootstrap.core_instance_id,
      workspace_id: bootstrap.workspace.id,
      session_id: null,
    };
    state = {
      hydrated: true,
      layout: sanitizeLayout(bootstrap.layout, bootstrap.layout_constraints),
      constraints: bootstrap.layout_constraints,
      defaults: bootstrap.layout,
      pending: null,
      dragging: null,
    };
    drag = null;
    notify();
  },

  /** Drop all held state (rebootstrap path, ARCHITECTURE §3.4). */
  reset(): void {
    context = null;
    drag = null;
    state = { hydrated: false, layout: null, constraints: null, defaults: null, pending: null, dragging: null };
    notify();
  },

  /**
   * Apply a validated `workspace_layout_changed` payload. This is the only
   * authoritative mutation path; the confirming event retires the preview.
   */
  applyLayoutEvent(layout: Layout): void {
    state = { ...state, layout, pending: null };
    notify();
  },

  /** Measured workbench geometry (editor-floor and 50% clamps). */
  setWorkbenchBox(box: WorkbenchBox): void {
    workbench.width = box.width;
    workbench.height = box.height;
    workbench.activityBarWidth = box.activityBarWidth;
  },

  /**
   * Re-clamp every open region against the last measured workbench box — the
   * window-resize path, purely local (no ResizeRegion/SaveLayout IPC): the
   * persisted layout (and any pending preview) is clamped in place and notify
   * only fires when a value actually changed, so the observer loop terminates.
   */
  reclampToWorkbench(): void {
    const cur = state.layout;
    const clampCtx = clampContextNow();
    if (!cur || !clampCtx) return;
    if (clampCtx.workbenchWidth <= 0 || clampCtx.workbenchHeight <= 0) return; // no measurement yet
    let layout: Layout = cur;
    let pending = state.pending;
    let changed = false;
    for (const region of Object.keys(REGION_SIZE_FIELD) as RegionId[]) {
      if (!regionOpen(cur, region)) continue;
      const field = REGION_SIZE_FIELD[region];
      const clamped = snapToWholePx(clampRegionSize(region, layout[field] as number, clampCtx));
      if (clamped !== (layout[field] as number)) {
        layout = { ...layout, [field]: clamped } as Layout;
        changed = true;
      }
      if (pending?.region === region) {
        const clampedPending = snapToWholePx(clampRegionSize(region, pending.sizePx, clampCtx));
        if (clampedPending !== pending.sizePx) {
          pending = { region, sizePx: clampedPending };
          changed = true;
        }
      }
    }
    if (!changed) return;
    state = { ...state, layout, pending };
    notify();
  },

  // --- Pointer drag lifecycle (ARCHITECTURE §3.1) ---------------------------

  /** `pointerdown`: record start; no IPC. */
  beginDrag(region: RegionId, startSize: number): void {
    drag = { region, startSize, current: startSize };
    state = { ...state, dragging: region };
    notify();
  },

  /**
   * `pointermove`: clamp the proposal and record it. No IPC, no notify —
   * the caller writes the returned size straight onto the grid container
   * via `el.style.setProperty` (DESIGN §1, ARCHITECTURE §6.2).
   */
  previewDrag(region: RegionId, proposalPx: number): number {
    const clampCtx = clampContextNow();
    if (!drag || drag.region !== region || !clampCtx) return proposalPx;
    const clamped = clampRegionSize(region, proposalPx, clampCtx);
    drag.current = clamped;
    return clamped;
  },

  isDragging(region: RegionId): boolean {
    return drag?.region === region;
  },

  /** `pointerup`: exactly one ResizeRegion, awaited, then SaveLayout. */
  endDrag(): void {
    if (!drag) return;
    const { region, current } = drag;
    drag = null;
    void commitResizePair(region, current);
  },

  /** A keyboard step / Home-End jump / double-click reset (DESIGN §4). */
  commitResize(region: RegionId, sizePx: number): void {
    void commitResizePair(region, sizePx);
  },

  // --- Toggles (ARCHITECTURE §3.2) ------------------------------------------

  /** Send ToggleSurface immediately, await it, then SaveLayout. No optimistic UI. */
  async toggle(surface: SurfaceId): Promise<void> {
    const ctx = context;
    if (!ctx) return;
    try {
      await toggleSurface(surface, ctx);
    } catch {
      return; // surfaced by the bridge
    }
    try {
      await saveLayout(ctx);
    } catch {
      // surfaced by the bridge
    }
  },

  /** The palette/menu Save Layout entry. */
  async saveNow(): Promise<void> {
    const ctx = context;
    if (!ctx) return;
    try {
      await saveLayout(ctx);
    } catch {
      // surfaced by the bridge
    }
  },
};

/** The displayed size of a region: a pending preview wins until the event lands. */
export function displayedSize(state: LayoutState, region: RegionId): number {
  if (state.pending?.region === region) return state.pending.sizePx;
  if (!state.layout) return 0;
  return regionSize(state.layout, region);
}
