// Performance plan, phase 5: a copy of several Arbesk assets out builds up
// to four items ahead of the one being placed (`arbesk_prefetch`, into the
// app cache), while placing stays one at a time and in order, with the
// same clash questions. Cancel stops the builds ahead and the placing, and
// leaves nothing extra in the folder; a build ahead that failed is told at
// its item's turn, in the words a copy always gave; a Skip places nothing.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { ARBESK_MOCK_ADDRESS, invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, closeOverlays, dialogHeading, goTo, installFakeCanvas, key, names, respond, settle, toasts } from "./app";

type Build = { path: string; ahead: boolean; outcome: string };
type MockArbesk = {
  addAsset(token: string, id: string, name: string, files: Array<{ id: string; format: string; name: string; bytes: number }>): void;
  holdBuilds(hold: boolean): void;
  releaseBuilds(which?: (path: string, ahead: boolean) => boolean): void;
  failBuilds(path: string, error: string): void;
  failBuildOnce(path: string, error: string): void;
  builds(): Build[];
  buildsRunning(): number;
  buildsPeak(): number;
  placements(): string[];
  placingPeak(): number;
  resetBuilds(): void;
};
const mockArbesk = () => (globalThis as unknown as { __mockArbesk: MockArbesk }).__mockArbesk;
type MockCalls = { argsOf(cmd: string): Array<Record<string, unknown>>; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;

const HOME = "/Users/me";
const SHAPES = `arbesk://${ARBESK_MOCK_ADDRESS}/1~Shapes`;
const SIX = [0, 1, 2, 3, 4, 5].map((n) => `Ahead${n}.gltf`);
const pathOf = (name: string) => `${SHAPES}/${name.replace(".gltf", "").toLowerCase()}~${name}`;
const nameOf = (path: string) => decodeURIComponent(path.slice(path.lastIndexOf("~") + 1));
const GATEWAY = "Arbesk's gateway could not be reached.";

const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;
const box = (title: string) =>
  $$(".files-dialog .choice-row").find((el) => el.querySelector(".action-row-title")?.textContent === title)!.querySelector<HTMLInputElement>("input")!;

beforeAll(async () => {
  installFakeCanvas();
  SIX.forEach((name, n) => mockArbesk().addAsset("1", `ahead${n}`, name.replace(".gltf", ""), []));
  for (const n of [1, 2]) mockArbesk().addAsset("1", `meshy${n}`, `Meshy${n}`, [{ id: "stl", format: "stl", name: `Meshy${n}.stl`, bytes: 100 }]);
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  for (const n of [1, 2, 3, 4, 5, 6]) await invoke("make_dir", { path: `${HOME}/Ahead${n}` });
  await invoke("create_file", { path: `${HOME}/Ahead5/Ahead2.gltf` });
  await boot();
  await key("1", { mod: true });
});

afterEach(async () => {
  mockArbesk().resetBuilds();
  calls().reset();
  await settle(10);
  await closeOverlays();
});

async function copyFromShapes(picks: string[]): Promise<void> {
  await goTo(SHAPES);
  await settle(20);
  await click(row(picks[0]));
  for (const name of picks.slice(1)) await click(row(name), { metaKey: true });
  await key("c", { mod: true });
}

/** Copy the six assets and paste them into `dir`, their builds held. */
async function pasteSixHeld(dir: string): Promise<void> {
  await copyFromShapes(SIX);
  mockArbesk().resetBuilds();
  mockArbesk().holdBuilds(true);
  await goTo(`${HOME}/${dir}`);
  await key("v", { mod: true });
  // The first being placed, the next four built ahead.
  await vi.waitFor(() => expect(mockArbesk().buildsRunning()).toBe(5), { timeout: 2000 });
}

const built = () => mockArbesk().builds().map((b) => [nameOf(b.path), b.ahead, b.outcome] as const);

test("six assets: four built ahead of the one placed, placed one at a time in order", async () => {
  await pasteSixHeld("Ahead1");
  await settle(10);
  expect(mockArbesk().placements()).toEqual([pathOf(SIX[0])]);
  expect(built().map(([name, ahead]) => [name, ahead]).sort()).toEqual([
    [SIX[0], false], [SIX[1], true], [SIX[2], true], [SIX[3], true], [SIX[4], true],
  ]);
  // The builds ahead land; nothing is placed while the first is still built.
  mockArbesk().releaseBuilds((_, ahead) => ahead);
  await settle(20);
  expect(names()).toEqual([]);
  expect(mockArbesk().placements()).toHaveLength(1);
  expect(built().filter(([name]) => name === SIX[5])).toEqual([]);
  mockArbesk().holdBuilds(false);
  await vi.waitFor(() => expect(names().sort()).toEqual(SIX), { timeout: 3000 });
  expect(mockArbesk().placements()).toEqual(SIX.map(pathOf));
  expect(mockArbesk().placingPeak()).toBe(1);
  expect(mockArbesk().buildsPeak()).toBeGreaterThan(1);
  // Each asset built once: the copy took what was built ahead.
  expect(built().map(([name]) => name).sort()).toEqual(SIX);
  expect(built().every(([, , outcome]) => outcome === "built")).toBe(true);
  expect(built().filter(([, ahead]) => !ahead).map(([name]) => name)).toEqual([SIX[0]]);
  expect(toasts().some((t) => t.startsWith("Could not"))).toBe(false);
});

test("Cancel stops the builds ahead and the placing; nothing lands", async () => {
  await pasteSixHeld("Ahead2");
  await click($("#btn-operations"));
  await settle(5);
  await click($(".operation-row .operation-cancel"));
  await vi.waitFor(() => expect(mockArbesk().buildsRunning()).toBe(0), { timeout: 2000 });
  await settle(20);
  expect(built().every(([, , outcome]) => outcome === "Cancelled.")).toBe(true);
  const cancelled = calls().argsOf("cancel_arbesk_parts").flatMap((a) => a.tickets as number[]);
  const ahead = calls().argsOf("arbesk_prefetch").map((a) => a.ticket as number);
  expect(ahead).toHaveLength(4);
  expect(ahead.every((ticket) => cancelled.includes(ticket))).toBe(true);
  mockArbesk().holdBuilds(false);
  await settle(40);
  expect(names()).toEqual([]);
  expect(built()).toHaveLength(5);
  expect(mockArbesk().placements()).toEqual([pathOf(SIX[0])]);
  expect($$(".operation-row")).toHaveLength(0);
  expect(toasts().some((t) => t.startsWith("Could not"))).toBe(false);
});

test("a build ahead that fails is told at its item's turn, in the copy's words", async () => {
  await copyFromShapes(SIX);
  mockArbesk().resetBuilds();
  mockArbesk().holdBuilds(true);
  mockArbesk().failBuilds(pathOf(SIX[3]), GATEWAY);
  await goTo(`${HOME}/Ahead3`);
  await key("v", { mod: true });
  await vi.waitFor(() => expect(mockArbesk().buildsRunning()).toBe(5), { timeout: 2000 });
  mockArbesk().releaseBuilds((_, ahead) => ahead);
  await vi.waitFor(() => expect(built()).toContainEqual([SIX[3], true, GATEWAY]), { timeout: 2000 });
  await settle(20);
  // Not yet: the first item is still being placed.
  expect(toasts().some((t) => t.startsWith("Could not"))).toBe(false);
  expect(names()).toEqual([]);
  mockArbesk().holdBuilds(false);
  await vi.waitFor(() => expect(names().sort()).toEqual(SIX.filter((name) => name !== SIX[3])), { timeout: 3000 });
  await vi.waitFor(() => expect(toasts()).toContain(`Could not copy “${SIX[3]}”: ${GATEWAY}`), { timeout: 2000 });
  // Its turn came: the copy built it itself, and told its own failure.
  expect(built().filter(([name]) => name === SIX[3])).toEqual([[SIX[3], true, GATEWAY], [SIX[3], false, GATEWAY]]);
  expect(mockArbesk().placements()).toEqual(SIX.map(pathOf));
});

test("a build ahead that fails once is built again by the copy, which lands it", async () => {
  await copyFromShapes(SIX);
  mockArbesk().resetBuilds();
  mockArbesk().failBuildOnce(pathOf(SIX[2]), GATEWAY);
  await goTo(`${HOME}/Ahead4`);
  await key("v", { mod: true });
  await vi.waitFor(() => expect(names().sort()).toEqual(SIX), { timeout: 3000 });
  await settle(20);
  expect(built().filter(([name]) => name === SIX[2])).toEqual([[SIX[2], true, GATEWAY], [SIX[2], false, "built"]]);
  expect(toasts().some((t) => t.includes(SIX[2]))).toBe(false);
});

test("Skip on a clash places nothing of an item built ahead", async () => {
  await copyFromShapes(SIX);
  mockArbesk().resetBuilds();
  await goTo(`${HOME}/Ahead5`);
  await key("v", { mod: true });
  await vi.waitFor(() => expect(dialogHeading()).toBe(`Replace file “${SIX[2]}”?`), { timeout: 3000 });
  await vi.waitFor(() => expect(built()).toContainEqual([SIX[2], true, "built"]), { timeout: 2000 });
  // Two placed so far, in order, the clash asked before the third.
  expect(mockArbesk().placements()).toEqual(SIX.slice(0, 2).map(pathOf));
  await respond("Skip");
  await vi.waitFor(() => expect(names().sort()).toEqual(SIX), { timeout: 3000 });
  expect(mockArbesk().placements()).toEqual(SIX.filter((name) => name !== SIX[2]).map(pathOf));
  // The empty file that was there stays: nothing went to the Trash.
  const listed = await invoke<Array<{ name: string; size: number }>>("list_dir", { path: `${HOME}/Ahead5`, showHidden: false });
  expect(listed.find((entry) => entry.name === SIX[2])?.size).toBe(0);
});

test("an asset's other files are built ahead by their pinned paths", async () => {
  await copyFromShapes(["Meshy1.gltf", "Meshy2.gltf"]);
  mockArbesk().resetBuilds();
  await goTo(`${HOME}/Ahead6`);
  await key("v", { mod: true });
  await settle(30);
  expect(dialogHeading()).toBe("Copy Which Files?");
  await click(box("STL (2 assets)"));
  await respond("Copy");
  await vi.waitFor(() => expect(names().sort()).toEqual(["Meshy1.gltf", "Meshy1.stl", "Meshy2.gltf", "Meshy2.stl"]), { timeout: 3000 });
  const ahead = calls().argsOf("arbesk_prefetch").map((a) => a.path as string);
  // The three after the first, as the copy places them: each file by the CID the dialog showed.
  expect(ahead).toEqual(mockArbesk().placements().slice(1));
  expect(ahead.filter((path) => /\/stl@[^/~]+~Meshy\d\.stl$/.test(path))).toHaveLength(2);
});
