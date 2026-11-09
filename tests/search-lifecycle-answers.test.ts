// Answers that arrive after the search changed are dropped, and closing stops
// the walk. Shared setup and drivers live in tests/support/search-lifecycle.ts.
import { afterEach, beforeAll, expect, test } from "vitest";
import { searchMock } from "../dev/mock-tauri/core";
import { $, goTo, key, names, settle, toasts, type } from "./app";
import { ROOT, cleanSearch, entry, headers, pause, setupSearch, startSearch } from "./support/search-lifecycle";

beforeAll(setupSearch);
afterEach(cleanSearch);

test("an answer that comes after Escape is dropped, even once another search runs", async () => {
  searchMock.hold = true;
  await startSearch(ROOT, "notes");
  expect(searchMock.held.map((h) => h.query)).toEqual(["notes"]);
  const late = searchMock.held.splice(0)[0];

  await key("Escape", { target: entry() });
  await settle(10);
  await key("f", { mod: true });
  await type(entry(), "zzz-nothing");
  await pause();
  await settle(10);
  expect(searchMock.held.map((h) => h.query)).toEqual(["zzz-nothing"]);

  late.answer({ stopped: "results" });
  await settle(10);
  expect(names()).toEqual([]);
  expect($("#view").textContent).toContain("Searching…");
  expect($("#searchbar").classList.contains("busy")).toBe(true);
  expect($("#search-banner").hidden).toBe(true);

  searchMock.held.splice(0)[0].answer();
  await settle(10);
  expect($("#view").textContent).toContain("No Results Found");
  expect($("#searchbar").classList.contains("busy")).toBe(false);
});

test("a late answer or failure after Escape shows nothing", async () => {
  searchMock.hold = true;
  await startSearch(ROOT, "notes");
  const [answered] = searchMock.held.splice(0);
  await key("Escape", { target: entry() });
  await settle(10);
  const listing = names();
  answered.answer({ stopped: "scanned" });
  await settle(10);
  expect(names()).toEqual(listing);
  expect($("#search-banner").hidden).toBe(true);
  expect($("#searchbar").hidden).toBe(true);

  await key("f", { mod: true });
  await type(entry(), "notes");
  await pause();
  await settle(10);
  const [failing] = searchMock.held.splice(0);
  await key("Escape", { target: entry() });
  await settle(10);
  const before = toasts().length;
  failing.fail("gone");
  await settle(10);
  expect(toasts().length).toBe(before);
  expect(names()).toEqual(listing);
});

test("an answer for a folder the view has left is dropped", async () => {
  searchMock.hold = true;
  await startSearch(ROOT, "notes");
  const [left] = searchMock.held.splice(0);
  expect(left.path).toBe(ROOT);

  await startSearch(`${ROOT}/Work`, "notes");
  expect(searchMock.held.map((h) => h.path)).toEqual([`${ROOT}/Work`]);
  left.answer({ stopped: "results" });
  await settle(10);
  expect(names()).toEqual(["notes-old.txt"]);
  expect($("#search-banner").hidden).toBe(true);

  searchMock.held.splice(0)[0].answer();
  await settle(10);
  expect(headers()).toEqual(["In This Folder", "In Subfolders"]);
  expect(names()).toEqual(["notes-old.txt", "notes-2023.txt"]);
});

test("closing a running search cancels its walk", async () => {
  searchMock.hold = true;
  await startSearch(ROOT, "notes");
  expect($("#searchbar").classList.contains("busy")).toBe(true);
  const before = searchMock.cancels;
  await key("Escape", { target: entry() });
  await settle(10);
  expect(searchMock.cancels).toBe(before + 1);
  expect($("#searchbar").classList.contains("busy")).toBe(false);
});

test("closing during the pause before the walk cancels it, and the walk never starts", async () => {
  searchMock.hold = true;
  await goTo(ROOT);
  await key("f", { mod: true });
  await type(entry(), "notes");
  const before = searchMock.cancels;
  await key("Escape", { target: entry() });
  await pause();
  await settle(10);
  expect(searchMock.cancels).toBe(before + 1);
  expect(searchMock.held).toHaveLength(0);
});

test("a search that ended has nothing to cancel when closed", async () => {
  await startSearch(ROOT, "notes");
  expect($("#searchbar").classList.contains("busy")).toBe(false);
  const before = searchMock.cancels;
  await key("Escape", { target: entry() });
  await settle(10);
  expect(searchMock.cancels).toBe(before);
});

test("clearing the text cancels a running walk; a new text doesn't cancel, it starts another", async () => {
  searchMock.hold = true;
  await startSearch(ROOT, "notes");
  const before = searchMock.cancels;
  await type(entry(), "old");
  expect(searchMock.cancels).toBe(before);
  await pause();
  await settle(10);
  expect(searchMock.held.map((h) => h.query)).toEqual(["notes", "old"]);
  await type(entry(), "  ");
  expect(searchMock.cancels).toBe(before + 1);
  expect($("#searchbar").classList.contains("busy")).toBe(false);
  expect(headers()).toEqual([]);
});

test("a stopped search's banner goes when the text changes", async () => {
  searchMock.hold = true;
  await startSearch(ROOT, "notes");
  searchMock.held.splice(0)[0].answer({ stopped: "results" });
  await settle(10);
  expect($("#search-banner").hidden).toBe(false);
  await type(entry(), "notes-old");
  expect($("#search-banner").hidden).toBe(true);
});
