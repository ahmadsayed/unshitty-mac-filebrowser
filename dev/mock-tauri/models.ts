// Small, valid 3D models for the mock's preview and the tests: one box,
// written as ASCII and binary STL, 3MF, glTF with a companion .bin, and GLB.
import { zipSync } from "three/examples/jsm/libs/fflate.module.js";

type Vec = [number, number, number];

/** A 40 × 20 × 10 box: its corners and its 12 triangles, wound outwards. */
const CORNERS: Vec[] = [
  [-20, -10, 0], [20, -10, 0], [20, 10, 0], [-20, 10, 0],
  [-20, -10, 10], [20, -10, 10], [20, 10, 10], [-20, 10, 10],
];
const TRIANGLES: Vec[] = [
  [0, 3, 2], [0, 2, 1], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4],
  [3, 7, 6], [3, 6, 2], [0, 4, 7], [0, 7, 3], [1, 2, 6], [1, 6, 5],
];

export const SAMPLE_TRIANGLES = TRIANGLES.length;

/** UTF-8 bytes in this realm's own Uint8Array: jsdom's TextEncoder answers
 *  with another realm's, which three.js doesn't take for an ArrayBuffer. */
const text = (s: string) => {
  const encoded = new TextEncoder().encode(s);
  const out = new Uint8Array(encoded.length);
  out.set(encoded);
  return out;
};

function normal([a, b, c]: Vec): Vec {
  const [p, q, r] = [CORNERS[a], CORNERS[b], CORNERS[c]];
  const u = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
  const v = [r[0] - p[0], r[1] - p[1], r[2] - p[2]];
  const n: Vec = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
  const length = Math.hypot(...n);
  return n.map((x) => x / length) as Vec;
}

export function sampleStlAscii(): Uint8Array {
  const facets = TRIANGLES.map((t) =>
    `facet normal ${normal(t).join(" ")}\n outer loop\n${t.map((i) => `  vertex ${CORNERS[i].join(" ")}\n`).join("")} endloop\nendfacet\n`,
  );
  return text(`solid box\n${facets.join("")}endsolid box\n`);
}

export function sampleStlBinary(): Uint8Array {
  const out = new DataView(new ArrayBuffer(84 + 50 * TRIANGLES.length));
  out.setUint32(80, TRIANGLES.length, true);
  TRIANGLES.forEach((t, i) => {
    const values = [...normal(t), ...t.flatMap((v) => CORNERS[v])];
    values.forEach((value, j) => out.setFloat32(84 + i * 50 + j * 4, value, true));
  });
  return new Uint8Array(out.buffer);
}

/** The sample cube as a 3MF, its coordinates in `unit` (3MF's default: millimetres). */
export function sample3mf(unit = "millimeter"): Uint8Array {
  const vertices = CORNERS.map(([x, y, z]) => `<vertex x="${x}" y="${y}" z="${z}"/>`).join("");
  const triangles = TRIANGLES.map(([a, b, c]) => `<triangle v1="${a}" v2="${b}" v3="${c}"/>`).join("");
  const model =
    `<?xml version="1.0" encoding="UTF-8"?><model unit="${unit}" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">` +
    `<resources><object id="1" type="model"><mesh><vertices>${vertices}</vertices><triangles>${triangles}</triangles></mesh></object></resources>` +
    `<build><item objectid="1"/></build></model>`;
  const rels =
    `<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">` +
    `<Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>`;
  const types =
    `<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">` +
    `<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>` +
    `<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>`;
  return zipSync({ "[Content_Types].xml": text(types), "_rels/.rels": text(rels), "3D/3dmodel.model": text(model) });
}

/** The box's positions then its indices, as one glTF buffer. */
export function sampleGltfBin(): Uint8Array {
  const positions = new Float32Array(CORNERS.flat());
  const indices = new Uint16Array(TRIANGLES.flat());
  const out = new Uint8Array(positions.byteLength + indices.byteLength);
  out.set(new Uint8Array(positions.buffer), 0);
  out.set(new Uint8Array(indices.buffer), positions.byteLength);
  return out;
}

/** The glTF document; `uri` names the buffer's file, or none for a GLB's own chunk. */
export function sampleGltfJson(uri?: string): Record<string, unknown> {
  const positionBytes = CORNERS.length * 12;
  const indexBytes = TRIANGLES.length * 3 * 2;
  return {
    asset: { version: "2.0" },
    scene: 0,
    scenes: [{ nodes: [0] }],
    nodes: [{ mesh: 0 }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 }, indices: 1, material: 0 }] }],
    materials: [{ pbrMetallicRoughness: { baseColorFactor: [0.35, 0.55, 0.85, 1], metallicFactor: 0, roughnessFactor: 0.5 } }],
    accessors: [
      { bufferView: 0, componentType: 5126, count: CORNERS.length, type: "VEC3", min: [-20, -10, 0], max: [20, 10, 10] },
      { bufferView: 1, componentType: 5123, count: TRIANGLES.length * 3, type: "SCALAR" },
    ],
    bufferViews: [
      { buffer: 0, byteOffset: 0, byteLength: positionBytes, target: 34962 },
      { buffer: 0, byteOffset: positionBytes, byteLength: indexBytes, target: 34963 },
    ],
    buffers: [{ byteLength: positionBytes + indexBytes, ...(uri ? { uri } : {}) }],
  };
}

export function sampleGltf(binName: string): Uint8Array {
  return text(JSON.stringify(sampleGltfJson(binName)));
}

const pad4 = (bytes: Uint8Array, fill: number) => {
  const out = new Uint8Array(Math.ceil(bytes.length / 4) * 4).fill(fill);
  out.set(bytes);
  return out;
};

export function sampleGlb(): Uint8Array {
  const json = pad4(text(JSON.stringify(sampleGltfJson())), 0x20);
  const bin = pad4(sampleGltfBin(), 0);
  const out = new DataView(new ArrayBuffer(12 + 8 + json.length + 8 + bin.length));
  out.setUint32(0, 0x46546c67, true);
  out.setUint32(4, 2, true);
  out.setUint32(8, out.byteLength, true);
  out.setUint32(12, json.length, true);
  out.setUint32(16, 0x4e4f534a, true);
  new Uint8Array(out.buffer).set(json, 20);
  out.setUint32(20 + json.length, bin.length, true);
  out.setUint32(24 + json.length, 0x004e4942, true);
  new Uint8Array(out.buffer).set(bin, 28 + json.length);
  return new Uint8Array(out.buffer);
}

/** The bytes the mock serves for a file name: a sample model, or null for other files. */
export function sampleModelBytes(name: string): Uint8Array | null {
  const lower = name.toLowerCase();
  const base = name.slice(0, name.lastIndexOf("."));
  if (lower.endsWith(".stl")) return lower.includes("binary") ? sampleStlBinary() : sampleStlAscii();
  if (lower.endsWith(".3mf")) return sample3mf();
  if (lower.endsWith(".glb")) return sampleGlb();
  if (lower.endsWith(".gltf")) return sampleGltf(`${encodeURIComponent(base)}.bin`);
  if (lower.endsWith(".bin")) return sampleGltfBin();
  return null;
}
