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
//
// Where a place's capabilities answer a rule (design D5: rename, Move to
// Trash, copy out, paste through `can_copy_in`, cut), it reads them, through `locations.ts` or
// `byPlace`, and keeps cells only for the frontend's own views (Recent,
// Starred, Network, the Trash); adding files to an asset reads `can_attach`.
// Delete Permanently, Star, Add to Bookmarks,
// Properties, Open With and Open Item Location have no matching capability
// and keep their tables. The toasts are the frontend's own either way.

import { capabilitiesOf, type PlaceCapabilities } from "./capabilities";
import { arbeskPasteRefusal, attachRefusal, capabilityRow, copyOutRefusalOf, isPseudoKind, isReadOnlyKind, isTrashKind, kindTakesPastes, type LocationKind, type ViewKind } from "./locations";

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
  | "openWith"
  | "openLocation"
  | "attach";

/** Where an action is asked for. */
export interface Here {
  /** The place it happens in: the location on screen, or the folder a paste is for. */
  kind: LocationKind;
  /** The kinds of the items it acts on, in selection order; none for paste and bookmark. */
  items: readonly LocationKind[];
  /** The items' paths, as `items`; cut and copy tell an Arbesk asset from a collection by them. */
  paths?: readonly string[];
  /** The items aren't in the folder on screen: search results, Recent, Starred. */
  elsewhere?: boolean;
  /** A paste's folder (a drop's too), which Arbesk checks is a collection;
   *  for `attach`, the asset the files are added to. */
  dir?: string;
  /** A paste that moves (Cut, a forced-move drag): Arbesk takes copies only. */
  moving?: boolean;
  /** The folders among `paths`, which Arbesk never takes. */
  dirs?: ReadonlySet<string>;
}

/** null: allowed; "": refused quietly; any other text: refused with that toast. */
export type Verdict = string | null;

const OK: Verdict = null;
const QUIET: Verdict = "";
const READ_ONLY: Verdict = "Shared folders are read only.";
const ARBESK_READ_ONLY: Verdict = "Arbesk is read only.";

/** A rule over kinds: what an action is allowed in each. */
type Grid = (kind: LocationKind) => Verdict;

/** A grid from a place's capability `flag` (allowed when it holds, refused
 *  quietly when not), with `views`' cells for the frontend's own views. */
function byPlace(flag: keyof PlaceCapabilities, views: Record<ViewKind, Verdict>): Grid {
  return (kind) => {
    const row = capabilityRow(kind);
    if (row === null) return views[kind as ViewKind];
    return capabilitiesOf(row)[flag] ? OK : QUIET;
  };
}

/** Paste, by the kind of folder it pastes into: wherever the place takes
 *  copies as it is (`kindTakesPastes`: `can_copy_in`, not read only, which
 *  keeps the Trash out); shares, servers and Arbesk say why not (Arbesk
 *  takes copies of model files into a collection). Kept: an SSH server says
 *  "Shared folders are read only." */
const paste = (here: Here): Verdict => {
  if (kindTakesPastes(here.kind)) return OK;
  if (here.kind === "arbesk") return arbeskPasteRefusal(here.dir ?? "", here.paths ?? [], here.moving ?? false, here.dirs);
  return isReadOnlyKind(here.kind) ? READ_ONLY : QUIET;
};

/** Move to Trash, by the location on screen: where the place trashes
 *  (`can_trash`: a folder, Drive; not shares, servers or Arbesk). In the
 *  Trash, Delete deletes permanently instead. */
const TRASH = byPlace("can_trash", {
  recent: OK, starred: OK,
  // Kept: Delete in Network reaches Move to Trash, and only the backend refuses
  // shares and servers (its menu has no Move to Trash).
  network: OK,
  trash: QUIET, "in-trash": QUIET,
});

/** Delete Permanently, by the location on screen: only inside the Trash, a
 *  view of the frontend's own (no capability). */
const DELETE_PERMANENTLY: Record<LocationKind, Verdict> = {
  trash: OK, "in-trash": OK,
  folder: QUIET, drive: QUIET, recent: QUIET, starred: QUIET, network: QUIET, share: QUIET, sftp: QUIET, arbesk: QUIET,
};

/** Star and Unstar, by the location on screen. Stars are the frontend's
 *  own (no capability). */
const STAR: Record<LocationKind, Verdict> = {
  folder: OK, recent: OK, starred: OK,
  // Kept: the list row's star shows on Drive, share and SSH rows, whose menus have no Star.
  drive: OK, share: OK, sftp: OK,
  // Starred lists stars by asking each path's folder; Arbesk items aren't starred.
  arbesk: QUIET,
  trash: QUIET, "in-trash": QUIET, network: QUIET,
};

/** Add to Bookmarks, by the folder it bookmarks: anything but Recent,
 *  Starred and Network. Bookmarks are the frontend's own (no capability). */
const BOOKMARK: Record<LocationKind, Verdict> = {
  folder: OK,
  // Kept: ⌘D bookmarks a Drive folder, a share, an SSH folder or a folder in
  // the Trash, though their location menus have no Add to Bookmarks.
  drive: OK, share: OK, sftp: OK, trash: OK, "in-trash": OK,
  // A collection can be bookmarked like an SSH folder.
  arbesk: OK,
  recent: QUIET, starred: QUIET, network: QUIET,
};

/** Rename, by each item's kind: where the place renames (`can_rename`: a
 *  folder, Drive; shares, servers and Arbesk are read only); its record
 *  names a trashed item. */
const RENAME_ITEM = byPlace("can_rename", {
  recent: OK, starred: OK, network: OK,
  trash: QUIET, "in-trash": QUIET,
});

/** Properties, by each item's kind: the backend can't read shares or SSH
 *  servers yet. Properties is an extension, not a capability: no row
 *  answers it. Arbesk's cell answers an item whose path isn't known;
 *  with its path, `propertiesOf` reads its row (spec F7). */
const PROPERTIES_ITEM: Record<LocationKind, Verdict> = {
  folder: OK, drive: OK, trash: OK, "in-trash": OK,
  share: QUIET, sftp: QUIET, arbesk: QUIET,
  recent: QUIET, starred: QUIET, network: QUIET,
};

/** Of Arbesk's rows, only an asset has Properties (its version, its
 *  files); an account, a collection and an asset's other file don't. */
const ARBESK_PROPERTIES: Record<"arbesk-account" | "arbesk-collection" | "arbesk-asset" | "arbesk-file", Verdict> = {
  "arbesk-asset": OK,
  "arbesk-account": QUIET, "arbesk-collection": QUIET, "arbesk-file": QUIET,
};

/** Properties of the item of `kind` at `path`: by its Arbesk row, read
 *  from its path as copy out reads it (the `arbesk` cell can't tell an
 *  asset from a collection); every other kind by its cell. */
function propertiesOf(kind: LocationKind, path: string | undefined): Verdict {
  if (kind !== "arbesk" || path === undefined) return PROPERTIES_ITEM[kind];
  return ARBESK_PROPERTIES[capabilityRow(kind, path) as keyof typeof ARBESK_PROPERTIES];
}

/** Properties: the first refusal among the items, each by `propertiesOf`. */
const properties = (here: Here): Verdict => {
  for (const [i, kind] of here.items.entries()) {
    const verdict = propertiesOf(kind, here.paths?.[i]);
    if (verdict !== OK) return verdict;
  }
  return OK;
};

/** Open With…, by the item's kind: a file or folder on this computer
 *  (the app is handed the path itself); Drive, shares, servers and the
 *  frontend's Network hand no local path over. Arbesk's cell answers an
 *  item whose path isn't known; with its path, its row decides (spec F8). */
const OPEN_WITH_ITEM: Record<LocationKind, Verdict> = {
  folder: OK, recent: OK, starred: OK, trash: OK, "in-trash": OK,
  drive: QUIET, share: QUIET, sftp: QUIET, arbesk: QUIET, network: QUIET,
};

/** Of Arbesk's rows, only an asset opens with an application: its model
 *  or one of its other files, fetched first (spec F8). */
const ARBESK_OPEN_WITH: Record<"arbesk-account" | "arbesk-collection" | "arbesk-asset" | "arbesk-file", Verdict> = {
  "arbesk-asset": OK,
  "arbesk-account": QUIET, "arbesk-collection": QUIET, "arbesk-file": QUIET,
};

/** Open With…: a single item, by `OPEN_WITH_ITEM`, an Arbesk one by its
 *  row read from its path (as `propertiesOf`). */
const openWith = (here: Here): Verdict => {
  if (here.items.length !== 1) return QUIET;
  const [kind, path] = [here.items[0], here.paths?.[0]];
  if (kind !== "arbesk" || path === undefined) return OPEN_WITH_ITEM[kind];
  return ARBESK_OPEN_WITH[capabilityRow(kind, path) as keyof typeof ARBESK_OPEN_WITH];
};

/** The first refusal among the items, by `grid`. */
function firstItemVerdict(grid: Grid, here: Here): Verdict {
  for (const kind of here.items) if (grid(kind) !== OK) return grid(kind);
  return OK;
}

const copyOut = (here: Here): Verdict => copyOutRefusalOf(here.items, here.paths);

/** Cut's toast for an item of a read-only `kind`: Arbesk's own words, a
 *  share's otherwise. The frontend's own wording, by kind. */
const cutRefusalOf = (kind: LocationKind): Verdict => (kind === "arbesk" ? ARBESK_READ_ONLY : READ_ONLY);

/** Cut: as copy, and never out of a read-only place: Arbesk, whose assets
 *  can only be copied (shares and servers give nothing out today, refused
 *  above; the toast is picked by the item's kind, so it stays right if
 *  they ever do). */
const cut = (here: Here): Verdict => {
  const stuck = here.items.find(isReadOnlyKind);
  return copyOut(here) ?? (stuck === undefined ? OK : cutRefusalOf(stuck));
};

/** Paste Into Asset and a drop on an asset (spec F10): files on this
 *  computer, copied, onto an item whose place takes them (`can_attach`:
 *  an Arbesk asset), each with a type to record. */
const attach = (here: Here): Verdict => attachRefusal(here.dir ?? "", here.paths ?? [], here.moving ?? false, here.dirs);

const RULES: Record<SharedAction, (here: Here) => Verdict> = {
  cut,
  copy: copyOut,
  paste,
  // Recent and Starred mix folders, so the name-clash check wouldn't work there.
  rename: (here) => (isPseudoKind(here.kind) ? QUIET : firstItemVerdict(RENAME_ITEM, here)),
  trash: (here) => TRASH(here.kind),
  deletePermanently: (here) => DELETE_PERMANENTLY[here.kind],
  star: (here) => STAR[here.kind],
  bookmark: (here) => BOOKMARK[here.kind],
  properties,
  openWith,
  attach,
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
