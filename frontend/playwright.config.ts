import { defineConfig, devices } from '@playwright/test'

/**
 * The dashboard, in a real browser, against a real Doppel.
 *
 * No `webServer`: each spec file starts its own server, because the
 * configuration under test -- public, private, dashboard off -- is a property of
 * the server rather than of the page. See e2e/src/server.ts.
 */
export default defineConfig({
  // The specs live under `e2e/tests`; `e2e/src` is the harness they drive, and
  // Playwright must not go looking for tests in it.
  testDir: './e2e/tests',
  // One browser. The dashboard uses nothing engine-specific, and three browsers
  // would triple a suite whose subject is this application's behaviour.
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: process.env.CI ? [['html', { open: 'never' }], ['list']] : 'list',
  use: {
    // Only on a failure: a trace for every passing test is minutes of upload for
    // something nobody opens.
    trace: 'retain-on-failure',
  },
  timeout: 30_000,
})
