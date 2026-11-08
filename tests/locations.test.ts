// What each kind of location allows (src/locations.ts). One row per kind of
// place, one column per question the app asks, so a new kind of location or a
// changed rule shows up here as a single cell.
import { expect, test } from "vitest";
import {
  acceptsNewItems,
  copyOutRefusal,
  isInTrash,
  isLocalFile,
  isLocalLocation,
  isLocalRegularFile,
  isPseudo,
  isReadOnlyLocation,
  locationKind,
  mayHaveVersions,
  searchesRecursively,
  knowsSizes,
  previewsRemotely,
  type LocationKind,
} from "../src/locations";

// The Trash folder the app shows: Finder's on macOS, freedesktop's files/ on Linux.
const TRASH = "/Users/me/.Trash";
const LINUX_TRASH = "/home/me/.local/share/Trash/files";

type Row = {
  path: string;
  /** The Trash's path, when it isn't the macOS one. */
  trash?: string;
  kind: LocationKind;
  pseudo: boolean;
  readOnly: boolean;
  local: boolean;
  inTrash: boolean;
  searches: boolean;
  versions: boolean;
  newItems: boolean;
};

const F = false;
const T = true;

// prettier-ignore
const ROWS: Record<string, Row> = {
  "a local folder":          { path: "/Users/me/Documents",              kind: "folder", pseudo: F, readOnly: F, local: T, inTrash: F, searches: T, versions: T, newItems: T },
  "the root":                { path: "/",                                kind: "folder", pseudo: F, readOnly: F, local: T, inTrash: F, searches: T, versions: T, newItems: T },
  // Nothing is pasted or made in the Trash: its items' records would be wrong.
  "the Trash":               { path: TRASH,                              kind: "trash", pseudo: F, readOnly: F, local: T, inTrash: T, searches: F, versions: F, newItems: F },
  // Only the Trash itself is left out of the versions lookup.
  "a folder in the Trash":   { path: `${TRASH}/old`,                     kind: "in-trash", pseudo: F, readOnly: F, local: T, inTrash: T, searches: F, versions: T, newItems: F },
  "the Linux Trash":         { path: LINUX_TRASH, trash: LINUX_TRASH,    kind: "trash", pseudo: F, readOnly: F, local: T, inTrash: T, searches: F, versions: F, newItems: F },
  "a folder in the Linux Trash": { path: `${LINUX_TRASH}/old/sub`, trash: LINUX_TRASH, kind: "in-trash", pseudo: F, readOnly: F, local: T, inTrash: T, searches: F, versions: T, newItems: F },
  // The rest of the Linux Trash (info/) isn't the Trash folder the app shows.
  "the Linux Trash's records": { path: "/home/me/.local/share/Trash/info", trash: LINUX_TRASH, kind: "folder", pseudo: F, readOnly: F, local: T, inTrash: F, searches: T, versions: T, newItems: T },
  "a name that starts like the Trash": { path: `${TRASH}can`,            kind: "folder", pseudo: F, readOnly: F, local: T, inTrash: F, searches: T, versions: T, newItems: T },
  "Recent":                  { path: "pseudo:recent",                    kind: "recent", pseudo: T, readOnly: F, local: F, inTrash: F, searches: F, versions: F, newItems: F },
  "Network":                 { path: "pseudo:network",                   kind: "network", pseudo: T, readOnly: F, local: F, inTrash: F, searches: F, versions: F, newItems: F },
  "a Google Drive folder":   { path: "gdrive://acct/root~My%20Drive",    kind: "drive", pseudo: F, readOnly: F, local: F, inTrash: F, searches: F, versions: F, newItems: T },
  "a received share":        { path: "share://r1/sub",                   kind: "share", pseudo: F, readOnly: T, local: F, inTrash: F, searches: F, versions: F, newItems: F },
  "an SSH server":           { path: "sftp://me@host/home",              kind: "sftp", pseudo: F, readOnly: T, local: F, inTrash: F, searches: F, versions: F, newItems: F },
  "an Arbesk collection":    { path: "arbesk://0x00000000000000000000000000000000000000ab/1~Set", kind: "arbesk", pseudo: F, readOnly: T, local: F, inTrash: F, searches: F, versions: F, newItems: F },
};

for (const [name, row] of Object.entries(ROWS)) {
  test(`${name}: what it allows`, () => {
    const trash = row.trash ?? TRASH;
    expect({
      kind: locationKind(row.path, trash),
      pseudo: isPseudo(row.path),
      readOnly: isReadOnlyLocation(row.path),
      local: isLocalLocation(row.path),
      inTrash: isInTrash(row.path, trash),
      searches: searchesRecursively(row.path, trash),
      versions: mayHaveVersions(row.path, trash),
      newItems: acceptsNewItems(row.path, trash),
    }).toEqual({
      kind: row.kind,
      pseudo: row.pseudo,
      readOnly: row.readOnly,
      local: row.local,
      inTrash: row.inTrash,
      searches: row.searches,
      versions: row.versions,
      newItems: row.newItems,
    });
  });
}

test("nothing is in the Trash before its path is known", () => {
  expect(isInTrash(TRASH, "")).toBe(false);
  expect(isInTrash("/", "")).toBe(false);
  expect(searchesRecursively(TRASH, "")).toBe(true);
  expect(mayHaveVersions(TRASH, "")).toBe(true);
  expect(acceptsNewItems(TRASH, "")).toBe(true);
});

test("the kind of each place, in one precedence", () => {
  expect(locationKind("pseudo:starred", TRASH)).toBe("starred");
  // No window opens a pseudo place that doesn't exist (isKnownPath refuses it); one is taken as Recent.
  expect(locationKind("pseudo:elsewhere", TRASH)).toBe("recent");
  expect(locationKind("pseudo:constructor", TRASH)).toBe("recent");
  // Nothing is in the Trash until its path is known.
  expect(locationKind(TRASH, "")).toBe("folder");
  expect(locationKind(`${TRASH}/old`, "")).toBe("folder");
});

test("sizes are known everywhere but Arbesk, and only Arbesk previews remotely", () => {
  const kinds: LocationKind[] = ["recent", "starred", "network", "trash", "in-trash", "share", "sftp", "drive", "arbesk", "folder"];
  expect(kinds.filter((kind) => !knowsSizes(kind))).toEqual(["arbesk"]);
  expect(kinds.filter(previewsRemotely)).toEqual(["arbesk"]);
});

test("the copy-out refusal, in the first item's words", () => {
  expect(copyOutRefusal(["arbesk://0x00000000000000000000000000000000000000ab/1~S/a~B.gltf", "sftp://me@host/b"])).toBe("Copying from Arbesk comes next.");
  expect(copyOutRefusal(["/Users/me/a.txt", "gdrive://acct/x~a"])).toBeNull();
  expect(copyOutRefusal(["share://r1/a.txt"])).toBe("Copying out of a shared folder comes next.");
  expect(copyOutRefusal(["sftp://me@host/a.txt", "share://r1/b"])).toBe("Copying from SSH servers comes next.");
  expect(copyOutRefusal(["/Users/me/a.txt", "sftp://me@host/b"])).toBe("Copying out of a shared folder comes next.");
  expect(copyOutRefusal([])).toBeNull();
});

test("a local file, and a local regular file, for the menu items that run or send one", () => {
  const file = { path: "/Users/me/a.dmg", is_dir: false, is_symlink: false };
  expect([isLocalFile(file), isLocalRegularFile(file)]).toEqual([true, true]);
  // Send with QR Code… takes a link to a file; Install Application… and Run in Terminal… don't.
  expect([isLocalFile({ ...file, is_symlink: true }), isLocalRegularFile({ ...file, is_symlink: true })]).toEqual([true, false]);
  for (const other of [{ ...file, is_dir: true }, { ...file, path: "gdrive://acct/x~a.dmg" }, { ...file, path: "share://r1/a.dmg" }, { ...file, path: "sftp://me@host/a.dmg" }]) {
    expect([isLocalFile(other), isLocalRegularFile(other)]).toEqual([false, false]);
  }
});
