import { invoke } from "@tauri-apps/api/core";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";

export interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number; // unix seconds
  is_hidden: boolean;
  is_symlink: boolean;
}

/**
 * List the contents of a directory.
 * Tauri v2 maps camelCase JS args to snake_case Rust params automatically.
 */
export async function listDir(
  path: string,
  showHidden: boolean
): Promise<Entry[]> {
  return invoke<Entry[]>("list_dir", { path, showHidden });
}

/** Item counts for several folders; `null` where a folder can't be read. */
export async function itemCounts(
  paths: string[],
  showHidden: boolean
): Promise<(number | null)[]> {
  return invoke<(number | null)[]>("item_counts", { paths, showHidden });
}

/** Create a new directory at the given path. Errors if the path already exists. */
export async function makeDir(path: string): Promise<void> {
  return invoke<void>("make_dir", { path });
}

/** Rename/move `from` to `to`. Errors if `to` already exists. */
export async function renameEntry(from: string, to: string): Promise<void> {
  return invoke<void>("rename_entry", { from, to });
}

/** True if anything (even a broken symlink) exists at `path`. */
export async function pathExists(path: string): Promise<boolean> {
  return invoke<boolean>("path_exists", { path });
}

/**
 * Copy `from` to `to`, recursively for folders. With `overwrite`, an existing
 * `to` is moved to the Trash and replaced; otherwise an existing `to` is an error.
 */
export async function copyEntry(from: string, to: string, overwrite: boolean): Promise<void> {
  return invoke<void>("copy_entry", { from, to, overwrite });
}

/** Move `from` to `to`; same conflict rules as `copyEntry`. */
export async function moveEntry(from: string, to: string, overwrite: boolean): Promise<void> {
  return invoke<void>("move_entry", { from, to, overwrite });
}

/** Move `path` to the Trash. */
export async function trashEntry(path: string): Promise<void> {
  return invoke<void>("trash_entry", { path });
}

/** Open a file (or an .app bundle) with its default macOS application. */
export async function openFile(path: string): Promise<void> {
  return openPath(path);
}

/** Open System Settings at Privacy & Security → Full Disk Access. */
export async function openFullDiskAccessSettings(): Promise<void> {
  return openUrl("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles");
}

/** Folders pinned to the sidebar. */
export async function loadPins(): Promise<string[]> {
  return invoke<string[]>("load_pins");
}

export async function savePins(paths: string[]): Promise<void> {
  return invoke<void>("save_pins", { paths });
}

/** A persisted UI flag (false until set). */
export async function getFlag(name: string): Promise<boolean> {
  return invoke<boolean>("get_flag", { name });
}

export async function setFlag(name: string, value: boolean): Promise<void> {
  return invoke<void>("set_flag", { name, value });
}

/** False when macOS hasn't granted the app Full Disk Access. */
export async function hasFullDiskAccess(): Promise<boolean> {
  return invoke<boolean>("has_full_disk_access");
}
