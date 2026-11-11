// An Arbesk scene's parts load together (performance plan, phase 3): the
// preview asks for a few parts at once, each under a cancel ticket, and
// places them by the scene's order whatever order they answer in; the
// preview's byte limit is shared by the calls in flight; a closed preview
// cancels its calls; one failed part leaves the others. The real app
// against the mock, with only the WebGL drawing (showModel) replaced.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import type { Group } from "three";
import { invoke } from "../dev/mock-tauri/core";
import { $, boot, click, closeOverlays, goTo, hit, key, settle } from "./app";
import { PARTS_AT_ONCE } from "../src/arbesk-parts";

const viewer = vi.hoisted(() => ({ showModel: vi.fn() }));

vi.mock("../src/model-viewer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/model-viewer")>();
  return { ...actual, showModel: viewer.showModel };
});

type MockCalls = { count(cmd: string): number; argsOf(cmd: string): Record<string, unknown>[]; failNext(cmd: string, error: string): void; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
type Held = { key: string; limit: number; ticket?: number };
type ScenePart = { key: string; format: "glb" | "3mf"; bytes?: number; copies: Array<{ x: number }> };
type MockArbesk = {
  addScene(token: string, id: string, name: string, parts: ScenePart[]): () => void;
  holdParts(hold: boolean): void;
  releasePart(key: string): void;
  heldParts(): Held[];
  cancelledParts(): number[];
};
const mock = () => (globalThis as unknown as { __mockArbesk: MockArbesk }).__mockArbesk;

const UNTITLED = "arbesk://0xabababababababababababababababababababab/2~Untitled%20Collection";
/** The preview's limit: the model and every part together. */
const LIMIT = 256 * 1024 * 1024;
const MIB = 1024 * 1024;

function fakeView(host: HTMLElement) {
  const canvas = document.createElement("canvas");
  canvas.className = "preview-model";
  host.appendChild(canvas);
  return { dispose: vi.fn(() => canvas.remove()) };
}

async function until(ok: () => boolean, ms = 10_000): Promise<void> {
  const end = Date.now() + ms;
  while (!ok()) {
    if (Date.now() > end) throw new Error("timed out waiting");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

/** Space on `name`, without waiting for what it shows. */
async function open(name: string): Promise<void> {
  await click(hit(name));
  await key(" ");
}

const copiesShown = () => {
  const [, scene] = viewer.showModel.mock.calls[0] as [HTMLElement, Group];
  return scene.children.map((copy) => copy.position.x);
};
const heldLimits = () => mock().heldParts().reduce((sum, call) => sum + call.limit, 0);

/** Row: six parts, glb and 3mf, each at its own x (the second twice). */
const ROW: ScenePart[] = [
  { key: "row-0", format: "glb", copies: [{ x: 0 }] },
  { key: "row-1", format: "3mf", copies: [{ x: 100 }, { x: 150 }] },
  { key: "row-2", format: "glb", copies: [{ x: 200 }] },
  { key: "row-3", format: "3mf", copies: [{ x: 300 }] },
  { key: "row-4", format: "glb", copies: [{ x: 400 }] },
  { key: "row-5", format: "glb", copies: [{ x: 500 }] },
];

beforeAll(async () => {
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  mock().addScene("2", "row", "Row", ROW);
  // Big: a part of 200 MiB among small ones, which loads only when asked
  // for alone with all that is left.
  mock().addScene("2", "big", "Bigger", [
    { key: "big-0", format: "glb", copies: [{ x: 0 }] },
    { key: "big-1", format: "glb", bytes: 200 * MIB, copies: [{ x: 10 }] },
    { key: "big-2", format: "glb", copies: [{ x: 20 }] },
  ]);
  await boot();
});

afterEach(async () => {
  mock().holdParts(false);
  await settle(10);
  await closeOverlays();
  if (!$("#preview-layer").hidden) await key("Escape");
  calls().reset();
  viewer.showModel.mockReset();
  viewer.showModel.mockImplementation(fakeView);
});

viewer.showModel.mockImplementation(fakeView);

test("a scene's parts are asked for together and keep their order and places when they answer out of order", async () => {
  await goTo(UNTITLED);
  mock().holdParts(true);
  await open("Row.gltf");
  await until(() => mock().heldParts().length === PARTS_AT_ONCE);
  expect(mock().heldParts().map((call) => call.key)).toEqual(["row-0", "row-1", "row-2", "row-3"]);
  // Each call under a ticket of its own; together within the preview's limit.
  expect(new Set(mock().heldParts().map((call) => call.ticket)).size).toBe(PARTS_AT_ONCE);
  expect(heldLimits()).toBeLessThanOrEqual(LIMIT);
  // The last answers first; a freed slot asks for the next part.
  for (const part of ["row-3", "row-2", "row-5", "row-4", "row-1", "row-0"]) {
    await until(() => mock().heldParts().some((call) => call.key === part));
    expect(heldLimits()).toBeLessThanOrEqual(LIMIT);
    mock().releasePart(part);
    await settle(4);
  }
  await until(() => viewer.showModel.mock.calls.length > 0);
  expect(copiesShown()).toEqual([0, 100, 150, 200, 300, 400, 500]);
  expect(calls().argsOf("arbesk_part").map((args) => args.key)).toEqual(ROW.map((part) => part.key));
});

test("the preview's byte limit holds with calls in flight: a part too large for its share is asked again alone", async () => {
  await goTo(UNTITLED);
  await open("Bigger.gltf");
  await until(() => viewer.showModel.mock.calls.length > 0);
  expect(copiesShown()).toEqual([0, 10, 20]);
  const asked = calls().argsOf("arbesk_part").map((args) => [args.key, args.limit as number] as const);
  // Shared three ways first, never more than the limit together.
  expect(asked.slice(0, 3).map(([part]) => part)).toEqual(["big-0", "big-1", "big-2"]);
  expect(asked.slice(0, 3).reduce((sum, [, limit]) => sum + limit, 0)).toBeLessThanOrEqual(LIMIT);
  const big = asked.filter(([part]) => part === "big-1").map(([, limit]) => limit);
  expect(big).toHaveLength(2);
  expect(big[0]).toBeLessThan(200 * MIB);
  expect(big[1]).toBeGreaterThanOrEqual(200 * MIB);
});

test("a closed preview cancels its part calls and asks for no more", async () => {
  await goTo(UNTITLED);
  mock().holdParts(true);
  await open("Row.gltf");
  await until(() => mock().heldParts().length === PARTS_AT_ONCE);
  const tickets = mock().heldParts().map((call) => call.ticket);
  await key("Escape");
  await settle(10);
  expect(mock().cancelledParts()).toEqual(expect.arrayContaining(tickets));
  // Cancelled, the held calls answered; none more was asked for.
  expect(mock().heldParts()).toEqual([]);
  expect(calls().count("arbesk_part")).toBe(PARTS_AT_ONCE);
  expect(viewer.showModel).not.toHaveBeenCalled();
});

test("a part that fails leaves the others in their places", async () => {
  await goTo(UNTITLED);
  calls().failNext("arbesk_part", "Arbesk's storage could not be reached: Bad gateway");
  await open("Row.gltf");
  await until(() => viewer.showModel.mock.calls.length > 0);
  expect(copiesShown()).toEqual([100, 150, 200, 300, 400, 500]);
});
