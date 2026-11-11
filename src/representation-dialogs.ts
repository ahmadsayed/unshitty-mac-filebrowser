// The dialogs that show an Arbesk asset's other files (spec F7): the
// Properties dialog's Files… page, read only for now (each row's Open
// With… and Copy come with copy-out, Remove… with adding files). Built
// with ui.ts' openDialog; the words come from representations.ts.

import type { Entry, RepresentationInfo } from "./api";
import { entryIcon } from "./icons";
import { fileSubtitle, staleReason } from "./representations";
import { openDialog, type DialogHandle } from "./ui";

/** A row of the boxed list: the file kind's icon, a title, a subtitle and,
 *  when out of date, its tag. Text goes in with textContent only. */
function fileRow(iconOf: Entry, title: string, subtitle: string, staleWhy: string | null): HTMLElement {
  const row = document.createElement("div");
  row.className = "action-row files-row";
  row.innerHTML = `<span class="files-row-icon">${entryIcon(iconOf)}</span><span class="action-row-text"><span class="action-row-title"></span><span class="action-row-subtitle"></span></span>`;
  row.querySelector(".action-row-title")!.textContent = title;
  row.querySelector(".action-row-subtitle")!.textContent = subtitle;
  if (staleWhy !== null) {
    const tag = document.createElement("span");
    tag.className = "out-of-date-tag";
    tag.textContent = "Out of date";
    tag.title = staleWhy;
    row.appendChild(tag);
  }
  return row;
}

/** An entry shaped only to pick a file's icon by its name. */
const iconEntry = (name: string): Entry => ({ name, path: name, is_dir: false, size: 0, modified: 0, is_hidden: false, is_symlink: false });

/** Files of “Box.gltf”: the model first, then each other file in stored
 *  order with its subtitle and, when out of date, a tag saying why. */
export function showFilesDialog(asset: Entry, files: readonly RepresentationInfo[]): DialogHandle {
  const extra = document.createElement("div");
  extra.className = "files-page";
  const list = document.createElement("div");
  list.className = "boxed-list";
  list.setAttribute("role", "list");
  list.setAttribute("aria-label", "Files");
  list.appendChild(fileRow(asset, asset.name, "Model", null));
  for (const rep of files) {
    list.appendChild(fileRow(iconEntry(rep.name), rep.name, fileSubtitle(rep), rep.stale ? staleReason(rep, files) : null));
  }
  extra.appendChild(list);
  return openDialog({
    heading: `Files of “${asset.name}”`,
    className: "files-dialog",
    extra,
    responses: [{ id: "close", label: "Close" }],
    defaultResponse: "close",
    closeResponse: "close",
  });
}
