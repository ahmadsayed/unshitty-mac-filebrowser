import { CANCELLED, endOperation, finishItem, initOperationsButton, opTitle, setOperationState, startOperation, updateOperation, type Operation, type ProgressUpdate } from "./operations";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { forgetDragOut, isOwnDragOut, leftWindow, startSystemDrag } from "./drag-out";
import { homeDir, documentDir, downloadDir, audioDir, pictureDir, videoDir, desktopDir, publicDir } from "@tauri-apps/api/path";
import {
  listDir, searchDir, cancelSearch, type SearchSummary, makeDir, renameEntry, renameMany, itemCounts, copyEntry, moveEntry, trashEntries, type TrashOutcome, pathExists,
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
  previewBytes, previewText, thumbnail, arbeskScene, arbeskPart, cancelArbeskParts, arbeskVersions, listVersions, versionCount, openVersion,
  driveAccounts, driveConnectStart, driveConnectFinish, driveConnectCancel, driveDisconnect,
  arbeskAccounts, arbeskConnectStart, arbeskConnectFinish, arbeskConnectCancel, arbeskDisconnect, arbeskSignedIn, arbeskCreateCollection, arbeskShareCollection,
  type ArbeskShared, type ArbeskRole, type ArbeskScene,
  driveClientStatus, driveClientSet, driveClientClear,
  openExternal,
  setPermissions,
  shareIdentity, setShareName, shareFolder, shareInfo, listShares, createShareLink,
  shareLinkFor, shareLinkRequiresPassphrase, removeShareAccess, stopSharing, addReceived, listReceived, removeReceived,
  type Entry, type DriveAccount, type ReceivedShare, type ShareSummary, type ShareInfo,
  type AccessEntry,
} from "./api";
import { appFormats, arbeskAttach, arbeskAttachPlan, arbeskDetach, arbeskExport, arbeskFile, arbeskRepresentations, capabilities, openWith, type AppChoice, type AttachChoice, type Attached, type PlannedFile, type RepresentationInfo } from "./api";
import { askAttachClash, chooseFileToOpen, chooseFilesToCopy, confirmAttach, confirmRemoveFile, iconEntry, showFilesDialog, type ClashChoice, type FilesActions, type FilesPage } from "./representation-dialogs";
import { assetFormat, copyNameAt, copySources, onlyChoice, openWithCandidates, uniqueNames, type AssetFiles, type CopyChoice, type FileChoice } from "./representations";
import { loadCapabilities } from "./capabilities";
import { entryIcon, filesEmblemOf, itemIcon, versionsBadge, fileKind, fileTypeDescription, fileTypeLabel, installIconSprite, isAppBundle, sym, type FileKind, type FolderKind } from "./icons";
import { appIconKey } from "./native-icons";
import { GRID_SIZES, ItemIcons, LIST_SIZES, THUMB_KINDS } from "./item-icons";
import { thumbType } from "./thumbnails";
import { formatDate, formatDateDetailed, formatItemCount, formatSize } from "./format";
import { LAYOUT_SIZES, platformTheme, type Theme } from "./theme";
import { contentHeight, headerTop, indicesInRect, indicesInRows, itemPos as slotPos, revealTop, stepRow, type Section, type ViewLayout } from "./view-layout";
import { applyToAllRow, cleanError, closePopover, dialogLinks, hasOpenOverlay, openDialog, openMenu, openPopover, showToast, type Anchor, type MenuItem } from "./ui";
import { checkForUpdates } from "./updates";
import { addVersionRows, insideVersionedRoot, restoreName, showDeletedFiles, showVersions } from "./versions";
import { LOCAL_VERSION_LIMIT, drawFilesSwitcher, drawVersionControl, filePage, fromArbeskSaves, fromLocal, type PreviewVersion } from "./preview-versions";
import { macDmgItems } from "./mac-dmg";
import { showOpenWith as showOpenWithDialog } from "./open-with";
import { linuxProgramItems } from "./linux-programs";
import { qrShareItems } from "./qr-share";
import { PREVIEW_ACTION_LABELS, previewActions, previewPlace, type PreviewAction } from "./preview-actions";
import { createShareProtection, promptSharePassphrase } from "./share-passphrase";
import { Selection, type Mark } from "./selection";
import { arrangeSearch, fold, SEARCH_LIMIT, SearchSession, searchSections as arrangedSections, type SearchChange, type SearchSection } from "./search";
import { FolderSummaries } from "./folder-summaries";
import { listingReaction, type ListingContext, type ListingPhase } from "./listing-failure";
import type { DriveAuthNeed } from "./drive-auth";
import { PathRefs, type Clip } from "./path-refs";
import { companionPath, gltfCompanions, gltfExternalUris, isGlb, modelFormat, type ModelFormat } from "./models";
import { accountOf, fromArbesk, fromDrive, PROVIDERS, providerOf, withAccount, withoutAccount, type Account, type ProviderActions, type ProviderId } from "./providers";
import { showOnlineAccounts, type OnlineAccountsHost } from "./online-accounts";
import { LatestSave } from "./latest-save";
import { fetchParts } from "./arbesk-parts";
import { acceptsNewItems, arbeskPasteRefusal, attachRefusal, takesFiles, arbeskTargetName, copyOutRefusal, isInTrash, isPseudo, hasLocalFileFacts, knowsSizes, takesDrops, locationKind, mayHaveVersions, previewsRemotely, PSEUDO_PREFIX, searchesRecursively, type LocationKind, type Pseudo } from "./locations";
import { allows, refusal, type Here, type SharedAction } from "./item-actions";
import { childPath, cloudAccountKey, homeFolder, isArbeskAccount, isArbeskAsset, isArbeskCollection, arbeskFilePin, isCloudPath as isCloud, isKnownPath, isRemotePath, isShareLink, isUnder, isWithin, localSegments, parentPath, pathName, pathSegments, placeOf, shareId, shareTicket, resolveTypedPath, safeLocalName, SHARE_PREFIX, isSharePath, isSftpPath, sftpAuthority, type Place } from "./paths";

// ---------------------------------------------------------------------------
// Preferences (persisted per machine; storage may be unavailable)
// ---------------------------------------------------------------------------

type ViewMode = "grid" | "list";
type SortKey = "name" | "size" | "modified" | "type";

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
const PSEUDO: Record<Pseudo, { label: string; icon: string; title: string; text: string; action?: string }> = {
  recent: { label: "Recent", icon: "document-open-recent", title: "No Recent Files", text: "" },
  starred: { label: "Starred", icon: "starred", title: "Starred files will appear here", text: "" },
  network: { label: "Network", icon: "network-workgroup", title: "No Network Locations", text: "Connect to a server, paste a share link or connect an online account.", action: "Online Accounts…" },
};
const RECENT = `${PSEUDO_PREFIX}recent`;
const STARRED = `${PSEUDO_PREFIX}starred`;
const NETWORK = `${PSEUDO_PREFIX}network`;

/** Current location: an absolute path, or "pseudo:<name>". */
let location = "";
/** Goes up each time navigate() shows a location: a reload asked for before
 *  drops what it learns after that, like the other late answers (#154). */
let viewSeq = 0;
/** Nearest folder that keeps versions, if the current location is in one. */
let versionedRootPath: string | null = null;
let entries: Entry[] = [];
/** Entries in display order (sorted + search-filtered). */
let shown: Entry[] = [];
let listingError: string | null = null;

/** The selected items, cursor and anchor (src/selection.ts); its indices are
 *  positions in `shown`. A copy or move selects what it made only when the
 *  user hasn't chosen anything since pasting (`selection.intent`, #50). */
const selection = new Selection<Entry>(() => shown);
/** The search: open or not, its text, the walk and its results (src/search.ts). */
const search = new SearchSession(
  (at, query, limit, onFound) => searchDir(at, query, prefs.showHidden, limit, onFound),
  cancelSearch,
  onSearchChange,
);
/** In a recursive search, the index in `shown` of the first "In Subfolders"
 * item; the ones before it are "In This Folder". -1 elsewhere. */
let sectionStart = -1;

/** Folder item counts and version badges for the view (src/folder-summaries.ts). */
const summaries = new FolderSummaries(itemCounts, () => location, ({ badges, recounted }) => {
  badges.forEach(patchIcon);
  patchCounts();
  itemIcons.recounted(recounted);
});
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
  /** The search's text, null when it was closed. */
  search: string | null;
  scrollTop: number;
}

let tabs: TabState[] = [];
let activeTab = -1;
/** Serialises tab switches so two async restores can't race. */
let tabBusy = false;
/** The tab being switched to while its folder is listed: a navigation that
 *  shows before it does shows in this tab, on top of its history (#154). */
let arrivingTab: TabState | null = null;

/** Files copied or cut, waiting to be pasted. */
let clipboard: Clip | null = null;

/** Starred files and folders. */
let starred = new Set<string>();

/** Folders the user added to the sidebar, after GNOME's default bookmarks. */
let pins: string[] = [];
/** Paths the sidebar always shows (places, default bookmarks, volumes). */
const fixedSidebarPaths = new Set<string>();

/** starred.json and pins.json: one write each at a time, the last with the
 *  latest stars and bookmarks (src/latest-save.ts, #172). Every change to
 *  them changes the state first and saves it after, so a save never writes
 *  a list from before another change. */
const starSaves = new LatestSave(() => [...starred], saveStarred, (err) =>
  showToast(`Could not save starred files: ${cleanError(err)}`)
);
const pinSaves = new LatestSave(() => [...pins], savePins, (err) =>
  showToast(`Could not save bookmarks: ${cleanError(err)}`)
);

/** What remembers paths (stars, bookmarks, the clipboard, tabs and their
 *  history) follows moved, renamed and trashed items through this
 *  (src/path-refs.ts). */
const pathRefs = new PathRefs<TabState>({
  stars: {
    get: () => starred,
    set: (next) => {
      starred = next;
      void starSaves.save();
    },
  },
  bookmarks: {
    get: () => pins,
    // The folder moved already: the sidebar shows where it is now, saved or not.
    set: (next) => void setPins(next),
  },
  clipboard: {
    get: () => clipboard,
    set: (next) => {
      clipboard = next;
    },
  },
  tabs: {
    // While a tab is switched to, a move that lands meanwhile follows both
    // the history it brings, which the view takes as its folder shows, and
    // the view's own, which stays if the switch fails (#172).
    get: () => ({ list: tabs, active: activeTab, back: backStack, forward: forwardStack, arriving: arrivingTab }),
    set: (next) => {
      tabs = next.list;
      backStack = next.back;
      forwardStack = next.forward;
      if (arrivingTab && next.arriving) {
        // In place: the switch holds this tab and takes its history from it.
        arrivingTab.back = next.arriving.back;
        arrivingTab.forward = next.arriving.forward;
      }
      renderTabbar();
    },
  },
});

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
/** Connected online accounts (src/providers.ts), which Network lists. */
let accounts: Account[] = [];
/** Accounts whose last request failed for want of a sign-in, by `provider:id`. */
const signInNeeded = new Set<string>();
const accountRef = (account: Account) => `${account.provider}:${account.id}`;
let cloudOpenNoticeShown = false;
/** Mounted volumes, as the backend reported them; used for breadcrumbs and
 * for guessing whether a drag crosses filesystems. */
let mountedVolumes: { name: string; path: string }[] = [];
const folderKinds = new Map<string, FolderKind>();

const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);
const currentTheme: Theme = platformTheme(isMac);
const MOD = isMac ? "⌘" : "Ctrl+";

/** Which picture each item shows, and when it's asked for (src/item-icons.ts). */
const itemIcons = new ItemIcons({
  followsContents: isMac,
  location: () => location,
  view: () => prefs,
  folderKind,
  compare: compareEntries,
  redraw: patchIcon,
});

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
let previewActionsEl: HTMLElement;
let previewVersionsEl: HTMLElement;
/** The preview header's switcher between an Arbesk save's model and its other files. */
let previewFilesEl: HTMLElement;

// SECURITY: escape HTML entities in user-derived strings before innerHTML insertion
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** What a path with no name of its own is called: a scheme's root. */
const UNNAMED: Record<Place, string> = { drive: "My Drive", share: "Shared Folder", sftp: "SSH Server", arbesk: "Arbesk", local: "/" };

function basename(path: string): string {
  return pathName(path) || UNNAMED[placeOf(path)];
}

/** Above a share's or a server's root, and above My Drive (the account's
 *  root folder, Network's entry), is Network. */
function parentOf(path: string): string {
  if (accounts.some((account) => account.rootPath === path) || (isCloud(path) && parentPath(path) === path)) return NETWORK;
  return parentPath(path) ?? NETWORK;
}

function receivedFor(path: string): ReceivedShare | undefined {
  return networkShares.find((share) => share.id === shareId(path));
}

/** What Network lists: one folder entry per received share, then one per
 *  connected server, then one per online account. */
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
  const online: Entry[] = accounts.map((account) => ({ ...place, name: PROVIDERS[account.provider].label, path: account.rootPath }));
  return [...shares, ...servers, ...online];
}

/** The SSH server a Network entry stands for. */
function serverFor(entry: Entry): SftpServer | undefined {
  return isSftpPath(entry.path) ? sftpServerList.find((server) => server.url === entry.path) : undefined;
}

/** The Network entry that stands for `path`: on an SSH server, the server's
 *  entry, whose path is where the server starts (the home on it, or the
 *  path typed), not its root. Any other path stands for itself. */
function networkEntryFor(path: string): string {
  if (!isSftpPath(path)) return path;
  const authority = sftpAuthority(path);
  return sftpServerList.find((server) => sftpAuthority(server.url) === authority)?.url ?? path;
}

/** What to select on arriving in `loc`: in Network, a server's root
 *  selects the server (Up from its root). */
function selectionOn(loc: string, select: string | undefined): string | undefined {
  return loc === NETWORK && select ? networkEntryFor(select) : select;
}

/** Network's second line: who shared a folder, what kind of server it is, or the account's address. */
function networkDetail(entry: Entry): string {
  if (isSftpPath(entry.path)) return "SSH server";
  const account = accountOf(entry.path, accounts);
  if (account) return account.email;
  return entry.sharer ? `Shared by ${entry.sharer}` : "";
}

function networkStatus(entry: Entry): { cls: string; label: string } {
  if (isSftpPath(entry.path) || providerOf(entry.path)) return { cls: "online", label: "Connected" };
  return { cls: entry.shareStatus ?? "", label: shareStatusLabel(entry.shareStatus) };
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

/** The path of `name` inside `dir`. In Drive it is a pending `~name` path. */
function joinPath(dir: string, name: string): string {
  return childPath(dir, name);
}

/** Pseudo locations that list entries (Recent, Starred, Network). */
function hasEntries(loc: string): boolean {
  return !isPseudo(loc) || loc === RECENT || loc === STARRED || loc === NETWORK;
}

/** Views of items from many folders, which get a Location column. */
function showsWhere(): boolean {
  return location === RECENT || location === STARRED || location === trashPath || search.isOpen;
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
    return pathSegments(loc).segments.length === 0 ? receivedFor(loc)?.folder || basename(loc) : basename(loc);
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
  if (e.is_dir) return summaries.count(e.path) ?? -1;
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

/** What `shown` was last built from: a presentation-only render (grid ↔ list,
 * thumbnails) reuses it instead of sorting a huge folder again. */
let shownFrom: unknown[] = [];

function computeShown(): void {
  const { text, results } = search;
  // Keyed on the folded text: "Cafe" and "café " show the same items.
  const from = [entries, results, fold(text.trim()), location, prefs.sortKey, prefs.sortReversed, prefs.sortKey === "size" && summaries.version];
  if (from.every((value, i) => value === shownFrom[i])) return;
  shownFrom = from;
  ({ items: shown, sectionStart } = arrangeSearch(entries, results, text, location, compareEntries, canSearchRecursively()));
}

/** The headed sections of a recursive search; none elsewhere. */
function searchSections(): SearchSection[] {
  return arrangedSections(sectionStart, shown.length, search.busy);
}

/** Whether item `i` is one of "In This Folder", which needs no folder line. */
function inThisFolderSection(i: number): boolean {
  return i < sectionStart;
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

function tabSnapshot(): TabState {
  return {
    location,
    back: [...backStack],
    forward: [...forwardStack],
    selected: selection.paths(),
    anchorIndex: selection.anchor,
    cursorIndex: selection.cursor,
    search: search.saved,
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

/** Restore a saved tab into the live view; false when its folder can't be listed.
 *  Its history comes back as its folder shows (`showLocation`). */
async function restoreTab(tab: TabState): Promise<boolean> {
  let went: Navigated;
  try {
    went = await navigate(tab.location, { history: "none", tab });
  } finally {
    if (arrivingTab === tab) arrivingTab = null;
  }
  if (went === "kept") return false;
  // A later navigation showed first: it shows in this tab instead.
  if (went === "overtaken") return true;
  if (tab.search !== null) {
    // navigate() closed the search; open it again the way the search button
    // does, so the placeholder names this folder and the recursive search runs.
    showSearchBar();
    searchEntry.value = tab.search;
    onSearchInput();
  }
  selection.retain(tab.selected, entries);
  selection.place(tab.anchorIndex, tab.cursorIndex);
  updateSelection();
  updateNavButtons();
  renderTabbar();
  scrollEl.scrollTop = tab.scrollTop;
  return true;
}

async function switchTab(index: number): Promise<void> {
  if (tabBusy || index === activeTab || index < 0 || index >= tabs.length) return;
  tabBusy = true;
  try {
    saveActiveTab();
    const previous = activeTab;
    activeTab = index;
    if (await restoreTab(tabs[index])) return;
    // navigate() showed a toast and left the view, but closed its search:
    // the previous tab stays active and comes back as it was saved.
    activeTab = previous;
    await restoreTab(tabs[previous]);
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
    search: null,
    scrollTop: 0,
  });
  activeTab = tabs.length - 1;
  if (search.isOpen) closeSearch(false);
  backStack = [];
  forwardStack = [];
  selection.reset();
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

/** How a navigation moves along the history once it shows: a new place
 *  ("push"), one step "back" or "forward", or not at all ("none"). */
type HistoryMove = "push" | "back" | "forward" | "none";

interface NavigateOptions {
  history?: HistoryMove;
  select?: string;
  /** A tab switch's navigation: only a later navigation that shows overtakes
   *  it, so the tab still shows when that one fails. */
  tab?: TabState;
}

/** What became of a navigate(): "shown" it went there; "kept" the folder
 *  couldn't be listed, so the view stayed and a toast said why; "overtaken"
 *  a later navigation started (for a tab switch: showed) before this one
 *  could show (#154). */
type Navigated = "shown" | "kept" | "overtaken";

/** What navigate() found at a location: its items, or the error or the
 *  share status page to show instead. */
interface Arrival {
  next: Entry[];
  error: string | null;
  page: "offline" | "no-access" | "missing" | null;
  /** The item to select there: in Network, the entry for the wanted path. */
  select: string | undefined;
}

/** Goes up with each navigate(): only the latest one shows its location. */
let navigationSeq = 0;

/**
 * Go to `loc`. Listing happens before anything changes, so a folder that
 * can't be read leaves the current view intact and shows a toast instead.
 * The latest navigation wins: one that a later one overtook while it
 * waited changes nothing, whichever answer lands first.
 */
async function navigate(loc: string, opts: NavigateOptions = {}): Promise<Navigated> {
  const seq = ++navigationSeq;
  const view = viewSeq;
  const overtaken = opts.tab ? () => view !== viewSeq : () => seq !== navigationSeq;
  if (opts.tab) arrivingTab = opts.tab;
  if (search.isOpen) closeSearch(false);
  closePreview();

  // Started now so it runs alongside the listing instead of after it.
  const rootLookup = lookupVersionedRoot(loc);
  const arrival = await arrive(loc, overtaken, opts.select);
  if (overtaken()) return "overtaken";
  if (!arrival) return "kept";
  await itemIcons.listingStarted(loc, arrival.next);
  const root = await rootLookup;
  if (overtaken()) return "overtaken";
  showLocation(loc, arrival, root, opts);
  return "shown";
}

/** List `loc` for navigate(). Null when the view keeps its place: a toast
 *  said why, or no toast because a later navigation overtook this one. */
async function arrive(loc: string, overtaken: () => boolean, wanted: string | undefined): Promise<Arrival | null> {
  if (loc === NETWORK) await Promise.all([refreshShares(), refreshSftpServers(), refreshAccounts()]);
  // Asked after the refresh: the server list says which entry is whose.
  const arrival: Arrival = { next: [], error: null, page: null, select: selectionOn(loc, wanted) };
  const shared = isSharePath(loc) ? receivedFor(loc) : undefined;
  if (shared && shared.status !== "online") {
    arrival.page = shared.status;
  } else if (hasEntries(loc)) {
    try {
      arrival.next = await listLocation(loc);
    } catch (err) {
      if (overtaken()) return null;
      const failed = listingReaction(loc, err, listingContext(loc, location ? "navigate" : "first"));
      if (failed.driveAuth) void offerDriveSignIn(failed.driveAuth, loc);
      if (failed.kind === "toast") {
        if (!failed.driveAuth) showListingToast(failed);
        return null;
      }
      if (failed.kind === "sharePage") arrival.page = failed.status;
      else arrival.error = failed.message;
    }
  }
  return arrival;
}

/** navigate()'s last step, with nothing left to wait for: the view shows `loc`. */
function showLocation(loc: string, arrival: Arrival, root: string | null, opts: NavigateOptions): void {
  const from = arrivingTab ? takeTabHistory(arrivingTab) : location;
  moveInHistory(loc, from, opts.history ?? "push");
  location = loc;
  viewSeq++;
  sharePage = arrival.page;
  versionedRootPath = root;
  entries = arrival.next;
  listingError = arrival.error;
  summaries.reset();
  selection.reset(arrival.select);
  if (search.isOpen) moveSearchHere();

  document.title = locationName(loc);
  if (activeTab >= 0 && tabs[activeTab]) tabs[activeTab].location = loc;
  renderTabbar();
  updateNavButtons();
  updateSidebarSelection();
  buildPathbar();
  updateShareSubtitle();
  renderView();
  scrollEl.scrollTop = 0;
  if (arrival.select) revealPath(arrival.select);
  viewEl.focus({ preventScroll: true });
}

/** The tab being switched to shows now, or a later navigation does in its
 *  place: the view takes the tab's history first. Returns where the tab was. */
function takeTabHistory(tab: TabState): string {
  backStack = [...tab.back];
  forwardStack = [...tab.forward];
  arrivingTab = null;
  return tab.location;
}

/** Move along the history from `from` as a navigation to `loc` shows. Back
 *  and Forward take their step only then: one that fails or is overtaken
 *  leaves the history as it was. */
function moveInHistory(loc: string, from: string, move: HistoryMove): void {
  if (move === "back") {
    backStack.pop();
    forwardStack.push(from);
  } else if (move === "forward") {
    forwardStack.pop();
    backStack.push(from);
  } else if (move === "push" && from && loc !== from) {
    backStack.push(from);
    forwardStack = [];
  }
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
  // The view may go somewhere else while this waits: what lands after that is dropped.
  const seq = viewSeq;
  const moved = () => seq !== viewSeq;
  if (isSharePath(location) || location === NETWORK) await refreshShares();
  if (location === NETWORK) await Promise.all([refreshSftpServers(), refreshAccounts()]);
  if (moved() || showShareStatusPage()) return;
  if (!(await relist(moved))) return;
  selection.retain(select ?? selection.paths(), entries);
  const mark = selection.mark();
  const top = scrollEl.scrollTop;
  renderView();
  scrollEl.scrollTop = top;
  if (select?.[0]) revealPath(select[0]);
  else followCursor(mark);
  const root = await lookupVersionedRoot(location);
  if (moved()) return;
  versionedRootPath = root;
  updateShareSubtitle();
  search.rerun();
}

/** List the location again for reload(). False when there is nothing more
 *  for it to do: the view moved on meanwhile, or went to the parent instead. */
async function relist(moved: () => boolean): Promise<boolean> {
  try {
    const listed = await listLocation(location);
    if (moved()) return false;
    entries = listed;
    listingError = null;
    sharePage = null;
    itemIcons.listingReloaded();
    summaries.reloaded();
    return true;
  } catch (err) {
    return !moved() && !(await listingFailed(err));
  }
}

/** After a reload, keep the cursor and anchor on the same items: new ones
 *  may have landed above them. */
function followCursor(mark: Mark): void {
  selection.follow(mark, true);
  updateSelection();
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
  const failed = listingReaction(location, err, listingContext(location, "reload"));
  if (failed.driveAuth) void offerDriveSignIn(failed.driveAuth, location);
  if (failed.kind === "sharePage") {
    sharePage = failed.status;
    return false;
  }
  // A folder that went away goes to its parent, or shows the error page if that fails too.
  if (failed.kind === "goTo" && (await navigate(failed.parent, { history: "none" })) !== "kept") return true;
  listingError = failed.message;
  return false;
}

/** What listingReaction needs to know about `loc` (src/listing-failure.ts). */
function listingContext(loc: string, phase: ListingPhase): ListingContext {
  return { phase, isMac, share: isSharePath(loc) ? receivedFor(loc)?.status : undefined, nameOf: basename, parentOf };
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

/** Refresh the connected online accounts, which Network lists. */
async function refreshAccounts(): Promise<void> {
  const [drive, arbesk] = await Promise.all([
    driveAccounts().then((list) => list.map(fromDrive)).catch(() => null),
    arbeskAccounts().then((list) => list.map(fromArbesk)).catch(() => null),
  ]);
  // A failed answer keeps what is known of that provider rather than emptying Network.
  const known = (provider: ProviderId, fresh: Account[] | null) => fresh ?? accounts.filter((a) => a.provider === provider);
  accounts = [...known("gdrive", drive), ...known("arbesk", arbesk)];
  for (const ref of [...signInNeeded]) {
    if (!accounts.some((account) => accountRef(account) === ref)) signInNeeded.delete(ref);
  }
}

/** The folder that keeps versions for `loc`, if any (never for Trash or Recent). */
function lookupVersionedRoot(loc: string): Promise<string | null> {
  if (!mayHaveVersions(loc, trashPath)) return Promise.resolve(null);
  return versionedRoot(loc).catch(() => null);
}

/** A failed folder open; macOS privacy blocks get a shortcut to the setting. */
function showListingToast(failed: { text: string; grantAccess: boolean }): void {
  showToast(failed.text, failed.grantAccess ? { button: { label: "Grant Access", onClick: openSettingsSafely } } : {});
}

function openSettingsSafely(): void {
  openFullDiskAccessSettings().catch((err) => showToast(`Could not open System Settings: ${cleanError(err)}`));
}

/** The history Back and Forward step along: the tab being switched to brings its own. */
function historyNow(): { back: string[]; forward: string[]; at: string } {
  const tab = arrivingTab;
  return tab ? { back: tab.back, forward: tab.forward, at: tab.location } : { back: backStack, forward: forwardStack, at: location };
}

function goBack(): void {
  const { back, at } = historyNow();
  const prev = back[back.length - 1];
  if (prev) void navigate(prev, { history: "back", select: childOf(prev, at) });
}

function goForward(): void {
  const { forward } = historyNow();
  const next = forward[forward.length - 1];
  if (next) void navigate(next, { history: "forward" });
}

function goUp(): void {
  if (isPseudo(location) || location === "/") return;
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
  const place = placeOf(loc);
  if (place === "drive") return driveCrumbs(loc);
  if (place === "arbesk") return arbeskCrumbs(loc);
  if (place !== "local") return remoteCrumbs(loc, place);
  if (isInTrash(loc, trashPath)) {
    return [{ label: "Trash", path: trashPath, icon: "user-trash" }, ...segments(trashPath, loc)];
  }
  if (home && isWithin(loc, home)) {
    return [{ label: "Home", path: home, icon: "user-home" }, ...segments(home, loc)];
  }
  const volume = mountedVolumes
    .filter((v) => isWithin(loc, v.path))
    .sort((a, b) => b.path.length - a.path.length)[0];
  if (volume) {
    return [{ label: volume.name, path: volume.path, icon: "drive-harddisk" }, ...segments(volume.path, loc)];
  }
  return [{ label: "", path: "/", icon: "drive-harddisk" }, ...segments("/", loc)];
}

/** Drive's crumbs: one per folder from My Drive down; the account alone is "Google Drive". */
function driveCrumbs(loc: string): Crumb[] {
  const { segments } = pathSegments(loc);
  if (!segments.length) return [{ label: "Google Drive", path: loc, icon: "network-workgroup" }];
  const crumbs: Crumb[] = segments.map((s) => ({ label: s.name || "My Drive", path: s.path }));
  crumbs[0] = { ...crumbs[0], icon: "network-workgroup" };
  return crumbs;
}

/** Arbesk's crumbs: the account is "Arbesk", then the collection and the asset. */
function arbeskCrumbs(loc: string): Crumb[] {
  const { root, segments } = pathSegments(loc);
  return [{ label: "Arbesk", path: root.path, icon: "network-workgroup" }, ...segments.map((s) => ({ label: s.name, path: s.path }))];
}

/** A share's or a server's crumbs: its root (the sharer's folder name, or the server), then its folders. */
function remoteCrumbs(loc: string, place: "share" | "sftp"): Crumb[] {
  const { root, segments } = pathSegments(loc);
  if (!root.key) return [{ label: "Network", path: NETWORK, icon: "network-workgroup" }];
  const label = place === "share" ? receivedFor(loc)?.folder || "Shared Folder" : root.name;
  return [{ label, path: root.path, icon: "network-workgroup" }, ...segments.map((s) => ({ label: s.name, path: s.path }))];
}

function segments(base: string, loc: string): Crumb[] {
  return localSegments(base, loc).map((s) => ({ label: s.name, path: s.path }));
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
  if (search.isOpen) closeSearch(false);
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
  // Going there waits for the check: a navigation started meanwhile (a click
  // in the sidebar) is the later one, so the typed path is dropped, silently.
  const seq = navigationSeq;
  const overtaken = () => seq !== navigationSeq;
  try {
    await listDir(typed, prefs.showHidden);
  } catch (err) {
    if (overtaken()) return;
    const failed = listingReaction(typed, err, listingContext(typed, "entry"));
    if (failed.kind === "connect") {
      closePathEntry();
      showConnectServerDialog(failed.url);
      return;
    }
    if (failed.kind === "toast") {
      if (failed.driveAuth) void offerDriveSignIn(failed.driveAuth, typed);
      else showListingToast(failed);
    }
    pathEntry.focus();
    return;
  }
  if (overtaken()) return;
  closePathEntry();
  await navigate(typed);
}

// ---------------------------------------------------------------------------
// Search (filters the current folder; replaces the path bar)
// ---------------------------------------------------------------------------

function openSearch(initial = ""): void {
  if (!hasEntries(location)) return;
  closePathEntry();
  if (!search.isOpen) showSearchBar();
  if (initial) searchEntry.value = initial;
  searchEntry.focus();
  onSearchInput();
}

/** Put the search bar in the path bar's place, for the current folder. */
function showSearchBar(): void {
  search.open(location, canSearchRecursively());
  pathbarEl.hidden = true;
  searchbarEl.hidden = false;
  $("#btn-search").classList.add("open");
  searchEntry.placeholder = `Search ${locationName(location)}`;
}

/** A key typed while a navigation waited opened the search on the folder
 *  being left; once the view has moved, the search goes with it. */
function moveSearchHere(): void {
  searchEntry.placeholder = `Search ${locationName(location)}`;
  search.moveTo(location, canSearchRecursively());
}

function closeSearch(rerender = true): void {
  if (!search.isOpen) return;
  search.close();
  showSearchBusy();
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
  return searchesRecursively(location, trashPath);
}

function selectFirstMatch(): void {
  if (!search.text || shown.length === 0) return;
  selection.selectFirst();
  updateSelection();
}

function onSearchInput(): void {
  search.setText(searchEntry.value);
}

/** Draw what the search says has changed. The selection rules live here:
 *  a new text selects its first match, as GNOME Files does while the
 *  search runs, and results arriving later don't move the cursor. */
function onSearchChange(change: SearchChange, error?: unknown): void {
  showSearchBusy();
  switch (change) {
    case "restarted":
      selection.dropCursor();
      showSearchStop(null);
      renderView();
      selectFirstMatch();
      break;
    case "grew":
      showSearchResults();
      break;
    case "ended":
      renderViewKeepingScroll(); // "Searching…" becomes "No Results Found"
      showSearchStop(search.stop);
      break;
    case "failed":
      renderView();
      showToast(`Could not search: ${cleanError(error)}`);
      break;
  }
}

/** Show streamed results. "In This Folder" is already shown and never
 *  moves: later results keep the cursor and anchor on the same items as the
 *  list grows, and only an empty view gets its first match selected. */
function showSearchResults(): void {
  const hadItems = shown.length > 0;
  if (!hadItems) {
    renderView();
    selectFirstMatch();
    return;
  }
  const mark = selection.mark();
  renderViewKeepingScroll();
  selection.follow(mark, false);
  updateSelection();
}

function showSearchBusy(): void {
  searchbarEl.classList.toggle("busy", search.busy);
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

/** Whether `entry`'s size is known: not for Arbesk's assets until they are built. */
function sizeKnown(entry: Entry): boolean {
  return knowsSizes(locationKind(entry.path, trashPath));
}

function sizeText(entry: Entry): string {
  if (!entry.is_dir) return sizeKnown(entry) ? formatSize(entry.size) : "";
  const n = summaries.count(entry.path);
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
  } else if (pseudo && (!hasEntries(location) || (shown.length === 0 && !search.text))) {
    viewEl.innerHTML = statusPage(pseudo.icon, pseudo.title, pseudo.text, pseudo.action);
    // Network's only action: the dialog that adds an online account.
    viewEl.querySelector(".status-action")?.addEventListener("click", () => void openOnlineAccounts());
  } else if (listingError !== null) {
    viewEl.innerHTML = statusPage("dialog-error", "Unable to Access Location", listingError);
  } else if (shown.length === 0 && search.text && search.busy) {
    viewEl.innerHTML = statusPage("system-search", "Searching…", "");
  } else if (shown.length === 0 && search.text) {
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
    if (prefs.sortKey === "size") void requestCounts(shown.filter((e) => e.is_dir).map((e) => e.path), true);
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
  const attempt = async (trust: boolean) => {
    if (dialog.busy) return;
    const server = serverInput.value.trim();
    if (!server) {
      message.textContent = "Enter a server, for example user@host.";
      serverInput.focus();
      return;
    }
    const refused = await dialog.run(() => connectOnce(server, trust), "Connecting…");
    if (refused) passwordInput.focus();
  };
  /** One connection attempt; true when the server refused the key or password. */
  const connectOnce = async (server: string, trust: boolean): Promise<boolean> => {
    try {
      const outcome = await sftpConnect(server, passwordInput.value || undefined, trust);
      if (outcome.kind === "connected") {
        dialog.close();
        await refreshSftpServers();
        await navigate(outcome.server.url);
        showToast(`Connected to ${outcome.server.label}`);
        return false;
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
        return false;
      }
      message.textContent = passwordInput.value
        ? "Authentication failed. Check your username and password."
        : "No SSH key was accepted. Enter your password.";
      return true;
    } catch (err) {
      message.textContent = cleanError(err);
      return false;
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
    // Escape and Cancel close it while it connects; the connection still goes ahead.
    busy: { liveWhileBusy: ["cancel"], status: message },
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
  input.placeholder = "Server or share link";
  input.spellcheck = false;
  input.setAttribute("aria-label", "Server address or share link");
  const button = document.createElement("button");
  button.className = "btn suggested";
  button.textContent = "Connect";
  // GNOME Files' network address bar: the entry, Connect, and a round info
  // button for what can be typed. Online accounts are added from Online
  // Accounts…, never from a button per provider here.
  const info = document.createElement("button");
  info.className = "flat-btn circular network-info";
  info.title = "Server Address Information";
  info.setAttribute("aria-label", "Server Address Information");
  info.setAttribute("aria-haspopup", "dialog");
  info.innerHTML = sym("info-outline");
  const error = document.createElement("span");
  error.className = "network-error";
  bar.append(input, button, info, error);
  info.addEventListener("click", () => openPopover(info, serverAddressHelp(), {}));

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

/** What Network's address entry accepts, one row per kind of address. */
const SERVER_PROTOCOLS: Array<{ name: string; prefix: string }> = [
  { name: "SSH File Transfer Protocol", prefix: "sftp:// or ssh://" },
  { name: "Shared folder link", prefix: SHARE_PREFIX },
];

/** The info button's popover, as GNOME Files' "Server Addresses". */
function serverAddressHelp(): HTMLElement {
  const box = document.createElement("div");
  box.className = "server-help";
  box.innerHTML =
    `<h3>Server Addresses</h3>` +
    `<p>Server addresses are made up of a protocol prefix and an address. Examples:</p>` +
    `<p class="server-help-examples">ssh://192.168.0.1, sftp://files.example.org</p>` +
    `<div class="server-help-protocols"><span class="heading">Available Protocols</span><span class="heading">Prefix</span>` +
    SERVER_PROTOCOLS.map((p) => `<span>${escapeHtml(p.name)}</span><span class="prefix">${escapeHtml(p.prefix)}</span>`).join("") +
    `</div>`;
  return box;
}

function statusPage(icon: string, title: string, text: string, action = ""): string {
  const button = action ? `<button class="btn pill status-action">${escapeHtml(action)}</button>` : "";
  return `<div class="status-page">${sym(icon)}<h1>${escapeHtml(title)}</h1>${text ? `<p>${escapeHtml(text)}</p>` : ""}${button}</div>`;
}

function itemClasses(e: Entry, i: number): string {
  const cut = clipboard?.mode === "cut" && clipboard.paths.includes(e.path);
  const offline = e.shareStatus === "offline" || e.shareStatus === "no-access" || e.shareStatus === "missing";
  return (e.is_hidden ? " hidden-entry" : "") + (cut ? " cut" : "") + (offline ? " offline" : "") +
    (selection.has(e.path) ? " selected" : "") + (i === selection.cursor ? " cursor" : "") +
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

function itemHtml(i: number, stars: boolean): string {
  const e = shown[i];
  const { x, y } = itemPos(i);
  const pos = `style="left:${x}px;top:${y}px"`;
  const cls = itemClasses(e, i);
  const sel = selection.has(e.path);
  if (prefs.view === "grid") {
    const detail = location === NETWORK ? networkDetail(e) : "";
    const shared = detail ? `<div class="shared-by">${escapeHtml(detail)}</div>`
      : inThisFolderSection(i) ? `<div class="shared-by where"></div>`
      : search.isOpen ? `<div class="shared-by where" title="${escapeHtml(parentOf(e.path))}">${escapeHtml(whereText(e.path))}</div>` : "";
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
  const from = location === trashPath ? e.origin : inThisFolderSection(i) ? undefined : e.path;
  const where = !showsWhere() ? "" : from
    ? `<span class="meta where" title="${escapeHtml(parentOf(from))}">${escapeHtml(whereText(from))}</span>`
    : `<span class="meta where"></span>`;
  return `<div class="list-row item${cls}${where ? " with-where" : ""}" data-index="${i}" role="option" aria-selected="${sel}" ${pos}>` +
    `<div class="name-cell"><div class="icon">${iconHtml(e)}</div><span class="name">${escapeHtml(e.name)}</span></div>` +
    `<span class="meta size">${sizeText(e)}</span>${where}` +
    `<span class="meta" title="${escapeHtml(formatDateDetailed(date))}">${formatDate(date)}</span>${starButton(e, stars)}</div>`;
}

/** The star at the end of a list row, where `stars` allows it: shown on hover, or always once starred. */
function starButton(e: Entry, stars: boolean): string {
  if (!stars) return "<span></span>";
  const on = starred.has(e.path);
  return `<button class="star${on ? " starred" : ""}" tabindex="-1" title="${on ? "Unstar" : "Star"}" aria-label="${on ? "Unstar" : "Star"}">${sym(on ? "starred" : "non-starred")}</button>`;
}

/** The "Location" column: the folder holding `path`, relative to Home where possible. */
function whereText(path: string): string {
  const dir = parentOf(path);
  if (dir === home) return "Home";
  if (home && isWithin(dir, home)) return dir.slice(home.length + 1);
  return dir;
}

// ---------------------------------------------------------------------------
// Item icons: src/item-icons.ts picks the picture, this draws it
// ---------------------------------------------------------------------------

/** An item's icon, with the emblems icons.ts puts on it. */
function iconHtml(e: Entry): string {
  return itemIcon(e, itemIcons.base(e), {
    link: e.is_symlink,
    versions: summaries.versioned(e.path),
    shared: location === NETWORK || sharedFolders.some((share) => share.path === e.path),
    files: filesEmblemOf(e),
  });
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

let layout: ViewLayout = {
  cols: 1, itemW: 0, itemH: 0, gap: 0, padX: 0, padY: 0, bodyTop: 0,
  count: 0, sections: [{ start: 0, header: false }], headerH: 0,
};
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
    const extraLine = location === NETWORK || search.isOpen;
    const itemH = Math.ceil(6 + icon + 6 + 2 * lineH + 9 + (extraLine ? lineH : 0));
    layout = { cols, itemW, itemH, gap: sizes.gridGap, padX: sizes.gridPad, padY: sizes.gridPad, bodyTop: 0, ...viewSections() };
  } else {
    const itemH = Math.max(LIST_SIZES[prefs.listZoom] + sizes.listPad, sizes.listMinRow);
    layout = { cols: 1, itemW: width - 12, itemH, gap: 0, padX: 6, padY: 0, bodyTop: sizes.listHeader, ...viewSections() };
  }
}

/** The sections `shown` is drawn in: one without a header outside search. */
function viewSections(): { count: number; sections: Section[]; headerH: number } {
  const headed = searchSections();
  if (!headed.length) return { count: shown.length, sections: [{ start: 0, header: false }], headerH: 0 };
  return {
    count: shown.length,
    sections: headed.map((s) => ({ start: s.start, header: true })),
    headerH: LAYOUT_SIZES[currentTheme].sectionHeader,
  };
}

/** A section's header row, drawn in the items layer like an item. */
function sectionHeaderHtml(s: SearchSection, k: number): string {
  const style = `top:${headerTop(layout, k)}px;left:${layout.padX}px;right:${layout.padX}px;height:${layout.headerH}px`;
  const busy = s.busy ? `<span class="searching">Searching…</span>` : "";
  return `<div class="section-header${k > 0 ? " later" : ""}" style="${style}">` +
    `<span class="label">${s.label}</span><span class="count">${s.count}</span>${busy}</div>`;
}

/** Top-left of item `i` within the items layer. */
function itemPos(i: number): { x: number; y: number } {
  return slotPos(layout, i);
}

function relayout(): void {
  if (!itemsLayer) return;
  computeLayout();
  const height = contentHeight(layout) + (prefs.view === "list" ? 6 : 0);
  itemsLayer.style.height = `${height}px`;
  viewEl.style.setProperty("--cell-w", `${layout.itemW}px`);
  viewEl.style.setProperty("--cell-h", `${layout.itemH}px`);
  renderWindow(true);
}

/** Render the items that intersect the viewport (plus a few rows either side). */
function renderWindow(force = false): void {
  if (!itemsLayer) return;
  const top = scrollEl.scrollTop - layout.bodyTop;
  const bottom = top + scrollEl.clientHeight;
  const overscan = (prefs.view === "grid" ? 2 : 8) * (layout.itemH + layout.gap);
  const visible = indicesInRows(layout, top, bottom);
  const { start, end } = indicesInRows(layout, top - overscan, bottom + overscan);
  if (!force && start === windowRange.start && end === windowRange.end) return;
  windowRange = { start, end };
  // At most two headers, so they are always drawn.
  const html = searchSections().map(sectionHeaderHtml);
  // The list row's star: decided once per render, not per row.
  const stars = allows("star", hereFor([]));
  for (let i = start; i <= end; i++) html.push(itemHtml(i, stars));
  itemsLayer.innerHTML = html.join("");
  itemIcons.rendered(shown.slice(start, end + 1), { start: visible.start - start, end: visible.end - start });
  // Both views: list view shows the counts, and either view shows the
  // version badge that comes with them. Only rendered folders are asked.
  const dirs: string[] = [];
  for (let i = start; i <= end; i++) if (shown[i].is_dir) dirs.push(shown[i].path);
  void requestCounts(dirs, false);
}

/** Scroll just enough to show item `i`, then render it. */
function scrollToIndex(i: number): void {
  if (!itemsLayer || i < 0 || i >= shown.length) return;
  const y = layout.bodyTop + itemPos(i).y;
  const visibleTop = scrollEl.scrollTop + layout.bodyTop;
  // Going up, the padding above the item (and its section's header) shows too.
  const above = Math.min(revealTop(layout, i), y - layout.bodyTop - layout.padY);
  if (y - layout.padY < visibleTop) {
    scrollEl.scrollTop = above;
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
  scrollToIndex(selection.cursor);
}

function resetZoom(): void {
  prefs.gridZoom = LAYOUT_SIZES[currentTheme].gridZoom;
  prefs.listZoom = 1;
  savePrefs();
  applyZoom();
  relayout();
  scrollToIndex(selection.cursor);
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
  scrollToIndex(selection.cursor);
}

function setSort(key: SortKey, reversed: boolean): void {
  prefs.sortKey = key;
  prefs.sortReversed = reversed;
  savePrefs();
  const keep = selection.mark().cursorPath;
  renderView();
  if (keep) selection.focus(selection.indexOf(keep));
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

/** Fetch "N items" for `paths`; `summaries` patches the visible cells as they land.
 *  Sorting by size sorts again once every folder is counted. */
async function requestCounts(paths: string[], resortWhenDone: boolean): Promise<void> {
  const landed = await summaries.request(paths, prefs.showHidden);
  if (landed && resortWhenDone && prefs.sortKey === "size") renderViewKeepingScroll();
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
    const isSel = !!shown[i] && selection.has(shown[i].path);
    el.classList.toggle("selected", isSel);
    el.setAttribute("aria-selected", String(isSel));
    el.classList.toggle("cursor", i === selection.cursor);
  });
  updateFloatingBar();
}

function selectOnly(index: number): void {
  selection.only(index);
  updateSelection();
}

function selectRange(from: number, to: number, additive: boolean): void {
  selection.range(from, to, additive);
  updateSelection();
}

function selectAll(): void {
  selection.all();
  updateSelection();
}

function clearSelection(): void {
  selection.clear();
  updateSelection();
}

function selectedEntries(): Entry[] {
  return selection.selected();
}

function revealPath(path: string): void {
  const i = selection.indexOf(path);
  if (i < 0) return;
  selection.focus(i);
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
      const n = summaries.count(e.path);
      fetchCount(e.path);
      text = `“${e.name}” selected` + (n === undefined || n === null ? "" : ` (containing ${n === 1 ? "1 item" : `${n.toLocaleString()} items`})`);
    } else {
      text = sizeKnown(e) ? `“${e.name}” selected (${formatSize(e.size)})` : `“${e.name}” selected`;
    }
  } else {
    const files = sel.filter((e) => !e.is_dir);
    const folders = sel.length - files.length;
    if (folders === 0 && !files.every(sizeKnown)) {
      text = `${sel.length.toLocaleString()} items selected`;
    } else if (folders === 0) {
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

/** The selected folder's count, for the floating bar, which `summaries` updates when it lands
 *  (on macOS its icon is asked for again then: see `itemIcons.recounted`). It asks only for a
 *  count it doesn't know, or one a reload may have changed. */
function fetchCount(path: string): void {
  summaries.one(path, prefs.showHidden);
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
  if (!isAppBundle(entry) && !isRemotePath(entry.path)) addRecent(entry.path).catch(() => {});
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

  if (e.shiftKey && selection.anchor >= 0) {
    selectRange(selection.anchor, index, toggleMod);
  } else if (toggleMod) {
    selection.toggle(index);
    selection.focus(index);
    updateSelection();
  } else if (!selection.has(shown[index].path)) {
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
    selection.focus(index);
    updateSelection();
  }
  if (selection.has(shown[index].path)) armDrag(e);
}

function startRubberband(start: MouseEvent, additive: boolean): void {
  selection.noteIntent();
  const band = $("#rubberband");
  const overlay = scrollEl.parentElement!;
  const base = additive ? selection.paths() : [];
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
    const next = [...base];
    // Items hit by the band, found from the layout rather than the DOM.
    for (const i of indicesInRect(layout, l, t - layout.bodyTop, r, b - layout.bodyTop)) next.push(shown[i].path);
    selection.replace(next);
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
    if (!selection.has(shown[index].path)) selectOnly(index);
    openItemMenu(at);
  } else {
    openBackgroundMenu(at);
  }
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

/** Drive items get Drive's menu; anything else there gets a folder's. */
const driveOrFolderItemMenu = (sel: Entry[], single: Entry | null): MenuItem[] =>
  sel.every((entry) => entry.cloud) ? cloudItemMenu(single) : folderItemMenu(sel, single);

/** The item menu of each kind of place. */
const ITEM_MENUS: Record<LocationKind, (sel: Entry[], single: Entry | null) => MenuItem[]> = {
  recent: recentItemMenu,
  starred: starredItemMenu,
  network: (_sel, single) => networkItemMenu(single),
  trash: trashItemMenu,
  "in-trash": trashItemMenu,
  share: (_sel, single) => shareItemMenu(single),
  sftp: (_sel, single) => shareItemMenu(single),
  arbesk: (sel) => arbeskItemMenu(sel),
  drive: driveOrFolderItemMenu,
  folder: driveOrFolderItemMenu,
};

function openItemMenu(at: Anchor): void {
  const sel = selectedEntries();
  if (sel.length === 0) return;
  const single = sel.length === 1 ? sel[0] : null;
  openMenu(at, ITEM_MENUS[hereKind()](sel, single));
}

/** An Arbesk asset: Open goes to Arbesk's Studio, Open With… hands one of
 *  its files to an application (fetched first, spec F8), Copy takes its
 *  file out (built first), Properties shows its version and other files;
 *  no Copy Location (an arbesk:// address means nothing outside the app).
 *  A collection can be shared by email. */
function arbeskItemMenu(sel: Entry[]): MenuItem[] {
  const single = sel.length === 1 ? sel[0] : null;
  const asset = single && isArbeskAsset(single.path) ? single : null;
  return [
    { label: "Open", action: activateSelection },
    ...(asset ? [{ label: "Open With…", action: () => openItemWith(asset) }] : []),
    // Collections can't be copied yet.
    { label: "Copy", disabled: sel.some((entry) => entry.is_dir), action: () => setClipboard("copy") },
    // Files from this computer, added to the asset as its other files (spec F10).
    ...(asset ? [{ label: "Paste Into Asset", disabled: !clipboard, action: () => pasteIntoAsset(asset) }] : []),
    ...(single && isArbeskCollection(single.path)
      ? [
          { separator: true } as const,
          { label: "Share Collection…", action: () => promptShareCollection(single.path) },
        ]
      : []),
    // An asset's version and its other files (spec F7).
    ...(single && isArbeskAsset(single.path) ? [{ separator: true } as const, propertiesMenuItem(single)] : []),
  ];
}

/** Read-only actions for a received share. Copy out arrives in Task 9. */
function shareItemMenu(single: Entry | null): MenuItem[] {
  return [
    { label: "Open", action: activateSelection },
    { label: "Copy", disabled: true },
    { label: "Copy Location", disabled: !single, action: copyLocation },
  ];
}

/** A Network item: open a received share or forget it; open a server or
 *  disconnect it; open an online account or disconnect it. */
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
  const account = single ? accountOf(single.path, accounts) : undefined;
  if (account) {
    // An arbesk:// address means nothing outside the app: no Copy Location.
    const copy: MenuItem[] = account.provider === "arbesk" ? [] : [{ label: "Copy Location", action: copyLocation }];
    return [
      { label: "Open", action: activateSelection },
      ...copy,
      { separator: true },
      { label: "Disconnect…", action: () => PROVIDER_ACTIONS[account.provider].disconnect(account) },
    ];
  }
  const id = single && isSharePath(single.path) ? shareId(single.path) : "";
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
    // Kept: offered even for an item in the folder on screen, unlike the
    // preview's Open Item Location (allows("openLocation") wants it elsewhere).
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
  const sel = selectedEntries().filter((entry) => !isRemotePath(entry.path));
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
  if (!acceptsNewItems(location, trashPath) || listingError !== null) return;
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
    // Nothing else responds while a document is made; a failure stays in the dialog.
    busy: { status: message },
  });

  /** Make `fileName` (or "fileName 2", …) here as the dialog's one action in flight. */
  const create = (fileName: string, make: (dest: string) => Promise<void>) =>
    void dialog.run(async () => {
      const dir = location;
      const name = await freeDocumentName(dir, fileName);
      const dest = joinPath(dir, name);
      await make(dest);
      dialog.close();
      await reload([dest]);
      showToast(`Created “${name}”`);
    });
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

  addChoice("Empty Text File", () => create("Untitled.txt", (dest) => createFile(dest)));

  try {
    const templates = (await listTemplates()).filter((entry) => !entry.is_dir);
    loading.remove();
    for (const template of templates) {
      addChoice(template.name, () =>
        create(template.name, async (dest) => {
          await copyEntry(template.path, dest, false);
        })
      );
    }
  } catch (err) {
    if (extra.isConnected) loading.textContent = `Could not read templates: ${cleanError(err)}`;
  }
}

/** Compress the selection into a new zip/tar.gz/tar.xz beside it. */
function showCompressDialog(sel: Entry[]): void {
  if (sel.length === 0 || sel.some((entry) => isRemotePath(entry.path))) return;
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
    // Escape and Cancel close it while it works; the archive still appears.
    busy: { liveWhileBusy: ["cancel"] },
    onResponse: async (id) => {
      if (id !== "compress") return;
      const name = validate();
      if (!name) return false;
      const format = formatInput.value as "zip" | "tar.gz" | "tar.xz";
      try {
        await compressArchive(sel.map((entry) => entry.path), joinPath(dir, name), format);
        dialog.close();
        await reload([joinPath(dir, name)]);
        showToast(`Created “${name}”`);
      } catch (err) {
        message.textContent = cleanError(err);
      }
      return false;
    },
  });
  validate();
  const dot = nameInput.value.lastIndexOf(".");
  nameInput.setSelectionRange(0, dot > 0 ? dot : nameInput.value.length);
  nameInput.focus();
}

/** Preview for a single local file; Open With for a single local file or
 * folder (an IDE or a terminal can take a folder: the user picks the app). */
function previewAndOpenWithItems(single: Entry | null): MenuItem[] {
  const local = single && !isRemotePath(single.path) ? single : null;
  const file = local && !local.is_dir ? local : null;
  const openable = single && allows("openWith", hereFor([single.path])) ? single : null;
  return [
    ...macDmgItems(single, isMac && !inTrash()),
    { label: "Preview", disabled: !file, action: () => file && void showPreview(file) },
    { label: "Open With…", disabled: !openable, action: () => openable && openItemWith(openable) },
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
  if (isRemotePath(entry.path)) return;
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
  const shareable = intoFolder && !isRemotePath(intoFolder.path) && !inTrash();
  const renameable = sel.length > 1 || !!single;
  return [
    { label: "Open", action: activateSelection },
    ...previewAndOpenWithItems(single),
    ...(single && !single.is_dir && !isRemotePath(single.path) && isExtractableArchive(single.name)
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
    ...(shareable ? [{ label: "Share Folder…", action: () => void showShareDialog(intoFolder) }] : []),
    ...qrShareItems(sel, !inTrash()),
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

// The background and location menus of each kind of place. In the Trash and
// the folders inside it, nothing adds to them, since only an item sent to the
// Trash knows where it came from (a pasted or new one wouldn't); a folder
// inside the Trash keeps Open With…, as trashed items do.

const emptyTrashMenu = (): MenuItem[] => [
  { label: "Empty Trash…", disabled: entries.length === 0, action: confirmEmptyTrash },
  { separator: true },
  { label: "Select All", action: selectAll },
];

const pseudoBackgroundMenu = (): MenuItem[] => [{ label: "Select All", action: selectAll }];

/** A local folder's location menu; Recent and Starred show it with everything off. */
const folderLocationMenu = (real: boolean): MenuItem[] => [
  { label: "New Folder…", disabled: !real, action: promptNewFolder },
  ...(real ? [{ label: "New Document…" as const, action: () => void showNewDocumentDialog() }, openCurrentFolderWithItem()] : []),
  ...(real ? [bookmarkMenuItem(location), { label: "Share This Folder…" as const, action: () => showShareDialogForPath(location) }] : []),
  { separator: true },
  { label: "Copy Location", disabled: !real, action: copyLocation },
  { label: "Paste", disabled: !real || !clipboard, action: () => paste(location) },
  { label: "Select All", disabled: !hasEntries(location), action: selectAll },
];

const readOnlyBackgroundMenu = (): MenuItem[] => [
  { label: "Show Hidden Files", check: prefs.showHidden, action: toggleHidden },
  { label: "Reload", action: () => void reload() },
  { separator: true },
  { label: "Select All", action: selectAll },
];

const readOnlyLocationMenu = (): MenuItem[] => [
  { label: "Copy Location", action: copyLocation },
  ...arbeskLocationMenu(),
];

/** Paste, in a collection only: the one place Arbesk takes files. */
const arbeskPasteItem = (): MenuItem[] =>
  isArbeskCollection(location) ? [{ label: "Paste", disabled: !clipboard, action: () => paste(location) }] : [];

/** New Collection…, in the account only: a collection can't hold a collection. */
const arbeskNewCollectionItem = (): MenuItem[] =>
  isArbeskAccount(location) ? [{ label: "New Collection…", action: promptNewCollection }] : [];

/** Share Collection…, inside a collection: add a collaborator by email. */
const arbeskShareItem = (): MenuItem[] =>
  isArbeskCollection(location) ? [{ label: "Share Collection…", action: () => promptShareCollection(location) }] : [];

/** Arbesk's background menu: read only, but the account takes a new collection
 *  and a collection takes a paste. */
const arbeskBackgroundMenu = (): MenuItem[] => [
  ...arbeskNewCollectionItem(),
  { label: "Show Hidden Files", check: prefs.showHidden, action: toggleHidden },
  { label: "Reload", action: () => void reload() },
  ...arbeskPasteItem(),
  ...arbeskShareItem(),
  { separator: true },
  { label: "Select All", action: selectAll },
];

/** Arbesk's location menu: its arbesk:// address means nothing outside the app. */
const arbeskLocationMenu = (): MenuItem[] => [
  ...arbeskNewCollectionItem(),
  { label: "Reload", action: () => void reload() },
  ...arbeskPasteItem(),
  ...arbeskShareItem(),
  { separator: true },
  { label: "Select All", disabled: !hasEntries(location), action: selectAll },
];

const BACKGROUND_MENUS: Record<LocationKind, () => MenuItem[]> = {
  recent: pseudoBackgroundMenu,
  starred: pseudoBackgroundMenu,
  network: () => [
    { label: "Connect to Server…", action: () => showConnectServerDialog() },
    { label: "Online Accounts…", action: () => void openOnlineAccounts() },
    { separator: true },
    { label: "Select All", action: selectAll },
  ],
  trash: emptyTrashMenu,
  "in-trash": () => [openCurrentFolderWithItem(), { separator: true }, { label: "Select All", action: selectAll }],
  share: readOnlyBackgroundMenu,
  sftp: readOnlyBackgroundMenu,
  arbesk: arbeskBackgroundMenu,
  drive: () => [
    { label: "New Folder…", action: promptNewFolder },
    { separator: true },
    { label: "Paste", disabled: !clipboard, action: () => paste(location) },
    { label: "Reload", action: () => void reload() },
    { label: "Select All", action: selectAll },
  ],
  folder: () => [
    { label: "New Folder…", action: promptNewFolder },
    { label: "New Document…", action: () => void showNewDocumentDialog() },
    openCurrentFolderWithItem(),
    { label: "Share This Folder…", action: () => showShareDialogForPath(location) },
    { separator: true },
    { label: "Paste", disabled: !clipboard, action: () => paste(location) },
    ...(versionedRootPath
      ? [{ label: "Deleted Files…" as const, action: () => showDeletedFiles(location, () => void reload()) }]
      : []),
    { label: "Select All", action: selectAll },
  ],
};

const LOCATION_MENUS: Record<LocationKind, () => MenuItem[]> = {
  recent: () => folderLocationMenu(false),
  starred: () => folderLocationMenu(false),
  network: () => [
    { label: "Connect to Server…", action: () => showConnectServerDialog() },
    { label: "Online Accounts…", action: () => void openOnlineAccounts() },
    { separator: true },
    { label: "Reload", action: () => void reload() },
  ],
  trash: emptyTrashMenu,
  "in-trash": () => [
    openCurrentFolderWithItem(),
    { separator: true },
    { label: "Copy Location", action: copyLocation },
    { label: "Select All", action: selectAll },
  ],
  share: readOnlyLocationMenu,
  sftp: readOnlyLocationMenu,
  arbesk: arbeskLocationMenu,
  drive: () => [
    { label: "New Folder…", action: promptNewFolder },
    { separator: true },
    { label: "Copy Location", action: copyLocation },
    { label: "Paste", disabled: !clipboard, action: () => paste(location) },
    { label: "Select All", disabled: !hasEntries(location), action: selectAll },
  ],
  folder: () => folderLocationMenu(true),
};

function openBackgroundMenu(at: Anchor): void {
  openMenu(at, BACKGROUND_MENUS[hereKind()]());
}

function openLocationMenu(anchor: HTMLElement): void {
  openMenu(anchor, LOCATION_MENUS[hereKind()]());
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
    { label: "Online Accounts…", action: () => void openOnlineAccounts() },
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

  const chosen = () => {
    let next = 0;
    extra.querySelectorAll<HTMLInputElement>("input[data-bit]").forEach((input) => {
      if (input.checked) next |= Number(input.dataset.bit);
    });
    return next;
  };

  const dialog = openDialog({
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
    // Nothing else responds while they change; a failure stays in the dialog.
    busy: { status: message },
    onResponse: async (id) => {
      if (id !== "change") return;
      await setPermissions(path, chosen());
      // Closed before onSaved reopens Properties.
      dialog.close();
      showToast("Permissions changed");
      onSaved();
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
  if (refuse("properties", hereFor([entry.path]))) return;
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
  itemIcons.dialogIcon(icon, entry, 64);
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
  const kind = locationKind(info.path, trashPath);
  const localFacts = hasLocalFileFacts(kind);
  if (info.is_dir) {
    if (!localFacts) {
      // Drive does not report recursive folder sizes cheaply.
      add("Size", "—");
    } else {
      const row = propertyRow("Size", "Calculating…");
      sizeValue = row.querySelector<HTMLElement>(".prop-value");
      list.appendChild(row);
    }
    if (info.item_count !== null) add("Contents", formatItemCount(info.item_count));
  } else if (knowsSizes(kind)) {
    add("Size", sizeAndBytes(info.size));
  }
  // An Arbesk asset's parent is its collection, shown by name: an arbesk://
  // address means nothing to read (its tooltip still holds it).
  add("Parent folder", kind === "arbesk" ? pathName(info.parent) : info.parent, info.parent);
  add("Modified", formatDateDetailed(info.modified));
  if (info.version !== undefined) add("Version", String(info.version));
  if (localFacts) {
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
  const canChangePermissions = !isRemotePath(info.path);
  const propertiesDialog = openDialog({
    heading: "Properties",
    className: "properties-dialog",
    extra,
    responses: [
      ...(canChangePermissions ? [{ id: "permissions", label: "Permissions…" }] : []),
      // An Arbesk asset's other files, a page of their own as Permissions… is.
      ...(entry.representations?.count ? [{ id: "files", label: "Files…" }] : []),
      { id: "close", label: "Close" },
    ],
    defaultResponse: "close",
    closeResponse: "close",
    // Files… waits on Arbesk: one fetch at a time, Close still live. Its
    // answer, success or failure, counts only while Properties is open, and
    // the page opens once the wait has ended, so Files… has its focus back
    // and the page returns it there when closed.
    busy: { liveWhileBusy: ["close"], errorText: (err) => `Could not show the files: ${cleanError(err)}` },
    onResponse: (id) => {
      if (id === "files") {
        void propertiesDialog
          .run(() =>
            arbeskRepresentations(entry.path).catch((err: unknown) => {
              if (extra.isConnected) throw err;
              return null;
            })
          )
          .then((files) => {
            if (files && extra.isConnected) openFilesPage(entry, files);
          });
        return false;
      }
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
  if (refuse("star", hereFor(paths))) return;
  for (const p of paths) {
    if (on) starred.add(p);
    else starred.delete(p);
  }
  // Starred lists what starred.json holds: wait for the write with this change.
  await starSaves.save();
  if (location === STARRED) await reload();
  else renderWindow(true);
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
  putOnClipboard(sel.map((e) => e.path), new Set(sel.filter((e) => e.is_dir).map((e) => e.path)), mode);
}

/** Copy or Cut of `paths` (`dirs` the folders among them), as the
 *  item-actions rule allows it: every way onto the clipboard goes here. */
function putOnClipboard(paths: string[], dirs: Set<string>, mode: "copy" | "cut"): void {
  if (refuse(mode, hereFor(paths))) return;
  clipboard = { paths, dirs, mode };
  // Paths also go to the system clipboard as text, handy for terminals.
  navigator.clipboard?.writeText(paths.join("\n")).catch(() => {});
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

/** What Replace does, in the conflict dialog: in Arbesk it saves the file as
 *  the asset's next version (spec D3), elsewhere the old item goes to the Trash. */
function conflictBody(kind: "file" | "folder", destDir: string): string {
  const where = `Another ${kind} with the same name already exists in “${locationName(destDir)}”.`;
  if (placeOf(destDir) === "arbesk") return `${where} Replacing it saves your file as its newest version; the earlier ones stay in its history.`;
  return `${where} Replacing it moves the existing ${kind} to the Trash.`;
}

/** GNOME-style conflict dialog. Resolves with the choice and whether to apply it to the rest. */
function askConflict(name: string, isDir: boolean, destDir: string, remaining: number): Promise<{ choice: ConflictChoice; all: boolean }> {
  return new Promise((resolve) => {
    const kind = isDir ? "folder" : "file";
    let all = false;
    const extra = applyToAllRow(remaining, (ticked) => (all = ticked)) ?? undefined;
    openDialog({
      heading: `Replace ${kind} “${name}”?`,
      body: conflictBody(kind, destDir),
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

/** "name (copy).txt", then "name (copy 2).txt", … — the first one that's
 *  free in `dir` and not `reserved` (lower-cased names a paste still
 *  plans to write, spec F9). */
async function freeCopyName(dir: string, name: string, isDir: boolean, reserved?: ReadonlySet<string>): Promise<string> {
  for (let n = 1; ; n++) {
    const candidate = copyNameAt(name, n, isDir);
    if (reserved?.has(candidate.toLowerCase())) continue;
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
  if (placeOf(destDir) === "arbesk" && !(await readyForArbesk(paths, dirs, paths.some(move), destDir))) return false;
  // The user's selection as of this paste; see transferBatch's end.
  const intent = selection.intent;
  const batch = { cancelled: false, started: false, measuring: false };
  const op = transferOperation(paths, paths.some(move), destDir, () => {
    batch.cancelled = true;
    // Waiting in the queue or still measuring: nothing to stop yet, so the
    // row goes at once and the batch ends before its first item.
    if (!batch.started || batch.measuring) endOperation(op);
    else void cancelOperation(op.id).catch(() => {});
  });
  // Arbesk assets with other files ask which files first (spec F9).
  const plan = needsCopyChoice(paths, move, destDir) ? await planArbeskCopy(paths, destDir, op, batch) : { paths, names: undefined };
  if (!plan) return false;
  const run = transferChain.then(async () => {
    if (batch.cancelled) return false;
    batch.started = true;
    transfersRunning++;
    try {
      return await transferBatch(plan.paths, dirs, move, destDir, op, batch, intent, plan.names);
    } finally {
      transfersRunning--;
      endOperation(op);
    }
  });
  transferChain = run.catch(() => {});
  return run;
}

// ---------------------------------------------------------------------------
// Copying an Arbesk asset's files out (spec F9)
// ---------------------------------------------------------------------------

/** A copy (never a move) of Arbesk assets to this computer or Drive, the
 *  routes copy out has: their other files may go too. */
function needsCopyChoice(paths: readonly string[], move: (path: string) => boolean, destDir: string): boolean {
  const out = placeOf(destDir) === "local" || placeOf(destDir) === "drive";
  return out && paths.some((path) => isArbeskAsset(path) && !move(path));
}

/** A failed read of one asset's list of files: which asset, and why. */
class AssetFilesError {
  constructor(readonly path: string, readonly reason: unknown) {}
}

/** Each asset's other files, at most 4 asked at a time (one `tokenURI`
 *  read each, the manifests from the cache). No new read starts once
 *  `batch` is cancelled or a read failed; a failure rejects with an
 *  `AssetFilesError` naming its asset. */
async function filesOfAssets(assets: readonly string[], batch: { cancelled: boolean }): Promise<AssetFiles[]> {
  const found: AssetFiles[] = new Array(assets.length);
  let next = 0;
  let failed = false;
  const worker = async () => {
    for (let i = next++; i < assets.length && !batch.cancelled && !failed; i = next++) {
      try {
        found[i] = { path: assets[i], name: basename(assets[i]), files: await arbeskRepresentations(assets[i]) };
      } catch (err) {
        failed = true;
        throw new AssetFilesError(assets[i], err);
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(4, assets.length) }, worker));
  return found;
}

/**
 * Before a copy of Arbesk assets out (spec F9): read each asset's other
 * files under the paste's row (asked at paste time, so a drop from another
 * window works too); when any has some, ask which to copy. The sources
 * become, per asset in order, its path (the model) and each chosen file's
 * pinned path, with their target names: repeated names within the paste
 * get Keep Both's naming. Null when nothing should be copied (Cancel, a
 * cancelled row, a failed read: a toast).
 */
async function planArbeskCopy(
  paths: string[], destDir: string, op: Operation, batch: { cancelled: boolean; measuring: boolean },
): Promise<{ paths: string[]; names?: Map<string, string> } | null> {
  const queued = op.state;
  setOperationState(op, "preparing");
  // Nothing runs yet: a Cancel takes the row away at once (runTransfer's).
  batch.measuring = true;
  const assets = paths.filter(isArbeskAsset);
  let read: AssetFiles[] = [];
  let failure: AssetFilesError | null = null;
  let choice: CopyChoice | null = null;
  try {
    read = await filesOfAssets(assets, batch);
    if (!batch.cancelled && read.some((asset) => asset.files.length > 0)) {
      choice = await chooseFilesToCopy(read, (asset) => iconEntryOf(asset.name));
    } else if (!batch.cancelled) {
      setOperationState(op, queued);
      return { paths, names: undefined };
    }
  } catch (err) {
    failure = err instanceof AssetFilesError ? err : new AssetFilesError(assets[0], err);
  } finally {
    batch.measuring = false;
  }
  if (batch.cancelled) return null;
  const byAsset = new Map(read.map((asset) => [asset.path, asset]));
  const sources = choice ? paths.flatMap((path) => (byAsset.has(path) ? copySources([byAsset.get(path)!], choice) : [{ path, name: basename(path) }])) : [];
  if (sources.length === 0) {
    if (failure !== null) showToast(`Could not copy “${basename(failure.path)}”: ${cleanError(failure.reason)}`);
    endOperation(op);
    return null;
  }
  setOperationState(op, queued);
  // Each target named as transferBatch would name it, an Arbesk one made a
  // safe local name; then the names repeated within the paste made unique.
  const named = sources.map(({ path, name }) => (placeOf(path) === "arbesk" && !isCloud(destDir) ? safeLocalName(name) : targetBase(path, destDir)));
  const unique = await uniqueNames(named, (name) => pathExists(joinPath(destDir, name)));
  return { paths: sources.map((source) => source.path), names: new Map(sources.map((source, i) => [source.path, unique[i]])) };
}

/** A copy's target name for `from` in `destDir` before any clash: its own
 *  name, made one plain local name when it comes from Drive or Arbesk. */
function targetBase(from: string, destDir: string): string {
  const unsafe = (isCloud(from) || placeOf(from) === "arbesk") && !isCloud(destDir);
  return unsafe ? safeLocalName(basename(from)) : basename(from);
}

/** An entry shaped only to pick a file's icon by its name. */
const iconEntryOf = (name: string): Entry => ({ name, path: name, is_dir: false, size: 0, modified: 0, is_hidden: false, is_symlink: false });

// ---------------------------------------------------------------------------
// Copying into Arbesk (spec 2026-11-09-arbesk-copy-in, D1, D2, D4)
// ---------------------------------------------------------------------------

/** The error a copy into Arbesk ends with when it needs a new sign-in. */
const ARBESK_SIGN_IN = "Sign in to Arbesk again to copy files into it.";

/**
 * Before a paste or drop into Arbesk starts: refuse what it can't take, in
 * one toast (D1; a drop from another app reaches here unchecked); confirm
 * that the files go public (D2); sign in first when the account has no
 * session (D4). False when nothing should start.
 */
async function readyForArbesk(paths: string[], dirs: ReadonlySet<string>, moving: boolean, destDir: string): Promise<boolean> {
  const why = arbeskPasteRefusal(destDir, paths, moving, dirs);
  if (why) {
    showToast(why);
    return false;
  }
  if (!(await confirmArbeskCopy())) return false;
  if (await arbeskSignedIn(destDir).catch(() => false)) return true;
  return connectArbesk(await arbeskAccountFor(destDir), { stay: true });
}

/** Copy to Arbesk?: asked before every paste or drop into it; Cancel is the default. */
function confirmArbeskCopy(): Promise<boolean> {
  return new Promise((resolve) => {
    openDialog({
      heading: "Copy to Arbesk?",
      body: "Files in Arbesk are public: anyone with their address can download them, and they can't be removed from IPFS.",
      responses: [
        { id: "cancel", label: "Cancel" },
        { id: "copy", label: "Copy", style: "suggested" },
      ],
      defaultResponse: "cancel",
      closeResponse: "cancel",
      onResponse: (id) => resolve(id === "copy"),
    });
  });
}

/** The Arbesk account `path` belongs to, loading the accounts when it isn't known yet. */
async function arbeskAccountFor(path: string): Promise<Account | undefined> {
  if (!accountOf(path, accounts)) await refreshAccounts();
  return accountOf(path, accounts);
}

/**
 * A batch into Arbesk signs in again at most once, when an item finds its
 * session gone (D4); the items after it reuse the answer. Null elsewhere.
 */
function signInAgainOnce(destDir: string): (() => Promise<boolean>) | null {
  if (placeOf(destDir) !== "arbesk") return null;
  let asked: Promise<boolean> | null = null;
  const ask = async () => {
    const account = await arbeskAccountFor(destDir);
    // Online Accounts says "Sign in again" until it does.
    if (account) signInNeeded.add(accountRef(account));
    return connectArbesk(account, { stay: true });
  };
  return () => (asked ??= ask());
}

/** One item of a batch; one that failed for want of an Arbesk session is
 *  tried once more after `signIn` succeeds. A sign-in that is cancelled
 *  stops the batch, so the items after it don't fail one by one. */
async function transferSigningIn(
  op: Operation, from: string, to: string, overwrite: boolean, moving: boolean,
  signIn: (() => Promise<boolean>) | null, batch: { cancelled: boolean },
): Promise<string> {
  const before = { bytes: op.doneBytes, items: op.doneItems };
  try {
    return await transferItem(op, from, to, overwrite, moving);
  } catch (err) {
    if (!signIn || cleanError(err) !== ARBESK_SIGN_IN) throw err;
    if (!(await signIn())) batch.cancelled = true;
    if (batch.cancelled) throw err;
    // The failed attempt's progress isn't counted twice.
    op.doneBytes = before.bytes;
    op.doneItems = before.items;
    return transferItem(op, from, to, overwrite, moving);
  }
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
    // Followed as it lands, so stars and bookmarks are saved before the rest
    // of the batch (src/path-refs.ts): the user can rename this item, or
    // close the window, while a name clash or a long item still waits.
    if (moving) pathRefs.moved([{ from, to: result }]);
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
    const open = [...landedIn].some((dir) => isUnder(location, dir));
    landedIn.clear();
    if (open) void reload();
  }, wait);
}

async function transferBatch(
  paths: string[], dirs: Set<string>, move: (path: string) => boolean, destDir: string,
  op: Operation, batch: { cancelled: boolean; measuring: boolean }, intent: number,
  names?: ReadonlyMap<string, string>,
): Promise<boolean> {
  await measureBatch(op, paths, batch);
  const done: string[] = [];
  const errors: string[] = [];
  let anyMove = false;
  let applyToAll: ConflictChoice | null = null;
  const signIn = signInAgainOnce(destDir);
  // Keep Both never takes a name the paste plans for another file (F9).
  const reserved = names && new Set([...names.values()].map((name) => name.toLowerCase()));

  for (let i = 0; i < paths.length && !batch.cancelled; i++) {
    const from = paths[i];
    // A Drive or Arbesk name can be "a/b" or "..": locally it must stay one
    // plain name. A copy out of Arbesk names each file as planned (F9).
    const base = names?.get(from) ?? targetBase(from, destDir);
    // A .glb is stored as glTF: its listed name, which a clash is checked against.
    const name = placeOf(destDir) === "arbesk" ? arbeskTargetName(base) : base;
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
      to = joinPath(destDir, await freeCopyName(destDir, name, isDir, reserved));
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
      if (choice === "keepboth") to = joinPath(destDir, await freeCopyName(destDir, name, isDir, reserved));
      else overwrite = true;
    }

    try {
      done.push(await transferSigningIn(op, from, to, overwrite, moving, signIn, batch));
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
  const untouched = destDir === location && intent === selection.intent;
  await reload(untouched && done.length ? done : undefined);
  return !batch.cancelled && errors.length === 0;
}

async function paste(destDir: string): Promise<void> {
  if (!clipboard) return;
  const { paths, dirs, mode } = clipboard;
  if (refuse("paste", { kind: locationKind(destDir, trashPath), items: [], paths, dir: destDir, moving: mode === "cut", dirs })) return;
  const pasting = clipboard;
  const complete = await runTransfer(paths, dirs, () => mode === "cut", destDir);
  // A cut is only consumed once every item has been dealt with. Its paths
  // follow the items it moved (src/path-refs.ts): it's known by its object.
  if (mode === "cut" && complete && clipboard === pasting) {
    clipboard = null;
    refreshCutMarks();
  }
}

function inTrash(): boolean {
  return isInTrash(location, trashPath);
}

/** The kind of place on screen (src/locations.ts). */
function hereKind(): LocationKind {
  return locationKind(location, trashPath);
}

/** An action on `paths` from the location on screen, for src/item-actions.ts. */
function hereFor(paths: readonly string[], elsewhere = false): Here {
  return { kind: hereKind(), items: paths.map((path) => locationKind(path, trashPath)), paths, elsewhere };
}

/** Whether `action` is refused `here`; says why in a toast when the rule has words for it. */
function refuse(action: SharedAction, here: Here): boolean {
  const why = refusal(action, here);
  if (why) showToast(why);
  return why !== null;
}

async function trashSelection(): Promise<void> {
  const paths = selectedEntries().map((e) => e.path);
  if (refuse("trash", hereFor(paths))) return;
  await trashPaths(paths);
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
  pathRefs.gone(done);
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
  if (sel.length === 0 || refuse("deletePermanently", hereFor(sel.map((e) => e.path)))) return;
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

type DropOp = "move" | "copy" | "trash" | "attach";

/** Where a drop lands: a folder (`dir`), or an item that takes files as
 *  its own (`attach`: an Arbesk asset, spec F10), whose path `dir` holds. */
interface DropTarget {
  dir: string;
  el: HTMLElement | null;
  kind: "dir" | "attach";
}

/** Where a drop at client point (x, y) lands, ignoring the dragged `sources`. */
function dropTargetAt(x: number, y: number, sources: string[]): DropTarget | null {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  if (!el) return null;
  const item = el.closest<HTMLElement>(".item");
  if (item && viewEl.contains(item)) {
    const e = shown[Number(item.dataset.index)];
    if (e?.is_dir && !isAppBundle(e) && !sources.includes(e.path)) return { dir: e.path, el: item, kind: "dir" };
    // An item that takes files highlights as a folder does (spec F10).
    if (e && !e.is_dir && takesFiles(e.path)) return { dir: e.path, el: item, kind: "attach" };
  }
  const row = el.closest<HTMLElement>(".sidebar-row");
  if (row) {
    const loc = row.dataset.location;
    return !loc || isPseudo(loc) ? null : { dir: loc, el: row, kind: "dir" };
  }
  const crumb = el.closest<HTMLElement>(".crumb");
  if (crumb) return crumb.dataset.path && !isPseudo(crumb.dataset.path) ? { dir: crumb.dataset.path, el: crumb, kind: "dir" } : null;
  // Anywhere else in the view means the folder being shown.
  if (el.closest(".view-overlay") && !isPseudo(location) && listingError === null) {
    return { dir: location, el: $(".view-overlay"), kind: "dir" };
  }
  return null;
}

/** The volume a path is on, for a quick guess before device ids arrive. */
function volumeRoot(path: string): string {
  return mountedVolumes
    .filter((v) => isWithin(path, v.path))
    .sort((a, b) => b.path.length - a.path.length)[0]?.path ?? "/";
}

/** The operation a drop would do, or null if it would do nothing useful. */
function dropOp(target: DropTarget, sources: string[], mods: { altKey: boolean; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean }, devices?: Map<string, number>, dirs?: ReadonlySet<string>): DropOp | null {
  const dir = target.dir;
  const forceMoveKey = isMac ? mods.metaKey : mods.shiftKey;
  // An asset takes copies of files on this computer as its own (spec F10).
  if (target.kind === "attach") return attachRefusal(dir, sources, forceMoveKey && !(isMac ? mods.altKey : mods.ctrlKey), dirs) === null ? "attach" : null;
  // A collection in Arbesk takes only copies of model files; a forced move or a folder says no.
  if (placeOf(dir) === "arbesk") return arbeskPasteRefusal(dir, sources, forceMoveKey && !(isMac ? mods.altKey : mods.ctrlKey), dirs) === null ? "copy" : null;
  // A place that takes no copies, or is read only, takes no drop: Paste's rule (`can_copy_in`).
  if (!takesDrops(dir) || copyOutRefusal(sources) !== null) return null;
  // Into itself or its own subfolder: refused.
  if (sources.some((src) => isWithin(dir, src))) return null;
  // Drive items go to Drive's Trash with Move to Trash, not the app's Trash.
  if (dir === trashPath) return inTrash() || sources.some(isCloud) ? null : "trash";
  // Only Move to Trash puts items in the Trash: a folder inside it takes nothing.
  if (isInTrash(dir, trashPath)) return null;
  const forceCopy = isMac ? mods.altKey : mods.ctrlKey;
  const forceMove = isMac ? mods.metaKey : mods.shiftKey;
  // Arbesk's assets can only be copied out: a forced move does nothing.
  const fromArbesk = sources.some((src) => placeOf(src) === "arbesk");
  if (forceMove && fromArbesk && !forceCopy) return null;
  // Moving items to the folder they are already in does nothing.
  const allHere = sources.every((src) => parentOf(src) === dir);
  if (forceCopy) return "copy";
  if (allHere) return null;
  if (forceMove) return "move";
  // Local ↔ Drive is another storage, and so is Arbesk: copy, before any device guess.
  const storage = (path: string) => (isCloud(path) ? cloudAccountKey(path) : placeOf(path) === "arbesk" ? "arbesk" : "");
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
  if (op === "attach") return attachFiles(target.dir, sources, dirs, false);
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
  // A drag copies or moves the items out of where they are.
  if (sel.length === 0 || refuse("copy", hereFor(sel.map((x) => x.path)))) return;
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
  };
  deviceMap(paths).then((m) => {
    if (drag?.paths === paths) for (const [k, v] of m) drag.devices.set(k, v);
  });

  window.addEventListener("mousemove", onDragMove);
  window.addEventListener("mouseup", onDragEnd);
  window.addEventListener("keydown", onDragKey, true);
  window.addEventListener("keyup", onDragKey, true);
  updateDrag();
}

/** Past the window's edge the system drag takes the local items to another
 * app (src/drag-out.ts) and the window's own drag ends. Returns whether it
 * did: Google Drive and Arbesk items can't leave, so a drag of only those stays. */
function dragOutOfWindow(): boolean {
  const all = drag!.paths;
  const stay = all.filter((path) => placeOf(path) === "drive" || placeOf(path) === "arbesk");
  const paths = all.filter((path) => !stay.includes(path));
  if (paths.length === 0) return false;
  endDrag(false);
  startSystemDrag(paths).then(
    () => stay.length && showToast(stayWords(stay)),
    (err) => showToast(`Could not drag out: ${cleanError(err)}`),
  );
  return true;
}

/** The toast for items a drag out of the window leaves behind. */
function stayWords(stay: readonly string[]): string {
  const [one, many] = stay.every(isCloud)
    ? ["A Google Drive item", "Google Drive items"]
    : stay.every((path) => placeOf(path) === "arbesk")
      ? ["An Arbesk item", "Arbesk items"]
      : ["An item", "items"];
  return stay.length === 1 ? `${one} stays in File Browser` : `${stay.length} ${many} stay in File Browser`;
}

function onDragMove(e: MouseEvent): void {
  if (!drag) return;
  if (leftWindow(e) && dragOutOfWindow()) return;
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
  const op = target ? dropOp(target, d.paths, d.mods, known, d.dirs) : null;

  // Hovering a folder for a moment opens it, so the drop can go deeper.
  if (target?.dir !== d.target?.dir) {
    clearTimeout(d.springTimer);
    if (target && op && op !== "trash" && op !== "attach" && target.dir !== location && target.el !== $(".view-overlay")) {
      d.springTimer = window.setTimeout(() => {
        if (drag === d && d.target?.dir === target.dir) navigate(target.dir).then(() => updateDrag());
      }, 1000);
    }
  }
  d.target = target;
  d.op = op;
  setDropHighlight(op ? target!.el : null);
  const copying = op === "copy" || op === "attach";
  d.ghost.querySelector<HTMLElement>(".drag-op")!.hidden = !copying;
  document.documentElement.style.cursor = copying ? "copy" : op ? "default" : "no-drop";
}

function autoScrollDuringDrag(): void {
  if (!drag) return;
  const r = scrollEl.getBoundingClientRect();
  if (drag.x < r.left || drag.x > r.right) return;
  // Past the window's bottom edge the drag goes out to other apps, so the
  // band reaches further in than out.
  if (drag.y > r.bottom - 32 && drag.y < r.bottom + 24) scrollEl.scrollTop += 16;
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

const NOT_FILES = "Only files and folders can be dropped here";

/** The local paths of a drop. On Linux a browser link arrives as its URL
 * (wry strips only `file://`); those are not files. */
function localDropPaths(paths: string[]): string[] {
  return paths.filter((path) => path.startsWith("/"));
}

/** What a drop from another app does. It copies where an internal drag
 * would move: the system shows a copy cursor, and the other app's window
 * keeps its files. The Trash still trashes. Modifier keys aren't reported.
 * Into Arbesk it copies, and the copy says in words what Arbesk can't take
 * (the system showed a copy cursor, so silence would look like a lost drop). */
function externalDropOp(target: DropTarget, sources: string[], own: boolean, devices?: Map<string, number>): DropOp | null {
  // Onto an asset it adds the files (spec F10), saying in words what it can't.
  if (target.kind === "attach") return "attach";
  if (placeOf(target.dir) === "arbesk") return "copy";
  const op = dropOp(target, sources, { altKey: false, metaKey: false, ctrlKey: false, shiftKey: false }, devices);
  // The window's own drag that left it and came back keeps the window's rule.
  return op === "move" && !own ? "copy" : op;
}

/** Files dragged in from Finder, GNOME Files or a browser's downloads. */
async function dropFromOtherApp(all: string[], target: DropTarget | null): Promise<void> {
  const paths = localDropPaths(all);
  // Decided on arrival, before any wait: the drag's end can be reported
  // meanwhile. A drag out that comes back is over once dropped here.
  const own = isOwnDragOut(paths);
  forgetDragOut();
  if (!target) return;
  // Web content (a link or an image on a page) has no local file: macOS
  // reports no paths, Linux its URL.
  if (paths.length < all.length || paths.length === 0) showToast(NOT_FILES);
  if (paths.length === 0) return;
  const op = externalDropOp(target, paths, own, await deviceMap([...paths, target.dir]));
  if (!op) return;
  const info = await pathInfo(paths).catch(() => []);
  const dirs = new Set(paths.filter((_, i) => info[i]?.is_dir));
  await performDrop(paths, dirs, target, op);
}

/** Drops from other apps arrive as Tauri drag-drop events, with paths. */
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
      // wry reports CSS pixels relative to the webview on macOS and Linux;
      // tauri only labels them physical. Scaling them would hit-test a 2×
      // display at half the pointer's position and drop into the wrong folder.
      const { x, y } = p.position;
      if (p.type === "enter") sources = localDropPaths(p.paths);
      const target = dropTargetAt(x, y, sources);
      if (p.type !== "drop") {
        const op = target && sources.length ? externalDropOp(target, sources, isOwnDragOut(sources)) : null;
        setDropHighlight(op ? target!.el : null);
        return;
      }
      setDropHighlight(null);
      sources = [];
      await dropFromOtherApp(p.paths, target);
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

/** Add or remove a bookmark. Adding asks the rule (src/item-actions.ts); removing always works.
 *  The sidebar shows it at once and takes it back if it couldn't be saved. */
async function setBookmarked(path: string, on: boolean): Promise<void> {
  if (on === pins.includes(path) || (on && fixedSidebarPaths.has(path))) return;
  if (on && refuse("bookmark", { kind: locationKind(path, trashPath), items: [] })) return;
  const at = pins.indexOf(path);
  if (!(await setPins(on ? [...pins, path] : pins.filter((p) => p !== path)))) undoBookmarked(path, on, at);
}

/** Take back a bookmark change that couldn't be saved, from the bookmarks as
 *  they are now: a move may have changed them meanwhile. A removed bookmark
 *  goes back at `at`, where it was. */
function undoBookmarked(path: string, on: boolean, at: number): void {
  if (on !== pins.includes(path)) return;
  void setPins(on ? pins.filter((p) => p !== path) : [...pins.slice(0, at), path, ...pins.slice(at)]);
}

/** Show `next` as the bookmarks at once, and save it: true once it is
 *  written (a failure says so in a toast). */
function setPins(next: string[]): Promise<boolean> {
  pins = next;
  renderPins();
  return pinSaves.save();
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

/**
 * Whether `name` can be given to an item. `siblings` are the names to check
 * for a clash: only the folder on screen is listed, so items elsewhere (a
 * search result in a subfolder, Recent) are checked when renamed instead.
 */
function validateName(name: string, isDir: boolean, original?: string, siblings: Entry[] = entries): Validation {
  const noun = isDir ? "Folder" : "File";
  const lower = isDir ? "folder" : "file";
  if (name === "" || name === original) return { valid: false, message: "", error: false };
  if (name.includes("/")) return { valid: false, message: `${noun} names cannot contain “/”.`, error: true };
  if (name === "." || name === "..") return { valid: false, message: `A ${lower} cannot be called “${name}”.`, error: true };
  if (new TextEncoder().encode(name).length > 255) return { valid: false, message: `${noun} name is too long.`, error: true };
  const clash = siblings.find((e) => e.name === name && e.name !== original);
  if (clash) return { valid: false, message: clashMessage(clash.is_dir), error: true };
  if (name.startsWith(".")) {
    return { valid: true, message: `${noun}s with “.” at the beginning of their name are hidden.`, error: false };
  }
  return { valid: true, message: "", error: false };
}

function clashMessage(isDir: boolean): string {
  return `A ${isDir ? "folder" : "file"} with that name already exists.`;
}

/** The listed items beside `entry`: known only for the folder on screen. */
function siblingsOf(entry: Entry): Entry[] {
  return parentOf(entry.path) === location ? entries : [];
}

/**
 * Rename `entry` to `name` in its own folder, which isn't the folder on
 * screen for a search result in a subfolder. Returns the new path. The
 * backend refuses an existing target too; this gives GNOME's message.
 */
async function renameInPlace(entry: Entry, name: string): Promise<string> {
  const target = joinPath(parentOf(entry.path), name);
  if (siblingsOf(entry) !== entries && (await pathExists(target))) {
    const [other] = await pathInfo([target]).catch(() => [null]);
    throw new Error(clashMessage(other?.is_dir ?? entry.is_dir));
  }
  return renameEntry(entry.path, target);
}

function showValidation(v: Validation, entry: HTMLElement, message: HTMLElement): void {
  message.textContent = v.message;
  message.classList.toggle("error", v.error);
  entry.classList.toggle("error", v.error);
}

// ---------------------------------------------------------------------------
// Rename (popover anchored to the item, as in GNOME Files)
// ---------------------------------------------------------------------------

/**
 * Rename several items in one folder with find/replace, prefix and suffix.
 * Kept: it doesn't ask the rename rule, as before. Only a folder's item menu
 * reaches it, where the rule refuses nothing but the Trash folder itself
 * (shown with hidden files), so a batch can still rename that one.
 */
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
    busy: { liveWhileBusy: ["cancel"] },
    onResponse: async (id) => {
      if (id !== "rename") return;
      const changed = validate();
      if (!changed) return false;
      try {
        const items = changed.map(({ entry, name }) => ({ from: entry.path, to: joinPath(location, name) }));
        await renameMany(items);
        // The backend renames all of them or, undoing what it did, none,
        // unless an item took an old name meanwhile: then that rename
        // stays, and the error says which (the reload below shows it).
        pathRefs.moved(items);
        dialog.close();
        showToast(`Renamed ${changed.length} ${changed.length === 1 ? "item" : "items"}`);
        await reload();
      } catch (err) {
        message.textContent = cleanError(err);
        await reload();
      }
      return false;
    },
  });
  validate();
  find.focus();
}

function startRename(entry: Entry): void {
  if (refuse("rename", hereFor([entry.path]))) return;
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
  /** A rename is in flight: a second Enter or click waits for it. */
  let renaming = false;

  const validate = () => {
    const v = validateName(input.value, entry.is_dir, entry.name, siblingsOf(entry));
    showValidation(v, input, message);
    button.disabled = !v.valid || renaming;
    return v.valid;
  };

  const submit = async () => {
    if (renaming || !validate()) return;
    renaming = true;
    button.disabled = true;
    let renamed: string;
    try {
      renamed = await renameInPlace(entry, input.value);
    } catch (err) {
      renaming = false;
      // Rename as the name wants it, so a click can try again like Enter.
      validate();
      showValidation({ valid: false, message: cleanError(err), error: true }, input, message);
      return;
    }
    pathRefs.moved([{ from: entry.path, to: renamed }]);
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
  if (!acceptsNewItems(location, trashPath) || listingError !== null) return;
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
    busy: { liveWhileBusy: ["cancel"] },
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
// New collection (Arbesk: a name, then a mint on Arbesk's chain)
// ---------------------------------------------------------------------------

/** The error a creation ends with when it needs a new sign-in. */
const ARBESK_CREATE_SIGN_IN = "Sign in to Arbesk again to create a collection.";

/**
 * New Collection…: the name, then the mint. Arbesk derives the token from
 * the account and the name, so the same name is the same collection; an
 * existing one stays in the dialog. Creating needs the session a paste
 * needs, asked for once if it is gone.
 */
function promptNewCollection(): void {
  if (!isArbeskAccount(location) || listingError !== null) return;
  const extra = document.createElement("div");
  extra.innerHTML = `
    <div class="entry-row"><label for="new-collection-name">Collection Name</label><input id="new-collection-name" type="text" spellcheck="false" /></div>
    <div class="field-message" style="margin-top:8px"></div>`;
  const dir = location;
  const row = extra.querySelector<HTMLElement>(".entry-row")!;
  const input = extra.querySelector<HTMLInputElement>("#new-collection-name")!;
  const message = extra.querySelector<HTMLElement>(".field-message")!;

  const validate = () => {
    const valid = input.value.trim() !== "";
    showValidation({ valid, message: "", error: false }, row, message);
    dialog.setResponseEnabled("create", valid);
    return valid;
  };

  const dialog = openDialog({
    heading: "New Collection",
    body: "Choose a name for the new collection.",
    extra,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "create", label: "Create", style: "suggested" },
    ],
    defaultResponse: "create",
    closeResponse: "cancel",
    busy: { liveWhileBusy: ["cancel"], status: message },
    onResponse: async (id) => {
      if (id !== "create" || !validate()) return;
      const name = input.value.trim();
      try {
        const account = await arbeskAccountFor(dir);
        if (!(await arbeskSignedIn(dir).catch(() => false))) {
          if (!(await connectArbesk(account, { stay: true }))) {
            dialog.close();
            return;
          }
        }
        dialog.setStatus("Creating collection…");
        let created: string;
        try {
          created = await arbeskCreateCollection(dir, name);
        } catch (err) {
          if (cleanError(err) !== ARBESK_CREATE_SIGN_IN) throw err;
          if (!(await connectArbesk(account, { stay: true }))) throw err;
          created = await arbeskCreateCollection(dir, name);
        }
        dialog.close();
        if (location === dir) await reload([created]);
        showToast(`Created “${name}”`);
      } catch (err) {
        message.textContent = cleanError(err);
        message.classList.add("error");
        row.classList.add("error");
        return false;
      }
    },
  });
  input.addEventListener("input", validate);
  validate();
  input.focus();
}

// ---------------------------------------------------------------------------
// Share a collection (Arbesk: add a collaborator by email)
// ---------------------------------------------------------------------------

/** The errors sharing ends with when it needs a new sign-in (or a new
 *  delegation, which Arbesk renews on the next sign-in). */
const ARBESK_SHARE_SIGN_IN = "Sign in to Arbesk again to share a collection.";
const ARBESK_SHARE_RELAY_FAILED =
  "Arbesk couldn't save the collaborator. Sign in again to renew its permission.";

/** Roles as Arbesk's editor list numbers them. */
const ARBESK_VIEWER: ArbeskRole = 1;
const ARBESK_EDITOR: ArbeskRole = 2;
/** The email shape Arbesk's own sign-in and invite fields accept. */
const ARBESK_EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/**
 * Share Collection…: add the Arbesk account behind an email as a viewer or an
 * editor. Arbesk resolves the email to a wallet address; the list and its
 * Merkle root are updated on IPFS and the chain. Sharing needs the session a
 * paste does, asked for once if it is gone.
 */
function promptShareCollection(path: string): void {
  if (!isArbeskCollection(path)) return;
  const extra = document.createElement("div");
  extra.className = "compress-fields";
  extra.innerHTML = `
    <label>Email<input id="share-collection-email" type="email" spellcheck="false" autocomplete="off" placeholder="name@example.com" /></label>
    <label>Role<select id="share-collection-role">
      <option value="${ARBESK_VIEWER}" selected>Viewer</option>
      <option value="${ARBESK_EDITOR}">Editor</option>
    </select></label>
    <div class="check-hint">Viewers can download the collection's assets, but Arbesk lists only Editors in Network for now; Editors can also add and save.</div>
    <div class="field-message"></div>`;
  const email = extra.querySelector<HTMLInputElement>("#share-collection-email")!;
  const role = extra.querySelector<HTMLSelectElement>("#share-collection-role")!;
  const message = extra.querySelector<HTMLElement>(".field-message")!;
  const row = email.closest("label")!;
  /** Cancel while the session check or sign-in is pending: start nothing. */
  let cancelled = false;

  const validate = () => {
    const value = email.value.trim();
    const valid = ARBESK_EMAIL.test(value);
    showValidation(
      { valid, message: valid || !value ? "" : "Enter an email address.", error: !valid && !!value },
      row,
      message,
    );
    dialog.setResponseEnabled("share", valid);
    return valid;
  };

  const dialog = openDialog({
    heading: "Share Collection",
    body: `Add a collaborator to “${pathName(path)}” by the email they use with Arbesk.`,
    extra,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "share", label: "Share", style: "suggested" },
    ],
    defaultResponse: "share",
    closeResponse: "cancel",
    busy: { liveWhileBusy: ["cancel"], status: message },
    onResponse: async (id) => {
      if (id === "cancel") {
        cancelled = true;
        return;
      }
      if (id !== "share" || !validate()) return;
      const value = email.value.trim();
      const chosen = Number(role.value) as ArbeskRole;
      try {
        const account = await arbeskAccountFor(path);
        if (cancelled) return;
        if (!(await arbeskSignedIn(path).catch(() => false))) {
          if (cancelled) return;
          if (!(await connectArbesk(account, { stay: true }))) {
            dialog.close();
            return;
          }
        }
        if (cancelled) return;
        dialog.setStatus("Sharing collection…");
        let shared: ArbeskShared;
        try {
          shared = await arbeskShareCollection(path, value, chosen);
        } catch (err) {
          const message = cleanError(err);
          if (
            message !== ARBESK_SHARE_SIGN_IN &&
            message !== ARBESK_SIGN_IN &&
            message !== ARBESK_SHARE_RELAY_FAILED
          ) {
            throw err;
          }
          if (cancelled) return;
          if (!(await connectArbesk(account, { stay: true }))) throw err;
          shared = await arbeskShareCollection(path, value, chosen);
        }
        dialog.close();
        const label = shared.role === ARBESK_EDITOR ? "Editor" : "Viewer";
        showToast(shared.added ? `Shared with ${value} as ${label}` : `${value} is now ${label}`);
      } catch (err) {
        message.textContent = cleanError(err);
        message.classList.add("error");
        row.classList.add("error");
        return false;
      }
    },
  });
  email.addEventListener("input", validate);
  validate();
  email.focus();
}

// ---------------------------------------------------------------------------
// Preview (Space)
// ---------------------------------------------------------------------------

let previewUrl: string | null = null;
let previewSeq = 0;
/** The 3D model on screen in the preview, released when it closes. */
let modelView: { dispose(): void } | null = null;
/** The most the 3D preview reads: the model and a .gltf's companions together. */
const MODEL_PREVIEW_LIMIT = 256 * 1024 * 1024;

function closePreview(): void {
  if (!previewLayer) return;
  resetPreviewBody();
  previewLayer.hidden = true;
  previewActionsEl.replaceChildren();
  previewName.textContent = "";
  previewName.hidden = false;
  previewVersionsEl.hidden = true;
  previewVersionsEl.replaceChildren();
  clearPreviewFiles();
}

/** Empty the preview's content (to show another version of the same item):
 *  what was loading for the old content is stale from now on. The new seq. */
function resetPreviewBody(): number {
  previewSeq += 1;
  cancelPartFetches(partTickets);
  previewLoaded();
  previewBody.replaceChildren();
  if (previewUrl) URL.revokeObjectURL(previewUrl);
  previewUrl = null;
  modelView?.dispose();
  modelView = null;
  return previewSeq;
}

/** The item's icon over a line of text: why there is no preview, or, with a
 *  spinner between them, that it is still loading. */
function previewNotice(entry: Entry, text: string, loading = false): HTMLElement {
  const box = document.createElement("div");
  box.className = loading ? "preview-empty preview-loading" : "preview-empty";
  const icon = document.createElement("div");
  icon.className = "preview-empty-icon";
  itemIcons.dialogIcon(icon, entry, 192);
  const label = document.createElement("p");
  label.textContent = text;
  const spinner = document.createElement("div");
  spinner.className = "preview-spinner";
  box.append(icon, ...(loading ? [spinner] : []), label);
  return box;
}

function previewEmpty(entry: Entry, text: string): void {
  previewLoaded();
  previewBody.appendChild(previewNotice(entry, text));
}

/** How long a preview may take before it says it is loading: one that is
 *  ready sooner never flashes the spinner. */
const PREVIEW_LOADING_DELAY = 200;
let previewLoadingTimer = 0;

/** Say "Loading…" if preview `seq` still shows nothing after the delay. */
function previewLoading(entry: Entry, seq: number): void {
  previewLoadingTimer = window.setTimeout(() => {
    if (previewStale(seq) || previewBody.childElementCount > 0) return;
    previewBody.appendChild(previewNotice(entry, "Loading…", true));
  }, PREVIEW_LOADING_DELAY);
}

/** The preview's content is here (or why there is none): no more "Loading…". */
function previewLoaded(): void {
  window.clearTimeout(previewLoadingTimer);
  previewBody.querySelector(".preview-loading")?.remove();
}

/** Each preview opened, counted: what belongs to the whole preview (its
 *  version control) and not only to its content outlives a switch to one
 *  of the save's files, which is a new `previewSeq`. */
let previewOpened = 0;

async function showPreview(entry: Entry): Promise<void> {
  if (!previewLayer) return;
  closePreview();
  previewOpened += 1;
  const seq = previewSeq;
  previewLayer.hidden = false;
  previewName.textContent = entry.name;
  previewBody.replaceChildren();
  renderPreviewActions(entry);

  if (isRemotePath(entry.path) && !previewsRemotely(locationKind(entry.path, trashPath))) {
    previewEmpty(entry, "Preview isn't available for remote items.");
    return;
  }
  if (entry.is_dir) {
    previewEmpty(entry, "Folder");
    return;
  }
  previewLoading(entry, seq);
  if (locationKind(entry.path, trashPath) === "arbesk" && modelFormat(entry.name) !== null) return previewArbeskFirst(entry, seq);
  void loadPreviewVersions(entry, seq);
  await previewEntry(entry, seq, fileKind(entry.name));
}

/**
 * An Arbesk model's first view (performance plan, phase 1): its scene is
 * asked for at once, as the asset's newest save is now, and answers that
 * save's CID; its parts and the files switcher are read by that CID, so a
 * save made meanwhile can't put another save's model beside the files.
 * The saves are read alongside and fill the version control when they
 * end: a newer save that landed meanwhile shows there, and the preview
 * stays on the save it shows. A scene that can't be read: no switcher.
 */
async function previewArbeskFirst(entry: Entry, seq: number): Promise<void> {
  const scene = arbeskScene(entry.path, null);
  const shown = scene.then(
    (answer) => answer.cid,
    () => null,
  );
  void shown.then((cid) => {
    if (cid && !previewStale(seq) && entry.representations?.count) void loadPreviewFiles(entry, cid, seq);
  });
  void loadArbeskVersions(entry, previewOpened, shown);
  await previewArbeskScene(entry, seq, null, scene);
}

/** The version control of an Arbesk model's first view, once its saves are
 *  read and its scene has said which save it shows (`shown`), at that save:
 *  while preview `opened` is open, even if one of the save's files is shown
 *  by then. A scene that can't be read: at the newest, as before. A shown
 *  save the saves read don't hold (the walk read the head before that save
 *  landed): the saves are read once more, and hold it then. */
async function loadArbeskVersions(entry: Entry, opened: number, shown: Promise<string | null>): Promise<void> {
  const read = () => previewVersionsOf(entry).catch((): VersionList => ({ versions: [], more: false, copies: new Map() }));
  const [first, cid] = await Promise.all([read(), shown]);
  const behind = cid !== null && first.versions.length > 0 && !first.versions.some((version) => version.id === cid);
  const list = behind && opened === previewOpened ? await read() : first;
  if (opened !== previewOpened || previewLayer.hidden || list.versions.length < 2) return;
  const index = cid === null ? list.versions.length - 1 : list.versions.findIndex((version) => version.id === cid);
  if (index >= 0) drawPreviewVersions(entry, list, index);
}

// ---- Versions in the preview -------------------------------------------------

/** Where the preview reads a version from: an older local version's
 *  read-only copy, or an Arbesk save. Empty: the item as it is now. */
interface PreviewAt {
  readPath?: string;
  arbeskVersion?: string | null;
}

/** An item's versions, oldest first, whether only the newest were read, and
 *  the read-only copies made for older local versions, by id, so stepping
 *  back to one reuses its copy. */
type VersionList = { versions: PreviewVersion[]; more: boolean; copies: Map<string, Promise<string>> };

/** An Arbesk asset's saves, or a file's history in a folder that keeps versions. */
async function previewVersionsOf(entry: Entry): Promise<VersionList> {
  const copies = new Map<string, Promise<string>>();
  if (locationKind(entry.path, trashPath) === "arbesk") return { versions: fromArbeskSaves(await arbeskVersions(entry.path)), more: false, copies };
  if (entry.is_symlink || !insideVersionedRoot(entry.path, versionedRootPath)) return { versions: [], more: false, copies };
  const [rows, total] = await Promise.all([listVersions(entry.path, LOCAL_VERSION_LIMIT, 0), versionCount(entry.path)]);
  return { versions: fromLocal(rows, entry.modified), more: total > rows.length, copies };
}

/** The version control, once the item's versions are known and there are
 *  two or more. */
async function loadPreviewVersions(entry: Entry, seq: number): Promise<void> {
  const list = await previewVersionsOf(entry).catch((): VersionList => ({ versions: [], more: false, copies: new Map() }));
  if (previewStale(seq)) return;
  if (list.versions.length >= 2) drawPreviewVersions(entry, list, list.versions.length - 1);
  // The asset now: its files only when the listing says it has some.
  const newest = list.versions[list.versions.length - 1];
  if (newest && entry.representations?.count) void loadPreviewFiles(entry, newest.id, seq);
}

function drawPreviewVersions(entry: Entry, list: VersionList, shown: number): void {
  // The control is the header's title now: the name goes into it.
  previewName.hidden = true;
  drawVersionControl(previewVersionsEl, entry.name, list.versions, shown, list.more, (index) => void showPreviewVersion(entry, list, index));
}

/** Show version `index` of the item in place of what the preview shows. */
async function showPreviewVersion(entry: Entry, list: VersionList, index: number): Promise<void> {
  const seq = resetPreviewBody();
  drawPreviewVersions(entry, list, index);
  // Each save keeps its own files (Arbesk R4): the switcher follows the
  // version on screen, which shows its model again.
  clearPreviewFiles();
  if (locationKind(entry.path, trashPath) === "arbesk") void loadPreviewFiles(entry, list.versions[index].id, seq);
  const refusal = versionRefusal(entry, list.versions[index]);
  if (refusal) {
    previewEmpty(entry, refusal);
    return;
  }
  previewLoading(entry, seq);
  try {
    const at = await previewAtVersion(entry, list, list.versions[index]);
    if (previewStale(seq)) return;
    await previewEntry(entry, seq, fileKind(entry.name), at);
  } catch {
    if (previewStale(seq)) return;
    previewEmpty(entry, "No preview available.");
  }
}

/** Why an older local version can't be shown, said before a copy of it is
 *  made: the preview has no viewer for its kind, or it is over the 3D limit. */
function versionRefusal(entry: Entry, version: PreviewVersion): string | null {
  if (version.current || locationKind(entry.path, trashPath) === "arbesk") return null;
  const kind = fileKind(entry.name);
  const model = modelFormat(entry.name);
  if (!model && kind !== "text" && kind !== "code" && !THUMB_KINDS.has(kind)) return "No preview available.";
  return model && (version.size ?? 0) > MODEL_PREVIEW_LIMIT ? "This file is too large to preview." : null;
}

/** Where `version` of the item is read from: an older local version is
 *  materialized once as the read-only copy Versions… → Open makes. */
async function previewAtVersion(entry: Entry, list: VersionList, version: PreviewVersion): Promise<PreviewAt> {
  // An Arbesk save by its CID, the current one's too, as its files are listed.
  if (locationKind(entry.path, trashPath) === "arbesk") return { arbeskVersion: version.id };
  if (version.current) return {};
  let copy = list.copies.get(version.id);
  if (!copy) {
    copy = openVersion(entry.path, Number(version.id), restoreName(entry.path, Math.round((version.date ?? 0) / 1000)));
    // A copy that failed is asked for again next time.
    copy.catch(() => list.copies.delete(version.id));
    list.copies.set(version.id, copy);
  }
  return { readPath: await copy };
}

function previewStale(seq: number): boolean {
  return seq !== previewSeq || Boolean(previewLayer.hidden);
}

async function previewEntry(entry: Entry, seq: number, kind: FileKind, at: PreviewAt = {}): Promise<void> {
  const model = modelFormat(entry.name);
  if (model) return previewModelEntry(entry, seq, model, at);
  if (kind === "text" || kind === "code") return previewTextEntry(entry, seq, at);
  if (THUMB_KINDS.has(kind)) return previewThumbnailEntry(entry, seq, at);
  previewEmpty(entry, "No preview available.");
}

async function previewTextEntry(entry: Entry, seq: number, at: PreviewAt): Promise<void> {
  try {
    const result = await previewText(at.readPath ?? entry.path, 200_000);
    if (previewStale(seq)) return;
    const pre = document.createElement("pre");
    pre.textContent = result.text + (result.truncated ? "\n…" : "");
    previewLoaded();
    previewBody.appendChild(pre);
  } catch {
    if (previewStale(seq)) return;
    previewEmpty(entry, "No preview available.");
  }
}

/**
 * A 3D model (glTF, GLB, STL, 3MF) you can turn and zoom. three.js loads only
 * now, in its own chunk. Without WebGL, or for a file it can't read, the
 * preview says there is none.
 */
async function previewModelEntry(entry: Entry, seq: number, named: ModelFormat, at: PreviewAt): Promise<void> {
  if (locationKind(entry.path, trashPath) === "arbesk") return previewArbeskScene(entry, seq, at.arbeskVersion ?? null);
  try {
    // A .gltf's companions come from the item's own folder, as they are now.
    const bytes = await previewBytes(at.readPath ?? entry.path, MODEL_PREVIEW_LIMIT);
    // The bytes decide over the name: an Arbesk .gltf previews as a GLB.
    const format: ModelFormat = named === "gltf" && isGlb(bytes) ? "glb" : named;
    const companions = format === "gltf" ? await readGltfCompanions(entry.path, bytes) : new Map<string, ArrayBuffer>();
    const viewer = await import("./model-viewer");
    const model = await viewer.loadModel(format, bytes, companions);
    if (previewStale(seq)) return;
    previewLoaded();
    modelView = viewer.showModel(previewBody, model);
  } catch (err) {
    if (previewStale(seq)) return;
    previewEmpty(entry, /too large/i.test(String(err)) ? "This file is too large to preview." : "No preview available.");
  }
}

/**
 * An Arbesk asset is a scene: the backend resolves its parts and where each
 * goes; each part is loaded once and copied to every place. A part that
 * fails is left out; with none, the preview says why.
 */
async function previewArbeskScene(entry: Entry, seq: number, version: string | null, asked?: Promise<ArbeskScene>): Promise<void> {
  // The scene is asked for before three.js loads (the first view's already
  // is); a refusal that comes first waits, handled, for loadArbeskParts.
  const scene = asked ?? arbeskScene(entry.path, version);
  void scene.catch(() => undefined);
  const viewer = await import("./model-viewer");
  const { parts, tooLarge } = await loadArbeskParts(entry.path, scene, seq, viewer).catch(() => ({ parts: [], tooLarge: false }));
  if (previewStale(seq)) return;
  if (!parts.length) {
    previewEmpty(entry, tooLarge ? "This file is too large to preview." : "No preview available.");
    return;
  }
  try {
    previewLoaded();
    modelView = viewer.showModel(previewBody, viewer.placeScene(parts));
  } catch {
    // No WebGL: as for any model.
    previewEmpty(entry, "No preview available.");
  }
}

/** The preview's part fetches under way, by ticket: a closed or stepped
 *  preview cancels them (performance plan, phase 3). */
const partTickets = new Set<number>();
// From the clock at load: a reload never reuses a ticket the backend may
// still hold a cancel for (`part_tickets.rs`).
let partTicketSeq = Date.now();

/** Stop these part fetches, queued or reading. */
function cancelPartFetches(tickets: Iterable<number>): void {
  const stopped = [...tickets];
  if (stopped.length === 0) return;
  stopped.forEach((ticket) => partTickets.delete(ticket));
  void cancelArbeskParts(stopped).catch(() => undefined);
}

/** One part's bytes under a ticket of its own, kept in `mine` while it runs. */
async function fetchArbeskPart(path: string, key: string, limit: number, version: string, mine: Set<number>): Promise<ArrayBuffer> {
  const ticket = ++partTicketSeq;
  partTickets.add(ticket);
  mine.add(ticket);
  try {
    return await arbeskPart(path, key, limit, version, ticket);
  } finally {
    partTickets.delete(ticket);
    mine.delete(ticket);
  }
}

/** Each part of the scene at `path`, loaded once, at most MODEL_PREVIEW_LIMIT
 *  bytes in all, from the save the scene was resolved from (its CID); none
 *  more once the preview has moved on. A few are fetched at once
 *  (`fetchParts`: the limit reserved per call in flight) and parsed one at
 *  a time in the scene's order, so the parts and their placements are the
 *  ones fetching them in turn gives. */
async function loadArbeskParts(path: string, scene: Promise<ArbeskScene>, seq: number, viewer: typeof import("./model-viewer")) {
  const { parts: stored, cid: version } = await scene;
  const loaded: ({ object: import("three").Object3D; placements: import("./model-viewer").ScenePlacement[] } | undefined)[] = [];
  const mine = new Set<number>();
  const { tooLarge } = await fetchParts({
    count: stored.length,
    budget: MODEL_PREVIEW_LIMIT,
    fetch: (index, limit) => fetchArbeskPart(path, stored[index].key, limit, version, mine),
    take: async (index, bytes) => {
      loaded[index] = { object: await viewer.loadScenePart(bytes), placements: stored[index].placements };
    },
    stale: () => previewStale(seq),
  });
  // Calls still under way once the budget ran out are no longer wanted.
  cancelPartFetches(mine);
  return { parts: loaded.filter((part) => part !== undefined), tooLarge };
}

/** The files a .gltf names in its own folder or below, by the URI it names
 *  them with; with the model, at most MODEL_PREVIEW_LIMIT bytes. */
async function readGltfCompanions(path: string, bytes: ArrayBuffer): Promise<Map<string, ArrayBuffer>> {
  const dir = parentPath(path) ?? path;
  const found = new Map<string, ArrayBuffer>();
  let left = MODEL_PREVIEW_LIMIT - bytes.byteLength;
  for (const uri of gltfCompanions(JSON.parse(new TextDecoder().decode(bytes)))) {
    const target = companionPath(uri)!.split("/").reduce(childPath, dir);
    const data = await previewBytes(target, Math.max(left, 1));
    left -= data.byteLength;
    found.set(uri, data);
  }
  return found;
}

async function previewThumbnailEntry(entry: Entry, seq: number, at: PreviewAt): Promise<void> {
  try {
    const bytes = await thumbnail(at.readPath ?? entry.path, 1024);
    if (previewStale(seq)) return;
    if (!bytes.byteLength) {
      previewEmpty(entry, "No preview available.");
      return;
    }
    previewUrl = URL.createObjectURL(new Blob([bytes], { type: thumbType(bytes) }));
    const image = document.createElement("img");
    image.src = previewUrl;
    image.alt = entry.name;
    previewLoaded();
    previewBody.appendChild(image);
  } catch {
    if (previewStale(seq)) return;
    previewEmpty(entry, "No preview available.");
  }
}

// ---- An Arbesk save's other files in the preview (spec F14) -----------------

/** The switcher's save on screen: the asset, the save (its manifest CID),
 *  its files and which is shown (-1: the model). */
interface PreviewFiles {
  entry: Entry;
  version: string;
  files: RepresentationInfo[];
  shown: number;
}

/** Each of the preview's file fetches gets its own operation id (no
 *  row: the preview says "Loading…"), unique across the app's windows:
 *  the backend's progress registry is app-wide, and two windows sharing
 *  an id would let the first fetch to end unregister the other, so
 *  Restart would not wait for it. A fetch the preview moved on from
 *  finishes into the app cache, as a scene's parts do, and is dropped: a
 *  cancel sent after it ended would be remembered for an operation that
 *  never comes, holding up Restart. */
let previewFetchSeq = 0;
function previewFetchOp(): string {
  // A per-window random part (getRandomValues needs no secure context, as
  // randomUUID does), the time and a count, as operations.ts' ids.
  const salt = Array.from(crypto.getRandomValues(new Uint32Array(2)), (n) => n.toString(36)).join("");
  return `preview-file-${Date.now().toString(36)}-${salt}-${++previewFetchSeq}`;
}

function clearPreviewFiles(): void {
  previewFilesEl.hidden = true;
  previewFilesEl.replaceChildren();
}

/** The files of the save `version` (a manifest CID) of the asset; the
 *  switcher when it has some. A list that can't be read shows none. */
async function loadPreviewFiles(entry: Entry, version: string, seq: number): Promise<void> {
  const files = await arbeskRepresentations(entry.path, version).catch((): RepresentationInfo[] => []);
  if (previewStale(seq) || files.length === 0) return;
  drawPreviewFiles({ entry, version, files, shown: -1 });
}

function drawPreviewFiles(state: PreviewFiles): void {
  drawFilesSwitcher(previewFilesEl, state.entry.name, state.files, state.shown, (index) => void showPreviewFile(state, index));
}

/** An asset's file as an entry, for its icon and the Open With title. */
function fileEntry(asset: Entry, rep: RepresentationInfo): Entry {
  return { ...asset, name: rep.name, path: rep.path, representations: undefined };
}

/** Show the model (-1) or file `index` of the save on screen: a file the
 *  viewer draws as a 3D model, any other on its file page. */
async function showPreviewFile(state: PreviewFiles, index: number): Promise<void> {
  const seq = resetPreviewBody();
  const next = { ...state, shown: index };
  drawPreviewFiles(next);
  const { entry, version } = next;
  if (index < 0) {
    previewLoading(entry, seq);
    // The shown save's manifest CID, the current one's too: the model and
    // the files listed beside it come from the same save, even if the
    // asset is saved again while the preview is open.
    await previewArbeskScene(entry, seq, version);
    return;
  }
  const rep = next.files[index];
  const model = modelFormat(`file.${rep.format}`);
  if (!model) {
    showFilePage(next, rep);
    return;
  }
  previewLoading(iconEntry(rep.name), seq);
  await previewFileModel(next, rep, model, seq);
}

/** A file of the save on screen as a 3D model: fetched from that save's
 *  manifest (never the current save's path) into the app cache, then read
 *  as a local model. A glTF that loads files beside it gets the file page. */
async function previewFileModel(state: PreviewFiles, rep: RepresentationInfo, named: ModelFormat, seq: number): Promise<void> {
  try {
    const file = await arbeskFile(state.entry.path, state.version, rep.id, { op: previewFetchOp() });
    if (previewStale(seq)) return;
    const bytes = await previewBytes(file, MODEL_PREVIEW_LIMIT);
    if (previewStale(seq)) return;
    const format: ModelFormat = named === "gltf" && isGlb(bytes) ? "glb" : named;
    if (format === "gltf" && gltfExternalUris(JSON.parse(new TextDecoder().decode(bytes))).length > 0) {
      showFilePage(state, rep);
      return;
    }
    const viewer = await import("./model-viewer");
    const model = await viewer.loadModel(format, bytes, new Map());
    if (previewStale(seq)) return;
    previewLoaded();
    modelView = viewer.showModel(previewBody, model);
  } catch (err) {
    if (previewStale(seq)) return;
    previewEmpty(iconEntry(rep.name), /too large/i.test(String(err)) ? "This file is too large to preview." : "No preview available.");
  }
}

/** The file page: what the file is, and Open With… for it. */
function showFilePage(state: PreviewFiles, rep: RepresentationInfo): void {
  previewLoaded();
  const icon = (box: HTMLElement) => itemIcons.dialogIcon(box, iconEntry(rep.name), 192);
  previewBody.replaceChildren(filePage(rep, state.files, icon, () => openPreviewFileWith(state, rep)));
}

/** Open With… on the file page (spec F8, the file chosen): fetched from
 *  the save on screen, as the page shows it. */
function openPreviewFileWith(state: PreviewFiles, rep: RepresentationInfo): void {
  if (refuse("openWith", hereFor([state.entry.path]))) return;
  showOpenWith(state.entry, { kind: "file", rep }, fileEntry(state.entry, rep), state.version);
}

// ---- Preview actions (#48) -------------------------------------------------


/** Close the preview and give the keyboard back to the view. */
function leavePreview(): void {
  closePreview();
  viewEl.focus();
}

const PREVIEW_RUN: Record<PreviewAction, (entry: Entry) => void> = {
  location: (entry) => {
    leavePreview();
    openItemLocation(entry);
  },
  rename: (entry) => showPreviewRename(entry),
  trash: (entry) => {
    leavePreview();
    if (!refuse("trash", hereFor([entry.path]))) void trashPaths([entry.path]);
  },
  open: (entry) => {
    leavePreview();
    activate(entry);
  },
};

/** Whether `entry` is shown away from its folder: a search result, or an item
 *  in Recent or Starred. Network's entries are its own, though a server's
 *  path is where it starts on the server, so its parent isn't Network. */
function shownElsewhere(entry: Entry): boolean {
  return search.isOpen || (location !== NETWORK && parentOf(entry.path) !== location);
}

function renderPreviewActions(entry: Entry): void {
  const actions = previewActions(previewPlace(hereFor([entry.path], shownElsewhere(entry))));
  const buttons: HTMLElement[] = actions.map((action) => {
    const button = document.createElement("button");
    button.className = action === "open" ? "btn suggested" : "btn";
    button.textContent = PREVIEW_ACTION_LABELS[action];
    button.addEventListener("click", () => PREVIEW_RUN[action](entry));
    return button;
  });
  // Open Item Location sits apart on the left; the rest are on the right.
  const gap = document.createElement("span");
  gap.className = "preview-actions-gap";
  buttons.splice(actions[0] === "location" ? 1 : 0, 0, gap);
  previewActionsEl.replaceChildren(...buttons);
}

/**
 * Rename from the preview: a dialog, since the preview covers the item the
 * rename popover points at. The preview stays open on the renamed item.
 * It doesn't ask the rename rule: `previewPlace` already decided to offer
 * it, keeping the Trash folder's exception (src/preview-actions.ts).
 */
function showPreviewRename(entry: Entry): void {
  const noun = entry.is_dir ? "Folder" : "File";
  const extra = document.createElement("div");
  extra.innerHTML = `
    <div class="entry-row"><label for="preview-rename-name">${noun} Name</label><input id="preview-rename-name" type="text" spellcheck="false" /></div>
    <div class="field-message" style="margin-top:8px"></div>`;
  const row = extra.querySelector<HTMLElement>(".entry-row")!;
  const input = extra.querySelector<HTMLInputElement>("input")!;
  const message = extra.querySelector<HTMLElement>(".field-message")!;
  input.value = entry.name;

  const validate = () => {
    const v = validateName(input.value, entry.is_dir, entry.name, siblingsOf(entry));
    showValidation(v, row, message);
    dialog.setResponseEnabled("rename", v.valid);
    return v.valid;
  };

  const dialog = openDialog({
    heading: `Rename ${noun}`,
    extra,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "rename", label: "Rename", style: "suggested" },
    ],
    defaultResponse: "rename",
    busy: { liveWhileBusy: ["cancel"] },
    onResponse: async (id) => {
      if (id !== "rename") return;
      if (!validate()) return false;
      let renamed: string;
      try {
        renamed = await renameInPlace(entry, input.value);
      } catch (err) {
        showValidation({ valid: false, message: cleanError(err), error: true }, row, message);
        return false;
      }
      pathRefs.moved([{ from: entry.path, to: renamed }]);
      await reload([renamed]);
      void showPreview({ ...entry, name: basename(renamed), path: renamed });
    },
  });
  input.addEventListener("input", validate);
  validate();
  const dot = entry.name.lastIndexOf(".");
  input.setSelectionRange(0, !entry.is_dir && dot > 0 ? dot : entry.name.length);
  input.focus();
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

/** "Open With…" for the folder on screen, as GNOME's background and path-bar
 * menus offer it, through the same rule as an item's. */
function openCurrentFolderWithItem(): MenuItem {
  return { label: "Open With…", action: () => openItemWith(folderEntry(location)) };
}

/** Open With… on an item, as `item-actions.ts`' rule allows it. */
function openItemWith(entry: Entry): void {
  if (refuse("openWith", hereFor([entry.path]))) return;
  showOpenWith(entry);
}

/** "Open With…" (src/open-with.ts), with the app's icons and Recent. For
 *  an Arbesk asset the chosen application gets one of its files, fetched
 *  first (`openAssetWith`): `chosen` when the Files… page named it, else
 *  the one the application opens, asked when it opens several (spec F8). */
function showOpenWith(entry: Entry, chosen?: FileChoice, title = entry, version?: string): void {
  const arbesk = isArbeskAsset(entry.path);
  showOpenWithDialog(title, {
    icon(span, app) {
      const bundle = { name: `${app.name}.app`, path: "", is_dir: true, size: 0, modified: 0, is_hidden: false, is_symlink: false };
      itemIcons.dialogIcon(span, bundle, 28, appIconKey(app.id));
    },
    errorText: cleanError,
    opened(file) {
      if (!isRemotePath(file.path)) addRecent(file.path).catch(() => {});
    },
    ...(arbesk ? { launch: (app: AppChoice) => openAssetWith(entry, app, chosen, version) } : {}),
  });
}

// ---------------------------------------------------------------------------
// Open With for an Arbesk asset (spec F8)
// ---------------------------------------------------------------------------

/** Which of `asset`'s files `app` gets: the one it opens, the model when
 *  it declares none, or the user's choice when it opens several (null:
 *  cancelled). The files' list is asked only when the app opens one of
 *  the formats the listing's summary names. */
async function fileForApp(asset: Entry, app: AppChoice): Promise<FileChoice | null> {
  const model = assetFormat(asset.name);
  const kept = asset.representations?.formats ?? [];
  const opens = await appFormats(app.id, [model, ...kept]);
  const files = opens.some((format) => kept.includes(format)) ? await arbeskRepresentations(asset.path) : [];
  const candidates = openWithCandidates(model, files, opens);
  return onlyChoice(candidates) ?? chooseFileToOpen(asset, app.name, candidates, files);
}

/** Hand one of `asset`'s files to `app`: fetched into the app cache as an
 *  operation (progress, Cancel), then opened with it. A failure is a toast;
 *  a cancel says nothing; the cached file never goes to Recent. A file of
 *  the save `version` (the preview's file page) is read from that save. */
async function openAssetWith(asset: Entry, app: AppChoice, chosen?: FileChoice, version?: string): Promise<void> {
  let choice: FileChoice | null;
  try {
    choice = chosen ?? (await fileForApp(asset, app));
  } catch (err) {
    showToast(`Could not open “${asset.name}”: ${cleanError(err)}`);
    return;
  }
  if (!choice) return;
  const [name, path] = choice.kind === "model" ? [asset.name, asset.path] : [choice.rep.name, choice.rep.path];
  const op = startOperation(opTitle("open", [name]), {
    totalItems: 1,
    onCancel: () => void cancelOperation(op.id).catch(() => {}),
  });
  setOperationState(op, "working");
  try {
    const tracked = { op: op.id, onProgress: (u: ProgressUpdate) => updateOperation(op, u) };
    const file =
      choice.kind === "file" && version !== undefined
        ? await arbeskFile(asset.path, version, choice.rep.id, tracked)
        : await arbeskExport(path, tracked);
    await openWith(file, app.id);
  } catch (err) {
    if (cleanError(err) !== CANCELLED) showToast(`Could not open “${name}”: ${cleanError(err)}`);
  } finally {
    endOperation(op);
  }
}

/** What the Files… page's rows do for `asset` (spec F7): Open With… a
 *  chosen file, and Copy, which puts its pinned path (the model: the
 *  asset's) on the clipboard. */
function filesActions(asset: Entry): FilesActions {
  const named = (choice: FileChoice): Entry => (choice.kind === "model" ? asset : fileEntry(asset, choice.rep));
  return {
    openWith: (choice) => {
      if (!refuse("openWith", hereFor([asset.path]))) showOpenWith(asset, choice, named(choice));
    },
    copy: (choice) => putOnClipboard([choice.kind === "model" ? asset.path : choice.rep.path], new Set(), "copy"),
    remove: (rep) => removeAssetFile(asset, rep),
  };
}

// ---------------------------------------------------------------------------
// Adding files to an Arbesk asset and removing them (spec F10–F12)
// ---------------------------------------------------------------------------

/** The Files… page open now, which a write to its asset draws again. */
let filesPage: { asset: string; page: FilesPage } | null = null;

function openFilesPage(asset: Entry, files: RepresentationInfo[]): void {
  const page = showFilesDialog(asset, files, filesActions(asset));
  filesPage = { asset: asset.path, page };
}

/** After a write to `asset` (spec F11): its row's emblem from the new
 *  summary at once, an open Files… page drawn again, then the collection
 *  reloaded (the write waited for it to settle, so it lists the new one). */
async function afterAssetWrite(asset: string, done: Attached): Promise<void> {
  const row = shown.find((entry) => entry.path === asset);
  if (row) {
    row.representations = done.representations ?? undefined;
    patchIcon(asset);
  }
  if (filesPage?.asset === asset) filesPage.page.redraw(done.files);
  if (isUnder(asset, location)) await reload();
}

/** The errors a write to an asset ends with when it needs a new sign-in. */
const ATTACH_SIGN_IN = "Sign in to Arbesk again to add files to it.";
const DETACH_SIGN_IN = "Sign in to Arbesk again to remove files from it.";

/** Paste Into Asset: the clipboard's files added to `asset`. */
function pasteIntoAsset(asset: Entry): void {
  if (!clipboard) return;
  const { paths, dirs, mode } = clipboard;
  void attachFiles(asset.path, paths, dirs, mode === "cut");
}

/** Why the local `.gltf` at `path` can't be added: it names a buffer or
 *  image by any URI but a `data:` one (beside it, above its folder, an
 *  absolute `file:` URI), so the one file stored wouldn't load; or its
 *  bytes aren't JSON to tell. Null for anything else, a file that can't
 *  be read too: the plan reads every file again and names why (too large,
 *  gone, not allowed), and refuses it. */
async function gltfRefusal(path: string): Promise<string | null> {
  if (!/\.gltf$/i.test(path)) return null;
  let bytes: ArrayBuffer;
  try {
    bytes = await previewBytes(path, MODEL_PREVIEW_LIMIT);
  } catch {
    return null;
  }
  let gltf: unknown;
  try {
    gltf = JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    return `“${basename(path)}” couldn't be read as glTF. Add a .glb instead.`;
  }
  return gltfExternalUris(gltf).length > 0 ? `“${basename(path)}” uses files beside it. Add a .glb instead.` : null;
}

/** For each file of the plan whose id a file the asset shows has, what
 *  the user answers; null when the paste stops (Escape). */
async function answerClashes(planned: PlannedFile[], assetName: string): Promise<Map<string, AttachChoice> | null> {
  const answers = new Map<string, AttachChoice>();
  const clashing = planned.filter((file) => file.clash?.kind === "valid");
  let all: ClashChoice | null = null;
  for (const [i, file] of clashing.entries()) {
    const clash = file.clash as Extract<PlannedFile["clash"], { kind: "valid" }>;
    let choice: ClashChoice | null = all;
    if (choice === null) {
      const answer = await askAttachClash(assetName, clash.existing, file.name, clashing.length - i - 1);
      if (!answer) return null;
      choice = answer.choice;
      if (answer.all) all = choice;
    }
    answers.set(file.path, choice === "replace" ? { kind: "replace", cid: clash.cid } : { kind: choice });
  }
  return answers;
}

/**
 * Paste Into Asset and a drop on an asset (spec F10): refuse what can't be
 * added, in one toast; settle each clash with a file the asset has; confirm
 * the files go public; sign in when needed; then add them as one new
 * version, as an operation. Nothing is written when every file is skipped
 * or refused.
 */
async function attachFiles(asset: string, paths: string[], dirs: ReadonlySet<string>, moving: boolean): Promise<void> {
  if (refuse("attach", { kind: locationKind(asset, trashPath), items: [], paths, dir: asset, moving, dirs })) return;
  const assetName = pathName(asset);
  for (const path of paths) {
    const refusal = await gltfRefusal(path);
    if (refusal) return showToast(refusal);
  }
  let planned: PlannedFile[];
  try {
    planned = await arbeskAttachPlan(asset, paths);
  } catch (err) {
    return showToast(cleanError(err));
  }
  // libadwaita shows one toast at a time: the refusals go in one.
  const refused = planned.flatMap((file) => (file.clash?.kind === "unreadable" ? [file.clash.message] : []));
  if (refused.length) showToast(refused.join(" "));
  const answers = await answerClashes(planned, assetName);
  if (!answers) return;
  const adding = planned
    .filter((file) => file.clash?.kind !== "unreadable" && answers.get(file.path)?.kind !== "skip")
    .map((file) => ({ path: file.path, choice: answers.get(file.path) }));
  if (adding.length === 0) return showToast([...refused, `Nothing was added to “${assetName}”.`].join(" "));
  const sources = planned.filter((file) => file.role === "source" && adding.some((add) => add.path === file.path)).map((file) => file.name);
  if (!(await confirmAttach(assetName, sources))) return;
  if (!(await arbeskSignedIn(asset).catch(() => false)) && !(await connectArbesk(await arbeskAccountFor(asset), { stay: true }))) return;
  await runAttach(asset, assetName, adding);
}

/** The attach as an operation, *Adding 2 files to “Box.gltf”*: progress
 *  over the uploaded bytes, Cancel until the collection is written. A lost
 *  session signs in again once and carries on. */
async function runAttach(asset: string, assetName: string, adding: Array<{ path: string; choice?: AttachChoice }>): Promise<void> {
  const what = adding.length === 1 ? `“${basename(adding[0].path)}”` : `${adding.length} files`;
  const op = startOperation(`Adding ${what} to “${assetName}”`, {
    totalItems: 1,
    onCancel: () => void cancelOperation(op.id).catch(() => {}),
  });
  setOperationState(op, "working");
  const attempt = () => arbeskAttach(asset, adding, { op: op.id, onProgress: (u) => updateOperation(op, u) });
  try {
    let done: Attached;
    try {
      done = await attempt();
    } catch (err) {
      if (cleanError(err) !== ATTACH_SIGN_IN) throw err;
      const account = await arbeskAccountFor(asset);
      if (account) signInNeeded.add(accountRef(account));
      if (!(await connectArbesk(account, { stay: true }))) return;
      done = await attempt();
    }
    if (done.refused.length) showToast(done.refused.join(" "));
    await afterAssetWrite(asset, done);
  } catch (err) {
    if (cleanError(err) !== CANCELLED) showToast(`Could not add to “${assetName}”: ${cleanError(err)}`);
    if (isUnder(asset, location)) void reload();
  } finally {
    endOperation(op);
  }
}

/** Remove… on a file of the Files… page (spec F12): asked first, Cancel
 *  the default; the write waits in the dialog (Cancel live until the
 *  collection is written), its failure said there. Only the file the row
 *  shows is removed (its path's pin), never one that replaced it since. A
 *  lost session marks the account as needing a sign-in, as an attach does. */
function removeAssetFile(asset: Entry, rep: RepresentationInfo): void {
  let op: string | null = null;
  confirmRemoveFile(
    asset.name,
    rep,
    async () => {
      op = `remove-${Date.now().toString(36)}`;
      let done: Attached;
      try {
        done = await arbeskDetach(asset.path, rep.id, arbeskFilePin(rep.path) ?? "", { op });
      } catch (err) {
        if (cleanError(err) === DETACH_SIGN_IN) {
          const account = await arbeskAccountFor(asset.path);
          if (account) signInNeeded.add(accountRef(account));
        }
        throw err;
      }
      await afterAssetWrite(asset.path, done);
    },
    () => {
      if (op) void cancelOperation(op).catch(() => {});
    },
  );
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
  if (extend && selection.anchor >= 0) selectRange(selection.anchor, to, false);
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
  selection.toggle(k.cur);
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
    else moveCursor(k.cur < 0 ? 0 : stepRow(layout, k.cur, 1), k.e.shiftKey);
    return true;
  },
  ArrowUp: (k) => {
    if (k.e.altKey || k.mod) return false;
    moveCursor(k.cur < 0 ? 0 : stepRow(layout, k.cur, -1), k.e.shiftKey);
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
    else if (search.isOpen) closeSearch();
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
  if (handler({ e, mod, cur: selection.cursor, cols, page })) e.preventDefault();
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
  [(k) => plainMod(k, "d"), () => setBookmarked(location, !pins.includes(location))],
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
  [(k) => plainMod(k, "c") && selection.size > 0, () => setClipboard("copy")],
  [(k) => plainMod(k, "x") && selection.size > 0, () => setClipboard("cut")],
  [(k) => plainMod(k, "v") && !!clipboard && !isPseudo(location), () => paste(location)],
  [(k) => plainMod(k, "i") && selection.size === 1, showSelectedProperties],
];

function showSelectedProperties(): void {
  const entry = selectedEntries()[0];
  if (entry) void showProperties(entry);
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
      // Nothing else responds while the name is saved; a failure stays in the dialog.
      busy: { status: message },
      onResponse: async (id) => {
        if (id !== "save") {
          resolve(false);
          return;
        }
        const name = input.value.trim();
        if (!name) {
          message.textContent = "Enter a name.";
          return false;
        }
        await setShareName(name);
        // Closed before the Share dialog opens on top.
        dialog.close();
        resolve(true);
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
  if (!entry.is_dir || isRemotePath(entry.path) || inTrash()) return;
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
    await dialog.run(async () => {
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
      }
    });
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
    if (dialog.busy) return;
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
    body:
      "People with File Browser on macOS or Linux can open this folder read-only, from Network, " +
      "while File Browser runs here and until you stop sharing. " +
      "To send files to a phone or any browser instead, use Send with QR Code… on the files.",
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
    // Nothing else responds while a link is made, Escape included.
    busy: {},
    onResponse: (id) => {
      if (id === "anyone" || id === "person") {
        void createLink(id).then(() => refreshShares().then(() => renderWindow(true)));
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

/** An Entry for a folder known only by its path, such as the current one. */
function folderEntry(path: string): Entry {
  return { name: basename(path), path, is_dir: true, size: 0, modified: 0, is_hidden: false, is_symlink: false };
}

function showShareDialogForPath(path: string): void {
  void showShareDialog(folderEntry(path));
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

/** One sign-in prompt at a time, so a burst of failed listings opens one dialog. */
let driveSignInPrompt = false;

/**
 * A Drive listing failed because the connection lost its credential: open the
 * dialog that fixes it, and after signing in show the folder that failed.
 * Without a client, that is the client setup dialog (with the guide) first.
 */
async function offerDriveSignIn(need: DriveAuthNeed, loc: string): Promise<void> {
  if (!accountOf(loc, accounts)) await refreshAccounts();
  const failed = accountOf(loc, accounts);
  // Online Accounts marks it "Sign in again" until it signs in.
  if (failed) signInNeeded.add(accountRef(failed));
  if (driveSignInPrompt || document.querySelector(".dialog-layer")) return;
  driveSignInPrompt = true;
  const after = async (account: DriveAccount) => {
    // The account may have been re-added under a new id; its root is where to
    // go then, not the stale Drive path that failed.
    if (cloudAccountKey(loc) !== cloudAccountKey(account.root_path)) await navigate(account.root_path);
    else if (location === loc) await reload([]);
    else await navigate(loc);
  };
  try {
    const status = await driveClientStatus().catch(() => null);
    if (!status?.configured) {
      openClientSetup(after);
      return;
    }
    const account = failed ?? accounts.find((item) => item.provider === "gdrive");
    // No account to sign in again: connect one with the client already set.
    if (account) openConnectDialog(account, after, need === "client");
    else openConnectDialog(null, after);
  } finally {
    driveSignInPrompt = false;
  }
}

/** Sign a Google account in again, from Online Accounts. */
async function signInDrive(account: Account): Promise<void> {
  const status = await driveClientStatus().catch(() => null);
  if (status?.configured) openConnectDialog(account);
  else openClientSetup();
}

/**
 * A token belongs to the client that issued it, so every Google account is
 * disconnected before the client can change. That needs asking first when
 * it removes an account that still works (anything but `current` whose
 * sign-in already failed).
 */
async function changeDriveClient(current: Account | null, before: () => void = () => {}): Promise<void> {
  await refreshAccounts();
  const google = accounts.filter((account) => account.provider === "gdrive");
  const working = google.filter((account) => !(account.id === current?.id && signInNeeded.has(accountRef(account))));
  if (working.length && !(await confirmChangeDriveClient(google))) return;
  try {
    for (const account of google) {
      await driveDisconnect(account.id);
      forgetAccount(account);
    }
    await driveClientClear();
    before();
    openClientSetup();
  } catch (err) {
    showToast(cleanError(err));
  }
}

function confirmChangeDriveClient(google: Account[]): Promise<boolean> {
  const who = google.length === 1 ? `“${google[0].email}” is` : `${google.length} Google accounts are`;
  return new Promise((resolve) => {
    openDialog({
      heading: "Change Google Client?",
      body: `${who} disconnected first, because a sign-in belongs to the client that made it. Files in Google Drive are not deleted.`,
      responses: [
        { id: "cancel", label: "Cancel" },
        { id: "change", label: "Disconnect and Change", style: "destructive" },
      ],
      defaultResponse: "cancel",
      closeResponse: "cancel",
      onResponse: (id) => resolve(id === "change"),
    });
  });
}

/** Drop an account the backend no longer has. */
function forgetAccount(account: Account): void {
  accounts = withoutAccount(accounts, account);
  signInNeeded.delete(accountRef(account));
}

/** What each provider does: its own dialogs, behind Online Accounts and Network's menus. */
const PROVIDER_ACTIONS: Record<ProviderId, ProviderActions> = {
  gdrive: {
    connect: () => void connectDrive(),
    signIn: (account) => void signInDrive(account),
    disconnect: (account) => confirmDisconnectDrive(account),
    links: (account) => [{ label: "Change Client…", action: () => void changeDriveClient(account) }],
  },
  arbesk: {
    connect: () => void connectArbesk(),
    // The session copies into Arbesk need; connecting again is also how an email changes.
    signIn: (account) => void connectArbesk(account),
    disconnect: (account) => confirmDisconnectArbesk(account),
    links: () => [],
  },
};

const ONLINE_ACCOUNTS: OnlineAccountsHost = {
  accounts: () => accounts,
  needsSignIn: (account) => signInNeeded.has(accountRef(account)),
  actions: PROVIDER_ACTIONS,
  open: (account) => void navigate(account.rootPath),
};

/** Online Accounts…, from the primary menu, Network's menus and its empty state. */
async function openOnlineAccounts(): Promise<void> {
  // Accounts load with Network; from elsewhere they may not be known yet.
  await refreshAccounts();
  showOnlineAccounts(ONLINE_ACCOUNTS);
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
function openClientSetup(after?: (account: DriveAccount) => Promise<void>): void {
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
  const links = dialogLinks([
    { label: "Open Guide", action: () => void openExternal(DRIVE_GUIDE_URL) },
    { label: "Choose File…", action: () => file.click() },
  ]);
  extra.append(links, json, file, error);

  const submit = async () => {
    await driveClientSet(json.value);
    dialog.close();
    openConnectDialog(null, after);
  };
  file.addEventListener("change", async () => {
    const chosen = file.files?.[0];
    if (!chosen) return;
    json.value = await chosen.text();
    file.value = "";
    await dialog.run(submit);
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
    // Cancel and the guide stay live while the client is checked; a failure
    // is shown, cleaned, under the paste box.
    busy: { liveWhileBusy: ["cancel", links.querySelector("button")!], status: error },
    onResponse: async (id) => {
      if (id !== "add") return;
      await submit();
    },
  });
  json.focus();
}

/**
 * The sign-in dialog. With `reconnect`, it signs the same account in again
 * (its id stays, so its paths do), which is what an expired token needs;
 * without it, it connects an account for the first time.
 */
function openConnectDialog(
  reconnect: Account | null = null,
  after?: (account: DriveAccount) => Promise<void>,
  clientProblem = false
): void {
  let flowId: string | null = null;
  const extra = document.createElement("div");
  const status = document.createElement("p");
  status.className = "dialog-text";
  status.textContent =
    "File Browser asks Google for access to see, edit, create and delete your Drive files. The token is kept in your system keychain. " +
    "Google warns that your client is unverified: choose Advanced, then continue.";
  const links = dialogLinks([
    { label: "Open Guide", action: () => void openExternal(DRIVE_GUIDE_URL) },
    {
      label: "Change Client…",
      action: () => {
        if (dialog.busy) return;
        void changeDriveClient(reconnect, () => dialog.close());
      },
    },
  ]);
  extra.append(status, links);
  const dialog = openDialog({
    heading: reconnect ? "Sign In to Google Drive" : "Connect to Google Drive",
    body: !reconnect
      ? "Google opens in your browser. This app never sees your password."
      : clientProblem
        ? `Google no longer accepts the OAuth client for “${reconnect.email}”. Use Change Client… with a freshly downloaded JSON file, or sign in again.`
        : `The Google Drive session for “${reconnect.email}” expired or was revoked. Sign in again to keep browsing its files.`,
    extra,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "connect", label: reconnect ? "Sign In" : "Open Browser", style: "suggested" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    // Cancel stays live while Google waits: it cancels the sign-in.
    busy: { liveWhileBusy: ["cancel", links.querySelector("button")!] },
    onResponse: async (id) => {
      if (id === "cancel") {
        if (flowId) await driveConnectCancel(flowId).catch(() => {});
        return;
      }
      try {
        const start = await driveConnectStart();
        flowId = start.flow_id;
        status.textContent = "Waiting for Google…";
        await openExternal(start.auth_url);
        const account = await driveConnectFinish(start.flow_id, reconnect?.id);
        accounts = withAccount(accounts, fromDrive(account));
        signInNeeded.delete(accountRef(fromDrive(account)));
        dialog.close();
        showToast(reconnect ? `Signed in to ${account.email}` : `Connected to ${account.email}`);
        if (after) await after(account);
        else if (reconnect) await reload([]);
        else await navigate(account.root_path);
      } catch (err) {
        const message = cleanError(err);
        if (/cancelled/i.test(message)) {
          // As before, a cancelled sign-in leaves Open Browser off.
          dialog.setResponseEnabled("connect", false);
        } else {
          status.textContent = message;
          showToast(message);
        }
        return false;
      }
    },
  });
}

/**
 * Connect an Arbesk account as `besk login` does: the email, then Arbesk's
 * own page in the browser, where the code Arbesk emails is typed. The app
 * keeps the email and wallet address Arbesk confirms, and the session for
 * copying into it (in the keychain). Resolves true once connected, false on
 * Cancel; with `stay` (a paste signing in) it doesn't open the account.
 */
function connectArbesk(reconnect?: Account, opts: { stay?: boolean } = {}): Promise<boolean> {
  return new Promise((resolve) => openArbeskConnect(reconnect, opts.stay ?? false, resolve));
}

function openArbeskConnect(reconnect: Account | undefined, stay: boolean, resolve: (connected: boolean) => void): void {
  let flowId: string | null = null;
  const extra = document.createElement("div");
  extra.className = "connect-fields";
  extra.innerHTML =
    `<label>Email<input class="entry" id="arbesk-email" type="email" spellcheck="false" autocomplete="email" placeholder="you@example.com" /></label>` +
    `<div class="field-message"></div>`;
  const email = extra.querySelector<HTMLInputElement>("#arbesk-email")!;
  const status = extra.querySelector<HTMLElement>(".field-message")!;
  email.value = reconnect?.email ?? "";
  const dialog = openDialog({
    heading: reconnect ? "Sign In to Arbesk" : "Connect to Arbesk",
    body: "Arbesk emails you a code. Enter it in your browser to sign in.",
    extra,
    className: "connect-dialog",
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "connect", label: "Open Browser", style: "suggested" },
    ],
    defaultResponse: "connect",
    closeResponse: "cancel",
    // Cancel stays live while Arbesk waits: it cancels the sign-in.
    busy: { liveWhileBusy: ["cancel"], status },
    onResponse: async (id) => {
      if (id === "cancel") {
        resolve(false);
        if (flowId) await arbeskConnectCancel(flowId).catch(() => {});
        return;
      }
      try {
        const start = await arbeskConnectStart(email.value);
        flowId = start.flow_id;
        status.textContent = "Waiting for Arbesk…";
        await openExternal(start.sign_in_url);
        const account = await arbeskConnectFinish(start.flow_id);
        accounts = withAccount(accounts, fromArbesk(account));
        signInNeeded.delete(accountRef(fromArbesk(account)));
        dialog.close();
        showToast(`Connected to ${account.email}`);
        resolve(true);
        if (!stay) await navigate(account.root_path);
      } catch (err) {
        const message = cleanError(err);
        status.textContent = /cancelled/i.test(message) ? "" : message;
        return false;
      }
    },
  });
  const sync = () => dialog.setResponseEnabled("connect", email.value.includes("@"));
  email.addEventListener("input", sync);
  sync();
  email.focus();
}

function confirmDisconnectArbesk(account: Account): void {
  openDialog({
    heading: "Disconnect Arbesk?",
    body: `“${account.email}” is removed from File Browser. Your collections on Arbesk are not changed.`,
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "disconnect", label: "Disconnect", style: "destructive" },
    ],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "disconnect") return;
      arbeskDisconnect(account.id)
        .then(async () => {
          forgetAccount(account);
          showToast("Disconnected from Arbesk");
          if (accountOf(location, [account])) await navigate(NETWORK);
          else if (location === NETWORK) await reload([]);
        })
        .catch((err) => showToast(`Could not disconnect: ${cleanError(err)}`));
    },
  });
}

function confirmDisconnectDrive(account: Account): void {
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
          forgetAccount(account);
          showToast("Disconnected from Google Drive");
          // Network, where the account was listed, shows it gone.
          if (accountOf(location, [account])) await navigate(NETWORK);
          else if (location === NETWORK) await reload([]);
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
  previewActionsEl = $("#preview-actions");
  previewVersionsEl = $("#preview-versions");
  previewFilesEl = $("#preview-files");
  // Escape on a preview button, or Escape or Space on a 3D model being
  // turned: the view's own keys don't see them.
  previewLayer.addEventListener("keydown", (event) => {
    const onModel = (event.target as Element).classList.contains("preview-model");
    if (event.key !== "Escape" && !(event.key === " " && onModel)) return;
    event.preventDefault();
    event.stopPropagation();
    leavePreview();
  });
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
  $("#btn-search").addEventListener("click", () => (search.isOpen ? closeSearch() : openSearch()));
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
    selection.noteIntent();
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
      moveCursor(selection.cursor < 0 ? 0 : selection.cursor, false);
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
    if (hasOpenOverlay() || search.isOpen) return;
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

  // What each kind of place allows, from the backend's places; nothing
  // waits for it: until it lands the built-in table says the same.
  void loadCapabilities(capabilities);
  // Tauri keeps $HOME as written: HOME=/Users/me/ would hold none of its
  // folders (isWithin), so trim the slash once, here (#170).
  home = homeFolder(await homeDir().catch(() => ""));
  // Only the backend knows the platform's Trash folder (~/.Trash on a Mac,
  // ~/.local/share/Trash/files on Linux). Without its answer there is no
  // Trash place, rather than a guess.
  trashPath = await trashDir().catch(() => "");
  await buildSidebar();
  const requested = new URLSearchParams(window.location.search).get("path");
  const initial = requested && isKnownPath(requested) ? requested : home || "/";
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
