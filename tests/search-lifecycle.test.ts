// A search's lifecycle: answers that arrive after the search changed are
// dropped, closing stops the walk, and a reload runs the search again. The
// mock holds searches (`searchMock`) so a test can answer them late.
import { afterEach, beforeAll, expect, test } from "vitest";
import { invoke, searchMock } from "../dev/mock-tauri/core";
import { $, $$, boot, click, goTo, key, names, settle, toasts, type } from "./app";

const ROOT = "/Users/me/Lifecycle";
const pause = (ms = 260) => new Promise((resolve) => setTimeout(resolve, ms));
const headers = () => $$("#view .section-header").map((h) => h.querySelector(".label")?.textContent ?? "");
const entry = () => $<HTMLInputElement>("#search-entry");

beforeAll(async () => {
  for (const dir of ["", "/Work", "/Work/2023", "/Archive"]) await invoke("make_dir", { path: ROOT + dir });
  for (const file of ["/notes.txt", "/a-notes.md", "/Work/notes-old.txt", "/Work/2023/notes-2023.txt", "/Archive/b-notes.zip"]) {
    await invoke("create_file", { path: ROOT + file });
  }
  await boot();
  await key("1", { mod: true }); // list view: names() is the display order
});

afterEach(async () => {
  searchMock.hold = false;
  for (const held of searchMock.held.splice(0)) held.answer();
  if (!$("#searchbar").hidden) await key("Escape", { target: entry() });
  await settle(10);
});

/** Go to `at`, open the search, type `query` and wait past the pause, so
 *  the walk has started (and, while the mock holds, waits in `held`). */
async function startSearch(at: string, query: string): Promise<void> {
  await goTo(at);
  await key("f", { mod: true });
  await type(entry(), query);
  await pause();
  await settle(10);
}

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

test("navigating away closes the search; its answer then shows nothing", async () => {
  searchMock.hold = true;
  await startSearch(ROOT, "notes");
  const [left] = searchMock.held.splice(0);
  await goTo(`${ROOT}/Archive`);
  expect($("#searchbar").hidden).toBe(true);
  left.answer({ stopped: "results" });
  await settle(10);
  expect(names()).toEqual(["b-notes.zip"]);
  expect(headers()).toEqual([]);
  expect($("#search-banner").hidden).toBe(true);
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

test("a failed search says so and shows what this folder has", async () => {
  searchMock.hold = true;
  await startSearch(ROOT, "notes");
  searchMock.held.splice(0)[0].fail("Permission denied");
  await settle(10);
  expect(toasts().some((t) => t.includes("Could not search: Permission denied"))).toBe(true);
  expect(headers()).toEqual(["In This Folder"]);
  expect(names()).toEqual(["a-notes.md", "notes.txt"]);
  expect($("#searchbar").classList.contains("busy")).toBe(false);
});

test("a reload runs the search again and keeps its text", async () => {
  await startSearch(ROOT, "notes");
  expect(names()).toEqual(["a-notes.md", "notes.txt", "b-notes.zip", "notes-old.txt", "notes-2023.txt"]);
  await invoke("create_file", { path: `${ROOT}/Archive/notes-new.txt` });
  await key("F5");
  await pause();
  await settle(10);
  expect(entry().value).toBe("notes");
  expect(names()).toEqual(["a-notes.md", "notes.txt", "b-notes.zip", "notes-new.txt", "notes-old.txt", "notes-2023.txt"]);
});

test("places that filter in place never walk, and filter what they list", async () => {
  searchMock.hold = true;
  await click($$(".sidebar-row").find((row) => row.dataset.location === "pseudo:recent")!);
  await settle(15);
  const all = names();
  await key("f", { mod: true });
  await type(entry(), "a");
  await pause();
  await settle(10);
  expect(searchMock.held).toHaveLength(0);
  expect($("#searchbar").classList.contains("busy")).toBe(false);
  expect(names()).toEqual(all.filter((name) => name.toLowerCase().includes("a")));
});
