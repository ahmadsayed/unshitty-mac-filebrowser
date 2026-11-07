// Folder item counts in the app: only the rows on screen ask for them,
// except when sorting by size, and the selected folder's count reaches the
// floating bar. The rules themselves are tested in folder-summaries.test.ts.
import { afterEach, beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, chooseMenu, click, goTo, item, key, prefs, settle, type } from "./app";

const HOME = "/Users/me";
type MockCounts = { log: { asked: string[][] }; hold(hold: boolean): void; held(): number; reset(): void };
const counts = () => (globalThis as unknown as { __mockCounts: MockCounts }).__mockCounts;
const asked = () => counts().log.asked.flat();

async function showAs(view: "grid" | "list"): Promise<void> {
  if (prefs().view !== view) await click($("#btn-view-toggle"));
}

/** Show Hidden off, as the other tests expect it. */
async function hideHidden(): Promise<void> {
  if (prefs().showHidden) await key("h", { ctrl: true });
  await settle(20);
}

async function sortBy(label: string): Promise<void> {
  await click($("#btn-view-menu"));
  await chooseMenu(label);
  await settle(15);
}

beforeAll(async () => {
  await boot();
});

afterEach(async () => {
  counts().reset();
  await hideHidden();
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

test("selecting a folder that is already being counted doesn't ask for it again", async () => {
  counts().hold(true);
  await goTo(HOME);
  const desktop = `${HOME}/Desktop`;
  expect(asked()).toContain(desktop);
  await click(item("Desktop").querySelector(".icon")!);
  await settle(10);
  expect($("#floating-bar").textContent).toBe("“Desktop” selected");
  counts().hold(false);
  await settle(10);
  expect(asked().filter((p) => p === desktop)).toEqual([desktop]);
  expect($("#floating-bar").textContent).toBe("“Desktop” selected (containing 1 item)");
});

test("Show Hidden counts folders again with the hidden items in", async () => {
  await showAs("list");
  await goTo("/Users");
  const sizeOf = (name: string) => item(name).querySelector(".size")?.textContent;
  const countOf = async (showHidden: boolean) =>
    ((await invoke("item_counts", { paths: [HOME], showHidden })) as { count: number }[])[0].count;
  const without = await countOf(false);
  const withHidden = await countOf(true);
  expect(withHidden).toBeGreaterThan(without);
  expect(sizeOf("me")).toBe(`${without} items`);

  await key("h", { ctrl: true });
  await settle(20);
  expect(prefs().showHidden).toBe(true);
  expect(sizeOf("me")).toBe(`${withHidden} items`);
  // The floating bar counts with them too.
  await click(item("me").querySelector(".icon")!);
  await settle(10);
  expect($("#floating-bar").textContent).toBe(`“me” selected (containing ${withHidden} items)`);

  await key("h", { ctrl: true });
  await settle(20);
  expect(prefs().showHidden).toBe(false);
  expect(sizeOf("me")).toBe(`${without} items`);
  await showAs("grid");
});

test("a count asked for before Show Hidden changed is dropped when it lands", async () => {
  await showAs("list");
  await goTo(HOME);
  counts().hold(true);
  await goTo("/Users");
  expect(counts().held()).toBe(1);
  await key("h", { ctrl: true });
  await settle(20);
  // Both answers land, the one counted without hidden items first.
  counts().hold(false);
  await settle(20);
  const answer = ((await invoke("item_counts", { paths: [HOME], showHidden: true })) as { count: number }[])[0].count;
  expect(item("me").querySelector(".size")?.textContent).toBe(`${answer} items`);
  await key("h", { ctrl: true });
  await settle(20);
  await showAs("grid");
});

test("selecting a folder on an SSH server doesn't ask for a count", async () => {
  (globalThis as unknown as { __mockSftp: { reset(): void } }).__mockSftp.reset();
  await click($$(".sidebar-row").find((row) => row.textContent?.trim() === "Network")!);
  await settle(20);
  await type($<HTMLInputElement>(".network-link"), "user@host");
  await click($(".network-paste .btn"));
  await settle(30);
  counts().reset();
  await click(item("Documents").querySelector(".icon")!);
  await settle(10);
  expect($("#floating-bar").textContent).toBe("“Documents” selected");
  expect(asked().filter((p) => p.startsWith("sftp://"))).toEqual([]);
  // Last in this file: typed paths now resolve on the server.
});
