// New Window: menu and keyboard both ask the backend for another window.
import { beforeAll, expect, test } from "vitest";
import { $, boot, chooseMenu, click, settle } from "./app";

const windows = () => (globalThis as unknown as { __mockNewWindows?: number }).__mockNewWindows ?? 0;

beforeAll(async () => {
  await boot();
  await settle(10);
  (globalThis as unknown as { __mockNewWindows?: number }).__mockNewWindows = 0;
});

test("New Window in the main menu", async () => {
  await click($("#btn-main-menu"));
  await chooseMenu("New Window");
  await settle(10);
  expect(windows()).toBe(1);
});

test("⌘N opens another window", async () => {
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "n", metaKey: true, bubbles: true }));
  await settle(10);
  expect(windows()).toBe(2);
});
