// Which files the 3D preview takes, and which companion files a .gltf may
// load (src/models.ts).
import { expect, test } from "vitest";
import { companionPath, gltfCompanions, gltfExternalUris, isGlb, modelFormat } from "../src/models";
import { sampleGlb, sampleGltf } from "../dev/mock-tauri/models";

test.each([
  ["bracket.stl", "stl"],
  ["BRACKET.STL", "stl"],
  ["box.3mf", "3mf"],
  ["lamp.glb", "glb"],
  ["lamp.gltf", "gltf"],
  ["lamp.bin", null],
  ["notes.txt", null],
  [".stl", null],
  ["stl", null],
])("%s is a %s model", (name, format) => {
  expect(modelFormat(name)).toBe(format);
});

test.each([
  ["lamp.bin", "lamp.bin"],
  ["textures/wood.png", "textures/wood.png"],
  ["./lamp.bin", "lamp.bin"],
  ["lamp%20light.bin", "lamp light.bin"],
  ["../secret.bin", null],
  ["textures/../../secret.bin", null],
  ["textures//wood.png", null],
  ["/etc/passwd", null],
  ["C:\\models\\lamp.bin", null],
  ["sub\\lamp.bin", null],
  ["https://example.com/lamp.bin", null],
  ["file:///etc/passwd", null],
  ["data:application/octet-stream;base64,AAAA", null],
  ["%E0%A4%A", null],
  ["..%2Fsecret.bin", null],
])("the glTF URI %s is the companion %s", (uri, path) => {
  expect(companionPath(uri)).toBe(path);
});

test("a .gltf's companions are its buffers' and images' files, each once, without data URIs or escapes", () => {
  expect(
    gltfCompanions({
      buffers: [{ uri: "lamp.bin" }, { uri: "data:application/octet-stream;base64,AAAA" }, { byteLength: 4 }],
      images: [{ uri: "textures/wood.png" }, { uri: "lamp.bin" }, { uri: "../escape.png" }, { bufferView: 1 }],
    }),
  ).toEqual(["lamp.bin", "textures/wood.png"]);
  expect(gltfCompanions({})).toEqual([]);
  expect(gltfCompanions(null)).toEqual([]);
  expect(gltfCompanions({ buffers: [{ uri: 7 }] })).toEqual([]);
});

test("what a .gltf loads from outside itself is every URI but a data: one, escapes and absolute ones too", () => {
  expect(
    gltfExternalUris({
      buffers: [{ uri: "../a.bin" }, { uri: "data:application/octet-stream;base64,AAAA" }, { uri: "file:///a.bin" }, { byteLength: 4 }],
      images: [{ uri: "/abs/wood.png" }, { uri: "DATA:image/png;base64,AA" }, { uri: "../a.bin" }, { uri: "https://example.com/t.png" }],
    }),
  ).toEqual(["../a.bin", "file:///a.bin", "/abs/wood.png", "https://example.com/t.png"]);
  expect(gltfExternalUris({ buffers: [{ uri: "data:,x" }] })).toEqual([]);
  expect(gltfExternalUris(null)).toEqual([]);
});

test("a GLB is known by its first bytes, whatever its name", () => {
  expect(isGlb(sampleGlb().slice().buffer)).toBe(true);
  expect(isGlb(sampleGltf("box.bin").slice().buffer)).toBe(false);
  expect(isGlb(new Uint8Array([0x67, 0x6c, 0x54]).buffer)).toBe(false);
  expect(isGlb(new ArrayBuffer(0))).toBe(false);
});
