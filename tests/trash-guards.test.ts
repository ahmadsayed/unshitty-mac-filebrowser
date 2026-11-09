// Nothing goes into the Trash except by Move to Trash: pasting there would
// leave items with no record of where they came from (Empty Trash then
// deletes them for good), and renaming a trashed item breaks its record.
// Covers the Trash itself and folders inside it, through every surface.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, boot, click, closeOverlays, goTo, item, key, menuLabels, names, settle, rightClick } from "./app";

const HOME = "/Users/me";
const TRASH = `${HOME}/.Trash`;
let pointAt: Element | null = null;

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
  document.elementFromPoint = () => pointAt;
  await invoke("make_dir", { path: `${HOME}/Old` });
  await invoke("make_dir", { path: `${HOME}/Old/Inner` });
  await invoke("trash_entries", { paths: [`${HOME}/Old`] });
});

beforeEach(closeOverlays);

async function select(name: string): Promise<void> {
  await click(item(name));
}

async function exists(path: string): Promise<boolean> {
  return (await invoke("path_exists", { path })) as boolean;
}

test("⌘V into the Trash does nothing", async () => {
  await goTo(HOME);
  await select("main.rs");
  await key("c", { mod: true });
  await goTo(TRASH);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).not.toContain("main.rs");
  expect(await exists(`${TRASH}/main.rs`)).toBe(false);
});

test("⌘X/⌘V into a folder inside the Trash leaves the item where it was", async () => {
  await goTo(HOME);
  await select("main.rs");
  await key("x", { mod: true });
  await goTo(`${TRASH}/Old`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).not.toContain("main.rs");
  expect(await exists(`${TRASH}/Old/main.rs`)).toBe(false);
  expect(await exists(`${HOME}/main.rs`)).toBe(true);
  await key("Escape");
});

test("F2 doesn't rename an item in the Trash", async () => {
  await goTo(TRASH);
  await select("old.txt");
  await key("F2");
  expect(document.querySelector(".rename-popover")).toBeNull();
  await closeOverlays();
  await goTo(`${TRASH}/Old`);
  await select("Inner");
  await key("F2");
  expect(document.querySelector(".rename-popover")).toBeNull();
});

test("⇧⌘N doesn't make a folder inside the Trash", async () => {
  await goTo(`${TRASH}/Old`);
  await key("N", { mod: true, shift: true });
  expect(document.querySelector("#new-folder-name")).toBeNull();
  await closeOverlays();
});

test("the Trash's own menus are unchanged", async () => {
  await goTo(TRASH);
  await rightClick($("#view"));
  expect(menuLabels()).toEqual(["Empty Trash…", "Select All"]);
  await closeOverlays();
  await click($("#btn-location-menu"));
  expect(menuLabels()).toEqual(["Empty Trash…", "Select All"]);
  await closeOverlays();
});

test("a folder inside the Trash offers nothing that adds to it, but keeps Open With…", async () => {
  await goTo(`${TRASH}/Old`);
  await rightClick($("#view"));
  expect(menuLabels()).toEqual(["Open With…", "Select All"]);
  await closeOverlays();
  await click($("#btn-location-menu"));
  expect(menuLabels()).toEqual(["Open With…", "Copy Location", "Select All"]);
  await closeOverlays();
});

const mockDrop = (paths: string[], x: number, y: number) =>
  (window as unknown as { __mockDrop: (paths: string[], x: number, y: number) => void }).__mockDrop(paths, x, y);

test("files dropped on a folder inside the Trash are refused", async () => {
  await goTo(`${TRASH}/Old`);
  pointAt = $(".view-overlay");
  mockDrop([`${HOME}/Desktop/todo.txt`], 10, 10);
  await settle(30);
  pointAt = item("Inner");
  mockDrop([`${HOME}/Desktop/todo.txt`], 10, 10);
  await settle(30);
  pointAt = null;
  expect(names()).toEqual(["Inner"]);
  expect(await exists(`${TRASH}/Old/todo.txt`)).toBe(false);
  expect(await exists(`${TRASH}/Old/Inner/todo.txt`)).toBe(false);
});
