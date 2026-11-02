// Actions in the Space preview (#48), in the order the buttons show.

export type PreviewAction = "location" | "rename" | "trash" | "open";

/** Where the previewed item is, as far as its actions are concerned. */
export interface PreviewPlace {
  /** On this computer or a mounted volume: not Drive, a share or SFTP. */
  local: boolean;
  /** In the Trash, where Restore and Delete Permanently are the menu's job. */
  inTrash: boolean;
  /** Not in the folder on screen: a search result, Recent or Starred. */
  elsewhere: boolean;
}

export const PREVIEW_ACTION_LABELS: Record<PreviewAction, string> = {
  location: "Open Item Location",
  rename: "Rename…",
  trash: "Move to Trash",
  open: "Open",
};

/** The actions the preview offers for an item in `place`. */
export function previewActions(place: PreviewPlace): PreviewAction[] {
  if (place.inTrash) return ["open"];
  return [
    ...(place.elsewhere ? (["location"] as const) : []),
    ...(place.local ? (["rename", "trash"] as const) : []),
    "open",
  ];
}
