import { CANCELLED, endOperation, finishItem, initOperationsButton, opTitle, setOperationState, startOperation, updateOperation, type Operation, type ProgressUpdate } from "./operations";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { homeDir, documentDir, downloadDir, audioDir, pictureDir, videoDir, desktopDir, publicDir } from "@tauri-apps/api/path";
import {
  listDir, searchDir, cancelSearch, type SearchSummary, makeDir, renameEntry, renameMany, itemCounts, type FolderSummary, copyEntry, moveEntry, trashEntries, type TrashOutcome, pathExists,
  extractArchive,
  newWindow,
  windowButtons,
  windowControl,
  createSymlink,
  createFile,
  listTemplates,
  compressArchive,
  sftpConnect, sftpServers, sftpDisconnect,
  type SftpServer,
  openFile, openFullDiskAccessSettings, loadPins, savePins, getFlag, setFlag, hasFullDiskAccess, pathInfo,
  recentFiles, addRecent, forgetRecent, loadStarred, saveStarred, starredFiles,
  trashInfo, restoreEntry, deletePermanently, emptyTrash, cancelOperation, measure, listVolumes, trashDir, fileProperties, folderSize,
  cancelFolderSize, newFolderWalk, makeVersionedDir, versionedRoot,
  previewText, thumbnail,
  listApps, openWith,
  driveAccounts, driveConnectStart, driveConnectFinish, driveConnectCancel, driveDisconnect,
  driveClientStatus, driveClientSet, driveClientClear,
  openExternal,
  setPermissions,
  shareIdentity, setShareName, shareFolder, shareInfo, listShares, createShareLink,
  shareLinkFor, shareLinkRequiresPassphrase, removeShareAccess, stopSharing, addReceived, listReceived, removeReceived,
  type Entry, type DriveAccount, type ReceivedShare, type ShareSummary, type ShareInfo,
  type AccessEntry,
} from "./api";
import { entryIcon, versionsBadge, fileKind, fileTypeDescription, fileTypeLabel, installIconSprite, isAppBundle, sym, type FileKind, type FolderKind } from "./icons";
import { cachedThumb, keepOnly, requestThumb, thumbKey } from "./thumbnails";
import { appIconKey, loadNativeIcons, nativeIcon, nativeIconSize, nativeIconUrl } from "./native-icons";
import { formatDate, formatDateDetailed, formatItemCount, formatSize } from "./format";
import { LAYOUT_SIZES, platformTheme, type Theme } from "./theme";
import { closePopover, dialogLinks, hasOpenOverlay, openDialog, openMenu, openPopover, showToast, type Anchor, type MenuItem } from "./ui";
import { checkForUpdates } from "./updates";
import { addVersionRows, insideVersionedRoot, showDeletedFiles, showVersions } from "./versions";
import { macDmgItems } from "./mac-dmg";
import { linuxProgramItems } from "./linux-programs";
import { qrShareItems } from "./qr-share";
import { createShareProtection, promptSharePassphrase } from "./share-passphrase";
import { CLOUD_PREFIX, cloudAccountKey, isCloudPath as isCloud, isRemotePath, isShareLink, shareTicket, joinCloudPath, resolveTypedPath, safeLocalName, SHARE_PREFIX, isSharePath, SFTP_PREFIX, isSftpPath, joinSftpPath } from "./paths";

// ---------------------------------------------------------------------------
// Preferences (persisted per machine; storage may be unavailable)
// ---------------------------------------------------------------------------

type ViewMode = "grid" | "list";
type SortKey = "name" | "size" | "modified" | "type";

const GRID_SIZES = [48, 64, 96, 128, 192, 256];
const LIST_SIZES = [16, 32, 48];

const prefs = {
  view: "grid" as ViewMode,
  sortKey: "name" as SortKey,
  sortReversed: false,
  showHidden: false,
  gridZoom: 2,
  listZoom: 1,
  showThumbnails: true,
};

function loadPrefs(): void {
  let saved: Partial<typeof prefs> | null = null;
  try {
    saved = JSON.parse(localStorage.getItem("prefs") ?? "{}");
  } catch {
    /* defaults */
  }
  Object.assign(prefs, saved);
  // Older versions saved a theme choice; the theme now follows the platform.
  delete (prefs as Record<string, unknown>).theme;
  // A fresh start uses the theme's icon size; a zoom the user chose is kept.
  if (typeof saved?.gridZoom !== "number") prefs.gridZoom = LAYOUT_SIZES[currentTheme].gridZoom;
}

function savePrefs(): void {
  try {
    localStorage.setItem("prefs", JSON.stringify(prefs));
  } catch {
    /* not persisted */
  }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/** Sidebar places that aren't folders. Recent and Starred list items from anywhere; Network lists received shares and SSH servers. */
type Pseudo = "recent" | "starred" | "network";
const PSEUDO: Record<Pseudo, { label: string; icon: string; title: string; text: string }> = {
  recent: { label: "Recent", icon: "document-open-recent", title: "No Recent Files", text: "" },
  starred: { label: "Starred", icon: "starred", title: "Starred files will appear here", text: "" },
  network: { label: "Network", icon: "network-workgroup", title: "No Network Locations", text: "Enter a server address or paste a share link below." },
};
const PSEUDO_PREFIX = "pseudo:";
const RECENT = `${PSEUDO_PREFIX}recent`;
const STARRED = `${PSEUDO_PREFIX}starred`;
const NETWORK = `${PSEUDO_PREFIX}network`;

/** Current location: an absolute path, or "pseudo:<name>". */
let location = "";
/** Nearest folder that keeps versions, if the current location is in one. */
let versionedRootPath: string | null = null;
let entries: Entry[] = [];
/** Entries in display order (sorted + search-filtered). */
let shown: Entry[] = [];
let listingError: string | null = null;

const selected = new Set<string>();
/** Bumped by every selection or view change the user makes, not by the
 *  app's own reloads. A copy or move selects what it made only when this
 *  hasn't moved since it was pasted: a newer choice by the user wins (#50). */
let selectionIntent = 0;
function noteSelectionIntent(): void {
  selectionIntent++;
}
let anchorIndex = -1;
let cursorIndex = -1;
let searchQuery: string | null = null;
/** Recursive search results; null means filter the current folder in memory. */
let searchResults: Entry[] | null = null;
let searchTimer = 0;
let searchSeq = 0;
/** Whether a recursive search is still sending results. */
let searching = false;
/** Cap on recursive results, so a broad query can't build an endless view. */
const SEARCH_LIMIT = 1000;

const counts = new Map<string, number | null>();
/** Bumped whenever `counts` changes; sorting by size depends on it. */
let countsVersion = 0;
/** Folders whose summary said they keep versions; drives the version badge. */
const versionedFolders = new Set<string>();
let backStack: string[] = [];
let forwardStack: string[] = [];

/** A tab's navigation state. The active tab's live state lives in the globals above. */
interface TabState {
  location: string;
  back: string[];
  forward: string[];
  selected: string[];
  anchorIndex: number;
  cursorIndex: number;
  searchQuery: string | null;
  searchText: string;
  scrollTop: number;
}

let tabs: TabState[] = [];
let activeTab = -1;
/** Serialises tab switches so two async restores can't race. */
let tabBusy = false;

/** Files copied or cut, waiting to be pasted. */
let clipboard: { paths: string[]; dirs: Set<string>; mode: "copy" | "cut" } | null = null;

/** Starred files and folders. */
let starred = new Set<string>();

/** Folders the user added to the sidebar, after GNOME's default bookmarks. */
let pins: string[] = [];
/** Paths the sidebar always shows (places, default bookmarks, volumes). */
const fixedSidebarPaths = new Set<string>();

let home = "";
let trashPath = "";
/** Received shares, as Network lists them. */
let networkShares: ReceivedShare[] = [];
/** Active SSH/SFTP connections, as the Network view lists them. */
let sftpServerList: SftpServer[] = [];
/** Local folders shared by this machine, for menus/emblems. */
let sharedFolders: ShareSummary[] = [];
/** Why a share path cannot be listed (offline, no access, moved). */
let sharePage: "offline" | "no-access" | "missing" | null = null;
/** The connected Google account, if any. */
let driveAccount: DriveAccount | null = null;
let cloudOpenNoticeShown = false;
/** Mounted volumes, as the backend reported them; used for breadcrumbs and
 * for guessing whether a drag crosses filesystems. */
let mountedVolumes: { name: string; path: string }[] = [];
const folderKinds = new Map<string, FolderKind>();

const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);
const currentTheme: Theme = platformTheme(isMac);
const MOD = isMac ? "⌘" : "Ctrl+";

// ---------------------------------------------------------------------------
// DOM
// ---------------------------------------------------------------------------

const $ = <T extends HTMLElement>(sel: string) => document.querySelector<T>(sel)!;

let viewEl: HTMLElement;
let scrollEl: HTMLElement;
let crumbsEl: HTMLElement;
let pathbarEl: HTMLElement;
let pathEntry: HTMLInputElement;
let searchbarEl: HTMLElement;
let searchEntry: HTMLInputElement;
let floatingBar: HTMLElement;
let shareSubtitle: HTMLElement;
let tabbar: HTMLElement;
let previewLayer: HTMLElement;
let previewName: HTMLElement;
let previewBody: HTMLElement;

// SECURITY: escape HTML entities in user-derived strings before innerHTML insertion
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function basename(path: string): string {
  if (isCloud(path)) {
    const segment = path.replace(/\/+$/, "").split("/").pop() ?? "";
    return cloudSegmentName(segment) || "My Drive";
  }
  if (isSharePath(path)) {
    const segment = path.slice(SHARE_PREFIX.length).split("/").filter(Boolean).pop() ?? "";
    try {
      return decodeURIComponent(segment) || "Shared Folder";
    } catch {
      return segment || "Shared Folder";
    }
  }
  if (isSftpPath(path)) {
    const rest = path.slice(SFTP_PREFIX.length).split("/").filter(Boolean);
    if (rest.length <= 1) return rest[0] ? decodeURIComponent(rest[0]) : "SSH Server";
    const segment = rest.pop()!;
    try {
      return decodeURIComponent(segment) || "SSH Server";
    } catch {
      return segment || "SSH Server";
    }
  }
  return path.replace(/\/+$/, "").split("/").pop() || "/";
}

function parentOf(path: string): string {
  if (isCloud(path)) {
    const trimmed = path.replace(/\/+$/, "");
    const slash = trimmed.lastIndexOf("/");
    return slash > "gdrive://".length ? trimmed.slice(0, slash) : trimmed;
  }
  if (isSharePath(path)) {
    const rest = path.slice(SHARE_PREFIX.length).split("/").filter(Boolean);
    rest.pop();
    return rest.length ? `${SHARE_PREFIX}${rest.join("/")}` : NETWORK;
  }
  if (isSftpPath(path)) {
    const rest = path.slice(SFTP_PREFIX.length).split("/").filter(Boolean);
    rest.pop();
    return rest.length ? `${SFTP_PREFIX}${rest.join("/")}` : NETWORK;
  }
  const p = path.replace(/\/+$/, "").replace(/\/[^/]*$/, "");
  return p || "/";
}

function shareIdOf(path: string): string {
  return path.slice(SHARE_PREFIX.length).split("/")[0] ?? "";
}

function receivedFor(path: string): ReceivedShare | undefined {
  return networkShares.find((share) => share.id === shareIdOf(path));
}

/** What Network lists: one folder entry per received share, then one per connected server. */
function networkEntries(): Entry[] {
  const place = { is_dir: true, size: 0, modified: 0, is_hidden: false, is_symlink: false };
  const shares: Entry[] = networkShares.map((share) => ({
    ...place,
    name: share.folder,
    path: `${SHARE_PREFIX}${share.id}`,
    sharer: share.sharer,
    shareStatus: share.status,
  }));
  const servers: Entry[] = sftpServerList.map((server) => ({ ...place, name: server.label, path: server.url }));
  return [...shares, ...servers];
}

/** The SSH server a Network entry stands for. */
function serverFor(entry: Entry): SftpServer | undefined {
  return isSftpPath(entry.path) ? sftpServerList.find((server) => server.url === entry.path) : undefined;
}

/** Network's second line: who shared a folder, or what kind of server it is. */
function networkDetail(entry: Entry): string {
  return isSftpPath(entry.path) ? "SSH server" : entry.sharer ? `Shared by ${entry.sharer}` : "";
}

function networkStatus(entry: Entry): { cls: string; label: string } {
  if (isSftpPath(entry.path)) return { cls: "online", label: "Connected" };
  return { cls: entry.shareStatus ?? "", label: shareStatusLabel(entry.shareStatus) };
}

/** Shared folders are read only: no New Folder, Paste, Rename, Trash or drops. */
function isReadOnlyLocation(path: string): boolean {
  return isSharePath(path) || isSftpPath(path);
}

/** A path on this machine or a mounted volume, not a place or a remote. */
function isLocalLocation(path: string): boolean {
  return !isPseudo(path) && !isCloud(path) && !isReadOnlyLocation(path);
}

function shareStatusLabel(status: Entry["shareStatus"]): string {
  switch (status) {
    case "online":
      return "Online";
    case "offline":
      return "Offline";
    case "no-access":
      return "No access";
    case "missing":
      return "Missing";
    default:
      return "";
  }
}

/** The display name inside a `<file-id>~<encoded name>` segment. */
function cloudSegmentName(segment: string): string {
  const tilde = segment.indexOf("~");
  const encoded = tilde >= 0 ? segment.slice(tilde + 1) : segment;
  try {
    return decodeURIComponent(encoded);
  } catch {
    return encoded;
  }
}

/** The first folder of an account, whose parent is the account itself. */
function isCloudRoot(path: string): boolean {
  return isCloud(path) && path.slice(CLOUD_PREFIX.length).split("/").filter(Boolean).length <= 1;
}

/** The path of `name` inside `dir`. In Drive it is a pending `~name` path. */
function joinPath(dir: string, name: string): string {
  if (isCloud(dir)) return joinCloudPath(dir, name);
  if (isSftpPath(dir)) return joinSftpPath(dir, name);
  return dir.replace(/\/+$/, "") + "/" + name;
}

function isPseudo(loc: string): loc is `pseudo:${Pseudo}` {
  return loc.startsWith(PSEUDO_PREFIX);
}

/** Pseudo locations that list entries (Recent, Starred, Network). */
function hasEntries(loc: string): boolean {
  return !isPseudo(loc) || loc === RECENT || loc === STARRED || loc === NETWORK;
}

/** Views of items from many folders, which get a Location column. */
function showsWhere(): boolean {
  return location === RECENT || location === STARRED || location === trashPath || searchQuery !== null;
}

/** The entries shown at `loc`. */
async function listLocation(loc: string): Promise<Entry[]> {
  if (loc === RECENT) return recentFiles(prefs.showHidden);
  if (loc === STARRED) return starredFiles(prefs.showHidden);
  if (loc === NETWORK) return networkEntries();
  if (isPseudo(loc)) return [];
  const list = await listDir(loc, prefs.showHidden);
  if (loc === trashPath && list.length) {
    // Where each item came from and when it was trashed, for the columns and Restore.
    const info = await trashInfo(list.map((e) => e.path)).catch(() => []);
    info.forEach((t, i) => Object.assign(list[i], t));
  }
  return list;
}

function locationName(loc: string): string {
  if (isPseudo(loc)) return PSEUDO[loc.slice(PSEUDO_PREFIX.length) as Pseudo].label;
  if (loc === home) return "Home";
  if (loc === trashPath) return "Trash";
  if (isSharePath(loc)) {
    // The root's label is the sharer's folder name (the id is opaque).
    const parts = loc.slice(SHARE_PREFIX.length).split("/").filter(Boolean);
    return parts.length <= 1 ? receivedFor(loc)?.folder || basename(loc) : basename(loc);
  }
  if (isSftpPath(loc)) return basename(loc) || "SSH Server";
  return basename(loc);
}

// ---------------------------------------------------------------------------
// Sorting + filtering
// ---------------------------------------------------------------------------

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function sortValue(e: Entry): number {
  if (prefs.sortKey === "modified") return e.trashed ?? e.modified;
  if (e.is_dir) return counts.get(e.path) ?? -1;
  return e.size;
}

function compareEntries(a: Entry, b: Entry): number {
  // Recent is always newest first, files and folders mixed, as in GNOME Files.
  if (location === RECENT) return (b.used ?? 0) - (a.used ?? 0) || collator.compare(a.name, b.name);
  // Folders always come first, as in GNOME Files' default.
  if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
  let r = 0;
  switch (prefs.sortKey) {
    case "name":
      r = collator.compare(a.name, b.name);
      break;
    case "type":
      r = collator.compare(fileTypeLabel(a), fileTypeLabel(b)) || collator.compare(a.name, b.name);
      break;
    // Size and date sort largest/newest first by default.
    case "size":
    case "modified":
      r = sortValue(b) - sortValue(a) || collator.compare(a.name, b.name);
      break;
  }
  return prefs.sortReversed ? -r : r;
}

function fold(s: string): string {
  // Strips combining marks exactly as the backend's search does.
  return s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

/** What `shown` was last built from: a presentation-only render (grid ↔ list,
 * thumbnails) reuses it instead of sorting a huge folder again. */
let shownFrom: unknown[] = [];

function computeShown(): void {
  const q = searchQuery ? fold(searchQuery.trim()) : "";
  const from = [entries, searchResults, q, location === RECENT, prefs.sortKey, prefs.sortReversed, prefs.sortKey === "size" && countsVersion];
  if (from.every((value, i) => value === shownFrom[i])) return;
  shownFrom = from;
  const fromResults = q && searchResults !== null;
  const list = !q ? entries : fromResults ? searchResults! : entries.filter((e) => fold(e.name).includes(q));
  shown = [...list].sort(compareEntries);
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

function tabSnapshot(): TabState {
  return {
    location,
    back: [...backStack],
    forward: [...forwardStack],
    selected: [...selected],
    anchorIndex,
    cursorIndex,
    searchQuery,
    searchText: searchEntry?.value ?? "",
    scrollTop: scrollEl?.scrollTop ?? 0,
  };
}

function saveActiveTab(): void {
  if (activeTab < 0 || !tabs[activeTab]) return;
  tabs[activeTab] = tabSnapshot();
}

function tabTitle(tab: TabState): string {
  if (isPseudo(tab.location)) {
    return PSEUDO[tab.location.slice(PSEUDO_PREFIX.length) as Pseudo].label;
  }
  return locationName(tab.location) || "Files";
}

/** Draw the tab strip, showing it only when a second tab exists. */
function renderTabbar(): void {
  if (!tabbar) return;
  tabbar.hidden = tabs.length <= 1;
  if (tabs.length <= 1) {
    tabbar.replaceChildren();
    return;
  }
  const next = document.createDocumentFragment();
  tabs.forEach((tab, i) => {
    const el = document.createElement("div");
    el.className = `tab${i === activeTab ? " active" : ""}`;
    el.dataset.tab = String(i);
    el.setAttribute("role", "tab");
    el.setAttribute("aria-selected", String(i === activeTab));
    el.tabIndex = i === activeTab ? 0 : -1;
    el.title = tabTitle(tab);
    const title = document.createElement("span");
    title.className = "tab-title";
    title.textContent = tabTitle(tab);
    const close = document.createElement("button");
    close.className = "flat-btn tab-close";
    close.innerHTML = sym("window-close");
    close.title = "Close Tab";
    close.setAttribute("aria-label", "Close Tab");
    close.addEventListener("click", (event) => {
      event.stopPropagation();
      void closeTab(i);
    });
    el.append(title, close);
    el.addEventListener("click", () => void switchTab(i));
    el.addEventListener("auxclick", (event) => {
      if (event.button !== 1) return;
      event.preventDefault();
      void closeTab(i);
    });
    next.appendChild(el);
  });
  const add = document.createElement("button");
  add.className = "flat-btn tab-add";
  add.textContent = "+";
  add.title = "New Tab";
  add.setAttribute("aria-label", "New Tab");
  add.addEventListener("click", () => void newTab());
  next.appendChild(add);
  tabbar.replaceChildren(next);
}

/** Restore a saved tab into the live view. */
async function restoreTab(tab: TabState): Promise<void> {
  const ok = await navigate(tab.location, { history: "none" });
  if (!ok) return;
  backStack = [...tab.back];
  forwardStack = [...tab.forward];
  selected.clear();
  const paths = new Set(entries.map((entry) => entry.path));
  for (const path of tab.selected) if (paths.has(path)) selected.add(path);
  const last = shown.length - 1;
  anchorIndex = last < 0 ? -1 : Math.min(tab.anchorIndex, last);
  cursorIndex = last < 0 ? -1 : Math.min(tab.cursorIndex, last);
  if (tab.searchQuery !== null) {
    searchQuery = tab.searchQuery;
    searchEntry.value = tab.searchText;
    pathbarEl.hidden = true;
    searchbarEl.hidden = false;
    $("#btn-search").classList.add("open");
    renderView();
    const filteredLast = shown.length - 1;
    anchorIndex = filteredLast < 0 ? -1 : Math.min(tab.anchorIndex, filteredLast);
    cursorIndex = filteredLast < 0 ? -1 : Math.min(tab.cursorIndex, filteredLast);
  }
  updateSelection();
  updateNavButtons();
  renderTabbar();
  scrollEl.scrollTop = tab.scrollTop;
}

async function switchTab(index: number): Promise<void> {
  if (tabBusy || index === activeTab || index < 0 || index >= tabs.length) return;
  tabBusy = true;
  try {
    saveActiveTab();
    activeTab = index;
    await restoreTab(tabs[index]);
  } finally {
    tabBusy = false;
  }
}

/** Open a new tab at the current folder, with a clean history and selection. */
async function newTab(): Promise<void> {
  if (tabBusy) return;
  saveActiveTab();
  tabs.push({
    location,
    back: [],
    forward: [],
    selected: [],
    anchorIndex: -1,
    cursorIndex: -1,
    searchQuery: null,
    searchText: "",
    scrollTop: 0,
  });
  activeTab = tabs.length - 1;
  if (searchQuery !== null) closeSearch(false);
  backStack = [];
  forwardStack = [];
  selected.clear();
  noteSelectionIntent();
  anchorIndex = cursorIndex = -1;
  renderView();
  updateNavButtons();
  updateSelection();
  renderTabbar();
  scrollEl.scrollTop = 0;
  viewEl.focus({ preventScroll: true });
}

async function closeTab(index = activeTab): Promise<void> {
  if (tabBusy || tabs.length <= 1 || index < 0 || index >= tabs.length) return;
  const wasActive = index === activeTab;
  if (!wasActive) saveActiveTab();
  const nextActive = index < activeTab ? activeTab - 1 : wasActive ? Math.min(index, tabs.length - 2) : activeTab;
  tabs.splice(index, 1);
  activeTab = nextActive;
  if (!wasActive) {
    renderTabbar();
    return;
  }
  tabBusy = true;
  try {
    await restoreTab(tabs[activeTab]);
  } finally {
    tabBusy = false;
  }
}

function cycleTab(delta: number): void {
  if (tabs.length <= 1) return;
  const next = (activeTab + delta + tabs.length) % tabs.length;
  void switchTab(next);
}

/** Another top-level window at the current place. */
function openNewWindow(): void {
  newWindow(location || undefined).catch((err) =>
    showToast(`Could not open a new window: ${cleanError(err)}`)
  );
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

/**
 * Go to `loc`. Listing happens before anything changes, so a folder that
 * can't be read leaves the current view intact and shows a toast instead.
 */
async function navigate(loc: string, opts: { history?: "push" | "none"; select?: string } = {}): Promise<boolean> {
  if (searchQuery !== null) closeSearch(false);
  closePreview();

  // Started now so it runs alongside the listing instead of after it.
  const rootLookup = lookupVersionedRoot(loc);
  let next: Entry[] = [];
  let error: string | null = null;
  let page: typeof sharePage = null;
  if (loc === NETWORK) await Promise.all([refreshShares(), refreshSftpServers()]);
  const shared = isSharePath(loc) ? receivedFor(loc) : undefined;
  if (shared && shared.status !== "online") {
    page = shared.status;
  } else if (hasEntries(loc)) {
    try {
      next = await listLocation(loc);
    } catch (err) {
      if (isSharePath(loc)) {
        page = receivedFor(loc)?.status === "no-access" ? "no-access" : "offline";
      } else if (location) {
        showAccessError(basename(loc), cleanError(err));
        return false;
      } else {
        error = cleanError(err);
      }
    }
  }

  if (opts.history !== "none" && location && loc !== location) {
    backStack.push(location);
    forwardStack = [];
  }

  location = loc;
  sharePage = page;
  versionedRootPath = await rootLookup;
  entries = next;
  listingError = error;
  counts.clear();
  countsVersion++;
  countsInFlight.clear();
  versionedFolders.clear();
  selected.clear();
  noteSelectionIntent();
  anchorIndex = cursorIndex = -1;
  if (opts.select) selected.add(opts.select);

  document.title = locationName(loc);
  if (activeTab >= 0 && tabs[activeTab]) tabs[activeTab].location = loc;
  renderTabbar();
  updateNavButtons();
  updateSidebarSelection();
  buildPathbar();
  updateShareSubtitle();
  renderView();
  scrollEl.scrollTop = 0;
  if (opts.select) revealPath(opts.select);
  viewEl.focus({ preventScroll: true });
  return true;
}

/** Re-list the current folder, keeping the selection where possible. */
async function reload(select?: string[]): Promise<void> {
  // A reload replaces the item elements under the pointer; wait until the
  // drag ends so a drop cannot land on a detached target.
  if (drag) {
    reloadAfterDrag = true;
    return;
  }
  if (!hasEntries(location)) return;
  if (isSharePath(location) || location === NETWORK) await refreshShares();
  if (location === NETWORK) await refreshSftpServers();
  if (showShareStatusPage()) return;
  try {
    entries = await listLocation(location);
    listingError = null;
    sharePage = null;
  } catch (err) {
    if (await listingFailed(err)) return;
  }
  const keep = select ?? [...selected];
  selected.clear();
  const paths = new Set(entries.map((e) => e.path));
  for (const p of keep) if (paths.has(p)) selected.add(p);
  const cursorPath = shown[cursorIndex]?.path;
  const anchorPath = shown[anchorIndex]?.path;
  const top = scrollEl.scrollTop;
  renderView();
  scrollEl.scrollTop = top;
  if (select?.[0]) revealPath(select[0]);
  else followCursor(cursorPath, anchorPath);
  versionedRootPath = await lookupVersionedRoot(location);
  updateShareSubtitle();
  if (searchQuery !== null) onSearchInput();
}

/** After a reload, keep the cursor and anchor on the same items: new ones
 *  may have landed above them. */
function followCursor(cursorPath: string | undefined, anchorPath: string | undefined): void {
  cursorIndex = indexOfPath(cursorPath, cursorIndex);
  anchorIndex = indexOfPath(anchorPath, anchorIndex);
  updateSelection();
}

/** Where `path` is now, or the old index (kept in range) when it's gone. */
function indexOfPath(path: string | undefined, fallback: number): number {
  const i = path === undefined ? -1 : shown.findIndex((e) => e.path === path);
  return i >= 0 ? i : Math.min(fallback, shown.length - 1);
}

/** A received share that isn't online shows its status page instead of a listing. */
function showShareStatusPage(): boolean {
  const received = isSharePath(location) ? receivedFor(location) : undefined;
  if (!received || received.status === "online") return false;
  entries = [];
  sharePage = received.status;
  renderView();
  return true;
}

/** Show why `reload` couldn't list the folder. True when it navigated away instead. */
async function listingFailed(err: unknown): Promise<boolean> {
  entries = [];
  if (isSharePath(location)) {
    sharePage = "offline";
    return false;
  }
  if (isLocalLocation(location) && isMissingPathError(err)) {
    // The folder went away under us: a cancelled copy deletes the folder it
    // was writing, and the view can still be showing it. Go to its parent
    // rather than stranding the view on an error page.
    if (await navigate(parentOf(location), { history: "none" })) return true;
  }
  listingError = cleanError(err);
  return false;
}

/** "Shared by Ahmed · Read only" under the path bar. */
function updateShareSubtitle(): void {
  if (!shareSubtitle) return;
  if (isSftpPath(location)) {
    shareSubtitle.hidden = false;
    shareSubtitle.textContent = "SSH server · Read only";
    return;
  }
  const received = isSharePath(location) ? receivedFor(location) : undefined;
  shareSubtitle.hidden = !received;
  shareSubtitle.textContent = received ? `Shared by ${received.sharer} · Read only` : "";
}

/** Refresh the received and locally shared folders. */
async function refreshShares(): Promise<void> {
  const [received, shared] = await Promise.all([
    listReceived().catch(() => []),
    listShares().catch(() => []),
  ]);
  networkShares = received;
  sharedFolders = shared;
}

/** Refresh the list of active SSH/SFTP servers. */
async function refreshSftpServers(): Promise<void> {
  sftpServerList = await sftpServers().catch(() => []);
}

/** The folder that keeps versions for `loc`, if any (never for Trash or Recent). */
function lookupVersionedRoot(loc: string): Promise<string | null> {
  if (isCloud(loc) || isSharePath(loc) || isSftpPath(loc) || isPseudo(loc) || (!!trashPath && loc === trashPath)) return Promise.resolve(null);
  return versionedRoot(loc).catch(() => null);
}

/** A failed folder open; macOS privacy blocks get a shortcut to the setting. */
function showAccessError(name: string, message: string): void {
  const blocked = isMac && /not permitted/i.test(message);
  showToast(`Unable to access “${name}”: ${message}`, blocked ? { button: { label: "Grant Access", onClick: openSettingsSafely } } : {});
}

function openSettingsSafely(): void {
  openFullDiskAccessSettings().catch((err) => showToast(`Could not open System Settings: ${cleanError(err)}`));
}

function cleanError(err: unknown): string {
  // "Failed to read directory: Permission denied (os error 13)" → "Permission denied"
  return String(err).replace(/^[^:]*:\s*/, "").replace(/\s*\(os error \d+\)$/, "");
}

/** True for ENOENT: the path was removed, not merely unreadable. */
function isMissingPathError(err: unknown): boolean {
  return /no such file or directory/i.test(String(err));
}

function goBack(): void {
  const prev = backStack.pop();
  if (!prev) return;
  const from = location;
  navigate(prev, { history: "none", select: childOf(prev, from) }).then((ok) => {
    if (ok) forwardStack.push(from);
    else backStack.push(prev);
    updateNavButtons();
  });
}

function goForward(): void {
  const next = forwardStack.pop();
  if (!next) return;
  const from = location;
  navigate(next, { history: "none" }).then((ok) => {
    if (ok) backStack.push(from);
    else forwardStack.push(next);
    updateNavButtons();
  });
}

function goUp(): void {
  if (isPseudo(location) || location === "/") return;
  if (isCloud(location) && isCloudRoot(location)) return;
  navigate(parentOf(location), { select: location });
}

/** If `child` lives directly inside `dir`, its path (so going back re-selects it). */
function childOf(dir: string, child: string): string | undefined {
  return !isPseudo(child) && parentOf(child) === dir ? child : undefined;
}

function updateNavButtons(): void {
  $<HTMLButtonElement>("#btn-back").disabled = backStack.length === 0;
  $<HTMLButtonElement>("#btn-forward").disabled = forwardStack.length === 0;
}

// ---------------------------------------------------------------------------
// Path bar
// ---------------------------------------------------------------------------

interface Crumb {
  label: string;
  path: string;
  icon?: string;
}

function crumbsFor(loc: string): Crumb[] {
  if (isPseudo(loc)) {
    const p = PSEUDO[loc.slice(PSEUDO_PREFIX.length) as Pseudo];
    return [{ label: p.label, path: loc, icon: p.icon }];
  }
  if (isCloud(loc)) {
    const parts = loc.slice(CLOUD_PREFIX.length).split("/").filter(Boolean);
    const account = parts.shift() ?? "";
    if (parts.length === 0) {
      return [{ label: "Google Drive", path: loc, icon: "network-workgroup" }];
    }
    let base = `${CLOUD_PREFIX}${account}`;
    const crumbs: Crumb[] = [];
    for (const segment of parts) {
      base += `/${segment}`;
      crumbs.push({ label: cloudSegmentName(segment) || "My Drive", path: base });
    }
    crumbs[0] = { ...crumbs[0], icon: "network-workgroup" };
    return crumbs;
  }
  if (isSharePath(loc)) {
    const parts = loc.slice(SHARE_PREFIX.length).split("/").filter(Boolean);
    const id = parts.shift() ?? "";
    if (!id) return [{ label: "Network", path: NETWORK, icon: "network-workgroup" }];
    const received = receivedFor(loc);
    const crumbs: Crumb[] = [
      { label: received?.folder || "Shared Folder", path: `${SHARE_PREFIX}${id}`, icon: "network-workgroup" },
    ];
    let base = `${SHARE_PREFIX}${id}`;
    for (const segment of parts) {
      base += `/${segment}`;
      let label = segment;
      try {
        label = decodeURIComponent(segment);
      } catch {
        /* keep the encoded name */
      }
      crumbs.push({ label, path: base });
    }
    return crumbs;
  }
  if (isSftpPath(loc)) {
    const parts = loc.slice(SFTP_PREFIX.length).split("/").filter(Boolean);
    const authority = parts.shift() ?? "";
    if (!authority) return [{ label: "Network", path: NETWORK, icon: "network-workgroup" }];
    const crumbs: Crumb[] = [
      { label: decodeURIComponent(authority), path: `${SFTP_PREFIX}${authority}/`, icon: "network-workgroup" },
    ];
    let base = `${SFTP_PREFIX}${authority}`;
    for (const segment of parts) {
      base += `/${segment}`;
      let label = segment;
      try {
        label = decodeURIComponent(segment);
      } catch {
        /* keep the encoded name */
      }
      crumbs.push({ label, path: base });
    }
    return crumbs;
  }
  if (trashPath && (loc === trashPath || loc.startsWith(trashPath + "/"))) {
    return [{ label: "Trash", path: trashPath, icon: "user-trash" }, ...segments(trashPath, loc)];
  }
  if (home && (loc === home || loc.startsWith(home + "/"))) {
    return [{ label: "Home", path: home, icon: "user-home" }, ...segments(home, loc)];
  }
  const volume = mountedVolumes
    .filter((v) => loc === v.path || loc.startsWith(v.path + "/"))
    .sort((a, b) => b.path.length - a.path.length)[0];
  if (volume) {
    return [{ label: volume.name, path: volume.path, icon: "drive-harddisk" }, ...segments(volume.path, loc)];
  }
  return [{ label: "", path: "/", icon: "drive-harddisk" }, ...segments("/", loc)];
}

function segments(base: string, loc: string): Crumb[] {
  const rest = loc.slice(base.length).split("/").filter(Boolean);
  return rest.map((name, i) => ({ label: name, path: joinPath(base, rest.slice(0, i + 1).join("/")) }));
}

function buildPathbar(): void {
  const crumbs = crumbsFor(location);
  crumbsEl.innerHTML = crumbs
    .map((c, i) => {
      const cls = i === crumbs.length - 1 ? "crumb current" : "crumb";
      const icon = c.icon ? sym(c.icon) : "";
      const label = c.label ? `<span>${escapeHtml(c.label)}</span>` : "";
      const badge = c.path === versionedRootPath ? versionsBadge("crumb-badge") : "";
      const title = c.label || "Computer";
      const sep = i > 0 ? `<span class="crumb-sep">/</span>` : "";
      return `${sep}<button class="${cls}" data-path="${escapeHtml(c.path)}" title="${escapeHtml(title)}">${icon}${label}${badge}</button>`;
    })
    .join("");
  requestAnimationFrame(() => {
    crumbsEl.scrollLeft = crumbsEl.scrollWidth;
    crumbsEl.classList.toggle("overflowing", crumbsEl.scrollWidth > crumbsEl.clientWidth);
  });
}

function openPathEntry(prefill?: string): void {
  if (searchQuery !== null) closeSearch(false);
  pathbarEl.classList.add("editing");
  crumbsEl.hidden = true;
  pathEntry.hidden = false;
  pathEntry.value = prefill ?? (isPseudo(location) ? "" : location);
  pathEntry.focus();
  if (prefill === undefined) pathEntry.select();
}

function closePathEntry(): void {
  if (pathEntry.hidden) return;
  pathEntry.hidden = true;
  crumbsEl.hidden = false;
  pathbarEl.classList.remove("editing");
  buildPathbar();
}

async function submitPathEntry(): Promise<void> {
  const raw = pathEntry.value.trim();
  if (isShareLink(raw)) {
    const result = await resolveShareLink(raw);
    if (!result) return;
    if ("error" in result) {
      showToast(result.error);
      pathEntry.focus();
      return;
    }
    closePathEntry();
    if (result.added) showToast("Folder added to Network");
    else showToast("This is your own shared folder.");
    await navigate(result.path);
    return;
  }
  const resolved = resolveTypedPath(pathEntry.value, location, home);
  if (resolved.error) {
    showToast(resolved.error);
    pathEntry.focus();
    return;
  }
  const typed = resolved.path;
  if (!typed) return;
  try {
    await listDir(typed, prefs.showHidden);
  } catch (err) {
    const message = cleanError(err);
    if (isSftpPath(typed) && /Connect to this server first/.test(message)) {
      closePathEntry();
      showConnectServerDialog(typed);
      return;
    }
    if (/not permitted/i.test(message)) showAccessError(typed, message);
    else showToast(`Unable to find “${typed}”: ${message}`);
    pathEntry.focus();
    return;
  }
  closePathEntry();
  await navigate(typed);
}

// ---------------------------------------------------------------------------
// Search (filters the current folder; replaces the path bar)
// ---------------------------------------------------------------------------

function openSearch(initial = ""): void {
  if (!hasEntries(location)) return;
  closePathEntry();
  if (searchQuery === null) {
    searchQuery = "";
    pathbarEl.hidden = true;
    searchbarEl.hidden = false;
    $("#btn-search").classList.add("open");
    searchEntry.placeholder = `Search ${locationName(location)}`;
  }
  if (initial) searchEntry.value = initial;
  searchEntry.focus();
  onSearchInput();
}

function closeSearch(rerender = true): void {
  if (searchQuery === null) return;
  searchQuery = null;
  searchResults = null;
  searchSeq += 1;
  clearTimeout(searchTimer);
  if (searching) void cancelSearch().catch(() => {});
  setSearching(false);
  showSearchStop(null);
  searchEntry.value = "";
  searchbarEl.hidden = true;
  pathbarEl.hidden = false;
  $("#btn-search").classList.remove("open");
  if (rerender) {
    renderView();
    viewEl.focus({ preventScroll: true });
  }
}

/** Recursive name search is for local folders; pseudo, Drive and shares filter in place. */
function canSearchRecursively(): boolean {
  return !isPseudo(location) && !isCloud(location) && !isSharePath(location) && !isSftpPath(location) && !inTrash();
}

function selectFirstMatch(): void {
  if (!searchQuery || shown.length === 0) return;
  selected.clear();
  selected.add(shown[0].path);
  cursorIndex = anchorIndex = 0;
  updateSelection();
}

function onSearchInput(): void {
  searchQuery = searchEntry.value;
  anchorIndex = cursorIndex = -1;
  searchResults = null;
  searchSeq += 1;
  clearTimeout(searchTimer);
  showSearchStop(null);
  const query = searchQuery.trim();
  const recursive = !!query && canSearchRecursively();
  if (searching && !recursive) void cancelSearch().catch(() => {});
  setSearching(recursive);
  if (recursive) {
    // Wait for a pause, then replace the quick local matches with the
    // recursive backend results as they arrive.
    const seq = searchSeq;
    searchTimer = window.setTimeout(() => runSearch(seq, query), 160);
  }
  renderView();
  // Like GNOME Files, the first local match is selected while search runs.
  selectFirstMatch();
}

function runSearch(seq: number, query: string): void {
  const at = location;
  const current = () => seq === searchSeq && searchQuery !== null && location === at;
  if (!current()) return;
  searchDir(at, query, prefs.showHidden, SEARCH_LIMIT, (batch) => {
    if (current()) addSearchResults(batch);
  })
    .then((summary) => {
      if (!current()) return;
      setSearching(false);
      if (searchResults === null) addSearchResults([]);
      else renderViewKeepingScroll(); // "Searching…" becomes "No Results Found"
      showSearchStop(summary);
    })
    .catch((err) => {
      if (!current()) return;
      setSearching(false);
      searchResults = [];
      renderView();
      showToast(`Could not search: ${cleanError(err)}`);
    });
}

/** Add streamed results. The first batch replaces the quick local matches;
 *  later ones keep the cursor and anchor on the same items as the list grows. */
function addSearchResults(batch: Entry[]): void {
  if (searchResults === null) {
    searchResults = batch;
    renderView();
    selectFirstMatch();
    return;
  }
  const cursorPath = shown[cursorIndex]?.path;
  const anchorPath = shown[anchorIndex]?.path;
  searchResults = searchResults.concat(batch);
  renderViewKeepingScroll();
  cursorIndex = shown.findIndex((e) => e.path === cursorPath);
  anchorIndex = shown.findIndex((e) => e.path === anchorPath);
  updateSelection();
}

function setSearching(on: boolean): void {
  searching = on;
  searchbarEl.classList.toggle("busy", on);
}

/** Say when a search stopped before looking everywhere, so a short list isn't
 *  mistaken for every match. */
function showSearchStop(summary: SearchSummary | null): void {
  const banner = $("#search-banner");
  banner.hidden = !summary?.stopped;
  if (!summary?.stopped) return;
  $("#search-banner-text").textContent = summary.stopped === "results"
    ? `Showing the first ${SEARCH_LIMIT.toLocaleString("en-US")} results. Try a more specific search.`
    : `Search stopped after looking at ${summary.scanned.toLocaleString("en-US")} items. Try a more specific search or a smaller folder.`;
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

function folderKind(entry: Entry): FolderKind {
  return folderKinds.get(entry.path) ?? "plain";
}

function sizeText(entry: Entry): string {
  if (!entry.is_dir) return formatSize(entry.size);
  const n = counts.get(entry.path);
  return n === undefined || n === null ? "" : formatItemCount(n);
}

function renderView(): void {
  computeShown();
  applyZoom();
  itemsLayer = null;
  windowRange = { start: 0, end: -1 };
  const pseudo = isPseudo(location) ? PSEUDO[location.slice(PSEUDO_PREFIX.length) as Pseudo] : null;

  if (sharePage) {
    viewEl.innerHTML = shareStatusPage(sharePage);
  } else if (pseudo && (!hasEntries(location) || (shown.length === 0 && !searchQuery))) {
    viewEl.innerHTML = statusPage(pseudo.icon, pseudo.title, pseudo.text);
  } else if (listingError !== null) {
    viewEl.innerHTML = statusPage("dialog-error", "Unable to Access Location", listingError);
  } else if (shown.length === 0 && searchQuery && searching) {
    viewEl.innerHTML = statusPage("system-search", "Searching…", "");
  } else if (shown.length === 0 && searchQuery) {
    viewEl.innerHTML = statusPage("system-search", "No Results Found", "Try a different search.");
  } else if (shown.length === 0 && location === trashPath) {
    viewEl.innerHTML = statusPage("user-trash", "Trash is Empty", "");
  } else if (shown.length === 0) {
    viewEl.innerHTML = statusPage("folder", "Folder is Empty", "");
  } else if (prefs.view === "grid") {
    viewEl.innerHTML = `<div class="grid"></div>`;
    itemsLayer = viewEl.querySelector<HTMLElement>(".grid");
  } else {
    viewEl.innerHTML = `<div class="list">${listHeaderHtml()}<div class="list-body"></div></div>`;
    itemsLayer = viewEl.querySelector<HTMLElement>(".list-body");
    // Sorting by size needs every folder's count; otherwise only visible rows ask.
    if (prefs.sortKey === "size") requestCounts(shown.filter((e) => e.is_dir).map((e) => e.path), true);
  }
  const overlay = viewEl.closest(".view-overlay") ?? viewEl;
  overlay.querySelector(".network-paste")?.remove();
  document.body.classList.toggle("network-view", location === NETWORK);
  if (location === NETWORK) renderNetworkPaste(overlay);
  relayout();
  updateViewButtons();
  updateFloatingBar();
}

async function disconnectServer(server: SftpServer): Promise<void> {
  try {
    await sftpDisconnect(server.id);
    await refreshSftpServers();
    await reload([]);
    showToast(`Disconnected from ${server.label}`);
  } catch (err) {
    showToast(`Could not disconnect: ${cleanError(err)}`);
  }
}

/** SSH/SFTP connection dialog, including host-key trust and password fallback. */
function showConnectServerDialog(prefill = "", connectNow = false): void {
  const extra = document.createElement("div");
  extra.className = "connect-fields";
  extra.innerHTML =
    `<label>Server<input class="entry" id="sftp-server" type="text" spellcheck="false" placeholder="user@host" /></label>` +
    `<label>Password<input class="entry" id="sftp-password" type="password" autocomplete="off" placeholder="Leave empty to use your SSH key" /></label>` +
    `<div class="field-message"></div>`;
  const serverInput = extra.querySelector<HTMLInputElement>("#sftp-server")!;
  const passwordInput = extra.querySelector<HTMLInputElement>("#sftp-password")!;
  const message = extra.querySelector<HTMLElement>(".field-message")!;
  serverInput.value = prefill;

  let dialog: ReturnType<typeof openDialog>;
  let busy = false;
  const attempt = async (trust: boolean) => {
    if (busy) return;
    const server = serverInput.value.trim();
    if (!server) {
      message.textContent = "Enter a server, for example user@host.";
      serverInput.focus();
      return;
    }
    busy = true;
    dialog.setResponseEnabled("connect", false);
    message.textContent = "Connecting…";
    try {
      const outcome = await sftpConnect(server, passwordInput.value || undefined, trust);
      if (outcome.kind === "connected") {
        dialog.close();
        await refreshSftpServers();
        await navigate(outcome.server.url);
        showToast(`Connected to ${outcome.server.label}`);
        return;
      }
      if (outcome.kind === "host-key-unknown") {
        message.textContent = "";
        openDialog({
          heading: "Trust This Server?",
          body: `The host key for ${outcome.server.label} is not known yet.\n\n${outcome.fingerprint}`,
          responses: [
            { id: "cancel", label: "Cancel" },
            { id: "trust", label: "Trust", style: "suggested" },
          ],
          defaultResponse: "cancel",
          closeResponse: "cancel",
          onResponse: (id) => {
            if (id !== "trust") return;
            void attempt(true);
          },
        });
        return;
      }
      message.textContent = passwordInput.value
        ? "Authentication failed. Check your username and password."
        : "No SSH key was accepted. Enter your password.";
      passwordInput.focus();
    } catch (err) {
      message.textContent = cleanError(err);
    } finally {
      busy = false;
      dialog.setResponseEnabled("connect", true);
    }
  };

  dialog = openDialog({
    heading: "Connect to Server",
    body: "Use an SSH key or your server password. Passwords are not saved.",
    extra,
    className: "connect-dialog",
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "connect", label: "Connect", style: "suggested" },
    ],
    defaultResponse: "connect",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "connect") return;
      void attempt(false);
      return false;
    },
  });
  serverInput.focus();
  if (connectNow && prefill) void attempt(false);
}

function shareStatusPage(kind: "offline" | "no-access" | "missing"): string {
  const received = receivedFor(location);
  const who = received?.sharer || "The sharer";
  switch (kind) {
    case "offline":
      return statusPage(
        "network-workgroup",
        `${who} Is Offline`,
        "Shared folders open while the computer sharing them is on and Files is running."
      );
    case "no-access":
      return statusPage(
        "dialog-error",
        "No Access",
        `${who} stopped sharing this folder with you.`
      );
    case "missing":
      return statusPage(
        "dialog-error",
        "Not Available",
        "The shared folder was moved or deleted."
      );
  }
}

/** The local folder a link belongs to, when the link is one of this machine's own. */
async function ownSharePath(link: string): Promise<string | null> {
  // By ticket, not text: the paste may differ in case, brackets or a slash.
  const ticket = shareTicket(link);
  for (const summary of await listShares().catch(() => [])) {
    const info = await shareInfo(summary.path).catch(() => null);
    if (!info) continue;
    for (const access of info.access) {
      const candidate = await shareLinkFor(info.id, access.id).catch(() => "");
      if (ticket !== null && shareTicket(candidate) === ticket) return info.path;
    }
  }
  return null;
}

/**
 * Add a pasted link to Network (or find its local folder when it is our own
 * link) and return the location to open. Errors are returned, not toasted, so
 * the Network paste bar can show them inline.
 */
async function resolveShareLink(
  text: string
): Promise<{ path: string; added: boolean } | { error: string } | null> {
  const link = text.trim();
  try {
    if (await shareLinkRequiresPassphrase(link)) {
      return await promptSharePassphrase((passphrase) => receiveShareLink(link, passphrase));
    }
    return await receiveShareLink(link);
  } catch (err) {
    return { error: cleanError(err) };
  }
}

async function receiveShareLink(link: string, passphrase?: string): Promise<{ path: string; added: boolean }> {
  try {
    const received = await addReceived(link, passphrase);
    await refreshShares();
    return { path: `${SHARE_PREFIX}${received.id}`, added: true };
  } catch (err) {
    const message = cleanError(err);
    if (/your own shared folder/i.test(message)) {
      const local = await ownSharePath(link);
      if (local) return { path: local, added: false };
    }
    throw message;
  }
}

function renderNetworkPaste(overlay: Element): void {
  const bar = document.createElement("div");
  bar.className = "network-paste";
  const input = document.createElement("input");
  input.className = "network-link";
  input.type = "text";
  input.placeholder = "Enter server address or share link";
  input.spellcheck = false;
  input.setAttribute("aria-label", "Server address or share link");
  const button = document.createElement("button");
  button.className = "btn suggested";
  button.textContent = "Connect";
  const error = document.createElement("span");
  error.className = "network-error";
  bar.append(input, button, error);

  const connect = async () => {
    const link = input.value.trim();
    if (!link || button.disabled) return;
    error.textContent = "";
    if (!isShareLink(link)) {
      // Anything else is a server address; the dialog asks for what it needs.
      showConnectServerDialog(link, true);
      return;
    }
    button.disabled = true;
    error.textContent = "";
    try {
      const result = await resolveShareLink(link);
      if (!result) return;
      if ("error" in result) {
        error.textContent = result.error;
        return;
      }
      if (!result.added) {
        showToast("This is your own shared folder.");
        await navigate(result.path);
        return;
      }
      input.value = "";
      await reload();
      showToast("Folder added to Network");
    } finally {
      button.disabled = false;
    }
  };
  button.addEventListener("click", () => void connect());
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void connect();
    }
  });
  overlay.appendChild(bar);
}

function statusPage(icon: string, title: string, text: string): string {
  return `<div class="status-page">${sym(icon)}<h1>${escapeHtml(title)}</h1>${text ? `<p>${escapeHtml(text)}</p>` : ""}</div>`;
}

function itemClasses(e: Entry, i: number): string {
  const cut = clipboard?.mode === "cut" && clipboard.paths.includes(e.path);
  const offline = e.shareStatus === "offline" || e.shareStatus === "no-access" || e.shareStatus === "missing";
  return (e.is_hidden ? " hidden-entry" : "") + (cut ? " cut" : "") + (offline ? " offline" : "") +
    (selected.has(e.path) ? " selected" : "") + (i === cursorIndex ? " cursor" : "") +
    // Striped lists (macOS theme): rows are virtualized, so :nth-child can't tell.
    (i % 2 ? " odd" : "");
}

function listHeaderHtml(): string {
  if (location === NETWORK) {
    return `<div class="list-header network"><span>Name</span><span>Details</span><span>Status</span><span></span></div>`;
  }
  const col = (key: SortKey, label: string) => {
    const active = prefs.sortKey === key;
    // The arrow shows the visual direction: name ascends A→Z, size/date descend.
    const descending = (key === "name" || key === "type") === prefs.sortReversed;
    const arrow = active ? sym(descending ? "sort-descending" : "sort-ascending") : "";
    return `<button data-sort="${key}" class="${active ? "sorted" : ""}">${label}${arrow}</button>`;
  };
  if (location === RECENT) {
    // Fixed order, so the headers are plain labels.
    return `<div class="list-header with-where"><span>Name</span><span>Size</span><span>Location</span><span>Recency</span><span></span></div>`;
  }
  const trash = location === trashPath;
  const where = showsWhere() ? `<span>${trash ? "Original Location" : "Location"}</span>` : "";
  return `<div class="list-header${where ? " with-where" : ""}">${col("name", "Name")}${col("size", "Size")}${where}${col("modified", trash ? "Trashed On" : "Modified")}<span></span></div>`;
}

function itemHtml(i: number): string {
  const e = shown[i];
  const { x, y } = itemPos(i);
  const pos = `style="left:${x}px;top:${y}px"`;
  const cls = itemClasses(e, i);
  const sel = selected.has(e.path);
  if (prefs.view === "grid") {
    const detail = location === NETWORK ? networkDetail(e) : "";
    const shared = detail ? `<div class="shared-by">${escapeHtml(detail)}</div>`
      : searchQuery !== null ? `<div class="shared-by where" title="${escapeHtml(parentOf(e.path))}">${escapeHtml(whereText(e.path))}</div>` : "";
    return `<div class="grid-item item${cls}" data-index="${i}" role="option" aria-selected="${sel}" title="${escapeHtml(e.name)}" ${pos}>` +
      `<div class="icon">${iconHtml(e)}</div>` +
      `<div class="name">${escapeHtml(e.name)}</div>${shared}</div>`;
  }
  if (location === NETWORK) {
    const status = networkStatus(e);
    return `<div class="list-row item network${cls}" data-index="${i}" role="option" aria-selected="${sel}" ${pos}>` +
      `<div class="name-cell"><div class="icon">${iconHtml(e)}</div><span class="name">${escapeHtml(e.name)}</span></div>` +
      `<span class="meta shared-by-cell">${escapeHtml(networkDetail(e))}</span>` +
      `<span class="meta network-status ${escapeHtml(status.cls)}">${escapeHtml(status.label)}</span>` +
      `<span></span></div>`;
  }
  const date = location === RECENT ? e.used ?? e.modified : e.trashed ?? e.modified;
  // In the Trash, the location is where the item came from.
  const from = location === trashPath ? e.origin : e.path;
  const where = !showsWhere() ? "" : from
    ? `<span class="meta where" title="${escapeHtml(parentOf(from))}">${escapeHtml(whereText(from))}</span>`
    : `<span class="meta where"></span>`;
  return `<div class="list-row item${cls}${where ? " with-where" : ""}" data-index="${i}" role="option" aria-selected="${sel}" ${pos}>` +
    `<div class="name-cell"><div class="icon">${iconHtml(e)}</div><span class="name">${escapeHtml(e.name)}</span></div>` +
    `<span class="meta size">${sizeText(e)}</span>${where}` +
    `<span class="meta" title="${escapeHtml(formatDateDetailed(date))}">${formatDate(date)}</span>${starButton(e)}</div>`;
}

/** The star at the end of a list row: shown on hover, or always once starred. */
function starButton(e: Entry): string {
  if (inTrash()) return "<span></span>";
  const on = starred.has(e.path);
  return `<button class="star${on ? " starred" : ""}" tabindex="-1" title="${on ? "Unstar" : "Star"}" aria-label="${on ? "Unstar" : "Star"}">${sym(on ? "starred" : "non-starred")}</button>`;
}

/** The "Location" column: the folder holding `path`, relative to Home where possible. */
function whereText(path: string): string {
  const dir = parentOf(path);
  if (dir === home) return "Home";
  if (home && dir.startsWith(home + "/")) return dir.slice(home.length + 1);
  return dir;
}

// ---------------------------------------------------------------------------
// Thumbnails (images, video, PDF and documents, via Quick Look on macOS)
// ---------------------------------------------------------------------------

const THUMB_KINDS = new Set<FileKind>(["image", "video", "pdf", "document", "spreadsheet", "presentation"]);

function wantsThumb(e: Entry): boolean {
  return prefs.showThumbnails && !e.is_dir && THUMB_KINDS.has(fileKind(e.name));
}

/** Pixel size to request: the displayed icon size at screen resolution, in a few steps. */
function thumbSize(): number {
  const shownAt = prefs.view === "grid" ? GRID_SIZES[prefs.gridZoom] : LIST_SIZES[prefs.listZoom];
  const px = shownAt * devicePixelRatio;
  return px <= 128 ? 128 : px <= 256 ? 256 : 512;
}

function thumbKeyFor(e: Entry, size = thumbSize()): string {
  return thumbKey(e.path, e.modified, e.size, size);
}

/** The thumbnail if it's loaded, otherwise the type icon. */
function iconHtml(e: Entry): string {
  if (wantsThumb(e)) {
    const url = cachedThumb(thumbKeyFor(e));
    if (url) {
      const link = e.is_symlink ? `<span class="emblem">${sym("emblem-symbolic-link")}</span>` : "";
      return `<img class="thumb" src="${url}" alt="" draggable="false">${link}`;
    }
  }
  if (location === NETWORK && isSftpPath(e.path)) return `<span class="server-glyph">${sym("network-server")}</span>`;
  const icon = nativeIconHtml(e) ?? entryIcon(e, folderKind(e), versionedFolders.has(e.path));
  if (location === NETWORK) return `${icon}<span class="emblem">${sym("network-workgroup")}</span>`;
  if (sharedFolders.some((share) => share.path === e.path)) {
    return `${icon}<span class="emblem">${sym("network-workgroup")}</span>`;
  }
  return icon;
}

// ---------------------------------------------------------------------------
// Icons from the operating system (src/native-icons.ts), drawn in place of
// the app's own once loaded
// ---------------------------------------------------------------------------

function wantsNativeIcon(e: Entry): boolean {
  return location !== NETWORK && !isRemotePath(e.path);
}

function nativeIconPx(): number {
  return nativeIconSize(prefs.view === "grid" ? GRID_SIZES[prefs.gridZoom] : LIST_SIZES[prefs.listZoom]);
}

/** The OS icon with the app's badges, or null until it's loaded (or if there is none). */
function nativeIconHtml(e: Entry): string | null {
  if (!wantsNativeIcon(e)) return null;
  const url = nativeIconUrl(e.path, nativeIconPx());
  if (!url) return null;
  const link = e.is_symlink ? `<span class="emblem">${sym("emblem-symbolic-link")}</span>` : "";
  const versions = versionedFolders.has(e.path) ? versionsBadge() : "";
  return `<img class="native-icon" src="${url}" alt="" draggable="false">${link}${versions}`;
}

/** Swap the OS icon for `key` (a path, or an `appIconKey`) into a dialog's
 *  icon once it loads; the app's drawing stays meanwhile, and wherever the OS
 *  has none. Badges next to the drawing are kept. */
function useNativeIcon(icon: HTMLElement, key: string, shownAt: number): void {
  if (isRemotePath(key)) return;
  void nativeIcon(key, nativeIconSize(shownAt)).then((url) => {
    const drawing = icon.querySelector("svg.fc, .versioned-icon");
    if (!url || !drawing) return;
    const img = document.createElement("img");
    img.className = "native-icon";
    img.src = url;
    img.alt = "";
    img.draggable = false;
    drawing.replaceWith(img);
  });
}

/** Ask for the OS icons of the rendered items. */
function loadItemIcons(start: number, end: number): void {
  const paths: string[] = [];
  for (let i = start; i <= end; i++) if (wantsNativeIcon(shown[i])) paths.push(shown[i].path);
  if (paths.length) loadNativeIcons(paths, nativeIconPx(), (ready) => ready.forEach(patchIcon));
}

/** Request thumbnails for the rendered items; queued ones scrolled away are dropped. */
function loadThumbs(start: number, end: number, visible: { start: number; end: number }): void {
  if (!prefs.showThumbnails) return;
  const size = thumbSize();
  const keys = new Set<string>();
  // Visible rows first, then the overscan around them: the order is the
  // queue's priority (see keepOnly).
  const order: number[] = [];
  for (let i = visible.start; i <= visible.end; i++) order.push(i);
  for (let i = start; i <= end; i++) if (i < visible.start || i > visible.end) order.push(i);
  for (const i of order) {
    const e = shown[i];
    if (!wantsThumb(e)) continue;
    const key = thumbKeyFor(e, size);
    keys.add(key);
    if (cachedThumb(key) === undefined) {
      requestThumb(key, e.path, size, (url) => url && patchIcon(e.path));
    }
  }
  keepOnly(keys);
}

function patchIcon(path: string): void {
  if (!itemsLayer) return;
  for (const el of itemsLayer.children as HTMLCollectionOf<HTMLElement>) {
    const e = shown[Number(el.dataset.index)];
    if (e?.path === path) el.querySelector(".icon")!.innerHTML = iconHtml(e);
  }
}

function toggleThumbnails(): void {
  prefs.showThumbnails = !prefs.showThumbnails;
  savePrefs();
  renderViewKeepingScroll();
}

function itemEl(index: number): HTMLElement | null {
  return viewEl.querySelector<HTMLElement>(`.item[data-index="${index}"]`);
}

// ---------------------------------------------------------------------------
// Virtualized layout. Items sit in fixed-size slots positioned by arithmetic,
// and only the ones in or near the viewport exist in the DOM, so a folder
// with 10k entries opens and scrolls like a small one.
// ---------------------------------------------------------------------------

interface Layout {
  cols: number;
  itemW: number;
  itemH: number;
  gap: number;
  padX: number;
  padY: number;
  /** Where the items layer starts within the scrolled content. */
  bodyTop: number;
}

let layout: Layout = { cols: 1, itemW: 0, itemH: 0, gap: 0, padX: 0, padY: 0, bodyTop: 0 };
let itemsLayer: HTMLElement | null = null;
let windowRange = { start: 0, end: -1 };

function computeLayout(): void {
  const sizes = LAYOUT_SIZES[currentTheme];
  const width = scrollEl.clientWidth;
  const lineH = parseFloat(getComputedStyle(document.body).fontSize) * 1.3;
  if (prefs.view === "grid") {
    const icon = GRID_SIZES[prefs.gridZoom];
    const inner = width - sizes.gridPad * 2;
    const cols = Math.max(1, Math.floor((inner + sizes.gridGap) / (icon + 36 + sizes.gridGap)));
    const itemW = Math.floor((inner - sizes.gridGap * (cols - 1)) / cols);
    // icon + gap + two lines of label + padding. Network adds one line for
    // "Shared by …", search one for the folder each result is in.
    const extraLine = location === NETWORK || searchQuery !== null;
    const itemH = Math.ceil(6 + icon + 6 + 2 * lineH + 9 + (extraLine ? lineH : 0));
    layout = { cols, itemW, itemH, gap: sizes.gridGap, padX: sizes.gridPad, padY: sizes.gridPad, bodyTop: 0 };
  } else {
    const itemH = Math.max(LIST_SIZES[prefs.listZoom] + sizes.listPad, sizes.listMinRow);
    layout = { cols: 1, itemW: width - 12, itemH, gap: 0, padX: 6, padY: 0, bodyTop: sizes.listHeader };
  }
}

/** Top-left of item `i` within the items layer. */
function itemPos(i: number): { x: number; y: number } {
  const row = Math.floor(i / layout.cols);
  const col = i % layout.cols;
  return { x: layout.padX + col * (layout.itemW + layout.gap), y: layout.padY + row * (layout.itemH + layout.gap) };
}

function relayout(): void {
  if (!itemsLayer) return;
  computeLayout();
  const rows = Math.ceil(shown.length / layout.cols);
  const height = layout.padY * 2 + rows * (layout.itemH + layout.gap) - layout.gap + (prefs.view === "list" ? 6 : 0);
  itemsLayer.style.height = `${height}px`;
  viewEl.style.setProperty("--cell-w", `${layout.itemW}px`);
  viewEl.style.setProperty("--cell-h", `${layout.itemH}px`);
  renderWindow(true);
}

/** Render the items that intersect the viewport (plus a few rows either side). */
function renderWindow(force = false): void {
  if (!itemsLayer) return;
  const pitch = layout.itemH + layout.gap;
  const top = scrollEl.scrollTop - layout.bodyTop - layout.padY;
  const overscan = prefs.view === "grid" ? 2 : 8;
  const firstVisible = Math.max(0, Math.floor(top / pitch));
  const lastVisible = Math.floor((top + scrollEl.clientHeight) / pitch);
  const firstRow = Math.max(0, firstVisible - overscan);
  const lastRow = lastVisible + overscan;
  const start = firstRow * layout.cols;
  const end = Math.min(shown.length - 1, (lastRow + 1) * layout.cols - 1);
  if (!force && start === windowRange.start && end === windowRange.end) return;
  windowRange = { start, end };
  const html: string[] = [];
  for (let i = start; i <= end; i++) html.push(itemHtml(i));
  itemsLayer.innerHTML = html.join("");
  loadItemIcons(start, end);
  loadThumbs(start, end, {
    start: firstVisible * layout.cols,
    end: Math.min(end, (lastVisible + 1) * layout.cols - 1),
  });
  // Both views: list view shows the counts, and either view shows the
  // version badge that comes with them. Only rendered folders are asked.
  const dirs: string[] = [];
  for (let i = start; i <= end; i++) if (shown[i].is_dir) dirs.push(shown[i].path);
  requestCounts(dirs, false);
}

/** Scroll just enough to show item `i`, then render it. */
function scrollToIndex(i: number): void {
  if (!itemsLayer || i < 0 || i >= shown.length) return;
  const y = layout.bodyTop + itemPos(i).y;
  const visibleTop = scrollEl.scrollTop + layout.bodyTop;
  if (y - layout.padY < visibleTop) {
    scrollEl.scrollTop = y - layout.bodyTop - layout.padY;
  } else if (y + layout.itemH + layout.padY > scrollEl.scrollTop + scrollEl.clientHeight) {
    scrollEl.scrollTop = y + layout.itemH + layout.padY - scrollEl.clientHeight;
  }
  renderWindow();
}

// ---------------------------------------------------------------------------
// Themes (src/theme.ts): the look only, picked by the platform
// ---------------------------------------------------------------------------

/** Show the platform's theme: CSS restyles everything from `data-theme`. */
function applyTheme(): void {
  document.documentElement.dataset.theme = currentTheme;
}

function applyZoom(): void {
  const root = document.documentElement.style;
  root.setProperty("--icon-size", `${GRID_SIZES[prefs.gridZoom]}px`);
  root.setProperty("--list-icon-size", `${LIST_SIZES[prefs.listZoom]}px`);
}

function zoom(delta: number): void {
  if (prefs.view === "grid") {
    prefs.gridZoom = Math.min(Math.max(prefs.gridZoom + delta, 0), GRID_SIZES.length - 1);
  } else {
    prefs.listZoom = Math.min(Math.max(prefs.listZoom + delta, 0), LIST_SIZES.length - 1);
  }
  savePrefs();
  applyZoom();
  relayout();
  scrollToIndex(cursorIndex);
}

function resetZoom(): void {
  prefs.gridZoom = LAYOUT_SIZES[currentTheme].gridZoom;
  prefs.listZoom = 1;
  savePrefs();
  applyZoom();
  relayout();
  scrollToIndex(cursorIndex);
}

// ---------------------------------------------------------------------------
// Zooming with the wheel or a trackpad pinch
//
// GNOME Files zooms with Ctrl + wheel, and on macOS a trackpad pinch is the
// same gesture with a modifier. Wheel deltas arrive in three flavours (pixels,
// lines, pages) and trackpads send dozens of tiny ones per second, so a notch
// steps once and small deltas accumulate towards a step.
// ---------------------------------------------------------------------------

/** A `gesturechange` event, which the DOM type definitions don't declare. */
interface GestureEventLike extends Event {
  scale?: number;
}

/** Wheel/trackpad distance still needed before the next zoom step. */
const ZOOM_STEP_PX = 24;
/** A single wheel notch is at least this far, in any delta mode. */
const ZOOM_NOTCH_PX = 50;

let zoomAccumulator = 0;
/** True between `gesturestart` and `gestureend`, so pinch and wheel can't
 * both answer the same gesture. */
let pinching = false;
/** Zoom steps already taken in the current pinch. */
let pinchSteps = 0;

function onViewWheel(e: WheelEvent): void {
  // Plain scrolling must keep scrolling; only the modifier (which is also
  // what macOS sends for a pinch) zooms.
  if (!(e.ctrlKey || e.metaKey) || pinching) return;
  // Without this the webview applies its own page zoom as well.
  e.preventDefault();

  // Normalise: a line is ~16 px, a page is roughly a viewport.
  const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
  const delta = -e.deltaY * unit;
  if (delta === 0) return;

  // One notch, one step — wheel hardware reports a big delta per notch and
  // its inertia would otherwise run through the whole zoom range.
  if (Math.abs(delta) >= ZOOM_NOTCH_PX) {
    zoomAccumulator = 0;
    zoom(Math.sign(delta));
    return;
  }
  zoomAccumulator += delta;
  if (Math.abs(zoomAccumulator) >= ZOOM_STEP_PX) {
    zoom(Math.sign(zoomAccumulator));
    zoomAccumulator = 0;
  }
}

/** Scrolling without a zoom gesture starts a fresh accumulation. */
function onViewScroll(): void {
  zoomAccumulator = 0;
}

function onGestureStart(e: Event): void {
  // Stops the webview from magnifying the whole page instead.
  e.preventDefault();
  pinching = true;
  pinchSteps = 0;
}

function onGestureChange(e: Event): void {
  if (!pinching) return;
  e.preventDefault();
  const scale = (e as GestureEventLike).scale ?? 1;
  // Every 15% of pinch is one icon-size step.
  const target = Math.round((scale - 1) / 0.15);
  const steps = target - pinchSteps;
  if (steps === 0) return;
  // At most two steps per event so a jumpy pinch doesn't skip sizes; the
  // rest follow on the next event, so pinching back lands where it began.
  const taken = Math.sign(steps) * Math.min(Math.abs(steps), 2);
  pinchSteps += taken;
  zoom(taken);
}

function onGestureEnd(e: Event): void {
  e.preventDefault();
  pinching = false;
  pinchSteps = 0;
}

function setView(view: ViewMode): void {
  if (prefs.view === view) return;
  prefs.view = view;
  savePrefs();
  renderView();
  scrollToIndex(cursorIndex);
}

function setSort(key: SortKey, reversed: boolean): void {
  prefs.sortKey = key;
  prefs.sortReversed = reversed;
  savePrefs();
  const keep = cursorIndex >= 0 ? shown[cursorIndex]?.path : undefined;
  renderView();
  if (keep) {
    cursorIndex = anchorIndex = shown.findIndex((e) => e.path === keep);
  }
}

function toggleHidden(): void {
  prefs.showHidden = !prefs.showHidden;
  savePrefs();
  reload();
}

function updateViewButtons(): void {
  const toggle = $("#btn-view-toggle");
  // Shows the view it switches to, like GNOME Files.
  const target = prefs.view === "grid" ? "list" : "grid";
  toggle.innerHTML = sym(`view-${target}`);
  toggle.title = target === "list" ? `List View (${MOD}1)` : `Grid View (${MOD}2)`;
}

/** Fetch "N items" for `paths`, then patch the visible cells. */
const countsInFlight = new Set<string>();
async function requestCounts(paths: string[], resortWhenDone: boolean): Promise<void> {
  const loc = location;
  // Remote items have no local item count; Drive returns null, Network has no size cell.
  const pending = paths.filter((p) => !isCloud(p) && !isSharePath(p) && !isSftpPath(p) && !counts.has(p) && !countsInFlight.has(p));
  if (pending.length === 0) return;
  pending.forEach((p) => countsInFlight.add(p));
  for (let i = 0; i < pending.length; i += 400) {
    const batch = pending.slice(i, i + 400);
    let result: (FolderSummary | null)[] = [];
    try {
      result = await itemCounts(batch, prefs.showHidden);
    } catch {
      /* leave those cells blank */
    }
    batch.forEach((p) => countsInFlight.delete(p));
    if (loc !== location) return;
    batch.forEach((p, j) => rememberSummary(p, result[j] ?? null));
    patchCounts();
  }
  if (resortWhenDone && prefs.sortKey === "size") renderViewKeepingScroll();
}

/** Store a folder's count, and redraw its icon if its version badge changed. */
function rememberSummary(path: string, summary: FolderSummary | null): void {
  counts.set(path, summary?.count ?? null);
  countsVersion++;
  const versioned = !!summary?.versioned;
  if (versioned === versionedFolders.has(path)) return;
  if (versioned) versionedFolders.add(path);
  else versionedFolders.delete(path);
  patchIcon(path);
}

function patchCounts(): void {
  viewEl.querySelectorAll<HTMLElement>(".list-row").forEach((row) => {
    const e = shown[Number(row.dataset.index)];
    const cell = row.querySelector(".size");
    if (e?.is_dir && cell) cell.textContent = sizeText(e);
  });
  updateFloatingBar();
}

function renderViewKeepingScroll(): void {
  const top = scrollEl.scrollTop;
  renderView();
  scrollEl.scrollTop = top;
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

/** Sync the rendered items (only those in the DOM) with the selection state. */
function updateSelection(): void {
  viewEl.querySelectorAll<HTMLElement>(".item").forEach((el) => {
    const i = Number(el.dataset.index);
    const isSel = selected.has(shown[i]?.path);
    el.classList.toggle("selected", isSel);
    el.setAttribute("aria-selected", String(isSel));
    el.classList.toggle("cursor", i === cursorIndex);
  });
  updateFloatingBar();
}

function selectOnly(index: number): void {
  noteSelectionIntent();
  selected.clear();
  if (shown[index]) selected.add(shown[index].path);
  anchorIndex = cursorIndex = index;
  updateSelection();
}

function selectRange(from: number, to: number, additive: boolean): void {
  noteSelectionIntent();
  if (!additive) selected.clear();
  const [a, b] = from < to ? [from, to] : [to, from];
  for (let i = a; i <= b; i++) if (shown[i]) selected.add(shown[i].path);
  cursorIndex = to;
  updateSelection();
}

function selectAll(): void {
  noteSelectionIntent();
  for (const e of shown) selected.add(e.path);
  updateSelection();
}

function clearSelection(): void {
  noteSelectionIntent();
  selected.clear();
  updateSelection();
}

function selectedEntries(): Entry[] {
  return shown.filter((e) => selected.has(e.path));
}

function revealPath(path: string): void {
  const i = shown.findIndex((e) => e.path === path);
  if (i < 0) return;
  anchorIndex = cursorIndex = i;
  updateSelection();
  scrollToIndex(i);
}

/** The floating bar in the bottom-right corner describes the selection. */
function updateFloatingBar(): void {
  const sel = selectedEntries();
  if (sel.length === 0) {
    floatingBar.hidden = true;
    return;
  }
  let text: string;
  if (sel.length === 1) {
    const e = sel[0];
    if (isAppBundle(e)) {
      text = `“${e.name}” selected`;
    } else if (e.is_dir) {
      const n = counts.get(e.path);
      if (n === undefined) fetchCount(e.path);
      text = `“${e.name}” selected` + (n === undefined || n === null ? "" : ` (containing ${n === 1 ? "1 item" : `${n.toLocaleString()} items`})`);
    } else {
      text = `“${e.name}” selected (${formatSize(e.size)})`;
    }
  } else {
    const files = sel.filter((e) => !e.is_dir);
    const folders = sel.length - files.length;
    if (folders === 0) {
      text = `${sel.length.toLocaleString()} items selected (${formatSize(files.reduce((s, e) => s + e.size, 0))})`;
    } else if (files.length === 0) {
      text = `${folders.toLocaleString()} folders selected`;
    } else {
      text = `${sel.length.toLocaleString()} items selected`;
    }
  }
  floatingBar.textContent = text;
  floatingBar.hidden = false;
}

function fetchCount(path: string): void {
  counts.set(path, null);
  countsVersion++;
  itemCounts([path], prefs.showHidden).then(([summary]) => {
    rememberSummary(path, summary);
    updateFloatingBar();
  }).catch(() => {});
}

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

function activate(entry: Entry): void {
  if (entry.is_dir && !isAppBundle(entry)) navigate(entry.path);
  else openEntry(entry);
}

async function openEntry(entry: Entry): Promise<void> {
  // Remote files download first, so they show as an operation (spec D6).
  const op = isRemotePath(entry.path)
    ? startOperation(opTitle("open", [entry.name]), {
      totalItems: 1,
      onCancel: () => void cancelOperation(op!.id).catch(() => {}),
    })
    : null;
  if (op) setOperationState(op, "working");
  try {
    await openFile(entry.path, op ? { op: op.id, onProgress: (u) => updateOperation(op, u) } : undefined);
  } catch (err) {
    if (cleanError(err) !== CANCELLED) showToast(`Could not open “${entry.name}”: ${cleanError(err)}`);
    return;
  } finally {
    if (op) endOperation(op);
  }
  if (isCloud(entry.path) && !entry.cloud?.is_google_doc && !cloudOpenNoticeShown) {
    cloudOpenNoticeShown = true;
    showToast("Opened a local copy. Changes are not uploaded to Google Drive.");
  }
  if (!isAppBundle(entry) && !isCloud(entry.path) && !isSharePath(entry.path) && !isSftpPath(entry.path)) addRecent(entry.path).catch(() => {});
}

/** Enter / Open: one item activates; several open every file (folders can't all be shown). */
function activateSelection(): void {
  const sel = selectedEntries();
  if (sel.length === 1) return activate(sel[0]);
  const openable = sel.filter((e) => !e.is_dir || isAppBundle(e));
  if (openable.length) openable.forEach(openEntry);
  else if (sel.length) navigate(sel[0].path);
}

// ---------------------------------------------------------------------------
// Mouse interaction in the view: click, modifiers, rubber band, menus
// ---------------------------------------------------------------------------

function indexFromEvent(e: MouseEvent): number {
  const item = (e.target as HTMLElement).closest<HTMLElement>(".item");
  return item ? Number(item.dataset.index) : -1;
}

/**
 * The item a press acts on. A grid tile is wider and taller than its icon and
 * name, so only those count: a press on the rest of the tile or in the gutter
 * starts a rubber band, which is otherwise hard to begin between items.
 */
function itemIndexAt(e: MouseEvent): number {
  const index = indexFromEvent(e);
  if (index < 0 || prefs.view !== "grid") return index;
  return (e.target as HTMLElement).closest(".grid-item > *") ? index : -1;
}

function onViewMouseDown(e: MouseEvent): void {
  viewEl.classList.remove("keyboard-nav");
  cancelPendingNarrow();
  const sortBtn = (e.target as HTMLElement).closest<HTMLElement>("[data-sort]");
  if (sortBtn) return;

  const index = itemIndexAt(e);
  if (index >= 0 && e.button === 0 && (e.target as HTMLElement).closest(".star")) {
    setStarred([shown[index].path], !starred.has(shown[index].path));
    return;
  }
  const toggleMod = isMac ? e.metaKey : e.ctrlKey;

  if (index < 0) {
    if (e.button !== 0) return;
    if (!toggleMod && !e.shiftKey) clearSelection();
    startRubberband(e, toggleMod || e.shiftKey);
    return;
  }

  // Right-clicks select in `onViewContextMenu`, which also sees the ones
  // whose press went to a menu backdrop.
  if (e.button !== 0) return;

  if (e.shiftKey && anchorIndex >= 0) {
    selectRange(anchorIndex, index, toggleMod);
  } else if (toggleMod) {
    noteSelectionIntent();
    const p = shown[index].path;
    if (selected.has(p)) selected.delete(p);
    else selected.add(p);
    anchorIndex = cursorIndex = index;
    updateSelection();
  } else if (!selected.has(shown[index].path)) {
    selectOnly(index);
  } else {
    // Clicking inside an existing multi-selection narrows it on release,
    // unless the press turns into a drag of the whole selection.
    const narrow = () => {
      selectOnly(index);
      viewEl.removeEventListener("mouseup", narrow);
      pendingNarrow = null;
    };
    pendingNarrow = narrow;
    viewEl.addEventListener("mouseup", narrow);
    anchorIndex = cursorIndex = index;
    updateSelection();
  }
  if (selected.has(shown[index].path)) armDrag(e);
}

function startRubberband(start: MouseEvent, additive: boolean): void {
  noteSelectionIntent();
  const band = $("#rubberband");
  const overlay = scrollEl.parentElement!;
  const base = new Set(additive ? selected : []);
  const origin = overlay.getBoundingClientRect();
  const startX = start.clientX - origin.left;
  const startY = start.clientY - origin.top + scrollEl.scrollTop;
  let moved = false;

  const onMove = (e: MouseEvent) => {
    const x = Math.min(Math.max(e.clientX - origin.left, 0), overlay.clientWidth);
    const y = Math.max(e.clientY - origin.top + scrollEl.scrollTop, 0);
    if (!moved && Math.hypot(x - startX, y - startY) < 4) return;
    moved = true;
    const l = Math.min(x, startX), r = Math.max(x, startX);
    const t = Math.min(y, startY), b = Math.max(y, startY);
    band.hidden = false;
    band.style.left = `${l}px`;
    band.style.top = `${t - scrollEl.scrollTop}px`;
    band.style.width = `${r - l}px`;
    band.style.height = `${b - t}px`;
    selected.clear();
    for (const p of base) selected.add(p);
    // Items hit by the band, found from the layout rather than the DOM.
    const pitchX = layout.itemW + layout.gap;
    const pitchY = layout.itemH + layout.gap;
    const top = t - layout.bodyTop - layout.padY;
    const bottom = b - layout.bodyTop - layout.padY;
    const firstRow = Math.max(0, Math.floor(top / pitchY));
    const lastRow = Math.floor(bottom / pitchY);
    const firstCol = Math.max(0, Math.floor((l - layout.padX) / pitchX));
    const lastCol = Math.min(layout.cols - 1, Math.floor((r - layout.padX) / pitchX));
    for (let row = firstRow; row <= lastRow; row++) {
      // Skip the gap below a row if the band only covers that gap.
      if (row * pitchY + layout.itemH < top) continue;
      for (let col = firstCol; col <= lastCol; col++) {
        if ((l - layout.padX) > col * pitchX + layout.itemW) continue;
        const i = row * layout.cols + col;
        if (i < shown.length) selected.add(shown[i].path);
      }
    }
    // Scroll when dragging past the top or bottom edge.
    const edge = e.clientY - origin.top;
    if (edge > overlay.clientHeight - 16) scrollEl.scrollTop += 12;
    else if (edge < 16) scrollEl.scrollTop -= 12;
    updateSelection();
  };
  const onUp = () => {
    band.hidden = true;
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}

function onViewContextMenu(e: MouseEvent): void {
  e.preventDefault();
  // A ⌃-click on macOS starts as a left press that may have armed a narrow;
  // the menu swallows its release, so drop it here.
  cancelPendingNarrow();
  if (!hasEntries(location) || listingError !== null) return;
  const at = { x: e.clientX, y: e.clientY };
  const index = itemIndexAt(e);
  if (index >= 0 && shown[index]) {
    // The one place a right-click selects: select the item that was
    // right-clicked, like GNOME Files, unless it is already part of a
    // selection (then the menu applies to the whole selection). Doing it here
    // rather than on mousedown also covers a press that closed another menu.
    if (!selected.has(shown[index].path)) selectOnly(index);
    openItemMenu(at);
  } else {
    openBackgroundMenu(at);
  }
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

function openItemMenu(at: Anchor): void {
  const sel = selectedEntries();
  if (sel.length === 0) return;
  const single = sel.length === 1 ? sel[0] : null;
  const items =
    location === RECENT ? recentItemMenu(sel, single)
    : inTrash() ? trashItemMenu(sel, single)
    : location === STARRED ? starredItemMenu(sel, single)
    : location === NETWORK ? networkItemMenu(single)
    : isReadOnlyLocation(location) ? shareItemMenu(single)
    : sel.every((entry) => entry.cloud) ? cloudItemMenu(single)
    : folderItemMenu(sel, single);
  openMenu(at, items);
}

/** Read-only actions for a received share. Copy out arrives in Task 9. */
function shareItemMenu(single: Entry | null): MenuItem[] {
  return [
    { label: "Open", action: activateSelection },
    { label: "Copy", disabled: true },
    { label: "Copy Location", disabled: !single, action: copyLocation },
    ...(isSftpPath(location) ? [] : [propertiesMenuItem(single)]),
  ];
}

/** A Network item: open a received share or forget it; open a server or disconnect it. */
function networkItemMenu(single: Entry | null): MenuItem[] {
  const server = single ? serverFor(single) : undefined;
  if (server) {
    return [
      { label: "Open", action: activateSelection },
      { label: "Copy Location", action: copyLocation },
      { separator: true },
      { label: "Disconnect", action: () => void disconnectServer(server) },
    ];
  }
  const id = single && isSharePath(single.path) ? shareIdOf(single.path) : "";
  return [
    { label: "Open", action: activateSelection },
    { label: "Copy Location", disabled: !single, action: copyLocation },
    { separator: true },
    {
      label: "Remove from Network",
      disabled: !id,
      action: () => {
        removeReceived(id)
          .then(async () => {
            await refreshShares();
            await reload([]);
            showToast("Removed from Network");
          })
          .catch((err) => showToast(`Could not remove the share: ${cleanError(err)}`));
      },
    },
  ];
}

/** Drive items: no stars, versions or permanent delete (spec "Menus"). */
function cloudItemMenu(single: Entry | null): MenuItem[] {
  const web = single?.cloud?.web_url;
  return [
    { label: "Open", action: activateSelection },
    ...(web ? [{ label: "Open in Browser", action: () => openExternal(web) }] : []),
    openLocationItem(single),
    { separator: true },
    ...cutCopyItems(),
    { separator: true },
    { label: "Rename…", disabled: !single, action: () => single && startRename(single) },
    { separator: true },
    { label: "Move to Trash", action: trashSelection },
    { separator: true },
    ...(single?.is_dir ? [bookmarkMenuItem(single.path), { separator: true } as const] : []),
    propertiesMenuItem(single),
  ];
}

const cutCopyItems = (): MenuItem[] => [
  { label: "Cut", action: () => setClipboard("cut") },
  { label: "Copy", action: () => setClipboard("copy") },
];

function openLocationItem(single: Entry | null): MenuItem {
  return { label: "Open Item Location", disabled: !single, action: () => single && openItemLocation(single) };
}

/** Duplicate every local selected item beside itself. */
async function duplicateSelection(): Promise<void> {
  const sel = selectedEntries().filter((entry) => !isCloud(entry.path) && !isSharePath(entry.path));
  if (sel.length === 0) return;
  let last = "";
  let done = 0;
  for (const entry of sel) {
    const dir = parentOf(entry.path);
    const name = await freeCopyName(dir, entry.name, entry.is_dir);
    const dest = joinPath(dir, name);
    try {
      await copyEntry(entry.path, dest, false);
      last = dest;
      done += 1;
    } catch (err) {
      showToast(`Could not duplicate “${entry.name}”: ${cleanError(err)}`);
    }
  }
  if (done === 0) return;
  await reload(last ? [last] : undefined);
  showToast(`Duplicated ${done} ${done === 1 ? "item" : "items"}`);
}

async function createLinkFor(entry: Entry): Promise<void> {
  try {
    const dest = await createSymlink(entry.path);
    await reload([dest]);
    showToast("Link created");
  } catch (err) {
    showToast(`Could not create the link: ${cleanError(err)}`);
  }
}

/** "Notes", "Notes 2", … while keeping the extension. */
async function freeDocumentName(dir: string, name: string): Promise<string> {
  const dot = name.lastIndexOf(".");
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
  for (let n = 1; ; n++) {
    const candidate = `${stem}${n === 1 ? "" : ` ${n}`}${ext}`;
    if (!(await pathExists(joinPath(dir, candidate)))) return candidate;
  }
}

/** New Document: an empty text file or a copy of a file from ~/Templates. */
async function showNewDocumentDialog(): Promise<void> {
  if (isPseudo(location) || isReadOnlyLocation(location) || listingError !== null) return;
  const extra = document.createElement("div");
  extra.className = "app-list";
  const loading = document.createElement("p");
  loading.className = "app-loading";
  loading.textContent = "Loading templates…";
  const message = document.createElement("div");
  message.className = "field-message";
  extra.append(loading, message);

  const dialog = openDialog({
    heading: "New Document",
    body: "Choose a template from your Templates folder.",
    extra,
    className: "open-with-dialog",
    responses: [{ id: "cancel", label: "Cancel" }],
    defaultResponse: "cancel",
    closeResponse: "cancel",
  });

  const create = async (name: string, make: (dest: string) => Promise<void>) => {
    const dest = joinPath(location, name);
    try {
      await make(dest);
      dialog.close();
      await reload([dest]);
      showToast(`Created “${name}”`);
    } catch (err) {
      message.textContent = cleanError(err);
    }
  };
  const addChoice = (label: string, action: () => void) => {
    const button = document.createElement("button");
    button.className = "app-choice";
    const icon = document.createElement("span");
    icon.className = "app-choice-icon";
    icon.innerHTML = entryIcon(
      { name: label, path: "", is_dir: false, size: 0, modified: 0, is_hidden: false, is_symlink: false },
      "plain"
    );
    const text = document.createElement("span");
    text.className = "app-choice-name";
    text.textContent = label;
    button.append(icon, text);
    button.addEventListener("click", action);
    extra.insertBefore(button, message);
  };

  addChoice("Empty Text File", () =>
    void (async () => {
      const name = await freeDocumentName(location, "Untitled.txt");
      await create(name, (dest) => createFile(dest));
    })()
  );

  try {
    const templates = (await listTemplates()).filter((entry) => !entry.is_dir);
    loading.remove();
    for (const template of templates) {
      addChoice(template.name, () =>
        void (async () => {
          const name = await freeDocumentName(location, template.name);
          await create(name, async (dest) => {
            await copyEntry(template.path, dest, false);
          });
        })()
      );
    }
  } catch (err) {
    if (extra.isConnected) loading.textContent = `Could not read templates: ${cleanError(err)}`;
  }
}

/** Compress the selection into a new zip/tar.gz/tar.xz beside it. */
function showCompressDialog(sel: Entry[]): void {
  if (sel.length === 0 || sel.some((entry) => isCloud(entry.path) || isSharePath(entry.path))) return;
  const dirs = new Set(sel.map((entry) => parentOf(entry.path)));
  if (dirs.size !== 1) {
    showToast("Compress works inside one folder.");
    return;
  }
  const dir = [...dirs][0];
  const stem = sel.length === 1 ? archiveStem(sel[0].name) : "Archive";
  const extra = document.createElement("div");
  extra.className = "compress-fields";
  extra.innerHTML =
    `<label>Archive name<input class="entry" id="compress-name" type="text" spellcheck="false" /></label>` +
    `<label>Format<select class="entry" id="compress-format">` +
    `<option value="zip">ZIP (.zip)</option><option value="tar.gz">Tar GZip (.tar.gz)</option>` +
    `<option value="tar.xz">Tar XZ (.tar.xz)</option></select></label>` +
    `<div class="field-message"></div>`;
  const nameInput = extra.querySelector<HTMLInputElement>("#compress-name")!;
  const formatInput = extra.querySelector<HTMLSelectElement>("#compress-format")!;
  const message = extra.querySelector<HTMLElement>(".field-message")!;
  const extensions: Record<string, string> = { zip: ".zip", "tar.gz": ".tar.gz", "tar.xz": ".tar.xz" };
  let extension = ".zip";
  nameInput.value = `${stem}${extension}`;

  let dialog: ReturnType<typeof openDialog>;
  const validate = () => {
    const name = nameInput.value.trim();
    let error = "";
    if (!name || name === "." || name === ".." || name.includes("/") || name.includes("\0")) {
      error = "A name can't be empty or contain “/”.";
    } else if (entries.some((entry) => parentOf(entry.path) === dir && entry.name === name)) {
      error = `A file named “${name}” already exists.`;
    }
    message.textContent = error;
    dialog.setResponseEnabled("compress", !error);
    return error ? null : name;
  };
  nameInput.addEventListener("input", validate);
  formatInput.addEventListener("change", () => {
    const next = extensions[formatInput.value] ?? ".zip";
    if (nameInput.value.endsWith(extension)) {
      nameInput.value = nameInput.value.slice(0, -extension.length) + next;
    }
    extension = next;
    validate();
  });
  dialog = openDialog({
    heading: `Compress ${sel.length} ${sel.length === 1 ? "Item" : "Items"}`,
    extra,
    className: "compress-dialog",
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "compress", label: "Compress", style: "suggested" },
    ],
    defaultResponse: "compress",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "compress") return;
      const name = validate();
      if (!name) return false;
      const format = formatInput.value as "zip" | "tar.gz" | "tar.xz";
      void (async () => {
        try {
          await compressArchive(sel.map((entry) => entry.path), joinPath(dir, name), format);
          dialog.close();
          await reload([joinPath(dir, name)]);
          showToast(`Created “${name}”`);
        } catch (err) {
          message.textContent = cleanError(err);
        }
      })();
      return false;
    },
  });
  validate();
  const dot = nameInput.value.lastIndexOf(".");
  nameInput.setSelectionRange(0, dot > 0 ? dot : nameInput.value.length);
  nameInput.focus();
}

/** Preview and Open With for a single local file. */
function previewAndOpenWithItems(single: Entry | null): MenuItem[] {
  const file = single && !single.is_dir && !isCloud(single.path) && !isSharePath(single.path) && !isSftpPath(single.path) ? single : null;
  return [
    ...macDmgItems(single, isMac && !inTrash()),
    { label: "Preview", disabled: !file, action: () => file && void showPreview(file) },
    { label: "Open With…", disabled: !file, action: () => file && void showOpenWith(file) },
    ...linuxProgramItems(single, /Linux/.test(navigator.userAgent) && !inTrash()),
  ];
}

const ARCHIVE_SUFFIXES = [
  ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".tbz", ".txz", ".tzst",
  ".tar", ".zip", ".jar", ".gz", ".bz2", ".xz", ".zst",
];

function isExtractableArchive(name: string): boolean {
  const lower = name.toLowerCase();
  return ARCHIVE_SUFFIXES.some((suffix) => lower.endsWith(suffix));
}

function archiveStem(name: string): string {
  const lower = name.toLowerCase();
  const suffix = ARCHIVE_SUFFIXES.find((item) => lower.endsWith(item));
  return suffix ? name.slice(0, -suffix.length) : name;
}

/** A folder name that doesn't exist yet: "Photos", "Photos 2", … */
async function freeFolderName(dir: string, base: string): Promise<string> {
  for (let n = 1; ; n++) {
    const candidate = n === 1 ? base : `${base} ${n}`;
    if (!(await pathExists(joinPath(dir, candidate)))) return candidate;
  }
}

/** Extract an archive into its folder or a new folder beside it. */
async function extractArchiveEntry(entry: Entry, toNewFolder: boolean): Promise<void> {
  if (isCloud(entry.path) || isSharePath(entry.path)) return;
  const dir = parentOf(entry.path);
  const dest = toNewFolder ? joinPath(dir, await freeFolderName(dir, archiveStem(entry.name))) : dir;
  showToast(`Extracting “${entry.name}”…`);
  try {
    const result = await extractArchive(entry.path, dest, toNewFolder);
    await reload(toNewFolder ? [dest] : undefined);
    const skipped = result.skipped ? `, ${result.skipped} links skipped` : "";
    showToast(`Extracted ${result.files} ${result.files === 1 ? "file" : "files"}${skipped}`);
  } catch (err) {
    showToast(`Could not extract “${entry.name}”: ${cleanError(err)}`);
  }
}

function recentItemMenu(sel: Entry[], single: Entry | null): MenuItem[] {
  return [
    { label: "Open", action: activateSelection },
    ...previewAndOpenWithItems(single),
    openLocationItem(single),
    { separator: true },
    ...cutCopyItems(),
    { separator: true },
    { separator: true },
    starMenuItem(sel),
    { separator: true },
    { label: "Move to Trash", action: trashSelection },
    { label: "Remove from Recent", action: removeFromRecent },
    { separator: true },
    propertiesMenuItem(single),
  ];
}

function trashItemMenu(sel: Entry[], single: Entry | null): MenuItem[] {
  // Only items at the top of the Trash know where they came from.
  const top = location === trashPath;
  const restorable = top && sel.some((e) => e.origin);
  return [
    { label: "Open", action: activateSelection },
    ...previewAndOpenWithItems(single),
    ...(top ? [{ label: "Restore From Trash", disabled: !restorable, action: restoreSelection }] : []),
    { separator: true },
    ...cutCopyItems(),
    { separator: true },
    { label: "Delete Permanently…", action: () => confirmDeletePermanently(sel) },
    { separator: true },
    propertiesMenuItem(single),
  ];
}

function starredItemMenu(sel: Entry[], single: Entry | null): MenuItem[] {
  return [
    { label: "Open", action: activateSelection },
    ...previewAndOpenWithItems(single),
    openLocationItem(single),
    { separator: true },
    ...cutCopyItems(),
    { separator: true },
    starMenuItem(sel),
    { separator: true },
    { label: "Move to Trash", action: trashSelection },
    { separator: true },
    propertiesMenuItem(single),
  ];
}

function folderItemMenu(sel: Entry[], single: Entry | null): MenuItem[] {
  const intoFolder = single?.is_dir && !isAppBundle(single) ? single : null;
  const app = single && isAppBundle(single) ? single : null;
  const shareable = intoFolder && !isCloud(intoFolder.path) && !isSharePath(intoFolder.path) && !inTrash();
  const renameable = sel.length > 1 || !!single;
  return [
    { label: "Open", action: activateSelection },
    ...previewAndOpenWithItems(single),
    ...(single && !single.is_dir && !isCloud(single.path) && !isSharePath(single.path) && isExtractableArchive(single.name)
      ? [
          { label: "Extract Here", action: () => void extractArchiveEntry(single, false) },
          {
            label: `Extract to “${archiveStem(single.name)}”`,
            action: () => void extractArchiveEntry(single, true),
          },
        ]
      : []),
    ...(app ? [{ label: "Show Package Contents", action: () => navigate(app.path) }] : []),
    ...(intoFolder ? [bookmarkMenuItem(intoFolder.path)] : []),
    ...(shareable ? [{ label: "Share…", action: () => void showShareDialog(intoFolder) }] : []),
    ...qrShareItems(single, !inTrash()),
    { separator: true },
    ...cutCopyItems(),
    ...(intoFolder ? [{ label: "Paste Into Folder", disabled: !clipboard, action: () => paste(intoFolder.path) }] : []),
    { label: "Duplicate", action: () => void duplicateSelection() },
    { label: "Create Link", disabled: !single, action: () => single && void createLinkFor(single) },
    { label: "Compress…", action: () => showCompressDialog(sel) },
    { separator: true },
    { label: "Rename…", disabled: !renameable, action: () => (sel.length > 1 ? showBatchRename(sel) : single && startRename(single)) },
    starMenuItem(sel),
    { separator: true },
    { label: "Move to Trash", action: trashSelection },
    { separator: true },
    ...versionsMenuItem(single),
    propertiesMenuItem(single),
  ];
}

function openBackgroundMenu(at: Anchor): void {
  if (isPseudo(location)) {
    const items: MenuItem[] = [
      ...(location === NETWORK
        ? [{ label: "Connect to Server…", action: () => showConnectServerDialog() }, { separator: true as const }]
        : []),
      { label: "Select All", action: selectAll },
    ];
    openMenu(at, items);
    return;
  }
  if (location === trashPath) {
    openMenu(at, [
      { label: "Empty Trash…", disabled: entries.length === 0, action: confirmEmptyTrash },
      { separator: true },
      { label: "Select All", action: selectAll },
    ]);
    return;
  }
  if (isReadOnlyLocation(location)) {
    openMenu(at, [
      { label: "Show Hidden Files", check: prefs.showHidden, action: toggleHidden },
      { label: "Reload", action: () => void reload() },
      { separator: true },
      { label: "Select All", action: selectAll },
    ]);
    return;
  }
  if (isCloud(location)) {
    openMenu(at, [
      { label: "New Folder…", action: promptNewFolder },
      { separator: true },
      { label: "Paste", disabled: !clipboard, action: () => paste(location) },
      { label: "Reload", action: () => void reload() },
      { label: "Select All", action: selectAll },
    ]);
    return;
  }
  openMenu(at, [
    { label: "New Folder…", action: promptNewFolder },
    { label: "New Document…", action: () => void showNewDocumentDialog() },
    { label: "Share This Folder…", action: () => showShareDialogForPath(location) },
    { separator: true },
    { label: "Paste", disabled: !clipboard, action: () => paste(location) },
    ...(versionedRootPath
      ? [{ label: "Deleted Files…" as const, action: () => showDeletedFiles(location, () => void reload()) }]
      : []),
    { label: "Select All", action: selectAll },
  ]);
}

function openLocationMenu(anchor: HTMLElement): void {
  const real = !isPseudo(location);
  if (location === NETWORK) {
    openMenu(anchor, [
      { label: "Connect to Server…", action: () => showConnectServerDialog() },
      { separator: true },
      { label: "Reload", action: () => void reload() },
    ]);
    return;
  }
  if (location === trashPath) {
    openMenu(anchor, [
      { label: "Empty Trash…", disabled: entries.length === 0, action: confirmEmptyTrash },
      { separator: true },
      { label: "Select All", action: selectAll },
    ]);
    return;
  }
  if (isReadOnlyLocation(location)) {
    openMenu(anchor, [
      { label: "Copy Location", action: copyLocation },
      { label: "Reload", action: () => void reload() },
      { separator: true },
      { label: "Select All", disabled: !hasEntries(location), action: selectAll },
    ]);
    return;
  }
  if (isCloud(location)) {
    openMenu(anchor, [
      { label: "New Folder…", action: promptNewFolder },
      { separator: true },
      { label: "Copy Location", action: copyLocation },
      { label: "Paste", disabled: !clipboard, action: () => paste(location) },
      { label: "Select All", disabled: !hasEntries(location), action: selectAll },
    ]);
    return;
  }
  openMenu(anchor, [
    { label: "New Folder…", disabled: !real, action: promptNewFolder },
    ...(real ? [{ label: "New Document…" as const, action: () => void showNewDocumentDialog() }] : []),
    ...(real ? [bookmarkMenuItem(location), { label: "Share This Folder…" as const, action: () => showShareDialogForPath(location) }] : []),
    { separator: true },
    { label: "Copy Location", disabled: !real, action: copyLocation },
    { label: "Paste", disabled: !real || !clipboard, action: () => paste(location) },
    { label: "Select All", disabled: !hasEntries(location), action: selectAll },
  ]);
}

function openViewMenu(anchor: HTMLElement): void {
  // Recent has a fixed order: newest first.
  const fixed = location === RECENT;
  const is = (k: SortKey, rev: boolean) => !fixed && prefs.sortKey === k && prefs.sortReversed === rev;
  openMenu(anchor, [
    { heading: "Sort" },
    { label: "A–Z", radio: is("name", false), disabled: fixed, action: () => setSort("name", false) },
    { label: "Z–A", radio: is("name", true), disabled: fixed, action: () => setSort("name", true) },
    { label: "Last Modified", radio: is("modified", false), disabled: fixed, action: () => setSort("modified", false) },
    { label: "First Modified", radio: is("modified", true), disabled: fixed, action: () => setSort("modified", true) },
    { label: "Size", radio: is("size", false), disabled: fixed, action: () => setSort("size", false) },
    { label: "Type", radio: is("type", false), disabled: fixed, action: () => setSort("type", false) },
  ]);
}

function openMainMenu(anchor: HTMLElement): void {
  openMenu(anchor, [
    { label: "New Window", action: () => void openNewWindow() },
    { label: "New Tab", action: () => void newTab() },
    { label: "Close Tab", disabled: tabs.length <= 1, action: () => void closeTab(activeTab) },
    { separator: true },
    { label: "Connect to Server…", action: () => showConnectServerDialog() },
    { separator: true },
    { label: "Show Hidden Files", check: prefs.showHidden, action: toggleHidden },
    { label: "Show Thumbnails", check: prefs.showThumbnails, action: toggleThumbnails },
    { separator: true },
    { label: "Keyboard Shortcuts", action: showShortcuts },
    { label: "Check for Updates…", action: checkForUpdates },
    { label: "About File Browser", action: showAbout },
  ]);
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

function propertiesMenuItem(single: Entry | null): MenuItem {
  return { label: "Properties", disabled: !single, action: () => single && showProperties(single) };
}

/** "Versions…" for a regular file inside a folder that keeps versions. */
function versionsMenuItem(single: Entry | null): MenuItem[] {
  if (!single || single.is_dir || single.is_symlink) return [];
  if (!insideVersionedRoot(single.path, versionedRootPath)) return [];
  return [
    {
      label: "Versions…",
      action: () => showVersions(single, () => void reload()),
    },
  ];
}

/** "12.3 MB (12,345,678 bytes)", but plain "512 bytes" so the number isn't repeated. */
function sizeAndBytes(bytes: number): string {
  return bytes < 1000 ? formatSize(bytes) : `${formatSize(bytes)} (${bytes.toLocaleString()} bytes)`;
}

function propertyRow(label: string, value: string, title = ""): HTMLElement {
  const row = document.createElement("div");
  row.className = "prop-row";
  const key = document.createElement("span");
  key.className = "prop-label";
  key.textContent = label;
  const text = document.createElement("span");
  text.className = "prop-value";
  text.textContent = value;
  if (title) text.title = title;
  row.append(key, text);
  return row;
}

/** Pick the nine read/write/execute bits, then apply them to `path`. */
function showPermissionsDialog(path: string, mode: number, onSaved: () => void): void {
  const extra = document.createElement("div");
  const matrix = document.createElement("div");
  matrix.className = "perm-grid";
  const heads = ["", "Owner", "Group", "Others"];
  heads.forEach((head) => {
    const cell = document.createElement("span");
    cell.className = "perm-head";
    cell.textContent = head;
    matrix.appendChild(cell);
  });
  const triples = [
    ["Read", [0o400, 0o040, 0o004]],
    ["Write", [0o200, 0o020, 0o002]],
    ["Execute", [0o100, 0o010, 0o001]],
  ] as const;
  for (const [label, bits] of triples) {
    const cell = document.createElement("span");
    cell.className = "perm-label";
    cell.textContent = label;
    matrix.appendChild(cell);
    bits.forEach((bit, i) => {
      const wrapper = document.createElement("label");
      wrapper.className = "perm-check";
      wrapper.title = `${label} for ${heads[i + 1].toLowerCase()}`;
      const input = document.createElement("input");
      input.type = "checkbox";
      input.dataset.bit = String(bit);
      input.checked = (mode & bit) !== 0;
      input.setAttribute("aria-label", wrapper.title);
      wrapper.appendChild(input);
      matrix.appendChild(wrapper);
    });
  }
  const message = document.createElement("div");
  message.className = "field-message perm-message";
  extra.append(matrix, message);

  let dialog: ReturnType<typeof openDialog>;
  const apply = async () => {
    let next = 0;
    extra.querySelectorAll<HTMLInputElement>("input[data-bit]").forEach((input) => {
      if (input.checked) next |= Number(input.dataset.bit);
    });
    try {
      await setPermissions(path, next);
      dialog.close();
      showToast("Permissions changed");
      onSaved();
    } catch (err) {
      message.textContent = cleanError(err);
    }
  };

  dialog = openDialog({
    heading: "Permissions",
    body: "Choose what the owner, group and others can do.",
    extra,
    className: "permissions-dialog",
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "change", label: "Change", style: "suggested" },
    ],
    defaultResponse: "change",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "change") return;
      void apply();
      return false;
    },
  });
}

/**
 * The Properties dialog: type, size, dates and permissions. A folder's total
 * size is summed in the background (like GNOME Files) while the rest of the
 * dialog is already usable.
 */
async function showProperties(entry: Entry): Promise<void> {
  let info;
  try {
    info = await fileProperties(entry.path, prefs.showHidden);
  } catch (err) {
    showToast(`Could not show properties: ${cleanError(err)}`);
    return;
  }

  const extra = document.createElement("div");
  extra.className = "properties";

  const header = document.createElement("div");
  header.className = "properties-header";
  const icon = document.createElement("div");
  icon.className = "properties-icon";
  icon.innerHTML = entryIcon(entry, folderKind(entry));
  useNativeIcon(icon, entry.path, 64);
  const name = document.createElement("div");
  name.className = "properties-name";
  name.textContent = info.name;
  const type = document.createElement("div");
  type.className = "properties-type";
  type.textContent = fileTypeDescription(entry);
  header.append(icon, name, type);

  const list = document.createElement("div");
  list.className = "props-list";
  const add = (label: string, value: string, title = "") => {
    if (value) list.appendChild(propertyRow(label, value, title));
  };

  let sizeValue: HTMLElement | null = null;
  let historyBytes = 0;
  let closePropertiesDialog: (() => void) | null = null;
  if (info.is_dir) {
    if (isCloud(info.path)) {
      // Drive does not report recursive folder sizes cheaply.
      add("Size", "—");
    } else {
      const row = propertyRow("Size", "Calculating…");
      sizeValue = row.querySelector<HTMLElement>(".prop-value");
      list.appendChild(row);
    }
    if (info.item_count !== null) add("Contents", formatItemCount(info.item_count));
  } else {
    add("Size", sizeAndBytes(info.size));
  }
  add("Parent folder", info.parent, info.parent);
  add("Modified", formatDateDetailed(info.modified));
  if (!isCloud(info.path)) {
    add("Accessed", formatDateDetailed(info.accessed));
    add("Created", formatDateDetailed(info.created));
    if (info.symlink_target) add("Link target", info.symlink_target, info.symlink_target);
    add("Permissions", `${info.mode} (${info.octal})`);
    add("Group", info.group);
    add("Others", info.others);
  }
  add("Owner", info.owner);
  extra.append(header, list);
  try {
    const root = await versionedRoot(entry.path).catch(() => null);
    historyBytes =
      (await addVersionRows(list, entry, root, () => void reload(), () => closePropertiesDialog?.()))
        ?.bytes ?? 0;
  } catch {
    /* Properties still works without the history rows */
  }

  // Closing the dialog stops the size walk, which can take minutes on Home.
  const walk = sizeValue ? await newFolderWalk().catch(() => 0) : 0;
  const canChangePermissions = !isCloud(info.path) && !isSharePath(info.path);
  const propertiesDialog = openDialog({
    heading: "Properties",
    className: "properties-dialog",
    extra,
    responses: [
      ...(canChangePermissions ? [{ id: "permissions", label: "Permissions…" }] : []),
      { id: "close", label: "Close" },
    ],
    defaultResponse: "close",
    closeResponse: "close",
    onResponse: (id) => {
      if (id === "permissions") {
        showPermissionsDialog(info.path, parseInt(info.octal, 8) & 0o777, () => {
          if (walk) cancelFolderSize(walk).catch(() => {});
          propertiesDialog.close();
          void showProperties(entry);
        });
        return false;
      }
      if (walk) cancelFolderSize(walk).catch(() => {});
    },
  });
  closePropertiesDialog = () => propertiesDialog.close();

  if (sizeValue) {
    try {
      const bytes = await folderSize(info.path, walk);
      if (sizeValue.isConnected) sizeValue.textContent = sizeAndBytes(bytes + historyBytes);
    } catch {
      if (sizeValue.isConnected) sizeValue.textContent = "Unknown";
    }
  }
}

// ---------------------------------------------------------------------------
// Starred
// ---------------------------------------------------------------------------

function starMenuItem(sel: Entry[]): MenuItem {
  const all = sel.every((e) => starred.has(e.path));
  return { label: all ? "Unstar" : "Star", action: () => setStarred(sel.map((e) => e.path), !all) };
}

async function setStarred(paths: string[], on: boolean): Promise<void> {
  for (const p of paths) {
    if (on) starred.add(p);
    else starred.delete(p);
  }
  await persistStarred();
  if (location === STARRED) await reload();
  else renderWindow(true);
}

async function persistStarred(): Promise<void> {
  try {
    await saveStarred([...starred]);
  } catch (err) {
    showToast(`Could not save starred files: ${cleanError(err)}`);
  }
}

/** Keep stars on items renamed or moved from `from` to `to`, and on anything inside them. */
function followMove(from: string, to: string): void {
  let changed = false;
  const next = new Set<string>();
  for (const p of starred) {
    if (p === from || p.startsWith(from + "/")) {
      next.add(to + p.slice(from.length));
      changed = true;
    } else {
      next.add(p);
    }
  }
  if (!changed) return;
  starred = next;
  persistStarred();
}

/** Show the folder holding `entry`, with it selected. */
function openItemLocation(entry: Entry): void {
  navigate(parentOf(entry.path), { select: entry.path });
}

async function removeFromRecent(): Promise<void> {
  const paths = selectedEntries().map((e) => e.path);
  if (paths.length === 0) return;
  try {
    await forgetRecent(paths);
  } catch (err) {
    showToast(`Could not remove from Recent: ${cleanError(err)}`);
  }
  await reload([]);
}

async function copyLocation(): Promise<void> {
  try {
    await navigator.clipboard.writeText(location);
  } catch {
    showToast("Could not copy the location");
  }
}

// ---------------------------------------------------------------------------
// Clipboard: copy / cut / paste
// ---------------------------------------------------------------------------

function setClipboard(mode: "copy" | "cut"): void {
  const sel = selectedEntries();
  if (sel.length === 0) return;
  if (sel.some((entry) => isSharePath(entry.path) || isSftpPath(entry.path))) {
    showToast(isSftpPath(sel[0].path) ? "Copying from SSH servers comes next." : "Copying out of a shared folder comes next.");
    return;
  }
  clipboard = { paths: sel.map((e) => e.path), dirs: new Set(sel.filter((e) => e.is_dir).map((e) => e.path)), mode };
  // Paths also go to the system clipboard as text, handy for terminals.
  navigator.clipboard?.writeText(clipboard.paths.join("\n")).catch(() => {});
  refreshCutMarks();
}

function cancelCut(): boolean {
  if (clipboard?.mode !== "cut") return false;
  clipboard = null;
  refreshCutMarks();
  return true;
}

function refreshCutMarks(): void {
  const cut = clipboard?.mode === "cut" ? new Set(clipboard.paths) : null;
  viewEl.querySelectorAll<HTMLElement>(".item").forEach((el) => {
    el.classList.toggle("cut", !!cut?.has(shown[Number(el.dataset.index)]?.path));
  });
}

type ConflictChoice = "replace" | "skip" | "keepboth" | "cancel";

/** GNOME-style conflict dialog. Resolves with the choice and whether to apply it to the rest. */
function askConflict(name: string, isDir: boolean, destDir: string, remaining: number): Promise<{ choice: ConflictChoice; all: boolean }> {
  return new Promise((resolve) => {
    const kind = isDir ? "folder" : "file";
    let all = false;
    let extra: HTMLElement | undefined;
    if (remaining > 0) {
      extra = document.createElement("label");
      extra.className = "check-row";
      extra.innerHTML = `<input type="checkbox" /><span>Apply this action to all conflicts (${remaining + 1})</span>`;
      extra.querySelector("input")!.addEventListener("change", (e) => (all = (e.target as HTMLInputElement).checked));
    }
    openDialog({
      heading: `Replace ${kind} “${name}”?`,
      body: `Another ${kind} with the same name already exists in “${locationName(destDir)}”. Replacing it moves the existing ${kind} to the Trash.`,
      extra,
      responses: [
        { id: "skip", label: "Skip" },
        { id: "keepboth", label: "Keep Both" },
        { id: "replace", label: "Replace", style: "destructive" },
      ],
      // Enter keeps both: the safe choice, nothing is replaced by accident.
      defaultResponse: "keepboth",
      closeResponse: "cancel",
      onResponse: (id) => resolve({ choice: id as ConflictChoice, all }),
    });
  });
}

/** "name (copy).txt", then "name (copy 2).txt", … — the first one that's free. */
async function freeCopyName(dir: string, name: string, isDir: boolean): Promise<string> {
  const dot = isDir ? -1 : name.lastIndexOf(".");
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
  for (let n = 1; ; n++) {
    const candidate = `${stem} (copy${n > 1 ? ` ${n}` : ""})${ext}`;
    if (!(await pathExists(joinPath(dir, candidate)))) return candidate;
  }
}

/** Transfers run one at a time; a new one waits behind this (spec D8). */
let transferChain: Promise<unknown> = Promise.resolve();
let transfersRunning = 0;

/** Wording and totals for a transfer batch's operation row. */
function transferOperation(paths: string[], moving: boolean, destDir: string, cancel: () => void): Operation {
  const title = opTitle(moving ? "move" : "copy", paths.map(basename), basename(destDir));
  return startOperation(title, { totalItems: paths.length, onCancel: cancel, waiting: transfersRunning > 0 });
}

/**
 * Copy or move `paths` into `destDir`, asking about name clashes. Shared by
 * paste and drag and drop; `move(path)` says whether that item is moved.
 * Batches queue behind each other and show in the operations popover.
 * Resolves true when every item was handled (none failed, nothing cancelled).
 */
async function runTransfer(paths: string[], dirs: Set<string>, move: (path: string) => boolean, destDir: string): Promise<boolean> {
  if (isPseudo(destDir)) return false;
  // The user's selection as of this paste; see transferBatch's end.
  const intent = selectionIntent;
  const batch = { cancelled: false, started: false, measuring: false };
  const op = transferOperation(paths, paths.some(move), destDir, () => {
    batch.cancelled = true;
    // Waiting in the queue or still measuring: nothing to stop yet, so the
    // row goes at once and the batch ends before its first item.
    if (!batch.started || batch.measuring) endOperation(op);
    else void cancelOperation(op.id).catch(() => {});
  });
  const run = transferChain.then(async () => {
    if (batch.cancelled) return false;
    batch.started = true;
    transfersRunning++;
    try {
      return await transferBatch(paths, dirs, move, destDir, op, batch, intent);
    } finally {
      transfersRunning--;
      endOperation(op);
    }
  });
  transferChain = run.catch(() => {});
  return run;
}

/** Measure the batch for its progress total; items only when that fails. */
async function measureBatch(op: Operation, paths: string[], batch: { measuring: boolean }): Promise<void> {
  setOperationState(op, "preparing");
  batch.measuring = true;
  try {
    const measured = await measure(paths);
    op.totalBytes = measured.bytes || null;
    op.totalItems = measured.items || paths.length;
  } catch {
    op.totalBytes = null;
  } finally {
    batch.measuring = false;
  }
  setOperationState(op, "working");
}

/** How long a finished item waits for its last progress update. */
const LAST_UPDATE_MS = 1000;

/**
 * One item of a batch, reporting into `op`; resolves with the new path. The
 * last update, which holds every byte of the item, can arrive after the
 * command returned, so a finished item waits briefly for it.
 */
async function transferItem(op: Operation, from: string, to: string, overwrite: boolean, moving: boolean): Promise<string> {
  let finished = false;
  let sawLast = () => {};
  const last = new Promise<void>((resolve) => (sawLast = resolve));
  const onProgress = (u: ProgressUpdate) => {
    if (finished) return;
    updateOperation(op, u);
    // A folder fills in place as it copies; let the open view follow.
    showLanded(parentOf(to));
    if (u.done) sawLast();
  };
  const tracked = { op: op.id, onProgress };
  try {
    const result = moving ? await moveEntry(from, to, overwrite, tracked) : await copyEntry(from, to, overwrite, tracked);
    await Promise.race([last, new Promise((resolve) => setTimeout(resolve, LAST_UPDATE_MS))]);
    if (moving) followMove(from, result);
    return result;
  } finally {
    finished = true;
    finishItem(op);
  }
}

/** Folders that received items since the last reload, and its timer. */
const landedIn = new Set<string>();
let landedReload: ReturnType<typeof setTimeout> | undefined;
let lastLandedReload = 0;

/**
 * Show what has landed in `destDir` while a transfer runs, at most once a
 * second: the open folder reloads when it is `destDir` or inside it (a
 * folder being filled).
 */
function showLanded(destDir: string): void {
  landedIn.add(destDir);
  if (landedReload !== undefined) return;
  const wait = Math.max(0, lastLandedReload + 1000 - Date.now());
  landedReload = setTimeout(() => {
    landedReload = undefined;
    lastLandedReload = Date.now();
    const open = [...landedIn].some((dir) => location === dir || location.startsWith(`${dir.replace(/\/+$/, "")}/`));
    landedIn.clear();
    if (open) void reload();
  }, wait);
}

async function transferBatch(
  paths: string[], dirs: Set<string>, move: (path: string) => boolean, destDir: string,
  op: Operation, batch: { cancelled: boolean; measuring: boolean }, intent: number,
): Promise<boolean> {
  await measureBatch(op, paths, batch);
  const done: string[] = [];
  const errors: string[] = [];
  let anyMove = false;
  let applyToAll: ConflictChoice | null = null;

  for (let i = 0; i < paths.length && !batch.cancelled; i++) {
    const from = paths[i];
    // A Drive name can be "a/b" or "..": locally it must stay one plain name.
    const name = isCloud(from) && !isCloud(destDir) ? safeLocalName(basename(from)) : basename(from);
    const isDir = dirs.has(from);
    const moving = move(from);
    anyMove ||= moving;
    let to = joinPath(destDir, name);
    let overwrite = false;

    // Same folder: in Drive `from` is an id path and `to` a pending name,
    // so compare folders, not paths.
    if (parentOf(from) === destDir.replace(/\/+$/, "") && basename(from) === name) {
      // A copy into its own folder makes a duplicate, as in GNOME Files;
      // moving into the same folder does nothing.
      if (moving) continue;
      to = joinPath(destDir, await freeCopyName(destDir, name, isDir));
    } else if (await pathExists(to)) {
      let choice: ConflictChoice | null = applyToAll;
      if (!choice) {
        const answer = await askConflict(name, isDir, destDir, paths.length - i - 1);
        choice = answer.choice;
        if (answer.all) applyToAll = choice;
      }
      if (choice === "cancel") {
        batch.cancelled = true;
        break;
      }
      if (choice === "skip") continue;
      if (choice === "keepboth") to = joinPath(destDir, await freeCopyName(destDir, name, isDir));
      else overwrite = true;
    }

    try {
      done.push(await transferItem(op, from, to, overwrite, moving));
      showLanded(destDir);
    } catch (err) {
      // A cancel is not a failure: no toast, finished items stay.
      if (cleanError(err) === CANCELLED) batch.cancelled = true;
      else errors.push(`“${name}”: ${cleanError(err)}`);
    }
  }

  const verb = anyMove ? "move" : "copy";
  if (errors.length) {
    showToast(errors.length === 1 ? `Could not ${verb} ${errors[0]}` : `Could not ${verb} ${errors.length} items. ${errors[0]}`);
  }
  // Select what landed only when the user hasn't chosen something else, or
  // gone anywhere, since pasting; otherwise keep their newer selection.
  const untouched = destDir === location && intent === selectionIntent;
  await reload(untouched && done.length ? done : undefined);
  return !batch.cancelled && errors.length === 0;
}

async function paste(destDir: string): Promise<void> {
  if (isReadOnlyLocation(destDir)) {
    showToast("Shared folders are read only.");
    return;
  }
  if (!clipboard) return;
  const { paths, dirs, mode } = clipboard;
  const complete = await runTransfer(paths, dirs, () => mode === "cut", destDir);
  // A cut is only consumed once every item has been dealt with.
  if (mode === "cut" && complete && clipboard?.paths === paths) {
    clipboard = null;
    refreshCutMarks();
  }
}

function inTrash(): boolean {
  return !!trashPath && (location === trashPath || location.startsWith(trashPath + "/"));
}

async function trashSelection(): Promise<void> {
  if (inTrash() || isReadOnlyLocation(location)) return;
  await trashPaths(selectedEntries().map((e) => e.path));
}

/** Delete / ⌘⌫: to the Trash, or out of it for good when already there. */
function trashOrDelete(): void {
  if (inTrash()) confirmDeletePermanently(selectedEntries());
  else trashSelection();
}

/** The last Move to Trash, for Undo: where each item went and came from. */
let lastTrashed: { trashed: string; origin: string }[] = [];

/** An operation counted in items, cancelled between them by the caller. */
function itemOperation(kind: "trash" | "delete", names: string[]): { op: Operation; cancelled: boolean } {
  const batch = { cancelled: false } as { op: Operation; cancelled: boolean };
  batch.op = startOperation(opTitle(kind, names), {
    totalItems: names.length,
    onCancel: () => {
      batch.cancelled = true;
    },
  });
  setOperationState(batch.op, "working");
  return batch;
}

async function trashPaths(paths: string[]): Promise<void> {
  if (paths.length === 0) return;
  const errors: string[] = [];
  const undoable: { trashed: string; origin: string }[] = [];
  const done: string[] = [];
  const op = startOperation(opTitle("trash", paths.map(basename)), {
    totalItems: paths.length,
    onCancel: () => void cancelOperation(op.id).catch(() => {}),
  });
  setOperationState(op, "working");
  let outcomes: TrashOutcome[] = [];
  try {
    // One batch, so one Finder request and one sound on macOS.
    outcomes = await trashEntries(paths, { op: op.id, onProgress: (u) => updateOperation(op, u) });
  } catch (err) {
    const what = paths.length === 1 ? `“${basename(paths[0])}”` : `${paths.length} items`;
    if (cleanError(err) !== CANCELLED) errors.push(`${what}: ${cleanError(err)}`);
  }
  outcomes.forEach(({ trashed, error }, i) => {
    if (error === null) {
      done.push(paths[i]);
      if (trashed) undoable.push({ trashed, origin: paths[i] });
    } else if (error !== CANCELLED) {
      errors.push(`“${basename(paths[i])}”: ${cleanError(error)}`);
    }
  });
  endOperation(op);
  if (clipboard) {
    const gone = new Set(done);
    clipboard.paths = clipboard.paths.filter((p) => !gone.has(p));
    if (clipboard.paths.length === 0) clipboard = null;
  }
  lastTrashed = undoable;
  if (errors.length) {
    showToast(`Could not move ${errors[0]} to the Trash`);
  } else if (done.length) {
    const message = done.length === 1 ? `“${basename(done[0])}” moved to trash` : `${done.length} items moved to trash`;
    showToast(message, undoable.length ? { button: { label: "Undo", onClick: undoTrash } } : {});
  }
  await reload([]);
}

/** Put the items from the last Move to Trash back. */
async function undoTrash(): Promise<void> {
  const items = lastTrashed;
  lastTrashed = [];
  if (items.length === 0) return;
  const restored: string[] = [];
  const errors: string[] = [];
  for (const { trashed, origin } of items) {
    try {
      restored.push(await restoreEntry(trashed));
    } catch (err) {
      errors.push(`“${basename(origin)}”: ${cleanError(err)}`);
    }
  }
  if (errors.length) showToast(`Could not restore ${errors[0]}`);
  await reload(restored);
}

async function restoreSelection(): Promise<void> {
  const errors: string[] = [];
  for (const e of selectedEntries()) {
    if (!e.origin) continue;
    try {
      await restoreEntry(e.path);
    } catch (err) {
      errors.push(`“${e.name}”: ${cleanError(err)}`);
    }
  }
  if (errors.length) showToast(errors.length === 1 ? `Could not restore ${errors[0]}` : `Could not restore ${errors.length} items. ${errors[0]}`);
  await reload([]);
}

function confirmDeletePermanently(sel: Entry[]): void {
  if (sel.length === 0 || !inTrash()) return;
  openDialog({
    heading: sel.length === 1
      ? `Are you sure you want to permanently delete “${sel[0].name}”?`
      : `Are you sure you want to permanently delete the ${sel.length} selected items?`,
    body: "If you delete an item, it will be permanently lost.",
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "delete", label: "Delete", style: "destructive" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id === "delete") deleteEntries(sel);
    },
  });
}

async function deleteEntries(sel: Entry[]): Promise<void> {
  const errors: string[] = [];
  const batch = itemOperation("delete", sel.map((e) => e.name));
  for (const e of sel) {
    if (batch.cancelled) break;
    try {
      await deletePermanently(e.path);
    } catch (err) {
      errors.push(`“${e.name}”: ${cleanError(err)}`);
    }
    finishItem(batch.op);
  }
  endOperation(batch.op);
  if (errors.length) showToast(`Could not delete ${errors[0]}`);
  await reload([]);
}

function confirmEmptyTrash(): void {
  openDialog({
    heading: "Empty all items from Trash?",
    body: "All items in the Trash will be permanently deleted.",
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "empty", label: "Empty Trash", style: "destructive" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id === "empty") runEmptyTrash();
    },
  });
}

async function runEmptyTrash(): Promise<void> {
  const count = trashPath ? (await listDir(trashPath, true).catch(() => [])).length : 0;
  const op = startOperation(opTitle("empty", []), {
    totalItems: count,
    onCancel: () => void cancelOperation(op.id).catch(() => {}),
  });
  setOperationState(op, "working");
  try {
    await emptyTrash({ op: op.id, onProgress: (u) => updateOperation(op, u) });
  } catch (err) {
    if (cleanError(err) !== CANCELLED) showToast(`Could not empty the Trash: ${cleanError(err)}`);
  }
  endOperation(op);
  lastTrashed = [];
  if (inTrash()) {
    if (location === trashPath) await reload([]);
    else await navigate(trashPath);
  }
}

// ---------------------------------------------------------------------------
// Drag and drop
//
// Dragging inside the window is done with mouse events rather than HTML5
// drag and drop, which WKWebView handles unreliably. Files dragged in from
// Finder arrive through Tauri's native drag-drop events instead.
// ---------------------------------------------------------------------------

type DropOp = "move" | "copy" | "trash";

interface DropTarget {
  dir: string;
  el: HTMLElement | null;
}

/** Where a drop at client point (x, y) lands, ignoring the dragged `sources`. */
function dropTargetAt(x: number, y: number, sources: string[]): DropTarget | null {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  if (!el) return null;
  const item = el.closest<HTMLElement>(".item");
  if (item && viewEl.contains(item)) {
    const e = shown[Number(item.dataset.index)];
    if (e?.is_dir && !isAppBundle(e) && !sources.includes(e.path)) return { dir: e.path, el: item };
  }
  const row = el.closest<HTMLElement>(".sidebar-row");
  if (row) {
    // A disconnected Google Drive row has no location.
    const loc = row.dataset.location;
    return !loc || isPseudo(loc) ? null : { dir: loc, el: row };
  }
  const crumb = el.closest<HTMLElement>(".crumb");
  if (crumb) return crumb.dataset.path && !isPseudo(crumb.dataset.path) ? { dir: crumb.dataset.path, el: crumb } : null;
  // Anywhere else in the view means the folder being shown.
  if (el.closest(".view-overlay") && !isPseudo(location) && listingError === null) {
    return { dir: location, el: $(".view-overlay") };
  }
  return null;
}

/** The volume a path is on, for a quick guess before device ids arrive. */
function volumeRoot(path: string): string {
  return mountedVolumes
    .filter((v) => path === v.path || path.startsWith(v.path + "/"))
    .sort((a, b) => b.path.length - a.path.length)[0]?.path ?? "/";
}

/** The operation a drop would do, or null if it would do nothing useful. */
function dropOp(target: DropTarget, sources: string[], mods: { altKey: boolean; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean }, devices?: Map<string, number>): DropOp | null {
  const dir = target.dir;
  if (isReadOnlyLocation(dir) || sources.some(isSharePath) || sources.some(isSftpPath)) return null;
  // Into itself or its own subfolder: refused.
  if (sources.some((src) => dir === src || dir.startsWith(src + "/"))) return null;
  // Drive items go to Drive's Trash with Move to Trash, not the app's Trash.
  if (dir === trashPath) return inTrash() || sources.some(isCloud) ? null : "trash";
  const forceCopy = isMac ? mods.altKey : mods.ctrlKey;
  const forceMove = isMac ? mods.metaKey : mods.shiftKey;
  // Moving items to the folder they are already in does nothing.
  const allHere = sources.every((src) => parentOf(src) === dir);
  if (forceCopy) return "copy";
  if (allHere) return null;
  if (forceMove) return "move";
  // Local ↔ Drive is another storage: copy, before any device guess.
  const storage = (path: string) => (isCloud(path) ? cloudAccountKey(path) : "");
  if (sources.some((src) => storage(src) !== storage(dir))) return "copy";
  // Same filesystem moves, another one copies (GNOME and Finder agree here).
  const same = (src: string) => {
    const a = devices?.get(src), b = devices?.get(dir);
    return a !== undefined && b !== undefined ? a === b : volumeRoot(src) === volumeRoot(dir);
  };
  return sources.every(same) ? "move" : "copy";
}

async function deviceMap(paths: string[]): Promise<Map<string, number>> {
  const map = new Map<string, number>();
  try {
    const info = await pathInfo(paths);
    paths.forEach((p, i) => info[i] && map.set(p, info[i]!.device));
  } catch {
    /* fall back to the volume guess */
  }
  return map;
}

/** Run a drop exactly as its cursor showed: trash, copy or move. */
async function performDrop(sources: string[], dirs: Set<string>, target: DropTarget, op: DropOp): Promise<void> {
  if (op === "trash") return trashPaths(sources);
  await runTransfer(sources, dirs, () => op === "move", target.dir);
}

let dropHighlight: HTMLElement | null = null;

function setDropHighlight(el: HTMLElement | null): void {
  if (dropHighlight === el) return;
  dropHighlight?.classList.remove("drop-target");
  dropHighlight = el;
  el?.classList.add("drop-target");
}

interface DragState {
  paths: string[];
  dirs: Set<string>;
  ghost: HTMLElement;
  target: DropTarget | null;
  op: DropOp | null;
  x: number;
  y: number;
  mods: { altKey: boolean; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean };
  devices: Map<string, number>;
  springTimer: number;
  scrollTimer: number;
  nativeStarted: boolean;
}

let drag: DragState | null = null;
/** A reload that arrived while a drag was in progress. */
let reloadAfterDrag = false;
/** Set while a press on a selected item may still narrow the selection on release. */
let pendingNarrow: (() => void) | null = null;

/** Forget a pending narrow whose release never reached the view (a menu took it). */
function cancelPendingNarrow(): void {
  if (!pendingNarrow) return;
  viewEl.removeEventListener("mouseup", pendingNarrow);
  pendingNarrow = null;
}

/** After a press on an item, start dragging once the pointer moves a few pixels. */
function armDrag(start: MouseEvent): void {
  const onMove = (e: MouseEvent) => {
    if (Math.hypot(e.clientX - start.clientX, e.clientY - start.clientY) < 5) return;
    cleanup();
    beginDrag(e);
  };
  const cleanup = () => {
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", cleanup);
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", cleanup);
}

function beginDrag(e: MouseEvent): void {
  const sel = selectedEntries();
  if (sel.length === 0) return;
  if (sel.some((entry) => isSharePath(entry.path) || isSftpPath(entry.path))) {
    showToast(isSftpPath(sel[0].path) ? "Copying from SSH servers comes next." : "Copying out of a shared folder comes next.");
    return;
  }
  cancelPendingNarrow();
  const ghost = document.createElement("div");
  ghost.className = "drag-ghost";
  ghost.innerHTML = `<div class="drag-icon">${iconHtml(sel[0])}</div>` +
    (sel.length > 1 ? `<span class="drag-count">${sel.length}</span>` : "") +
    `<span class="drag-op" hidden>+</span>`;
  document.body.appendChild(ghost);
  document.documentElement.classList.add("dragging");

  const paths = sel.map((x) => x.path);
  drag = {
    paths,
    dirs: new Set(sel.filter((x) => x.is_dir).map((x) => x.path)),
    ghost,
    target: null,
    op: null,
    x: e.clientX,
    y: e.clientY,
    mods: { altKey: e.altKey, metaKey: e.metaKey, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey },
    devices: new Map(),
    springTimer: 0,
    scrollTimer: window.setInterval(autoScrollDuringDrag, 50),
    nativeStarted: false,
  };
  deviceMap(paths).then((m) => {
    if (drag?.paths === paths) for (const [k, v] of m) drag.devices.set(k, v);
  });

  window.addEventListener("mousemove", onDragMove);
  window.addEventListener("mouseup", onDragEnd);
  window.addEventListener("keydown", onDragKey, true);
  window.addEventListener("keyup", onDragKey, true);
  document.documentElement.addEventListener("mouseleave", onDragLeaveWindow);
  updateDrag();
}

/** A 1×1 transparent PNG, the drag image for native drag-out. */
const DRAG_IMAGE =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z1ZkAAAAASUVORK5CYII=";

/** Start a native file drag when the pointer leaves the window. */
function onDragLeaveWindow(): void {
  if (!drag || drag.nativeStarted) return;
  const paths = drag.paths.filter((path) => !isCloud(path) && !isSharePath(path));
  if (paths.length === 0) return;
  drag.nativeStarted = true;
  endDrag(false);
  startNativeDrag(paths);
}

function startNativeDrag(paths: string[]): void {
  const api = (window as unknown as {
    __TAURI__?: { drag?: { startDrag?: (options: unknown) => Promise<void> } };
  }).__TAURI__?.drag;
  if (!api?.startDrag) return;
  api
    .startDrag({ item: paths, icon: DRAG_IMAGE, mode: "copy" })
    .catch((err) => showToast(`Could not drag out: ${cleanError(err)}`));
}

function onDragMove(e: MouseEvent): void {
  if (!drag) return;
  drag.x = e.clientX;
  drag.y = e.clientY;
  drag.mods = { altKey: e.altKey, metaKey: e.metaKey, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey };
  updateDrag();
}

function onDragKey(e: KeyboardEvent): void {
  if (!drag) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.type === "keydown" && e.key === "Escape") return endDrag(false);
  drag.mods = { altKey: e.altKey, metaKey: e.metaKey, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey };
  updateDrag();
}

/** Recompute target, operation, ghost and cursor for the current pointer. */
function updateDrag(): void {
  if (!drag) return;
  const d = drag;
  d.ghost.style.left = `${d.x + 4}px`;
  d.ghost.style.top = `${d.y + 4}px`;
  const target = dropTargetAt(d.x, d.y, d.paths);
  // Fetch the target's device id once, for an exact move/copy decision.
  if (target && !d.devices.has(target.dir)) {
    d.devices.set(target.dir, -1);
    deviceMap([target.dir]).then((m) => {
      if (drag === d && m.has(target.dir)) {
        d.devices.set(target.dir, m.get(target.dir)!);
        updateDrag();
      }
    });
  }
  const known = new Map([...d.devices].filter(([, v]) => v !== -1));
  const op = target ? dropOp(target, d.paths, d.mods, known) : null;

  // Hovering a folder for a moment opens it, so the drop can go deeper.
  if (target?.dir !== d.target?.dir) {
    clearTimeout(d.springTimer);
    if (target && op && op !== "trash" && target.dir !== location && target.el !== $(".view-overlay")) {
      d.springTimer = window.setTimeout(() => {
        if (drag === d && d.target?.dir === target.dir) navigate(target.dir).then(() => updateDrag());
      }, 1000);
    }
  }
  d.target = target;
  d.op = op;
  setDropHighlight(op ? target!.el : null);
  d.ghost.querySelector<HTMLElement>(".drag-op")!.hidden = op !== "copy";
  document.documentElement.style.cursor = op === "copy" ? "copy" : op ? "default" : "no-drop";
}

function autoScrollDuringDrag(): void {
  if (!drag) return;
  const r = scrollEl.getBoundingClientRect();
  if (drag.x < r.left || drag.x > r.right) return;
  if (drag.y > r.bottom - 24 && drag.y < r.bottom + 24) scrollEl.scrollTop += 16;
  else if (drag.y < r.top + 24 && drag.y > r.top - 24) scrollEl.scrollTop -= 16;
  else return;
  updateDrag();
}

function onDragEnd(): void {
  endDrag(true);
}

function endDrag(drop: boolean): void {
  const d = drag;
  if (!d) return;
  drag = null;
  clearTimeout(d.springTimer);
  clearInterval(d.scrollTimer);
  window.removeEventListener("mousemove", onDragMove);
  window.removeEventListener("mouseup", onDragEnd);
  window.removeEventListener("keydown", onDragKey, true);
  window.removeEventListener("keyup", onDragKey, true);
  document.documentElement.removeEventListener("mouseleave", onDragLeaveWindow);
  d.ghost.remove();
  document.documentElement.classList.remove("dragging");
  document.documentElement.style.cursor = "";
  setDropHighlight(null);
  if (drop && d.target && d.op) performDrop(d.paths, d.dirs, d.target, d.op);
  if (reloadAfterDrag) {
    reloadAfterDrag = false;
    void reload();
  }
}

/** Files dragged in from Finder (or any other app) arrive as paths. */
function initExternalDrops(): void {
  let sources: string[] = [];
  getCurrentWebview()
    .onDragDropEvent(async (event) => {
      const p = event.payload;
      if (p.type === "leave") {
        sources = [];
        setDropHighlight(null);
        return;
      }
      const x = p.position.x / devicePixelRatio;
      const y = p.position.y / devicePixelRatio;
      if (p.type === "enter") sources = p.paths;
      const target = dropTargetAt(x, y, sources);
      // Finder doesn't tell us the modifier keys; use the default operation.
      const noMods = { altKey: false, metaKey: false, ctrlKey: false, shiftKey: false };
      if (p.type !== "drop") {
        const op = target ? dropOp(target, sources, noMods) : null;
        setDropHighlight(op ? target!.el : null);
        return;
      }
      setDropHighlight(null);
      const paths = p.paths;
      sources = [];
      if (!target || paths.length === 0) return;
      const devices = await deviceMap([...paths, target.dir]);
      const op = dropOp(target, paths, noMods, devices);
      if (!op) return;
      const info = await pathInfo(paths).catch(() => []);
      const dirs = new Set(paths.filter((_, i) => info[i]?.is_dir));
      await performDrop(paths, dirs, target, op);
    })
    .catch(() => {
      /* not running under Tauri */
    });
}

// ---------------------------------------------------------------------------
// Bookmarks (folders pinned to the sidebar)
// ---------------------------------------------------------------------------

function bookmarkMenuItem(path: string): MenuItem {
  if (pins.includes(path)) return { label: "Remove from Bookmarks", action: () => setBookmarked(path, false) };
  return { label: "Add to Bookmarks", disabled: fixedSidebarPaths.has(path), action: () => setBookmarked(path, true) };
}

async function setBookmarked(path: string, on: boolean): Promise<void> {
  if (on === pins.includes(path) || (on && fixedSidebarPaths.has(path))) return;
  const next = on ? [...pins, path] : pins.filter((p) => p !== path);
  try {
    await savePins(next);
  } catch (err) {
    showToast(`Could not save bookmarks: ${cleanError(err)}`);
    return;
  }
  pins = next;
  renderPins();
}

function renderPins(): void {
  const section = $("#sidebar-bookmarks");
  section.querySelectorAll(".sidebar-row.pin").forEach((r) => r.remove());
  for (const path of pins) sidebarRow(section, basename(path), "folder", path, false).classList.add("pin");
  updateSidebarSelection();
  badgeVersionedPins(section);
}

/** Mark bookmarked folders that keep versions (one batched call for all pins). */
async function badgeVersionedPins(section: HTMLElement): Promise<void> {
  const shownPins = [...pins];
  const summaries = await itemCounts(shownPins, false).catch(() => []);
  shownPins.forEach((path, i) => {
    const row = [...section.querySelectorAll<HTMLElement>(".sidebar-row.pin")].find((r) => r.dataset.location === path);
    if (!summaries[i]?.versioned || !row || row.querySelector(".sidebar-badge")) return;
    row.querySelector(".sidebar-row-label")!.insertAdjacentHTML("afterend", versionsBadge("sidebar-badge"));
  });
}

function onSidebarContextMenu(e: MouseEvent): void {
  const row = (e.target as HTMLElement).closest<HTMLElement>(".sidebar-row");
  if (!row) return;
  e.preventDefault();
  const loc = row.dataset.location!;
  const items: MenuItem[] = [{ label: "Open", action: () => navigate(loc) }];
  if (row.classList.contains("pin")) {
    items.push({ separator: true }, { label: "Remove from Bookmarks", action: () => setBookmarked(loc, false) });
  }
  if (loc === trashPath) items.push({ separator: true }, { label: "Empty Trash…", action: confirmEmptyTrash });
  openMenu({ x: e.clientX, y: e.clientY }, items);
}

// ---------------------------------------------------------------------------
// Full Disk Access banner (macOS)
// ---------------------------------------------------------------------------

async function initAccessBanner(): Promise<void> {
  if (!isMac) return;
  const banner = $("#fda-banner");
  $("#fda-dismiss").innerHTML = sym("window-close");
  $("#fda-open").addEventListener("click", openSettingsSafely);
  $("#fda-dismiss").addEventListener("click", () => {
    banner.hidden = true;
    setFlag("fda-banner-dismissed", true).catch(() => {});
  });
  try {
    if (await getFlag("fda-banner-dismissed")) return;
    banner.hidden = await hasFullDiskAccess();
  } catch {
    /* no banner if the check itself fails */
  }
}

// ---------------------------------------------------------------------------
// Name validation (messages match GNOME Files)
// ---------------------------------------------------------------------------

interface Validation {
  valid: boolean;
  message: string;
  error: boolean;
}

function validateName(name: string, isDir: boolean, original?: string): Validation {
  const noun = isDir ? "Folder" : "File";
  const lower = isDir ? "folder" : "file";
  if (name === "" || name === original) return { valid: false, message: "", error: false };
  if (name.includes("/")) return { valid: false, message: `${noun} names cannot contain “/”.`, error: true };
  if (name === "." || name === "..") return { valid: false, message: `A ${lower} cannot be called “${name}”.`, error: true };
  if (new TextEncoder().encode(name).length > 255) return { valid: false, message: `${noun} name is too long.`, error: true };
  const clash = entries.find((e) => e.name === name && e.name !== original);
  if (clash) {
    return { valid: false, message: `A ${clash.is_dir ? "folder" : "file"} with that name already exists.`, error: true };
  }
  if (name.startsWith(".")) {
    return { valid: true, message: `${noun}s with “.” at the beginning of their name are hidden.`, error: false };
  }
  return { valid: true, message: "", error: false };
}

function showValidation(v: Validation, entry: HTMLElement, message: HTMLElement): void {
  message.textContent = v.message;
  message.classList.toggle("error", v.error);
  entry.classList.toggle("error", v.error);
}

// ---------------------------------------------------------------------------
// Rename (popover anchored to the item, as in GNOME Files)
// ---------------------------------------------------------------------------

/** Rename several items in one folder with find/replace, prefix and suffix. */
function showBatchRename(sel: Entry[]): void {
  if (sel.length < 2) {
    if (sel[0]) startRename(sel[0]);
    return;
  }
  if (sel.some((entry) => parentOf(entry.path) !== location)) {
    showToast("Batch rename works inside one folder.");
    return;
  }
  const selectedPaths = new Set(sel.map((entry) => entry.path));
  const extra = document.createElement("div");
  extra.className = "batch-rename";
  extra.innerHTML =
    `<div class="batch-fields">` +
    `<label>Find<input class="entry" id="batch-find" type="text" spellcheck="false" /></label>` +
    `<label>Replace with<input class="entry" id="batch-replace" type="text" spellcheck="false" /></label>` +
    `<label>Add prefix<input class="entry" id="batch-prefix" type="text" spellcheck="false" /></label>` +
    `<label>Add suffix<input class="entry" id="batch-suffix" type="text" spellcheck="false" /></label>` +
    `</div><div class="batch-preview"></div><div class="field-message"></div>`;
  const field = <T extends HTMLInputElement>(id: string) => extra.querySelector<T>(`#${id}`)!;
  const find = field("batch-find");
  const replace = field("batch-replace");
  const prefix = field("batch-prefix");
  const suffix = field("batch-suffix");
  const preview = extra.querySelector<HTMLElement>(".batch-preview")!;
  const message = extra.querySelector<HTMLElement>(".field-message")!;

  const names = () => {
    const findText = find.value;
    return sel.map((entry) => {
      let name = entry.name;
      if (findText) name = name.split(findText).join(replace.value);
      if (prefix.value) name = prefix.value + name;
      if (suffix.value) {
        if (entry.is_dir) name += suffix.value;
        else {
          const dot = name.lastIndexOf(".");
          name = dot > 0 ? `${name.slice(0, dot)}${suffix.value}${name.slice(dot)}` : name + suffix.value;
        }
      }
      return { entry, name };
    });
  };

  let dialog: ReturnType<typeof openDialog>;
  const validate = () => {
    const pairs = names();
    const current = new Set(sel.map((entry) => entry.name.toLowerCase()));
    const changed = pairs.filter(({ entry, name }) => name !== entry.name);
    const seen = new Set<string>();
    let error = "";
    for (const { entry, name } of pairs) {
      if (!name || name === "." || name === ".." || name.includes("/") || name.includes("\0")) {
        error = "A name can't be empty or contain “/”.";
        break;
      }
      const lower = name.toLowerCase();
      if (seen.has(lower)) {
        error = "Two items would have the same name.";
        break;
      }
      seen.add(lower);
      if (lower !== entry.name.toLowerCase() && current.has(lower)) {
        error = "Renaming would overwrite another selected item.";
        break;
      }
      const clash = entries.find(
        (other) => !selectedPaths.has(other.path) && other.name.toLowerCase() === lower
      );
      if (lower !== entry.name.toLowerCase() && clash) {
        error = `A file or folder named “${name}” already exists.`;
        break;
      }
    }
    message.textContent = error;
    preview.replaceChildren();
    if (!error) {
      for (const { entry, name } of pairs.slice(0, 8)) {
        const row = document.createElement("div");
        row.className = "batch-preview-row";
        const before = document.createElement("span");
        before.className = "batch-old";
        before.textContent = entry.name;
        const arrow = document.createElement("span");
        arrow.className = "batch-arrow";
        arrow.textContent = "→";
        const after = document.createElement("span");
        after.className = "batch-new";
        after.textContent = name;
        row.append(before, arrow, after);
        preview.appendChild(row);
      }
      if (pairs.length > 8) {
        const more = document.createElement("div");
        more.className = "batch-more";
        more.textContent = `and ${pairs.length - 8} more`;
        preview.appendChild(more);
      }
    }
    const ok = !error && changed.length > 0;
    dialog.setResponseEnabled("rename", ok);
    return ok ? changed : null;
  };

  const inputs = [find, replace, prefix, suffix];
  inputs.forEach((input) => input.addEventListener("input", validate));

  dialog = openDialog({
    heading: `Rename ${sel.length} Items`,
    body: "Leave a field empty to skip it.",
    extra,
    className: "batch-rename-dialog",
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "rename", label: "Rename", style: "suggested" },
    ],
    defaultResponse: "rename",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "rename") return;
      const changed = validate();
      if (!changed) return false;
      void (async () => {
        try {
          await renameMany(changed.map(({ entry, name }) => ({ from: entry.path, to: joinPath(location, name) })));
          dialog.close();
          showToast(`Renamed ${changed.length} ${changed.length === 1 ? "item" : "items"}`);
          await reload();
        } catch (err) {
          message.textContent = cleanError(err);
        }
      })();
      return false;
    },
  });
  validate();
  find.focus();
}

function startRename(entry: Entry): void {
  // Recent and Starred mix folders, so the name-clash check wouldn't work there.
  if (isPseudo(location) || isReadOnlyLocation(entry.path)) return;
  const index = shown.findIndex((e) => e.path === entry.path);
  if (index < 0) return;
  scrollToIndex(index);
  const el = itemEl(index);
  if (!el) return;

  const box = document.createElement("div");
  box.className = "rename-popover";
  box.innerHTML = `
    <div class="title">${entry.is_dir ? "Rename Folder" : "Rename File"}</div>
    <div class="row">
      <input class="entry" type="text" spellcheck="false" aria-label="New name" />
      <button class="btn suggested" disabled>Rename</button>
    </div>
    <div class="field-message"></div>`;
  const input = box.querySelector("input")!;
  const button = box.querySelector("button")!;
  const message = box.querySelector<HTMLElement>(".field-message")!;
  input.value = entry.name;

  const validate = () => {
    const v = validateName(input.value, entry.is_dir, entry.name);
    showValidation(v, input, message);
    button.disabled = !v.valid;
    return v.valid;
  };

  const submit = async () => {
    if (!validate()) return;
    const target = joinPath(location, input.value);
    button.disabled = true;
    let renamed: string;
    try {
      renamed = await renameEntry(entry.path, target);
    } catch (err) {
      showValidation({ valid: false, message: cleanError(err), error: true }, input, message);
      return;
    }
    followMove(entry.path, renamed);
    handle.close();
    await reload([renamed]);
  };

  input.addEventListener("input", validate);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      submit();
    }
  });
  button.addEventListener("click", submit);

  const anchor = el.querySelector<HTMLElement>(".name") ?? el;
  const handle = openPopover(anchor, box, { focus: input });
  // Select the name without its extension, like GNOME Files.
  const dot = entry.name.lastIndexOf(".");
  input.setSelectionRange(0, !entry.is_dir && dot > 0 ? dot : entry.name.length);
}

// ---------------------------------------------------------------------------
// New folder (AdwDialog with an entry row)
// ---------------------------------------------------------------------------

function promptNewFolder(): void {
  if (isPseudo(location) || isReadOnlyLocation(location) || listingError !== null) return;
  const inside = !!versionedRootPath;
  // Keep Versions never applies to Drive (spec D9).
  const versionsRow = isCloud(location)
    ? ""
    : `<div class="check-row versions-check">
      <label><input id="new-folder-versions" type="checkbox" ${inside ? "disabled" : ""} /> Keep versions <span class="badge experimental">Experimental</span></label>
      <div class="check-hint">${inside ? "Already inside a folder that keeps versions" : "Keeps every saved version of the files in this folder."}</div>
    </div>`;
  const extra = document.createElement("div");
  extra.innerHTML = `
    <div class="entry-row"><label for="new-folder-name">Folder Name</label><input id="new-folder-name" type="text" spellcheck="false" /></div>
    ${versionsRow}
    <div class="field-message" style="margin-top:8px"></div>`;
  const row = extra.querySelector<HTMLElement>(".entry-row")!;
  const input = extra.querySelector("input#new-folder-name")! as HTMLInputElement;
  const keep = extra.querySelector<HTMLInputElement>("#new-folder-versions");
  const message = extra.querySelector<HTMLElement>(".field-message")!;
  const dir = location;

  const validate = () => {
    const v = validateName(input.value, true);
    showValidation(v, row, message);
    dialog.setResponseEnabled("create", v.valid);
    return v.valid;
  };

  const dialog = openDialog({
    heading: "New Folder",
    extra,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "create", label: "Create", style: "suggested" },
    ],
    defaultResponse: "create",
    onResponse: async (id) => {
      if (id !== "create") return;
      if (!validate()) return false;
      const target = joinPath(dir, input.value);
      let created = target;
      try {
        if (keep?.checked) await makeVersionedDir(target);
        else created = await makeDir(target);
      } catch (err) {
        // D13: the folder may exist even when turning versions on failed.
        if (await pathExists(target).catch(() => false)) {
          dialog.close();
          await reload([target]);
          showToast(cleanError(err));
          return;
        }
        showValidation({ valid: false, message: cleanError(err), error: true }, row, message);
        return false;
      }
      if (location === dir) await reload([created]);
    },
  });
  input.addEventListener("input", validate);
  validate();
}

// ---------------------------------------------------------------------------
// Preview (Space)
// ---------------------------------------------------------------------------

let previewUrl: string | null = null;
let previewSeq = 0;

function closePreview(): void {
  if (!previewLayer) return;
  previewSeq += 1;
  previewLayer.hidden = true;
  previewBody.replaceChildren();
  previewName.textContent = "";
  if (previewUrl) URL.revokeObjectURL(previewUrl);
  previewUrl = null;
}

function previewEmpty(entry: Entry, text: string): void {
  const box = document.createElement("div");
  box.className = "preview-empty";
  const icon = document.createElement("div");
  icon.className = "preview-empty-icon";
  icon.innerHTML = entryIcon(entry, folderKind(entry));
  useNativeIcon(icon, entry.path, 192);
  const label = document.createElement("p");
  label.textContent = text;
  box.append(icon, label);
  previewBody.appendChild(box);
}

async function showPreview(entry: Entry): Promise<void> {
  if (!previewLayer) return;
  closePreview();
  const seq = previewSeq;
  previewLayer.hidden = false;
  previewName.textContent = entry.name;
  previewBody.replaceChildren();

  if (isCloud(entry.path) || isSharePath(entry.path) || isSftpPath(entry.path)) {
    previewEmpty(entry, "Preview isn't available for remote items.");
    return;
  }
  if (entry.is_dir) {
    previewEmpty(entry, "Folder");
    return;
  }
  await previewEntry(entry, seq, fileKind(entry.name));
}

function previewStale(seq: number): boolean {
  return seq !== previewSeq || Boolean(previewLayer.hidden);
}

async function previewEntry(entry: Entry, seq: number, kind: FileKind): Promise<void> {
  if (kind === "text" || kind === "code") return previewTextEntry(entry, seq);
  if (THUMB_KINDS.has(kind)) return previewThumbnailEntry(entry, seq);
  previewEmpty(entry, "No preview available.");
}

async function previewTextEntry(entry: Entry, seq: number): Promise<void> {
  try {
    const result = await previewText(entry.path, 200_000);
    if (previewStale(seq)) return;
    const pre = document.createElement("pre");
    pre.textContent = result.text + (result.truncated ? "\n…" : "");
    previewBody.appendChild(pre);
  } catch {
    if (previewStale(seq)) return;
    previewEmpty(entry, "No preview available.");
  }
}

async function previewThumbnailEntry(entry: Entry, seq: number): Promise<void> {
  try {
    const bytes = await thumbnail(entry.path, 1024);
    if (previewStale(seq)) return;
    if (!bytes.byteLength) {
      previewEmpty(entry, "No preview available.");
      return;
    }
    previewUrl = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
    const image = document.createElement("img");
    image.src = previewUrl;
    image.alt = entry.name;
    previewBody.appendChild(image);
  } catch {
    if (previewStale(seq)) return;
    previewEmpty(entry, "No preview available.");
  }
}

function togglePreview(): void {
  if (!previewLayer) return;
  if (!previewLayer.hidden) {
    closePreview();
    return;
  }
  const sel = selectedEntries();
  if (sel.length === 1) void showPreview(sel[0]);
}

/** A list of installed applications for "Open With…". */
function showOpenWith(entry: Entry): void {
  const extra = document.createElement("div");
  extra.className = "app-list";
  const loading = document.createElement("p");
  loading.className = "app-loading";
  loading.textContent = "Loading applications…";
  const message = document.createElement("div");
  message.className = "field-message";
  extra.append(loading, message);

  const dialog = openDialog({
    heading: `Open “${entry.name}” With`,
    extra,
    className: "open-with-dialog",
    responses: [{ id: "cancel", label: "Cancel" }],
    defaultResponse: "cancel",
    closeResponse: "cancel",
  });

  void listApps()
    .then((apps) => {
      if (!extra.isConnected) return;
      loading.remove();
      if (apps.length === 0) {
        const empty = document.createElement("p");
        empty.className = "app-loading";
        empty.textContent = "No applications found.";
        extra.prepend(empty);
        return;
      }
      for (const app of apps) {
        const button = document.createElement("button");
        button.className = "app-choice";
        const icon = document.createElement("span");
        icon.className = "app-choice-icon";
        icon.innerHTML = entryIcon(
          {
            name: `${app.name}.app`,
            path: "",
            is_dir: true,
            size: 0,
            modified: 0,
            is_hidden: false,
            is_symlink: false,
          },
          "plain"
        );
        useNativeIcon(icon, appIconKey(app.id), 28);
        const label = document.createElement("span");
        label.className = "app-choice-name";
        label.textContent = app.name;
        button.append(icon, label);
        button.addEventListener("click", () => {
          button.disabled = true;
          openWith(entry.path, app.id)
            .then(() => {
              dialog.close();
              if (!isCloud(entry.path) && !isSharePath(entry.path)) addRecent(entry.path).catch(() => {});
            })
            .catch((err) => {
              button.disabled = false;
              message.textContent = cleanError(err);
            });
        });
        extra.insertBefore(button, message);
      }
    })
    .catch((err) => {
      if (!extra.isConnected) return;
      loading.textContent = `Could not list applications: ${cleanError(err)}`;
    });
}

// ---------------------------------------------------------------------------
// Keyboard shortcuts + about
// ---------------------------------------------------------------------------

const SHORTCUTS: [string, [string, string][]][] = [
  ["Navigation", [
    ["Back", "Alt+←"], ["Forward", "Alt+→"], ["Parent folder", "Alt+↑"], ["Home folder", "Alt+Home"],
    ["Enter location", `${MOD}L`], ["Search", `${MOD}F`], ["Reload", `${MOD}R`],
    ["New window", `${MOD}N`], ["New tab", `${MOD}T`], ["Close tab", `${MOD}W`], ["Next / previous tab", "Ctrl+Page Down / Page Up"],
  ]],
  ["View", [
    ["List view", `${MOD}1`], ["Grid view", `${MOD}2`], ["Zoom in / out", `${MOD}+ / ${MOD}−, Ctrl + wheel, pinch`],
    ["Reset zoom", `${MOD}0`], ["Show hidden files", isMac ? "⇧⌘." : "Ctrl+H"],
  ]],
  ["Files", [
    ["Open", "Enter"], ["Preview", "Space"], ["Rename", "F2"], ["New folder", isMac ? "⇧⌘N" : "Shift+Ctrl+N"], ["Select all", `${MOD}A`],
    ["Copy / Cut / Paste", `${MOD}C / ${MOD}X / ${MOD}V`], ["Move to Trash", isMac ? "⌘⌫ or Delete" : "Delete"], ["Undo Move to Trash", `${MOD}Z`],
    ["Properties", `${MOD}I`], ["Cancel cut", "Esc"], ["Bookmark this folder", `${MOD}D`],
  ]],
];

function showShortcuts(): void {
  const grid = document.createElement("div");
  grid.className = "shortcuts";
  for (const [group, rows] of SHORTCUTS) {
    const g = document.createElement("div");
    g.className = "group";
    g.textContent = group;
    grid.appendChild(g);
    for (const [label, keys] of rows) {
      const l = document.createElement("span");
      l.textContent = label;
      const k = document.createElement("kbd");
      k.textContent = keys;
      grid.append(l, k);
    }
  }
  openDialog({ heading: "Keyboard Shortcuts", extra: grid, responses: [{ id: "close", label: "Close" }], defaultResponse: "close", closeResponse: "close" });
}

declare const __APP_VERSION__: string;

function showAbout(): void {
  const extra = document.createElement("div");
  extra.className = "about";
  extra.innerHTML = `<svg class="about-icon" viewBox="0 0 64 64"><use href="#fc-folder-home"/></svg>
    <p>A GNOME Files look-alike for macOS and Linux.<br/>Version ${escapeHtml(__APP_VERSION__)}</p>
    <p class="dim">An independent hobby project, not affiliated with or endorsed by the
      GNOME Project, Apple or anyone else. Design credit goes to the GNOME Files team.</p>
    <p class="dim">Licensed under the GNU General Public License, version 3 or later.
      This program comes with absolutely no warranty.</p>`;
  openDialog({ heading: "File Browser", extra, responses: [{ id: "close", label: "Close" }], defaultResponse: "close", closeResponse: "close" });
}

// ---------------------------------------------------------------------------
// Keyboard handling
// ---------------------------------------------------------------------------

function moveCursor(to: number, extend: boolean): void {
  if (shown.length === 0) return;
  to = Math.min(Math.max(to, 0), shown.length - 1);
  viewEl.classList.add("keyboard-nav");
  if (extend && anchorIndex >= 0) selectRange(anchorIndex, to, false);
  else selectOnly(to);
  scrollToIndex(to);
}

/** A key press in the view, with what the key handlers need to know. */
interface ViewKey {
  e: KeyboardEvent;
  mod: boolean;
  /** The cursor, or -1 when there is none. */
  cur: number;
  cols: number;
  /** Items per page, for Page Up/Down. */
  page: number;
}

/** Move the cursor to `index`, or to the first item when there is no cursor yet. */
const stepFrom = (k: ViewKey, delta: number) => moveCursor(k.cur < 0 ? 0 : k.cur + delta, k.e.shiftKey);

/** Left/right move by one item, in grid view only. */
function gridStep(k: ViewKey, delta: number): boolean {
  if (k.e.altKey || k.mod) return false;
  if (prefs.view === "grid") stepFrom(k, delta);
  return true;
}

function toggleCursorItem(k: ViewKey): void {
  if (k.cur < 0 || !k.mod) return;
  noteSelectionIntent();
  const p = shown[k.cur].path;
  if (selected.has(p)) selected.delete(p);
  else selected.add(p);
  updateSelection();
}

/**
 * View keys. A handler returns false to leave the key alone (no
 * preventDefault), so Alt and ⌘ combinations reach the global shortcuts.
 */
const VIEW_KEYS: Record<string, (k: ViewKey) => boolean> = {
  ArrowRight: (k) => gridStep(k, 1),
  ArrowLeft: (k) => gridStep(k, -1),
  ArrowDown: (k) => {
    if (k.e.altKey) return false;
    if (k.mod) activateSelection();
    else stepFrom(k, k.cols);
    return true;
  },
  ArrowUp: (k) => {
    if (k.e.altKey || k.mod) return false;
    stepFrom(k, -k.cols);
    return true;
  },
  Home: (k) => {
    if (k.e.altKey) return false;
    moveCursor(0, k.e.shiftKey);
    return true;
  },
  End: (k) => (moveCursor(shown.length - 1, k.e.shiftKey), true),
  PageDown: (k) => (moveCursor(k.cur + k.page, k.e.shiftKey), true),
  PageUp: (k) => (moveCursor(k.cur - k.page, k.e.shiftKey), true),
  Enter: () => (activateSelection(), true),
  F2: () => {
    const sel = selectedEntries();
    if (sel.length === 1) startRename(sel[0]);
    return true;
  },
  Escape: () => {
    if (!previewLayer?.hidden) closePreview();
    else if (searchQuery !== null) closeSearch();
    else if (!cancelCut()) clearSelection();
    return true;
  },
  Delete: () => (trashOrDelete(), true),
  Backspace: (k) => {
    if (k.mod) trashOrDelete();
    else goBack();
    return true;
  },
  " ": (k) => {
    if (k.mod) toggleCursorItem(k);
    else togglePreview();
    return true;
  },
};

function onViewKeyDown(e: KeyboardEvent): void {
  const mod = isMac ? e.metaKey : e.ctrlKey;
  const handler = VIEW_KEYS[e.key];
  if (!handler) {
    typeAhead(e, mod);
    return;
  }
  const cols = layout.cols;
  const page = Math.max(Math.floor(scrollEl.clientHeight / (layout.itemH + layout.gap)), 1) * cols;
  if (handler({ e, mod, cur: cursorIndex, cols, page })) e.preventDefault();
}

/** Printable keys start a search, like GNOME Files. */
function typeAhead(e: KeyboardEvent, mod: boolean): void {
  if (e.key.length !== 1 || mod || e.altKey || e.ctrlKey || e.metaKey || e.key === " ") return;
  e.preventDefault();
  openSearch(e.key);
}

/** The modifier state of a key press, with ⌘ (macOS) or Ctrl (elsewhere) as `mod`. */
interface Keys {
  mod: boolean;
  shift: boolean;
  alt: boolean;
  ctrl: boolean;
  meta: boolean;
  /** `e.key` as typed. */
  raw: string;
  /** `e.key` lower-cased, for letter shortcuts. */
  key: string;
}

type Shortcut = [when: (k: Keys) => boolean, action: () => void];

const plainMod = (k: Keys, key: string) => k.mod && !k.shift && k.key === key;

/** Shortcuts that work everywhere, even while typing in a field. First match wins. */
const ANYWHERE_SHORTCUTS: Shortcut[] = [
  [(k) => plainMod(k, "l"), () => openPathEntry()],
  [(k) => plainMod(k, "f"), () => openSearch()],
  [(k) => plainMod(k, "n"), () => void openNewWindow()],
  [(k) => plainMod(k, "t"), () => void newTab()],
  [(k) => plainMod(k, "w"), () => void closeTab(activeTab)],
  [(k) => k.ctrl && k.raw === "PageDown", () => cycleTab(1)],
  [(k) => k.ctrl && k.raw === "PageUp", () => cycleTab(-1)],
  [(k) => k.mod && k.shift && k.key === "n", promptNewFolder],
  [(k) => plainMod(k, "d") && !isPseudo(location), () => setBookmarked(location, !pins.includes(location))],
  [(k) => (k.mod && k.shift && (k.raw === "." || k.raw === ">")) || (k.ctrl && !k.shift && k.key === "h"), toggleHidden],
  [(k) => plainMod(k, "r") || k.raw === "F5", () => reload()],
  [(k) => k.mod && (k.raw === "?" || (k.shift && k.raw === "/")), showShortcuts],
  [(k) => k.mod && k.key === "1", () => setView("list")],
  [(k) => k.mod && k.key === "2", () => setView("grid")],
  [(k) => k.mod && (k.raw === "=" || k.raw === "+"), () => zoom(1)],
  [(k) => k.mod && k.raw === "-", () => zoom(-1)],
  [(k) => k.mod && k.key === "0", resetZoom],
  [(k) => k.alt && k.raw === "ArrowLeft", goBack],
  [(k) => k.alt && k.raw === "ArrowRight", goForward],
  [(k) => k.alt && k.raw === "ArrowUp", goUp],
  [(k) => k.alt && k.raw === "Home", () => navigate(home)],
  [(k) => isMac && k.meta && k.raw === "[", goBack],
  [(k) => isMac && k.meta && k.raw === "]", goForward],
  [(k) => isMac && k.meta && k.raw === "ArrowUp", goUp],
];

/** Shortcuts for the files themselves; ignored while typing in a field. */
const VIEW_SHORTCUTS: Shortcut[] = [
  [(k) => plainMod(k, "z") && lastTrashed.length > 0, undoTrash],
  [(k) => k.mod && k.key === "a", selectAll],
  [(k) => plainMod(k, "c") && selected.size > 0, () => setClipboard("copy")],
  [(k) => plainMod(k, "x") && selected.size > 0, () => setClipboard("cut")],
  [(k) => plainMod(k, "v") && !!clipboard && !isPseudo(location), () => paste(location)],
  [(k) => plainMod(k, "i") && selected.size === 1, showSelectedProperties],
];

function showSelectedProperties(): void {
  const entry = selectedEntries()[0];
  if (entry) showProperties(entry);
}

function onGlobalKeyDown(e: KeyboardEvent): void {
  if (hasOpenOverlay()) return;
  const k: Keys = {
    mod: isMac ? e.metaKey : e.ctrlKey,
    shift: e.shiftKey,
    alt: e.altKey,
    ctrl: e.ctrlKey,
    meta: e.metaKey,
    raw: e.key,
    key: e.key.toLowerCase(),
  };
  const inInput = (e.target as HTMLElement).tagName === "INPUT";
  const tables = inInput ? [ANYWHERE_SHORTCUTS] : [ANYWHERE_SHORTCUTS, VIEW_SHORTCUTS];
  for (const table of tables) {
    const hit = table.find(([when]) => when(k));
    if (hit) {
      e.preventDefault();
      hit[1]();
      return;
    }
  }
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

/** Add a sidebar row; `fixed` rows are built-in and can't be bookmarked again. */
function sidebarRow(
  section: HTMLElement,
  label: string,
  icon: string,
  loc: string,
  fixed = true,
  onClick?: () => void
): HTMLElement {
  const row = document.createElement("div");
  row.className = "sidebar-row";
  row.dataset.location = loc;
  row.setAttribute("role", "link");
  row.innerHTML = `${sym(icon)}<span class="sidebar-row-label"></span>`;
  row.querySelector(".sidebar-row-label")!.textContent = label;
  row.title = loc ? (isPseudo(loc) ? label : loc) : label;
  row.addEventListener("click", onClick ?? (() => void navigate(loc)));
  section.appendChild(row);
  if (fixed && loc && !isPseudo(loc)) fixedSidebarPaths.add(loc);
  return row;
}

function updateSidebarSelection(): void {
  document.querySelectorAll<HTMLElement>(".sidebar-row").forEach((row) => {
    row.classList.toggle("selected", row.dataset.location === location);
  });
}

/** How to create the OAuth client File Browser needs (docs/google-drive.md). */
const DRIVE_GUIDE_URL =
  "https://github.com/ahmadsayed/unshitty-mac-filebrowser/blob/main/docs/google-drive.md";

/** Ask for the name visitors see, the first time sharing is used. */
async function ensureShareName(): Promise<boolean> {
  try {
    const identity = await shareIdentity();
    if (identity.name) return true;
  } catch (err) {
    showToast(`Could not read sharing settings: ${cleanError(err)}`);
    return false;
  }

  return new Promise((resolve) => {
    const extra = document.createElement("div");
    extra.innerHTML =
      `<div class="entry-row"><label for="share-name">Your name</label>` +
      `<input id="share-name" class="entry" type="text" spellcheck="false" /></div>` +
      `<div class="field-message"></div>`;
    const input = extra.querySelector<HTMLInputElement>("#share-name")!;
    const message = extra.querySelector<HTMLElement>(".field-message")!;
    // Pre-filled with the account name, as the spec asks (D12).
    const account = home && home !== "/" ? basename(home) : "";
    input.value = account;
    let dialog: ReturnType<typeof openDialog>;

    const save = async () => {
      const name = input.value.trim();
      if (!name) {
        message.textContent = "Enter a name.";
        return;
      }
      try {
        await setShareName(name);
        resolve(true);
        dialog.close();
      } catch (err) {
        message.textContent = cleanError(err);
      }
    };

    input.addEventListener("input", () => dialog.setResponseEnabled("save", !!input.value.trim()));
    dialog = openDialog({
      heading: "What name should people see?",
      body: "People you share with will see this name.",
      extra,
      responses: [
        { id: "cancel", label: "Cancel" },
        { id: "save", label: "Continue", style: "suggested" },
      ],
      defaultResponse: "save",
      closeResponse: "cancel",
      onResponse: (id) => {
        if (id !== "save") {
          resolve(false);
          return;
        }
        void save();
        return false;
      },
    });
    dialog.setResponseEnabled("save", !!input.value.trim());
  });
}

function accessState(entry: AccessEntry): string {
  switch (entry.state) {
    case "unused":
      return "Hasn't opened the link yet";
    case "online":
      return "Online";
    case "offline":
      return "Offline";
  }
}

function shareAccessHtml(info: ShareInfo): string {
  if (!info.access.length) {
    return `<p class="share-empty">No links yet. Copy a link to let someone in.</p>`;
  }
  return info.access
    .map((entry) => {
      const name = entry.kind === "anyone" ? "Anyone with the link" : entry.name || "Specific person";
      const visitors = entry.kind === "anyone" && entry.visitors
        ? ` · ${entry.visitors} ${entry.visitors === 1 ? "computer" : "computers"}`
        : "";
      const protection = entry.protected ? " · Passphrase protected" : "";
      return `<div class="share-access-row">` +
        `<div><div class="share-access-name">${escapeHtml(name)}</div>` +
        `<div class="share-access-state">${escapeHtml(accessState(entry) + visitors + protection)}</div></div>` +
        `<button class="btn share-copy" data-copy="${escapeHtml(entry.id)}">Copy Link</button>` +
        `<button class="flat-btn share-remove" data-remove="${escapeHtml(entry.id)}" ` +
        `title="Remove access" aria-label="Remove access">${sym("window-close")}</button></div>`;
    })
    .join("");
}

/** The Share dialog for a local folder (and, later, Manage Sharing). */
async function showShareDialog(entry: Entry): Promise<void> {
  if (!entry.is_dir || isReadOnlyLocation(entry.path) || isCloud(entry.path) || inTrash()) return;
  if (!(await ensureShareName())) return;

  // A big folder takes a moment to walk and hash, so say so instead of
  // leaving the menu action looking ignored.
  const preparing = openDialog({
    heading: `Share “${entry.name}”`,
    body: "Preparing…",
    responses: [],
    closeResponse: "cancel",
  });
  let info: ShareInfo;
  try {
    info = await shareFolder(entry.path);
  } catch (err) {
    preparing.close();
    showToast(`Could not share “${entry.name}”: ${cleanError(err)}`);
    return;
  }
  preparing.close();

  const extra = document.createElement("div");
  const protection = createShareProtection();
  const accessList = document.createElement("div");
  accessList.className = "share-access-list";
  extra.append(protection.element, accessList);
  const render = () => {
    accessList.innerHTML = shareAccessHtml(info);
  };
  render();

  let dialog: ReturnType<typeof openDialog>;
  let busy = false;
  const setBusy = (value: boolean) => {
    busy = value;
    protection.element.disabled = value;
    for (const id of ["anyone", "person", "stop", "close"]) dialog.setResponseEnabled(id, !value);
    accessList.querySelectorAll<HTMLButtonElement>("button").forEach((button) => { button.disabled = value; });
  };
  const copyToClipboard = async (link: string) => {
    try {
      await navigator.clipboard.writeText(link);
      showToast("Link copied");
    } catch (err) {
      showToast(`Could not copy the link: ${cleanError(err)}`);
    }
  };
  const createLink = async (kind: "anyone" | "person") => {
    const passphrase = protection.read();
    if (passphrase === null) return;
    setBusy(true);
    try {
      const person = kind === "person" ? await promptPersonName() : undefined;
      if (person === null) return;
      const made = await createShareLink(info.id, kind, person, passphrase);
      if (!info.access.some((entry) => entry.id === made.entry.id)) info.access.push(made.entry);
      protection.clear();
      render();
      await copyToClipboard(made.link);
    } catch (err) {
      showToast(`Could not create the link: ${cleanError(err)}`);
    } finally {
      setBusy(false);
    }
  };

  const confirmStop = () => {
    openDialog({
      heading: `Stop sharing “${entry.name}”?`,
      body: "People you shared it with will no longer be able to open it.",
      responses: [
        { id: "cancel", label: "Cancel" },
        { id: "stop", label: "Stop Sharing", style: "destructive" },
      ],
      defaultResponse: "cancel",
      closeResponse: "cancel",
      onResponse: (id) => {
        if (id !== "stop") return;
            stopSharing(info.id)
          .then(async () => {
            dialog.close();
            protection.clear();
            await refreshShares();
            renderWindow(true);
            showToast("Stopped sharing");
          })
          .catch((err) => showToast(`Could not stop sharing: ${cleanError(err)}`));
      },
    });
  };

  accessList.addEventListener("click", (event) => {
    if (busy) return;
    const copy = (event.target as HTMLElement).closest<HTMLElement>("[data-copy]");
    if (copy?.dataset.copy) {
      shareLinkFor(info.id, copy.dataset.copy)
        .then(copyToClipboard)
        .catch((err) => showToast(`Could not copy the link: ${cleanError(err)}`));
      return;
    }
    const button = (event.target as HTMLElement).closest<HTMLElement>("[data-remove]");
    if (!button?.dataset.remove) return;
    const id = button.dataset.remove;
    removeShareAccess(info.id, id)
      .then(() => {
        info.access = info.access.filter((entry) => entry.id !== id);
        render();
        showToast("Access removed");
        void refreshShares();
      })
      .catch((err) => showToast(`Could not remove access: ${cleanError(err)}`));
  });

  dialog = openDialog({
    heading: `Share “${entry.name}”`,
    body: "Who can open this folder. People you share with can open it read-only.",
    extra,
    className: "share-dialog",
    responses: [
      { id: "anyone", label: "Copy Link" },
      { id: "person", label: "Specific Person…" },
      { id: "stop", label: "Stop Sharing…", style: "destructive" },
      { id: "close", label: "Close" },
    ],
    defaultResponse: "close",
    closeResponse: "close",
    onResponse: async (id) => {
      if (busy) return false;
      if (id === "anyone" || id === "person") {
        await createLink(id);
        void refreshShares().then(() => renderWindow(true));
        return false;
      }
      if (id === "stop") {
        confirmStop();
        return false;
      }
      protection.clear();
    },
  });
}

function showShareDialogForPath(path: string): void {
  void showShareDialog({
    name: basename(path),
    path,
    is_dir: true,
    size: 0,
    modified: 0,
    is_hidden: false,
    is_symlink: false,
  });
}

function promptPersonName(): Promise<string | null> {
  return new Promise((resolve) => {
    const extra = document.createElement("div");
    extra.innerHTML =
      `<div class="entry-row"><label for="share-person">Person's name</label>` +
      `<input id="share-person" class="entry" type="text" spellcheck="false" /></div>` +
      `<div class="field-message"></div>`;
    const input = extra.querySelector<HTMLInputElement>("#share-person")!;
    const message = extra.querySelector<HTMLElement>(".field-message")!;
    let dialog: ReturnType<typeof openDialog>;
    const create = () => {
      const name = input.value.trim();
      if (!name) {
        message.textContent = "Enter a name.";
        return;
      }
      resolve(name);
      dialog.close();
    };
    input.addEventListener("input", () => dialog.setResponseEnabled("create", !!input.value.trim()));
    dialog = openDialog({
      heading: "Share With a Specific Person",
      body: "Give the link to this person. It works on the first computer that opens it.",
      extra,
      responses: [
        { id: "cancel", label: "Cancel" },
        { id: "create", label: "Create Link", style: "suggested" },
      ],
      defaultResponse: "create",
      closeResponse: "cancel",
      onResponse: (id) => {
        if (id !== "create") {
          resolve(null);
          return;
        }
        create();
        return false;
      },
    });
    dialog.setResponseEnabled("create", false);
  });
}

async function connectDrive(): Promise<void> {
  const status = await driveClientStatus().catch(() => null);
  if (status?.configured) openConnectDialog();
  else openClientSetup();
}


/**
 * File Browser ships no Google client: the user creates a Desktop-app OAuth
 * client in their own Google Cloud project and gives us its JSON file, by
 * choosing the downloaded file or pasting its contents.
 */
function openClientSetup(): void {
  const extra = document.createElement("div");
  const json = document.createElement("textarea");
  json.className = "client-json";
  json.placeholder = "Paste the client JSON here, or choose the downloaded file";
  json.spellcheck = false;
  const file = document.createElement("input");
  file.type = "file";
  file.accept = ".json,application/json";
  file.hidden = true;
  const error = document.createElement("p");
  error.className = "dialog-text dialog-error";
  extra.append(
    dialogLinks([
      { label: "Open Guide", action: () => void openExternal(DRIVE_GUIDE_URL) },
      { label: "Choose File…", action: () => file.click() },
    ]),
    json,
    file,
    error,
  );

  const submit = async (): Promise<boolean> => {
    try {
      await driveClientSet(json.value);
      dialog.close();
      openConnectDialog();
      return true;
    } catch (err) {
      error.textContent = cleanError(err);
      return false;
    }
  };
  file.addEventListener("change", async () => {
    const chosen = file.files?.[0];
    if (!chosen) return;
    json.value = await chosen.text();
    file.value = "";
    await submit();
  });

  // No default response: Enter must stay a newline in the paste box.
  const dialog = openDialog({
    heading: "Add Your Google Client",
    body: "Google Drive needs an OAuth client from your own Google Cloud project. The guide shows how to create one.",
    extra,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "add", label: "Add Client", style: "suggested" },
    ],
    closeResponse: "cancel",
    onResponse: async (id) => {
      if (id !== "add") return;
      if (!(await submit())) return false;
    },
  });
  json.focus();
}

function openConnectDialog(): void {
  let flowId: string | null = null;
  let busy = false;
  const extra = document.createElement("div");
  const status = document.createElement("p");
  status.className = "dialog-text";
  status.textContent =
    "File Browser asks Google for access to see, edit, create and delete your Drive files. The token is kept in your system keychain. " +
    "Google warns that your client is unverified: choose Advanced, then continue.";
  extra.append(
    status,
    dialogLinks([
      { label: "Open Guide", action: () => void openExternal(DRIVE_GUIDE_URL) },
      {
        label: "Change Client…",
        action: () => {
          if (busy) return;
          driveClientClear()
            .then(() => {
              dialog.close();
              openClientSetup();
            })
            .catch((err) => showToast(cleanError(err)));
        },
      },
    ]),
  );
  const dialog = openDialog({
    heading: "Connect to Google Drive",
    body: "Google opens in your browser. This app never sees your password.",
    extra,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "connect", label: "Open Browser", style: "suggested" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: async (id) => {
      if (id === "cancel") {
        if (flowId) await driveConnectCancel(flowId).catch(() => {});
        return;
      }
      if (busy) return false;
      busy = true;
      dialog.setResponseEnabled("connect", false);
      try {
        const start = await driveConnectStart();
        flowId = start.flow_id;
        status.textContent = "Waiting for Google…";
        await openExternal(start.auth_url);
        const account = await driveConnectFinish(start.flow_id);
        driveAccount = account;
        dialog.close();
        showToast(`Connected to ${account.email}`);
        await buildSidebar();
        await navigate(account.root_path);
      } catch (err) {
        const message = cleanError(err);
        if (!/cancelled/i.test(message)) {
          status.textContent = message;
          dialog.setResponseEnabled("connect", true);
          showToast(message);
        }
        busy = false;
        return false;
      }
      busy = false;
    },
  });
}

function confirmDisconnectDrive(): void {
  const account = driveAccount;
  if (!account) return;
  openDialog({
    heading: "Disconnect Google Drive?",
    body: `“${account.email}” will be removed from File Browser. Files in Google Drive are not deleted.`,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "disconnect", label: "Disconnect", style: "destructive" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "disconnect") return;
      driveDisconnect(account.id)
        .then(async () => {
          driveAccount = null;
          showToast("Disconnected from Google Drive");
          if (isCloud(location)) await navigate(home || "/");
          await buildSidebar();
        })
        .catch((err) => showToast(`Could not disconnect: ${cleanError(err)}`));
    },
  });
}

async function buildSidebar(): Promise<void> {
  const places = $("#sidebar-places");
  const bookmarks = $("#sidebar-bookmarks");
  const mounts = $("#sidebar-mounts");

  places.replaceChildren();
  bookmarks.replaceChildren();
  mounts.replaceChildren();
  fixedSidebarPaths.clear();

  try {
    driveAccount = (await driveAccounts())[0] ?? null;
  } catch {
    driveAccount = null;
  }

  const tryDir = async (fn: () => Promise<string>) => {
    try {
      return await fn();
    } catch {
      return "";
    }
  };

  if (home) sidebarRow(places, "Home", "user-home", home);
  sidebarRow(places, "Recent", "document-open-recent", RECENT);
  sidebarRow(places, "Starred", "starred", STARRED);
  sidebarRow(places, "Network", "network-workgroup", NETWORK);
  const driveRow = sidebarRow(
    places,
    "Google Drive",
    "network-workgroup",
    driveAccount?.root_path ?? "",
    !!driveAccount,
    driveAccount ? undefined : () => void connectDrive()
  );
  if (driveAccount) {
    driveRow.title = driveAccount.email;
    driveRow.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      event.stopPropagation();
      openMenu(event, [{ label: "Disconnect…", action: () => confirmDisconnectDrive() }]);
    });
  } else {
    driveRow.title = "Connect to Google Drive";
  }
  if (trashPath) sidebarRow(places, "Trash", "user-trash", trashPath);

  // Desktop plus GNOME's default bookmarks, resolved to this machine's folders
  // (on macOS "Videos" is ~/Movies, so labels use the real folder name).
  const xdg: [FolderKind, () => Promise<string>, string][] = [
    ["desktop", desktopDir, "user-desktop"],
    ["documents", documentDir, "folder-documents"],
    ["music", audioDir, "folder-music"],
    ["pictures", pictureDir, "folder-pictures"],
    ["videos", videoDir, "folder-videos"],
    ["download", downloadDir, "folder-download"],
  ];
  if (home) folderKinds.set(home, "home");
  for (const [kind, fn, icon] of xdg) {
    const path = await tryDir(fn);
    if (!path) continue;
    folderKinds.set(path, kind);
    sidebarRow(bookmarks, basename(path), icon, path);
  }
  const pub = await tryDir(publicDir);
  if (pub) folderKinds.set(pub, "publicshare");

  // Mounted volumes. The backend knows where the platform keeps them
  // (`/Volumes` on macOS, udisks2's mount folders and `/mnt` on Linux).
  try {
    mountedVolumes = await listVolumes();
    for (const v of mountedVolumes) sidebarRow(mounts, v.name, "drive-harddisk", v.path);
  } catch {
    /* no volume listing on this system */
  }
  $("#sidebar-mounts-separator").hidden = mounts.childElementCount === 0;

  pins = await loadPins().catch(() => []);
  starred = new Set(await loadStarred().catch(() => []));
  renderPins();
}

// ---------------------------------------------------------------------------
// Window buttons: on Linux the header bars are the title bar
// ---------------------------------------------------------------------------

type WindowAction = "minimize" | "maximize" | "close";

const WINDOW_BUTTON_LABELS: Record<WindowAction, string> = { minimize: "Minimize", maximize: "Maximize", close: "Close" };

/** Draw the buttons GNOME's `button-layout` asks for: the start ones in the
 * sidebar header, the end ones in the content header bar. macOS gets none. */
async function setupWindowButtons(): Promise<void> {
  const buttons = await windowButtons().catch(() => ({ start: [], end: [] }));
  placeWindowButtons($(".sidebar-header"), buttons.start, "start");
  placeWindowButtons($(".headerbar"), buttons.end, "end");
  if (!document.querySelector(".window-maximize")) return;
  const sync = () => windowControl("state").then(showMaximized, () => {});
  window.addEventListener("resize", sync);
  sync();
}

function placeWindowButtons(bar: HTMLElement, names: string[], side: "start" | "end"): void {
  const actions = names.filter((name): name is WindowAction => name in WINDOW_BUTTON_LABELS);
  if (!actions.length) return;
  const box = document.createElement("div");
  box.className = `window-controls window-controls-${side}`;
  for (const action of actions) {
    const btn = document.createElement("button");
    btn.className = `window-control window-${action}`;
    btn.title = WINDOW_BUTTON_LABELS[action];
    btn.setAttribute("aria-label", btn.title);
    btn.innerHTML = sym(`window-${action}`);
    btn.addEventListener("click", () => void windowControl(action).then(showMaximized, () => {}));
    box.append(btn);
  }
  if (side === "start") bar.prepend(box);
  else bar.append(box);
}

/** Maximize turns into Restore while the window is maximized. */
function showMaximized(maximized: boolean): void {
  for (const btn of document.querySelectorAll<HTMLElement>(".window-maximize")) {
    btn.title = maximized ? "Restore" : "Maximize";
    btn.setAttribute("aria-label", btn.title);
    btn.innerHTML = sym(maximized ? "window-restore" : "window-maximize");
  }
}

// ---------------------------------------------------------------------------
// Initialise
// ---------------------------------------------------------------------------

async function init(): Promise<void> {
  loadPrefs();
  applyTheme();
  installIconSprite(currentTheme);
  if (isMac) document.documentElement.classList.add("platform-mac");
  void setupWindowButtons();

  viewEl = $("#view");
  scrollEl = $("#view-scroll");
  crumbsEl = $("#crumbs");
  pathbarEl = $("#pathbar");
  pathEntry = $<HTMLInputElement>("#path-entry");
  searchbarEl = $("#searchbar");
  searchEntry = $<HTMLInputElement>("#search-entry");
  floatingBar = $("#floating-bar");
  shareSubtitle = $("#share-subtitle");
  tabbar = $("#tabbar");
  previewLayer = $("#preview-layer");
  previewName = $("#preview-name");
  previewBody = $("#preview-body");
  $("#preview-close").innerHTML = sym("window-close");
  $("#preview-close").addEventListener("click", closePreview);
  previewLayer.addEventListener("click", (event) => {
    if (event.target === previewLayer) closePreview();
  });

  // Static icons
  $("#btn-search").innerHTML = sym("system-search");
  $("#btn-main-menu").innerHTML = sym("open-menu");
  $("#btn-back").innerHTML = sym("go-previous");
  $("#btn-forward").innerHTML = sym("go-next");
  $("#btn-location-menu").innerHTML = sym("view-more");
  $("#btn-new-folder").innerHTML = sym("folder-new");
  initOperationsButton($("#btn-operations"));
  $("#btn-view-menu").innerHTML = sym("pan-down");
  $("#search-icon").innerHTML = sym("system-search");

  // Headerbar
  $("#btn-back").addEventListener("click", goBack);
  $("#btn-forward").addEventListener("click", goForward);
  $("#btn-search").addEventListener("click", () => (searchQuery === null ? openSearch() : closeSearch()));
  $("#btn-main-menu").addEventListener("click", (e) => openMainMenu(e.currentTarget as HTMLElement));
  $("#btn-location-menu").addEventListener("click", (e) => openLocationMenu(e.currentTarget as HTMLElement));
  $("#btn-new-folder").addEventListener("click", promptNewFolder);
  $("#btn-view-toggle").addEventListener("click", () => setView(prefs.view === "grid" ? "list" : "grid"));
  $("#btn-view-menu").addEventListener("click", (e) => openViewMenu(e.currentTarget as HTMLElement));

  // Path bar: crumbs navigate; clicking the empty part edits the location.
  crumbsEl.addEventListener("click", (e) => {
    const crumb = (e.target as HTMLElement).closest<HTMLElement>(".crumb");
    if (crumb?.dataset.path) {
      if (crumb.dataset.path !== location) navigate(crumb.dataset.path, { select: childOf(crumb.dataset.path, location) });
    } else {
      openPathEntry();
    }
  });
  pathbarEl.addEventListener("click", (e) => {
    if (e.target === pathbarEl) openPathEntry();
  });
  pathEntry.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      submitPathEntry();
    } else if (e.key === "Escape") {
      e.preventDefault();
      closePathEntry();
      viewEl.focus();
    }
  });
  pathEntry.addEventListener("blur", () => setTimeout(closePathEntry, 0));

  // Search
  // Typing a search picks its first match: the user's choice, unlike the
  // reloads that rerun the search.
  searchEntry.addEventListener("input", () => {
    noteSelectionIntent();
    onSearchInput();
  });
  searchEntry.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      closeSearch();
    } else if (e.key === "Enter") {
      e.preventDefault();
      activateSelection();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      viewEl.focus();
      moveCursor(cursorIndex < 0 ? 0 : cursorIndex, false);
    }
  });

  // View
  viewEl.addEventListener("mousedown", onViewMouseDown);
  // Zoom: Ctrl/⌘ + wheel, and the trackpad pinch macOS delivers as gestures.
  viewEl.addEventListener("wheel", onViewWheel, { passive: false });
  viewEl.addEventListener("gesturestart", onGestureStart);
  viewEl.addEventListener("gesturechange", onGestureChange);
  viewEl.addEventListener("gestureend", onGestureEnd);
  viewEl.addEventListener("dblclick", (e) => {
    const i = itemIndexAt(e);
    if ((e.target as HTMLElement).closest(".star")) return;
    if (i >= 0 && !e.shiftKey && !e.metaKey && !e.ctrlKey) activate(shown[i]);
  });
  viewEl.addEventListener("contextmenu", onViewContextMenu);
  viewEl.addEventListener("keydown", onViewKeyDown);
  viewEl.addEventListener("click", (e) => {
    const sortBtn = (e.target as HTMLElement).closest<HTMLElement>("[data-sort]");
    if (!sortBtn) return;
    const key = sortBtn.dataset.sort as SortKey;
    setSort(key, prefs.sortKey === key ? !prefs.sortReversed : false);
  });
  scrollEl.addEventListener("mousedown", (e) => {
    // Clicks below the last item land on the scroller, not the view.
    if (e.target === scrollEl) onViewMouseDown(e);
  });
  let scrollFrame = 0;
  scrollEl.addEventListener("scroll", () => {
    onViewScroll();
    cancelAnimationFrame(scrollFrame);
    scrollFrame = requestAnimationFrame(() => renderWindow());
  });
  new ResizeObserver(() => relayout()).observe(scrollEl);
  scrollEl.addEventListener("contextmenu", (e) => {
    if (e.target === scrollEl) onViewContextMenu(e);
  });
  document.addEventListener("keydown", onGlobalKeyDown);
  // Files saved or created in other apps meanwhile show up when the window
  // comes back. Recent also changes while it is open, so refresh it too.
  window.addEventListener("focus", () => {
    if (hasOpenOverlay() || searchQuery !== null) return;
    if (location === RECENT) void reload();
    else if (!isPseudo(location) && location !== trashPath) void reload();
  });
  // Mouse back/forward buttons
  window.addEventListener("mouseup", (e) => {
    if (e.button === 3) goBack();
    if (e.button === 4) goForward();
  });
  window.addEventListener("blur", closePopover);
  $(".sidebar").addEventListener("contextmenu", onSidebarContextMenu);

  home = await homeDir().catch(() => "");
  // The backend knows the platform's Trash folder; the fallback only matters
  // if the command fails.
  trashPath = await trashDir().catch(() => (home ? joinPath(home, ".Trash") : ""));
  await buildSidebar();
  const requested = new URLSearchParams(window.location.search).get("path");
  const initial =
    requested && (requested.startsWith("/") || isPseudo(requested) || isCloud(requested) || isSharePath(requested))
      ? requested
      : home || "/";
  // Checking received shares waits on every sharer, a connect timeout for
  // each offline one, so only a share needs it before the first view. The
  // emblems on shared folders appear once it is done.
  const shares = refreshShares();
  if (isSharePath(initial)) await shares;
  await navigate(initial, { history: "none" });
  void shares.then(() => renderWindow(true));
  tabs.push(tabSnapshot());
  activeTab = 0;
  renderTabbar();
  initExternalDrops();
  await initAccessBanner();
}

window.addEventListener("DOMContentLoaded", () => {
  init().catch(console.error);
});
