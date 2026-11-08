// What each kind of location allows. The app asks these questions instead of
// combining the path kinds itself, so the rules for Recent, Starred, Network,
// Google Drive, received shares, SSH servers and the Trash live in one place.
// Kept free of DOM and app state so every branch can be tested directly
// (tests/locations.test.ts).

import { isWithin, placeOf, PSEUDO_PREFIX, type Place, type Pseudo } from "./paths";

/** Every pseudo location starts with this, and Recent, Starred and Network
 *  (the places that aren't folders) are its names: spelled in paths.ts,
 *  which can't import this file. */
export { PSEUDO_PREFIX, type Pseudo };

/**
 * Which kind of place a path is, in one precedence: the Trash (the folder
 * itself, then anything inside it), then Recent, Starred and Network, then
 * the path's scheme (Google Drive, a received share, an SSH server, or a
 * folder on this computer or a mounted volume).
 */
export type LocationKind = "recent" | "starred" | "network" | "trash" | "in-trash" | "share" | "sftp" | "arbesk" | "drive" | "folder";

const PSEUDO_KINDS = new Map<string, LocationKind>([["recent", "recent"], ["starred", "starred"], ["network", "network"]]);
const PLACE_KINDS: Record<Place, LocationKind> = { drive: "drive", share: "share", sftp: "sftp", arbesk: "arbesk", local: "folder" };

/** The kind of place `path` is. Nothing is in the Trash until its path is
 *  known; an unknown `pseudo:` name (no window opens one: `isKnownPath`
 *  refuses it) is taken as Recent. */
export function locationKind(path: string, trashPath: string): LocationKind {
  if (trashPath && isWithin(path, trashPath)) return path === trashPath ? "trash" : "in-trash";
  if (isPseudo(path)) return PSEUDO_KINDS.get(path.slice(PSEUDO_PREFIX.length)) ?? "recent";
  return PLACE_KINDS[placeOf(path)];
}

/** Whether `path` is one of `kinds`. */
function kindIn(path: string, trashPath: string, kinds: readonly LocationKind[]): boolean {
  return kinds.includes(locationKind(path, trashPath));
}

export function isPseudo(path: string): path is `pseudo:${Pseudo}` {
  return path.startsWith(PSEUDO_PREFIX);
}

/** Recent, Starred and Network: the kinds that aren't folders. */
export function isPseudoKind(kind: LocationKind): boolean {
  return kind === "recent" || kind === "starred" || kind === "network";
}

/** The Trash or a folder inside it. */
export function isTrashKind(kind: LocationKind): boolean {
  return kind === "trash" || kind === "in-trash";
}

/** Shared folders, SSH servers and Arbesk are read only: no New Folder, Paste, Rename, Trash or drops. */
export function isReadOnlyKind(kind: LocationKind): boolean {
  return kind === "share" || kind === "sftp" || kind === "arbesk";
}

/** Whether a place's files have known sizes and dates: Arbesk's are known
 *  only once an asset is built, so its size column stays empty. */
export function knowsSizes(kind: LocationKind): boolean {
  return kind !== "arbesk";
}

/** The remote places whose files Space previews: Arbesk's 3D models, built
 *  into the app cache. Drive, shares and SSH servers have no preview yet. */
export function previewsRemotely(kind: LocationKind): boolean {
  return kind === "arbesk";
}

/** Whether `path`'s place hands over its own thumbnails, whatever kind of
 *  file an item is: Arbesk, the one each asset was saved with. Elsewhere
 *  only pictures and documents get one (`THUMB_KINDS`, item-icons.ts). The
 *  Trash holds only local files, so it plays no part. */
export function providesThumbnails(path: string): boolean {
  return locationKind(path, "") === "arbesk";
}

/** `isReadOnlyKind` for `path`. */
export function isReadOnlyLocation(path: string): boolean {
  return isReadOnlyKind(locationKind(path, ""));
}

/** A path on this machine or a mounted volume, not a place or a remote. */
export function isLocalLocation(path: string): boolean {
  return kindIn(path, "", ["folder"]);
}

/** The Trash or a folder inside it; nothing until the Trash's path is known. */
export function isInTrash(path: string, trashPath: string): boolean {
  return isTrashKind(locationKind(path, trashPath));
}

/** Recursive name search is for local folders; the others filter in place. */
export function searchesRecursively(path: string, trashPath: string): boolean {
  return kindIn(path, trashPath, ["folder"]);
}

/** Whether to look for a versioned folder holding `path`: never for the Trash or a remote. */
export function mayHaveVersions(path: string, trashPath: string): boolean {
  return kindIn(path, trashPath, ["folder", "in-trash"]);
}

/**
 * Whether New Folder, New Document and Paste can add to a place of this
 * kind: a folder or Drive. Never the Trash or a folder in it: an item put
 * there has no record of where it came from, so it couldn't be restored and
 * Empty Trash would delete it for good.
 */
export function kindAcceptsNewItems(kind: LocationKind): boolean {
  return kind === "folder" || kind === "drive";
}

/** `kindAcceptsNewItems` for `path`. */
export function acceptsNewItems(path: string, trashPath: string): boolean {
  return kindAcceptsNewItems(locationKind(path, trashPath));
}

/** The copy-out refusal's words, by the first item's kind; a share's otherwise. */
const COPY_OUT_WORDS: Partial<Record<LocationKind, string>> = {
  sftp: "Copying from SSH servers comes next.",
  arbesk: "Copying from Arbesk comes next.",
};

/**
 * Why items of these kinds can't be copied, cut or dragged out of where they
 * are: shared folders, SSH servers and Arbesk can't be copied from yet. Null
 * when none of them is on one; the first item's kind picks the wording.
 */
export function copyOutRefusalOf(kinds: readonly LocationKind[]): string | null {
  if (!kinds.some(isReadOnlyKind)) return null;
  return COPY_OUT_WORDS[kinds[0]] ?? "Copying out of a shared folder comes next.";
}

/** `copyOutRefusalOf` for paths. */
export function copyOutRefusal(paths: readonly string[]): string | null {
  return copyOutRefusalOf(paths.map((path) => locationKind(path, "")));
}

/** What the item actions below need to know about an item. */
interface ItemFacts {
  path: string;
  is_dir: boolean;
  is_symlink: boolean;
}

/** A file (or a link to one) on this computer or a mounted volume: not a
 *  folder, and not on Drive, a share or an SSH server. */
export function isLocalFile(e: Omit<ItemFacts, "is_symlink">): boolean {
  return !e.is_dir && placeOf(e.path) === "local";
}

/** `isLocalFile`, and not a link: what Install Application… and Run in Terminal… take. */
export function isLocalRegularFile(e: ItemFacts): boolean {
  return isLocalFile(e) && !e.is_symlink;
}
