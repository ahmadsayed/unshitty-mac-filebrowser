// Only the backend knows where the Trash is: ~/.Trash on a Mac,
// ~/.local/share/Trash/files on Linux. When it can't say, there is no Trash
// place, rather than a guess that is wrong on Linux.
import { beforeAll, expect, test } from "vitest";
import "../dev/mock-tauri/core";
import { $$, boot, goTo } from "./app";

type MockCalls = { failNext(cmd: string, error: string): void };

beforeAll(async () => {
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)", configurable: true });
  (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls.failNext("trash_dir", "IPC failed");
  await boot();
});

test("without the backend's Trash folder there is no Trash place, and ~/.Trash is a folder like any other", async () => {
  // The app took its Linux code paths.
  expect(document.documentElement.dataset.theme).toBe("gnome");
  const rows = $$(".sidebar-row");
  expect(rows.map((row) => row.textContent?.trim())).not.toContain("Trash");
  expect(rows.map((row) => row.dataset.location)).not.toContain("/Users/me/.Trash");

  await goTo("/Users/me/.Trash");
  expect($$("#crumbs .crumb").map((crumb) => crumb.title)).not.toContain("Trash");
});
