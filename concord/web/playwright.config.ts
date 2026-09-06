import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/browser',
  outputDir: process.env.CONCORD_PLAYWRIGHT_OUTPUT_DIR ?? `test-results/browser-${process.pid}`,
  fullyParallel: false,
  retries: 0,
  workers: 1,
  reporter: 'line',
  use: {
    baseURL: 'http://127.0.0.1:4174',
    browserName: 'chromium',
    headless: true,
  },
  webServer: {
    command: 'npm run dev -- --host 127.0.0.1 --port 4174',
    url: 'http://127.0.0.1:4174/test-harness.html',
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
