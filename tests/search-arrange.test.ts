// What a search shows and in which sections (src/search.ts), without the app.
import { expect, test } from "vitest";
import type { Entry } from "../src/api";
import { arrangeSearch, searchSections } from "../src/search";

const entry = (path: string, is_dir = false): Entry =>
  ({ path, name: path.split("/").pop()!, is_dir, size: 0, modified: 0, is_hidden: false, is_symlink: false }) as Entry;
const byName = (a: Entry, b: Entry) => a.name.localeCompare(b.name);
const paths = (items: Entry[]) => items.map((e) => e.path);

const AT = "/home/me/Notes";
const listing = [entry(`${AT}/b-notes.txt`), entry(`${AT}/Café menu.pdf`), entry(`${AT}/a-notes.md`), entry(`${AT}/todo.txt`)];
const walked = [
  entry(`${AT}/a-notes.md`), // in `at` itself: already shown from the listing
  entry(`${AT}/Work/2023/notes-2023.txt`),
  entry(`${AT}/Work/z-notes.txt`),
  entry(`${AT}/Archive/c-notes.zip`),
];

test("no search: the whole listing, sorted, without sections", () => {
  for (const query of ["", "   "]) {
    const { items, sectionStart } = arrangeSearch(listing, null, query, AT, byName, true);
    expect(paths(items)).toEqual([`${AT}/a-notes.md`, `${AT}/b-notes.txt`, `${AT}/Café menu.pdf`, `${AT}/todo.txt`]);
    expect(sectionStart).toBe(-1);
  }
});

test("the listing is never sorted in place", () => {
  const copy = [...listing];
  arrangeSearch(listing, null, "", AT, byName, true);
  arrangeSearch(listing, walked, "notes", AT, byName, true);
  expect(listing).toEqual(copy);
});

test("a place that filters in place: matches by folded, trimmed name, no sections, results ignored", () => {
  const { items, sectionStart } = arrangeSearch(listing, walked, "  CAFE ", AT, byName, false);
  expect(paths(items)).toEqual([`${AT}/Café menu.pdf`]);
  expect(sectionStart).toBe(-1);
});

test("a recursive search: this folder's matches, then subfolders' nearest first, each sorted", () => {
  const { items, sectionStart } = arrangeSearch(listing, walked, "Notes", AT, byName, true);
  expect(paths(items)).toEqual([
    `${AT}/a-notes.md`,
    `${AT}/b-notes.txt`,
    `${AT}/Archive/c-notes.zip`,
    `${AT}/Work/z-notes.txt`,
    `${AT}/Work/2023/notes-2023.txt`,
  ]);
  expect(sectionStart).toBe(2);
});

test("before the walk answers, only this folder's matches", () => {
  const { items, sectionStart } = arrangeSearch(listing, null, "notes", AT, byName, true);
  expect(paths(items)).toEqual([`${AT}/a-notes.md`, `${AT}/b-notes.txt`]);
  expect(sectionStart).toBe(2);
  expect(arrangeSearch(listing, null, "zzz", AT, byName, true)).toEqual({ items: [], sectionStart: 0 });
});

test("from the root of the disk, a child of / is in this folder", () => {
  const root = [entry("/Users", true), entry("/usr", true)];
  const found = [entry("/Users", true), entry("/Users/me/users.txt")];
  const { items, sectionStart } = arrangeSearch(root, found, "users", "/", byName, true);
  expect(paths(items)).toEqual(["/Users", "/Users/me/users.txt"]);
  expect(sectionStart).toBe(1);
});

test("sections: none outside a recursive search", () => {
  expect(searchSections(-1, 5, true)).toEqual([]);
});

test("sections: both parts with their counts; only the subfolders' says it is searching", () => {
  expect(searchSections(2, 5, true)).toEqual([
    { label: "In This Folder", start: 0, count: 2, busy: false },
    { label: "In Subfolders", start: 2, count: 3, busy: true },
  ]);
  expect(searchSections(2, 5, false)[1]).toEqual({ label: "In Subfolders", start: 2, count: 3, busy: false });
});

test("sections: an empty subfolder part shows only while searching, and only under matches here", () => {
  expect(searchSections(2, 2, true)).toEqual([
    { label: "In This Folder", start: 0, count: 2, busy: false },
    { label: "In Subfolders", start: 2, count: 0, busy: true },
  ]);
  expect(searchSections(2, 2, false)).toEqual([{ label: "In This Folder", start: 0, count: 2, busy: false }]);
  expect(searchSections(0, 0, true)).toEqual([]);
  expect(searchSections(0, 3, true)).toEqual([{ label: "In Subfolders", start: 0, count: 3, busy: true }]);
});
