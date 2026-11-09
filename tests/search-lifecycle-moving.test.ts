// The search follows the view: a navigation, a failed walk, a reload and the
// places that filter in place. Shared setup and drivers live in
// tests/support/search-lifecycle.ts.
import { afterEach, beforeAll, expect, test } from "vitest";
import { invoke, nativeIconMock, searchMock } from "../dev/mock-tauri/core";
import { $, $$, click, goTo, hit, key, names, settle, title, toasts, type } from "./app";
import { ROOT, cleanSearch, entry, headers, pause, setupSearch, startSearch } from "./support/search-lifecycle";

beforeAll(setupSearch);
afterEach(cleanSearch);

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

// A key typed while a navigation still waits for its listing or icons opens
// the search before the view has moved: the search then goes with the view.
const press = (k: string) =>
  $("#view").dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));

test("a search opened while a folder is opening searches that folder", async () => {
  await goTo(ROOT);
  hit("Work").dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
  await Promise.resolve();
  press("n");
  await settle(15);
  expect(title()).toBe("Work");
  expect(entry().placeholder).toBe("Search Work");
  await type(entry(), "notes");
  await pause();
  await settle(10);
  expect(headers()).toEqual(["In This Folder", "In Subfolders"]);
  expect(names()).toEqual(["notes-old.txt", "notes-2023.txt"]);
});

test("a walk that started before the view moved is dropped, and the new folder is walked", async () => {
  // A folder no other test opens, so its icons aren't known yet and the
  // navigation waits for them.
  await invoke("make_dir", { path: `${ROOT}/Late` });
  await invoke("create_file", { path: `${ROOT}/Late/zeta.txt` });
  await goTo(ROOT);
  searchMock.hold = true;
  nativeIconMock.enabled = true;
  nativeIconMock.delay = 1000;
  await key("l", { mod: true });
  await type($<HTMLInputElement>("#path-entry"), `${ROOT}/Late`);
  $("#path-entry").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  await new Promise((resolve) => setTimeout(resolve, 5));
  press("z"); // in the folder being left, "z" finds Archive/b-notes.zip
  await pause(600);
  await settle(10);
  expect(title()).toBe("Late");
  expect(entry().placeholder).toBe("Search Late");
  expect(searchMock.held.at(-1)?.path).toBe(`${ROOT}/Late`);
  for (const held of searchMock.held.splice(0)) held.answer();
  await settle(10);
  expect(names()).toEqual(["zeta.txt"]);
  expect(headers()).toEqual(["In This Folder"]);
  expect($("#searchbar").classList.contains("busy")).toBe(false);
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
