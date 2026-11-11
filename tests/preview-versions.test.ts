// One list of versions for the Space preview, from a Keep Versions history or
// an Arbesk asset's saves (src/preview-versions.ts).
import { expect, test, vi } from "vitest";
import { changesText, filesMenu, fromArbeskSaves, fromLocal, shownFileLabel, versionMenu, versionTitle, type PreviewVersion } from "../src/preview-versions";
import type { RepresentationInfo } from "../src/api";
import type { VersionInfo } from "../src/api";

const row = (id: number, mtime: number, over: Partial<VersionInfo> = {}): VersionInfo => ({
  id,
  size: 10,
  mtime,
  recorded_at: mtime,
  deleted: false,
  current: false,
  ...over,
});

test("deletion markers are not versions", () => {
  // Newest first, as the backend lists them.
  const rows = [row(3, 300, { current: true }), row(2, 200), row(0, 0, { deleted: true, recorded_at: 150 }), row(1, 100)];
  expect(fromLocal(rows)).toEqual([
    { id: "1", date: 100_000, current: false, size: 10 },
    { id: "2", date: 200_000, current: false, size: 10 },
    { id: "3", date: 300_000, current: true, size: 10 },
  ]);
});

test("the file on disk is the current version when no captured one matches it", () => {
  // Newest row a deletion marker: the file was recreated and not captured yet.
  const rows = [row(0, 0, { deleted: true, recorded_at: 400 }), row(2, 200), row(1, 100)];
  const versions = fromLocal(rows, 500);
  expect(versions.map((v) => v.id)).toEqual(["1", "2", "current"]);
  expect(versions.at(-1)).toMatchObject({ date: 500_000, current: true });
  // With a current row, nothing is added.
  expect(fromLocal([row(2, 200, { current: true }), row(1, 100)], 500).map((v) => v.id)).toEqual(["1", "2"]);
});

test("Arbesk versions: the newest is current", () => {
  expect(fromArbeskSaves([{ id: "a", timestamp: 5 }, { id: "b", timestamp: null }])).toEqual([
    { id: "a", date: 5, current: false },
    { id: "b", date: null, current: true },
  ]);
  expect(fromArbeskSaves([])).toEqual([]);
});

test("titles count from the oldest", () => {
  expect(versionTitle(2, 12)).toBe("Version 3 of 12");
  expect(versionTitle(0, 1)).toBe("Version 1 of 1");
});

test("the menu lists newest first and marks the shown one", () => {
  const versions: PreviewVersion[] = [
    { id: "a", date: null, current: false },
    { id: "b", date: Date.UTC(2026, 10, 8, 12), current: false },
    { id: "c", date: null, current: true },
  ];
  const pick = vi.fn();
  const items = versionMenu(versions, 1, pick) as Array<{ label: string; radio: boolean; accel: string; action(): void }>;
  expect(items.map((i) => i.label)).toEqual(["Version 3 — Current", "Version 2", "Version 1"]);
  expect(items.map((i) => i.radio)).toEqual([false, true, false]);
  expect(items[1].accel).not.toBe("");
  expect(items[2].accel).toBe("");
  items[1].action();
  expect(pick).toHaveBeenCalledWith(1);
});

// ---- An Arbesk save's other files (spec F14) ----

const rep = (id: string, format: string, extra: Partial<RepresentationInfo> = {}): RepresentationInfo => ({
  id, role: "mesh", format, name: `Box.${format}`, bytes: 2048, stale: false, derived_from: null,
  tool: null, profile: null, stats: null, created_at: 0, path: `arbesk://x/1~S/a~Box.gltf/${id}~Box.${format}`, ...extra,
});

test("a save's rows say what it changed in the files, joined with ·", () => {
  const saves = fromArbeskSaves([{ id: "a", timestamp: null, changes: ["Added Blender file", "Added G-code (Original Prusa MK4, PETG)"] }, { id: "b", timestamp: null }]);
  expect(changesText(saves[0])).toBe("Added Blender file · Added G-code (Original Prusa MK4, PETG)");
  expect(changesText(saves[1])).toBe("");
  // A save without changes has no key, as before.
  expect(saves[1]).toEqual({ id: "b", date: null, current: true });
  const items = versionMenu(saves, 1, () => {}) as Array<{ subtitle?: string }>;
  expect(items.map((i) => i.subtitle)).toEqual([undefined, "Added Blender file · Added G-code (Original Prusa MK4, PETG)"]);
});

test("the switcher says what is shown and lists the model, then each file with its subtitle", () => {
  expect(shownFileLabel(null)).toBe("Model");
  expect(shownFileLabel(rep("stl", "stl"))).toBe("STL");
  expect(shownFileLabel(rep("gcode", "gcode"))).toBe("G-code");
  expect(shownFileLabel(rep("source", "blend"))).toBe("Blender file");
  expect(shownFileLabel(rep("ply", "ply"))).toBe("PLY");
  const pick = vi.fn();
  const items = filesMenu("Box.gltf", [rep("stl", "stl"), rep("source", "blend", { role: "source" })], 1, pick) as Array<{ label: string; subtitle: string; radio: boolean; action(): void }>;
  expect(items.map((i) => [i.label, i.subtitle, i.radio])).toEqual([
    ["Box.gltf", "Model", false],
    ["Box.stl", "STL · 2.0 kB · Mesh", false],
    ["Box.blend", "Blender file · 2.0 kB · Source", true],
  ]);
  items[0].action();
  items[2].action();
  expect(pick.mock.calls).toEqual([[-1], [1]]);
});
