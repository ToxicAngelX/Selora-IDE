// Editor minimap keyboard parity: the click-to-navigate affordance is also a
// focusable slider with arrow/Page/Home/End keys (UI_SPEC §17: every
// interactive control has a keyboard equivalent).
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render } from '@testing-library/react';
import { EditorMinimap } from './EditorMinimap';
import type { CodeLineData } from './editor-types';

const lines: CodeLineData[] = Array.from({ length: 20 }, (_, index) => ({
  lineNum: index + 1,
  tokens: [{ text: `line ${index + 1}`, type: 'syn-default' }],
}));

describe('EditorMinimap', () => {
  it('exposes the minimap as a labelled vertical slider', () => {
    const { getByRole } = render(
      <EditorMinimap lines={lines} scrollTop={0} scrollHeight={1000} clientHeight={100} />,
    );
    const slider = getByRole('slider', { name: 'Code minimap' });
    expect(slider.getAttribute('aria-orientation')).toBe('vertical');
    expect(slider.getAttribute('aria-valuemin')).toBe('1');
    expect(slider.getAttribute('aria-valuemax')).toBe('20');
  });

  it('navigates with arrows, Page keys, Home and End', () => {
    const onNavigateLine = vi.fn();
    const { getByRole } = render(
      <EditorMinimap
        lines={lines}
        scrollTop={0}
        scrollHeight={1000}
        clientHeight={100}
        onNavigateLine={onNavigateLine}
      />,
    );
    const slider = getByRole('slider', { name: 'Code minimap' });
    fireEvent.keyDown(slider, { key: 'ArrowDown' });
    expect(onNavigateLine).toHaveBeenLastCalledWith(2);
    fireEvent.keyDown(slider, { key: 'PageDown' });
    expect(onNavigateLine).toHaveBeenLastCalledWith(12);
    fireEvent.keyDown(slider, { key: 'End' });
    expect(onNavigateLine).toHaveBeenLastCalledWith(20);
    fireEvent.keyDown(slider, { key: 'ArrowDown' });
    expect(onNavigateLine).toHaveBeenLastCalledWith(20);
    fireEvent.keyDown(slider, { key: 'Home' });
    expect(onNavigateLine).toHaveBeenLastCalledWith(1);
  });
});
