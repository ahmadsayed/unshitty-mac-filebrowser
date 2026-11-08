// The 3D viewer's loading half (src/model-viewer.ts) reads real files: the
// mock's sample box as STL (ASCII and binary), 3MF, GLB and glTF. Drawing
// needs WebGL, which jsdom lacks, so showModel is checked to fail cleanly.
import { expect, test, vi } from "vitest";
import { Box3, Mesh, Vector3, type Object3D } from "three";
import { loadModel, showModel } from "../src/model-viewer";
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
