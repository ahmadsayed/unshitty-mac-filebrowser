// Shared setup and drivers for search-lifecycle tests: the folder tree, the
// 160 ms pause, and the held walk the mock answers by hand.
import { invoke, nativeIconMock, searchMock } from "../../dev/mock-tauri/core";
import { $, $$, boot, goTo, key, settle, type } from "../app";

export const ROOT = "/Users/me/Lifecycle";
export const pause = (ms = 260) => new Promise((resolve) => setTimeout(resolve, ms));
export const headers = () => $$("#view .section-header").map((h) => h.querySelector(".label")?.textContent ?? "");
export const entry = () => $<HTMLInputElement>("#search-entry");

export async function setupSearch(): Promise<void> {
  for (const dir of ["", "/Work", "/Work/2023", "/Archive"]) await invoke("make_dir", { path: ROOT + dir });
  for (const file of ["/notes.txt", "/a-notes.md", "/Work/notes-old.txt", "/Work/2023/notes-2023.txt", "/Archive/b-notes.zip"]) {
    await invoke("create_file", { path: ROOT + file });
  }
  await boot();
  await key("1", { mod: true }); // list view: names() is the display order
}

export async function cleanSearch(): Promise<void> {
  nativeIconMock.enabled = false;
  nativeIconMock.delay = 0;
  searchMock.hold = false;
  for (const held of searchMock.held.splice(0)) held.answer();
  if (!$("#searchbar").hidden) await key("Escape", { target: entry() });
  await settle(10);
}

/** Go to `at`, open the search, type `query` and wait past the pause, so
 *  the walk has started (and, while the mock holds, waits in `held`). */
export async function startSearch(at: string, query: string): Promise<void> {
  await goTo(at);
  await key("f", { mod: true });
  await type(entry(), query);
  await pause();
  await settle(10);
}
