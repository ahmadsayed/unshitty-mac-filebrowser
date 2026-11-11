// Performance plan, phase 4: a paste or drop into an Arbesk collection takes
// the collection's names once per batch (the view's when it is on screen,
// else one listing) instead of asking path_exists per file; Replace, Skip
// and Keep Both ask exactly as before.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, dialogHeading, goTo, installFakeCanvas, key, names, open, respond, settle } from "./app";

const UP = "/Users/me/PasteNames";
const mockCalls = () =>
  (globalThis as unknown as { __mockCalls: { count(cmd: string): number; argsOf(cmd: string): Array<Record<string, unknown>>; reset(): void } }).__mockCalls;
const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;

/** path_exists calls asked of Arbesk since the last reset. */
const arbeskExistsCalls = () => mockCalls().argsOf("path_exists").filter((a) => String(a.path).startsWith("arbesk://")).length;
/** list_dir calls of `path` since the last reset. */
const listings = (path: string) => mockCalls().argsOf("list_dir").filter((a) => a.path === path).length;

async function collectionPath(name: string): Promise<string> {
  const { ARBESK_MOCK_ADDRESS: address } = await import("../dev/mock-tauri/core");
  const collections = await invoke<Array<{ name: string; path: string }>>("list_dir", { path: `arbesk://${address}`, showHidden: false });
  return collections.find((c) => c.name === name)!.path;
}

async function listed(path: string): Promise<string[]> {
  return (await invoke<Array<{ name: string }>>("list_dir", { path, showHidden: false })).map((a) => a.name);
}

async function openCollection(name: string): Promise<void> {
  await click($$(".sidebar-row").find((r) => r.textContent?.trim() === "Network")!);
  await settle(20);
  await open("Arbesk");
  await settle(20);
  await open(name);
  await settle(20);
}

/** Copy `picks` from the local folder. */
async function copyFromUp(...picks: string[]): Promise<void> {
  await goTo(UP);
  for (const [i, name] of picks.entries()) await click(row(name), { metaKey: i > 0 });
  await key("c", { mod: true });
}

beforeAll(async () => {
  installFakeCanvas();
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  await invoke("make_dir", { path: UP });
  for (const name of ["Cube.glb", "Gear.glb", "Nut.glb", "Bolt.3mf", "Peg.gltf", "Peg.glb", "Pin.glb", "Cam.glb", "Rod.glb"]) {
    await invoke("create_file", { path: `${UP}/${name}` });
  }
  await boot();
  await key("1", { mod: true });
});

test("a paste of several files into the collection on screen asks no name of Arbesk", async () => {
  await copyFromUp("Gear.glb", "Nut.glb", "Bolt.3mf");
  await openCollection("Untitled Collection");
  const collection = await collectionPath("Untitled Collection");
  mockCalls().reset();
  await key("v", { mod: true });
  await settle(10);
  expect(dialogHeading()).toBe("Copy to Arbesk?");
  await respond("Copy");
  await vi.waitFor(() => expect(names()).toEqual(expect.arrayContaining(["Gear.gltf", "Nut.gltf", "Bolt.3mf"])), { timeout: 2000 });
  expect(arbeskExistsCalls()).toBe(0);
  expect(mockCalls().count("copy_entry")).toBe(3);
  expect(await listed(collection)).toEqual(expect.arrayContaining(["Gear.gltf", "Nut.gltf", "Bolt.3mf"]));
});

test("a drop of several files on a collection not on screen asks no name of Arbesk", async () => {
  const pointAt = document.elementFromPoint;
  let target: Element | null = null;
  document.elementFromPoint = () => target;
  try {
    await openCollection("Shapes");
    const shapes = await collectionPath("Shapes");
    // Shapes in the sidebar, where a drop from another app can reach it.
    await key("d", { mod: true });
    await settle(10);
    await goTo(UP);
    target = $$(".sidebar-row").find((r) => r.dataset.location === shapes)!;
    mockCalls().reset();
    const drop = (window as unknown as { __mockDrop: (paths: string[], x: number, y: number) => void }).__mockDrop;
    drop([`${UP}/Pin.glb`, `${UP}/Cam.glb`, `${UP}/Rod.glb`], 10, 10);
    await settle(30);
    expect(dialogHeading()).toBe("Copy to Arbesk?");
    await respond("Copy");
    await vi.waitFor(async () => expect(await listed(shapes)).toEqual(expect.arrayContaining(["Pin.gltf", "Cam.gltf", "Rod.gltf"])), { timeout: 2000 });
    await settle(20);
    // The test's own listings above are not the paste's: count before them.
    expect(arbeskExistsCalls()).toBe(0);
    expect(mockCalls().count("copy_entry")).toBe(3);
  } finally {
    document.elementFromPoint = pointAt;
  }
});

test("a drop's one listing is all it asks, however many files", async () => {
  const pointAt = document.elementFromPoint;
  let target: Element | null = null;
  document.elementFromPoint = () => target;
  try {
    const shapes = await collectionPath("Shapes");
    await goTo(UP);
    target = $$(".sidebar-row").find((r) => r.dataset.location === shapes)!;
    for (const name of ["A.glb", "B.glb", "C.glb", "D.glb", "E.glb"]) await invoke("create_file", { path: `${UP}/${name}` });
    mockCalls().reset();
    const drop = (window as unknown as { __mockDrop: (paths: string[], x: number, y: number) => void }).__mockDrop;
    drop(["A", "B", "C", "D", "E"].map((n) => `${UP}/${n}.glb`), 10, 10);
    await settle(30);
    await respond("Copy");
    await vi.waitFor(() => expect(mockCalls().count("copy_entry")).toBe(5), { timeout: 2000 });
    await settle(20);
    expect(listings(shapes)).toBe(1);
    expect(arbeskExistsCalls()).toBe(0);
  } finally {
    document.elementFromPoint = pointAt;
  }
});

test("a name already there, or landed earlier in the same paste, still asks Replace, Skip or Keep Both", async () => {
  // Peg.gltf and Peg.glb both land as Peg.gltf: the second finds the first.
  await copyFromUp("Cube.glb", "Peg.gltf", "Peg.glb");
  await openCollection("Untitled Collection");
  const collection = await collectionPath("Untitled Collection");
  const before = await listed(collection);
  expect(before).toContain("Cube.gltf");
  mockCalls().reset();
  await key("v", { mod: true });
  await settle(10);
  await respond("Copy");
  await settle(20);
  expect(dialogHeading()).toBe("Replace file “Cube.gltf”?");
  await respond("Skip");
  await settle(30);
  await vi.waitFor(() => expect(dialogHeading()).toBe("Replace file “Peg.gltf”?"), { timeout: 2000 });
  await respond("Keep Both");
  await vi.waitFor(() => expect(names()).toContain("Peg (copy).gltf"), { timeout: 2000 });
  const after = await listed(collection);
  expect(after.filter((n) => n === "Cube.gltf")).toHaveLength(1);
  expect(after).toEqual(expect.arrayContaining(["Peg.gltf", "Peg (copy).gltf"]));
  expect(arbeskExistsCalls()).toBe(0);
  expect(mockCalls().count("copy_entry")).toBe(2);
});
