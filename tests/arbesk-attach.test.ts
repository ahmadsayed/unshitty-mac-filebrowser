// Adding files to an Arbesk asset and removing them (spec F10–F12): Paste
// Into Asset and drops on an asset, each refusal's toast, the confirmation,
// the id clash dialog, one version per paste, CONFLICT and a failed upload
// changing nothing, signing in again, the emblem after an attach, and
// Remove… on the Files… page. The account is connected before the app boots.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, accountGroups, boot, click, closeOverlays, dialogHeading, goTo, installFakeCanvas, key, menuLabels, names, open, openOnlineAccounts, respond, rightClick, settle, toasts } from "./app";

const ATT = "/Users/me/Att";
const PRINTS = "/Users/me/Documents/Projects/Prints";
const PUBLIC = "Files in Arbesk are public: anyone with their address can download them, and they can't be removed from IPFS.";

type MockCalls = { hold(cmd: string): void; release(cmd: string): void; count(cmd: string): number };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
type ArbeskHooks = {
  signedIn: boolean;
  expireOnce(): void;
  conflictNext(): void;
  failUploadNext(): void;
  changeFile(token: string, asset: string, file: string): void;
  addAsset(token: string, id: string, name: string, files: unknown[], unreadable?: unknown[]): void;
};
const arbeskMock = () => (globalThis as unknown as { __mockArbesk: ArbeskHooks }).__mockArbesk;

const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;
const dialogButtons = () => $$(".dialog-layer:last-child .dialog-responses button").map((b) => b.textContent);
const dialogTexts = () => $$(".dialog-layer:last-child .dialog-text").map((p) => p.textContent);
const emblemTitle = (name: string) => row(name).querySelector<HTMLElement>(".emblem-files")?.title ?? null;

let root = "";
beforeAll(async () => {
  installFakeCanvas();
  const core = await import("../dev/mock-tauri/core");
  root = `arbesk://${core.ARBESK_MOCK_ADDRESS}`;
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  await invoke("make_dir", { path: ATT });
  for (const name of ["Part.stl", "Two.stl", "Bracket.gcode", "Lid.blend", "README", "Notes.plan"]) {
    await invoke("create_file", { path: `${ATT}/${name}` });
  }
  await invoke("make_dir", { path: `${ATT}/Parts.stl` });
  // An asset holding an entry File Browser can't read, under the id `plan`.
  arbeskMock().addAsset("2", "hook", "Hook", [], [{ id: "plan", role: "someday" }]);
  await boot();
  await key("1", { mod: true });
});

async function openCollection(name: string): Promise<void> {
  await click($$(".sidebar-row").find((el) => el.textContent?.trim() === "Network")!);
  await settle(20);
  await open("Arbesk");
  await settle(20);
  await open(name);
  await settle(20);
}

/** Copy (or cut) `picks` from `dir`. */
async function copyFrom(dir: string, picks: string[], cut = false): Promise<void> {
  await goTo(dir);
  for (const [i, name] of picks.entries()) await click(row(name), { metaKey: i > 0 });
  await key(cut ? "x" : "c", { mod: true });
}

/** Paste Into Asset on `asset` in the open collection. */
async function pasteInto(asset: string): Promise<void> {
  await click(row(asset));
  await rightClick(row(asset));
  await click($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Paste Into Asset")!);
  await settle(20);
}

/** The ids of `asset`'s files in collection `token`, as the mock keeps them. */
async function idsOf(token: string, segment: string): Promise<string[]> {
  const files = await invoke<Array<{ id: string }>>("arbesk_representations", { path: `${root}/${token}/${segment}`, version: null });
  return files.map((file) => file.id);
}

test("Paste Into Asset asks first, Cancel the default, then adds the file as one version", async () => {
  await copyFrom(ATT, ["Part.stl"]);
  await openCollection("Untitled Collection");
  expect(emblemTitle("Table.gltf")).toBeNull();
  await click(row("Table.gltf"));
  await rightClick(row("Table.gltf"));
  expect(menuLabels()).toEqual(["Open", "Open With…", "Copy", "Paste Into Asset", "Properties"]);
  await click($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Paste Into Asset")!);
  await settle(20);
  expect(dialogHeading()).toBe("Add to “Table.gltf”?");
  expect(dialogTexts()).toEqual([PUBLIC]);
  expect(dialogButtons()).toEqual(["Cancel", "Add"]);
  // Enter is Cancel: nothing goes public by accident.
  await key("Enter");
  await settle(20);
  expect(calls().count("arbesk_attach")).toBe(0);

  await pasteInto("Table.gltf");
  await respond("Add");
  await vi.waitFor(() => expect(emblemTitle("Table.gltf")).toBe("Has 1 other file"), { timeout: 2000 });
  expect(calls().count("arbesk_attach")).toBe(1);
  expect(await idsOf("2~Untitled%20Collection", "table~Table.gltf")).toEqual(["stl"]);
  // The emblem's count changes with the next attach too.
  await copyFrom(ATT, ["Bracket.gcode"]);
  await openCollection("Untitled Collection");
  await pasteInto("Table.gltf");
  await respond("Add");
  await vi.waitFor(() => expect(emblemTitle("Table.gltf")).toBe("Has 2 other files"), { timeout: 2000 });
});

test("a source file adds a paragraph to the confirmation", async () => {
  await copyFrom(ATT, ["Lid.blend", "Two.stl"]);
  await openCollection("Untitled Collection");
  await pasteInto("Hook.gltf");
  expect(dialogHeading()).toBe("Add to “Hook.gltf”?");
  expect(dialogTexts()).toEqual([
    PUBLIC,
    "“Lid.blend” is a source file. It can hold more than the model, such as other scenes and embedded images.",
  ]);
  await respond("Cancel");
  await settle(10);
  expect(calls().count("arbesk_attach")).toBe(2);
});

test("each refusal is one toast before any dialog", async () => {
  const cases: Array<[string, string[], boolean, string]> = [
    [ATT, ["Part.stl"], true, "Arbesk is read only."],
    [ATT, ["README"], false, "“README” has no file type to record."],
    [ATT, ["Parts.stl"], false, "Only files on this computer can be added to an asset."],
    [PRINTS, ["lamp.gltf"], false, "“lamp.gltf” uses files beside it. Add a .glb instead."],
    [PRINTS, ["scan.stl"], false, "“scan.stl” is larger than 256 MB."],
  ];
  for (const [dir, picks, cut, words] of cases) {
    await copyFrom(dir, picks, cut);
    await openCollection("Untitled Collection");
    await pasteInto("Table.gltf");
    expect(dialogHeading(), words).toBeUndefined();
    expect(toasts()).toContain(words);
    await closeOverlays();
  }
  // An Arbesk asset's file isn't one on this computer.
  await openCollection("Shapes");
  await click(row("Box.gltf"));
  await key("c", { mod: true });
  await pasteInto("Box.3mf");
  expect(toasts()).toContain("Only files on this computer can be added to an asset.");
  await closeOverlays();
  expect(calls().count("arbesk_attach")).toBe(2);
});

test("a clash asks in its own words; Enter keeps both, Skip of every file adds nothing", async () => {
  await copyFrom(ATT, ["Part.stl"]);
  await openCollection("Untitled Collection");
  await pasteInto("Table.gltf");
  expect(dialogHeading()).toBe("Replace “Part.stl” in “Table.gltf”?");
  expect(dialogTexts()[0]).toMatch(/^STL · .* · Mesh$/);
  expect($(".dialog-layer:last-child .dialog-extra").textContent).toContain("The new file is “Part.stl”.");
  expect(dialogButtons()).toEqual(["Skip", "Keep Both", "Replace"]);
  await key("Enter");
  await settle(20);
  expect(dialogHeading()).toBe("Add to “Table.gltf”?");
  await respond("Add");
  await vi.waitFor(() => expect(emblemTitle("Table.gltf")).toBe("Has 3 other files"), { timeout: 2000 });
  expect(await idsOf("2~Untitled%20Collection", "table~Table.gltf")).toEqual(["stl", "gcode", "stl-2"]);

  const before = calls().count("arbesk_attach");
  await pasteInto("Table.gltf");
  await respond("Skip");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts()).toContain("Nothing was added to “Table.gltf”.");
  expect(calls().count("arbesk_attach")).toBe(before);
});

test("an id a file File Browser can't show holds is refused in words", async () => {
  await copyFrom(ATT, ["Notes.plan"]);
  await openCollection("Untitled Collection");
  await pasteInto("Hook.gltf");
  expect(dialogHeading()).toBeUndefined();
  expect(toasts()).toEqual(["This asset already has a file “plan” that File Browser can't show. Nothing was added to “Hook.gltf”."]);
  await closeOverlays();
});

test("a drop of two STLs on an asset records two ids; on the collection's empty space it still copies in", async () => {
  const pointAt = document.elementFromPoint;
  let target: Element | null = null;
  document.elementFromPoint = () => target;
  const drop = (window as unknown as { __mockDrop: (paths: string[], x: number, y: number) => void }).__mockDrop;
  try {
    await openCollection("Shapes");
    target = row("Box.3mf");
    drop([`${ATT}/Part.stl`, `${ATT}/Two.stl`], 10, 10);
    await settle(30);
    expect(dialogHeading()).toBe("Add to “Box.3mf”?");
    await respond("Add");
    await vi.waitFor(() => expect(emblemTitle("Box.3mf")).toBe("Has 2 other files"), { timeout: 2000 });
    expect(await idsOf("1~Shapes", "box-m~Box.3mf")).toEqual(["stl", "stl-2"]);
    // Anything but a file of this computer is said in words.
    target = row("Box.3mf");
    drop([`${ATT}/README`], 10, 10);
    await settle(30);
    expect(toasts()).toContain("“README” has no file type to record.");
    await closeOverlays();

    target = $(".view-overlay");
    drop([`${PRINTS}/lamp.glb`], 10, 10);
    await settle(30);
    expect(dialogHeading()).toBe("Copy to Arbesk?");
    await respond("Cancel");
    await settle(10);
  } finally {
    document.elementFromPoint = pointAt;
  }
});

test("another save and a failed upload change nothing", async () => {
  await openCollection("Untitled Collection");
  const before = await idsOf("2~Untitled%20Collection", "table~Table.gltf");
  for (const [fail, words] of [
    [() => arbeskMock().conflictNext(), "“Table.gltf” changed while the files were being added. Nothing was changed; try again."],
    // cleanError keeps what follows the step's own words.
    [() => arbeskMock().failUploadNext(), "The server answered 500."],
  ] as const) {
    fail();
    await copyFrom(ATT, ["Lid.blend"]);
    await openCollection("Untitled Collection");
    await pasteInto("Table.gltf");
    await respond("Add");
    await vi.waitFor(() => expect(toasts()).toContain(`Could not add to “Table.gltf”: ${words}`), { timeout: 2000 });
    expect(await idsOf("2~Untitled%20Collection", "table~Table.gltf")).toEqual(before);
    await closeOverlays();
  }
});

test("an expired session signs in again with the add words, then carries on", async () => {
  arbeskMock().expireOnce();
  await copyFrom(ATT, ["Lid.blend"]);
  await openCollection("Untitled Collection");
  await pasteInto("Table.gltf");
  await respond("Add");
  await vi.waitFor(() => expect(dialogHeading()).toBe("Sign In to Arbesk"), { timeout: 2000 });
  await respond("Open Browser");
  await vi.waitFor(() => expect(emblemTitle("Table.gltf")).toBe("Has 4 other files"), { timeout: 2000 });
  expect(toasts().filter((t) => t.startsWith("Could not add"))).toEqual([]);
  expect(names()).toContain("Table.gltf");
});

/** The Files… page of `asset` in the open collection. */
async function openFiles(asset: string): Promise<void> {
  await click(row(asset));
  await key("i", { mod: true });
  await settle(10);
  await respond("Files…");
  await settle(20);
}

const filesRows = () => $$(".files-dialog .files-row .action-row-title").map((el) => el.textContent);

test("Remove… asks first, waits in the dialog until done and redraws the Files… page", async () => {
  await openCollection("Untitled Collection");
  await openFiles("Table.gltf");
  expect($(".files-dialog .files-hint").textContent).toBe("Paste or drop files onto the asset to add them.");
  expect(filesRows()).toEqual(["Table.gltf", "Part.stl", "Bracket.gcode", "Part.stl", "Lid.blend"]);
  // The model can't be removed.
  await click($$(".files-dialog .files-row-menu")[0]);
  expect(menuLabels()).toEqual(["Open With…", "Copy"]);
  await key("Escape");
  await click($$(".files-dialog .files-row-menu")[2]);
  expect(menuLabels()).toEqual(["Open With…", "Copy", "Remove…"]);
  await click($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Remove…")!);
  await settle(10);
  expect(dialogHeading()).toBe("Remove “Bracket.gcode” From “Table.gltf”?");
  expect(dialogTexts()).toEqual(["Arbesk keeps it in the asset's earlier versions."]);
  expect(dialogButtons()).toEqual(["Cancel", "Remove"]);
  // Enter is Cancel.
  await key("Enter");
  await settle(10);
  expect(dialogHeading()).toBe("Files of “Table.gltf”");

  await click($$(".files-dialog .files-row-menu")[2]);
  await click($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Remove…")!);
  await settle(10);
  calls().hold("arbesk_detach");
  await respond("Remove");
  await settle(10);
  const button = (label: string) => $$(".dialog-layer:last-child .dialog-responses button").find((b) => b.textContent === label)!;
  expect(button("Remove").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(false);
  calls().release("arbesk_detach");
  await vi.waitFor(() => expect(dialogHeading()).toBe("Files of “Table.gltf”"), { timeout: 2000 });
  expect(filesRows()).toEqual(["Table.gltf", "Part.stl", "Part.stl", "Lid.blend"]);
  await closeOverlays();
  await vi.waitFor(() => expect(emblemTitle("Table.gltf")).toBe("Has 3 other files"), { timeout: 2000 });
});

test("Remove… without a session says so in the dialog", async () => {
  arbeskMock().expireOnce();
  await openCollection("Untitled Collection");
  await openFiles("Table.gltf");
  await click($$(".files-dialog .files-row-menu")[1]);
  await click($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Remove…")!);
  await settle(10);
  await respond("Remove");
  await settle(20);
  expect(dialogHeading()).toBe("Remove “Part.stl” From “Table.gltf”?");
  expect($(".dialog-layer:last-child .field-message").textContent).toBe("Sign in to Arbesk again to remove files from it.");
  await closeOverlays();
  // The account now reads as needing a sign-in, as after an attach.
  await openOnlineAccounts();
  expect(accountGroups()[0].rows).toEqual(["Arbesk · me@example.com · Sign in again"]);
  await closeOverlays();
  arbeskMock().signedIn = true;
});

test("Remove… of a file replaced since the page showed it removes nothing", async () => {
  await openCollection("Untitled Collection");
  await openFiles("Table.gltf");
  const before = await idsOf("2~Untitled%20Collection", "table~Table.gltf");
  // Another save puts other bytes under the row's id.
  arbeskMock().changeFile("2", "table", "stl");
  await click($$(".files-dialog .files-row-menu")[1]);
  await click($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Remove…")!);
  await settle(10);
  await respond("Remove");
  await settle(20);
  expect($(".dialog-layer:last-child .field-message").textContent).toBe("This file was replaced in the asset since it was shown. Nothing was removed.");
  expect(await idsOf("2~Untitled%20Collection", "table~Table.gltf")).toEqual(before);
  await closeOverlays();
});
