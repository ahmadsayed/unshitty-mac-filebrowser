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
  /** Present only for a Google Drive entry. */
  cloud?: CloudEntry;
}

/** Drive-specific fields on an `Entry`; local entries omit them. */
export interface CloudEntry {
  account: string;
  id: string;
  mime_type: string;
  is_google_doc: boolean;
  is_shortcut: boolean;
  web_url?: string;
  can_download: boolean;
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

/** A mounted volume, as the sidebar lists it. */
export interface Volume {
  name: string;
  path: string;
}

/** A connected Google account. Tokens are deliberately not part of this. */
export interface DriveAccount {
  id: string;
  email: string;
  name: string;
  root_id: string;
  root_name: string;
  root_path: string;
  scopes: string[];
  connected_at: number;
}

export interface DriveConnectStart {
  flow_id: string;
  auth_url: string;
}

/** The user's own OAuth client. The secret never reaches the frontend. */
export interface DriveClientStatus {
  configured: boolean;
  client_id: string | null;
}

export async function driveClientStatus(): Promise<DriveClientStatus> {
  return invoke<DriveClientStatus>("drive_client_status");
}

/** Store the client from the JSON file Google lets users download. */
export async function driveClientSet(json: string): Promise<DriveClientStatus> {
  return invoke<DriveClientStatus>("drive_client_set", { json });
}

export async function driveClientClear(): Promise<void> {
  return invoke<void>("drive_client_clear");
}

export async function driveAccounts(): Promise<DriveAccount[]> {
  return invoke<DriveAccount[]>("drive_accounts");
}

export async function driveConnectStart(): Promise<DriveConnectStart> {
  return invoke<DriveConnectStart>("drive_connect_start");
}

export async function driveConnectFinish(flowId: string): Promise<DriveAccount> {
  return invoke<DriveAccount>("drive_connect_finish", { flowId });
}

export async function driveConnectCancel(flowId: string): Promise<void> {
  return invoke<void>("drive_connect_cancel", { flowId });
}

export async function driveDisconnect(account: string): Promise<void> {
  return invoke<void>("drive_disconnect", { account });
}

/**
 * Mounted volumes. `/Volumes` on macOS, the udisks2 mount folders and `/mnt`
 * on Linux — the backend knows where to look.
 */
export async function listVolumes(): Promise<Volume[]> {
  return invoke<Volume[]>("volumes");
}

/** A folder's item count, and whether it keeps versions. */
export interface FolderSummary {
  count: number;
  versioned: boolean;
}

/** Summaries for several folders; `null` where a folder can't be read. */
export async function itemCounts(
  paths: string[],
  showHidden: boolean
): Promise<(FolderSummary | null)[]> {
  return invoke<(FolderSummary | null)[]>("item_counts", { paths, showHidden });
}

/** Create a new directory at the given path. Errors if the path already exists. */
export async function makeDir(path: string): Promise<string> {
  return invoke<string>("make_dir", { path });
}

/** Rename/move `from` to `to`. Errors if `to` already exists. */
export async function renameEntry(from: string, to: string): Promise<string> {
  return invoke<string>("rename_entry", { from, to });
}

/** True if anything (even a broken symlink) exists at `path`. */
export async function pathExists(path: string): Promise<boolean> {
  return invoke<boolean>("path_exists", { path });
}

/**
 * Copy `from` to `to`, recursively for folders. With `overwrite`, an existing
 * `to` is moved to the Trash and replaced; otherwise an existing `to` is an error.
 */
export async function copyEntry(from: string, to: string, overwrite: boolean): Promise<string> {
  return invoke<string>("copy_entry", { from, to, overwrite });
}

/** Move `from` to `to`; same conflict rules as `copyEntry`. */
export async function moveEntry(from: string, to: string, overwrite: boolean): Promise<string> {
  return invoke<string>("move_entry", { from, to, overwrite });
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

/**
 * The folder the Trash view lists. macOS uses `~/.Trash`; Linux uses
 * `~/.local/share/Trash/files`.
 */
export async function trashDir(): Promise<string> {
  return invoke<string>("trash_dir");
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
  if (path.startsWith("gdrive://")) {
    const target = await invoke<DriveOpenTarget>("drive_open", { path });
    return target.kind === "url" ? openUrl(target.url) : openPath(target.path);
  }
  return openPath(path);
}

export type DriveOpenTarget =
  | { kind: "file"; path: string }
  | { kind: "url"; url: string };

/** Open a URL in the default browser (used by the sign-in flow). */
export async function openExternal(url: string): Promise<void> {
  return openUrl(url);
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

/** Everything the Properties dialog shows for one path. */
export interface FileProperties {
  name: string;
  path: string;
  /** The containing folder, or empty for a filesystem root. */
  parent: string;
  is_dir: boolean;
  is_symlink: boolean;
  symlink_target: string | null;
  size: number;
  /** Direct children for folders; `null` for files. */
  item_count: number | null;
  modified: number;
  accessed: number;
  /** Birth time; 0 when the filesystem doesn't report one. */
  created: number;
  /** `ls -l`-style permissions, e.g. "drwxr-xr-x". */
  mode: string;
  /** The same bits in octal, e.g. "755". */
  octal: string;
  owner: string;
  group: string;
  others: string;
}

/** Metadata for the Properties dialog. */
export async function fileProperties(path: string, showHidden: boolean): Promise<FileProperties> {
  return invoke<FileProperties>("file_properties", { path, showHidden });
}

/** Total size of every file inside a folder, recursively; `walk` names it for `cancelFolderSize`. */
export async function folderSize(path: string, walk: number): Promise<number> {
  return invoke<number>("folder_size", { path, walk });
}

/**
 * A fresh id for `folderSize`. The backend hands them out so they only grow
 * for the life of the process, whatever the clock or a webview reload does.
 */
export async function newFolderWalk(): Promise<number> {
  return invoke<number>("new_folder_walk");
}

/** Stop folder-size walk `walk` and every earlier one. */
export async function cancelFolderSize(walk: number): Promise<void> {
  return invoke<void>("cancel_folder_size", { walk });
}

/** PNG bytes of a thumbnail no larger than `size` px; empty when there is none. */
export async function thumbnail(path: string, size: number): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("thumbnail", { path, size });
}

// ---------------------------------------------------------------------------
// Keep Versions
// ---------------------------------------------------------------------------

export interface VersionInfo {
  id: number;
  size: number;
  /** Unix seconds, for display. */
  mtime: number;
  recorded_at: number;
  deleted: boolean;
  current: boolean;
}

export interface DiffLine {
  /** ' ', '-' or '+'. */
  tag: string;
  text: string;
}

export type VersionDiff =
  | { kind: "text"; lines: DiffLine[]; truncated: boolean }
  | { kind: "same" }
  | { kind: "binary" }
  | { kind: "too_large" }
  | { kind: "deleted" }
  | { kind: "not_on_disk" };

export interface DeletedFile {
  /** Absolute path of the deleted file. */
  path: string;
  deleted_at: number;
  size: number;
}

export interface HistoryInfo {
  bytes: number;
  versions: number;
  files: number;
}

/** The nearest folder that keeps versions, or null. */
export async function versionedRoot(path: string): Promise<string | null> {
  return invoke<string | null>("versioned_root", { path });
}

/** Create a folder plus its `.versions/` history. Errors leave the folder in place. */
export async function makeVersionedDir(path: string): Promise<void> {
  return invoke<void>("make_versioned_dir", { path });
}

export async function listVersions(
  path: string,
  limit?: number,
  offset?: number
): Promise<VersionInfo[]> {
  return invoke<VersionInfo[]>("list_versions", { path, limit, offset });
}

export async function versionCount(path: string): Promise<number> {
  return invoke<number>("version_count", { path });
}

export async function versionDiff(path: string, version: number): Promise<VersionDiff> {
  return invoke<VersionDiff>("version_diff", { path, version });
}

export async function restoreVersionCopy(
  path: string,
  version: number,
  name: string
): Promise<string> {
  return invoke<string>("restore_version_copy", { path, version, name });
}

/** Materialize a version as a read-only cache copy; returns its path. */
export async function openVersion(path: string, version: number, name: string): Promise<string> {
  return invoke<string>("open_version", { path, version, name });
}

export async function replaceWithVersion(path: string, version: number): Promise<void> {
  return invoke<void>("replace_with_version", { path, version });
}

export async function deletedFiles(dir: string): Promise<DeletedFile[]> {
  return invoke<DeletedFile[]>("deleted_files", { dir });
}

export async function restoreDeleted(path: string, fallbackName: string): Promise<string> {
  return invoke<string>("restore_deleted", { path, fallbackName });
}

/** Materialize a deleted file's last version as a read-only cache copy. */
export async function openDeleted(path: string, name: string): Promise<string> {
  return invoke<string>("open_deleted", { path, name });
}

export async function historyInfo(root: string): Promise<HistoryInfo> {
  return invoke<HistoryInfo>("history_info", { root });
}

export async function clearHistory(root: string): Promise<void> {
  return invoke<void>("clear_history", { root });
}

export async function stopKeepingVersions(root: string): Promise<void> {
  return invoke<void>("stop_keeping_versions", { root });
}
