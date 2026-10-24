// Keep Versions UI: the Versions… dialog, deleted files, and the Properties
// rows. Plain DOM in the same style as the rest of the app.
import {
  clearHistory,
  deletedFiles,
  historyInfo,
  listVersions,
  openDeleted,
  openFile,
  openVersion,
  replaceWithVersion,
  restoreDeleted,
  restoreVersionCopy,
  stopKeepingVersions,
  versionCount,
  versionDiff,
  versionedRoot,
  type DeletedFile,
  type Entry,
  type HistoryInfo,
  type VersionDiff,
  type VersionInfo,
} from "./api";
import { formatDate, formatDateDetailed, formatSize } from "./format";
import { sym } from "./icons";
import { openDialog, showToast } from "./ui";

function clean(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

/** `2026-10-24 14-05`, local time (no colon, so Finder doesn't turn it into `/`). */
export function restoreStamp(unixSeconds: number, now = new Date()): string {
  const date = unixSeconds ? new Date(unixSeconds * 1000) : now;
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}-${pad(date.getMinutes())}`;
}

/** `<stem> (<stamp>)<ext>` for Restore Copy, matching the spec. */
export function restoreName(path: string, unixSeconds: number): string {
  const name = path.slice(path.lastIndexOf("/") + 1) || path;
  const dot = name.lastIndexOf(".");
  const hasExt = dot > 0;
  const stem = hasExt ? name.slice(0, dot) : name;
  const ext = hasExt ? name.slice(dot) : "";
  return `${stem} (${restoreStamp(unixSeconds)})${ext}`;
}

const OPEN_COPY_TOAST = "Opened a read-only copy. Use Restore Copy to edit.";

/** Open a version in the default app from a read-only cache copy. */
function openVersionCopy(entry: Entry, version: VersionInfo): void {
  const name = restoreName(entry.path, version.mtime);
  showToast("Preparing a copy…", { timeout: 60_000 });
  openVersion(entry.path, version.id, name)
    .then((path) => openFile(path))
    .then(() => showToast(OPEN_COPY_TOAST))
    .catch((err) => showToast(`Could not open the copy: ${clean(err)}`));
}

/** Open a deleted file's last real version without restoring it. */
function openDeletedCopy(file: DeletedFile): void {
  const name = restoreName(file.path, file.deleted_at);
  showToast("Preparing a copy…", { timeout: 60_000 });
  openDeleted(file.path, name)
    .then((path) => openFile(path))
    .then(() => showToast("Opened a read-only copy. Use Restore to keep it."))
    .catch((err) => showToast(`Could not open the copy: ${clean(err)}`));
}

function isInside(path: string, root: string): boolean {
  return path === root || path.startsWith(root.replace(/\/+$/, "") + "/");
}

function versionLabel(row: VersionInfo): string {
  return formatDate(row.deleted ? row.recorded_at : row.mtime);
}

function addTag(row: HTMLElement, text: string, className: string): void {
  const tag = document.createElement("span");
  tag.className = `version-tag ${className}`;
  tag.textContent = text;
  row.appendChild(tag);
}

/** What the diff area says when there is no line diff to show. */
const DIFF_MESSAGES: Record<Exclude<VersionDiff["kind"], "text">, string> = {
  same: "This version is the same as the current file.",
  binary: "No preview for this type of file.",
  too_large: "This file is too large to compare.",
  deleted: "The file was deleted at this point.",
  not_on_disk: "The file is not in the folder any more.",
};

const DIFF_LINE_CLASS: Record<string, string> = { "+": "diff-line added", "-": "diff-line removed" };

function renderTextDiff(diff: HTMLElement, lines: { tag: string; text: string }[], truncated: boolean): void {
  if (!lines.length) {
    diff.textContent = "No differences.";
    return;
  }
  for (const line of lines) {
    const div = document.createElement("div");
    div.className = DIFF_LINE_CLASS[line.tag] ?? "diff-line";
    div.textContent = line.tag + line.text;
    diff.appendChild(div);
  }
  if (truncated) {
    const more = document.createElement("div");
    more.className = "diff-line truncated";
    more.textContent = "… diff truncated";
    diff.appendChild(more);
  }
}

/** The Versions… dialog for one file. */
export async function showVersions(entry: Entry, onChanged: () => void): Promise<void> {
  let rows: VersionInfo[];
  let total: number;
  try {
    [rows, total] = await Promise.all([listVersions(entry.path, 1000, 0), versionCount(entry.path)]);
  } catch (err) {
    showToast(`Could not read the version history: ${clean(err)}`);
    return;
  }

  const extra = document.createElement("div");
  extra.className = "versions-layout";
  const list = document.createElement("div");
  list.className = "versions-list";
  list.setAttribute("role", "listbox");
  const note = document.createElement("div");
  note.className = "versions-note";
  if (total > rows.length) {
    note.textContent = `Showing the newest ${rows.length.toLocaleString()} of ${total.toLocaleString()} versions.`;
  }
  const diff = document.createElement("pre");
  diff.className = "versions-diff";
  extra.append(list, note, diff);

  let selected: VersionInfo | null = null;
  let token = 0;
  let canReplace = false;
  let loading = false;
  let dialog: ReturnType<typeof openDialog>;

  const setButtons = () => {
    const openable = !!selected && !selected.deleted && !loading;
    const usable = !!selected && !selected.deleted && !selected.current && !loading;
    dialog.setResponseEnabled("open", openable);
    dialog.setResponseEnabled("restore", usable);
    dialog.setResponseEnabled("replace", usable && canReplace);
  };

  const setDiff = (value: VersionDiff) => {
    diff.className = "versions-diff";
    diff.replaceChildren();
    if (value.kind === "text") renderTextDiff(diff, value.lines, value.truncated);
    else diff.textContent = DIFF_MESSAGES[value.kind];
  };

  const select = async (row: VersionInfo, button: HTMLElement) => {
    selected = row;
    loading = true;
    for (const other of list.querySelectorAll<HTMLElement>(".version-row")) {
      other.classList.toggle("selected", other === button);
      other.setAttribute("aria-selected", String(other === button));
    }
    setButtons();
    const mine = ++token;
    diff.className = "versions-diff loading";
    diff.textContent = "Loading…";
    try {
      const value = await versionDiff(entry.path, row.id);
      if (mine !== token) return;
      canReplace = value.kind !== "not_on_disk" && value.kind !== "deleted";
      setDiff(value);
    } catch (err) {
      if (mine !== token) return;
      diff.className = "versions-diff";
      diff.textContent = `Could not load the diff: ${clean(err)}`;
    } finally {
      if (mine === token) {
        loading = false;
        setButtons();
      }
    }
  };

  rows.forEach((row) => {
    const button = document.createElement("button");
    button.className = "version-row";
    button.setAttribute("role", "option");
    button.setAttribute("aria-selected", "false");
    const when = document.createElement("span");
    when.className = "version-when";
    when.textContent = versionLabel(row);
    when.title = formatDateDetailed(row.deleted ? row.recorded_at : row.mtime);
    const size = document.createElement("span");
    size.className = "version-size";
    size.textContent = row.deleted ? "—" : formatSize(row.size);
    button.append(when, size);
    if (row.current) addTag(button, "Current", "current");
    if (row.deleted) addTag(button, "Deleted", "deleted");
    button.addEventListener("click", () => select(row, button));
    list.appendChild(button);
  });

  dialog = openDialog({
    heading: `Versions of “${entry.name}”`,
    className: "versions-dialog",
    extra,
    responses: [
      { id: "open", label: "Open Copy" },
      { id: "restore", label: "Restore Copy" },
      { id: "replace", label: "Replace…", style: "destructive" },
      { id: "close", label: "Close" },
    ],
    closeResponse: "close",
    onResponse: (id) => {
      if (id === "open" && selected && !selected.deleted) {
        openVersionCopy(entry, selected);
      } else if (id === "restore" && selected) {
        const name = restoreName(entry.path, selected.deleted ? selected.recorded_at : selected.mtime);
        restoreVersionCopy(entry.path, selected.id, name)
          .then((path) => {
            showToast(`Restored as “${path.slice(path.lastIndexOf("/") + 1)}”`);
            onChanged();
          })
          .catch((err) => showToast(`Could not restore: ${clean(err)}`));
      } else if (id === "replace" && selected) {
        const version = selected;
        setTimeout(() => confirmReplace(entry, version, onChanged), 0);
      }
    },
  });
  setButtons();
  if (rows.length) {
    const first = list.querySelector<HTMLElement>(".version-row");
    if (first) void select(rows[0], first);
  } else {
    diff.textContent = "This file has no versions.";
  }
}

function confirmReplace(entry: Entry, version: VersionInfo, onChanged: () => void): void {
  openDialog({
    heading: `Replace “${entry.name}”?`,
    body: `The current file will be moved to the Trash and replaced with the version from ${versionLabel(version)}. It stays in the history too.`,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "replace", label: "Replace", style: "destructive" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "replace") return;
      replaceWithVersion(entry.path, version.id)
        .then(() => {
          showToast(`Replaced “${entry.name}”`);
          onChanged();
        })
        .catch((err) => showToast(`Could not replace: ${clean(err)}`));
    },
  });
}

/** Deleted Files… for a folder inside a versioned root. */
export async function showDeletedFiles(dir: string, onChanged: () => void): Promise<void> {
  let files: DeletedFile[];
  try {
    files = await deletedFiles(dir);
  } catch (err) {
    showToast(`Could not read deleted files: ${clean(err)}`);
    return;
  }

  const extra = document.createElement("div");
  extra.className = "deleted-list";
  if (!files.length) {
    const empty = document.createElement("div");
    empty.className = "deleted-empty";
    empty.textContent = "No Deleted Files";
    extra.appendChild(empty);
  }

  const restore = (file: DeletedFile, row: HTMLElement) => {
    const name = restoreName(file.path, file.deleted_at);
    restoreDeleted(file.path, name)
      .then((path) => {
        showToast(`Restored “${path.slice(path.lastIndexOf("/") + 1)}”`);
        row.remove();
        onChanged();
      })
      .catch((err) => showToast(`Could not restore: ${clean(err)}`));
  };

  for (const file of files) {
    const row = document.createElement("div");
    row.className = "deleted-row";
    const name = document.createElement("span");
    name.className = "deleted-name";
    name.textContent = file.path.slice(file.path.lastIndexOf("/") + 1);
    name.title = file.path;
    const when = document.createElement("span");
    when.className = "deleted-when";
    when.textContent = formatDate(file.deleted_at);
    when.title = formatDateDetailed(file.deleted_at);
    const size = document.createElement("span");
    size.className = "deleted-size";
    size.textContent = formatSize(file.size);
    const actions = document.createElement("div");
    actions.className = "deleted-actions";
    const open = document.createElement("button");
    open.className = "btn pill";
    open.textContent = "Open";
    open.addEventListener("click", () => openDeletedCopy(file));
    const restoreButton = document.createElement("button");
    restoreButton.className = "btn pill";
    restoreButton.textContent = "Restore";
    restoreButton.addEventListener("click", () => restore(file, row));
    actions.append(open, restoreButton);
    row.append(name, when, size, actions);
    extra.appendChild(row);
  }

  openDialog({
    heading: "Deleted Files",
    className: "deleted-dialog",
    extra,
    responses: [{ id: "close", label: "Close" }],
    closeResponse: "close",
  });
}

/**
 * Add the version-related rows to an open Properties dialog. Files get a
 * count; the versioned root gets history size plus Clear/Stop actions.
 */
export async function addVersionRows(
  list: HTMLElement,
  entry: Entry,
  root: string | null,
  onChanged: () => void,
  onClose?: () => void
): Promise<HistoryInfo | null> {
  if (!root) return null;
  if (entry.path === root && entry.is_dir) {
    let info: HistoryInfo;
    try {
      info = await historyInfo(root);
    } catch {
      return null;
    }
    addRow(
      list,
      "Version History",
      `${formatSize(info.bytes)}, ${info.versions.toLocaleString()} versions of ${info.files.toLocaleString()} files`
    );
    const actions = document.createElement("div");
    actions.className = "props-actions";
    actions.append(
      actionRow("Clear History…", () =>
        confirmClear(root, entry.name, onChanged, onClose)
      ),
      actionRow("Stop Keeping Versions…", () =>
        confirmStop(root, entry.name, onChanged, onClose)
      )
    );
    // A separate group keeps destructive actions out of the facts list.
    if (list.parentElement) list.insertAdjacentElement("afterend", actions);
    else list.appendChild(actions);
    return info;
  }
  if (!entry.is_dir && isInside(entry.path, root)) {
    try {
      const count = await versionCount(entry.path);
      if (count > 0) addRow(list, "Versions", count.toLocaleString());
    } catch {
      /* the file may have vanished; Properties already shows the rest */
    }
  }
  return null;
}

function addRow(list: HTMLElement, label: string, value: string): void {
  const row = document.createElement("div");
  row.className = "prop-row";
  const key = document.createElement("div");
  key.className = "prop-label";
  key.textContent = label;
  const val = document.createElement("div");
  val.className = "prop-value";
  val.textContent = value;
  row.append(key, val);
  list.appendChild(row);
}

/** A full-width destructive row that opens a confirmation dialog. */
function actionRow(label: string, onClick: () => void): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "props-action";
  const text = document.createElement("span");
  text.textContent = label;
  const chevron = document.createElement("span");
  chevron.className = "props-action-chevron";
  chevron.innerHTML = sym("go-next");
  button.append(text, chevron);
  button.addEventListener("click", onClick);
  return button;
}

function closeProperties(onClose?: () => void): void {
  onClose?.();
}

function confirmClear(
  root: string,
  name: string,
  onChanged: () => void,
  onClose?: () => void
): void {
  openDialog({
    heading: "Clear Version History?",
    body: `All earlier versions of the files in “${name}” will be moved to the Trash. The current files are not changed.`,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "clear", label: "Clear History", style: "destructive" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "clear") return;
      clearHistory(root)
        .then(() => {
          showToast("Version history cleared");
          closeProperties(onClose);
          onChanged();
        })
        .catch((err) => showToast(`Could not clear the history: ${clean(err)}`));
    },
  });
}

function confirmStop(
  root: string,
  name: string,
  onChanged: () => void,
  onClose?: () => void
): void {
  openDialog({
    heading: "Stop Keeping Versions?",
    body: `Earlier versions of the files in “${name}” will be moved to the Trash and new changes won't be recorded. The current files are not changed.`,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "stop", label: "Stop Keeping Versions", style: "destructive" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "stop") return;
      stopKeepingVersions(root)
        .then(() => {
          showToast("Stopped keeping versions");
          closeProperties(onClose);
          onChanged();
        })
        .catch((err) => showToast(`Could not stop keeping versions: ${clean(err)}`));
    },
  });
}

/** For tests/preview: whether a path is inside a root, with a boundary. */
export function insideVersionedRoot(path: string, root: string | null): boolean {
  return !!root && isInside(path, root);
}

/** Re-exported so main.ts has one place to ask. */
export { versionedRoot };
