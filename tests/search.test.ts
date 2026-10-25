// Recursive search: results from subfolders, the Location column, and
// opening a found folder. Each file boots a fresh app.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, goTo, key, names, settle, title, type } from "./app";

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
