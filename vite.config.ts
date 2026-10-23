import { defineConfig } from "vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;
// `npm run dev:mock` swaps the Tauri API for an in-memory fake so the UI can
// be previewed in a plain browser.
const mock = process.env.MOCK_TAURI === "1";

// https://vite.dev/config/
export default defineConfig(() => ({
  resolve: mock
    ? {
        alias: {
          "@tauri-apps/api/core": "/dev/mock-tauri/core.ts",
          "@tauri-apps/api/path": "/dev/mock-tauri/path.ts",
        },
      }
    : undefined,

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
