// Search shows "In This Folder" at once, then "In Subfolders", nearest
// folders first (spec 2026-10-31-search-sections, D1–D9).
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, cursorName, goTo, key, names, selectedNames, settle, type } from "./app";

const ROOT = "/Users/me/Sections";
const pause = (ms = 260) => new Promise((resolve) => setTimeout(resolve, ms));

const headers = () => $$("#view .section-header").map((h) => h.querySelector(".label")?.textContent ?? "");
const counts = () => $$("#view .section-header .count").map((c) => c.textContent);

beforeAll(async () => {
  for (const dir of ["", "/Work", "/Work/2023", "/Archive"]) await invoke("make_dir", { path: ROOT + dir });
  for (const file of ["/notes.txt", "/a-notes.md", "/Work/notes-old.txt", "/Work/2023/notes-2023.txt", "/Archive/b-notes.zip"]) {
    await invoke("create_file", { path: ROOT + file });
  }
  await boot();
  await key("1", { mod: true }); // list view: names() is the display order
});

/** Open the search, type `query` and let the recursive search finish. */
async function search(query: string, wait = true): Promise<void> {
  if ($("#searchbar").hidden === false) {
    await key("Escape", { target: $("#search-entry") });
    await settle(10);
  }
  await goTo(ROOT);
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), query);
  if (wait) {
    await pause();
    await settle(10);
  }
}

test("this folder's matches come first, then subfolders nearest first", async () => {
  await search("notes");
  expect(headers()).toEqual(["In This Folder", "In Subfolders"]);
  expect(counts()).toEqual(["2", "3"]);
  // One level down (Archive, Work) before two (Work/2023).
  expect(names()).toEqual(["a-notes.md", "notes.txt", "b-notes.zip", "notes-old.txt", "notes-2023.txt"]);
  expect(selectedNames()).toEqual(["a-notes.md"]);
});

test("the second section says it is searching until the search ends", async () => {
  await search("notes", false);
  expect(headers()).toEqual(["In This Folder", "In Subfolders"]);
  expect($$("#view .section-header")[1].textContent).toContain("Searching…");
  await pause();
  await settle(10);
  expect($("#view").textContent).not.toContain("Searching…");
});

test("results arriving later don't move the cursor or the selection", async () => {
  await search("notes", false);
  await key("ArrowDown", { target: $("#view") });
  expect(cursorName()).toBe("notes.txt");
  await pause();
  await settle(10);
  expect(cursorName()).toBe("notes.txt");
  expect(selectedNames()).toEqual(["notes.txt"]);
});

test("only subfolder matches: one header, and its first match selected", async () => {
  await search("old");
  expect(headers()).toEqual(["In Subfolders"]);
  expect(selectedNames()).toEqual(["notes-old.txt"]);
});

test("only matches here: the subfolder header goes once the search ends", async () => {
  await search("a-notes");
  expect(headers()).toEqual(["In This Folder"]);
  expect(names()).toEqual(["a-notes.md"]);
});

test("nothing anywhere: No Results Found, without headers", async () => {
  await search("zzz-nothing");
  expect(headers()).toEqual([]);
  expect($("#view").textContent).toContain("No Results Found");
});

test("Down from the last item here goes to the first subfolder match", async () => {
  await search("notes");
  $("#view").focus();
  await key("ArrowDown", { target: $("#view") });
  await key("ArrowDown", { target: $("#view") });
  expect(cursorName()).toBe("b-notes.zip");
  await key("ArrowUp", { target: $("#view") });
  expect(cursorName()).toBe("notes.txt");
});

test("sorting sorts within each section; this folder stays first", async () => {
  await search("notes");
  await click($('#view [data-sort="name"]'));
  await settle();
  expect(names()).toEqual(["notes.txt", "a-notes.md", "notes-old.txt", "b-notes.zip", "notes-2023.txt"]);
  await click($('#view [data-sort="name"]'));
  await settle();
  expect(names()).toEqual(["a-notes.md", "notes.txt", "b-notes.zip", "notes-old.txt", "notes-2023.txt"]);
});

test("searching from the root of the disk still splits here from deeper", async () => {
  await key("Escape", { target: $("#search-entry") });
  await settle(10);
  await goTo("/");
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "Users");
  await pause();
  await settle(10);
  expect(headers()[0]).toBe("In This Folder");
  expect(names()[0]).toBe("Users");
});

test("places that filter in place show no sections", async () => {
  await key("Escape", { target: $("#search-entry") });
  await settle(10);
  for (const place of ["pseudo:recent", "/Users/me/.Trash"]) {
    await click($$(".sidebar-row").find((row) => row.dataset.location === place)!);
    await settle(15);
    expect(document.querySelector(".sidebar-row.selected, .sidebar-row.active")?.getAttribute("data-location")).toBe(place);
    await key("f", { mod: true });
    await type($<HTMLInputElement>("#search-entry"), "a");
    await pause();
    await settle(10);
    expect(headers()).toEqual([]);
    await key("Escape", { target: $("#search-entry") });
    await settle(10);
  }
});
