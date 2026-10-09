// ResizeHandle cadence tests (ARCHITECTURE §10): a synthesized pointer
// sequence sends zero `invoke` during the drag and exactly
// [ResizeRegion, SaveLayout] — in that order — on pointerup; each keyboard
// completion (Arrow ±4px, Shift+Arrow ±16px, Home/End, double-click reset)
// sends the same single pair. The invoke-level mock exercises the whole
// stack: handle → layout store → bridge → envelope assembly.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render } from '@testing-library/react';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { invoke } from '@tauri-apps/api/core';
import { ResizeHandle } from './ResizeHandle';
import { getLayoutSnapshot, layoutStore } from '../features/layout/layout-store';
import { makeBootstrap, makeLayout } from '../testing/fixtures';
import type { RegionId } from '../bridge/bridge';

const invokeMock = vi.mocked(invoke);

function dispatches(): Array<{ type: string; payload?: unknown }> {
  return invokeMock.mock.calls
    .filter(([command]) => command === 'dispatch_command')
    .map(([, args]) => (args as { envelope: { payload: { type: string; payload?: unknown } } }).envelope.payload);
}

function setup(layout = makeLayout()) {
  layoutStore.hydrate(makeBootstrap({ layout }));
  layoutStore.setWorkbenchBox({ width: 1440, height: 900, activityBarWidth: 48 });
  const onPreview = vi.fn();
  const utils = render(<ResizeHandle region="primary_sidebar" onPreview={onPreview} />);
  return { onPreview, handle: utils.container.firstElementChild as HTMLElement, ...utils };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  layoutStore.reset();
  // Keyboard steps come from the token layer (DESIGN §4) — provide the
  // tokens the way the real stylesheet does.
  document.documentElement.style.setProperty('--space-1', '4px');
  document.documentElement.style.setProperty('--space-4', '16px');
});

describe('pointer drag cadence (ARCHITECTURE §3.1)', () => {
  it('zero IPC during the drag; exactly [resize_region, save_layout] on pointerup, in order', async () => {
    const { handle, onPreview } = setup();
    fireEvent.pointerDown(handle, { pointerId: 1, button: 0, clientX: 200, clientY: 100 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 220, clientY: 100 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 250, clientY: 100 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 260, clientY: 100 });
    // Zero IPC during the drag; the preview wrote CSS custom properties.
    expect(invokeMock).not.toHaveBeenCalled();
    expect(onPreview).toHaveBeenCalledTimes(3);
    expect(onPreview).toHaveBeenLastCalledWith('primary_sidebar', 300);

    await act(async () => {
      fireEvent.pointerUp(handle, { pointerId: 1, clientX: 260, clientY: 100 });
    });

    const sent = dispatches();
    expect(sent).toHaveLength(2);
    expect(sent[0]).toEqual({ type: 'resize_region', payload: { region: 'primary_sidebar', size_px: 300 } });
    expect(sent[1]).toEqual({ type: 'save_layout' });
    expect(invokeMock.mock.invocationCallOrder[0]).toBeLessThan(invokeMock.mock.invocationCallOrder[1]);
  });

  it('clamps an out-of-range drag client-side before the single dispatch', async () => {
    const { handle } = setup();
    fireEvent.pointerDown(handle, { pointerId: 1, button: 0, clientX: 200, clientY: 100 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 5000, clientY: 100 });
    await act(async () => {
      fireEvent.pointerUp(handle, { pointerId: 1, clientX: 5000, clientY: 100 });
    });
    const sent = dispatches();
    expect(sent).toEqual([
      { type: 'resize_region', payload: { region: 'primary_sidebar', size_px: 480 } },
      { type: 'save_layout' },
    ]);
  });

  it('the pending preview survives until the confirming event retires it', async () => {
    const { handle } = setup();
    fireEvent.pointerDown(handle, { pointerId: 1, button: 0, clientX: 200, clientY: 100 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 260, clientY: 100 });
    await act(async () => {
      fireEvent.pointerUp(handle, { pointerId: 1, clientX: 260, clientY: 100 });
    });
    expect(getLayoutSnapshot().pending).toEqual({ region: 'primary_sidebar', sizePx: 300 });
    layoutStore.applyLayoutEvent(makeLayout({ primary_sidebar_width_px: 300 }));
    expect(getLayoutSnapshot().pending).toBeNull();
    expect(getLayoutSnapshot().layout?.primary_sidebar_width_px).toBe(300);
  });

  it('a drag without movement still sends the idempotent pair', async () => {
    const { handle } = setup();
    fireEvent.pointerDown(handle, { pointerId: 1, button: 0, clientX: 200, clientY: 100 });
    await act(async () => {
      fireEvent.pointerUp(handle, { pointerId: 1, clientX: 200, clientY: 100 });
    });
    expect(dispatches()).toEqual([
      { type: 'resize_region', payload: { region: 'primary_sidebar', size_px: 240 } },
      { type: 'save_layout' },
    ]);
  });
});

describe('keyboard resize cadence (DESIGN §4, ARCHITECTURE §3.1)', () => {
  it('Arrow steps 4px (--space-1) and Shift+Arrow steps 16px (--space-4)', async () => {
    const { handle } = setup();
    fireEvent.keyDown(handle, { key: 'ArrowRight' });
    await act(async () => {});
    expect(dispatches()).toEqual([
      { type: 'resize_region', payload: { region: 'primary_sidebar', size_px: 244 } },
      { type: 'save_layout' },
    ]);

    invokeMock.mockClear();
    fireEvent.keyDown(handle, { key: 'ArrowRight', shiftKey: true });
    await act(async () => {});
    expect(dispatches()).toEqual([
      { type: 'resize_region', payload: { region: 'primary_sidebar', size_px: 260 } },
      { type: 'save_layout' },
    ]);
  });

  it('ArrowLeft shrinks; Home/End jump to the region min/max from layout_constraints', async () => {
    const { handle } = setup(makeLayout({ primary_sidebar_width_px: 300 }));
    fireEvent.keyDown(handle, { key: 'ArrowLeft' });
    await act(async () => {});
    expect(dispatches()[0]).toEqual({ type: 'resize_region', payload: { region: 'primary_sidebar', size_px: 296 } });

    invokeMock.mockClear();
    fireEvent.keyDown(handle, { key: 'Home' });
    await act(async () => {});
    expect(dispatches()[0]).toEqual({ type: 'resize_region', payload: { region: 'primary_sidebar', size_px: 160 } });

    invokeMock.mockClear();
    fireEvent.keyDown(handle, { key: 'End' });
    await act(async () => {});
    expect(dispatches()[0]).toEqual({ type: 'resize_region', payload: { region: 'primary_sidebar', size_px: 480 } });
  });

  it('double-click resets to the LayoutDto value delivered in bootstrap.layout', async () => {
    const { handle } = setup(makeLayout({ primary_sidebar_width_px: 240 }));
    // A user resize event moved the live layout away from the bootstrap value.
    layoutStore.applyLayoutEvent(makeLayout({ primary_sidebar_width_px: 380 }));
    fireEvent.doubleClick(handle);
    await act(async () => {});
    expect(dispatches()).toEqual([
      { type: 'resize_region', payload: { region: 'primary_sidebar', size_px: 240 } },
      { type: 'save_layout' },
    ]);
  });

  it('the agent dock grows on ArrowLeft; the bottom dock grows on ArrowDown', async () => {
    layoutStore.hydrate(makeBootstrap({ layout: makeLayout({ agent_dock_open: true, bottom_dock_open: true }) }));
    layoutStore.setWorkbenchBox({ width: 1440, height: 900, activityBarWidth: 48 });
    const onPreview = vi.fn();
    const agent = render(<ResizeHandle region="agent_dock" onPreview={onPreview} />);
    const agentHandle = agent.container.firstElementChild as HTMLElement;
    fireEvent.keyDown(agentHandle, { key: 'ArrowLeft' });
    await act(async () => {});
    expect(dispatches()[0]).toEqual({ type: 'resize_region', payload: { region: 'agent_dock', size_px: 364 } });

    invokeMock.mockClear();
    const bottom = render(<ResizeHandle region="bottom_dock" onPreview={onPreview} />);
    const bottomHandle = bottom.container.firstElementChild as HTMLElement;
    fireEvent.keyDown(bottomHandle, { key: 'ArrowDown' });
    await act(async () => {});
    expect(dispatches()[0]).toEqual({ type: 'resize_region', payload: { region: 'bottom_dock', size_px: 204 } });
  });
});

describe('separator semantics (DESIGN §4)', () => {
  it('carries role, orientation, label, and ARIA bounds from layout_constraints', () => {
    const { handle } = setup();
    expect(handle.getAttribute('role')).toBe('separator');
    expect(handle.getAttribute('tabindex')).toBe('0');
    expect(handle.getAttribute('aria-orientation')).toBe('vertical');
    expect(handle.getAttribute('aria-label')).toBe('Resize sidebar');
    expect(Number(handle.getAttribute('aria-valuemin'))).toBe(160);
    expect(Number(handle.getAttribute('aria-valuemax'))).toBe(480);
    expect(Number(handle.getAttribute('aria-valuenow'))).toBe(240);
  });

  it('the bottom dock handle is horizontal and reads its own bounds', () => {
    layoutStore.hydrate(makeBootstrap());
    layoutStore.setWorkbenchBox({ width: 1440, height: 900, activityBarWidth: 48 });
    const { container } = render(<ResizeHandle region="bottom_dock" onPreview={vi.fn()} />);
    const handle = container.firstElementChild as HTMLElement;
    expect(handle.getAttribute('aria-orientation')).toBe('horizontal');
    expect(handle.getAttribute('aria-label')).toBe('Resize panel');
    expect(Number(handle.getAttribute('aria-valuemin'))).toBe(120);
    expect(Number(handle.getAttribute('aria-valuemax'))).toBe(800);
    expect(Number(handle.getAttribute('aria-valuenow'))).toBe(200);
  });

  it('off-axis arrows do nothing', async () => {
    const { handle } = setup();
    fireEvent.keyDown(handle, { key: 'ArrowDown' });
    fireEvent.keyDown(handle, { key: 'ArrowUp' });
    await act(async () => {});
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
