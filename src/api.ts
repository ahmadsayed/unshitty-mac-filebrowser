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
  /** Only in Recent: when the file was last used (unix seconds). */
  used?: number;
  /** Only in the Trash: where the item came from, if known. */
  origin?: string | null;
  /** Only in the Trash: when it was trashed (unix seconds). */
  trashed?: number;
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

/** Move `path` to the Trash. Resolves with its path in the Trash when known (for Undo). */
export async function trashEntry(path: string): Promise<string | null> {
  return invoke<string | null>("trash_entry", { path });
}

export interface TrashInfo {
  origin: string | null;
  trashed: number;
}

/** Original location and trash date of items in the Trash. */
export async function trashInfo(paths: string[]): Promise<TrashInfo[]> {
  return invoke<TrashInfo[]>("trash_info", { paths });
}

/** Move an item in the Trash back to where it came from; resolves with that path. */
export async function restoreEntry(path: string): Promise<string> {
  return invoke<string>("restore_entry", { path });
}

/** Delete an item in the Trash for good. */
export async function deletePermanently(path: string): Promise<void> {
  return invoke<void>("delete_permanently", { path });
}

/** Delete everything in the Trash. */
export async function emptyTrash(): Promise<void> {
  return invoke<void>("empty_trash");
}

/** Files used lately, here or in other apps, newest first. */
export async function recentFiles(showHidden: boolean): Promise<Entry[]> {
  return invoke<Entry[]>("recent_files", { showHidden });
}

/** Note that a file was opened from this app, so Recent lists it. */
export async function addRecent(path: string): Promise<void> {
  return invoke<void>("add_recent", { path });
}

/** "Remove from Recent": hide files until they're next opened. */
export async function forgetRecent(paths: string[]): Promise<void> {
  return invoke<void>("forget_recent", { paths });
}

/** Paths of starred files and folders. */
export async function loadStarred(): Promise<string[]> {
  return invoke<string[]>("load_starred");
}

export async function saveStarred(paths: string[]): Promise<void> {
  return invoke<void>("save_starred", { paths });
}

/** Starred items that still exist. */
export async function starredFiles(showHidden: boolean): Promise<Entry[]> {
  return invoke<Entry[]>("starred_files", { showHidden });
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

export interface PathInfo {
  is_dir: boolean;
  /** Filesystem id; equal ids mean a move is a cheap rename. */
  device: number;
}

/** Kind and filesystem of each path; `null` where a path no longer exists. */
export async function pathInfo(paths: string[]): Promise<(PathInfo | null)[]> {
  return invoke<(PathInfo | null)[]>("path_info", { paths });
}

/** PNG bytes of a thumbnail no larger than `size` px; empty when there is none. */
export async function thumbnail(path: string, size: number): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("thumbnail", { path, size });
}
