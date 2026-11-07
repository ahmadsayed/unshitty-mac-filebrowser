// Tabs: independent locations, switching, closing and new tabs from the
// keyboard. Each test file boots a fresh app.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, cursorName, goTo, key, names, selectedNames, settle, title, toasts, type } from "./app";

const HOME = "/Users/me";

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

test("⌘T opens a second tab at the current folder", async () => {
  await goTo(HOME);
  expect($("#tabbar").hidden).toBe(true);

  await key("t", { mod: true });
  await settle(10);
  expect($("#tabbar").hidden).toBe(false);
  expect($$(".tab")).toHaveLength(2);
  expect($$(".tab")[1].classList.contains("active")).toBe(true);
  expect(title()).toBe("Home");
  expect(names()).toContain("Documents");
});

test("tabs keep their own location", async () => {
  await goTo(`${HOME}/Documents`);
  expect(title()).toBe("Documents");

  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("Home");
  expect(names()).toContain("Documents");

  await click($$(".tab")[1]);
  await settle(20);
  expect(title()).toBe("Documents");
  expect(names()).toContain("Quarterly report.pdf");

  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("Home");
});

test("the active tab's close button removes it", async () => {
  await click($$(".tab.active .tab-close")[0]);
  await settle(20);
  expect($$(".tab")).toHaveLength(0);
  expect($("#tabbar").hidden).toBe(true);
  expect(title()).toBe("Documents");
  expect(names()).toContain("Quarterly report.pdf");
});

test("⌘W closes the active tab and Ctrl+Page Down cycles", async () => {
  await key("t", { mod: true });
  await settle(10);
  await goTo(`${HOME}/Pictures`);
  await key("t", { mod: true });
  await settle(10);
  await goTo(`${HOME}/Music`);
  expect($$(".tab")).toHaveLength(3);

  await key("PageUp", { ctrl: true });
  await settle(20);
  expect(title()).toBe("Pictures");

  await key("PageUp", { ctrl: true });
  await settle(20);
  expect(title()).toBe("Documents");

  await key("PageDown", { ctrl: true });
  await settle(20);
  expect(title()).toBe("Pictures");

  await key("w", { mod: true });
  await settle(20);
  expect($$(".tab")).toHaveLength(2);
  expect(title()).toBe("Music");
});

test("switching back to a tab brings back its selection, cursor and anchor", async () => {
  await goTo(HOME);
  const all = names();
  await key("ArrowDown"); // the first item
  await key("ArrowDown"); // the second: cursor and anchor
  await key("ArrowDown", { shift: true }); // second and third, cursor on the third
  expect(selectedNames()).toEqual(all.slice(1, 3));
  expect(cursorName()).toBe(all[2]);

  const here = $$(".tab").findIndex((tab) => tab.classList.contains("active"));
  await click($$(".tab")[1 - here]);
  await settle(20);
  expect(title()).not.toBe("Home");
  await click($$(".tab")[here]);
  await settle(20);
  expect(title()).toBe("Home");
  expect(selectedNames()).toEqual(all.slice(1, 3));
  expect(cursorName()).toBe(all[2]);
  // The anchor came back too: Shift extends the range from it.
  await key("ArrowDown", { shift: true });
  expect(selectedNames()).toEqual(all.slice(1, 4));
});

// ---- A tab that was searching ----------------------------------------------

const SECTIONS = `${HOME}/TabSearch`;
const pause = (ms = 260) => new Promise((resolve) => setTimeout(resolve, ms));
const headers = () => $$("#view .section-header").map((h) => h.querySelector(".label")?.textContent ?? "");
const activeIndex = () => $$(".tab").findIndex((tab) => tab.classList.contains("active"));

test("a tab that was searching comes back with its subfolder results and its placeholder", async () => {
  for (const dir of ["", "/Work"]) await invoke("make_dir", { path: SECTIONS + dir });
  for (const file of ["/notes.txt", "/Work/notes-old.txt"]) await invoke("create_file", { path: SECTIONS + file });

  await goTo(SECTIONS);
  const here = activeIndex();
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "notes");
  await pause();
  await settle(10);
  expect(headers()).toEqual(["In This Folder", "In Subfolders"]);
  expect(names()).toEqual(["notes.txt", "notes-old.txt"]);

  // The other tab searches its own folder, which sets another placeholder.
  await click($$(".tab")[1 - here]);
  await settle(20);
  await key("f", { mod: true });
  await settle(10);
  const otherPlaceholder = $<HTMLInputElement>("#search-entry").placeholder;
  expect(otherPlaceholder).not.toBe("Search TabSearch");

  await click($$(".tab")[here]);
  await settle(20);
  await pause();
  await settle(10);
  expect(title()).toBe("TabSearch");
  expect($("#searchbar").hidden).toBe(false);
  expect($<HTMLInputElement>("#search-entry").value).toBe("notes");
  expect(headers()).toEqual(["In This Folder", "In Subfolders"]);
  expect(names()).toEqual(["notes.txt", "notes-old.txt"]);
  expect($<HTMLInputElement>("#search-entry").placeholder).toBe("Search TabSearch");
});

test("a tab whose folder is gone stays behind, and keeps its own place for a retry", async () => {
  const GONE = `${HOME}/TabGone`;
  await invoke("make_dir", { path: GONE });
  await goTo(HOME);
  const here = activeIndex();
  await click($$(".tab")[1 - here]);
  await settle(20);
  await goTo(GONE);
  expect(title()).toBe("TabGone");
  await click($$(".tab")[here]);
  await settle(20);

  await invoke("trash_entries", { paths: [GONE] });
  await click($$(".tab")[1 - here]);
  await settle(20);
  expect(toasts().some((t) => t.includes("Unable to access “TabGone”"))).toBe(true);
  // The view didn't change, so neither did the active tab.
  expect(title()).toBe("Home");
  expect(activeIndex()).toBe(here);
  // Clicking this tab again saves its own view, not over the other tab's.
  await click($$(".tab")[here]);
  await settle(20);

  // Once the folder is back, the other tab opens there again.
  await invoke("make_dir", { path: GONE });
  await click($$(".tab")[1 - here]);
  await settle(20);
  expect(title()).toBe("TabGone");
  await click($$(".tab")[here]);
  await settle(20);
  expect(title()).toBe("Home");
});

test("a search stays open when the other tab's folder can't be opened", async () => {
  const GONE = `${HOME}/TabGone`;
  const here = activeIndex();
  await goTo(SECTIONS);
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "notes");
  await pause();
  await settle(10);
  await invoke("trash_entries", { paths: [GONE] });
  await click($$(".tab")[1 - here]);
  await settle(20);
  await pause();
  await settle(10);
  expect(activeIndex()).toBe(here);
  expect($("#searchbar").hidden).toBe(false);
  expect($<HTMLInputElement>("#search-entry").value).toBe("notes");
  expect(headers()).toEqual(["In This Folder", "In Subfolders"]);
  expect(names()).toEqual(["notes.txt", "notes-old.txt"]);
});
