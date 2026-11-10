// What each kind of location allows. The app asks these questions instead of
// combining the path kinds itself, so the rules for Recent, Starred, Network,
// Google Drive, received shares, SSH servers and the Trash live in one place.
// Kept free of DOM and app state so every branch can be tested directly
// (tests/locations.test.ts).
//
// What a place allows comes from the backend's providers (design D5): each
// rule that a place's capabilities answer reads them (`capabilitiesOf`,
// through `capabilityRow`) rather than restating them per kind. Rules about
// the frontend's own views (Recent, Starred, Network, the Trash and folders
// in it, which are no provider's) and those no capability matches stay here,
// each saying so.

import { capabilitiesOf, type CapabilityRow, type PlaceCapabilities } from "./capabilities";
import { isArbeskAsset, isArbeskCollection, isWithin, placeOf, PSEUDO_PREFIX, type Place, type Pseudo } from "./paths";

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

/** The frontend's own views: Recent, Starred, Network, the Trash and a
 *  folder in it. No provider answers for them (the Trash's items are this
 *  computer's, but what the view allows is the frontend's), so each rule
 *  keeps its own answer for them. */
export type ViewKind = "recent" | "starred" | "network" | "trash" | "in-trash";

const VIEW_KINDS: readonly LocationKind[] = ["recent", "starred", "network", "trash", "in-trash"];

/** Whether `kind` is one of the frontend's own views (`ViewKind`). */
function isViewKind(kind: LocationKind): kind is ViewKind {
  return VIEW_KINDS.includes(kind);
}

/** The backend's row for an Arbesk path's copies, by its number of
 *  segments: its asset or collection row, and its account's for anything
 *  else (an account, the root, a malformed or non-Arbesk path), which
 *  neither takes copies in nor gives them out. Only the copy flags hold for
 *  the root and a malformed path the account's row answers (a Rust test in
 *  `providers.rs` pins it): the provider refuses New Collection there,
 *  where the account row allows it (`copiesOf`). A malformed path shaped
 *  like a collection or an asset gets that row, as before step 7: the
 *  backend refuses its copy itself. */
function arbeskRow(path: string): CapabilityRow {
  if (isArbeskAsset(path)) return "arbesk-asset";
  return isArbeskCollection(path) ? "arbesk-collection" : "arbesk-account";
}

/** Which row of the backend's capability table answers for `kind`, at
 *  `path` where Arbesk's account, collection and asset differ (an Arbesk
 *  item whose path isn't known counts as a collection); null for the
 *  frontend's own views (`ViewKind`). With a path, the row is right for the
 *  copy flags only (`can_copy_in`, `can_copy_out`): the Arbesk root and a
 *  malformed path get the account's row, though their provider refuses New
 *  Collection, so read a path's row through `copiesOf`. */
export function capabilityRow(kind: LocationKind, path?: string): CapabilityRow | null {
  if (isViewKind(kind)) return null;
  if (kind === "arbesk") return path === undefined ? "arbesk-collection" : arbeskRow(path);
  return kind;
}

/** What the place of `kind` allows, as its provider says (the same for
 *  all its paths: for Arbesk, a collection's); null for the frontend's own
 *  views. */
function capsOf(kind: LocationKind): PlaceCapabilities | null {
  const row = capabilityRow(kind);
  return row ? capabilitiesOf(row) : null;
}

/** The copy flags of the place of `kind` at `path`, the only flags a
 *  path's row answers right for every Arbesk path (`capabilityRow`); null
 *  for the frontend's own views. */
function copiesOf(kind: LocationKind, path?: string): Pick<PlaceCapabilities, "can_copy_in" | "can_copy_out"> | null {
  const row = capabilityRow(kind, path);
  if (row === null) return null;
  const { can_copy_in, can_copy_out } = capabilitiesOf(row);
  return { can_copy_in, can_copy_out };
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

/** Whether a place is read only (the provider's `read_only`): shared
 *  folders, SSH servers and Arbesk. No New Folder, Paste, Rename, Trash or
 *  drops. The frontend's own views aren't. */
export function isReadOnlyKind(kind: LocationKind): boolean {
  return capsOf(kind)?.read_only ?? false;
}

/** Whether a place's files have known sizes and dates (the provider's
 *  `knows_sizes`): Arbesk's are known only once an asset is built, so its
 *  size column stays empty. The frontend's own views list items that do. */
export function knowsSizes(kind: LocationKind): boolean {
  return capsOf(kind)?.knows_sizes ?? true;
}

/** The remote places whose files Space previews: Arbesk's 3D models, built
 *  into the app cache. Drive, shares and SSH servers have no preview yet.
 *  Stays here: the preview is an extension, not one of the capabilities. */
export function previewsRemotely(kind: LocationKind): boolean {
  return kind === "arbesk";
}

/** Whether `path`'s place hands over its own thumbnails, whatever kind of
 *  file an item is: Arbesk, the one each asset was saved with. Elsewhere
 *  only pictures and documents get one (`THUMB_KINDS`, item-icons.ts). The
 *  Trash holds only local files, so it plays no part. Stays here: the
 *  capabilities' `has_thumbnails` says a place has thumbnails at all
 *  (this computer and Drive too), not that it has one for every file. */
export function providesThumbnails(path: string): boolean {
  return locationKind(path, "") === "arbesk";
}

/** `isReadOnlyKind` for `path`. */
export function isReadOnlyLocation(path: string): boolean {
  return isReadOnlyKind(locationKind(path, ""));
}

/** A path on this machine or a mounted volume, not a place or a remote.
 *  Where a path is, not what it allows: no capability. */
export function isLocalLocation(path: string): boolean {
  return kindIn(path, "", ["folder"]);
}

/** The Trash or a folder inside it; nothing until the Trash's path is known. */
export function isInTrash(path: string, trashPath: string): boolean {
  return isTrashKind(locationKind(path, trashPath));
}

/** Recursive name search is for places that search (the provider's
 *  `searchable`: this computer's folders); the others filter in place, and
 *  so do the frontend's own views, the Trash included. */
export function searchesRecursively(path: string, trashPath: string): boolean {
  return capsOf(locationKind(path, trashPath))?.searchable ?? false;
}

/** Whether to look for a versioned folder holding `path`: never for the
 *  Trash or a remote. Stays here: Keep Versions is the app's, not a
 *  place's capability. */
export function mayHaveVersions(path: string, trashPath: string): boolean {
  return kindIn(path, trashPath, ["folder", "in-trash"]);
}

/**
 * Whether New Folder and New Document can add to a place of this kind: one
 * that creates and isn't read only (the provider's `can_create` and
 * `read_only`), a folder or Drive. Paste reads `can_copy_in` instead
 * (`kindTakesPastes`). An Arbesk account creates only
 * collections, through New Collection, and stays read only. Never the Trash
 * or a folder in it (a view of the frontend's own): an item put there has
 * no record of where it came from, so it couldn't be restored and Empty
 * Trash would delete it for good.
 */
export function kindAcceptsNewItems(kind: LocationKind): boolean {
  const caps = capsOf(kind);
  return caps !== null && caps.can_create && !caps.read_only;
}

/** `kindAcceptsNewItems` for `path`. */
export function acceptsNewItems(path: string, trashPath: string): boolean {
  return kindAcceptsNewItems(locationKind(path, trashPath));
}

/** Whether Paste and a drop can put anything into a place of this kind
 *  as it is: one that takes copies and isn't read only (the provider's
 *  `can_copy_in` and `read_only`), a folder or Drive. An Arbesk collection
 *  takes copies but is read only: what it takes is `arbeskPasteRefusal`'s
 *  rule. Never the Trash or a folder in it, as `kindAcceptsNewItems`
 *  (a drop on the Trash is Move to Trash: `takesDrops`). */
export function kindTakesPastes(kind: LocationKind): boolean {
  const caps = capsOf(kind);
  return caps !== null && caps.can_copy_in && !caps.read_only;
}

/** Whether a drop can put anything into `path`'s place as it is
 *  (`kindTakesPastes`), the one rule Paste reads too. The Trash's path
 *  isn't passed: a drop on the Trash is Move to Trash, not a copy in, so
 *  the Trash and a folder in it count as this computer's folders here and
 *  the drop's own Trash rules decide. Arbesk's collections go through
 *  `arbeskPasteRefusal` first. */
export function takesDrops(path: string): boolean {
  return kindTakesPastes(locationKind(path, ""));
}

/** The copy-out refusal's words, by the first item's kind; a share's
 *  otherwise. The frontend's own (D5): the backend's differ. */
const COPY_OUT_WORDS: Partial<Record<LocationKind, string>> = {
  sftp: "Copying from SSH servers comes next.",
  arbesk: "Copying Arbesk collections comes next.",
};

/** Whether the item at `path`, of `kind`, can't be copied out of where it
 *  is (the provider's `can_copy_out`): anything in a share or on an SSH
 *  server, and an Arbesk account or collection. An Arbesk asset can be (its
 *  file is built first); one whose path isn't known counts as a collection.
 *  Items in the frontend's own views are their places' items. */
function staysPut(kind: LocationKind, path: string | undefined): boolean {
  const caps = copiesOf(kind, path);
  return caps !== null && !caps.can_copy_out;
}

/**
 * Why items of these kinds, at these paths (in the same order), can't be
 * copied or dragged out of where they are: shared folders, SSH servers and
 * Arbesk collections can't be copied from yet. Null when none of them is
 * one; the first item's kind picks the wording.
 */
export function copyOutRefusalOf(kinds: readonly LocationKind[], paths: readonly string[] = []): string | null {
  if (!kinds.some((kind, i) => staysPut(kind, paths[i]))) return null;
  const first = staysPut(kinds[0], paths[0]) ? COPY_OUT_WORDS[kinds[0]] : undefined;
  return first ?? "Copying out of a shared folder comes next.";
}

/** `copyOutRefusalOf` for paths. */
export function copyOutRefusal(paths: readonly string[]): string | null {
  return copyOutRefusalOf(paths.map((path) => locationKind(path, "")), paths);
}

const ARBESK_MODEL = /\.(gltf|glb|3mf)$/i;

/** Whether `path` is a file Arbesk takes: a local `.gltf`, `.glb` or `.3mf`.
 *  Stays here: the capabilities say where copies go in, not which files. */
export function isArbeskModelFile(path: string): boolean {
  return placeOf(path) === "local" && ARBESK_MODEL.test(path);
}

/** Why `paths` can't be pasted or dropped into `dir` in Arbesk, in order:
 *  a cut or move, a folder that takes no copies (the provider's
 *  `can_copy_in`: only a collection does), an item that isn't a model file
 *  on this computer (any of `dirs`, the folders among `paths`, whatever its
 *  name). Null when only copies of model files go into a collection. */
export function arbeskPasteRefusal(dir: string, paths: readonly string[], moving: boolean, dirs: ReadonlySet<string> = new Set()): string | null {
  if (moving) return "Arbesk is read only.";
  if (!copiesOf("arbesk", dir)?.can_copy_in) return "Paste into a collection.";
  if (!paths.every((path) => !dirs.has(path) && isArbeskModelFile(path))) return "Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk.";
  return null;
}

/** The name a copied file has once Arbesk lists it: a `.glb` is stored as
 *  glTF, and the extension is listed in lower case (the stem keeps its case). */
export function arbeskTargetName(name: string): string {
  return name.replace(/\.glb$/i, ".gltf").replace(/\.(gltf|3mf)$/i, (ext) => ext.toLowerCase());
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
