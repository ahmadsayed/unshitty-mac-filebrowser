// An Arbesk asset's other files (its representations) as the window words
// them: format labels, the roles' words, the subtitle a file's row carries
// and why one is out of date (spec F7). Pure rules, no DOM: the dialogs
// that show them are representation-dialogs.ts, tested here directly
// (tests/representations.test.ts).

import type { RepresentationInfo, RepresentationsSummary } from "./api";
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

// ---------------------------------------------------------------------------
// Open With (spec F8)
// ---------------------------------------------------------------------------

/** One of an asset's files a task can take: its model, or another file. */
export type FileChoice = { kind: "model" } | { kind: "file"; rep: RepresentationInfo };

/** The model's format: its listed name's extension (`gltf`, `3mf`). */
export function assetFormat(assetName: string): string {
  const dot = assetName.lastIndexOf(".");
  return dot > 0 ? assetName.slice(dot + 1).toLowerCase() : "";
}

/** The asset's files an application opens, by the formats it declares
 *  (`opens`, from `app_formats`): the model first when its format is one,
 *  then each file whose format is, in stored order. */
export function openWithCandidates(model: string, files: readonly RepresentationInfo[], opens: readonly string[]): FileChoice[] {
  const choices: FileChoice[] = opens.includes(model) ? [{ kind: "model" }] : [];
  for (const rep of files) if (opens.includes(rep.format)) choices.push({ kind: "file", rep });
  return choices;
}

/** Which file an application gets without asking: the model when it
 *  declares none of them (as GNOME Files hands any file to the app the
 *  user chose), the one it opens, or null when it opens several. */
export function onlyChoice(candidates: readonly FileChoice[]): FileChoice | null {
  if (candidates.length === 0) return { kind: "model" };
  return candidates.length === 1 ? candidates[0] : null;
}

/** The choice selected first: the first that is not out of date (the
 *  model never is), else the first. */
export function defaultChoice(candidates: readonly FileChoice[]): number {
  const fresh = candidates.findIndex((choice) => choice.kind === "model" || !choice.rep.stale);
  return Math.max(0, fresh);
}

// ---------------------------------------------------------------------------
// Copy out (spec F9)
// ---------------------------------------------------------------------------

/** "name (copy).txt", then "name (copy 2).txt", …: Keep Both's `n`th name. */
export function copyNameAt(name: string, n: number, isDir = false): string {
  const dot = isDir ? -1 : name.lastIndexOf(".");
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
  return `${stem} (copy${n > 1 ? ` ${n}` : ""})${ext}`;
}

/** "G-code (3 assets)", "STL (1 asset)": a format's row when several assets are copied. */
export function formatRowLabel(format: string, assets: number): string {
  return `${formatLabel(format)} (${assets} ${assets === 1 ? "asset" : "assets"})`;
}

/** One asset of a copy out, with its other files. */
export interface AssetFiles {
  path: string;
  name: string;
  files: readonly RepresentationInfo[];
}

/** What a copy dialog chose: for one asset, the model and each file by
 *  index; for several, the models and each format. */
export interface CopyChoice {
  model: boolean;
  /** One asset: the chosen files' indexes in its list. */
  files?: ReadonlySet<number>;
  /** Several assets: the chosen formats. */
  formats?: ReadonlySet<string>;
}

/** One asset of a copy out by its other files' distinct formats, in
 *  stored order: as its full list gives them, or its listing's summary
 *  (performance plan, phase 4). */
export interface AssetFormats {
  path: string;
  name: string;
  formats: readonly string[];
}

/** `asset`'s formats as its full list gives them. */
export function formatsOfFiles(asset: AssetFiles): AssetFormats {
  return { path: asset.path, name: asset.name, formats: [...new Set(asset.files.map((rep) => rep.format))] };
}

/** The asset at `path` by the formats its listing's summary names (none
 *  when the listing gave it no summary: it has no other files). */
export function formatsOfSummary(path: string, name: string, summary: RepresentationsSummary | undefined): AssetFormats {
  return { path, name, formats: summary?.formats ?? [] };
}

/** The formats present across `assets`, in first-seen order, with how
 *  many assets hold each: the several-asset copy dialog's rows. */
export function formatCounts(assets: readonly AssetFormats[]): Array<{ format: string; assets: number }> {
  const counts = new Map<string, number>();
  for (const asset of assets) {
    for (const format of new Set(asset.formats)) counts.set(format, (counts.get(format) ?? 0) + 1);
  }
  return [...counts].map(([format, n]) => ({ format, assets: n }));
}

/** The formats present across `assets`' full lists (`formatCounts`). */
export function formatsAcross(assets: readonly AssetFiles[]): Array<{ format: string; assets: number }> {
  return formatCounts(assets.map(formatsOfFiles));
}

/** Whether a copy that chose `choice` takes any file of an asset whose
 *  summary is `summary`: only then is its full list read. */
export function choiceTakesFrom(summary: RepresentationsSummary | undefined, choice: CopyChoice): boolean {
  return !!summary && summary.count > 0 && summary.formats.some((format) => !!choice.formats?.has(format));
}

/** A copy's sources and their target names: per asset, in order, its path
 *  (the model, when chosen) and each chosen file's pinned path. A file is
 *  named as listed; the model by the asset's listed name. */
export function copySources(assets: readonly AssetFiles[], choice: CopyChoice): Array<{ path: string; name: string }> {
  const sources: Array<{ path: string; name: string }> = [];
  for (const asset of assets) {
    if (choice.model) sources.push({ path: asset.path, name: asset.name });
    asset.files.forEach((rep, i) => {
      const chosen = choice.files ? choice.files.has(i) : !!choice.formats?.has(rep.format);
      if (chosen) sources.push({ path: rep.path, name: rep.name });
    });
  }
  return sources;
}

/** The names repeated within one paste after their first: each gets the
 *  next Keep Both name `taken` (the folder, asked by the caller) and the
 *  paste's other names leave free. Names are compared ignoring case, as a
 *  macOS volume does: on a case-sensitive one that only renames a file
 *  that could have kept its name, never loses one. */
export async function uniqueNames(names: readonly string[], taken: (name: string) => Promise<boolean>): Promise<string[]> {
  const key = (name: string) => name.toLowerCase();
  const used = new Set(names.map(key));
  const seen = new Set<string>();
  const out: string[] = [];
  for (const name of names) {
    if (!seen.has(key(name))) {
      seen.add(key(name));
      out.push(name);
      continue;
    }
    let n = 1;
    let candidate = copyNameAt(name, n);
    while (used.has(key(candidate)) || (await taken(candidate))) candidate = copyNameAt(name, ++n);
    used.add(key(candidate));
    out.push(candidate);
  }
  return out;
}
