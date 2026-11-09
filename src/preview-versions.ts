// Versions in the Space preview: one list, oldest first, whether it comes
// from a Keep Versions history or an Arbesk asset's saves, and the control
// in the preview's header that steps through it. main.ts shows the version
// picked.
import type { ArbeskVersion, VersionInfo } from "./api";
import { formatDate } from "./format";
import { sym } from "./icons";
import { openMenu, type MenuItem } from "./ui";

/** One version the preview can show. */
export interface PreviewVersion {
  /** The history's own id: a Keep Versions row id, or an Arbesk manifest CID. */
  id: string;
  /** When it was saved, in milliseconds, if known. */
  date: number | null;
  /** The file or asset as it is now. */
  current: boolean;
  /** Its size in bytes, when the history says. */
  size?: number;
}

/** How many Keep Versions rows the preview reads, as Versions… does. */
export const LOCAL_VERSION_LIMIT = 1000;

/** The id of the file on disk when no captured version matches it. */
export const ON_DISK = "current";

/** A Keep Versions history (newest first, as `list_versions` gives it) as
 *  versions oldest first, without its deletion markers. When no captured
 *  version is the file as it is now (changed since, recreated, or too large
 *  to capture), the file on disk, last modified at `modified` (seconds), is
 *  the newest. */
export function fromLocal(rows: VersionInfo[], modified = 0): PreviewVersion[] {
  const versions: PreviewVersion[] = rows
    .filter((row) => !row.deleted)
    .map((row) => ({ id: String(row.id), date: row.mtime * 1000, current: row.current, size: row.size }))
    .reverse();
  if (versions.length && !versions.some((v) => v.current)) versions.push({ id: ON_DISK, date: modified * 1000, current: true });
  return versions;
}

/** An Arbesk asset's saves (oldest first): the last one is the asset now. */
export function fromArbeskSaves(rows: ArbeskVersion[]): PreviewVersion[] {
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

/** One header button with only an icon: its words go to its tooltip and to screen readers. */
function stepButton(icon: string, label: string, disabled: boolean, onClick: () => void): HTMLButtonElement {
  const button = document.createElement("button");
  button.className = "flat-btn";
  button.innerHTML = sym(icon);
  button.title = label;
  button.setAttribute("aria-label", label);
  button.disabled = disabled;
  button.addEventListener("click", onClick);
  return button;
}

/**
 * Draw the control in `host`, the header's centred title: Older Version, the
 * item's `name` over "Version N of M · date" (a button opening the list),
 * Newer Version. `more` says the list holds only the newest versions. `pick`
 * shows another one.
 */
export function drawVersionControl(
  host: HTMLElement,
  name: string,
  versions: PreviewVersion[],
  shown: number,
  more: boolean,
  pick: (index: number) => void,
): void {
  const label = document.createElement("button");
  label.className = "flat-btn preview-version-label";
  label.setAttribute("aria-haspopup", "menu");
  label.innerHTML = `<span class="preview-title-name"></span><span class="preview-version-subtitle"><span class="preview-version-title"></span><span class="preview-version-date"></span></span>`;
  label.querySelector(".preview-title-name")!.textContent = name;
  label.querySelector(".preview-version-title")!.textContent = versionTitle(shown, versions.length);
  label.querySelector(".preview-version-date")!.textContent = versionDate(versions[shown]);
  const note: MenuItem[] = more ? [{ heading: `Showing the newest ${LOCAL_VERSION_LIMIT.toLocaleString("en-US")} versions.` }] : [];
  label.addEventListener("click", () => openVersionMenu(label, [...note, ...versionMenu(versions, shown, pick)]));
  const buttons = [
    stepButton("go-previous", "Older Version", shown === 0, () => pick(shown - 1)),
    label,
    stepButton("go-next", "Newer Version", shown === versions.length - 1, () => pick(shown + 1)),
  ];
  // Keep the keyboard where it was: the same button, or the list when that
  // button is now disabled at an end of the history.
  const focused = [...host.children].indexOf(document.activeElement as Element);
  host.replaceChildren(...buttons);
  host.hidden = false;
  if (focused >= 0) (buttons[focused].disabled ? label : buttons[focused]).focus();
}

/** The list of versions, scrolled to the one shown. */
function openVersionMenu(label: HTMLElement, items: MenuItem[]): void {
  openMenu(label, items, { className: "preview-version-menu" });
  const shown = document.querySelector(".preview-version-menu .radio-dot")?.closest("button");
  shown?.scrollIntoView?.({ block: "nearest" });
}
