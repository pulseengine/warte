import { defineConfig } from "@playwright/test";

// Modelled on rivet's tests/playwright: one real `warte serve` (release
// build), serial, Chromium. The server runs in fixture/ so it picks up
// fixture/warte.toml and the synthetic ground-station state.
export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  timeout: 30_000,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:3013",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "cargo run --release --manifest-path ../../../Cargo.toml -- serve",
    cwd: "fixture",
    url: "http://127.0.0.1:3013/api/health",
    timeout: 300_000,
    reuseExistingServer: !process.env.CI,
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
});
