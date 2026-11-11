// The shared fixture for src-tauri/crates/arbesk/src/representations.rs:
// Arbesk's own asset-core answers each case (which entries parse, what
// `listRepresentations` keeps, `currentCidOf`, `isStale`, `representationId`,
// `defaultRole`, `encodingFor`) and besk answers what a save changed
// (`representationChanges`, the version menu's words, spec F14), so the Rust port is checked against the
// rules it mirrors, never against answers written by hand. The cases are
// asset-core's own tests (test/asset-core-representations.test.js) plus the
// node matrices whose values need ECMAScript's ToNumber (spec F1).
// Run with Bun from this repository, given an Arbesk checkout:
//   bun scripts/make-representations-fixture.ts ~/Projects/arbesk src-tauri/crates/arbesk/tests/fixtures/representations-stale.json
// Re-run it in the same PR as a Rust change whenever asset-core's rules change.
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const [arbesk, out] = process.argv.slice(2);
if (!arbesk || !out) throw new Error("usage: bun scripts/make-representations-fixture.ts <arbesk checkout> <out file>");
const core = resolve(arbesk, "packages/asset-core/src/manifest");
const reps = await import(join(core, "representations.ts"));
const commit = execFileSync("git", ["-C", arbesk, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();

type Json = any;

// ---- Entries (asset-core's test fixtures) ------------------------------------

const source = {
  id: "source", role: "source", format: "blend", name: "Bracket.blend", cid: "bafyBlend",
  bytes: 4812331, encoding: "raw", created_at: 1793900000000,
};
const gcode = {
  id: "gcode-original-prusa-mk4-petg", role: "machine", format: "gcode",
  name: "Bracket_0.2mm_PETG_MK4.gcode", cid: "bafyGcode", bytes: 812331, encoding: "brotli",
  derived_from: { ref: "primary", cid: "bafyModel" },
  tool: { name: "PrusaSlicer", version: "2.9.2" },
  profile: { printer: "Original Prusa MK4", material: "PETG", layer_mm: 0.2, nozzle_mm: 0.4 },
  stats: { print_time_s: 7420, filament_g: 38.5 },
  created_at: 1793900000000,
};
const keys = (n: number) => Object.fromEntries(Array.from({ length: n }, (_, i) => ["k" + i, i]));
const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
const MAX = reps.MAX_REPRESENTATION_BYTES;

const patches: Array<[string, Json]> = [
  ["the spec's source file", {}],
  ["an upper-case id", { id: "Source" }],
  ["an id with an underscore", { id: "my_file" }],
  ["an empty id", { id: "" }],
  ["a 64-character id", { id: "a".repeat(64) }],
  ["a 65-character id", { id: "a".repeat(65) }],
  ["the reserved id primary", { id: "primary" }],
  ["an unknown role", { role: "print" }],
  ["an unknown encoding", { encoding: "gzip" }],
  ["a format with a dot", { format: ".stl" }],
  ["an upper-case format", { format: "STL" }],
  ["a 16-letter format", { format: "a".repeat(16) }],
  ["a 17-letter format", { format: "a".repeat(17) }],
  ["a 256 MB file", { bytes: MAX }],
  ["a file over 256 MB", { bytes: MAX + 1 }],
  ["a fractional size", { bytes: 1.5 }],
  ["a negative size", { bytes: -1 }],
  ["a size as a string", { bytes: "12" }],
  ["an empty name", { name: "" }],
  ["a 255-unit name", { name: "😀".repeat(127) + "a" }],
  ["a 256-unit name of 128 characters", { name: "😀".repeat(128) }],
  ["an empty cid", { cid: "" }],
  ["no cid", { cid: undefined }],
  ["a cid that is a number", { cid: 7 }],
  ["created_at as a string", { created_at: "2026-11-10T00:00:00Z" }],
  ["no created_at", { created_at: undefined }],
  ["a derived_from ref that is no id", { derived_from: { ref: "Not An Id", cid: "x" } }],
  ["a derived_from from another entry", { derived_from: { ref: "stl", cid: "x" } }],
  ["a derived_from with an empty cid", { derived_from: { ref: "stl", cid: "" } }],
  ["a derived_from of null", { derived_from: null }],
  ["a derived_from with its own extra key", { derived_from: { ref: "primary", cid: "x", note: 1 } }],
  ["a derived_from matrix of 16 numbers", { derived_from: { ref: "primary", cid: "x", transform_matrix: IDENTITY } }],
  ["a derived_from matrix of 15 numbers", { derived_from: { ref: "primary", cid: "x", transform_matrix: IDENTITY.slice(1) } }],
  ["a derived_from matrix with a string", { derived_from: { ref: "primary", cid: "x", transform_matrix: ["1", ...IDENTITY.slice(1)] } }],
  ["a profile with an object value", { profile: { printer: { model: "MK4" } } }],
  ["a profile with a boolean value", { profile: { sliced: true } }],
  ["a profile of 32 keys", { profile: keys(32) }],
  ["a profile over 32 keys", { profile: keys(33) }],
  ["stats over 32 keys", { stats: keys(33) }],
  ["a tool of null", { tool: null }],
  ["a tool that is an array", { tool: ["PrusaSlicer"] }],
  ["a newer client's key", { checksum: "abc" }],
];
const entries: Json[] = patches.map(([label, patch]) => {
  const entry = JSON.parse(JSON.stringify({ ...source, ...patch }));
  return { label, entry, valid: reps.representationSchema.safeParse(entry).success };
});
entries.push({ label: "the spec's G-code", entry: gcode, valid: reps.representationSchema.safeParse(gcode).success });
for (const [label, entry] of [["an array", [source]], ["a string", "source"], ["null", null], ["a number", 3]] as const) {
  entries.push({ label: `an entry that is ${label}`, entry, valid: reps.representationSchema.safeParse(entry).success });
}

// ---- Lists ----------------------------------------------------------------------

const manifest = (extra: Json = {}) => ({
  type: "asset", version: 3,
  scene: { nodes: [{ node_id: "node_1", source: { cid: "bafyModel" } }] },
  ...extra,
});
const listCases: Array<[string, Json]> = [
  ["valid entries in stored order", manifest({ representations: [gcode, source] })],
  ["an entry that does not parse left out", manifest({ representations: [{ id: "BAD" }, source] })],
  ["no list", manifest()],
  ["a list that is a string", manifest({ representations: "nope" })],
  ["a list that is an object", manifest({ representations: { 0: source } })],
  ["a duplicate id kept twice", manifest({ representations: [source, { ...source, cid: "bafyDup" }] })],
  ["a foreign entry and nulls", manifest({ representations: [null, { id: "print-plan", role: "plan" }, source, 7] })],
];
const lists = listCases.map(([label, m]) => {
  const stored = Array.isArray(m.representations) ? m.representations : [];
  const kept = stored.flatMap((entry: Json, i: number) => (reps.representationSchema.safeParse(entry).success ? [i] : []));
  if (kept.length !== reps.listRepresentations(m).length) throw new Error(`${label}: list and schema disagree`);
  const ids = stored.flatMap((entry: Json) => (typeof entry?.id === "string" ? [entry.id] : []));
  return { label, manifest: m, kept, stored_count: stored.length, stored_ids: ids };
});

// ---- currentCidOf ---------------------------------------------------------------

const cidCases: Array<[string, Json, string]> = [
  ["the model", manifest({ representations: [source] }), "primary"],
  ["another entry", manifest({ representations: [source] }), "source"],
  ["a missing entry", manifest({ representations: [source] }), "missing"],
  ["no scene", {}, "primary"],
  ["an empty source cid", { scene: { nodes: [{ source: { cid: "" } }] } }, "primary"],
  ["a source cid that is a number", { scene: { nodes: [{ source: { cid: 7 } }] } }, "primary"],
  ["an entry this client can't parse", manifest({ representations: [{ id: "source", cid: "bafyForeign" }, source] }), "source"],
  ["only an entry this client can't parse", manifest({ representations: [{ id: "source", cid: "bafyForeign" }] }), "source"],
  ["the first of two valid entries", manifest({ representations: [source, { ...source, cid: "bafyDup" }] }), "source"],
];
const cids = cidCases.map(([label, m, ref]) => ({ label, manifest: m, ref, cid: reps.currentCidOf(m, ref) }));

// ---- isStale ----------------------------------------------------------------------

const stl = { ...source, id: "stl", role: "mesh", format: "stl", cid: "bafyStl", derived_from: { ref: "source", cid: "bafyBlend" } };
const placed = { ...gcode, derived_from: { ref: "primary", cid: "bafyModel", transform_matrix: IDENTITY } };
const withMatrix = (matrix: Json) => manifest({ scene: { nodes: [{ node_id: "node_1", source: { cid: "bafyModel" }, transform_matrix: matrix }] } });
const staleCases: Array<[string, Json, Json]> = [
  ["no derived_from", manifest({ representations: [source] }), source],
  ["primary, the same model", manifest({ representations: [gcode] }), gcode],
  ["primary, another model", manifest({ representations: [gcode], scene: { nodes: [{ source: { cid: "bafyModel2" } }] } }), gcode],
  ["primary, no source node", { version: 1, scene: { nodes: [{ node_id: "n", child_ref: {} }] } }, gcode],
  ["chained, the same source", manifest({ representations: [source, stl] }), stl],
  ["chained, another source", manifest({ representations: [{ ...source, cid: "bafyBlend2" }, stl] }), stl],
  ["chained, the source removed", manifest({ representations: [stl] }), stl],
  ["chained, the source can't be parsed", manifest({ representations: [{ id: "source", cid: "bafyBlend" }, stl] }), stl],
  ["placed, unmoved", manifest(), placed],
  ["placed, scaled", withMatrix([2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1]), placed],
  ["unplaced entry, scaled model", withMatrix([2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1]), gcode],
  ["placed, identity written out and more nodes", manifest({ scene: { nodes: [
    { node_id: "node_1", source: { cid: "bafyModel" }, transform_matrix: IDENTITY },
    { node_id: "node_2", source: { cid: "bafyOther" } },
  ] } }), placed],
  ["placed, a 15-value matrix reads as identity", withMatrix(IDENTITY.slice(1)), placed],
  ["placed, a matrix that is a string", withMatrix("1,0,0"), placed],
  ["placed, -0 equals 0", withMatrix([1, -0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]), placed],
];
// Values that need ToNumber, at a 1 (index 0) and at a 0 (index 1).
const odd: Json[] = [
  "", "  ", " \n\t1 ", "0x10", "0X1f", "0o7", "0b11", "-0x10", "Infinity", "-Infinity", "+Infinity",
  "inf", "nan", "NaN", "1_000", "1", "-.5", "5.", "1e3", "+1", "1e", ".", "0", "-0", "00", "1.0",
  " 1 ", "﻿0", "1 1", [], [7], ["7"], [1, 2], [null], [[]], [[1]], [true], {},
  true, false, null, 1.0, 0,
];
for (const value of odd) {
  for (const index of [0, 1]) {
    const matrix: Json[] = [...IDENTITY];
    matrix[index] = value;
    staleCases.push([`ToNumber(${JSON.stringify(value)}) at index ${index}`, withMatrix(matrix), placed]);
  }
}
const stale = staleCases.map(([label, m, rep]) => {
  if (!reps.representationSchema.safeParse(rep).success) throw new Error(`${label}: the entry must be valid`);
  return { label, manifest: m, rep, stale: reps.isStale(m, rep) };
});

// ---- Ids and tables ----------------------------------------------------------------

const idCases: Array<[string, string, Json]> = [
  ["machine", "gcode", { printer: "Original Prusa MK4", material: "PETG" }],
  ["source", "blend", undefined],
  ["mesh", "stl", undefined],
  ["machine", "gcode", { layer_mm: 0.2 }],
  ["machine", "gcode", { printer: "x".repeat(100), material: "PLA" }],
  ["machine", "gcode", { printer: "Bambu Lab X1 Carbon", material: 0.2 }],
  ["machine", "gcode", { printer: "", material: "PLA" }],
  ["machine", "gcode", { printer: "---", material: "PLA" }],
  ["other", "", undefined],
  ["view", "glb", { printer: "Ünïcödé Printer!" }],
  ["machine", "3mf", { printer: 12, material: "PETG-CF" }],
];
const ids = idCases.map(([role, format, profile]) => ({ role, format, profile: profile ?? null, id: reps.representationId(role, format, profile) }));
const formats = ["blend", "step", "stp", "f3d", "fcstd", "scad", "iges", "igs", "stl", "obj", "ply", "3mf", "glb", "gltf",
  "usdz", "gcode", "bgcode", "mtl", "dxf", "svg", "txt", "json", "csv", "zip", "BLEND", "GCODE"];
const tables = formats.map((format) => ({ format, role: reps.defaultRole(format), encoding: reps.encodingFor(format) }));

// ---- What a save changed (besk's representationChanges) -----------------------------

const { representationChanges } = await import(resolve(arbesk, "packages/besk/src/representations.ts"));
const stlFile = { ...source, id: "stl", role: "mesh", format: "stl", name: "Bracket.stl", cid: "bafyStl" };
const changeCases: Array<[string, Json, Json]> = [
  ["the first version's files are added", null, manifest({ representations: [source, gcode] })],
  ["a first version without files", null, manifest()],
  ["nothing changed", manifest({ representations: [source] }), manifest({ representations: [source] })],
  ["a file added", manifest({ representations: [source] }), manifest({ representations: [source, gcode] })],
  ["a file replaced", manifest({ representations: [gcode] }), manifest({ representations: [{ ...gcode, cid: "bafyGcode2" }] })],
  ["a file removed", manifest({ representations: [source, gcode] }), manifest({ representations: [source] })],
  ["the list dropped", manifest({ representations: [source, gcode] }), manifest()],
  ["added, replaced and removed together", manifest({ representations: [source, stlFile] }),
    manifest({ representations: [gcode, { ...source, cid: "bafyBlend2" }] })],
  ["an unknown format in capitals", null, manifest({ representations: [{ ...source, id: "ply", format: "ply" }] })],
  ["a printer without a material", null, manifest({ representations: [{ ...gcode, profile: { printer: "Original Prusa MK4" } }] })],
  ["an empty printer left out", null, manifest({ representations: [{ ...gcode, profile: { printer: "", material: "PLA" } }] })],
  ["numbers in the profile", null, manifest({ representations: [{ ...gcode, profile: { printer: 12, material: 0.5 } }] })],
  ["entries this client can't read ignored", manifest({ representations: [{ id: "plan" }, source] }),
    manifest({ representations: [{ id: "source", cid: "x" }, { id: "plan2" }] })],
  ["a duplicate id compared as besk's Map", manifest({ representations: [source, { ...source, cid: "bafyDup" }] }),
    manifest({ representations: [{ ...source, cid: "bafyDup" }] })],
  ["a duplicate id in the new list", manifest({ representations: [source] }),
    manifest({ representations: [{ ...source, cid: "bafyA" }, { ...source, cid: "bafyB" }] })],
];
const changes = changeCases.map(([label, prev, next]) => ({
  label, prev, next, changes: representationChanges(prev, next).map((change: { text: string }) => change.text),
}));

const fixture = {
  about: "asset-core's answers for the representation cases (made by scripts/make-representations-fixture.ts; do not edit by hand)",
  arbesk_commit: commit,
  entries, lists, cids, stale, ids, tables, changes,
};
writeFileSync(out, JSON.stringify(fixture, null, 1) + "\n");
console.log(`${commit}: ${entries.length} entries, ${lists.length} lists, ${cids.length} cids, ${stale.length} stale, ${ids.length} ids, ${changes.length} changes`);
