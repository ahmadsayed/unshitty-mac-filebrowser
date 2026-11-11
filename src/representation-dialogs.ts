// The dialogs that show an Arbesk asset's other files: the Properties
// dialog's Files… page (spec F7: each row's Open With…, Copy and Remove…,
// F8, F9 and F12), the choice of which file an application opens (F8), of
// which files a copy takes out (F9), and adding files and removing one
// (F10, F12: the confirmations and the id clash). Built
// with ui.ts' openDialog and openMenu; the words and rules come from
// representations.ts.

import type { Entry, RepresentationInfo } from "./api";
import { entryIcon, sym } from "./icons";
import { defaultChoice, fileSubtitle, formatCounts, formatRowLabel, formatsOfFiles, staleReason, type AssetFiles, type AssetFormats, type CopyChoice, type FileChoice } from "./representations";
import { applyToAllRow, openDialog, openMenu, type DialogHandle } from "./ui";

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
export const iconEntry = (name: string): Entry => ({ name, path: name, is_dir: false, size: 0, modified: 0, is_hidden: false, is_symlink: false });

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

/** What the Files… page's rows do: Open With… and Copy, for the model and
 *  each file; Remove…, for each file. */
export interface FilesActions {
  openWith(choice: FileChoice): void;
  copy(choice: FileChoice): void;
  remove?(rep: RepresentationInfo): void;
}

/** A row's menu button (`view-more`), opening Open With…, Copy and, for a
 *  file, Remove…. */
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
      ...(choice.kind === "file" && actions.remove
        ? [{ separator: true } as const, { label: "Remove…", action: () => actions.remove!(choice.rep) }]
        : []),
    ])
  );
  return button;
}

/** An open Files… page: its dialog, and how to draw it again from the
 *  asset's files after a write (spec F11). */
export interface FilesPage {
  dialog: DialogHandle;
  redraw(files: readonly RepresentationInfo[]): void;
}

/** Files of “Box.gltf”: the model first, then each other file in stored
 *  order with its subtitle and, when out of date, a tag saying why; each
 *  row's menu offers Open With…, Copy and, for a file, Remove…. A dim line
 *  below says how to add files. */
export function showFilesDialog(asset: Entry, files: readonly RepresentationInfo[], actions: FilesActions): FilesPage {
  const extra = document.createElement("div");
  extra.className = "files-page";
  const list = boxedList("Files");
  const fill = (shown: readonly RepresentationInfo[]) => {
    const choices: FileChoice[] = [{ kind: "model" }, ...shown.map((rep): FileChoice => ({ kind: "file", rep }))];
    list.replaceChildren(
      ...choices.map((choice) => {
        const [icon, title, subtitle, staleWhy] = choiceParts(asset, choice, shown);
        const row = fileRow(icon, title, subtitle, staleWhy);
        row.appendChild(rowMenu(title, choice, actions));
        return row;
      })
    );
  };
  fill(files);
  const hint = document.createElement("p");
  hint.className = "files-hint";
  hint.textContent = "Paste or drop files onto the asset to add them.";
  extra.append(list, hint);
  const dialog = openDialog({
    heading: `Files of “${asset.name}”`,
    className: "files-dialog",
    extra,
    responses: [{ id: "close", label: "Close" }],
    defaultResponse: "close",
    closeResponse: "close",
  });
  return { dialog, redraw: (shown) => list.isConnected && fill(shown) };
}

/** Remove “Bracket.gcode” From “Box.gltf”? (spec F12): Cancel is the
 *  default; Remove runs `remove` in the dialog's busy mode, Cancel staying
 *  live (`cancel` stops the write while it can) and a failure shown in the
 *  dialog in words. */
export function confirmRemoveFile(assetName: string, rep: RepresentationInfo, remove: () => Promise<unknown>, cancel: () => void): DialogHandle {
  const status = document.createElement("div");
  status.className = "field-message error";
  status.setAttribute("role", "status");
  let dialog: DialogHandle | undefined = undefined;
  dialog = openDialog({
    heading: `Remove “${rep.name}” From “${assetName}”?`,
    body: "Arbesk keeps it in the asset's earlier versions.",
    extra: status,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "remove", label: "Remove", style: "destructive" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    busy: { liveWhileBusy: ["cancel"], status },
    onResponse: (id) => {
      if (id === "remove") {
        dialog?.setStatus("Removing…");
        return remove().then(() => undefined);
      }
      if (dialog?.busy) cancel();
    },
  });
  return dialog;
}

/** A paragraph of a dialog's text. */
function paragraph(text: string): HTMLElement {
  const p = document.createElement("p");
  p.className = "dialog-text";
  p.textContent = text;
  return p;
}

/** Add to “Box.gltf”? (spec F10), asked before every paste or drop onto an
 *  asset: the files go public; a source file can hold more than the model
 *  (Arbesk R7), said of each. Cancel is the default. */
export function confirmAttach(assetName: string, sources: readonly string[]): Promise<boolean> {
  return new Promise((resolve) => {
    const extra = document.createElement("div");
    extra.className = "dialog-paragraphs";
    for (const name of sources) {
      extra.appendChild(paragraph(`“${name}” is a source file. It can hold more than the model, such as other scenes and embedded images.`));
    }
    openDialog({
      heading: `Add to “${assetName}”?`,
      body: "Files in Arbesk are public: anyone with their address can download them, and they can't be removed from IPFS.",
      extra: sources.length ? extra : undefined,
      responses: [
        { id: "cancel", label: "Cancel" },
        { id: "add", label: "Add", style: "suggested" },
      ],
      defaultResponse: "cancel",
      closeResponse: "cancel",
      onResponse: (id) => resolve(id === "add"),
    });
  });
}

/** What was answered for a file whose id the asset already has. */
export type ClashChoice = "replace" | "skip" | "keep_both";

/** Replace “Bracket.stl” in “Box.gltf”? (spec F10): the file the asset
 *  has, with its subtitle, and the new one's name; Skip, Keep Both
 *  (Enter) and Replace, and Apply this action to all when `remaining`
 *  more clash. Null on Escape, which stops the paste. */
export function askAttachClash(assetName: string, existing: RepresentationInfo, newName: string, remaining: number): Promise<{ choice: ClashChoice; all: boolean } | null> {
  return new Promise((resolve) => {
    const extra = document.createElement("div");
    extra.className = "dialog-paragraphs";
    extra.appendChild(paragraph(`The new file is “${newName}”.`));
    let all = false;
    const check = applyToAllRow(remaining, (ticked) => (all = ticked));
    if (check) extra.appendChild(check);
    openDialog({
      heading: `Replace “${existing.name}” in “${assetName}”?`,
      body: fileSubtitle(existing),
      extra,
      responses: [
        { id: "skip", label: "Skip" },
        { id: "keep_both", label: "Keep Both" },
        { id: "replace", label: "Replace", style: "destructive" },
      ],
      // Enter keeps both: nothing is replaced by accident.
      defaultResponse: "keep_both",
      closeResponse: "cancel",
      onResponse: (id) => resolve(id === "cancel" ? null : { choice: id as ClashChoice, all }),
    });
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

/** Which files a copy out takes (spec F9): one asset by its files, several
 *  by their full lists' formats (`chooseFormatsToCopy`). */
export function chooseFilesToCopy(assets: readonly AssetFiles[], iconOf: (asset: { name: string }) => Entry): Promise<CopyChoice | null> {
  return assets.length === 1 ? askWhichFiles({ one: assets[0] }, iconOf) : chooseFormatsToCopy(assets.map(formatsOfFiles), iconOf);
}

/**
 * Which files a copy out of several assets takes, by their formats: the
 * Models row and a row per format (spec F9). The formats come from the
 * full lists or from the listing's summaries (performance plan, phase 4:
 * the same rows). When `withdrawn` settles first, the dialog closes and
 * the answer is null, as for Cancel.
 */
export function chooseFormatsToCopy(
  assets: readonly AssetFormats[], iconOf: (asset: { name: string }) => Entry, withdrawn?: Promise<unknown>,
): Promise<CopyChoice | null> {
  return askWhichFiles({ several: assets }, iconOf, withdrawn);
}

/** The rows of the copy dialog: one asset's model and files, or several
 *  assets' Models and formats; the check boxes after the model's, in order. */
function copyRows(asked: { one: AssetFiles } | { several: readonly AssetFormats[] }, iconOf: (asset: { name: string }) => Entry, list: HTMLElement, model: HTMLInputElement): HTMLInputElement[] {
  const rows: HTMLInputElement[] = [];
  if ("one" in asked) {
    const one = asked.one;
    list.appendChild(fileRow(iconOf(one), one.name, "Model", null, model));
    for (const rep of one.files) {
      const box = checkBox(false);
      list.appendChild(fileRow(iconEntry(rep.name), rep.name, fileSubtitle(rep), rep.stale ? staleReason(rep, one.files) : null, box));
      rows.push(box);
    }
    return rows;
  }
  list.appendChild(fileRow(iconOf(asked.several[0]), "Models", "", null, model));
  for (const { format, assets: n } of formatCounts(asked.several)) {
    const box = checkBox(false);
    box.dataset.format = format;
    list.appendChild(fileRow(iconEntry(`x.${format}`), formatRowLabel(format, n), "", null, box));
    rows.push(box);
  }
  return rows;
}

/** Copy Which Files of “Box.gltf”? (one asset: a row per file) or Copy
 *  Which Files? (several: Models and a row per format, applied to each),
 *  with All Files above, checked when every row is, mixed when some are.
 *  Copy is insensitive while nothing is checked. Resolves with the
 *  choice, or null on Cancel or Escape (or once `withdrawn` settles). */
function askWhichFiles(
  asked: { one: AssetFiles } | { several: readonly AssetFormats[] }, iconOf: (asset: { name: string }) => Entry, withdrawn?: Promise<unknown>,
): Promise<CopyChoice | null> {
  return new Promise((resolve) => {
    const one = "one" in asked ? asked.one : null;
    const extra = document.createElement("div");
    extra.className = "files-page";
    const list = boxedList("Files");
    const all = checkBox(false);
    list.appendChild(fileRow(null, "All Files", "", null, all));
    all.closest("label")!.classList.add("all-files-row");
    const model = checkBox(true);
    const rows: HTMLInputElement[] = [model, ...copyRows(asked, iconOf, list, model)];
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
    let answered = false;
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
        answered = true;
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
    const withdraw = () => {
      if (answered) return;
      answered = true;
      dialog?.close();
      resolve(null);
    };
    withdrawn?.then(withdraw, withdraw);
  });
}
