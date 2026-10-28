// Sorting the listing: a presentation-only change (grid ↔ list, thumbnails)
// reuses the sorted items, and anything the order depends on sorts again.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, boot, chooseMenu, click, goTo, names, prefs, settle } from "./app";

const HOME = "/Users/me";

beforeAll(async () => {
  await boot();
});

afterEach(() => {
  vi.restoreAllMocks();
});

/** Count the sorts of the listing (by the app's comparator) from now on. */
function countListingSorts(): () => number {
  const sort = vi.spyOn(Array.prototype, "sort");
  return () => sort.mock.calls.filter(([compare]) => compare?.name === "compareEntries").length;
}

async function showAs(view: "grid" | "list"): Promise<void> {
  if (prefs().view !== view) await click($("#btn-view-toggle"));
}

async function sortBy(label: string): Promise<void> {
  await click($("#btn-view-menu"));
  await chooseMenu(label);
  await settle(15);
}

test("switching between grid and list keeps the order without sorting again", async () => {
  await goTo(HOME);
  const order = names();
  const sorts = countListingSorts();
  await click($("#btn-view-toggle"));
  await click($("#btn-view-toggle"));
  expect(sorts()).toBe(0);
  expect(names()).toEqual(order);
});

test("changing the sort order sorts again", async () => {
  await goTo(HOME);
  await sortBy("A–Z");
  const order = names();
  await click($("#btn-view-toggle"));
  await sortBy("Z–A");
  expect(names()).not.toEqual(order);
  await sortBy("A–Z");
  expect(names()).toEqual(order);
  await click($("#btn-view-toggle"));
});

test("sorting by size orders folders by their item counts once they arrive", async () => {
  // Opening a folder sorts it before its folders are counted; the counts
  // arriving must sort it again. Only the list view counts every folder.
  await showAs("list");
  await goTo("/");
  await sortBy("Size");
  await goTo(HOME);
  const listing = (await invoke("list_dir", { path: HOME, showHidden: false })) as { name: string; path: string; is_dir: boolean }[];
  const folders = listing.filter((e) => e.is_dir);
  const summaries = (await invoke("item_counts", { paths: folders.map((f) => f.path), showHidden: false })) as ({ count: number } | null)[];
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
  const expected = folders
    .map((f, i) => ({ name: f.name, count: summaries[i]?.count ?? -1 }))
    .sort((a, b) => b.count - a.count || collator.compare(a.name, b.name))
    .map((f) => f.name);
  expect(names().slice(0, expected.length)).toEqual(expected);
  await sortBy("A–Z");
  await showAs("grid");
});
