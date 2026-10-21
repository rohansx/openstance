import { defineConfig, devices } from '@playwright/test';

// Smoke tests against a real `openstance serve` with the UI embedded
// (P4-UI-06). e2e/global-setup.ts builds a throwaway home from the fictional
// fixtures/demo dataset and starts the binary named by OPENSTANCE_BIN
// (default: ../target/debug/openstance). Build the UI and then the binary
// first: `npm run build && cargo build -p openstance`.
export default defineConfig({
  testDir: 'e2e',
  globalSetup: './e2e/global-setup.ts',
  // One server and one graph: tests that write run in order.
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  timeout: 30_000,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    ...devices['Desktop Chrome'],
    trace: 'retain-on-failure',
  },
});
