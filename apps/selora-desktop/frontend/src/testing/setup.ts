// Vitest setup: @testing-library/react's auto-cleanup only self-registers
// when a global `afterEach` exists; with `globals: false` we register it
// ourselves so component tests never share a DOM.
import { cleanup, configure } from '@testing-library/react';
import { afterEach } from 'vitest';

// Lazy region chunks are transformed on first request. A cold Vitest worker can
// take a few seconds to resolve the symbol-heavy Agent/Dock graphs, so keep the
// async query budget above the old 1s default instead of racing the loader.
configure({ asyncUtilTimeout: 5000 });

afterEach(() => {
  cleanup();
});
