// Folder item counts in the app: only the rows on screen ask for them,
// except when sorting by size, and the selected folder's count reaches the
// floating bar. The rules themselves are tested in folder-summaries.test.ts.
import { afterEach, beforeAll, expect, test } from "vitest";
import { $, boot, chooseMenu, click, goTo, item, prefs, settle } from "./app";

const HOME = "/Users/me";
type MockCounts = { log: { asked: string[][] }; hold(hold: boolean): void; held(): number; reset(): void };
const counts = () => (globalThis as unknown as { __mockCounts: MockCounts }).__mockCounts;
const asked = () => counts().log.asked.flat();

async function showAs(view: "grid" | "list"): Promise<void> {
  if (prefs().view !== view) await click($("#btn-view-toggle"));
}

async function sortBy(label: string): Promise<void> {
  await click($("#btn-view-menu"));
  await chooseMenu(label);
  await settle(15);
}

beforeAll(async () => {
  await boot();
});

afterEach(() => {
  counts().reset();
});

test("only the folders on screen are counted", async () => {
  await showAs("list");
  counts().reset();
  await goTo("/big");
  await settle(20);
  const folders = new Set(asked());
  expect(folders.size).toBeGreaterThan(0);
  expect(folders.size).toBeLessThan(200);
  expect([...folders].every((p) => p.startsWith("/big/folder "))).toBe(true);
});

test("sorting by size counts every folder, in batches of 400", async () => {
  await goTo(HOME);
  counts().reset();
  await sortBy("Size");
  await goTo("/big");
  await settle(40);
  expect(new Set(asked()).size).toBe(1000);
  expect(Math.max(...counts().log.asked.map((batch) => batch.length))).toBeLessThanOrEqual(400);
  await sortBy("A–Z");
  await showAs("grid");
});

test("the floating bar shows the selected folder's count", async () => {
  await goTo(HOME);
  await click(item("Desktop").querySelector(".icon")!);
  await settle(10);
  expect($("#floating-bar").textContent).toBe("“Desktop” selected (containing 1 item)");
});
