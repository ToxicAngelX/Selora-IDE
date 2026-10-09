// Selora IDE — Phase Z2 region resize handle.
//
// ARCHITECTURE §3.1 / DESIGN §4: a ≥6px hit zone that is its own grid track
// centered on the region boundary; zero IPC during a drag (the pointermove
// path clamps locally and writes the CSS custom property on the grid
// container through `onPreview`); exactly one ResizeRegion on pointerup,
// awaited, then SaveLayout. Keyboard: Arrow ±4px (--space-1), Shift+Arrow
// ±16px (--space-4), Home/End to the region bounds, double-click resets to
// the bootstrap-delivered default — every completion is the same
// ResizeRegion + SaveLayout pair. The 1px lit line is the focus indicator
// (no outline around a 6px strip).
import { useRef, useSyncExternalStore } from 'react';
import {
  displayedSize,
  getLayoutSnapshot,
  layoutStore,
  regionMax,
  regionMin,
  regionSize,
  subscribeLayout,
  type LayoutState,
} from '../features/layout/layout-store';
import { readTokenNumber } from '../features/layout/dom-tokens';
import type { RegionId } from '../bridge/bridge';

const REGION_LABEL: Record<RegionId, string> = {
  primary_sidebar: 'Resize sidebar',
  agent_dock: 'Resize agent dock',
  bottom_dock: 'Resize panel',
};

type DragStart = {
  pointerId: number;
  startClientX: number;
  startClientY: number;
  startSize: number;
};

export type ResizeHandleProps = {
  region: RegionId;
  /** Writes the clamped preview onto the grid container (el.style.setProperty). */
  onPreview: (region: RegionId, sizePx: number) => void;
};

export function ResizeHandle({ region, onPreview }: ResizeHandleProps) {
  const state: LayoutState = useSyncExternalStore(subscribeLayout, getLayoutSnapshot);
  const dragRef = useRef<DragStart | null>(null);

  const { layout, constraints, defaults } = state;
  if (!layout || !constraints || !defaults) return null;

  const sizeNow = displayedSize(state, region);
  const min = regionMin(constraints, region);
  const max = regionMax(constraints, region);
  const orientation = region === 'bottom_dock' ? 'horizontal' : 'vertical';

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const startSize = displayedSize(layoutStore.getSnapshot(), region);
    dragRef.current = {
      pointerId: event.pointerId,
      startClientX: event.clientX,
      startClientY: event.clientY,
      startSize,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    layoutStore.beginDrag(region, startSize);
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    // The boundary moves with the pointer; the region's size derives from it.
    const delta =
      region === 'primary_sidebar'
        ? event.clientX - drag.startClientX
        : region === 'agent_dock'
          ? drag.startClientX - event.clientX
          : event.clientY - drag.startClientY;
    const proposal = drag.startSize + delta;
    const clamped = layoutStore.previewDrag(region, proposal);
    onPreview(region, clamped);
  };

  const finishDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    layoutStore.endDrag();
  };

  // DESIGN §4: steps are the --space-1 / --space-4 tokens, never literals.
  // A missing token is reported, not papered over.
  const stepFor = (token: string): number | null => {
    const value = readTokenNumber(token);
    if (value === null || value <= 0) {
      console.error(`Selora: token ${token} is missing or unparsable; keyboard resize is disabled`);
      return null;
    }
    return value;
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      layoutStore.commitResize(region, event.key === 'Home' ? min : max);
      return;
    }
    let grow: boolean;
    if (region === 'bottom_dock') {
      if (event.key === 'ArrowDown') grow = true;
      else if (event.key === 'ArrowUp') grow = false;
      else return;
    } else {
      const growKey = region === 'primary_sidebar' ? 'ArrowRight' : 'ArrowLeft';
      if (event.key === growKey) grow = true;
      else if (event.key === (region === 'primary_sidebar' ? 'ArrowLeft' : 'ArrowRight')) grow = false;
      else return;
    }
    event.preventDefault();
    const step = stepFor(event.shiftKey ? '--space-4' : '--space-1');
    if (step === null) return;
    const size = displayedSize(layoutStore.getSnapshot(), region);
    layoutStore.commitResize(region, size + (grow ? step : -step));
  };

  const onDoubleClick = () => {
    // DESIGN §4: reset to the LayoutDto value delivered in bootstrap.layout.
    layoutStore.commitResize(region, regionSize(defaults, region));
  };

  return (
    <div
      className={`region-handle region-handle-${orientation}${state.dragging === region ? ' is-dragging' : ''}`}
      role="separator"
      tabIndex={0}
      aria-orientation={orientation}
      aria-label={REGION_LABEL[region]}
      aria-valuemin={Math.round(min)}
      aria-valuemax={Math.round(max)}
      aria-valuenow={Math.round(sizeNow)}
      aria-valuetext={`${Math.round(sizeNow)} pixels`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={finishDrag}
      onPointerCancel={finishDrag}
      onKeyDown={onKeyDown}
      onDoubleClick={onDoubleClick}
    />
  );
}
