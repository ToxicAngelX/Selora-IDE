/// <reference types="vitest" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import packageJson from './package.json';

// The repository URL lives in package.json (repository.url) so the About
// dialog and the update check cannot drift from the published repo. The
// Rust side reads [workspace.package] repository from Cargo.toml; CI's
// scripts/check-release-metadata.ps1 asserts the two agree.
const repositoryUrl = packageJson.repository.url.replace(/\.git$/, '');

export default defineConfig({
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
    __REPOSITORY_URL__: JSON.stringify(repositoryUrl),
  },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  test: {
    environment: 'happy-dom',
    globals: false,
    css: false,
    setupFiles: ['src/testing/setup.ts'],
  },
});
