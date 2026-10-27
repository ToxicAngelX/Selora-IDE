// Layout-store tests (ARCHITECTURE §10): hydrate, constraint enforcement
// (min/max, the 50% workbench ceiling, the editor floor with handle tracks
// reserved), event application retiring the preview, and stale-event
// rejection at the gate seam.
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../bridge/bridge', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../bridge/bridge')>();
  return {
    ...actual,
    resizeRegion: vi.fn(),
    saveLayout: vi.fn(),
    toggleSurface: vi.fn(),
  };
});

import { resizeRegion, saveLayout, toggleSurface } from '../../bridge/bridge';
import {
  clampRegionSize,
  displayedSize,
  getLayoutSnapshot,
  layoutStore,
  regionMax,
  regionMin,
  snapToWholePx,
} from './layout-store';
import { FIXTURE_CONSTRAINTS, makeBootstrap, makeLayout } from '../../testing/fixtures';

const resizeRegionMock = vi.mocked(resizeRegion);
const saveLayoutMock = vi.mocked(saveLayout);
const toggleSurfaceMock = vi.mocked(toggleSurface);

const BOX = { width: 1440, height: 900, activityBarWidth: 48 };

/** The ClampContext the store derives from BOX + a layout. */
function clampCtx(layout = makeLayout(), overrides: Partial<{ workbenchWidth: number; workbenchHeight: number }> = {}) {
  return {
    constraints: FIXTURE_CONSTRAINTS,
    layout,
    workbenchWidth: BOX.width,
    workbenchHeight: BOX.height,
    activityBarWidth: BOX.activityBarWidth,
    ...overrides,
  };
}

function hydrate(layout = makeLayout()) {
  const bootstrap = makeBootstrap({ layout });
  layoutStore.hydrate(bootstrap);
  layoutStore.setWorkbenchBox(BOX);
  return bootstrap;
}

beforeEach(() => {
  vi.clearAllMocks();
  resizeRegionMock.mockResolvedValue(undefined);
  saveLayoutMock.mockResolvedValue(undefined);
  toggleSurfaceMock.mockResolvedValue(undefined);
  layoutStore.reset();
});

describe('hydrate (ARCHITECTURE §3.4 step 1)', () => {
  it('takes layout, constraints, and defaults from bootstrap', () => {
    const layout = makeLayout({ primary_sidebar_width_px: 320, agent_dock_open: true });
    hydrate(layout);
    const state = getLayoutSnapshot();
    expect(state.hydrated).toBe(true);
    expect(state.layout).toEqual(layout);
    expect(state.constraints).toEqual(FIXTURE_CONSTRAINTS);
    expect(state.defaults).toEqual(layout);
  });

  it('sanitizes out-of-bounds persisted sizes into the constraints', () => {
    hydrate(makeLayout({ primary_sidebar_width_px: 999, agent_dock_width_px: 10, bottom_dock_height_px: 5000 }));
    const layout = getLayoutSnapshot().layout!;
    expect(layout.primary_sidebar_width_px).toBe(480);
    expect(layout.agent_dock_width_px).toBe(240);
    expect(layout.bottom_dock_height_px).toBe(800);
  });
});

describe('clampRegionSize (ARCHITECTURE §3.1, §1.1)', () => {
  const ctx = clampCtx();

  it('clamps into the static min/max from layout_constraints', () => {
    expect(clampRegionSize('primary_sidebar', 100, ctx)).toBe(160);
    expect(clampRegionSize('primary_sidebar', 999, ctx)).toBe(480);
    expect(clampRegionSize('agent_dock', 100, ctx)).toBe(240);
    expect(clampRegionSize('agent_dock', 999, ctx)).toBe(600);
    expect(clampRegionSize('bottom_dock', 10, ctx)).toBe(120);
    // On a 2000px workbench the 50% ceiling (1000px) exceeds the static 800
    // ceiling, so the static bound binds.
    const tall = clampCtx(undefined, { workbenchHeight: 2000 });
    expect(clampRegionSize('bottom_dock', 9999, tall)).toBe(800);
  });

  it('clamps the bottom dock to 50% of the measured workbench height', () => {
    // 900px workbench → 450px ceiling, below the 800 static max.
    expect(clampRegionSize('bottom_dock', 700, ctx)).toBe(450);
    expect(clampRegionSize('bottom_dock', 300, ctx)).toBe(300);
  });

  it('never lets a width region push the editor below 400px, reserving both handle tracks', () => {
    // Sidebar: agent closed → available = 1440 - 48 - 6 - 400 = 986 → max 480.
    expect(clampRegionSize('primary_sidebar', 600, ctx)).toBe(480);
    // Agent open at 360: available = 1440 - 48 - (240 + 6) - 6 - 400 = 740.
    const agentOpenCtx = clampCtx(makeLayout({ primary_sidebar_width_px: 240 }));
    expect(clampRegionSize('agent_dock', 800, agentOpenCtx)).toBe(600);
    // Narrow workbench (960 window, 904 workbench): sidebar max = 904 - 48 - 6 - 400 = 450.
    const narrow = clampCtx(undefined, { workbenchWidth: 904 });
    expect(clampRegionSize('primary_sidebar', 480, narrow)).toBe(450);
    // Both docks open at minimum on the 960 window still fits: 48+160+6+400+6+240 = 860.
    expect(clampRegionSize('agent_dock', 240, clampCtx(makeLayout({ primary_sidebar_width_px: 160 }), { workbenchWidth: 904 }))).toBe(240);
  });

  it('rejects non-finite proposals by falling to the region minimum', () => {
    expect(clampRegionSize('primary_sidebar', Number.NaN, ctx)).toBe(160);
  });

  it('regionMin/regionMax expose the ARIA bounds from constraints', () => {
    expect(regionMin(FIXTURE_CONSTRAINTS, 'primary_sidebar')).toBe(160);
    expect(regionMax(FIXTURE_CONSTRAINTS, 'bottom_dock')).toBe(800);
  });
});

describe('workspace_layout_changed application (ARCHITECTURE §2.4)', () => {
  it('replaces the authoritative layout and retires the pending preview', async () => {
    hydrate(makeLayout({ primary_sidebar_width_px: 240 }));
    await layoutStore.commitResize('primary_sidebar', 320);
    expect(getLayoutSnapshot().pending).toEqual({ region: 'primary_sidebar', sizePx: 320 });
    layoutStore.applyLayoutEvent(makeLayout({ primary_sidebar_width_px: 320 }));
    expect(getLayoutSnapshot().layout?.primary_sidebar_width_px).toBe(320);
    expect(getLayoutSnapshot().pending).toBeNull();
    expect(displayedSize(getLayoutSnapshot(), 'primary_sidebar')).toBe(320);
  });

  it('a stale event never reaches the store — only validated events do (the gate owns rejection)', () => {
    hydrate();
    // The app-store gate drops stale events before reduceEvent runs; the
    // store has no stale path of its own. Applying twice is therefore
    // always a *newer* event:
    layoutStore.applyLayoutEvent(makeLayout({ primary_sidebar_width_px: 300 }));
    layoutStore.applyLayoutEvent(makeLayout({ primary_sidebar_width_px: 360 }));
    expect(getLayoutSnapshot().layout?.primary_sidebar_width_px).toBe(360);
  });

  it('re-clamps an oversized authoritative payload against the measured workbench box', () => {
    hydrate();
    // The event may carry a size persisted on a wider window and the
    // ResizeObserver won't refire for a dock re-widening, so the store must
    // clamp on application: 904 - 48 - 6 - 400 = 450 on this narrow box.
    layoutStore.setWorkbenchBox({ width: 904, height: 900, activityBarWidth: 48 });
    layoutStore.applyLayoutEvent(makeLayout({ primary_sidebar_width_px: 480 }));
    expect(getLayoutSnapshot().layout?.primary_sidebar_width_px).toBe(450);
  });
});

describe('resize command cadence (ARCHITECTURE §3.1)', () => {
  it('a committed resize sends exactly one ResizeRegion then one SaveLayout, snapped to whole pixels', async () => {
    hydrate();
    await layoutStore.commitResize('primary_sidebar', 244.7);
    expect(resizeRegionMock).toHaveBeenCalledTimes(1);
    expect(resizeRegionMock).toHaveBeenCalledWith('primary_sidebar', 245, expect.anything());
    expect(saveLayoutMock).toHaveBeenCalledTimes(1);
    expect(resizeRegionMock.mock.invocationCallOrder[0]).toBeLessThan(
      saveLayoutMock.mock.invocationCallOrder[0],
    );
  });

  it('a failed resize rolls the preview back and skips the save', async () => {
    hydrate();
    resizeRegionMock.mockRejectedValueOnce({ code: 'invalid_region_size', message: 'x', retryable: false });
    await layoutStore.commitResize('primary_sidebar', 320);
    expect(resizeRegionMock).toHaveBeenCalledTimes(1);
    expect(saveLayoutMock).not.toHaveBeenCalled();
    expect(getLayoutSnapshot().pending).toBeNull();
    // The authoritative layout is untouched — the event never came.
    expect(getLayoutSnapshot().layout?.primary_sidebar_width_px).toBe(240);
  });

  it('pointer drag: begin/preview/end with zero IPC until endDrag', async () => {
    hydrate();
    layoutStore.beginDrag('primary_sidebar', 240);
    expect(getLayoutSnapshot().dragging).toBe('primary_sidebar');
    const clamped = layoutStore.previewDrag('primary_sidebar', 900); // clamps to 480
    expect(clamped).toBe(480);
    expect(resizeRegionMock).not.toHaveBeenCalled();
    expect(saveLayoutMock).not.toHaveBeenCalled();
    layoutStore.endDrag();
    await vi.waitFor(() => expect(resizeRegionMock).toHaveBeenCalledTimes(1));
    expect(resizeRegionMock).toHaveBeenCalledWith('primary_sidebar', 480, expect.anything());
    expect(saveLayoutMock).toHaveBeenCalledTimes(1);
  });

  it('dragging state clears once the drag ends', async () => {
    hydrate();
    layoutStore.beginDrag('agent_dock', 360);
    expect(getLayoutSnapshot().dragging).toBe('agent_dock');
    layoutStore.endDrag();
    await vi.waitFor(() => expect(saveLayoutMock).toHaveBeenCalled());
    expect(getLayoutSnapshot().dragging).toBeNull();
  });
});

describe('toggle cadence (ARCHITECTURE §3.2)', () => {
  it('sends ToggleSurface immediately, then SaveLayout — with no optimistic layout change', async () => {
    hydrate();
    const promise = layoutStore.toggle('agent');
    expect(toggleSurfaceMock).toHaveBeenCalledTimes(1);
    expect(getLayoutSnapshot().layout?.agent_dock_open).toBe(false); // no optimistic UI
    await promise;
    expect(saveLayoutMock).toHaveBeenCalledTimes(1);
    expect(toggleSurfaceMock.mock.invocationCallOrder[0]).toBeLessThan(
      saveLayoutMock.mock.invocationCallOrder[0],
    );
    // State still unchanged — only the event flips it.
    expect(getLayoutSnapshot().layout?.agent_dock_open).toBe(false);
    layoutStore.applyLayoutEvent(makeLayout({ agent_dock_open: true }));
    expect(getLayoutSnapshot().layout?.agent_dock_open).toBe(true);
  });

  it('a failed toggle skips the save', async () => {
    hydrate();
    toggleSurfaceMock.mockRejectedValueOnce(new Error('boom'));
    await layoutStore.toggle('agent');
    expect(saveLayoutMock).not.toHaveBeenCalled();
  });

  it('saveNow sends only SaveLayout (the palette entry)', async () => {
    hydrate();
    await layoutStore.saveNow();
    expect(saveLayoutMock).toHaveBeenCalledTimes(1);
    expect(toggleSurfaceMock).not.toHaveBeenCalled();
  });
});

describe('reset (the rebootstrap path)', () => {
  it('discards all held state', () => {
    hydrate();
    layoutStore.applyLayoutEvent(makeLayout({ agent_dock_open: true }));
    layoutStore.reset();
    const state = getLayoutSnapshot();
    expect(state.hydrated).toBe(false);
    expect(state.layout).toBeNull();
    expect(state.constraints).toBeNull();
    expect(state.defaults).toBeNull();
    expect(state.pending).toBeNull();
  });
});

describe('snapToWholePx (DESIGN §4)', () => {
  it('snaps final sizes to whole CSS pixels', () => {
    expect(snapToWholePx(244.4)).toBe(244);
    expect(snapToWholePx(244.5)).toBe(245);
  });
});
