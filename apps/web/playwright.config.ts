import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    baseURL: process.env.PLAYWRIGHT_BASE_URL || 'http://127.0.0.1:4173',
    trace: 'on-first-retry'
  },
  webServer:
    process.env.PLAYWRIGHT_EXTERNAL_SERVER === '1'
      ? undefined
      : {
          command: 'pnpm build:self-host && PORT=4173 node build-node',
          port: 4173,
          env: {
            PIQAE_AUTH_MODE: 'demo',
            PUBLIC_PIQAE_DASHBOARD_MODE: 'demo'
          },
          reuseExistingServer: !process.env.CI,
          timeout: 120_000
        },
  projects: [
    { name: 'desktop-chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'mobile-chromium', use: { ...devices['Pixel 7'] } }
  ]
});
