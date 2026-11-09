// Reference fixtures for src-tauri/crates/arbesk/src/compose.rs and decompose.rs:
// Arbesk's own asset-core stores each sample model (decompose into an
// in-memory IPFS) and rebuilds it (compose), so the Rust code is checked
// against Arbesk's in both directions.
// Run with Bun from this repository, given an Arbesk checkout:
//   bun scripts/make-arbesk-fixtures.ts ~/Projects/arbesk src-tauri/crates/arbesk/tests/fixtures/arbesk
// Writes <out>/<case>/{input.<ext>, source.json, cids/<cid>, names.json,
// expected.<ext>}; names.json maps each part's CID to the file name
// asset-core uploaded it as.
import { mkdirSync, writeFileSync } from "node:fs";
import { unzipSync, zipSync } from "three/examples/jsm/libs/fflate.module.js";
import { join, resolve } from "node:path";
import { sample3mf, sampleGlb, sampleGltfBin, sampleGltfJson } from "../dev/mock-tauri/models.ts";

const [arbesk, out] = process.argv.slice(2);
if (!arbesk || !out) throw new Error("usage: bun scripts/make-arbesk-fixtures.ts <arbesk checkout> <out dir>");
const assetCore = resolve(arbesk, "packages/asset-core/src");
const { createArbeskCore } = await import(join(assetCore, "facade.ts"));
const { createMemoryIpfs } = await import(join(assetCore, "storage/memory-ipfs.ts"));

// A 1x1 red PNG.
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

function base64(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64");
}

/** The model file a case starts from, as the user would have it on disk. */
function inputFile(input: unknown): Uint8Array {
  return input instanceof ArrayBuffer ? new Uint8Array(input) : new TextEncoder().encode(JSON.stringify(input));
}

async function makeCase(
  name: string,
  [inputExt, ext]: [string, string],
  input: unknown,
  opts: { format?: string } = {},
): Promise<void> {
  const { read, write, dump } = createMemoryIpfs();
  const names: Record<string, string> = {};
  const recording = {
    ...write,
    async write(...args: Parameters<typeof write.write>) {
      const cid = await write.write(...args);
      names[cid] = args[1] as string;
      return cid;
    },
  };
  const core = createArbeskCore({ ipfsRead: read, ipfsWrite: recording });
  const { compositeCid } = await core.decompose(input as never, opts);
  if (!compositeCid) throw new Error(`${name}: no composite CID`);
  const composite = await read.getJSON(compositeCid);
  const expected = new Uint8Array(await (await core.compose(composite, opts)).arrayBuffer());
  const dir = join(out, name);
  mkdirSync(join(dir, "cids"), { recursive: true });
  writeFileSync(join(dir, `input.${inputExt}`), inputFile(input));
  writeFileSync(join(dir, "names.json"), JSON.stringify(names, null, 2));
  writeFileSync(join(dir, "source.json"), JSON.stringify(composite, null, 2));
  const parts = [...dump()].filter(([cid]) => cid !== compositeCid);
  for (const [cid, bytes] of parts) writeFileSync(join(dir, "cids", cid), bytes);
  writeFileSync(join(dir, `expected.${ext}`), expected);
  console.log(name, compositeCid, `${parts.length} parts`, `${expected.length} bytes`);
}

await makeCase("glb-box", ["glb", "gltf"], sampleGlb().slice().buffer);

const withImage = sampleGltfJson(`data:application/octet-stream;base64,${base64(sampleGltfBin())}`) as Record<string, any>;
withImage.images = [{ uri: `data:image/png;base64,${PNG}`, mimeType: "image/png", name: "red" }];
withImage.textures = [{ source: 0 }];
await makeCase("gltf-image", ["gltf", "gltf"], withImage);

await makeCase("3mf-box", ["3mf", "3mf"], sample3mf().slice().buffer, { format: "3mf" });

const withThumb = unzipSync(sample3mf());
withThumb["Metadata/thumbnail.png"] = new Uint8Array(Buffer.from(PNG, "base64"));
await makeCase("3mf-thumbnail", ["3mf", "3mf"], zipSync(withThumb).slice().buffer, { format: "3mf" });

/**
 * sampleGlb with the red PNG as a bufferView image between the box's
 * positions and indices, a sparse accessor and a Draco reference to views
 * after it (the Draco view holds no Draco data: only its index matters), and
 * a BIN chunk padded past the buffer's byteLength. Cutting the image out
 * shifts the later views, renumbers their users and adds the first view's
 * missing byteOffset, as glb-parser.ts's pruneBufferImageData does.
 */
function glbWithTexture(): ArrayBuffer {
  const glb = sampleGlb();
  const view = new DataView(glb.buffer, glb.byteOffset, glb.byteLength);
  const jsonLength = view.getUint32(12, true);
  const json = JSON.parse(new TextDecoder().decode(glb.subarray(20, 20 + jsonLength)));
  const bin = glb.subarray(28 + jsonLength);
  const png = new Uint8Array(Buffer.from(PNG, "base64"));
  // positions 0..96 | png 96..166 | pad | indices 168..240 | sparse values 240..252 | sparse index 252
  const body = new Uint8Array(256);
  body.set(bin.subarray(0, 96), 0);
  body.set(png, 96);
  body.set(bin.subarray(96, 168), 168);
  new DataView(body.buffer).setFloat32(240, 30, true);
  json.bufferViews = [
    { buffer: 0, byteLength: 96, target: 34962 },
    { buffer: 0, byteOffset: 96, byteLength: png.length },
    { buffer: 0, byteOffset: 168, byteLength: 72, target: 34963 },
    { buffer: 0, byteOffset: 240, byteLength: 12 },
    { buffer: 0, byteOffset: 252, byteLength: 1 },
  ];
  json.accessors[1].bufferView = 2;
  json.accessors.push({
    componentType: 5126,
    count: 8,
    type: "VEC3",
    sparse: { count: 1, indices: { bufferView: 4, componentType: 5121 }, values: { bufferView: 3 } },
  });
  json.meshes.push({
    primitives: [{ attributes: { POSITION: 2 }, extensions: { KHR_draco_mesh_compression: { bufferView: 3, attributes: { POSITION: 0 } } } }],
  });
  json.images = [{ bufferView: 1, name: "red" }];
  json.textures = [{ source: 0 }];
  json.buffers = [{ byteLength: 253 }];
  const text = new TextEncoder().encode(JSON.stringify(json));
  const jsonChunk = new Uint8Array(Math.ceil(text.length / 4) * 4).fill(0x20);
  jsonChunk.set(text);
  const out = new Uint8Array(12 + 8 + jsonChunk.length + 8 + body.length);
  const header = new DataView(out.buffer);
  header.setUint32(0, 0x46546c67, true);
  header.setUint32(4, 2, true);
  header.setUint32(8, out.length, true);
  header.setUint32(12, jsonChunk.length, true);
  header.setUint32(16, 0x4e4f534a, true);
  out.set(jsonChunk, 20);
  header.setUint32(20 + jsonChunk.length, body.length, true);
  header.setUint32(24 + jsonChunk.length, 0x004e4942, true);
  out.set(body, 28 + jsonChunk.length);
  return out.buffer;
}

await makeCase("glb-texture", ["glb", "gltf"], glbWithTexture());
