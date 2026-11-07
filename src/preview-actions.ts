// Actions in the Space preview (#48), in the order the buttons show, from
// the rules every surface shares (src/item-actions.ts).

import { allows, type Here } from "./item-actions";
import { isPseudoKind, isTrashKind } from "./locations";

export type PreviewAction = "location" | "rename" | "trash" | "open";

/** Which actions the previewed item gets, as far as its place allows. */
export interface PreviewPlace {
  /** In the Trash, where Restore and Delete Permanently are the menu's job: Open only. */
  inTrash: boolean;
  location: boolean;
  rename: boolean;
  trash: boolean;
}

export const PREVIEW_ACTION_LABELS: Record<PreviewAction, string> = {
  location: "Open Item Location",
  rename: "Rename…",
  trash: "Move to Trash",
  open: "Open",
};

/**
 * What the preview offers for the one item in `here`. Kept as they were:
 * only items on this computer get Rename… and Move to Trash, so a Drive
 * item's preview lacks the two its menu offers; and the Trash folder itself
 * (shown with hidden files) gets Rename…, which F2 and the menus refuse.
 */
export function previewPlace(here: Here): PreviewPlace {
  const item = here.items[0];
  const local = item === "folder" || isTrashKind(item);
  const trashFolder = item === "trash" && !isPseudoKind(here.kind);
  return {
    inTrash: item === "in-trash" || isTrashKind(here.kind),
    location: allows("openLocation", here),
    rename: local && (allows("rename", here) || trashFolder),
    trash: local && allows("trash", here),
  };
}

/** The actions the preview offers for an item in `place`. */
export function previewActions(place: PreviewPlace): PreviewAction[] {
  if (place.inTrash) return ["open"];
  return [
    ...(place.location ? (["location"] as const) : []),
    ...(place.rename ? (["rename"] as const) : []),
    ...(place.trash ? (["trash"] as const) : []),
    "open",
  ];
}
