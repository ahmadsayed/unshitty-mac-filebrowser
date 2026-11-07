// Whether an action that several surfaces reach (menus, shortcuts, the Space
// preview, the list row's star, a drag) may run where it is asked, and the
// toast that says why not. The action functions in main.ts ask `refusal`
// first, so every surface that reaches one shares its rule; the menus keep
// their own per-place layouts and may still leave an allowed action out.
// Kept free of DOM and app state so every cell can be tested directly
// (tests/item-actions.test.ts).
//
// A few cells keep a difference between surfaces exactly as it was; each is
// marked "Kept:" and waits for a decision rather than being changed here.

import { copyOutRefusalOf, isPseudoKind, isTrashKind, type LocationKind } from "./locations";

export type SharedAction =
  | "cut"
  | "copy"
  | "paste"
  | "rename"
  | "trash"
  | "deletePermanently"
  | "star"
  | "bookmark"
  | "properties"
  | "openLocation";

/** Where an action is asked for. */
export interface Here {
  /** The place it happens in: the location on screen, or the folder a paste is for. */
  kind: LocationKind;
  /** The kinds of the items it acts on, in selection order; none for paste and bookmark. */
  items: readonly LocationKind[];
  /** The items aren't in the folder on screen: search results, Recent, Starred. */
  elsewhere?: boolean;
}

/** null: allowed; "": refused quietly; any other text: refused with that toast. */
export type Verdict = string | null;

const OK: Verdict = null;
const QUIET: Verdict = "";
const READ_ONLY: Verdict = "Shared folders are read only.";

/** Paste, by the kind of folder it pastes into. */
const PASTE: Record<LocationKind, Verdict> = {
  folder: OK, drive: OK,
  share: READ_ONLY, sftp: READ_ONLY,
  // Only Move to Trash puts items in the Trash: pasted ones would have no record.
  trash: QUIET, "in-trash": QUIET,
  recent: QUIET, starred: QUIET, network: QUIET,
};

/** Move to Trash, by the location on screen. In the Trash, Delete deletes permanently instead. */
const TRASH: Record<LocationKind, Verdict> = {
  folder: OK, drive: OK, recent: OK, starred: OK,
  // Kept: Delete in Network reaches Move to Trash, and only the backend refuses
  // shares and servers (its menu has no Move to Trash).
  network: OK,
  share: QUIET, sftp: QUIET, trash: QUIET, "in-trash": QUIET,
};

/** Delete Permanently, by the location on screen: only inside the Trash. */
const DELETE_PERMANENTLY: Record<LocationKind, Verdict> = {
  trash: OK, "in-trash": OK,
  folder: QUIET, drive: QUIET, recent: QUIET, starred: QUIET, network: QUIET, share: QUIET, sftp: QUIET,
};

/** Star and Unstar, by the location on screen. */
const STAR: Record<LocationKind, Verdict> = {
  folder: OK, recent: OK, starred: OK,
  // Kept: the list row's star shows on Drive, share and SSH rows, whose menus have no Star.
  drive: OK, share: OK, sftp: OK,
  trash: QUIET, "in-trash": QUIET, network: QUIET,
};

/** ⌘D, by the folder it bookmarks: anything but Recent, Starred and Network. */
const BOOKMARK: Record<LocationKind, Verdict> = {
  folder: OK, drive: OK,
  // Kept: ⌘D bookmarks a share, an SSH folder or a folder in the Trash, though
  // their location menus have no Add to Bookmarks.
  share: OK, sftp: OK, trash: OK, "in-trash": OK,
  recent: QUIET, starred: QUIET, network: QUIET,
};

/** Rename, by each item's kind: its record names a trashed item, and shares and servers are read only. */
const RENAME_ITEM: Record<LocationKind, Verdict> = {
  folder: OK, drive: OK, recent: OK, starred: OK, network: OK,
  share: QUIET, sftp: QUIET, trash: QUIET, "in-trash": QUIET,
};

/** Properties, by each item's kind: the backend can't read shares or SSH servers yet. */
const PROPERTIES_ITEM: Record<LocationKind, Verdict> = {
  folder: OK, drive: OK, trash: OK, "in-trash": OK,
  share: QUIET, sftp: QUIET,
  recent: QUIET, starred: QUIET, network: QUIET,
};

/** The first refusal among the items, by `grid`. */
function firstItemVerdict(grid: Record<LocationKind, Verdict>, here: Here): Verdict {
  for (const kind of here.items) if (grid[kind] !== OK) return grid[kind];
  return OK;
}

const copyOut = (here: Here): Verdict => copyOutRefusalOf(here.items);

const RULES: Record<SharedAction, (here: Here) => Verdict> = {
  cut: copyOut,
  copy: copyOut,
  paste: (here) => PASTE[here.kind],
  // Recent and Starred mix folders, so the name-clash check wouldn't work there.
  rename: (here) => (isPseudoKind(here.kind) ? QUIET : firstItemVerdict(RENAME_ITEM, here)),
  trash: (here) => TRASH[here.kind],
  deletePermanently: (here) => DELETE_PERMANENTLY[here.kind],
  star: (here) => STAR[here.kind],
  bookmark: (here) => BOOKMARK[here.kind],
  properties: (here) => firstItemVerdict(PROPERTIES_ITEM, here),
  // Only for an item shown away from its folder, and never in the Trash,
  // where Restore From Trash is the way back.
  openLocation: (here) =>
    here.elsewhere && !isTrashKind(here.kind) && !here.items.includes("in-trash") ? OK : QUIET,
};

/** Why `action` can't run `here`: null when it can (see `Verdict`). */
export function refusal(action: SharedAction, here: Here): Verdict {
  return RULES[action](here);
}

export function allows(action: SharedAction, here: Here): boolean {
  return refusal(action, here) === OK;
}
