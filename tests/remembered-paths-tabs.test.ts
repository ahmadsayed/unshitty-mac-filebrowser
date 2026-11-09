// Bookmarks and tabs (with their Back and Forward) follow a move and a
// rename, driven through the real app. The rules are in tests/path-refs.test.ts.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, click, closeOverlays, dialogHeading, goTo, item, key, mouse, names, respond, settle, title, type } from "./app";
import { DEST, ROOT, RUN, cleanRefs, exists, ops, rename, saved, select, setupRefs, star, starred } from "./support/remembered-paths";

beforeAll(setupRefs);
afterEach(cleanRefs);

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
