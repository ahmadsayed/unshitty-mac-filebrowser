// The dialogs that show an Arbesk asset's other files: the Properties
// dialog's Files… page (spec F7: each row's Open With… and Copy, F8 and
// F9; Remove… comes with adding files), the choice of which file an
// application opens (F8) and of which files a copy takes out (F9). Built
// with ui.ts' openDialog and openMenu; the words and rules come from
// representations.ts.

import type { Entry, RepresentationInfo } from "./api";
import { entryIcon, sym } from "./icons";
import { defaultChoice, fileSubtitle, formatRowLabel, formatsAcross, staleReason, type AssetFiles, type CopyChoice, type FileChoice } from "./representations";
import { openDialog, openMenu, type DialogHandle } from "./ui";

/** A row of the boxed list: the file kind's icon, a title, a subtitle and,
 *  when out of date, its tag. Text goes in with textContent only. A
 *  `control` (a radio or check box) makes the whole row a label for it. */
function fileRow(iconOf: Entry | null, title: string, subtitle: string, staleWhy: string | null, control?: HTMLInputElement): HTMLElement {
  const row = document.createElement(control ? "label" : "div");
  row.className = control ? "action-row files-row choice-row" : "action-row files-row";
  const icon = iconOf ? `<span class="files-row-icon">${entryIcon(iconOf)}</span>` : "";
  row.innerHTML = `${icon}<span class="action-row-text"><span class="action-row-title"></span><span class="action-row-subtitle"></span></span>`;
  row.querySelector(".action-row-title")!.textContent = title;
  const sub = row.querySelector<HTMLElement>(".action-row-subtitle")!;
  sub.textContent = subtitle;
  sub.hidden = subtitle === "";
  if (control) row.prepend(control);
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

/** A boxed list for a dialog's page. */
function boxedList(label: string): HTMLElement {
  const list = document.createElement("div");
  list.className = "boxed-list";
  list.setAttribute("role", "list");
  list.setAttribute("aria-label", label);
  return list;
}

/** A choice's row parts: the icon's entry, its title, subtitle and why it is out of date. */
function choiceParts(asset: Entry, choice: FileChoice, files: readonly RepresentationInfo[]): [Entry, string, string, string | null] {
  if (choice.kind === "model") return [asset, asset.name, "Model", null];
  const { rep } = choice;
  return [iconEntry(rep.name), rep.name, fileSubtitle(rep), rep.stale ? staleReason(rep, files) : null];
}

/** What the Files… page's rows do: Open With… and Copy, for the model and each file. */
export interface FilesActions {
  openWith(choice: FileChoice): void;
  copy(choice: FileChoice): void;
}

/** A row's menu button (`view-more`), opening Open With… and Copy. */
function rowMenu(name: string, choice: FileChoice, actions: FilesActions): HTMLButtonElement {
  const button = document.createElement("button");
  button.className = "flat-btn files-row-menu";
  button.title = "Menu";
  button.setAttribute("aria-label", `Menu for “${name}”`);
  button.innerHTML = sym("view-more");
  button.addEventListener("click", () =>
    openMenu(button, [
      { label: "Open With…", action: () => actions.openWith(choice) },
      { label: "Copy", action: () => actions.copy(choice) },
    ])
  );
  return button;
}

/** Files of “Box.gltf”: the model first, then each other file in stored
 *  order with its subtitle and, when out of date, a tag saying why; each
 *  row's menu offers Open With… and Copy. */
export function showFilesDialog(asset: Entry, files: readonly RepresentationInfo[], actions: FilesActions): DialogHandle {
  const extra = document.createElement("div");
  extra.className = "files-page";
  const list = boxedList("Files");
  const choices: FileChoice[] = [{ kind: "model" }, ...files.map((rep): FileChoice => ({ kind: "file", rep }))];
  for (const choice of choices) {
    const [icon, title, subtitle, staleWhy] = choiceParts(asset, choice, files);
    const row = fileRow(icon, title, subtitle, staleWhy);
    row.appendChild(rowMenu(title, choice, actions));
    list.appendChild(row);
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

/** Open Which File With PrusaSlicer?: a radio row per file the
 *  application opens, the first one not out of date selected. Resolves
 *  with the chosen file, or null on Cancel or Escape. */
export function chooseFileToOpen(asset: Entry, appName: string, candidates: readonly FileChoice[], files: readonly RepresentationInfo[]): Promise<FileChoice | null> {
  return new Promise((resolve) => {
    const extra = document.createElement("div");
    extra.className = "files-page";
    const list = boxedList("Files");
    list.setAttribute("role", "radiogroup");
    const selected = defaultChoice(candidates);
    const radios = candidates.map((choice, i) => {
      const radio = document.createElement("input");
      radio.type = "radio";
      radio.name = "open-which-file";
      radio.checked = i === selected;
      const [icon, title, subtitle, staleWhy] = choiceParts(asset, choice, files);
      list.appendChild(fileRow(icon, title, subtitle, staleWhy, radio));
      return radio;
    });
    extra.appendChild(list);
    openDialog({
      heading: `Open Which File With ${appName}?`,
      className: "files-dialog",
      extra,
      responses: [
        { id: "cancel", label: "Cancel" },
        { id: "open", label: "Open", style: "suggested" },
      ],
      defaultResponse: "open",
      closeResponse: "cancel",
      onResponse: (id) => {
        const at = radios.findIndex((radio) => radio.checked);
        resolve(id === "open" && at >= 0 ? candidates[at] : null);
      },
    });
  });
}

/** A check box for the copy dialog's rows. */
function checkBox(checked: boolean): HTMLInputElement {
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = checked;
  return box;
}

/** Copy Which Files of “Box.gltf”? (one asset: a row per file) or Copy
 *  Which Files? (several: Models and a row per format, applied to each),
 *  with All Files above, checked when every row is, mixed when some are.
 *  Copy is insensitive while nothing is checked. Resolves with the
 *  choice, or null on Cancel or Escape. */
export function chooseFilesToCopy(assets: readonly AssetFiles[], iconOf: (asset: AssetFiles) => Entry): Promise<CopyChoice | null> {
  return new Promise((resolve) => {
    const one = assets.length === 1 ? assets[0] : null;
    const extra = document.createElement("div");
    extra.className = "files-page";
    const list = boxedList("Files");
    const all = checkBox(false);
    list.appendChild(fileRow(null, "All Files", "", null, all));
    all.closest("label")!.classList.add("all-files-row");
    const model = checkBox(true);
    const rows: HTMLInputElement[] = [model];
    if (one) {
      list.appendChild(fileRow(iconOf(one), one.name, "Model", null, model));
      for (const rep of one.files) {
        const box = checkBox(false);
        list.appendChild(fileRow(iconEntry(rep.name), rep.name, fileSubtitle(rep), rep.stale ? staleReason(rep, one.files) : null, box));
        rows.push(box);
      }
    } else {
      list.appendChild(fileRow(iconOf(assets[0]), "Models", "", null, model));
      for (const { format, assets: n } of formatsAcross(assets)) {
        const box = checkBox(false);
        box.dataset.format = format;
        list.appendChild(fileRow(iconEntry(`x.${format}`), formatRowLabel(format, n), "", null, box));
        rows.push(box);
      }
    }
    extra.appendChild(list);
    let dialog: DialogHandle | undefined;
    const sync = () => {
      const checked = rows.filter((box) => box.checked).length;
      all.checked = checked === rows.length;
      all.indeterminate = checked > 0 && checked < rows.length;
      dialog?.setResponseEnabled("copy", checked > 0);
    };
    all.addEventListener("change", () => {
      const every = rows.every((box) => box.checked);
      for (const box of rows) box.checked = !every;
      sync();
    });
    for (const box of rows) box.addEventListener("change", sync);
    dialog = openDialog({
      heading: one ? `Copy Which Files of “${one.name}”?` : "Copy Which Files?",
      className: "files-dialog",
      extra,
      responses: [
        { id: "cancel", label: "Cancel" },
        { id: "copy", label: "Copy", style: "suggested" },
      ],
      defaultResponse: "copy",
      closeResponse: "cancel",
      onResponse: (id) => {
        if (id !== "copy") return resolve(null);
        const chosen = rows.slice(1).filter((box) => box.checked);
        resolve(
          one
            ? { model: model.checked, files: new Set(rows.slice(1).flatMap((box, i) => (box.checked ? [i] : []))) }
            : { model: model.checked, formats: new Set(chosen.map((box) => box.dataset.format!)) }
        );
      },
    });
    sync();
  });
}
