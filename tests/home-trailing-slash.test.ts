// Home written with a trailing slash (HOME=/Users/me/): Tauri's homeDir keeps
// $HOME as written, and the app trims the slash once at start-up, so Home
// keeps its crumbs, the Location column and "~" in the path bar (#170).
import { beforeAll, expect, test, vi } from "vitest";
import { $, $$, boot, goTo, key, names, settle, type } from "./app";

vi.mock("@tauri-apps/api/path", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/path")>()),
  homeDir: async () => "/Users/me/",
}));

const pause = (ms = 260) => new Promise((resolve) => setTimeout(resolve, ms));

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

/** Each crumb as [title, path]: the title is its label, or "Computer" for "/". */
function crumbs(): [string, string][] {
  return $$("#crumbs .crumb").map((c) => [c.title, c.dataset.path ?? ""]);
}

test("the window opens at Home, without the slash", () => {
  expect(crumbs()).toEqual([["Home", "/Users/me"]]);
  const row = $$(".sidebar-row").find((r) => r.textContent === "Home");
  expect(row?.dataset.location).toBe("/Users/me");
});

test("Home's subfolders start at Home, not at the computer's root", async () => {
  await goTo("/Users/me/Documents");
  expect(crumbs()).toEqual([
    ["Home", "/Users/me"],
    ["Documents", "/Users/me/Documents"],
  ]);
});

test("\"~\" in the path bar is Home", async () => {
  await goTo("~/Documents");
  await key("l", { mod: true });
  expect($<HTMLInputElement>("#path-entry").value).toBe("/Users/me/Documents");
  await key("Escape", { target: $("#path-entry") });
});

test("the Location column shows a search result's folder from Home", async () => {
  await goTo("/Users/me");
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "README");
  await pause();
  await settle(10);
  expect(names()).toContain("README.md");
  const where = $$(".where").map((cell) => cell.textContent ?? "");
  expect(where).toContain("Documents/Projects/unshitty-mac-filebrowser");
});
