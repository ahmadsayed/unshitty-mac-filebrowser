// Arbesk in Network: connect with an email through the mock, browse the
// account's collections and assets read only, and keep its row when the
// backend can't answer for a moment.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, accountGroups, boot, chooseAccountRow, click, closeOverlays, dialogHeading, goTo, installFakeCanvas, key, menuLabels, mouse, names, open, openOnlineAccounts, respond, settle, toasts, type } from "./app";

const calls = () =>
  (globalThis as unknown as { __mockCalls: { failNext(cmd: string, error: string): void } }).__mockCalls;

beforeAll(async () => {
  installFakeCanvas(); // the mock draws the thumbnails Arbesk hands over
  await boot();
  await key("1", { mod: true }); // the list view, with its size and date cells
});

async function openNetwork(): Promise<void> {
  await click($$(".sidebar-row").find((row) => row.textContent?.trim() === "Network")!);
  await settle(20);
}

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;
const crumbs = () => $$(".crumb").map((el) => el.textContent?.trim());

test("Arbesk connects with an email and opens on the account's collections", async () => {
  await openOnlineAccounts();
  await chooseAccountRow("Connect an Account", "Arbesk");
  expect(dialogHeading()).toBe("Connect to Arbesk");
  expect($(".dialog-text").textContent).toBe("Arbesk emails you a code. Enter it in your browser to sign in.");
  const email = $<HTMLInputElement>("#arbesk-email");
  const openBrowser = $$(".dialog button").find((b) => b.textContent === "Open Browser")!;
  expect(openBrowser.hasAttribute("disabled")).toBe(true);
  await type(email, "me@example.com");
  expect(openBrowser.hasAttribute("disabled")).toBe(false);

  await respond("Open Browser");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts()).toContain("Connected to me@example.com");
  expect(crumbs()).toEqual(["Arbesk"]);
  expect(names().sort()).toEqual(["Shapes", "Untitled Collection"]);
});

test("Network lists the account as Arbesk, and Online Accounts lists it with its email", async () => {
  await openNetwork();
  expect(names()).toContain("Arbesk");
  await openOnlineAccounts();
  expect(accountGroups()[0]).toEqual({ title: "Your Accounts", rows: ["Arbesk · me@example.com"] });
  await closeOverlays();
  // An arbesk:// address means nothing outside the app: no Copy Location.
  await rightClick(row("Arbesk"));
  expect(menuLabels()).toEqual(["Open", "Disconnect…"]);
  await closeOverlays();
});

test("a collection lists its assets as files, without sizes", async () => {
  await openNetwork();
  await open("Arbesk");
  await settle(20);
  await open("Shapes");
  await settle(20);
  expect(crumbs()).toEqual(["Arbesk", "Shapes"]);
  expect(names().sort()).toEqual(["Big.gltf", "Box.3mf", "Box.gltf"]);
  expect(row("Box.gltf").querySelector(".meta.size")?.textContent).toBe("");
  await click(row("Box.3mf"));
  expect($("#floating-bar").textContent).toBe("“Box.3mf” selected");
});

test("an asset shows the thumbnail it was saved with, and one saved without keeps its icon", async () => {
  expect(crumbs()).toEqual(["Arbesk", "Shapes"]);
  await settle(20);
  expect(row("Box.gltf").querySelector(".icon img")).not.toBeNull();
  expect(row("Box.3mf").querySelector(".icon img")).toBeNull();
  expect(row("Big.gltf").querySelector(".icon img")).toBeNull();
});

test("nothing in Arbesk can be written, renamed or cut", async () => {
  await click(row("Box.gltf"));
  await rightClick(row("Box.gltf"));
  expect(menuLabels()).toEqual(["Open", "Copy"]);
  await closeOverlays();

  await click(row("Box.gltf"));
  await key("x", { mod: true });
  await settle();
  expect(toasts()).toContain("Arbesk is read only.");

  // A local file on the clipboard isn't a model: refused before anything starts.
  await goTo("/Users/me/Desktop");
  await click(row("todo.txt"));
  await key("c", { mod: true });
  await openNetwork();
  await open("Arbesk");
  await settle(20);
  await open("Shapes");
  await settle(20);
  await key("v", { mod: true });
  await settle();
  expect(toasts()).toContain("Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk.");
  expect(names().sort()).toEqual(["Big.gltf", "Box.3mf", "Box.gltf"]);
});

test("a collection can't be copied out yet", async () => {
  await openNetwork();
  await open("Arbesk");
  await settle(20);
  await click(row("Shapes"));
  await rightClick(row("Shapes"));
  expect($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Copy")?.hasAttribute("disabled")).toBe(true);
  await closeOverlays();
  await key("c", { mod: true });
  await settle();
  expect(toasts()).toContain("Copying Arbesk collections comes next.");
});

test("Copy and Paste take assets out to a folder, asking when a name is taken", async () => {
  await invoke("make_dir", { path: "/Users/me/Models" });
  await openNetwork();
  await open("Arbesk");
  await settle(20);
  await open("Shapes");
  await settle(20);
  await click(row("Box.gltf"));
  await click(row("Box.3mf"), { shiftKey: true });
  await key("c", { mod: true });
  await goTo("/Users/me/Models");
  await key("v", { mod: true });
  await settle(30);
  expect(names().sort()).toEqual(["Box.3mf", "Box.gltf"]);

  // Again: the names are taken, so it asks; Enter keeps both.
  await key("v", { mod: true });
  await settle(30);
  expect(dialogHeading()).toMatch(/Replace/);
  expect($(".dialog-layer .dialog-text").textContent).toBe(
    "Another file with the same name already exists in “Models”. Replacing it moves the existing file to the Trash.",
  );
  await key("Enter");
  await settle(30);
  expect(dialogHeading()).toMatch(/Replace/);
  await key("Enter");
  await settle(30);
  expect(names().sort()).toEqual(["Box (copy).3mf", "Box (copy).gltf", "Box.3mf", "Box.gltf"]);
});

test("a drag from Arbesk copies, and a forced move does nothing", async () => {
  const pointAt = document.elementFromPoint;
  let target: Element | null = null;
  document.elementFromPoint = () => target;
  const desktop = $$(".sidebar-row").find((r) => r.dataset.location === "/Users/me/Desktop")!;
  const lit: boolean[] = [];
  const dragTo = async (name: string, mods: MouseEventInit = {}) => {
    await mouse(row(name), "mousedown", { clientX: 50, clientY: 60 });
    target = desktop;
    for (const at of [300, 301]) {
      window.dispatchEvent(new MouseEvent("mousemove", { clientX: at, clientY: at, ...mods }));
      await settle(10);
    }
    lit.push(desktop.classList.contains("drop-target"));
    window.dispatchEvent(new MouseEvent("mouseup", { clientX: 301, clientY: 301, ...mods }));
    await settle(30);
  };
  try {
    await openNetwork();
    await open("Arbesk");
    await settle(20);
    await open("Shapes");
    await settle(20);
    await click(row("Box.3mf"));
    await dragTo("Box.3mf", { metaKey: true });
    expect(await invoke("path_exists", { path: "/Users/me/Desktop/Box.3mf" })).toBe(false);
    await click(row("Box.3mf"));
    await dragTo("Box.3mf");
    expect(await invoke("path_exists", { path: "/Users/me/Desktop/Box.3mf" })).toBe(true);
    // The forced move never offered the Desktop as a target; the copy did.
    expect(lit).toEqual([false, true]);
    // The asset is still in its collection.
    expect(names()).toContain("Box.3mf");
  } finally {
    document.elementFromPoint = pointAt;
  }
});

test("past the window's edge an asset stays in File Browser, as a Drive item does", async () => {
  const startDrag = vi.fn(async () => {});
  const tauri = window as unknown as { __TAURI__?: { drag?: { startDrag: typeof startDrag } } };
  tauri.__TAURI__ = { drag: { startDrag } };
  try {
    await openNetwork();
    await open("Arbesk");
    await settle(20);
    await open("Shapes");
    await settle(20);
    await click(row("Box.gltf"));
    await mouse(row("Box.gltf"), "mousedown", { clientX: 50, clientY: 60 });
    window.dispatchEvent(new MouseEvent("mousemove", { clientX: 300, clientY: 300, buttons: 1 }));
    await settle(10);
    window.dispatchEvent(new MouseEvent("mousemove", { clientX: -5, clientY: 300, buttons: 1 }));
    await settle(20);
    expect(startDrag).not.toHaveBeenCalled();
    expect($(".drag-ghost")).not.toBeNull();
    window.dispatchEvent(new MouseEvent("mouseup", { clientX: -5, clientY: 300 }));
    await settle(20);
    expect(toasts().some((t) => t.startsWith("Could not drag out"))).toBe(false);
  } finally {
    delete tauri.__TAURI__;
  }
});

test("Open on an asset opens it in Arbesk's Studio, in the browser", async () => {
  await openNetwork();
  await open("Arbesk");
  await settle(20);
  await open("Shapes");
  await settle(20);
  await click($("#btn-location-menu"));
  expect(menuLabels()).not.toContain("Copy Location");
  await closeOverlays();

  const log = vi.spyOn(console, "log");
  await open("Box.gltf");
  await settle(20);
  expect(log).toHaveBeenCalledWith("mock open_url", "https://promptscad.com/studio?asset=1&assetId=box-g");
  log.mockRestore();
});

// ----- Copying into a collection (spec 2026-11-09-arbesk-copy-in) -----

const UP = "/Users/me/Up";
const PUBLIC = "Files in Arbesk are public: anyone with their address can download them, and they can't be removed from IPFS.";
const ONLY_MODELS = "Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk.";
const arbeskMock = () => (globalThis as unknown as { __mockArbesk: { signedIn: boolean; expireOnce(): void } }).__mockArbesk;
const dialogButtons = () => $$(".dialog-layer .dialog-responses button").map((b) => b.textContent);
const copyFailures = () => toasts().filter((t) => t.startsWith("Could not copy"));

async function openCollection(name: string): Promise<void> {
  await openNetwork();
  await open("Arbesk");
  await settle(20);
  await open(name);
  await settle(20);
}

/** Copy `picks` from the local folder Up. */
async function copyFromUp(...picks: string[]): Promise<void> {
  await goTo(UP);
  for (const [i, name] of picks.entries()) await click(row(name), { metaKey: i > 0 });
  await key("c", { mod: true });
}

/** Paste into the open collection and answer Copy to Arbesk? with Copy. */
async function pasteAndConfirm(): Promise<void> {
  await key("v", { mod: true });
  await settle(10);
  expect(dialogHeading()).toBe("Copy to Arbesk?");
  await respond("Copy");
  await settle(20);
}

/** What the mock lists at `segments` below the account (its collections when empty). */
async function listedIn(segments: string): Promise<Array<{ name: string; path: string }>> {
  const { ARBESK_MOCK_ADDRESS: address } = await import("../dev/mock-tauri/core");
  return invoke("list_dir", { path: `arbesk://${address}/${segments}`, showHidden: false });
}

test("Copy and Paste put model files into a collection after confirming", async () => {
  await invoke("make_dir", { path: UP });
  for (const name of ["Box.glb", "Tray.3mf", "Gear.glb", "Bolt.3mf", "Nut.glb", "Peg.gltf", "notes.txt"]) {
    await invoke("create_file", { path: `${UP}/${name}` });
  }
  await invoke("make_dir", { path: `${UP}/Kit.glb` });
  await copyFromUp("Box.glb", "Tray.3mf");
  await openCollection("Untitled Collection");
  await key("v", { mod: true });
  await settle(10);
  expect(dialogHeading()).toBe("Copy to Arbesk?");
  expect($(".dialog-layer .dialog-text").textContent).toBe(PUBLIC);
  expect(dialogButtons()).toEqual(["Cancel", "Copy"]);
  // Enter is Cancel: nothing goes public by accident.
  await key("Enter");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(names().sort()).toEqual(["Cube.gltf", "Table.gltf"]);

  await pasteAndConfirm();
  // A .glb is stored as glTF, so it lands as Box.gltf.
  await vi.waitFor(() => expect(names().sort()).toEqual(["Box.gltf", "Cube.gltf", "Table.gltf", "Tray.3mf"]), { timeout: 2000 });
  expect(copyFailures()).toEqual([]);
});

test("a .glb pastes as its listed .gltf name: a clash asks, Keep Both and Replace", async () => {
  await copyFromUp("Box.glb");
  await openCollection("Untitled Collection");
  for (const copy of ["Box (copy).gltf", "Box (copy 2).gltf"]) {
    await pasteAndConfirm();
    expect(dialogHeading()).toBe("Replace file “Box.gltf”?");
    // Replace in Arbesk saves a version: nothing goes to the Trash.
    expect($(".dialog-layer .dialog-text").textContent).toBe(
      "Another file with the same name already exists in “Untitled Collection”. Replacing it saves your file as its newest version; the earlier ones stay in its history.",
    );
    await respond("Keep Both");
    await vi.waitFor(() => expect(names()).toContain(copy), { timeout: 2000 });
  }
  await pasteAndConfirm();
  expect(dialogHeading()).toBe("Replace file “Box.gltf”?");
  await respond("Replace");
  await settle(30);
  // Replace saves the file as the asset's next version: still one Box.gltf.
  const assets = await listedIn("2~Untitled%20Collection");
  expect(assets.filter((a) => a.name === "Box.gltf")).toHaveLength(1);
  const box = assets.find((a) => a.name === "Box.gltf")!.path;
  expect(await invoke<unknown[]>("arbesk_versions", { path: box })).toHaveLength(2);
  expect(assets.filter((a) => a.name.startsWith("Box")).map((a) => a.name)).toEqual(["Box.gltf", "Box (copy).gltf", "Box (copy 2).gltf"]);
});

test("an upper-case extension still clashes with the listed lower-case name", async () => {
  await invoke("make_dir", { path: UP }).catch(() => {});
  await invoke("create_file", { path: `${UP}/Box.GLTF` });
  await copyFromUp("Box.GLTF");
  await openCollection("Untitled Collection");
  await pasteAndConfirm();
  // Arbesk lists it as Box.gltf, which is taken: asked, not added as a second Box.
  expect(dialogHeading()).toBe("Replace file “Box.gltf”?");
  await respond("Skip");
  await settle(20);
  const assets = await listedIn("2~Untitled%20Collection");
  expect(assets.filter((a) => /^box\./i.test(a.name))).toHaveLength(1);
});

test("a paste with no session signs in first, and Cancel copies nothing", async () => {
  arbeskMock().signedIn = false;
  await copyFromUp("Gear.glb");
  await openCollection("Untitled Collection");
  await pasteAndConfirm();
  expect(dialogHeading()).toBe("Sign In to Arbesk");
  expect($<HTMLInputElement>("#arbesk-email").value).toBe("me@example.com");
  await respond("Cancel");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect((await listedIn("2~Untitled%20Collection")).map((a) => a.name)).not.toContain("Gear.gltf");

  await pasteAndConfirm();
  expect(dialogHeading()).toBe("Sign In to Arbesk");
  await respond("Open Browser");
  await vi.waitFor(() => expect(names()).toContain("Gear.gltf"), { timeout: 2000 });
  // Signing in for a paste stays in the collection.
  expect(crumbs()).toEqual(["Arbesk", "Untitled Collection"]);
  expect(copyFailures()).toEqual([]);
});

test("an expired session signs in again once and carries on", async () => {
  arbeskMock().expireOnce();
  await copyFromUp("Bolt.3mf", "Nut.glb");
  await openCollection("Untitled Collection");
  await pasteAndConfirm();
  await vi.waitFor(() => expect(dialogHeading()).toBe("Sign In to Arbesk"), { timeout: 2000 });
  await respond("Open Browser");
  await vi.waitFor(() => expect(names()).toEqual(expect.arrayContaining(["Bolt.3mf", "Nut.gltf"])), { timeout: 2000 });
  expect(dialogHeading()).toBeUndefined();
  const listed = (await listedIn("2~Untitled%20Collection")).map((a) => a.name);
  expect(listed.filter((n) => n === "Bolt.3mf" || n === "Nut.gltf").sort()).toEqual(["Bolt.3mf", "Nut.gltf"]);
  expect(copyFailures()).toEqual([]);
  expect(crumbs()).toEqual(["Arbesk", "Untitled Collection"]);
});

test("cancelling the mid-batch sign-in stops the batch", async () => {
  for (const name of ["Cap.3mf", "Lid.glb", "Rim.glb"]) await invoke("create_file", { path: `${UP}/${name}` });
  arbeskMock().expireOnce();
  await copyFromUp("Cap.3mf", "Lid.glb", "Rim.glb");
  await openCollection("Untitled Collection");
  await pasteAndConfirm();
  await vi.waitFor(() => expect(dialogHeading()).toBe("Sign In to Arbesk"), { timeout: 2000 });
  await respond("Cancel");
  await settle(40);
  // Nothing after the first item ran: no clash dialog, no further failures.
  expect(dialogHeading()).toBeUndefined();
  const listed = (await listedIn("2~Untitled%20Collection")).map((a) => a.name);
  expect(listed).not.toContain("Lid.gltf");
  expect(listed).not.toContain("Rim.gltf");
  expect(copyFailures()).toHaveLength(1);
  // Only the item that met the expired session is reported.
  expect(copyFailures()[0]).toMatch(/^Could not copy “Cap\.3mf”: /);
  arbeskMock().signedIn = true;
});

test("a text file or a folder is refused with the D1 toast, before any dialog", async () => {
  for (const pick of ["notes.txt", "Kit.glb"]) {
    await copyFromUp(pick);
    await openCollection("Shapes");
    await key("v", { mod: true });
    await settle(10);
    expect(dialogHeading()).toBeUndefined();
    expect(toasts()).toContain(ONLY_MODELS);
    await closeOverlays();
  }
  expect(names().sort()).toEqual(["Big.gltf", "Box.3mf", "Box.gltf"]);
});

test("a drop on a collection copies; a forced move or a folder does nothing", async () => {
  const pointAt = document.elementFromPoint;
  let target: Element | null = null;
  document.elementFromPoint = () => target;
  // Shapes in the sidebar, where a drag from a local folder can reach it.
  await openCollection("Shapes");
  const shapes = (await listedIn("")).find((c) => c.name === "Shapes")!.path;
  await key("d", { mod: true });
  await settle(10);
  const bookmark = () => $$(".sidebar-row").find((r) => r.dataset.location === shapes)!;
  const lit: boolean[] = [];
  const dragTo = async (name: string, mods: MouseEventInit = {}) => {
    await click(row(name));
    await mouse(row(name), "mousedown", { clientX: 50, clientY: 60 });
    target = bookmark();
    for (const at of [300, 301]) {
      window.dispatchEvent(new MouseEvent("mousemove", { clientX: at, clientY: at, ...mods }));
      await settle(10);
    }
    lit.push(bookmark().classList.contains("drop-target"));
    window.dispatchEvent(new MouseEvent("mouseup", { clientX: 301, clientY: 301, ...mods }));
    await settle(20);
  };
  try {
    await goTo(UP);
    await dragTo("Peg.gltf", { metaKey: true });
    expect(dialogHeading()).toBeUndefined();
    await dragTo("Kit.glb");
    expect(dialogHeading()).toBeUndefined();
    await dragTo("Peg.gltf");
    expect(lit).toEqual([false, false, true]);
    expect(dialogHeading()).toBe("Copy to Arbesk?");
    await respond("Copy");
    await settle(30);
    expect((await listedIn("1~Shapes")).map((a) => a.name)).toContain("Peg.gltf");
    expect(await invoke("path_exists", { path: `${UP}/Peg.gltf` })).toBe(true);

    // From another app, onto the open collection: a folder is refused in words.
    await openCollection("Shapes");
    target = $(".view-overlay");
    const drop = (window as unknown as { __mockDrop: (paths: string[], x: number, y: number) => void }).__mockDrop;
    drop([`${UP}/Kit.glb`], 10, 10);
    await settle(30);
    expect(dialogHeading()).toBeUndefined();
    expect(toasts()).toContain(ONLY_MODELS);
    drop([`${UP}/Tray.3mf`], 10, 10);
    await settle(30);
    expect(dialogHeading()).toBe("Copy to Arbesk?");
    await respond("Copy");
    await vi.waitFor(() => expect(names()).toContain("Tray.3mf"), { timeout: 2000 });
  } finally {
    document.elementFromPoint = pointAt;
  }
});

test("a failed accounts answer keeps the Arbesk row in Network", async () => {
  calls().failNext("arbesk_accounts", "offline");
  await openNetwork();
  expect(names()).toContain("Arbesk");
});

test("Disconnect… removes the Arbesk account and leaves its collections alone", async () => {
  await openNetwork();
  await rightClick(row("Arbesk"));
  await click($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Disconnect…")!);
  expect(dialogHeading()).toBe("Disconnect Arbesk?");
  expect($(".dialog-text").textContent).toBe("“me@example.com” is removed from File Browser. Your collections on Arbesk are not changed.");
  await respond("Disconnect");
  await settle(20);
  expect(names()).not.toContain("Arbesk");
  expect(await invoke("arbesk_accounts", {})).toEqual([]);
});
