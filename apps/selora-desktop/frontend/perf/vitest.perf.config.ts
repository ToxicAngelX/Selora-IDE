/// <reference types="vitest" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The performance spec is deliberately outside the normal `npm test` glob so a
// routine test run stays fast. scripts/perf.ps1 invokes this config directly.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'happy-dom',
    globals: false,
    css: false,
    setupFiles: ['src/testing/setup.ts'],
    include: ['perf/**/*.perf.tsx'],
    testTimeout: 120_000,
  },
});