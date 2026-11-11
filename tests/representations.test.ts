// The words for an Arbesk asset's other files (src/representations.ts).
import { expect, test } from "vitest";
import type { RepresentationInfo } from "../src/api";
import { fileSubtitle, formatLabel, profileText, staleReason, statsText, toolText } from "../src/representations";
import { filesEmblemOf, filesTitle } from "../src/icons";

const rep = (over: Partial<RepresentationInfo>): RepresentationInfo => ({
  id: "stl", role: "mesh", format: "stl", name: "A.stl", bytes: 1000, stale: false, derived_from: null,
  tool: null, profile: null, stats: null, created_at: 0, ...over,
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
