// What each kind of location allows (src/locations.ts). One row per kind of
// place, one column per question the app asks, so a new kind of location or a
// changed rule shows up here as a single cell.
import { expect, test } from "vitest";
import {
  acceptsNewItems,
  isInTrash,
  isLocalLocation,
  isPseudo,
  isReadOnlyLocation,
  mayHaveVersions,
  searchesRecursively,
} from "../src/locations";

const TRASH = "/Users/me/.Trash";

type Row = {
  path: string;
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
  "a local folder":          { path: "/Users/me/Documents",              pseudo: F, readOnly: F, local: T, inTrash: F, searches: T, versions: T, newItems: T },
  "the root":                { path: "/",                                pseudo: F, readOnly: F, local: T, inTrash: F, searches: T, versions: T, newItems: T },
  "the Trash":               { path: TRASH,                              pseudo: F, readOnly: F, local: T, inTrash: T, searches: F, versions: F, newItems: T },
  // Only the Trash itself is left out of the versions lookup.
  "a folder in the Trash":   { path: `${TRASH}/old`,                     pseudo: F, readOnly: F, local: T, inTrash: T, searches: F, versions: T, newItems: T },
  "a name that starts like the Trash": { path: `${TRASH}can`,            pseudo: F, readOnly: F, local: T, inTrash: F, searches: T, versions: T, newItems: T },
  "Recent":                  { path: "pseudo:recent",                    pseudo: T, readOnly: F, local: F, inTrash: F, searches: F, versions: F, newItems: F },
  "Network":                 { path: "pseudo:network",                   pseudo: T, readOnly: F, local: F, inTrash: F, searches: F, versions: F, newItems: F },
  "a Google Drive folder":   { path: "gdrive://acct/root~My%20Drive",    pseudo: F, readOnly: F, local: F, inTrash: F, searches: F, versions: F, newItems: T },
  "a received share":        { path: "share://r1/sub",                   pseudo: F, readOnly: T, local: F, inTrash: F, searches: F, versions: F, newItems: F },
  "an SSH server":           { path: "sftp://me@host/home",              pseudo: F, readOnly: T, local: F, inTrash: F, searches: F, versions: F, newItems: F },
};

for (const [name, row] of Object.entries(ROWS)) {
  test(`${name}: what it allows`, () => {
    expect({
      pseudo: isPseudo(row.path),
      readOnly: isReadOnlyLocation(row.path),
      local: isLocalLocation(row.path),
      inTrash: isInTrash(row.path, TRASH),
      searches: searchesRecursively(row.path, TRASH),
      versions: mayHaveVersions(row.path, TRASH),
      newItems: acceptsNewItems(row.path),
    }).toEqual({
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
});
