// What the app remembers by path (stars, bookmarks, the clipboard) after items are
// moved, renamed or sent to the Trash, driven through the real app. The
// rules themselves are tested on their own in tests/path-refs.test.ts.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, goTo, hit, item, key, mouse, names, respond, settle, title, toasts, type } from "./app";

const ROOT = "/Users/me/Refs";
const DEST = `${ROOT}/Dest`;

beforeAll(async () => {
  for (const dir of ["", "/Dest", "/Other", "/A", "/Clash", "/Trashy", "/Empty", "/Marked", "/Outer", "/Outer/In"]) {
    await invoke("make_dir", { path: ROOT + dir });
  }
  for (const file of ["/note.txt", "/A/inner.txt", "/keep.txt", "/Dest/keep.txt", "/Trashy/x.txt", "/Trashy/y.txt", "/Trashy/z.txt"]) {
    await invoke("create_file", { path: ROOT + file });
  }
  await boot();
  await key("1", { mod: true }); // list view: rows have a star button
});

const starred = (name: string) => item(name).querySelector(".star")!.classList.contains("starred");
const saved = () => invoke<string[]>("load_starred");
const exists = (path: string) => invoke<boolean>("path_exists", { path });

async function star(name: string): Promise<void> {
  await mouse(item(name).querySelector(".star")!, "mousedown");
  await settle(10);
}

/** Select `list` (the first plainly, the rest added) in the folder on screen. */
async function select(...list: string[]): Promise<void> {
  await click(hit(list[0]));
  for (const name of list.slice(1)) await click(hit(name), { metaKey: true });
}

/** F2 on `name`, type `to`, Enter. */
async function rename(name: string, to: string): Promise<void> {
  await select(name);
  await key("F2");
  const input = $<HTMLInputElement>(".rename-popover input");
  await type(input, to);
  await key("Enter", { target: input });
  await settle(20);
}

// ---- Stars ------------------------------------------------------------------

test("stars follow a cut and paste: the item, and what is inside a moved folder", async () => {
  await goTo(`${ROOT}/A`);
  await star("inner.txt");
  await goTo(ROOT);
  await star("note.txt");
  await star("A");

  await select("note.txt", "A");
  await key("x", { mod: true });
  await goTo(DEST);
  await key("v", { mod: true });
  await settle(30);

  expect(starred("note.txt")).toBe(true);
  expect(starred("A")).toBe(true);
  const stars = await saved();
  expect(stars).toEqual(expect.arrayContaining([`${DEST}/note.txt`, `${DEST}/A`, `${DEST}/A/inner.txt`]));
  for (const old of [`${ROOT}/note.txt`, `${ROOT}/A`, `${ROOT}/A/inner.txt`]) expect(stars).not.toContain(old);
  await goTo(`${DEST}/A`);
  expect(starred("inner.txt")).toBe(true);
});

test("stars follow a rename, on the folder and inside it", async () => {
  await goTo(DEST);
  await rename("A", "B");
  expect(names()).toContain("B");
  expect(starred("B")).toBe(true);
  const stars = await saved();
  expect(stars).toEqual(expect.arrayContaining([`${DEST}/B`, `${DEST}/B/inner.txt`]));
  expect(stars).not.toContain(`${DEST}/A`);
  expect(stars).not.toContain(`${DEST}/A/inner.txt`);
});

test("the preview's Rename… keeps the star", async () => {
  await goTo(DEST);
  await click(hit("note.txt"));
  await key(" ");
  await settle(20);
  await click($$("#preview-actions button").find((b) => b.textContent === "Rename…")!);
  await settle(10);
  await type($<HTMLInputElement>(".dialog input"), "note-final.txt");
  await respond("Rename");
  await settle(20);
  await key("Escape");
  await settle(10);

  expect(starred("note-final.txt")).toBe(true);
  const stars = await saved();
  expect(stars).toContain(`${DEST}/note-final.txt`);
  expect(stars).not.toContain(`${DEST}/note.txt`);
});

test("a move that was skipped leaves the star where it was", async () => {
  await goTo(ROOT);
  await star("keep.txt");
  await select("keep.txt");
  await key("x", { mod: true });
  await goTo(DEST);
  await key("v", { mod: true });
  await settle(20);
  await respond("Skip");
  await settle(20);

  expect(await exists(`${ROOT}/keep.txt`)).toBe(true);
  const stars = await saved();
  expect(stars).toContain(`${ROOT}/keep.txt`);
  expect(stars).not.toContain(`${DEST}/keep.txt`);
  expect(starred("keep.txt")).toBe(false);
});

// ---- The clipboard ------------------------------------------------------------

test("a cut that was pasted in full is used up", async () => {
  await goTo(`${ROOT}/Other`);
  await invoke("create_file", { path: `${ROOT}/Other/once.txt` });
  await key("r", { mod: true });
  await select("once.txt");
  await key("x", { mod: true });
  await goTo(`${ROOT}/Clash`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toContain("once.txt");
  expect(item("once.txt").classList.contains("cut")).toBe(false);

  await goTo(`${ROOT}/Empty`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toEqual([]);
});

test("a cut item sent to the Trash leaves the clipboard; the rest still pastes", async () => {
  await goTo(`${ROOT}/Trashy`);
  await select("x.txt", "y.txt");
  await key("x", { mod: true });
  await select("x.txt");
  await key("Delete");
  await settle(20);
  expect(names()).not.toContain("x.txt");

  await goTo(`${ROOT}/Other`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toContain("y.txt");
  expect(names()).not.toContain("x.txt");
  expect(toasts().join(" ")).not.toMatch(/Could not move/);
});

test("when every cut item went to the Trash, there is nothing to paste", async () => {
  await goTo(`${ROOT}/Trashy`);
  await select("z.txt");
  await key("x", { mod: true });
  await key("Delete");
  await settle(20);

  await goTo(`${ROOT}/Empty`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toEqual([]);
  await mouse($("#view"), "mousedown", { button: 2 });
  await mouse($("#view"), "contextmenu", { button: 2 });
  const paste = $$(".popover .menu-item").find((b) => b.textContent === "Paste");
  expect(paste?.hasAttribute("disabled")).toBe(true);
  await key("Escape");
});

// ---- Bookmarks ----------------------------------------------------------------

const bookmark = (label: string) =>
  $$("#sidebar-bookmarks .sidebar-row.pin").find((row) => row.textContent?.trim() === label);
const pins = () => invoke<string[]>("load_pins");

test("a bookmark follows a rename of its folder", async () => {
  await goTo(`${ROOT}/Marked`);
  await key("d", { mod: true });
  expect(bookmark("Marked")).toBeDefined();
  await goTo(ROOT);
  await rename("Marked", "Marked 2");

  expect(bookmark("Marked")).toBeUndefined();
  expect(bookmark("Marked 2")?.dataset.location).toBe(`${ROOT}/Marked 2`);
  expect(await pins()).toContain(`${ROOT}/Marked 2`);
  expect(await pins()).not.toContain(`${ROOT}/Marked`);
  await click(bookmark("Marked 2")!);
  await settle(15);
  expect(title()).toBe("Marked 2");
});

test("a bookmark inside a moved folder follows it, keeping its place in the list", async () => {
  await goTo(`${ROOT}/Outer/In`);
  await key("d", { mod: true });
  const before = await pins();
  expect(before.at(-1)).toBe(`${ROOT}/Outer/In`);

  await goTo(ROOT);
  await select("Outer");
  await key("x", { mod: true });
  await goTo(DEST);
  await key("v", { mod: true });
  await settle(30);

  expect(await pins()).toEqual([...before.slice(0, -1), `${DEST}/Outer/In`]);
  expect(bookmark("In")?.dataset.location).toBe(`${DEST}/Outer/In`);
});
