import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "e2e",
  workers: 1,
  timeout: 30000,
  use: {
    baseURL: "http://127.0.0.1:37419",
    permissions: ["clipboard-read", "clipboard-write"],
    viewport: { width: 1200, height: 900 },
  },
  reporter: "list",
});
