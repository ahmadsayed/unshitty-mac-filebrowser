// PathRefs on its own: each holder's rule for moved and trashed items, and
// one replacement (so one save) per holder per report.
import { expect, test } from "vitest";
import { followPath, PathRefs, type Clip, type Slot } from "../src/path-refs";

/** A holder that counts how often it was replaced. */
function slot<T>(value: T): Slot<T> & { value: T; sets: number } {
  const held = {
    value,
    sets: 0,
    get: () => held.value,
    set: (next: T) => {
      held.value = next;
      held.sets++;
    },
  };
  return held;
}

function setup(opts: { stars?: string[]; bookmarks?: string[]; clipboard?: Clip | null } = {}) {
  const stars = slot(new Set(opts.stars ?? []));
  const bookmarks = slot(opts.bookmarks ?? []);
  const clipboard = slot<Clip | null>(opts.clipboard ?? null);
  const refs = new PathRefs({ stars, bookmarks, clipboard });
  return { refs, stars, bookmarks, clipboard };
}

const clip = (paths: string[], mode: Clip["mode"] = "cut", dirs: string[] = []): Clip => ({ paths, dirs: new Set(dirs), mode });

// ---- followPath -------------------------------------------------------------

test("a moved item and what is inside it take the new path; a sibling with the same start doesn't", () => {
  const moves = [{ from: "/home/me/A", to: "/home/me/Dest/A" }];
  expect(followPath("/home/me/A", moves)).toBe("/home/me/Dest/A");
  expect(followPath("/home/me/A/b/c.txt", moves)).toBe("/home/me/Dest/A/b/c.txt");
  expect(followPath("/home/me/AB", moves)).toBe("/home/me/AB");
  expect(followPath("/home/me", moves)).toBe("/home/me");
});

test("moves are taken in order, so a path follows a chain of them", () => {
  const moves = [
    { from: "/d/a", to: "/d/b" },
    { from: "/d/b", to: "/e/b" },
  ];
  expect(followPath("/d/a/x", moves)).toBe("/e/b/x");
  expect(followPath("/d/c", moves)).toBe("/d/c");
});

test("a Drive item inside a renamed Drive folder keeps its own segment", () => {
  const moves = [{ from: "gdrive://me/1~Old", to: "gdrive://me/1~New" }];
  expect(followPath("gdrive://me/1~Old/2~file.txt", moves)).toBe("gdrive://me/1~New/2~file.txt");
  expect(followPath("gdrive://me/1~Older", moves)).toBe("gdrive://me/1~Older");
});

// ---- Stars ----------------------------------------------------------------------

test("stars follow a batch, replaced and so saved once", () => {
  const { refs, stars } = setup({ stars: ["/d/a.txt", "/d/F", "/d/F/in.txt", "/d/other.txt"] });
  refs.moved([
    { from: "/d/a.txt", to: "/e/a.txt" },
    { from: "/d/F", to: "/e/F" },
  ]);
  expect(stars.sets).toBe(1);
  expect([...stars.value].sort()).toEqual(["/d/other.txt", "/e/F", "/e/F/in.txt", "/e/a.txt"]);
});

test("stars that no move touches aren't replaced or saved", () => {
  const { refs, stars } = setup({ stars: ["/d/a.txt"] });
  refs.moved([{ from: "/d/b.txt", to: "/d/c.txt" }]);
  refs.moved([]);
  expect(stars.sets).toBe(0);
});

test("a trashed item keeps its star", () => {
  const { refs, stars } = setup({ stars: ["/d/a.txt"] });
  refs.gone(["/d/a.txt"]);
  expect(stars.sets).toBe(0);
  expect([...stars.value]).toEqual(["/d/a.txt"]);
});

// ---- Bookmarks -------------------------------------------------------------------

test("bookmarks follow a batch in their own order, replaced and so saved once", () => {
  const { refs, bookmarks } = setup({ bookmarks: ["/d/Projects", "/d/F/In", "/d/Other"] });
  refs.moved([
    { from: "/d/F", to: "/e/F" },
    { from: "/d/Projects", to: "/d/Projects 2" },
  ]);
  expect(bookmarks.sets).toBe(1);
  expect(bookmarks.value).toEqual(["/d/Projects 2", "/e/F/In", "/d/Other"]);
});

test("two bookmarks that end at one path become one", () => {
  // Replace: the moved folder takes the place of another bookmarked one.
  const { refs, bookmarks } = setup({ bookmarks: ["/d/Old", "/d/New"] });
  refs.moved([{ from: "/d/New", to: "/d/Old" }]);
  expect(bookmarks.value).toEqual(["/d/Old"]);
});

test("bookmarks no move touches aren't saved, and a trashed folder keeps its bookmark", () => {
  const { refs, bookmarks } = setup({ bookmarks: ["/d/Projects"] });
  refs.moved([{ from: "/d/Project", to: "/d/Plan" }]);
  refs.gone(["/d/Projects"]);
  expect(bookmarks.sets).toBe(0);
  expect(bookmarks.value).toEqual(["/d/Projects"]);
});

// ---- The clipboard ----------------------------------------------------------------

test("the clipboard follows a move, paths and folders alike, and stays the same cut", () => {
  const cut = clip(["/d/a.txt", "/d/F", "/d/b.txt"], "cut", ["/d/F"]);
  const { refs, clipboard } = setup({ clipboard: cut });
  refs.moved([
    { from: "/d/a.txt", to: "/d/a2.txt" },
    { from: "/d/F", to: "/e/F" },
  ]);
  expect(clipboard.sets).toBe(1);
  expect(clipboard.value).toBe(cut);
  expect(cut.paths).toEqual(["/d/a2.txt", "/e/F", "/d/b.txt"]);
  expect([...cut.dirs]).toEqual(["/e/F"]);
  expect(cut.mode).toBe("cut");
});

test("a copied item inside a moved folder follows it", () => {
  const copy = clip(["/d/F/x.txt"], "copy");
  const { refs } = setup({ clipboard: copy });
  refs.moved([{ from: "/d/F", to: "/d/G" }]);
  expect(copy.paths).toEqual(["/d/G/x.txt"]);
});

test("a move that touches nothing on the clipboard leaves it alone", () => {
  const copy = clip(["/d/a.txt"], "copy");
  const { refs, clipboard } = setup({ clipboard: copy });
  refs.moved([{ from: "/d/b.txt", to: "/d/c.txt" }]);
  expect(clipboard.sets).toBe(0);
  expect(copy.paths).toEqual(["/d/a.txt"]);
});

test("trashed items leave the clipboard, which stays the same cut", () => {
  const cut = clip(["/d/a.txt", "/d/b.txt"]);
  const { refs, clipboard } = setup({ clipboard: cut });
  refs.gone(["/d/a.txt", "/d/elsewhere.txt"]);
  expect(clipboard.value).toBe(cut);
  expect(cut.paths).toEqual(["/d/b.txt"]);
});

test("the clipboard empties when every item in it went to the Trash", () => {
  const { refs, clipboard } = setup({ clipboard: clip(["/d/a.txt"]) });
  refs.gone(["/d/a.txt"]);
  expect(clipboard.value).toBeNull();
});

test("nothing on the clipboard: a trash leaves it empty", () => {
  const { refs, clipboard } = setup();
  refs.gone(["/d/a.txt"]);
  expect(clipboard.value).toBeNull();
  expect(clipboard.sets).toBe(0);
});
