// What the app remembers by path (stars, bookmarks, the clipboard, tabs and
// their Back and Forward) after items are
// moved, renamed or sent to the Trash, driven through the real app. The
// rules themselves are tested on their own in tests/path-refs.test.ts.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, closeOverlays, dialogHeading, goTo, hit, item, key, mouse, names, respond, settle, title, toasts, type } from "./app";

const ROOT = "/Users/me/Refs";
const DEST = `${ROOT}/Dest`;
/** Where the tests of a paste still running work, so ROOT fits on screen. */
const RUN = "/Users/me/Running";

beforeAll(async () => {
  for (const dir of ["", "/Dest", "/Other", "/A", "/Clash", "/Trashy", "/Empty", "/Marked", "/Outer", "/Outer/In", "/Cuts", "/CutDest", "/CopySrc", "/CopyDest", "/Spare", "/Halt", "/HaltDest", "/HaltNext", "/TabDir", "/Box", "/Box/Sub", "/Hist", "/Fwd"]) {
    await invoke("make_dir", { path: ROOT + dir });
  }
  for (const dir of ["", "/Wait", "/Wait/Folder", "/WaitDest", "/Race", "/Race/Folder", "/RaceDest"]) {
    await invoke("make_dir", { path: RUN + dir });
  }
  for (const file of ["/note.txt", "/A/inner.txt", "/keep.txt", "/Dest/keep.txt", "/Trashy/x.txt", "/Trashy/y.txt", "/Trashy/z.txt", "/Cuts/c.txt", "/CopySrc/f.txt", "/Halt/p.txt", "/Halt/q.txt", "/HaltDest/q.txt", "/TabDir/t.txt", "/Box/Sub/s.txt"]) {
    await invoke("create_file", { path: ROOT + file });
  }
  for (const file of ["/Wait/a.txt", "/Wait/b.txt", "/WaitDest/b.txt", "/Race/a.txt", "/Race/b.txt"]) {
    await invoke("create_file", { path: RUN + file });
  }
  await boot();
  await key("1", { mod: true }); // list view: rows have a star button
});

type MockOps = { hold(): void; held(): number; release(): void; reset(): void };
const ops = () => (globalThis as unknown as { __mockOps: MockOps }).__mockOps;

afterEach(async () => {
  ops().release();
  ops().reset();
  await settle(20);
  await closeOverlays();
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

// ---- Tabs and their Back and Forward ---------------------------------------------

const tabTitles = () => $$(".tab .tab-title").map((t) => t.textContent);

test("Back and Forward follow a rename in the tab on screen", async () => {
  await goTo(`${ROOT}/Hist`);
  await goTo(ROOT);
  await rename("Hist", "Hist 2");
  await key("ArrowLeft", { alt: true });
  await settle(15);
  expect(title()).toBe("Hist 2");

  await goTo(ROOT);
  await goTo(`${ROOT}/Fwd`);
  await key("ArrowLeft", { alt: true });
  await settle(15);
  expect(title()).toBe("Refs");
  await rename("Fwd", "Fwd 2");
  await key("ArrowRight", { alt: true });
  await settle(15);
  expect(title()).toBe("Fwd 2");
});

test("a tab in the background follows a rename of the folder it shows", async () => {
  await goTo(`${ROOT}/TabDir`);
  await key("t", { mod: true }); // a second tab, also at TabDir
  await settle(10);
  await goTo(ROOT);
  await rename("TabDir", "TabDir 2");
  expect(tabTitles()).toEqual(["TabDir 2", "Refs"]);

  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("TabDir 2");
  expect(names()).toEqual(["t.txt"]);
  await key("w", { mod: true });
  await settle(20);
});

test("a tab in the background follows a folder moved around it, and so does its history", async () => {
  await goTo(`${ROOT}/Box`);
  await goTo(`${ROOT}/Box/Sub`);
  await key("t", { mod: true }); // a second tab; the first keeps Box in its Back
  await settle(10);
  await goTo(ROOT);
  await select("Box");
  await key("x", { mod: true });
  await goTo(DEST);
  await key("v", { mod: true });
  await settle(30);
  expect(tabTitles()).toEqual(["Sub", "Dest"]);

  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("Sub");
  expect(names()).toEqual(["s.txt"]);
  await key("ArrowLeft", { alt: true });
  await settle(15);
  expect(title()).toBe("Box");
  expect(names()).toEqual(["Sub"]);
  await key("w", { mod: true });
  await settle(20);
});

// ---- While a paste runs ------------------------------------------------------------

/** Wait until the mock holds the next item of a paste. */
async function waitHeld(): Promise<void> {
  await vi.waitFor(() => expect(ops().held()).toBe(1), { timeout: 2000 });
  await settle(10);
}

test("an item a paste moved is followed as it lands, while the paste waits on a name clash", async () => {
  await goTo(`${RUN}/Wait`);
  await star("a.txt");
  await goTo(`${RUN}/Wait/Folder`);
  await key("d", { mod: true });
  await goTo(`${RUN}/Wait`);
  await select("Folder", "a.txt", "b.txt");
  await key("x", { mod: true });
  await goTo(`${RUN}/WaitDest`);
  await key("v", { mod: true }); // Folder and a.txt move; b.txt clashes
  await settle(30);
  expect(dialogHeading()).toMatch(/b\.txt/);

  // Saved already: closing the window now loses nothing.
  const stars = await saved();
  expect(stars).toContain(`${RUN}/WaitDest/a.txt`);
  expect(stars).not.toContain(`${RUN}/Wait/a.txt`);
  expect(await pins()).toContain(`${RUN}/WaitDest/Folder`);
  expect(await pins()).not.toContain(`${RUN}/Wait/Folder`);
  expect(bookmark("Folder")?.dataset.location).toBe(`${RUN}/WaitDest/Folder`);

  await respond("Skip");
  await settle(20);
  expect(starred("a.txt")).toBe(true);
  expect(await saved()).toContain(`${RUN}/WaitDest/a.txt`);
});

test("an item renamed after it landed, while the rest of its paste runs, keeps its star and bookmark", async () => {
  await goTo(`${RUN}/Race`);
  await star("a.txt");
  await goTo(`${RUN}/Race/Folder`);
  await key("d", { mod: true });
  await goTo(`${RUN}/Race`);
  await select("Folder", "a.txt", "b.txt");
  await key("x", { mod: true });
  ops().hold();
  await goTo(`${RUN}/RaceDest`);
  await key("v", { mod: true });
  await waitHeld(); // Folder
  ops().release();
  await waitHeld(); // a.txt
  ops().release();
  await waitHeld(); // b.txt: Folder and a.txt have landed
  await key("r", { mod: true });
  await settle(20);
  expect(names()).toEqual(["Folder", "a.txt"]);
  expect(starred("a.txt")).toBe(true);

  await rename("a.txt", "a2.txt");
  await rename("Folder", "Folder 2");
  ops().release();
  await settle(40);

  expect(names()).toEqual(["Folder 2", "a2.txt", "b.txt"]);
  expect(starred("a2.txt")).toBe(true);
  const stars = await saved();
  expect(stars).toContain(`${RUN}/RaceDest/a2.txt`);
  expect(stars).not.toContain(`${RUN}/RaceDest/a.txt`);
  expect(await pins()).toContain(`${RUN}/RaceDest/Folder 2`);
  expect(await pins()).not.toContain(`${RUN}/RaceDest/Folder`);
  expect(bookmark("Folder 2")?.dataset.location).toBe(`${RUN}/RaceDest/Folder 2`);
});
