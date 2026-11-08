// Which files the 3D preview takes, and which companion files a .gltf may
// load (src/models.ts).
import { expect, test } from "vitest";
import { companionPath, gltfCompanions, modelFormat } from "../src/models";

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
