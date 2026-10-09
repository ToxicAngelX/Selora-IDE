/// <reference types="vitest" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  test: {
    environment: 'happy-dom',
    globals: false,
    css: false,
    setupFiles: ['src/testing/setup.ts'],
  },
});
