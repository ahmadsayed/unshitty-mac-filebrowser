// Which picture each item shows, and when it's asked for. In order: its
// thumbnail once loaded (src/thumbnails.ts; for an Arbesk asset, the one it
// was saved with), an SSH server's glyph in
// Network, the OS icon once loaded (src/native-icons.ts), otherwise the
// app's drawing; icons.ts puts the emblems on top. The OS icons of a
// folder's first screenful are waited for, briefly, before it's shown. On
// macOS a folder's icon shows whether it holds anything (Finder's full
// folder), so the folders' cached icons are asked for again once per listing,
// and when a folder's count is asked for again. No DOM for the view: `redraw`
// patches one item's icon, and only the rendered items are passed in.

import type { Entry } from "./api";
import { fileKind, itemIcon, type FileKind, type FolderKind, type IconBase } from "./icons";
import { providesThumbnails, PSEUDO_PREFIX } from "./locations";
import { loadNativeIcons, nativeIcon, nativeIconSize, nativeIconUrl, preloadNativeIcons, recheckNativeIcons } from "./native-icons";
import { isRemotePath, isSftpPath } from "./paths";
import { PROVIDERS, providerOf } from "./providers";
import { cachedThumb, keepOnly, requestThumb, thumbKey } from "./thumbnails";

/** Icon sizes, in CSS pixels, per zoom level of each view. */
export const GRID_SIZES = [48, 64, 96, 128, 192, 256];
export const LIST_SIZES = [16, 32, 48];

/** Kinds of files that get a thumbnail (images, video, PDF and documents, via Quick Look on macOS). */
export const THUMB_KINDS = new Set<FileKind>(["image", "video", "pdf", "document", "spreadsheet", "presentation"]);

const NETWORK = `${PSEUDO_PREFIX}network`;

/** The OS icons usually arrive a few milliseconds after the listing. Wait
 *  for those of the first items, briefly, so a folder appears with them
 *  instead of with the app's drawings swapped for them a moment later. */
const ICON_WAIT_MS = 100;
/** About a screenful: the items sorted first are the ones shown. */
const ICON_WAIT_ITEMS = 200;
/** Past this, sorting a copy just to find the first screenful costs more than it saves. */
const ICON_WAIT_SORT_MAX = 5000;

/**
 * Whether the OS has an icon for `path`: never for remote places (Drive,
 * shares, SSH servers), nor for anything shown in Network. Without `loc`,
 * wherever it's shown (a dialog's icon).
 */
export function hasOsIcons(path: string, loc?: string): boolean {
  return loc !== NETWORK && !isRemotePath(path);
}

/** The view and zoom, and whether thumbnails are on (the app's prefs). */
export interface IconView {
  view: "grid" | "list";
  gridZoom: number;
  listZoom: number;
  showThumbnails: boolean;
}

/** What the app tells ItemIcons. */
export interface ItemIconsHost {
  /** macOS: a folder's OS icon shows whether it holds anything. */
  followsContents: boolean;
  /** The location on screen. */
  location(): string;
  view(): IconView;
  /** The app's drawing for a folder: plain, or one of the special folders. */
  folderKind(e: Entry): FolderKind;
  /** The view's sort order, to find the first screenful of a listing. */
  compare(a: Entry, b: Entry): number;
  /** Draw an item's icon again: its picture changed or arrived. */
  redraw(path: string): void;
}

export class ItemIcons {
  /** Rendered folders whose OS icon was asked for again since the listing loaded. */
  private readonly checked = new Set<string>();

  constructor(private readonly host: ItemIconsHost) {}

  /** The size items' icons are shown at, in CSS pixels, in this view and zoom. */
  shownPx(): number {
    const v = this.host.view();
    return v.view === "grid" ? GRID_SIZES[v.gridZoom] : LIST_SIZES[v.listZoom];
  }

  /** The thumbnail if it's loaded, an SSH server's or an online account's
   *  glyph in Network, the OS icon if it's loaded, otherwise the app's drawing. */
  base(e: Entry): IconBase {
    const thumb = this.wantsThumb(e) ? cachedThumb(this.thumbKeyFor(e)) : null;
    if (thumb) return { thumb };
    const loc = this.host.location();
    if (loc === NETWORK && isSftpPath(e.path)) return { glyph: "network-server" };
    const provider = loc === NETWORK ? providerOf(e.path) : null;
    if (provider) return { glyph: PROVIDERS[provider].icon };
    const native = hasOsIcons(e.path, loc) ? nativeIconUrl(e.path, this.nativePx()) : null;
    return native ? { native } : { drawn: this.host.folderKind(e) };
  }

  /**
   * A new listing of `loc` is about to be shown (navigating): wait, at most
   * ICON_WAIT_MS, for the OS icons of its first screenful. The folders' icons
   * cached on an earlier visit are asked for again in the same wait, so going
   * back shows them as they are now, not swapped later.
   */
  listingStarted(loc: string, next: Entry[]): Promise<void> {
    this.checked.clear();
    if (loc === NETWORK || next.length > ICON_WAIT_SORT_MAX) return Promise.resolve();
    const first = next.length <= ICON_WAIT_ITEMS ? next : [...next].sort((a, b) => this.host.compare(a, b)).slice(0, ICON_WAIT_ITEMS);
    const local = first.filter((e) => hasOsIcons(e.path, loc));
    if (!local.length) return Promise.resolve();
    const px = this.nativePx();
    const folders = this.host.followsContents ? local.filter((e) => e.is_dir).map((e) => e.path) : [];
    for (const path of folders) this.checked.add(path);
    const recheck = recheckNativeIcons(folders, px, this.changed);
    return Promise.all([
      preloadNativeIcons(local.map((e) => e.path), px, ICON_WAIT_MS),
      Promise.race([recheck, new Promise<void>((resolve) => setTimeout(resolve, ICON_WAIT_MS))]),
    ]).then(() => {});
  }

  /** The location was listed again (a reload): its folders are asked for
   *  again as they're rendered, without holding the view back. */
  listingReloaded(): void {
    this.checked.clear();
  }

  /**
   * The view rendered `items` (its rendered range, in order), of which
   * `visible` (indices into `items`) are on screen: ask for their OS icons and
   * thumbnails, the visible ones' thumbnails first.
   */
  rendered(items: readonly Entry[], visible: { start: number; end: number }): void {
    const loc = this.host.location();
    const paths = items.filter((e) => hasOsIcons(e.path, loc)).map((e) => e.path);
    if (paths.length) loadNativeIcons(paths, this.nativePx(), this.arrived);
    this.recheckFolders(items, loc);
    this.loadThumbs(items, visible);
  }

  /** Folders whose count was asked for again (the selected one): on macOS
   *  what they hold shows in their icon, so it's asked for again too. */
  recounted(paths: string[]): void {
    if (!this.host.followsContents) return;
    const folders = paths.filter((path) => hasOsIcons(path));
    if (folders.length) void recheckNativeIcons(folders, this.nativePx(), this.changed);
  }

  /** Draw `entry` in a dialog's icon, then swap in the OS icon for `key` (its
   *  path, or an `appIconKey`) once it loads; the app's drawing stays meanwhile,
   *  and wherever the OS has none. The link emblem shows on both. */
  dialogIcon(icon: HTMLElement, entry: Entry, shownAt: number, key = entry.path): void {
    const emblems = { link: entry.is_symlink, versions: false, shared: false, files: "none" as const };
    icon.innerHTML = itemIcon(entry, { drawn: this.host.folderKind(entry) }, emblems);
    if (!hasOsIcons(key)) return;
    void nativeIcon(key, nativeIconSize(shownAt)).then((url) => {
      if (url) icon.innerHTML = itemIcon(entry, { native: url }, emblems);
    });
  }

  private nativePx(): number {
    return nativeIconSize(this.shownPx());
  }

  /** OS icons that just loaded. */
  private readonly arrived = (paths: string[]): void => {
    for (const path of paths) this.host.redraw(path);
  };

  /** Redraw items whose OS icon changed, loading it at the current size too:
   *  a zoom may have come while it was asked for. */
  private readonly changed = (paths: string[]): void => {
    this.arrived(paths);
    loadNativeIcons(paths, this.nativePx(), this.arrived);
  };

  /** On macOS a folder's icon cached on an earlier visit is asked for again,
   *  once per listing, for the folders rendered. */
  private recheckFolders(items: readonly Entry[], loc: string): void {
    if (!this.host.followsContents) return;
    const folders: string[] = [];
    for (const e of items) {
      if (e.is_dir && hasOsIcons(e.path, loc) && !this.checked.has(e.path)) {
        this.checked.add(e.path);
        folders.push(e.path);
      }
    }
    if (folders.length) void recheckNativeIcons(folders, this.nativePx(), this.changed);
  }

  /** Pictures and documents, and any file of a place that hands over its
   *  own thumbnails (Arbesk); the backend answers empty when there is none. */
  private wantsThumb(e: Entry): boolean {
    if (!this.host.view().showThumbnails || e.is_dir) return false;
    return THUMB_KINDS.has(fileKind(e.name)) || providesThumbnails(e.path);
  }

  /** Pixel size to request: the displayed icon size at screen resolution, in a few steps. */
  private thumbSize(): number {
    const px = this.shownPx() * devicePixelRatio;
    return px <= 128 ? 128 : px <= 256 ? 256 : 512;
  }

  private thumbKeyFor(e: Entry, size = this.thumbSize()): string {
    return thumbKey(e.path, e.modified, e.size, size);
  }

  /** Request thumbnails for the rendered items; queued ones scrolled away are dropped. */
  private loadThumbs(items: readonly Entry[], visible: { start: number; end: number }): void {
    if (!this.host.view().showThumbnails) return;
    const size = this.thumbSize();
    const keys = new Set<string>();
    // Visible rows first, then the overscan around them: the order is the
    // queue's priority (see keepOnly).
    const order: number[] = [];
    for (let i = visible.start; i <= visible.end; i++) order.push(i);
    for (let i = 0; i < items.length; i++) if (i < visible.start || i > visible.end) order.push(i);
    for (const i of order) {
      const e = items[i];
      if (!this.wantsThumb(e)) continue;
      const key = this.thumbKeyFor(e, size);
      keys.add(key);
      if (cachedThumb(key) === undefined) {
        requestThumb(key, e.path, size, (url) => url && this.host.redraw(e.path));
      }
    }
    keepOnly(keys);
  }
}
