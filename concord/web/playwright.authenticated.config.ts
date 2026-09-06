import { defineConfig } from '@playwright/test';

const port = Number(process.env.CONCORD_FRONTEND_PORT);
if (!Number.isInteger(port) || port < 1) throw new Error('CONCORD_FRONTEND_PORT is required');

export default defineConfig({
  testDir: './tests/authenticated',
  outputDir: process.env.CONCORD_PLAYWRIGHT_OUTPUT_DIR ?? `test-results/authenticated-${process.pid}`,
  fullyParallel: false,
  retries: 0,
  workers: 1,
  reporter: 'line',
  use: { baseURL: `http://127.0.0.1:${port}`, browserName: 'chromium', headless: true },
  webServer: {
    command: `npm run dev -- --host 127.0.0.1 --port ${port}`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
