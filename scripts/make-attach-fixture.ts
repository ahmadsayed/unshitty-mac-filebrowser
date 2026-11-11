// The reference fixture for src-tauri/crates/arbesk/src/attach.rs (spec F11):
// the next asset manifest `besk attach` writes, made by running Arbesk's own
// `attachRepresentation` (packages/besk/src/representations.ts) with its
// storage and chain stubbed out, so the Rust write is checked field for
// field against besk's, a bare composite's wrapper included.
// Run with Bun from this repository, given an Arbesk checkout:
//   bun scripts/make-attach-fixture.ts ~/Projects/arbesk src-tauri/crates/arbesk/tests/fixtures/attach.json
// Re-run in the same PR as any change to besk's attach or asset-core's
// representation rules; the Arbesk commit it ran is recorded.
import { mock } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const [arbesk, out] = process.argv.slice(2);
if (!arbesk || !out) throw new Error("usage: bun scripts/make-attach-fixture.ts <arbesk checkout> <out file>");
const besk = resolve(arbesk, "packages/besk/src");
const { detectFormat } = await import(resolve(arbesk, "packages/asset-core/src/formats/index.ts"));

/** What the stubbed storage and chain hold for one case. */
const store = { manifest: null as unknown, written: null as unknown, raw: 0 };
mock.module(join(besk, "catalog.ts"), () => ({
  detectFormat,
  getManifest: async () => structuredClone(store.manifest),
  getCollectionManifest: async () => ({ cid: "bafyCollection", manifest: {} }),
  readRawBytes: async () => new Uint8Array(),
  writeRawBytes: async () => `bafyRaw${++store.raw}`,
  writeManifest: async (manifest: unknown) => {
    store.written = manifest;
    return "bafyNext";
  },
  replaceAssetEntry: () => {},
  updateCollection: async (_session: unknown, _token: string, mutate: (draft: unknown) => void) => mutate({ assets: {} }),
}));
const { attachRepresentation } = await import(join(besk, "representations.ts"));

const NOW = 1_762_000_000_000;
Date.now = () => NOW;

const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
/** An asset manifest as the Studio saves one, with other files (one this
 *  app can't read) and keys besk doesn't know. */
const asset = {
  type: "asset",
  name: "Box",
  asset_id: "a1",
  version: 3,
  timestamp: 1000,
  scene: { nodes: [{ node_id: "node_1", transform_matrix: [2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 1, 2, 3, 1], source: { cid: "bafySrc", format: "gltf", path: "composite.gltf" } }] },
  thumbnail: { cid: "bafyThumb", mime: "image/webp" },
  future: { kept: true },
  representations: [
    { id: "stl", role: "mesh", format: "stl", name: "Box.stl", cid: "bafyOldStl", bytes: 3, encoding: "raw", created_at: 1, derived_from: { ref: "primary", cid: "bafyOld", transform_matrix: IDENTITY } },
    { id: "plan", role: "someday", format: "json", newer: [1, 2] },
  ],
};
/** A bare composite `besk upload` stored, glTF without a name and 3MF with one. */
const bareGltf = { asset: { version: "2.0" }, nodes: [{ mesh: 0 }], arbesk: { parts: ["bafyPart"] } };
const bare3mf = { arbesk_format: "composite-3mf", name: "Bracket", version: 2, parts: ["bafyPart"] };

const cases = [
  { name: "an STL replaces the stored one", current: asset, file: ["Box.stl", "solid box\n"] },
  { name: "a source file is made from nothing", current: asset, file: ["Box.blend", "BLENDER-v300"] },
  { name: "a G-code is stored brotli-framed", current: asset, file: ["Box.gcode", "G1 X1 Y1\n".repeat(50)] },
  { name: "a bare glTF composite is wrapped", current: bareGltf, file: ["Box.stl", "solid\n"] },
  { name: "a bare 3MF composite is wrapped", current: bare3mf, file: ["Bracket.3mf", "PK\u0003\u0004"] },
];

const dir = mkdtempSync(join(tmpdir(), "attach-fixture-"));
const made = [];
for (const [i, { name, current, file }] of cases.entries()) {
  store.manifest = current;
  store.written = null;
  store.raw = 0;
  mkdirSync(join(dir, String(i)));
  const path = join(dir, String(i), file[0]);
  writeFileSync(path, file[1]);
  const listed = `Box.${detectFormat(current) === "3mf" ? "3mf" : "gltf"}`;
  const { representation } = await attachRepresentation({} as never, { tokenId: "1", assetID: "a1", cid: "bafyBase", name: "Box", file: path });
  const next = store.written;
  made.push({
    name,
    cid: "bafyBase",
    listed,
    asset_id: "a1",
    now: NOW,
    current,
    files: [{ name: file[0], format: representation.format, bytes: representation.bytes, cid: representation.cid }],
    next,
  });
}

const commit = execFileSync("git", ["-C", arbesk, "rev-parse", "HEAD"]).toString().trim();
const about =
  "The next asset manifest `besk attach` writes for each case, made by scripts/make-attach-fixture.ts from Arbesk's own attachRepresentation; attach.rs' test builds the same from `current` and compares.";
writeFileSync(out, JSON.stringify({ about, arbesk_commit: commit, cases: made }, null, 2) + "\n");
console.log(`wrote ${made.length} cases from Arbesk ${commit}`);
