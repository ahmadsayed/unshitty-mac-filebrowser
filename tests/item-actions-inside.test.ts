// A shared action asks its rule (src/item-actions.ts) inside the action
// function, so every surface that reaches it is refused alike. Here the rules
// for Properties and Add to Bookmarks refuse everywhere, with words, and each
// menu item and shortcut must say so and do nothing. No place refuses them
// with words today, so only a stand-in rule can show the split.
import { beforeAll, beforeEach, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, boot, chooseMenu, click, closeOverlays, goTo, hit, key, mouse, settle, toasts } from "./app";

const refused = vi.hoisted(() => new Set<string>());
const WORDS = "Refused by the test.";

vi.mock("../src/item-actions", async (importOriginal) => {
  const real = await importOriginal<typeof import("../src/item-actions")>();
  const refusal: typeof real.refusal = (action, here) => (refused.has(action) ? WORDS : real.refusal(action, here));
  return { ...real, refusal, allows: (action: Parameters<typeof refusal>[0], here: Parameters<typeof refusal>[1]) => refusal(action, here) === null };
});

const HERE = "/Users/me/Inside";

const calls = () =>
  (globalThis as unknown as { __mockCalls: { count(cmd: string): number; reset(): void } }).__mockCalls;

beforeAll(async () => {
  await invoke("make_dir", { path: HERE });
  await invoke("make_dir", { path: `${HERE}/Sub` });
  await invoke("create_file", { path: `${HERE}/note.txt` });
  await boot();
  await goTo(HERE);
});

beforeEach(async () => {
  await closeOverlays();
  refused.clear();
  calls().reset();
  document.querySelectorAll(".toast").forEach((t) => t.remove());
});

async function itemMenu(name: string): Promise<void> {
  await click(hit(name));
  await mouse(hit(name), "mousedown", { button: 2 });
  await mouse(hit(name), "contextmenu", { button: 2 });
}

/** What happened: the backend calls `cmd` made, and whether the refusal was shown. */
async function outcome(cmd: string): Promise<{ calls: number; told: boolean }> {
  await settle(10);
  return { calls: calls().count(cmd), told: toasts().some((t) => t.includes(WORDS)) };
}

const REFUSED = { calls: 0, told: true };

test("Properties: the item menu and ⌘I both ask the rule", async () => {
  refused.add("properties");
  await itemMenu("note.txt");
  await chooseMenu("Properties");
  expect(await outcome("file_properties")).toEqual(REFUSED);

  await closeOverlays();
  document.querySelectorAll(".toast").forEach((t) => t.remove());
  await click(hit("note.txt"));
  await key("i", { mod: true });
  expect(await outcome("file_properties")).toEqual(REFUSED);
});

test("Add to Bookmarks: the item menu, the location menu and ⌘D all ask the rule", async () => {
  refused.add("bookmark");
  await itemMenu("Sub");
  await chooseMenu("Add to Bookmarks");
  expect(await outcome("save_pins")).toEqual(REFUSED);

  await closeOverlays();
  document.querySelectorAll(".toast").forEach((t) => t.remove());
  await click($("#btn-location-menu"));
  await chooseMenu("Add to Bookmarks");
  expect(await outcome("save_pins")).toEqual(REFUSED);

  await closeOverlays();
  document.querySelectorAll(".toast").forEach((t) => t.remove());
  await key("d", { mod: true });
  expect(await outcome("save_pins")).toEqual(REFUSED);
});

test("a bookmark can always be removed", async () => {
  await key("d", { mod: true });
  expect(await outcome("save_pins")).toEqual({ calls: 1, told: false });

  refused.add("bookmark");
  calls().reset();
  await click($("#btn-location-menu"));
  await chooseMenu("Remove from Bookmarks");
  expect(await outcome("save_pins")).toEqual({ calls: 1, told: false });
});
