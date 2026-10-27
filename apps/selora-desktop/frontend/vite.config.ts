/// <reference types="vitest" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import packageJson from './package.json';

export default defineConfig({
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
  },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  test: {
    environment: 'happy-dom',
    globals: false,
    css: false,
    setupFiles: ['src/testing/setup.ts'],
    // Lazy chunks share happy-dom under parallel workers and can starve each
    // other; run files serially so `npm test` is deterministic.
    fileParallelism: false,
    // The shell mounts several lazy chunks (AgentDock, CodeMirror, bottom
    // dock). Under a loaded full-suite run their dynamic import can exceed
    // the 5s default; 20s keeps the gate deterministic without weakening a
    // single assertion.
    testTimeout: 20_000,
  },
});
