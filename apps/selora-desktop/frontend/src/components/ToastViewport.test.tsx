// Toast live-region contract: each toast owns the polite status announcement;
// the viewport is a labelled region and must never steal focus.
import { afterEach, describe, expect, it } from 'vitest';
import { act, render } from '@testing-library/react';
import { ToastViewport } from './ToastViewport';
import { toastStore } from '../features/toasts/toast-store';

afterEach(() => {
  toastStore.clearAll();
});

describe('ToastViewport live region', () => {
  it('announces each toast through role=status without focusing it', () => {
    act(() => {
      toastStore.pushLocal('info', 'Saved');
    });
    const { getByRole, getByText } = render(<ToastViewport />);
    const region = getByRole('region', { name: 'Live notifications' });
    expect(region.getAttribute('aria-live')).toBeNull();
    const status = getByRole('status');
    expect(status.textContent).toContain('Saved');
    expect(document.activeElement).toBe(document.body);
    expect(getByText('Saved')).toBeTruthy();
  });
});
