// Saving stars and bookmarks, and switching tabs, while other answers land
// (#172): what the window shows and what starred.json and pins.json hold end
// at the latest state, whichever answer lands first. The mock holds a
// command's calls until the test releases them, in the order they were made,
// or the latest one first (`releaseLast`).
import { afterEach, beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, chooseMenu, click, closeOverlays, goTo, hit, item, key, mouse, settle, title, toasts, type } from "./app";

const ROOT = "/Users/me/Saves";

type MockCalls = {
  hold(cmd: string): void;
  release(cmd: string): void;
  releaseLast(cmd: string): void;
  failNext(cmd: string, error: string): void;
  reset(): void;
};
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
const DENIED = "Failed to read directory: Permission denied (os error 13)";

beforeAll(async () => {
  for (const dir of ["", "/Stars", "/Marks", "/Marks/Old", "/Marks/Book", "/Marks/One", "/Marks/Two", "/Marks/Book 2", "/Tabs", "/Tabs/H", "/Tabs/T", "/Kept", "/Kept/K", "/Kept/T", "/Back", "/Back/G"]) {
    await invoke("make_dir", { path: ROOT + dir });
  }
  for (const file of ["/Stars/a.txt", "/Stars/b.txt"]) await invoke("create_file", { path: ROOT + file });
  await boot();
  await key("1", { mod: true }); // list view: rows have a star button
});

afterEach(async () => {
  calls().reset();
  await settle(20);
  await closeOverlays();
  // One tab, even after a failed test.
  for (let i = 0; i < 3 && $$(".tab").length > 1; i++) await key("w", { mod: true });
  await settle(20);
});

/** F2 on `name`, type `to`, Enter. */
async function rename(name: string, to: string): Promise<void> {
  await click(hit(name));
  await key("F2");
  const input = $<HTMLInputElement>(".rename-popover input");
  await type(input, to);
  await key("Enter", { target: input });
  await settle(20);
}

async function back(): Promise<string> {
  await key("ArrowLeft", { alt: true });
  await settle(20);
  return title();
}

// ---- Stars ----------------------------------------------------------------------

const savedStars = () => invoke<string[]>("load_starred");

async function star(name: string): Promise<void> {
  await mouse(item(name).querySelector(".star")!, "mousedown");
  await settle(10);
}

test("two stars saved close together: starred.json ends with both, whichever save lands first", async () => {
  await goTo(`${ROOT}/Stars`);
  calls().hold("save_starred");
  await star("a.txt");
  await star("b.txt");
  calls().releaseLast("save_starred");
  await settle(20);
  calls().release("save_starred");
  await settle(20);

  expect(await savedStars()).toEqual(expect.arrayContaining([`${ROOT}/Stars/a.txt`, `${ROOT}/Stars/b.txt`]));
});

// ---- Bookmarks ------------------------------------------------------------------

const bookmark = (label: string) =>
  $$("#sidebar-bookmarks .sidebar-row.pin").find((row) => row.textContent?.trim() === label);
const savedPins = () => invoke<string[]>("load_pins");

async function addBookmark(name: string): Promise<void> {
  await click(hit(name));
  await mouse(hit(name), "mousedown", { button: 2 });
  await mouse(hit(name), "contextmenu", { button: 2 });
  await chooseMenu("Add to Bookmarks");
}

test("two bookmarks added close together are both kept, whichever save lands first", async () => {
  await goTo(`${ROOT}/Marks`);
  calls().hold("save_pins");
  await addBookmark("One");
  await addBookmark("Two");
  calls().releaseLast("save_pins");
  await settle(20);
  calls().release("save_pins");
  await settle(20);

  expect(bookmark("One")?.dataset.location).toBe(`${ROOT}/Marks/One`);
  expect(bookmark("Two")?.dataset.location).toBe(`${ROOT}/Marks/Two`);
  expect(await savedPins()).toEqual(expect.arrayContaining([`${ROOT}/Marks/One`, `${ROOT}/Marks/Two`]));
});

test("a bookmark added while a bookmarked folder is renamed: both are kept, shown and saved", async () => {
  await goTo(`${ROOT}/Marks/Old`);
  await key("d", { mod: true });
  await goTo(`${ROOT}/Marks`);
  calls().hold("save_pins");
  await addBookmark("Book"); // its save waits
  await rename("Old", "Old 2"); // the bookmark follows meanwhile
  calls().release("save_pins");
  await settle(20);

  expect(bookmark("Old")).toBeUndefined();
  expect(bookmark("Old 2")?.dataset.location).toBe(`${ROOT}/Marks/Old 2`);
  expect(bookmark("Book")?.dataset.location).toBe(`${ROOT}/Marks/Book`);
  const pins = await savedPins();
  expect(pins).toEqual(expect.arrayContaining([`${ROOT}/Marks/Old 2`, `${ROOT}/Marks/Book`]));
  expect(pins).not.toContain(`${ROOT}/Marks/Old`);
});

test("a bookmark that couldn't be saved is taken back; so is a removal, in its place", async () => {
  await goTo(`${ROOT}/Marks`);
  const before = await savedPins();
  calls().failNext("save_pins", "Permission denied (os error 13)");
  await addBookmark("Book 2");
  expect(bookmark("Book 2")).toBeUndefined();
  expect(toasts().join(" ")).toContain("Could not save bookmarks");
  expect(await savedPins()).toEqual(before);

  // The first bookmark comes back first, not at the end.
  const rows = () => $$("#sidebar-bookmarks .sidebar-row.pin");
  const shown = rows().map((row) => row.dataset.location);
  expect(shown.length).toBeGreaterThan(1);
  calls().failNext("save_pins", "Permission denied (os error 13)");
  await mouse(rows()[0], "mousedown", { button: 2 });
  await mouse(rows()[0], "contextmenu", { button: 2 });
  await chooseMenu("Remove from Bookmarks");
  expect(rows().map((row) => row.dataset.location)).toEqual(shown);
  expect(await savedPins()).toEqual(before);
});

// ---- Tabs: a move that lands while a tab is switched to -------------------------

test("a tab switched to while a folder in its Back is renamed goes Back to the new name", async () => {
  // The first tab came from H to T; the second shows Tabs, where H is.
  await goTo(`${ROOT}/Tabs/H`);
  await goTo(`${ROOT}/Tabs/T`);
  await key("t", { mod: true });
  await settle(10);
  await goTo(`${ROOT}/Tabs`);

  calls().hold("list_dir");
  await click($$(".tab")[0]); // waits on T's listing
  await rename("H", "H 2");
  calls().release("list_dir");
  await settle(20);
  expect($$(".tab")[0].classList.contains("active")).toBe(true);
  expect(title()).toBe("T");
  expect(await back()).toBe("H 2");
});

test("a tab whose switch failed while a folder in its Back was renamed goes Back to the new name later", async () => {
  await goTo(`${ROOT}/Kept/K`);
  await goTo(`${ROOT}/Kept/T`);
  await key("t", { mod: true });
  await settle(10);
  await goTo(`${ROOT}/Kept`);

  calls().hold("list_dir");
  calls().failNext("list_dir", DENIED);
  await click($$(".tab")[0]); // T can't be listed this time
  await rename("K", "K 2");
  calls().release("list_dir");
  await settle(20);
  expect($$(".tab")[1].classList.contains("active")).toBe(true);
  expect(title()).toBe("Kept");

  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("T");
  expect(await back()).toBe("K 2");
});

test("the tab a failed switch comes back to goes Back to a folder renamed meanwhile", async () => {
  // The second tab came from G to Back.
  await goTo(`${ROOT}/Back`);
  await key("t", { mod: true });
  await settle(10);
  await goTo(`${ROOT}/Back/G`);
  await goTo(`${ROOT}/Back`);

  calls().hold("list_dir");
  calls().failNext("list_dir", DENIED);
  await click($$(".tab")[0]); // the first tab's folder can't be listed this time
  calls().release("list_dir");
  calls().hold("list_dir"); // the second tab comes back, waiting on its listing
  await settle(20);
  await rename("G", "G 2");
  calls().release("list_dir");
  await settle(20);
  expect($$(".tab")[1].classList.contains("active")).toBe(true);
  expect(title()).toBe("Back");
  expect(await back()).toBe("G 2");
});
