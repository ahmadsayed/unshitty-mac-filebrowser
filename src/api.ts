import { Channel, invoke } from "@tauri-apps/api/core";
import type { ProgressUpdate } from "./operations";
import { openUrl } from "@tauri-apps/plugin-opener";

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
  /** Shares view: who shared this folder. */
  sharer?: string;
  /** Shares view: whether the sharer is reachable. */
  shareStatus?: "online" | "offline" | "no-access" | "missing";
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

/** Recursively find entries whose name contains `query`. */
/** Why a search ended before looking everywhere: it found `limit` matches,
 *  or it looked at as many items as one search may. */
export type SearchStop = "results" | "scanned";

export interface SearchSummary {
  stopped: SearchStop | null;
  scanned: number;
}

/** Search `path` recursively by name. Matches arrive in batches through
 *  `onFound` while the search runs; a new search in this window stops it. */
export async function searchDir(
  path: string,
  query: string,
  showHidden: boolean,
  limit: number,
  onFound: (batch: Entry[]) => void
): Promise<SearchSummary> {
  const onResults = new Channel<Entry[]>();
  onResults.onmessage = onFound;
  return invoke<SearchSummary>("search_dir", { path, query, showHidden, limit, onResults });
}

/** Stop this window's running search. */
export async function cancelSearch(): Promise<void> {
  return invoke("cancel_search");
}

export interface ExtractResult {
  path: string;
  files: number;
  folders: number;
  skipped: number;
}

/** Extract a supported archive, never overwriting existing files. */
export async function extractArchive(
  path: string,
  dest: string,
  create: boolean
): Promise<ExtractResult> {
  return invoke<ExtractResult>("extract_archive", { path, dest, create });
}

export interface SftpServer {
  id: string;
  label: string;
  url: string;
  host: string;
  port: number;
  user: string;
}

export type SftpConnectOutcome =
  | { kind: "connected"; server: SftpServer }
  | { kind: "auth-needed"; server: SftpServer }
  | { kind: "host-key-unknown"; server: SftpServer; fingerprint: string };

/** Connect to an SSH/SFTP server; key/agent first, password only when needed. */
export async function sftpConnect(
  url: string,
  password: string | undefined,
  trustHost: boolean
): Promise<SftpConnectOutcome> {
  return invoke<SftpConnectOutcome>("sftp_connect", { url, password, trustHost });
}

export async function sftpServers(): Promise<SftpServer[]> {
  return invoke<SftpServer[]>("sftp_servers");
}

export async function sftpDisconnect(id: string): Promise<void> {
  return invoke<void>("sftp_disconnect", { id });
}

export async function sftpOpen(path: string): Promise<string> {
  return invoke<string>("sftp_open", { path });
}

/** Window buttons for each end of the header bars; none on macOS. */
export interface WindowButtons {
  start: string[];
  end: string[];
}

/** The window buttons the user chose in GNOME (`button-layout`). */
export async function windowButtons(): Promise<WindowButtons> {
  return invoke<WindowButtons>("window_buttons");
}

/** Minimize, toggle maximize or close this window (`state` only asks).
 * Answers whether the window is maximized. */
export async function windowControl(action: "minimize" | "maximize" | "close" | "state"): Promise<boolean> {
  return invoke<boolean>("window_control", { action });
}

/** Open another File Browser window at `path` (home when omitted). */
export async function newWindow(path?: string): Promise<void> {
  return invoke<void>("new_window", { path: path ?? null });
}

export interface InstalledAppImage {
  name: string;
  path: string;
  desktop: string;
}

/** Copy an AppImage into user-local storage and register a GNOME launcher. */
export function installAppImage(path: string, name: string): Promise<InstalledAppImage> {
  return invoke("install_appimage", { path, name });
}

/** Request a terminal to run a local script with Bash; not a success verdict. */
export function runShellScript(path: string): Promise<void> {
  return invoke("run_shell_script", { path });
}

/** Create a zip/tar.gz/tar.xz archive of `paths`; never replaces an existing file. */
export async function compressArchive(
  paths: string[],
  dest: string,
  format: "zip" | "tar.gz" | "tar.xz"
): Promise<void> {
  return invoke<void>("compress_archive", { paths, dest, format });
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

// ---------------------------------------------------------------------------
// Folder sharing (iroh)
// ---------------------------------------------------------------------------

export interface ShareIdentity {
  name: string | null;
}

export type ShareStatus = "ok" | "missing";
export type LinkState = "unused" | "online" | "offline";

export interface AccessEntry {
  id: string;
  kind: "anyone" | "person";
  name: string;
  state: LinkState;
  visitors: number;
  protected: boolean;
}

export interface ShareInfo {
  id: string;
  path: string;
  status: ShareStatus;
  access: AccessEntry[];
}

export interface ShareSummary {
  id: string;
  path: string;
  name: string;
  status: ShareStatus;
}

export interface ReceivedShare {
  id: string;
  folder: string;
  sharer: string;
  status: "online" | "offline" | "no-access" | "missing";
}

export interface LinkResult {
  link: string;
  entry: AccessEntry;
}

export async function shareIdentity(): Promise<ShareIdentity> {
  return invoke<ShareIdentity>("share_identity");
}

export async function setShareName(name: string): Promise<void> {
  return invoke<void>("set_share_name", { name });
}

export async function shareFolder(path: string): Promise<ShareInfo> {
  return invoke<ShareInfo>("share_folder", { path });
}

export async function shareInfo(path: string): Promise<ShareInfo | null> {
  return invoke<ShareInfo | null>("share_info", { path });
}

export async function listShares(): Promise<ShareSummary[]> {
  return invoke<ShareSummary[]>("list_shares");
}

export async function createShareLink(
  share: string,
  kind: "anyone" | "person",
  person?: string,
  passphrase?: string
): Promise<LinkResult> {
  return invoke<LinkResult>("create_link", { share, kind, person, passphrase });
}

export async function shareLinkRequiresPassphrase(link: string): Promise<boolean> {
  return invoke<boolean>("share_link_requires_passphrase", { link });
}

export async function shareLinkFor(share: string, entry: string): Promise<string> {
  return invoke<string>("share_link", { share, entry });
}

export async function removeShareAccess(share: string, entry: string): Promise<void> {
  return invoke<void>("remove_access", { share, entry });
}

export async function stopSharing(share: string): Promise<void> {
  return invoke<void>("stop_sharing", { share });
}

export async function addReceived(link: string, passphrase?: string): Promise<ReceivedShare> {
  return invoke<ReceivedShare>("add_received", { link, passphrase });
}

export async function listReceived(): Promise<ReceivedShare[]> {
  return invoke<ReceivedShare[]>("list_received");
}

export async function removeReceived(id: string): Promise<void> {
  return invoke<void>("remove_received", { id });
}

export async function shareOpen(path: string): Promise<string> {
  return invoke<string>("share_open", { path });
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
/** Progress for one long command: an operation id and where updates go. */
export interface Tracked {
  op: string;
  onProgress?: (update: ProgressUpdate) => void;
}

/** The arguments a long command takes for its operation (spec D3). */
function trackedArgs(tracked?: Tracked): { op: string; onProgress: Channel<ProgressUpdate> } {
  const channel = new Channel<ProgressUpdate>();
  if (tracked?.onProgress) channel.onmessage = tracked.onProgress;
  return { op: tracked?.op ?? "", onProgress: channel };
}

export async function copyEntry(from: string, to: string, overwrite: boolean, tracked?: Tracked): Promise<string> {
  return invoke<string>("copy_entry", { from, to, overwrite, ...trackedArgs(tracked) });
}

/** Stop operation `op`; its running command ends with "Cancelled.". */
export async function cancelOperation(op: string): Promise<void> {
  return invoke<void>("cancel_operation", { op });
}

/** Total size and item count of `paths`, for an operation's progress. */
export async function measure(paths: string[]): Promise<{ bytes: number; items: number }> {
  return invoke<{ bytes: number; items: number }>("measure", { paths });
}

/** Move `from` to `to`; same conflict rules as `copyEntry`. */
export async function moveEntry(from: string, to: string, overwrite: boolean, tracked?: Tracked): Promise<string> {
  return invoke<string>("move_entry", { from, to, overwrite, ...trackedArgs(tracked) });
}

/** One path of a Move to Trash: its path in the Trash when known (for Undo), or why it stayed. */
export interface TrashOutcome {
  trashed: string | null;
  error: string | null;
}

/**
 * Move `paths` to the Trash as one batch (one Finder request, so one sound,
 * on macOS). Progress counts items; a cancel stops between the backend's
 * calls and leaves the rest with the "Cancelled." error.
 */
export async function trashEntries(paths: string[], tracked?: Tracked): Promise<TrashOutcome[]> {
  return invoke<TrashOutcome[]>("trash_entries", { paths, ...trackedArgs(tracked) });
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
export async function emptyTrash(tracked?: Tracked): Promise<void> {
  return invoke<void>("empty_trash", trackedArgs(tracked));
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

/** Open a local file or downloaded remote file with its default application. */
export async function openFile(path: string, tracked?: Tracked): Promise<void> {
  if (path.startsWith("gdrive://")) {
    const target = await invoke<DriveOpenTarget>("drive_open", { path, ...trackedArgs(tracked) });
    return target.kind === "url" ? openUrl(target.url) : invoke<void>("open_file", { path: target.path });
  }
  if (path.startsWith("share://")) {
    const localPath = await invoke<string>("share_open", { path, ...trackedArgs(tracked) });
    return invoke<void>("open_file", { path: localPath });
  }
  if (path.startsWith("sftp://")) {
    const localPath = await invoke<string>("sftp_open", { path, ...trackedArgs(tracked) });
    return invoke<void>("open_file", { path: localPath });
  }
  return invoke<void>("open_file", { path });
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

/** Change the read/write/execute bits; `mode` is the low nine bits. */
export async function setPermissions(path: string, mode: number): Promise<void> {
  return invoke<void>("set_permissions", { path, mode });
}

/** Create a symlink beside an item and return its path. */
export async function createSymlink(path: string): Promise<string> {
  return invoke<string>("create_symlink", { path });
}

/** Create an empty file; the backend refuses to replace one. */
export async function createFile(path: string): Promise<void> {
  return invoke<void>("create_file", { path });
}

/** Files in the user's Templates folder, for New Document. */
export async function listTemplates(): Promise<Entry[]> {
  return invoke<Entry[]>("list_templates");
}

/** Rename several items in place; the backend checks them all first. */
export async function renameMany(items: { from: string; to: string }[]): Promise<void> {
  return invoke<void>("rename_many", { items });
}

export interface TextPreview {
  text: string;
  truncated: boolean;
}

/** The beginning of a text file, for the preview panel. */
export async function previewText(path: string, limit: number): Promise<TextPreview> {
  return invoke<TextPreview>("preview_text", { path, limit });
}

export interface AppChoice {
  name: string;
  /** macOS: the `.app` path; Linux: the `.desktop` file path. */
  id: string;
}

/** Applications offered by Open With. */
export async function listApps(): Promise<AppChoice[]> {
  return invoke<AppChoice[]>("list_apps");
}

/** Open a local file with a chosen application. */
export async function openWith(path: string, app: string): Promise<void> {
  return invoke<void>("open_with", { path, app });
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

/** PNG bytes of a thumbnail no larger than `size` px; empty when there is
 *  none or the request was cancelled. `ticket` names it for cancelling. */
export async function thumbnail(path: string, size: number, ticket?: number): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("thumbnail", { path, size, ticket });
}

/** Stop thumbnail requests that are no longer on screen. */
export async function cancelThumbnails(tickets: number[]): Promise<void> {
  return invoke<void>("cancel_thumbnails", { tickets });
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
