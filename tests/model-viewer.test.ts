// The 3D viewer's loading half (src/model-viewer.ts) reads real files: the
// mock's sample box as STL (ASCII and binary), 3MF, GLB and glTF. Drawing
// needs WebGL, which jsdom lacks, so showModel is checked to fail cleanly.
import { expect, test, vi } from "vitest";
import { Box3, Mesh, Quaternion, Vector3, type Object3D } from "three";
import { loadModel, modelControls, showModel, turned, turnForKey, type Turn } from "../src/model-viewer";
import {
  SAMPLE_TRIANGLES, sample3mf, sampleGlb, sampleGltf, sampleGltfBin, sampleGltfJson, sampleStlAscii, sampleStlBinary,
} from "../dev/mock-tauri/models";

/** The bytes copied into this realm's ArrayBuffer (jsdom's TextEncoder answers in another one). */
const buffer = (bytes: Uint8Array): ArrayBuffer => {
  const out = new Uint8Array(bytes.length);
  out.set(bytes);
  return out.buffer;
};

function triangles(object: Object3D): number {
  let count = 0;
  object.traverse((node) => {
    if (!(node instanceof Mesh)) return;
    const geometry = node.geometry;
    count += (geometry.index?.count ?? geometry.getAttribute("position").count) / 3;
  });
  return count;
}

const size = (object: Object3D) => new Box3().setFromObject(object).getSize(new Vector3());

test.each([
  ["ASCII STL", () => loadModel("stl", buffer(sampleStlAscii()))],
  ["binary STL", () => loadModel("stl", buffer(sampleStlBinary()))],
  ["3MF", () => loadModel("3mf", buffer(sample3mf()))],
])("a %s box loads and stands up: printing formats are Z-up", async (_name, load) => {
  const model = await load();
  expect(triangles(model)).toBe(SAMPLE_TRIANGLES);
  // 40 wide, 20 deep, 10 tall in the file (Z up) is 10 tall in Y on screen.
  const { x, y, z } = size(model);
  expect([x, y, z].map(Math.round)).toEqual([40, 10, 20]);
});

/** Each mesh's material colour, as hex. */
function colours(object: Object3D): string[] {
  const found: string[] = [];
  object.traverse((node) => node instanceof Mesh && found.push((node.material as { color: { getHexString(): string } }).color.getHexString()));
  return found;
}

test("a model that names no colour is drawn grey; one that does keeps it", async () => {
  expect(colours(await loadModel("3mf", buffer(sample3mf())))).toEqual(["8c8c96"]);
  expect(colours(await loadModel("stl", buffer(sampleStlAscii())))).toEqual(["8c8c96"]);
  const json = sampleGltfJson(`data:application/octet-stream;base64,${Buffer.from(sampleGltfBin()).toString("base64")}`);
  delete json.materials;
  (json.meshes as Array<{ primitives: Array<{ material?: number }> }>)[0].primitives[0].material = undefined;
  expect(colours(await loadModel("gltf", buffer(new TextEncoder().encode(JSON.stringify(json)))))).toEqual(["8c8c96"]);
  // The sample GLB's own blue survives (linear 0.35, 0.55, 0.85 as sRGB).
  expect(colours(await loadModel("glb", buffer(sampleGlb())))).not.toEqual(["8c8c96"]);
});

test("a GLB box loads as it is: glTF is already Y-up", async () => {
  const model = await loadModel("glb", buffer(sampleGlb()));
  expect(triangles(model)).toBe(SAMPLE_TRIANGLES);
  const { x, y, z } = size(model);
  expect([x, y, z].map(Math.round)).toEqual([40, 20, 10]);
});

test("a .gltf with its buffer embedded as a data URI loads without companions", async () => {
  const base64 = Buffer.from(sampleGltfBin()).toString("base64");
  const json = sampleGltfJson(`data:application/octet-stream;base64,${base64}`);
  const model = await loadModel("gltf", buffer(new TextEncoder().encode(JSON.stringify(json))));
  expect(triangles(model)).toBe(SAMPLE_TRIANGLES);
});

test("a .gltf loads its companion .bin from the bytes it is given, and lets the URL go", async () => {
  // jsdom's blob URLs can't be fetched; hand the loader the same bytes as a data URI.
  const bin = `data:application/octet-stream;base64,${Buffer.from(sampleGltfBin()).toString("base64")}`;
  const create = vi.spyOn(URL, "createObjectURL").mockReturnValue(bin);
  const revoke = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
  try {
    const model = await loadModel("gltf", buffer(sampleGltf("lamp.bin")), new Map([["lamp.bin", buffer(sampleGltfBin())]]));
    expect(triangles(model)).toBe(SAMPLE_TRIANGLES);
    expect(create).toHaveBeenCalledTimes(1);
    expect(revoke).toHaveBeenCalledWith(bin);
  } finally {
    create.mockRestore();
    revoke.mockRestore();
  }
});

test("a .gltf whose companion wasn't read fails instead of fetching it", async () => {
  await expect(loadModel("gltf", buffer(sampleGltf("lamp.bin")))).rejects.toThrow();
});

test("files that hold no model are refused", async () => {
  const empty = JSON.stringify({ asset: { version: "2.0" }, scene: 0, scenes: [{ nodes: [] }] });
  await expect(loadModel("gltf", buffer(new TextEncoder().encode(empty)))).rejects.toThrow("nothing to show");
  await expect(loadModel("glb", new ArrayBuffer(12))).rejects.toThrow();
  await expect(loadModel("3mf", new ArrayBuffer(12))).rejects.toThrow();
});

test("without WebGL, showModel throws and leaves nothing behind", async () => {
  const host = document.createElement("div");
  const model = await loadModel("stl", buffer(sampleStlAscii()));
  expect(() => showModel(host, model)).toThrow();
  expect(host.querySelector("canvas")).toBeNull();
});

// ----- Turning the model -----

const RIGHT = new Vector3(1, 0, 0);
/** Where the model's front (facing the viewer, +Z) points after `goal`. */
const front = (goal: Quaternion) => new Vector3(0, 0, 1).applyQuaternion(goal).toArray().map((v) => Math.round(v * 1000) / 1000 + 0);

test("Rotate Left and Right swing the front to that side, a quarter turn about the vertical", () => {
  const [left, right] = (() => {
    const asked: Turn[] = [];
    const bar = modelControls((t) => asked.push(t));
    for (const button of bar.querySelectorAll("button")) button.click();
    return asked;
  })();
  expect(front(turned(new Quaternion(), left, RIGHT))).toEqual([-1, 0, 0]);
  expect(front(turned(new Quaternion(), right, RIGHT))).toEqual([1, 0, 0]);
});

test("the controls are Rotate Left, Rotate Right and Reset View, each a labelled button", () => {
  const asked: Turn[] = [];
  const bar = modelControls((t) => asked.push(t));
  const buttons = [...bar.querySelectorAll("button")];
  expect(buttons.map((b) => [b.title, b.getAttribute("aria-label")])).toEqual([
    ["Rotate Left", "Rotate Left"],
    ["Rotate Right", "Rotate Right"],
    ["Reset View", "Reset View"],
  ]);
  buttons[2].click();
  expect(asked).toEqual(["reset"]);
});

test("arrow keys turn in 15° steps the way a drag would, Home resets, other keys do nothing", () => {
  expect(front(turned(new Quaternion(), turnForKey("ArrowLeft")!, RIGHT))[0]).toBeLessThan(0);
  expect(front(turned(new Quaternion(), turnForKey("ArrowRight")!, RIGHT))[0]).toBeGreaterThan(0);
  // Up tips the front up, showing the underside, as dragging up does.
  expect(front(turned(new Quaternion(), turnForKey("ArrowUp")!, RIGHT))[1]).toBeGreaterThan(0);
  expect(front(turned(new Quaternion(), turnForKey("ArrowDown")!, RIGHT))[1]).toBeLessThan(0);
  expect(turnForKey("Home")).toBe("reset");
  expect(turnForKey("a")).toBeNull();
  expect(turnForKey("Escape")).toBeNull();
});

test("turns add up, pitch follows the view's own horizontal axis, and reset undoes them all", () => {
  const step = turnForKey("ArrowRight")!;
  let goal = new Quaternion();
  for (let i = 0; i < 6; i++) goal = turned(goal, step, RIGHT);
  expect(front(goal)).toEqual([1, 0, 0]);
  // Seen from the side (the view's right is world -Z), pitch turns about that axis.
  expect(front(turned(new Quaternion(), { axis: "pitch", degrees: -90 }, new Vector3(0, 0, -2)))).toEqual([0, 0, 1]);
  expect(turned(goal, "reset", RIGHT).equals(new Quaternion())).toBe(true);
});
