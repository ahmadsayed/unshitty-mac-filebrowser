// Home at "/" (HOME=/ for system and container accounts): the path bar and
// the Location column treat "/" as written, so it contains only itself and
// other folders keep their disk and volume crumbs. Characterization tests:
// they pin what the app did before the path rules moved into src/paths.ts.
import { beforeAll, expect, test, vi } from "vitest";
import { $, $$, boot, goTo, key, names, settle, type } from "./app";

vi.mock("@tauri-apps/api/path", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/path")>()),
  homeDir: async () => "/",
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

test("\"/\" itself is Home", async () => {
  await goTo("/");
  expect(crumbs()).toEqual([["Home", "/"]]);
});

test("a folder below \"/\" starts at the computer's root, not at Home", async () => {
  await goTo("/Users/me/Documents");
  expect(crumbs()).toEqual([
    ["Computer", "/"],
    ["Users", "/Users"],
    ["me", "/Users/me"],
    ["Documents", "/Users/me/Documents"],
  ]);
});

test("a mounted volume keeps its own crumb", async () => {
  await goTo("/Volumes/Backup Drive");
  expect(crumbs()).toEqual([["Backup Drive", "/Volumes/Backup Drive"]]);
});

test("the Location column shows a search result's whole folder", async () => {
  await goTo("/Users/me");
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), "README");
  await pause();
  await settle(10);
  expect(names()).toContain("README.md");
  const where = $$(".where").map((cell) => cell.textContent ?? "");
  expect(where.length).toBeGreaterThan(0);
  for (const text of where) expect(text.startsWith("/Users/me/")).toBe(true);
});
