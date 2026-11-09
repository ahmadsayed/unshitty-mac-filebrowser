// Versions in the Space preview: one list, oldest first, whether it comes
// from a Keep Versions history or an Arbesk asset's saves, and the words the
// preview's version control uses. No DOM here; main.ts draws the control.
import type { ArbeskVersion, VersionInfo } from "./api";
import { formatDate } from "./format";
import type { MenuItem } from "./ui";

/** One version the preview can show. */
export interface PreviewVersion {
  /** The history's own id: a Keep Versions row id, or an Arbesk manifest CID. */
  id: string;
  /** When it was saved, in milliseconds, if known. */
  date: number | null;
  /** The file or asset as it is now. */
  current: boolean;
}

/** How many Keep Versions rows the preview reads, as Versions… does. */
export const LOCAL_VERSION_LIMIT = 1000;

/** A Keep Versions history (newest first, as `list_versions` gives it) as
 *  versions oldest first, without its deletion markers. */
export function fromLocal(rows: VersionInfo[]): PreviewVersion[] {
  return rows
    .filter((row) => !row.deleted)
    .map((row) => ({ id: String(row.id), date: row.mtime * 1000, current: row.current }))
    .reverse();
}

/** An Arbesk asset's saves (oldest first): the last one is the asset now. */
export function fromArbesk(rows: ArbeskVersion[]): PreviewVersion[] {
  return rows.map((row, i) => ({ id: row.id, date: row.timestamp, current: i === rows.length - 1 }));
}

/** "Version 3 of 12": `index` counts from the oldest, which is 0. */
export function versionTitle(index: number, total: number): string {
  return `Version ${index + 1} of ${total}`;
}

/** When a version was saved, as the file list writes dates; "" when unknown. */
export function versionDate(version: PreviewVersion): string {
  return version.date === null ? "" : formatDate(Math.round(version.date / 1000));
}

/** The version list's menu, newest first, the shown one marked. */
export function versionMenu(versions: PreviewVersion[], shown: number, pick: (index: number) => void): MenuItem[] {
  return versions
    .map((version, index) => ({
      label: version.current ? `Version ${index + 1} — Current` : `Version ${index + 1}`,
      accel: versionDate(version),
      radio: index === shown,
      action: () => pick(index),
    }))
    .reverse();
}
