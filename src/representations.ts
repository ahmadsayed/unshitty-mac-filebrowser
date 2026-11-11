// An Arbesk asset's other files (its representations) as the window words
// them: format labels, the roles' words, the subtitle a file's row carries
// and why one is out of date (spec F7). Pure rules, no DOM: the dialogs
// that show them are representation-dialogs.ts, tested here directly
// (tests/representations.test.ts).

import type { RepresentationInfo } from "./api";
import { formatSize } from "./format";

/** besk's `FORMAT_LABELS` (packages/besk/src/representations.ts); any
 *  other format is shown in capitals. */
const FORMAT_LABELS: Record<string, string> = {
  gcode: "G-code", bgcode: "binary G-code", blend: "Blender file", stl: "STL",
  obj: "OBJ", "3mf": "3MF", step: "STEP", stp: "STEP", glb: "GLB", gltf: "glTF",
  usdz: "USDZ", f3d: "Fusion file", fcstd: "FreeCAD file", scad: "OpenSCAD file",
};

/** A format's label: "G-code", "Blender file", or the format in capitals. */
export function formatLabel(format: string): string {
  return FORMAT_LABELS[format] ?? format.toUpperCase();
}

/** What each role means, in the row's words. */
export const ROLE_WORDS: Record<RepresentationInfo["role"], string> = {
  source: "Source",
  mesh: "Mesh",
  view: "For viewing",
  machine: "For the printer",
  other: "Other",
};

type Detail = Record<string, string | number> | null;

const present = (value: string | number | undefined): value is string | number => value !== undefined && value !== "";

/** The tool a file was made with: "PrusaSlicer 2.9.2" from its `name` and
 *  `version`; a tool recorded otherwise shows its values in order. */
export function toolText(tool: Detail): string {
  if (!tool) return "";
  const named = [tool.name, tool.version].filter(present);
  return (named.length ? named : Object.values(tool).filter(present)).join(" ");
}

/** The profile a file was sliced with, as `besk attach` records it:
 *  "Original Prusa MK4, PETG, 0.2 mm layers, 0.4 mm nozzle". Other keys
 *  are left out. */
export function profileText(profile: Detail): string {
  if (!profile) return "";
  const parts = [
    profile.printer,
    profile.material,
    present(profile.layer_mm) ? `${profile.layer_mm} mm layers` : undefined,
    present(profile.nozzle_mm) ? `${profile.nozzle_mm} mm nozzle` : undefined,
  ];
  return parts.filter(present).join(", ");
}

/** "2 h 3 min", "45 min", "3 h". */
function duration(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  const [h, m] = [Math.floor(minutes / 60), minutes % 60];
  if (h === 0) return `${m} min`;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** A sliced file's stats: the print time ("2 h 3 min", from
 *  `print_time_s`) and the filament it takes ("38.5 g", from
 *  `filament_g`). Other keys are left out. */
export function statsText(stats: Detail): string[] {
  if (!stats) return [];
  const parts: string[] = [];
  if (typeof stats.print_time_s === "number" && stats.print_time_s >= 0) parts.push(duration(stats.print_time_s));
  if (typeof stats.filament_g === "number" && stats.filament_g >= 0) parts.push(`${stats.filament_g} g`);
  return parts;
}

/** A file's subtitle, joined with "·": its format, size and role, then
 *  the tool, the profile and the stats when present. */
export function fileSubtitle(rep: RepresentationInfo): string {
  const parts = [formatLabel(rep.format), formatSize(rep.bytes), ROLE_WORDS[rep.role] ?? "", toolText(rep.tool), profileText(rep.profile), ...statsText(rep.stats)];
  return parts.filter((part) => part !== "").join(" · ");
}

/** Why `rep` is out of date, for its tag's tooltip: made from an earlier
 *  model, from an earlier version of another of the asset's files (by its
 *  id), or from a file this list doesn't hold (gone, or an entry File
 *  Browser can't read: true either way). */
export function staleReason(rep: RepresentationInfo, listed: readonly RepresentationInfo[]): string {
  if (rep.derived_from === "primary") return "Made from an earlier version of the model";
  if (rep.derived_from !== null && listed.some((other) => other.id === rep.derived_from)) {
    return `Made from an earlier “${rep.derived_from}”`;
  }
  return "Made from a file File Browser can't find in this asset";
}
