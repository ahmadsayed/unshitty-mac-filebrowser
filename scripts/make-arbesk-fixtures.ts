// Reference fixtures for src-tauri/src/arbesk/compose.rs: Arbesk's own
// asset-core stores each sample model (decompose into an in-memory IPFS) and
// rebuilds it (compose), so the Rust build is checked against Arbesk's code.
// Run with Bun from this repository, given an Arbesk checkout:
//   bun scripts/make-arbesk-fixtures.ts ~/Projects/arbesk src-tauri/tests/fixtures/arbesk
// Writes <out>/<case>/{source.json, cids/<cid>, expected.<ext>}.
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

async function makeCase(name: string, ext: string, input: unknown, opts: { format?: string } = {}): Promise<void> {
  const { read, write, dump } = createMemoryIpfs();
  const core = createArbeskCore({ ipfsRead: read, ipfsWrite: write });
  const { compositeCid } = await core.decompose(input as never, opts);
  if (!compositeCid) throw new Error(`${name}: no composite CID`);
  const composite = await read.getJSON(compositeCid);
  const expected = new Uint8Array(await (await core.compose(composite, opts)).arrayBuffer());
  const dir = join(out, name);
  mkdirSync(join(dir, "cids"), { recursive: true });
  writeFileSync(join(dir, "source.json"), JSON.stringify(composite, null, 2));
  const parts = [...dump()].filter(([cid]) => cid !== compositeCid);
  for (const [cid, bytes] of parts) writeFileSync(join(dir, "cids", cid), bytes);
  writeFileSync(join(dir, `expected.${ext}`), expected);
  console.log(name, compositeCid, `${parts.length} parts`, `${expected.length} bytes`);
}

await makeCase("glb-box", "gltf", sampleGlb().slice().buffer);

const withImage = sampleGltfJson(`data:application/octet-stream;base64,${base64(sampleGltfBin())}`) as Record<string, any>;
withImage.images = [{ uri: `data:image/png;base64,${PNG}`, mimeType: "image/png", name: "red" }];
withImage.textures = [{ source: 0 }];
await makeCase("gltf-image", "gltf", withImage);

await makeCase("3mf-box", "3mf", sample3mf().slice().buffer, { format: "3mf" });

const withThumb = unzipSync(sample3mf());
withThumb["Metadata/thumbnail.png"] = new Uint8Array(Buffer.from(PNG, "base64"));
await makeCase("3mf-thumbnail", "3mf", zipSync(withThumb).slice().buffer, { format: "3mf" });
