// The words for an Arbesk asset's other files (src/representations.ts).
import { expect, test } from "vitest";
import type { RepresentationInfo } from "../src/api";
import { assetFormat, copyNameAt, copySources, defaultChoice, fileSubtitle, formatLabel, formatRowLabel, formatsAcross, onlyChoice, openWithCandidates, profileText, staleReason, statsText, toolText, uniqueNames } from "../src/representations";
import { filesEmblemOf, filesTitle } from "../src/icons";

const rep = (over: Partial<RepresentationInfo>): RepresentationInfo => ({
  id: "stl", role: "mesh", format: "stl", name: "A.stl", bytes: 1000, stale: false, derived_from: null,
  tool: null, profile: null, stats: null, created_at: 0, path: "", ...over,
});

test("format labels are besk's, others in capitals", () => {
  expect(["gcode", "blend", "stl", "3mf", "bgcode", "step", "xyz"].map(formatLabel)).toEqual(["G-code", "Blender file", "STL", "3MF", "binary G-code", "STEP", "XYZ"]);
});

test("tool, profile and stats in words", () => {
  expect(toolText({ name: "PrusaSlicer", version: "2.9.2" })).toBe("PrusaSlicer 2.9.2");
  expect(toolText({ app: "Cura" })).toBe("Cura");
  expect(toolText(null)).toBe("");
  expect(profileText({ printer: "Original Prusa MK4", material: "PETG", layer_mm: 0.2, nozzle_mm: 0.4 })).toBe("Original Prusa MK4, PETG, 0.2 mm layers, 0.4 mm nozzle");
  expect(profileText({ material: "PLA", extra: "x" })).toBe("PLA");
  expect(statsText({ print_time_s: 7380, filament_g: 38.5 })).toEqual(["2 h 3 min", "38.5 g"]);
  expect(statsText({ print_time_s: 2700 })).toEqual(["45 min"]);
  expect(statsText({ print_time_s: 10800, other: 1 })).toEqual(["3 h"]);
  expect(statsText({ print_time_s: "long" })).toEqual([]);
});

test("a file's subtitle, joined with ·", () => {
  expect(fileSubtitle(rep({}))).toBe("STL · 1.0 kB · Mesh");
  expect(fileSubtitle(rep({ role: "view", format: "usdz", tool: { name: "Reality" } }))).toBe("USDZ · 1.0 kB · For viewing · Reality");
});

test("why a file is out of date", () => {
  const source = rep({ id: "source", format: "blend" });
  expect(staleReason(rep({ stale: true, derived_from: "primary" }), [])).toBe("Made from an earlier version of the model");
  expect(staleReason(rep({ stale: true, derived_from: "source" }), [source])).toBe("Made from an earlier “source”");
  expect(staleReason(rep({ stale: true, derived_from: "source" }), [])).toBe("Made from a file File Browser can't find in this asset");
});

test("the files emblem and its tooltip, from the listing's summary", () => {
  const entry = { name: "A.gltf", path: "x", is_dir: false, size: 0, modified: 0, is_hidden: false, is_symlink: false };
  expect(filesEmblemOf(entry)).toBe("none");
  expect(filesEmblemOf({ ...entry, representations: { count: 0, stale: 0, formats: [] } })).toBe("none");
  expect(filesEmblemOf({ ...entry, representations: { count: 2, stale: 0, formats: ["stl"] } })).toBe("fresh");
  expect(filesEmblemOf({ ...entry, representations: { count: 2, stale: 1, formats: ["stl"] } })).toBe("stale");
  expect(filesTitle({ count: 1, stale: 0 })).toBe("Has 1 other file");
  expect(filesTitle({ count: 3, stale: 1 })).toBe("Has 3 other files, 1 out of date");
});

test("Open With's candidates: the model when the app opens its format, then each file it opens (spec F8)", () => {
  const files = [rep({ id: "source", format: "blend", name: "B.blend" }), rep({ id: "stl" }), rep({ id: "gcode", format: "gcode", name: "B.gcode", stale: true })];
  expect(assetFormat("Box.GLTF")).toBe("gltf");
  expect(assetFormat("Box")).toBe("");
  const slicer = openWithCandidates("gltf", files, ["gcode", "stl", "3mf"]);
  expect(slicer.map((c) => (c.kind === "model" ? "model" : c.rep.id))).toEqual(["stl", "gcode"]);
  const blender = openWithCandidates("gltf", files, ["gltf", "blend"]);
  expect(blender.map((c) => (c.kind === "model" ? "model" : c.rep.id))).toEqual(["model", "source"]);
  // None declared: the model; one: it; several: ask.
  expect(onlyChoice([])).toEqual({ kind: "model" });
  expect(onlyChoice(slicer.slice(1))).toBe(slicer[1]);
  expect(onlyChoice(slicer)).toBeNull();
  // The first not out of date is chosen first; the model never is.
  const stale = openWithCandidates("gltf", files, ["gcode", "stl"]).reverse();
  expect(defaultChoice(stale)).toBe(1);
  expect(defaultChoice([{ kind: "file", rep: rep({ stale: true }) }])).toBe(0);
});

test("copy out: rows per format, sources in order, Keep Both names within the paste (spec F9)", async () => {
  expect(copyNameAt("Box.stl", 1)).toBe("Box (copy).stl");
  expect(copyNameAt("Box.stl", 3)).toBe("Box (copy 3).stl");
  expect(copyNameAt("Kit.d", 1, true)).toBe("Kit.d (copy)");
  expect(formatRowLabel("gcode", 3)).toBe("G-code (3 assets)");
  expect(formatRowLabel("stl", 1)).toBe("STL (1 asset)");
  const box = { path: "a/box", name: "Box.gltf", files: [rep({ name: "Box.stl", path: "a/box/stl" })] };
  const bracket = { path: "a/br", name: "Bracket.gltf", files: [rep({ id: "gcode", format: "gcode", name: "B.gcode", path: "a/br/g" }), rep({ name: "B.stl", path: "a/br/stl" })] };
  expect(formatsAcross([box, bracket])).toEqual([{ format: "stl", assets: 2 }, { format: "gcode", assets: 1 }]);
  expect(copySources([box, bracket], { model: true, formats: new Set(["stl"]) })).toEqual([
    { path: "a/box", name: "Box.gltf" }, { path: "a/box/stl", name: "Box.stl" }, { path: "a/br", name: "Bracket.gltf" }, { path: "a/br/stl", name: "B.stl" },
  ]);
  expect(copySources([bracket], { model: false, files: new Set([0]) })).toEqual([{ path: "a/br/g", name: "B.gcode" }]);
  // A repeat gets the next name free in the paste and the folder.
  const inFolder = new Set(["Box (copy).stl"]);
  expect(await uniqueNames(["Box.stl", "Box.gltf", "Box.stl", "Box.stl"], async (name) => inFolder.has(name))).toEqual([
    "Box.stl", "Box.gltf", "Box (copy 2).stl", "Box (copy 3).stl",
  ]);
  expect(await uniqueNames(["A.stl", "A (copy).stl", "A.stl"], async () => false)).toEqual(["A.stl", "A (copy).stl", "A (copy 2).stl"]);
});
