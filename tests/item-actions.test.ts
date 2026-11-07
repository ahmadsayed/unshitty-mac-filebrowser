// The shared action rules (src/item-actions.ts), one cell per action and
// kind of place, so a changed rule shows up here as a single cell. The
// surfaces themselves are pinned by tests/item-actions-matrix.test.ts.
import { expect, test } from "vitest";
import { allows, refusal, type Here, type SharedAction } from "../src/item-actions";
import type { LocationKind } from "../src/locations";

const KINDS: LocationKind[] = ["recent", "starred", "network", "trash", "in-trash", "share", "sftp", "drive", "folder"];

// ✓ allowed, · refused quietly, R "Shared folders are read only."
const Y = "✓";
const N = "·";
const R = "R";
const SHOWN: Record<string, string | null> = { [Y]: null, [N]: "", [R]: "Shared folders are read only." };

// Actions decided by the place they happen in (for paste: the folder pasted into).
// prettier-ignore
const BY_PLACE: Partial<Record<SharedAction, string[]>> = {
  //                 recent starred network trash in-trash share sftp drive folder
  paste:             [N,    N,      N,      N,    N,       R,    R,   Y,    Y],
  // Kept: Network ✓, which only the backend refuses.
  trash:             [Y,    Y,      Y,      N,    N,       N,    N,   Y,    Y],
  deletePermanently: [N,    N,      N,      Y,    Y,       N,    N,   N,    N],
  // Kept: share, sftp and drive ✓, for the list row's star.
  star:              [Y,    Y,      N,      N,    N,       Y,    Y,   Y,    Y],
  // Kept: trash, in-trash, share and sftp ✓, through ⌘D.
  bookmark:          [N,    N,      N,      Y,    Y,       Y,    Y,   Y,    Y],
};

for (const [action, cells] of Object.entries(BY_PLACE) as [SharedAction, string[]][]) {
  test(`${action}, by the kind of place`, () => {
    const got = KINDS.map((kind) => refusal(action, { kind, items: [] }));
    expect(got).toEqual(cells.map((cell) => SHOWN[cell]));
  });
}

// Actions decided by the items' own kinds, from a folder on screen.
// prettier-ignore
const BY_ITEM: Partial<Record<SharedAction, string[]>> = {
  //          recent starred network trash in-trash share sftp drive folder
  rename:     [Y,    Y,      Y,      N,    N,       N,    N,   Y,    Y],
  properties: [N,    N,      N,      Y,    Y,       N,    N,   Y,    Y],
};

for (const [action, cells] of Object.entries(BY_ITEM) as [SharedAction, string[]][]) {
  test(`${action}, by the items' kind`, () => {
    const got = KINDS.map((item) => refusal(action, { kind: "folder", items: ["folder", item] }));
    expect(got).toEqual(cells.map((cell) => SHOWN[cell]));
  });
}

test("rename: never in Recent, Starred or Network, which mix folders", () => {
  for (const kind of ["recent", "starred", "network"] as const) {
    expect(allows("rename", { kind, items: ["folder"] })).toBe(false);
  }
  expect(allows("rename", { kind: "drive", items: ["drive"] })).toBe(true);
});

test("cut, copy and a drag: not out of a share or an SSH server yet, said in the first item's words", () => {
  for (const action of ["cut", "copy"] as const) {
    const at = (items: LocationKind[]): Here => ({ kind: "network", items });
    expect(refusal(action, at(["folder", "drive", "in-trash"]))).toBeNull();
    expect(refusal(action, at(["share", "sftp"]))).toBe("Copying out of a shared folder comes next.");
    expect(refusal(action, at(["sftp", "share"]))).toBe("Copying from SSH servers comes next.");
    // Mixed with a local item the share wording stands, as before.
    expect(refusal(action, at(["folder", "sftp"]))).toBe("Copying out of a shared folder comes next.");
  }
});

test("openLocation: for an item shown away from its folder, never in the Trash", () => {
  const away: Here = { kind: "recent", items: ["folder"], elsewhere: true };
  expect(allows("openLocation", away)).toBe(true);
  expect(allows("openLocation", { ...away, elsewhere: false })).toBe(false);
  expect(allows("openLocation", { ...away, kind: "trash" })).toBe(false);
  expect(allows("openLocation", { ...away, items: ["in-trash"] })).toBe(false);
  expect(allows("openLocation", { kind: "folder", items: ["folder"] })).toBe(false);
});
