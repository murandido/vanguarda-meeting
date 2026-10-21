import { fileURLToPath } from "node:url";
import { defineConfig, devices } from "@playwright/test";

// Raiz do workspace Cargo (um nível acima de e2e/).
const raiz = fileURLToPath(new URL("..", import.meta.url));

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL: "http://127.0.0.1:3000",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        // DPR 1 para que coordenadas CSS == pixels do canvas nas asserções.
        viewport: { width: 1280, height: 800 },
        deviceScaleFactor: 1,
      },
    },
  ],
  webServer: {
    command: "cargo run -p server",
    cwd: raiz,
    env: {
      DATABASE_URL: "sqlite::memory:",
      BIND: "127.0.0.1:3000",
      // Must match baseURL: POSTs and WebSockets check the Origin header.
      PUBLIC_URL: "http://127.0.0.1:3000",
      // Login de desenvolvimento: os testes entram por /auth/dev/{login}.
      DEV_LOGIN: "1",
      // caio and dani have Ritmo weeks of their own (closing makes a week
      // read-only, so they must not share it with the other tests).
      ALLOWED_GITHUB_USERS: "ana,bia,caio,dani",
      // A Tuesday (the default meeting day): the week can be closed today.
      DEV_TODAY: "2026-10-20",
    },
    url: "http://127.0.0.1:3000/health",
    reuseExistingServer: !process.env.CI,
    // Primeira compilação do cargo pode demorar.
    timeout: 180_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
