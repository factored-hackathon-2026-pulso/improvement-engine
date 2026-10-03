import { defineConfig } from '@playwright/test';
const base = process.env.BASE_URL ?? 'http://127.0.0.1:5173';
export default defineConfig({
  testDir: 'tests/e2e', timeout: 30000, workers: 1,
  use: { baseURL: base, trace: 'retain-on-failure' },
  webServer: process.env.E2E_TARGET === 'real' ? undefined : [
    { command: 'node fixture-server/server.mjs', url: 'http://127.0.0.1:4010/healthz', reuseExistingServer: false },
    { command: 'npx vite --host 127.0.0.1', url: base, reuseExistingServer: false },
  ],
});
