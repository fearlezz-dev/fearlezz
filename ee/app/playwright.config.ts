// The host's pages in every state worth seeing, as stills: `ee-page-look` in fearlezz.toml.
// Fearlezz compares each still with the same test on the base commit, so there are no golden
// files; Playwright only writes the pictures. Each run gets a free port in FEARLEZZ_PORT, so
// two worktrees, or a worktree and its base, never collide.
import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.FEARLEZZ_PORT ?? 5174);
const out = process.env.FEARLEZZ_OUT;

export default defineConfig({
  testDir: "e2e",
  testMatch: "*.look.ts",
  fullyParallel: true,
  forbidOnly: true,
  webServer: {
    command: `vite --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: false,
    // The dev server builds the page's comparison to wasm first (vite.config.ts), which takes a
    // few minutes on a cold cache.
    timeout: 600_000,
  },
  // The full Chromium in its headless mode: nixpkgs ships it, not Playwright's headless shell.
  use: { baseURL: `http://localhost:${port}`, channel: "chromium" },
  expect: { toHaveScreenshot: { animations: "disabled", caret: "hide" } },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } } },
    { name: "mobile", use: { ...devices["Pixel 7"], defaultBrowserType: "chromium" } },
  ],
  // A still is `<project>/<screen>/<theme>`, such as `desktop/work/waits-for-you/dark`, which
  // the proof host groups by screen, viewport and theme.
  ...(out
    ? {
        snapshotPathTemplate: `${out}/stills/{projectName}/{arg}{ext}`,
        outputDir: `${out}/results`,
        reporter: [["line"], ["junit", { outputFile: `${out}/junit.xml` }]],
      }
    : { snapshotPathTemplate: "e2e/stills/{projectName}/{arg}{ext}", outputDir: "e2e/results" }),
});
