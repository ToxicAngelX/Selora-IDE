// Vitest setup: @testing-library/react's auto-cleanup only self-registers
// when a global `afterEach` exists; with `globals: false` we register it
// ourselves so component tests never share a DOM.
import { cleanup, configure } from '@testing-library/react';
import { afterEach } from 'vitest';

// The shell lazy-loads the sidebar, dock, editor and agent chunks; on a busy
// CI machine a cold import can exceed Testing Library's 1 s default even
// though the component is correct. Keep the assertions strict, give the
// dynamic imports room.
configure({ asyncUtilTimeout: 5000 });

afterEach(() => {
  cleanup();
});
