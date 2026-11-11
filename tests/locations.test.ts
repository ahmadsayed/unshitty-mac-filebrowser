// What each kind of location allows (src/locations.ts). One row per kind of
// place, one column per question the app asks, so a new kind of location or a
// changed rule shows up here as a single cell.
import { expect, test } from "vitest";
import {
  acceptsNewItems,
  arbeskPasteRefusal,
  arbeskTargetName,
  capabilityRow,
  isArbeskModelFile,
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
  providesThumbnails,
  type LocationKind,
} from "../src/locations";

// The Trash folder the app shows: Finder's on macOS, freedesktop's files/ on Linux.
const TRASH = "/Users/me/.Trash";

/** A path of each kind of place. */
const PLACE_PATHS: Record<LocationKind, string> = {
  recent: "pseudo:recent",
  starred: "pseudo:starred",
  network: "pseudo:network",
  trash: TRASH,
  "in-trash": `${TRASH}/old.png`,
  share: "share://r1/Photos/a.png",
  sftp: "sftp://me@host/a.png",
  drive: "gdrive://acct/x~a.png",
  arbesk: "arbesk://0x00000000000000000000000000000000000000ab/1~S/a~B.gltf",
  folder: "/Users/me/a.gltf",
};
const locationOf = (kind: LocationKind) => PLACE_PATHS[kind];
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

test("only Arbesk hands over its own thumbnails, whatever kind of file an item is", () => {
  const kinds: LocationKind[] = ["recent", "starred", "network", "trash", "in-trash", "share", "sftp", "drive", "arbesk", "folder"];
  expect(kinds.filter((kind) => providesThumbnails(locationOf(kind)))).toEqual(["arbesk"]);
});

test("the copy-out refusal, in the first item's words", () => {
  const account = "arbesk://0x00000000000000000000000000000000000000ab";
  // An Arbesk asset can be copied out; an account or a collection can't yet.
  expect(copyOutRefusal([`${account}/1~S/a~B.gltf`, `${account}/1~S/b~C.3mf`, "/Users/me/a.txt"])).toBeNull();
  expect(copyOutRefusal([`${account}/1~S`, `${account}/1~S/a~B.gltf`])).toBe("Copying Arbesk collections comes next.");
  expect(copyOutRefusal([account])).toBe("Copying Arbesk collections comes next.");
  // An asset that can go doesn't pick the words; as for a local item, the share's stand.
  expect(copyOutRefusal([`${account}/1~S/a~B.gltf`, "sftp://me@host/b"])).toBe("Copying out of a shared folder comes next.");
  expect(copyOutRefusal(["/Users/me/a.txt", "gdrive://acct/x~a"])).toBeNull();
  expect(copyOutRefusal(["share://r1/a.txt"])).toBe("Copying out of a shared folder comes next.");
  expect(copyOutRefusal(["sftp://me@host/a.txt", "share://r1/b"])).toBe("Copying from SSH servers comes next.");
  expect(copyOutRefusal(["/Users/me/a.txt", "sftp://me@host/b"])).toBe("Copying out of a shared folder comes next.");
  expect(copyOutRefusal([])).toBeNull();
});

test("an Arbesk asset's other file has its own row, never the account's", () => {
  const asset = "arbesk://0x00000000000000000000000000000000000000ab/1~S/a~B.gltf";
  for (const file of [`${asset}/stl~B.stl`, `${asset}/gcode@bafyG1~B.gcode`]) {
    expect(locationKind(file, ""), file).toBe("arbesk");
    expect(capabilityRow("arbesk", file), file).toBe("arbesk-file");
    // It copies out as its asset does (the account's row would refuse it).
    expect(copyOutRefusal([file]), file).toBeNull();
  }
  // A fourth segment that isn't a file's stays the account's, as the
  // backend answers it (providers.rs' malformed-path test).
  for (const file of ["x", "GCODE~x", "primary~x", "stl@~x", "stl@ba-fy~x"]) {
    expect(capabilityRow("arbesk", `${asset}/${file}`), file).toBe("arbesk-account");
  }
});

const COLLECTION = "arbesk://0x00000000000000000000000000000000000000ab/1~S";
const ONLY_MODELS = "Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk.";

test("only .gltf, .glb and .3mf files on this computer are Arbesk model files", () => {
  for (const ok of ["/Users/me/a.gltf", "/Users/me/B.GLB", "/Users/me/c.3Mf"]) expect(isArbeskModelFile(ok), ok).toBe(true);
  for (const no of ["/Users/me/a.txt", "/Users/me/gltf", "/Users/me/a.gltf.bak", "gdrive://acct/x~a.glb", "share://r1/a.glb", "sftp://me@host/a.glb", `${COLLECTION}/a~B.gltf`]) {
    expect(isArbeskModelFile(no), no).toBe(false);
  }
});

test("the paste refusal for an Arbesk folder, in order", () => {
  const model = ["/Users/me/a.glb"];
  expect(arbeskPasteRefusal(COLLECTION, model, true)).toBe("Arbesk is read only.");
  expect(arbeskPasteRefusal("arbesk://0x00000000000000000000000000000000000000ab", model, true)).toBe("Arbesk is read only.");
  expect(arbeskPasteRefusal("arbesk://0x00000000000000000000000000000000000000ab", model, false)).toBe("Paste into a collection.");
  expect(arbeskPasteRefusal("", model, false)).toBe("Paste into a collection.");
  expect(arbeskPasteRefusal(COLLECTION, ["/Users/me/a.glb", "/Users/me/a.txt"], false)).toBe(ONLY_MODELS);
  expect(arbeskPasteRefusal(COLLECTION, ["gdrive://acct/x~a.glb"], false)).toBe(ONLY_MODELS);
  expect(arbeskPasteRefusal(COLLECTION, [], false)).toBeNull();
  expect(arbeskPasteRefusal(COLLECTION, [...model, "/Users/me/b.3mf"], false)).toBeNull();
});

test("a folder is never a model, whatever its name", () => {
  const folder = "/Users/me/Kit.glb";
  expect(arbeskPasteRefusal(COLLECTION, [folder], false, new Set([folder]))).toBe(ONLY_MODELS);
  expect(arbeskPasteRefusal(COLLECTION, ["/Users/me/a.3mf", folder], false, new Set([folder]))).toBe(ONLY_MODELS);
  expect(arbeskPasteRefusal(COLLECTION, [folder], false, new Set(["/Users/me/other.glb"]))).toBeNull();
});

test("a .glb is listed as a .gltf once it is in Arbesk", () => {
  expect(arbeskTargetName("Box.GLB")).toBe("Box.gltf");
  expect(arbeskTargetName("Box.glb")).toBe("Box.gltf");
  expect(arbeskTargetName("a.b.glb")).toBe("a.b.gltf");
  expect(arbeskTargetName("Box.gltf")).toBe("Box.gltf");
  expect(arbeskTargetName("Box.3mf")).toBe("Box.3mf");
  // Arbesk lists the extension in lower case; the stem keeps its case.
  expect(arbeskTargetName("Box.GLTF")).toBe("Box.gltf");
  expect(arbeskTargetName("Box.3MF")).toBe("Box.3mf");
  expect(arbeskTargetName("MyBox.3Mf")).toBe("MyBox.3mf");
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
