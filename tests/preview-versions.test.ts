// One list of versions for the Space preview, from a Keep Versions history or
// an Arbesk asset's saves (src/preview-versions.ts).
import { expect, test, vi } from "vitest";
import { fromArbeskSaves, fromLocal, versionMenu, versionTitle, type PreviewVersion } from "../src/preview-versions";
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
    { id: "1", date: 100_000, current: false },
    { id: "2", date: 200_000, current: false },
    { id: "3", date: 300_000, current: true },
  ]);
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
