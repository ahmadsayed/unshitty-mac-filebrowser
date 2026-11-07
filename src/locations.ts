// What each kind of location allows. The app asks these questions instead of
// combining the path kinds itself, so the rules for Recent, Starred, Network,
// Google Drive, received shares, SSH servers and the Trash live in one place.
// Kept free of DOM and app state so every branch can be tested directly
// (tests/locations.test.ts).

import { isRemotePath, isSftpPath, isSharePath } from "./paths";

/** The places that aren't folders: Recent, Starred and Network. */
export type Pseudo = "recent" | "starred" | "network";

/** Every pseudo location starts with this. */
export const PSEUDO_PREFIX = "pseudo:";

export function isPseudo(path: string): path is `pseudo:${Pseudo}` {
  return path.startsWith(PSEUDO_PREFIX);
}

/** Shared folders and SSH servers are read only: no New Folder, Paste, Rename, Trash or drops. */
export function isReadOnlyLocation(path: string): boolean {
  return isSharePath(path) || isSftpPath(path);
}

/** A path on this machine or a mounted volume, not a place or a remote. */
export function isLocalLocation(path: string): boolean {
  return !isPseudo(path) && !isRemotePath(path);
}

/** The Trash or a folder inside it; nothing until the Trash's path is known. */
export function isInTrash(path: string, trashPath: string): boolean {
  return !!trashPath && (path === trashPath || path.startsWith(trashPath + "/"));
}

/** Recursive name search is for local folders; the others filter in place. */
export function searchesRecursively(path: string, trashPath: string): boolean {
  return isLocalLocation(path) && !isInTrash(path, trashPath);
}

/** Whether to look for a versioned folder holding `path`: never for the Trash or a remote. */
export function mayHaveVersions(path: string, trashPath: string): boolean {
  return isLocalLocation(path) && !(!!trashPath && path === trashPath);
}

/**
 * Whether New Folder, New Document and Paste can add to `path`. Never the
 * Trash or a folder in it: an item put there has no record of where it came
 * from, so it couldn't be restored and Empty Trash would delete it for good.
 */
export function acceptsNewItems(path: string, trashPath: string): boolean {
  return !isPseudo(path) && !isReadOnlyLocation(path) && !isInTrash(path, trashPath);
}

/** Whether an item at `path` can be renamed: not on a read-only remote, and not in the Trash, where its record names it. */
export function canRename(path: string, trashPath: string): boolean {
  return !isReadOnlyLocation(path) && !isInTrash(path, trashPath);
}

/** Whether Properties can describe an item at `path`: the backend can't read shared folders or SSH servers yet. */
export function hasProperties(path: string): boolean {
  return !isPseudo(path) && !isReadOnlyLocation(path);
}
