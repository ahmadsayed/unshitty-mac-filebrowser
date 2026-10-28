// Recursive search: results from subfolders, the Location column, and
// opening a found folder. Each file boots a fresh app.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, goTo, item, key, names, settle, title, type } from "./app";

const HOME = "/Users/me";
const pause = (ms = 260) => new Promise((resolve) => setTimeout(resolve, ms));

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

test("search finds matches in subfolders and shows where they are", async () => {
  await goTo(HOME);
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "README");
  await pause();
  await settle(10);

  expect(names()).toContain("README.md");
  expect($("#view").textContent).toContain("Location");
  const where = $$(".where").map((cell) => cell.textContent);
  expect(where.some((text) => text?.includes("unshitty-mac-filebrowser"))).toBe(true);
});

test("search can find and open a folder", async () => {
  await key("Escape", { target: $("#search-entry") });
  await settle(10);
  await goTo(HOME);
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "Projects");
  await pause();
  await settle(10);
  expect(names()).toContain("Projects");

  await key("Enter", { target: $("#search-entry") });
  await settle(20);
  expect(title()).toBe("Projects");
});

test("search says it is still searching until results arrive", async () => {
  await key("Escape", { target: $("#search-entry") });
  await settle(10);
  await goTo(HOME);
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "README");
  // Nothing in Home itself matches, and the recursive search hasn't answered.
  expect($("#view").textContent).toContain("Searching…");
  expect($("#searchbar").classList.contains("busy")).toBe(true);
  await pause();
  await settle(10);
  expect(names()).toContain("README.md");
  expect($("#searchbar").classList.contains("busy")).toBe(false);
});

test("search ignores accents like the quick filter does", async () => {
  await key("Escape", { target: $("#search-entry") });
  await settle(10);
  await invoke("make_dir", { path: `${HOME}/Documents/Café Notes` });
  await goTo(HOME);
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "cafe");
  await pause();
  await settle(10);
  expect(names()).toContain("Café Notes");
});

test("search says when it stopped at the result limit", async () => {
  await key("Escape", { target: $("#search-entry") });
  await settle(10);
  await goTo("/big");
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "file");
  await pause();
  await settle(10);
  expect($("#search-banner").hidden).toBe(false);
  expect($("#search-banner").textContent).toContain("Showing the first 1,000 results");

  await type($<HTMLInputElement>("#search-entry"), "folder 1");
  expect($("#search-banner").hidden).toBe(true);
  await pause();
  await settle(10);
  expect(names()).toContain("folder 1");
  expect($("#search-banner").hidden).toBe(true);

  await key("Escape", { target: $("#search-entry") });
  await settle(10);
  expect($("#search-banner").hidden).toBe(true);
});

test("grid view shows the folder each result is in", async () => {
  await goTo(HOME);
  await key("2", { mod: true });
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "README");
  await pause();
  await settle(10);
  const tile = item("README.md");
  expect(tile.querySelector(".where")?.textContent).toContain("unshitty-mac-filebrowser");
  await key("Escape", { target: $("#search-entry") });
  await settle(10);
  expect($$(".grid-item .where")).toHaveLength(0);
  await key("1", { mod: true });
});
