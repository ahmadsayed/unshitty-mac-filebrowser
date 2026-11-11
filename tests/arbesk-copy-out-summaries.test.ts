// Performance plan, phase 4: a copy out of Arbesk plans from what the
// listing already said of each asset's other files. An asset whose summary
// says it has none is never asked for its list; the several-asset dialog
// is built from the summaries' formats (the rows the full lists give) while
// the lists of the assets with files are read, and once the choice is made
// only the assets it takes a format of are read. What is copied is what
// the lists say, as before. A summary stands in only while its listing is
// current: a write of this app into the collection, or the window left and
// back, makes the clipboard's unknown again.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { ARBESK_MOCK_ADDRESS, invoke } from "../dev/mock-tauri/core";
import type { Entry, RepresentationInfo } from "../src/api";
import { chooseFilesToCopy, chooseFormatsToCopy } from "../src/representation-dialogs";
import { formatCounts, formatsOfFiles, formatsOfSummary } from "../src/representations";
import { $$, boot, click, closeOverlays, dialogHeading, goTo, installFakeCanvas, key, names, respond, settle } from "./app";

type MockArbesk = { addAsset(token: string, id: string, name: string, files: Array<{ id: string; format: string; name: string; bytes: number }>): void };
const mockArbesk = () => (globalThis as unknown as { __mockArbesk: MockArbesk }).__mockArbesk;
type MockCalls = { count(cmd: string): number; hold(cmd: string): void; release(cmd: string): void; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;

const HOME = "/Users/me";
const SHAPES = `arbesk://${ARBESK_MOCK_ADDRESS}/1~Shapes`;
const UNTITLED = `arbesk://${ARBESK_MOCK_ADDRESS}/2~Untitled%20Collection`;
const FILES = `${HOME}/SumFiles`;
const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;
const checkTitles = () => $$(".files-dialog .choice-row .action-row-title").map((el) => el.textContent);
const box = (title: string) =>
  $$(".files-dialog .choice-row").find((el) => el.querySelector(".action-row-title")?.textContent === title)!.querySelector<HTMLInputElement>("input")!;

beforeAll(async () => {
  installFakeCanvas();
  for (const n of [1, 2, 3]) mockArbesk().addAsset("1", `plain${n}`, `Plain${n}`, []);
  for (const n of [1, 2, 3, 4]) mockArbesk().addAsset("1", `print${n}`, `Print${n}`, [{ id: "gcode", format: "gcode", name: `Print${n}.gcode`, bytes: 100 }]);
  mockArbesk().addAsset("1", "mesh", "Mesh", [{ id: "stl", format: "stl", name: "Mesh.stl", bytes: 100 }]);
  for (const [id, name] of [["late", "Late"], ["other", "Other"]]) mockArbesk().addAsset("1", id, name, []);
  // More formats than a listing's summary names (another client's list), away from Shapes' rows.
  mockArbesk().addAsset("2", "wide", "Wide", Array.from({ length: 33 }, (_, i) => ({ id: `f${i}`, format: `f${i}`, name: `Wide.f${i}`, bytes: 10 })));
  await invoke("make_dir", { path: FILES });
  for (const name of ["Late.stl", "Other.stl"]) await invoke("create_file", { path: `${FILES}/${name}` });
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  for (const dir of ["Sum1", "Sum2", "Sum3", "Sum4", "Sum5", "Sum6", "Sum7", "Sum8"]) await invoke("make_dir", { path: `${HOME}/${dir}` });
  await boot();
  await key("1", { mod: true });
});

afterEach(async () => {
  calls().reset();
  await settle(10);
  await closeOverlays();
});

async function copyFromShapes(picks: string[], from = SHAPES): Promise<void> {
  await goTo(from);
  await settle(20);
  await click(row(picks[0]));
  for (const name of picks.slice(1)) await click(row(name), { metaKey: true });
  await key("c", { mod: true });
}

test("a copy out of assets with no other files asks for no list and no question", async () => {
  await copyFromShapes(["Plain1.gltf", "Plain2.gltf", "Plain3.gltf", "Box.3mf"]);
  calls().reset();
  await goTo(`${HOME}/Sum1`);
  await key("v", { mod: true });
  await vi.waitFor(() => expect(names().sort()).toEqual(["Box.3mf", "Plain1.gltf", "Plain2.gltf", "Plain3.gltf"]), { timeout: 2000 });
  expect(dialogHeading()).toBeUndefined();
  expect(calls().count("arbesk_representations")).toBe(0);
});

test("only the assets with files are read; the question comes at once, from the summaries", async () => {
  await copyFromShapes(["Plain1.gltf", "Print1.gltf", "Mesh.gltf"]);
  calls().reset();
  calls().hold("arbesk_representations");
  await goTo(`${HOME}/Sum2`);
  await key("v", { mod: true });
  await settle(30);
  // Asked while the lists are still being read.
  expect(dialogHeading()).toBe("Copy Which Files?");
  expect(checkTitles()).toEqual(["All Files", "Models", "STL (1 asset)", "G-code (1 asset)"]);
  expect(calls().count("arbesk_representations")).toBe(2);
  await click(box("STL (1 asset)"));
  await respond("Copy");
  await settle(20);
  // Nothing is copied before the lists the choice needs have landed.
  expect(names()).toEqual([]);
  calls().release("arbesk_representations");
  await vi.waitFor(() => expect(names().sort()).toEqual(["Mesh.gltf", "Mesh.stl", "Plain1.gltf", "Print1.gltf"]), { timeout: 2000 });
  expect(calls().count("arbesk_representations")).toBe(2);
});

test("once the choice is made, assets it takes nothing from are not read", async () => {
  await copyFromShapes(["Print1.gltf", "Print2.gltf", "Print3.gltf", "Print4.gltf", "Mesh.gltf", "Box.gltf"]);
  calls().reset();
  calls().hold("arbesk_representations");
  await goTo(`${HOME}/Sum3`);
  await key("v", { mod: true });
  await settle(30);
  expect(checkTitles()).toEqual(["All Files", "Models", "STL (2 assets)", "G-code (4 assets)"]);
  // Four at a time, in the paste's order: Box, Mesh, Print1 and Print2; Print3 and Print4 wait their turn.
  expect(calls().count("arbesk_representations")).toBe(4);
  await click(box("STL (2 assets)"));
  await respond("Copy");
  await settle(20);
  calls().release("arbesk_representations");
  await vi.waitFor(() => expect(names()).toHaveLength(8), { timeout: 2000 });
  expect(names().filter((name) => !name.endsWith(".gltf")).sort()).toEqual(["Box.stl", "Mesh.stl"]);
  // Print1 and Print2 were read before the choice; Print3 and Print4 never.
  expect(calls().count("arbesk_representations")).toBe(4);
});

test("Models unchecked still leaves out every model, read or not", async () => {
  await copyFromShapes(["Plain1.gltf", "Print1.gltf", "Mesh.gltf"]);
  await goTo(`${HOME}/Sum4`);
  await key("v", { mod: true });
  await settle(30);
  await click(box("Models"));
  await click(box("STL (1 asset)"));
  await respond("Copy");
  await vi.waitFor(() => expect(names()).toEqual(["Mesh.stl"]), { timeout: 2000 });
});

test("a dialog built from the summaries offers the rows one built from the full lists does", async () => {
  const listed = await invoke<Entry[]>("list_dir", { path: SHAPES, showHidden: false });
  const assets = listed.filter((entry) => !entry.is_dir);
  const lists = await Promise.all(
    assets.map(async (entry) => ({ path: entry.path, name: entry.name, files: await invoke<RepresentationInfo[]>("arbesk_representations", { path: entry.path, version: null }) })),
  );
  const fromSummaries = assets.map((entry) => formatsOfSummary(entry.path, entry.name, entry.representations));
  expect(formatCounts(fromSummaries)).toEqual(formatCounts(lists.map(formatsOfFiles)));
  expect(formatCounts(fromSummaries).length).toBeGreaterThan(2);
  const rowsOf = async (open: () => Promise<unknown>) => {
    const answer = open();
    await settle(5);
    const rows = checkTitles();
    await respond("Cancel");
    expect(await answer).toBeNull();
    return rows;
  };
  const icon = (asset: { name: string }): Entry => ({ name: asset.name, path: asset.name, is_dir: false, size: 0, modified: 0, is_hidden: false, is_symlink: false });
  const fromLists = await rowsOf(() => chooseFilesToCopy(lists, icon));
  expect(await rowsOf(() => chooseFormatsToCopy(fromSummaries, icon))).toEqual(fromLists);
  expect(fromLists.slice(0, 2)).toEqual(["All Files", "Models"]);
});

test("a file this app attached after the Copy is still offered: the clipboard's summary no longer answers", async () => {
  const pointAt = document.elementFromPoint;
  let target: Element | null = null;
  document.elementFromPoint = () => target;
  try {
    await copyFromShapes(["Late.gltf"]);
    // Late.stl dropped on the asset: added by this app, the Copy still on the clipboard.
    target = row("Late.gltf");
    (window as unknown as { __mockDrop: (paths: string[], x: number, y: number) => void }).__mockDrop([`${FILES}/Late.stl`], 10, 10);
    await settle(30);
    expect(dialogHeading()).toBe("Add to “Late.gltf”?");
    await respond("Add");
    await vi.waitFor(() => expect(row("Late.gltf").querySelector(".emblem-files")).not.toBeNull(), { timeout: 2000 });
  } finally {
    document.elementFromPoint = pointAt;
  }
  calls().reset();
  await goTo(`${HOME}/Sum5`);
  await key("v", { mod: true });
  await vi.waitFor(() => expect(dialogHeading()).toBe("Copy Which Files of “Late.gltf”?"), { timeout: 2000 });
  expect(calls().count("arbesk_representations")).toBe(1);
  await click(box("Late.stl"));
  await respond("Copy");
  await vi.waitFor(() => expect(names().sort()).toEqual(["Late.gltf", "Late.stl"]), { timeout: 2000 });
});

test("a file another app attached is offered once the window has come back", async () => {
  await copyFromShapes(["Other.gltf", "Plain1.gltf"]);
  // Attached elsewhere (the Studio, besk), while this window was in the background.
  await invoke("arbesk_attach", { path: `${SHAPES}/other~Other.gltf`, files: [{ path: `${FILES}/Other.stl` }], op: "elsewhere" });
  window.dispatchEvent(new Event("focus"));
  await settle(20);
  calls().reset();
  await goTo(`${HOME}/Sum6`);
  await key("v", { mod: true });
  await vi.waitFor(() => expect(dialogHeading()).toBe("Copy Which Files?"), { timeout: 2000 });
  expect(checkTitles()).toEqual(["All Files", "Models", "STL (1 asset)"]);
  // Every list read first, as before phase 4.
  expect(calls().count("arbesk_representations")).toBe(2);
  await click(box("STL (1 asset)"));
  await respond("Copy");
  await vi.waitFor(() => expect(names().sort()).toEqual(["Other.gltf", "Other.stl", "Plain1.gltf"]), { timeout: 2000 });
});

test("Cancel stops the reads not yet started", async () => {
  await copyFromShapes(["Print1.gltf", "Print2.gltf", "Print3.gltf", "Print4.gltf", "Mesh.gltf", "Box.gltf"]);
  calls().reset();
  calls().hold("arbesk_representations");
  await goTo(`${HOME}/Sum7`);
  await key("v", { mod: true });
  await settle(30);
  expect(dialogHeading()).toBe("Copy Which Files?");
  expect(calls().count("arbesk_representations")).toBe(4);
  await respond("Cancel");
  calls().release("arbesk_representations");
  await settle(40);
  // Print3 and Print4 were never asked for.
  expect(calls().count("arbesk_representations")).toBe(4);
  expect(names()).toEqual([]);
  expect(dialogHeading()).toBeUndefined();
});

test("a summary that may leave formats out reads every list first, and the dialog offers every format", async () => {
  await copyFromShapes(["Wide.gltf", "Table.gltf"], UNTITLED);
  calls().reset();
  calls().hold("arbesk_representations");
  await goTo(`${HOME}/Sum8`);
  await key("v", { mod: true });
  await settle(30);
  // Not asked from the summaries: Wide's names only 32 of its 33 formats.
  expect(dialogHeading()).toBeUndefined();
  calls().release("arbesk_representations");
  await vi.waitFor(() => expect(dialogHeading()).toBe("Copy Which Files?"), { timeout: 2000 });
  expect(checkTitles().filter((title) => /^F\d+ \(1 asset\)$/.test(title))).toHaveLength(33);
  expect(calls().count("arbesk_representations")).toBe(2);
  await respond("Cancel");
});
