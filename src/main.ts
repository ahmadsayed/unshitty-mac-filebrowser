import { getCurrentWebview } from "@tauri-apps/api/webview";
import { homeDir, documentDir, downloadDir, audioDir, pictureDir, videoDir, desktopDir, publicDir } from "@tauri-apps/api/path";
import {
  listDir, makeDir, renameEntry, itemCounts, copyEntry, moveEntry, trashEntry, pathExists,
  openFile, openFullDiskAccessSettings, loadPins, savePins, getFlag, setFlag, hasFullDiskAccess, pathInfo,
  recentFiles, addRecent, forgetRecent, loadStarred, saveStarred, starredFiles,
  trashInfo, restoreEntry, deletePermanently, emptyTrash, listVolumes, trashDir, type Entry,
} from "./api";
import { entryIcon, fileKind, fileTypeLabel, installIconSprite, isAppBundle, sym, type FileKind, type FolderKind } from "./icons";
import { cachedThumb, keepOnly, requestThumb, thumbKey } from "./thumbnails";
import { formatDate, formatDateDetailed, formatItemCount, formatSize } from "./format";
import { closePopover, hasOpenOverlay, openDialog, openMenu, openPopover, showToast, type Anchor, type MenuItem } from "./ui";

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
  try {
    Object.assign(prefs, JSON.parse(localStorage.getItem("prefs") ?? "{}"));
  } catch {
    /* defaults */
  }
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

/** Sidebar places that aren't folders. Recent and Starred list items from anywhere; Network is a placeholder. */
type Pseudo = "recent" | "starred" | "network";
const PSEUDO: Record<Pseudo, { label: string; icon: string; title: string; text: string }> = {
  recent: { label: "Recent", icon: "document-open-recent", title: "No Recent Files", text: "" },
  starred: { label: "Starred", icon: "starred", title: "Starred files will appear here", text: "" },
  network: { label: "Network", icon: "network-workgroup", title: "No Network Locations", text: "Network locations aren't supported yet." },
};
const PSEUDO_PREFIX = "pseudo:";
const RECENT = `${PSEUDO_PREFIX}recent`;
const STARRED = `${PSEUDO_PREFIX}starred`;

/** Current location: an absolute path, or "pseudo:<name>". */
let location = "";
let entries: Entry[] = [];
/** Entries in display order (sorted + search-filtered). */
let shown: Entry[] = [];
let listingError: string | null = null;

const selected = new Set<string>();
let anchorIndex = -1;
let cursorIndex = -1;
let searchQuery: string | null = null;

const counts = new Map<string, number | null>();
let backStack: string[] = [];
let forwardStack: string[] = [];

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
/** Mounted volumes, as the backend reported them; used for breadcrumbs and
 * for guessing whether a drag crosses filesystems. */
let mountedVolumes: { name: string; path: string }[] = [];
const folderKinds = new Map<string, FolderKind>();

const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);
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

// SECURITY: escape HTML entities in user-derived strings before innerHTML insertion
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function basename(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || "/";
}

function parentOf(path: string): string {
  const p = path.replace(/\/+$/, "").replace(/\/[^/]*$/, "");
  return p || "/";
}

function joinPath(dir: string, name: string): string {
  return dir.replace(/\/+$/, "") + "/" + name;
}

function isPseudo(loc: string): loc is `pseudo:${Pseudo}` {
  return loc.startsWith(PSEUDO_PREFIX);
}

/** Pseudo locations that list entries (Recent, Starred). */
function hasEntries(loc: string): boolean {
  return !isPseudo(loc) || loc === RECENT || loc === STARRED;
}

/** Views of items from many folders, which get a Location column. */
function showsWhere(): boolean {
  return location === RECENT || location === STARRED || location === trashPath;
}

/** The entries shown at `loc`. */
async function listLocation(loc: string): Promise<Entry[]> {
  if (loc === RECENT) return recentFiles(prefs.showHidden);
  if (loc === STARRED) return starredFiles(prefs.showHidden);
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
  return s.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLowerCase();
}

function computeShown(): void {
  const q = searchQuery ? fold(searchQuery.trim()) : "";
  const list = q ? entries.filter((e) => fold(e.name).includes(q)) : entries;
  shown = [...list].sort(compareEntries);
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

  let next: Entry[] = [];
  let error: string | null = null;
  if (hasEntries(loc)) {
    try {
      next = await listLocation(loc);
    } catch (err) {
      if (location) {
        showAccessError(basename(loc), cleanError(err));
        return false;
      }
      error = cleanError(err);
    }
  }

  if (opts.history !== "none" && location && loc !== location) {
    backStack.push(location);
    forwardStack = [];
  }

  location = loc;
  entries = next;
  listingError = error;
  counts.clear();
  countsInFlight.clear();
  selected.clear();
  anchorIndex = cursorIndex = -1;
  if (opts.select) selected.add(opts.select);

  document.title = locationName(loc);
  updateNavButtons();
  updateSidebarSelection();
  buildPathbar();
  renderView();
  scrollEl.scrollTop = 0;
  if (opts.select) revealPath(opts.select);
  viewEl.focus({ preventScroll: true });
  return true;
}

/** Re-list the current folder, keeping the selection where possible. */
async function reload(select?: string[]): Promise<void> {
  if (!hasEntries(location)) return;
  try {
    entries = await listLocation(location);
    listingError = null;
  } catch (err) {
    entries = [];
    listingError = cleanError(err);
  }
  const keep = select ?? [...selected];
  selected.clear();
  const paths = new Set(entries.map((e) => e.path));
  for (const p of keep) if (paths.has(p)) selected.add(p);
  const top = scrollEl.scrollTop;
  renderView();
  scrollEl.scrollTop = top;
  if (select?.[0]) revealPath(select[0]);
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
      const title = c.label || "Computer";
      const sep = i > 0 ? `<span class="crumb-sep">/</span>` : "";
      return `${sep}<button class="${cls}" data-path="${escapeHtml(c.path)}" title="${escapeHtml(title)}">${icon}${label}</button>`;
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
  let typed = pathEntry.value.trim();
  if (!typed) return;
  if (typed === "~" || typed.startsWith("~/")) typed = home + typed.slice(1);
  else if (!typed.startsWith("/") && !isPseudo(location)) typed = joinPath(location, typed);
  typed = typed.length > 1 ? typed.replace(/\/+$/, "") : typed;
  try {
    await listDir(typed, prefs.showHidden);
  } catch (err) {
    const message = cleanError(err);
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
  searchEntry.value = "";
  searchbarEl.hidden = true;
  pathbarEl.hidden = false;
  $("#btn-search").classList.remove("open");
  if (rerender) {
    renderView();
    viewEl.focus({ preventScroll: true });
  }
}

function onSearchInput(): void {
  searchQuery = searchEntry.value;
  anchorIndex = cursorIndex = -1;
  renderView();
  // Like GNOME Files, the first match is selected so Enter opens it.
  if (searchQuery && shown.length) {
    selected.clear();
    selected.add(shown[0].path);
    cursorIndex = anchorIndex = 0;
    updateSelection();
  }
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

  if (pseudo && (!hasEntries(location) || (shown.length === 0 && !searchQuery))) {
    viewEl.innerHTML = statusPage(pseudo.icon, pseudo.title, pseudo.text);
  } else if (listingError !== null) {
    viewEl.innerHTML = statusPage("dialog-error", "Unable to Access Location", listingError);
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
  relayout();
  updateViewButtons();
  updateFloatingBar();
}

function statusPage(icon: string, title: string, text: string): string {
  return `<div class="status-page">${sym(icon)}<h1>${escapeHtml(title)}</h1>${text ? `<p>${escapeHtml(text)}</p>` : ""}</div>`;
}

function itemClasses(e: Entry, i: number): string {
  const cut = clipboard?.mode === "cut" && clipboard.paths.includes(e.path);
  return (e.is_hidden ? " hidden-entry" : "") + (cut ? " cut" : "") +
    (selected.has(e.path) ? " selected" : "") + (i === cursorIndex ? " cursor" : "");
}

function listHeaderHtml(): string {
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
    return `<div class="grid-item item${cls}" data-index="${i}" role="option" aria-selected="${sel}" title="${escapeHtml(e.name)}" ${pos}>` +
      `<div class="icon">${iconHtml(e)}</div>` +
      `<div class="name">${escapeHtml(e.name)}</div></div>`;
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
  return entryIcon(e, folderKind(e));
}

/** Request thumbnails for the rendered items; queued ones scrolled away are dropped. */
function loadThumbs(start: number, end: number): void {
  if (!prefs.showThumbnails) return;
  const size = thumbSize();
  const keys = new Set<string>();
  for (let i = start; i <= end; i++) {
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

const GRID_PAD = 12;
const GRID_GAP = 6;
/** Height of the sticky list header plus its bottom margin. */
const LIST_HEADER_H = 35;

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
  const width = scrollEl.clientWidth;
  const lineH = parseFloat(getComputedStyle(document.body).fontSize) * 1.3;
  if (prefs.view === "grid") {
    const icon = GRID_SIZES[prefs.gridZoom];
    const inner = width - GRID_PAD * 2;
    const cols = Math.max(1, Math.floor((inner + GRID_GAP) / (icon + 36 + GRID_GAP)));
    const itemW = Math.floor((inner - GRID_GAP * (cols - 1)) / cols);
    // icon + gap + two lines of label + padding; a selected item grows to show its full name.
    const itemH = Math.ceil(6 + icon + 6 + 2 * lineH + 9);
    layout = { cols, itemW, itemH, gap: GRID_GAP, padX: GRID_PAD, padY: GRID_PAD, bodyTop: 0 };
  } else {
    const itemH = Math.max(LIST_SIZES[prefs.listZoom] + 12, 34);
    layout = { cols: 1, itemW: width - 12, itemH, gap: 0, padX: 6, padY: 0, bodyTop: LIST_HEADER_H };
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
  const firstRow = Math.max(0, Math.floor(top / pitch) - overscan);
  const lastRow = Math.floor((top + scrollEl.clientHeight) / pitch) + overscan;
  const start = firstRow * layout.cols;
  const end = Math.min(shown.length - 1, (lastRow + 1) * layout.cols - 1);
  if (!force && start === windowRange.start && end === windowRange.end) return;
  windowRange = { start, end };
  const html: string[] = [];
  for (let i = start; i <= end; i++) html.push(itemHtml(i));
  itemsLayer.innerHTML = html.join("");
  loadThumbs(start, end);
  if (prefs.view === "list") {
    const dirs: string[] = [];
    for (let i = start; i <= end; i++) if (shown[i].is_dir) dirs.push(shown[i].path);
    requestCounts(dirs, false);
  }
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
  prefs.gridZoom = 2;
  prefs.listZoom = 1;
  savePrefs();
  applyZoom();
  relayout();
  scrollToIndex(cursorIndex);
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
  const pending = paths.filter((p) => !counts.has(p) && !countsInFlight.has(p));
  if (pending.length === 0) return;
  pending.forEach((p) => countsInFlight.add(p));
  for (let i = 0; i < pending.length; i += 400) {
    const batch = pending.slice(i, i + 400);
    let result: (number | null)[] = [];
    try {
      result = await itemCounts(batch, prefs.showHidden);
    } catch {
      /* leave those cells blank */
    }
    batch.forEach((p) => countsInFlight.delete(p));
    if (loc !== location) return;
    batch.forEach((p, j) => counts.set(p, result[j] ?? null));
    patchCounts();
  }
  if (resortWhenDone && prefs.sortKey === "size") renderViewKeepingScroll();
}

function patchCounts(): void {
  viewEl.querySelectorAll<HTMLElement>(".list-row").forEach((row) => {
    const e = shown[Number(row.dataset.index)];
    if (e?.is_dir) row.querySelector(".size")!.textContent = sizeText(e);
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
  selected.clear();
  if (shown[index]) selected.add(shown[index].path);
  anchorIndex = cursorIndex = index;
  updateSelection();
}

function selectRange(from: number, to: number, additive: boolean): void {
  if (!additive) selected.clear();
  const [a, b] = from < to ? [from, to] : [to, from];
  for (let i = a; i <= b; i++) if (shown[i]) selected.add(shown[i].path);
  cursorIndex = to;
  updateSelection();
}

function selectAll(): void {
  for (const e of shown) selected.add(e.path);
  updateSelection();
}

function clearSelection(): void {
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
  itemCounts([path], prefs.showHidden).then(([n]) => {
    counts.set(path, n);
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
  try {
    await openFile(entry.path);
  } catch (err) {
    showToast(`Could not open “${entry.name}”: ${cleanError(err)}`);
    return;
  }
  if (!isAppBundle(entry)) addRecent(entry.path).catch(() => {});
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

function onViewMouseDown(e: MouseEvent): void {
  viewEl.classList.remove("keyboard-nav");
  const sortBtn = (e.target as HTMLElement).closest<HTMLElement>("[data-sort]");
  if (sortBtn) return;

  const index = indexFromEvent(e);
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

  if (e.button === 2) {
    if (!selected.has(shown[index].path)) selectOnly(index);
    return;
  }
  if (e.button !== 0) return;

  if (e.shiftKey && anchorIndex >= 0) {
    selectRange(anchorIndex, index, toggleMod);
  } else if (toggleMod) {
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
  if (!hasEntries(location) || listingError !== null) return;
  const at = { x: e.clientX, y: e.clientY };
  if (indexFromEvent(e) >= 0) openItemMenu(at);
  else openBackgroundMenu(at);
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

function openItemMenu(at: Anchor): void {
  const sel = selectedEntries();
  if (sel.length === 0) return;
  const single = sel.length === 1 ? sel[0] : null;
  const intoFolder = single?.is_dir && !isAppBundle(single) ? single : null;
  const app = single && isAppBundle(single) ? single : null;
  if (location === RECENT) {
    openMenu(at, [
      { label: "Open", action: activateSelection },
      { label: "Open Item Location", disabled: !single, action: () => single && openItemLocation(single) },
      { separator: true },
      { label: "Cut", action: () => setClipboard("cut") },
      { label: "Copy", action: () => setClipboard("copy") },
      { separator: true },
      { separator: true },
      starMenuItem(sel),
      { separator: true },
      { label: "Move to Trash", action: trashSelection },
      { label: "Remove from Recent", action: removeFromRecent },
    ]);
    return;
  }
  if (inTrash()) {
    // Only items at the top of the Trash know where they came from.
    const restorable = location === trashPath && sel.some((e) => e.origin);
    openMenu(at, [
      { label: "Open", action: activateSelection },
      ...(location === trashPath ? [{ label: "Restore From Trash", disabled: !restorable, action: restoreSelection }] : []),
      { separator: true },
      { label: "Cut", action: () => setClipboard("cut") },
      { label: "Copy", action: () => setClipboard("copy") },
      { separator: true },
      { label: "Delete Permanently…", action: () => confirmDeletePermanently(sel) },
    ]);
    return;
  }
  if (location === STARRED) {
    openMenu(at, [
      { label: "Open", action: activateSelection },
      { label: "Open Item Location", disabled: !single, action: () => single && openItemLocation(single) },
      { separator: true },
      { label: "Cut", action: () => setClipboard("cut") },
      { label: "Copy", action: () => setClipboard("copy") },
      { separator: true },
      starMenuItem(sel),
      { separator: true },
      { label: "Move to Trash", action: trashSelection },
    ]);
    return;
  }
  openMenu(at, [
    { label: "Open", action: activateSelection },
    ...(app ? [{ label: "Show Package Contents", action: () => navigate(app.path) }] : []),
    ...(intoFolder ? [bookmarkMenuItem(intoFolder.path)] : []),
    { separator: true },
    { label: "Cut", action: () => setClipboard("cut") },
    { label: "Copy", action: () => setClipboard("copy") },
    ...(intoFolder ? [{ label: "Paste Into Folder", disabled: !clipboard, action: () => paste(intoFolder.path) }] : []),
    { separator: true },
    { label: "Rename…", disabled: !single, action: () => single && startRename(single) },
    starMenuItem(sel),
    { separator: true },
    { label: "Move to Trash", action: trashSelection },
  ]);
}

function openBackgroundMenu(at: Anchor): void {
  if (isPseudo(location)) {
    openMenu(at, [{ label: "Select All", action: selectAll }]);
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
  openMenu(at, [
    { label: "New Folder…", action: promptNewFolder },
    { separator: true },
    { label: "Paste", disabled: !clipboard, action: () => paste(location) },
    { label: "Select All", action: selectAll },
  ]);
}

function openLocationMenu(anchor: HTMLElement): void {
  const real = !isPseudo(location);
  if (location === trashPath) {
    openMenu(anchor, [
      { label: "Empty Trash…", disabled: entries.length === 0, action: confirmEmptyTrash },
      { separator: true },
      { label: "Select All", action: selectAll },
    ]);
    return;
  }
  openMenu(anchor, [
    { label: "New Folder…", disabled: !real, action: promptNewFolder },
    ...(real ? [bookmarkMenuItem(location)] : []),
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
    { label: "Show Hidden Files", check: prefs.showHidden, action: toggleHidden },
    { label: "Show Thumbnails", check: prefs.showThumbnails, action: toggleThumbnails },
    { separator: true },
    { label: "Keyboard Shortcuts", action: showShortcuts },
    { label: "About Files", action: showAbout },
  ]);
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

let transferring = false;

/**
 * Copy or move `paths` into `destDir`, asking about name clashes. Shared by
 * paste and drag and drop; `move(path)` says whether that item is moved.
 * Resolves true when every item was handled (none failed, nothing cancelled).
 */
async function runTransfer(paths: string[], dirs: Set<string>, move: (path: string) => boolean, destDir: string): Promise<boolean> {
  if (transferring || isPseudo(destDir)) return false;
  transferring = true;
  let progressShown = false;

  const done: string[] = [];
  const errors: string[] = [];
  let anyMove = false;
  let cancelled = false;
  let applyToAll: ConflictChoice | null = null;

  for (let i = 0; i < paths.length; i++) {
    const from = paths[i];
    const name = basename(from);
    const isDir = dirs.has(from);
    const moving = move(from);
    anyMove ||= moving;
    let to = joinPath(destDir, name);
    let overwrite = false;

    if (from === to) {
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
        cancelled = true;
        break;
      }
      if (choice === "skip") continue;
      if (choice === "keepboth") to = joinPath(destDir, await freeCopyName(destDir, name, isDir));
      else overwrite = true;
    }

    // Only a slow transfer gets a progress toast, never one waiting on the dialog.
    const slow = setTimeout(() => {
      progressShown = true;
      showToast(`${moving ? "Moving" : "Copying"} “${name}”…`, { timeout: 600_000 });
    }, 600);
    try {
      if (moving) {
        await moveEntry(from, to, overwrite);
        followMove(from, to);
      } else {
        await copyEntry(from, to, overwrite);
      }
      done.push(to);
    } catch (err) {
      errors.push(`“${name}”: ${cleanError(err)}`);
    }
    clearTimeout(slow);
  }

  transferring = false;
  const verb = anyMove ? "move" : "copy";
  if (errors.length) {
    showToast(errors.length === 1 ? `Could not ${verb} ${errors[0]}` : `Could not ${verb} ${errors.length} items. ${errors[0]}`);
  } else if (progressShown) {
    document.querySelectorAll(".toast").forEach((t) => t.remove());
  }
  if (destDir === location) await reload(done.length ? done : undefined);
  else await reload();
  return !cancelled && errors.length === 0;
}

async function paste(destDir: string): Promise<void> {
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
  if (inTrash()) return;
  await trashPaths(selectedEntries().map((e) => e.path));
}

/** Delete / ⌘⌫: to the Trash, or out of it for good when already there. */
function trashOrDelete(): void {
  if (inTrash()) confirmDeletePermanently(selectedEntries());
  else trashSelection();
}

/** The last Move to Trash, for Undo: where each item went and came from. */
let lastTrashed: { trashed: string; origin: string }[] = [];

async function trashPaths(paths: string[]): Promise<void> {
  if (paths.length === 0) return;
  const errors: string[] = [];
  const undoable: { trashed: string; origin: string }[] = [];
  let done = 0;
  for (const p of paths) {
    try {
      const trashed = await trashEntry(p);
      if (trashed) undoable.push({ trashed, origin: p });
      done++;
    } catch (err) {
      errors.push(`“${basename(p)}”: ${cleanError(err)}`);
    }
  }
  if (clipboard) {
    clipboard.paths = clipboard.paths.filter((p) => !paths.includes(p));
    if (clipboard.paths.length === 0) clipboard = null;
  }
  lastTrashed = undoable;
  if (errors.length) {
    showToast(`Could not move ${errors[0]} to the Trash`);
  } else {
    const message = done === 1 ? `“${basename(paths[0])}” moved to trash` : `${done} items moved to trash`;
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
  for (const e of sel) {
    try {
      await deletePermanently(e.path);
    } catch (err) {
      errors.push(`“${e.name}”: ${cleanError(err)}`);
    }
  }
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
  try {
    await emptyTrash();
  } catch (err) {
    showToast(`Could not empty the Trash: ${cleanError(err)}`);
  }
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
  if (row) return isPseudo(row.dataset.location!) ? null : { dir: row.dataset.location!, el: row };
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
  // Into itself or its own subfolder: refused.
  if (sources.some((src) => dir === src || dir.startsWith(src + "/"))) return null;
  if (dir === trashPath) return inTrash() ? null : "trash";
  const forceCopy = isMac ? mods.altKey : mods.ctrlKey;
  const forceMove = isMac ? mods.metaKey : mods.shiftKey;
  // Moving items to the folder they are already in does nothing.
  const allHere = sources.every((src) => parentOf(src) === dir);
  if (forceCopy) return "copy";
  if (allHere) return null;
  if (forceMove) return "move";
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
}

let drag: DragState | null = null;
/** Set while a press on a selected item may still narrow the selection on release. */
let pendingNarrow: (() => void) | null = null;

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
  if (pendingNarrow) {
    viewEl.removeEventListener("mouseup", pendingNarrow);
    pendingNarrow = null;
  }
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
  d.ghost.remove();
  document.documentElement.classList.remove("dragging");
  document.documentElement.style.cursor = "";
  setDropHighlight(null);
  if (drop && d.target && d.op) performDrop(d.paths, d.dirs, d.target, d.op);
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

function startRename(entry: Entry): void {
  // Recent and Starred mix folders, so the name-clash check wouldn't work there.
  if (isPseudo(location)) return;
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
    try {
      await renameEntry(entry.path, target);
    } catch (err) {
      showValidation({ valid: false, message: cleanError(err), error: true }, input, message);
      return;
    }
    followMove(entry.path, target);
    handle.close();
    await reload([target]);
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
  if (isPseudo(location) || listingError !== null) return;
  const extra = document.createElement("div");
  extra.innerHTML = `
    <div class="entry-row"><label for="new-folder-name">Folder Name</label><input id="new-folder-name" type="text" spellcheck="false" /></div>
    <div class="field-message" style="margin-top:8px"></div>`;
  const row = extra.querySelector<HTMLElement>(".entry-row")!;
  const input = extra.querySelector("input")!;
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
      try {
        await makeDir(target);
      } catch (err) {
        showValidation({ valid: false, message: cleanError(err), error: true }, row, message);
        return false;
      }
      if (location === dir) await reload([target]);
    },
  });
  input.addEventListener("input", validate);
  validate();
}

// ---------------------------------------------------------------------------
// Keyboard shortcuts + about
// ---------------------------------------------------------------------------

const SHORTCUTS: [string, [string, string][]][] = [
  ["Navigation", [
    ["Back", "Alt+←"], ["Forward", "Alt+→"], ["Parent folder", "Alt+↑"], ["Home folder", "Alt+Home"],
    ["Enter location", `${MOD}L`], ["Search", `${MOD}F`], ["Reload", `${MOD}R`],
  ]],
  ["View", [
    ["List view", `${MOD}1`], ["Grid view", `${MOD}2`], ["Zoom in / out", `${MOD}+ / ${MOD}−`],
    ["Reset zoom", `${MOD}0`], ["Show hidden files", isMac ? "⇧⌘." : "Ctrl+H"],
  ]],
  ["Files", [
    ["Open", "Enter"], ["Rename", "F2"], ["New folder", isMac ? "⇧⌘N" : "Shift+Ctrl+N"], ["Select all", `${MOD}A`],
    ["Copy / Cut / Paste", `${MOD}C / ${MOD}X / ${MOD}V`], ["Move to Trash", isMac ? "⌘⌫ or Delete" : "Delete"], ["Undo Move to Trash", `${MOD}Z`],
    ["Cancel cut", "Esc"], ["Bookmark this folder", `${MOD}D`],
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
    <p>A GNOME Files look-alike for macOS.<br/>Version ${escapeHtml(__APP_VERSION__)}</p>
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

function onViewKeyDown(e: KeyboardEvent): void {
  const mod = isMac ? e.metaKey : e.ctrlKey;
  const cols = layout.cols;
  const cur = cursorIndex;
  const page = Math.max(Math.floor(scrollEl.clientHeight / (layout.itemH + layout.gap)), 1) * cols;

  switch (e.key) {
    case "ArrowRight":
      if (e.altKey || mod) return;
      if (prefs.view === "grid") moveCursor(cur < 0 ? 0 : cur + 1, e.shiftKey);
      break;
    case "ArrowLeft":
      if (e.altKey || mod) return;
      if (prefs.view === "grid") moveCursor(cur < 0 ? 0 : cur - 1, e.shiftKey);
      break;
    case "ArrowDown":
      if (e.altKey) return;
      if (mod) activateSelection();
      else moveCursor(cur < 0 ? 0 : cur + cols, e.shiftKey);
      break;
    case "ArrowUp":
      if (e.altKey || mod) return;
      moveCursor(cur < 0 ? 0 : cur - cols, e.shiftKey);
      break;
    case "Home":
      if (e.altKey) return;
      moveCursor(0, e.shiftKey);
      break;
    case "End":
      moveCursor(shown.length - 1, e.shiftKey);
      break;
    case "PageDown":
      moveCursor(cur + page, e.shiftKey);
      break;
    case "PageUp":
      moveCursor(cur - page, e.shiftKey);
      break;
    case "Enter":
      activateSelection();
      break;
    case "F2": {
      const sel = selectedEntries();
      if (sel.length === 1) startRename(sel[0]);
      break;
    }
    case "Escape":
      if (searchQuery !== null) closeSearch();
      else if (!cancelCut()) clearSelection();
      break;
    case "Delete":
      trashOrDelete();
      break;
    case "Backspace":
      if (mod) trashOrDelete();
      else goBack();
      break;
    case " ":
      if (cur >= 0 && mod) {
        const p = shown[cur].path;
        if (selected.has(p)) selected.delete(p);
        else selected.add(p);
        updateSelection();
      }
      break;
    default:
      // Type-ahead: printable keys start a search, like GNOME Files.
      if (e.key.length === 1 && !mod && !e.altKey && !e.ctrlKey && !e.metaKey && e.key !== " ") {
        e.preventDefault();
        openSearch(e.key);
      }
      return;
  }
  e.preventDefault();
}

function onGlobalKeyDown(e: KeyboardEvent): void {
  if (hasOpenOverlay()) return;
  const mod = isMac ? e.metaKey : e.ctrlKey;
  const key = e.key.toLowerCase();
  const inInput = (e.target as HTMLElement).tagName === "INPUT";

  const run = (fn: () => void) => {
    e.preventDefault();
    fn();
  };

  if (mod && !e.shiftKey && key === "l") return run(() => openPathEntry());
  if (mod && !e.shiftKey && key === "f") return run(() => openSearch());
  if (mod && e.shiftKey && key === "n") return run(promptNewFolder);
  if (mod && !e.shiftKey && key === "d" && !isPseudo(location)) return run(() => setBookmarked(location, !pins.includes(location)));
  if ((mod && e.shiftKey && (e.key === "." || e.key === ">")) || (e.ctrlKey && !e.shiftKey && key === "h")) return run(toggleHidden);
  if ((mod && !e.shiftKey && key === "r") || e.key === "F5") return run(() => reload());
  if (mod && (e.key === "?" || (e.shiftKey && e.key === "/"))) return run(showShortcuts);
  if (mod && key === "1") return run(() => setView("list"));
  if (mod && key === "2") return run(() => setView("grid"));
  if (mod && (e.key === "=" || e.key === "+")) return run(() => zoom(1));
  if (mod && e.key === "-") return run(() => zoom(-1));
  if (mod && key === "0") return run(resetZoom);
  if (e.altKey && e.key === "ArrowLeft") return run(goBack);
  if (e.altKey && e.key === "ArrowRight") return run(goForward);
  if (e.altKey && e.key === "ArrowUp") return run(goUp);
  if (e.altKey && e.key === "Home") return run(() => navigate(home));
  if (isMac && e.metaKey && e.key === "[") return run(goBack);
  if (isMac && e.metaKey && e.key === "]") return run(goForward);
  if (isMac && e.metaKey && e.key === "ArrowUp") return run(goUp);
  if (inInput) return;
  if (mod && !e.shiftKey && key === "z" && lastTrashed.length) return run(undoTrash);
  if (mod && key === "a") return run(selectAll);
  if (mod && !e.shiftKey && key === "c" && selected.size) return run(() => setClipboard("copy"));
  if (mod && !e.shiftKey && key === "x" && selected.size) return run(() => setClipboard("cut"));
  if (mod && !e.shiftKey && key === "v" && clipboard && !isPseudo(location)) return run(() => paste(location));
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

/** Add a sidebar row; `fixed` rows are built-in and can't be bookmarked again. */
function sidebarRow(section: HTMLElement, label: string, icon: string, loc: string, fixed = true): HTMLElement {
  const row = document.createElement("div");
  row.className = "sidebar-row";
  row.dataset.location = loc;
  row.setAttribute("role", "link");
  row.innerHTML = `${sym(icon)}<span class="sidebar-row-label"></span>`;
  row.querySelector(".sidebar-row-label")!.textContent = label;
  row.title = isPseudo(loc) ? label : loc;
  row.addEventListener("click", () => navigate(loc));
  section.appendChild(row);
  if (fixed && !isPseudo(loc)) fixedSidebarPaths.add(loc);
  return row;
}

function updateSidebarSelection(): void {
  document.querySelectorAll<HTMLElement>(".sidebar-row").forEach((row) => {
    row.classList.toggle("selected", row.dataset.location === location);
  });
}

async function buildSidebar(): Promise<void> {
  const places = $("#sidebar-places");
  const bookmarks = $("#sidebar-bookmarks");
  const mounts = $("#sidebar-mounts");

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
  sidebarRow(places, "Network", "network-workgroup", `${PSEUDO_PREFIX}network`);
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
// Initialise
// ---------------------------------------------------------------------------

async function init(): Promise<void> {
  loadPrefs();
  installIconSprite();
  if (isMac) document.documentElement.classList.add("platform-mac");

  viewEl = $("#view");
  scrollEl = $("#view-scroll");
  crumbsEl = $("#crumbs");
  pathbarEl = $("#pathbar");
  pathEntry = $<HTMLInputElement>("#path-entry");
  searchbarEl = $("#searchbar");
  searchEntry = $<HTMLInputElement>("#search-entry");
  floatingBar = $("#floating-bar");

  // Static icons
  $("#btn-search").innerHTML = sym("system-search");
  $("#btn-main-menu").innerHTML = sym("open-menu");
  $("#btn-back").innerHTML = sym("go-previous");
  $("#btn-forward").innerHTML = sym("go-next");
  $("#btn-location-menu").innerHTML = sym("view-more");
  $("#btn-new-folder").innerHTML = sym("folder-new");
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
  searchEntry.addEventListener("input", onSearchInput);
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
  viewEl.addEventListener("dblclick", (e) => {
    const i = indexFromEvent(e);
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
    cancelAnimationFrame(scrollFrame);
    scrollFrame = requestAnimationFrame(() => renderWindow());
  });
  new ResizeObserver(() => relayout()).observe(scrollEl);
  scrollEl.addEventListener("contextmenu", (e) => {
    if (e.target === scrollEl) onViewContextMenu(e);
  });
  document.addEventListener("keydown", onGlobalKeyDown);
  // Files opened in other apps meanwhile show up when the window comes back.
  window.addEventListener("focus", () => {
    if (location === RECENT && !hasOpenOverlay()) reload();
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
  await navigate(home || "/", { history: "none" });
  initExternalDrops();
  await initAccessBanner();
}

window.addEventListener("DOMContentLoaded", () => {
  init().catch(console.error);
});
