// Stars and the clipboard follow a move, a rename and the Trash, driven
// through the real app. The rules themselves are in tests/path-refs.test.ts.
import { afterEach, beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, click, closeOverlays, dialogHeading, goTo, hit, item, key, mouse, names, respond, settle, title, toasts, type } from "./app";
import { DEST, ROOT, RUN, cleanRefs, exists, rename, saved, select, setupRefs, star, starred } from "./support/remembered-paths";

beforeAll(setupRefs);
afterEach(cleanRefs);

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

test("a cut item that was renamed stays cut, and is pasted from its new name", async () => {
  await goTo(`${ROOT}/Cuts`);
  await select("c.txt");
  await key("x", { mod: true });
  await rename("c.txt", "c2.txt");
  expect(item("c2.txt").classList.contains("cut")).toBe(true);

  await goTo(`${ROOT}/CutDest`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toEqual(["c2.txt"]);
  expect(toasts().join(" ")).not.toMatch(/Could not move/);
  expect(await exists(`${ROOT}/Cuts/c2.txt`)).toBe(false);

  // The cut was used up.
  await goTo(`${ROOT}/Spare`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toEqual([]);
});

test("a cut whose paste stopped part-way keeps every item, each where it is now", async () => {
  await goTo(`${ROOT}/Halt`);
  await select("p.txt", "q.txt");
  await key("x", { mod: true });
  await goTo(`${ROOT}/HaltDest`);
  await key("v", { mod: true }); // p.txt moves; q.txt clashes
  await closeOverlays(); // closing the clash dialog cancels the rest
  await settle(20);
  expect(names()).toEqual(["p.txt", "q.txt"]);
  // The cut wasn't used up: p.txt is still cut, at its new place.
  expect(item("p.txt").classList.contains("cut")).toBe(true);

  await goTo(`${ROOT}/HaltNext`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toEqual(["p.txt", "q.txt"]);
  expect(toasts().join(" ")).not.toMatch(/Could not move/);
});

test("a copied item inside a renamed folder is pasted from the folder's new path", async () => {
  await goTo(`${ROOT}/CopySrc`);
  await select("f.txt");
  await key("c", { mod: true });
  await goTo(ROOT);
  await rename("CopySrc", "CopySrc 2");

  await goTo(`${ROOT}/CopyDest`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toEqual(["f.txt"]);
  expect(toasts().join(" ")).not.toMatch(/Could not copy/);
  expect(await exists(`${ROOT}/CopySrc 2/f.txt`)).toBe(true);
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
