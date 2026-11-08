// Space on a 3D model (glTF, GLB, STL, 3MF) shows it in the preview: the real
// app and loader against the mock, with only the WebGL drawing (showModel)
// replaced, since jsdom has none. Closing the preview releases the view.
import { beforeAll, beforeEach, expect, test, vi } from "vitest";
import { Group, type Object3D } from "three";
import { $, boot, click, goTo, hit, key, settle } from "./app";
import { SAMPLE_TRIANGLES } from "../dev/mock-tauri/models";

const viewer = vi.hoisted(() => ({
  showModel: vi.fn(),
  loadModel: vi.fn(),
  real: { showModel: null as unknown as (host: HTMLElement, object: unknown) => unknown },
}));

vi.mock("../src/model-viewer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/model-viewer")>();
  viewer.real.showModel = actual.showModel;
  viewer.loadModel.mockImplementation(actual.loadModel);
  return { ...actual, loadModel: viewer.loadModel, showModel: viewer.showModel };
});

const PRINTS = "/Users/me/Documents/Projects/Prints";
type MockCalls = { count(cmd: string): number; failNext(cmd: string, error: string): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;

/** The fake view: a canvas in the preview, and a dispose to count. */
function fakeView(host: HTMLElement) {
  const canvas = document.createElement("canvas");
  canvas.className = "preview-model";
  canvas.tabIndex = -1;
  host.appendChild(canvas);
  return { dispose: vi.fn(() => canvas.remove()) };
}

function triangles(object: Object3D): number {
  let count = 0;
  object.traverse((node) => {
    const geometry = (node as { geometry?: { index: { count: number } | null; getAttribute(n: string): { count: number } } }).geometry;
    if (geometry) count += (geometry.index?.count ?? geometry.getAttribute("position").count) / 3;
  });
  return count;
}

/** Wait for `ok`, checking every 10 ms: the viewer (three.js) loads on the
 *  first preview, which takes a while under the coverage run. */
async function until(ok: () => boolean, ms = 10_000): Promise<void> {
  const end = Date.now() + ms;
  while (!ok()) {
    if (Date.now() > end) throw new Error("timed out waiting for the preview");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

async function preview(name: string): Promise<void> {
  await click(hit(name));
  await key(" ");
  await until(() => $("#preview-body").childElementCount > 0);
  await settle(10);
}

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
  await goTo(PRINTS);
});

beforeEach(() => {
  viewer.showModel.mockReset();
  viewer.showModel.mockImplementation(fakeView);
});

test.each(["bracket.stl", "bracket-binary.stl", "box.3mf", "lamp.glb"])("Space shows %s as a 3D model", async (name) => {
  await preview(name);
  expect($("#preview-layer").hidden).toBe(false);
  expect($("#preview-name").textContent).toBe(name);
  expect($("#preview-body canvas.preview-model")).toBeTruthy();
  const [host, model] = viewer.showModel.mock.calls[0];
  expect(host).toBe($("#preview-body"));
  expect(triangles(model)).toBe(SAMPLE_TRIANGLES);

  const view = viewer.showModel.mock.results[0].value as { dispose: ReturnType<typeof vi.fn> };
  await key("Escape");
  await settle(10);
  expect($("#preview-layer").hidden).toBe(true);
  expect(view.dispose).toHaveBeenCalledTimes(1);
});

test.each(["Escape", " "])("%j on the model being turned closes the preview", async (name) => {
  await preview("bracket.stl");
  const canvas = $<HTMLCanvasElement>("#preview-body canvas.preview-model");
  // Dragging focuses the canvas, so the view no longer sees the keys.
  canvas.focus();
  await key(name, { target: canvas });
  await settle(10);
  expect($("#preview-layer").hidden).toBe(true);
  expect(document.activeElement).toBe($("#view"));
});

test("a .gltf reads the companion .bin it names next to it, and nothing else", async () => {
  viewer.loadModel.mockResolvedValueOnce(new Group());
  const before = calls().count("preview_bytes");
  await preview("lamp.gltf");
  expect(calls().count("preview_bytes") - before).toBe(2);
  const [format, , companions] = viewer.loadModel.mock.lastCall!;
  expect(format).toBe("gltf");
  expect([...(companions as Map<string, ArrayBuffer>).entries()].map(([uri, data]) => [uri, data.byteLength])).toEqual([
    ["lamp.bin", 168],
  ]);
  expect(viewer.showModel).toHaveBeenCalledTimes(1);
  await key("Escape");
});

test("a model over the size cap says it is too large, without reading it", async () => {
  await preview("scan.stl");
  expect($("#preview-body .preview-empty").textContent).toBe("This file is too large to preview.");
  expect(viewer.showModel).not.toHaveBeenCalled();
  await key("Escape");
});

test("without WebGL the preview says there is none", async () => {
  viewer.showModel.mockImplementation((host: HTMLElement, object: unknown) => viewer.real.showModel(host, object));
  await preview("bracket.stl");
  expect($("#preview-body .preview-empty").textContent).toBe("No preview available.");
  expect($("#preview-body canvas")).toBeNull();
  await key("Escape");
});

test("a model that can't be read says there is no preview", async () => {
  calls().failNext("preview_bytes", "Failed to read file: Input/output error (os error 5)");
  await preview("box.3mf");
  expect($("#preview-body .preview-empty").textContent).toBe("No preview available.");
  await key("Escape");
});

test("a model closed before it finished loading is never shown", async () => {
  let finish!: (object: Object3D) => void;
  viewer.loadModel.mockImplementationOnce(() => new Promise<Object3D>((resolve) => (finish = resolve)));
  await click(hit("lamp.glb"));
  await key(" ");
  await settle(20);
  await key("Escape");
  await settle(5);
  finish(new Group());
  await settle(10);
  expect(viewer.showModel).not.toHaveBeenCalled();
  expect($("#preview-layer").hidden).toBe(true);
});
