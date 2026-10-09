// Vitest setup: @testing-library/react's auto-cleanup only self-registers
// when a global `afterEach` exists; with `globals: false` we register it
// ourselves so component tests never share a DOM.
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

afterEach(() => {
  cleanup();
});
