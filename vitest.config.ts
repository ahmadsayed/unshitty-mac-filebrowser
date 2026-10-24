import { defineConfig } from "vitest/config";

// Frontend tests run the real app in jsdom against the in-memory backend in
// dev/mock-tauri/, the same stand-in `npm run dev:mock` uses. Coverage goes to
// coverage/coverage-final.json, which the CRAP gate hands to fallow.
export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify("0.0.0-test"),
  },
  resolve: {
    alias: {
      "@tauri-apps/api/core": "/dev/mock-tauri/core.ts",
      "@tauri-apps/api/path": "/dev/mock-tauri/path.ts",
      "@tauri-apps/plugin-opener": "/dev/mock-tauri/opener.ts",
      "@tauri-apps/api/webview": "/dev/mock-tauri/webview.ts",
    },
  },
  test: {
    environment: "jsdom",
    include: ["tests/**/*.test.ts"],
    setupFiles: ["tests/setup.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.ts", "dev/mock-tauri/**/*.ts"],
      reporter: ["json", "text-summary"],
      reportsDirectory: "coverage",
    },
  },
});
