import { defineConfig, devices } from '@playwright/test';

// Two throwaway servers, one per project, so the desktop and phone suites each start from an empty library.
const desktopPort = Number(process.env.E2E_PORT ?? 8799);
const phonePort = Number(process.env.E2E_PHONE_PORT ?? 8798);

const server = (port: number) => ({
  command: 'node e2e/serve.mjs',
  url: `http://127.0.0.1:${port}/readyz`,
  reuseExistingServer: false,
  timeout: 120_000,
  env: { E2E_PORT: String(port) },
});

// The tests run against a real server binary (with ffmpeg) in a throwaway data directory.
// Build first: `npm run build` here, then `cargo build` at the repository root (the UI is embedded).
export default defineConfig({
  testDir: 'e2e',
  timeout: 60_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  workers: 1,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    trace: 'off',
    launchOptions: process.env.PW_CHROMIUM ? { executablePath: process.env.PW_CHROMIUM } : {},
  },
  projects: [
    {
      name: 'desktop',
      testMatch: /sorting\.spec\.ts/,
      use: { baseURL: `http://127.0.0.1:${desktopPort}` },
    },
    {
      // A phone: touch input, a 412 x 915 viewport, mobile user agent.
      name: 'phone',
      testMatch: /mobile\.spec\.ts/,
      use: { ...devices['Pixel 7'], baseURL: `http://127.0.0.1:${phonePort}` },
    },
  ],
  webServer: [server(desktopPort), server(phonePort)],
});
