// The view's selection, cursor and anchor (src/selection.ts), without the DOM.
// The user's changes count as intent (a paste then leaves their newer choice
// alone, #50); the app's own changes never do.
import { beforeEach, expect, test } from "vitest";
import { Selection } from "../src/selection";

type Item = { path: string };
const items = (...names: string[]): Item[] => names.map((n) => ({ path: `/d/${n}` }));

let shown: Item[];
let sel: Selection<Item>;

beforeEach(() => {
  shown = items("a", "b", "c", "d", "e");
  sel = new Selection(() => shown);
});

const picked = () => sel.selected().map((item) => item.path.slice(3)).join("");

/** Runs `change` and says whether it counted as the user's choice. */
function bumps(change: () => void): boolean {
  const before = sel.intent;
  change();
  return sel.intent !== before;
}

// ---- The user's changes -----------------------------------------------------

test("starts empty, with no cursor or anchor", () => {
  expect(picked()).toBe("");
  expect([sel.size, sel.anchor, sel.cursor]).toEqual([0, -1, -1]);
});

test("only selects one item and puts the cursor and anchor on it", () => {
  sel.range(0, 1, false);
  expect(bumps(() => sel.only(3))).toBe(true);
  expect(picked()).toBe("d");
  expect([sel.anchor, sel.cursor]).toEqual([3, 3]);
});

test("only past the end selects nothing but still moves the cursor", () => {
  sel.only(9);
  expect(picked()).toBe("");
  expect([sel.anchor, sel.cursor]).toEqual([9, 9]);
});

test("range selects between two items in either direction and moves only the cursor", () => {
  sel.only(3);
  expect(bumps(() => sel.range(3, 1, false))).toBe(true);
  expect(picked()).toBe("bcd");
  expect([sel.anchor, sel.cursor]).toEqual([3, 1]);
  sel.range(1, 2, false);
  expect(picked()).toBe("bc");
});

test("an additive range keeps what was selected", () => {
  sel.only(0);
  sel.range(3, 4, true);
  expect(picked()).toBe("ade");
});

test("all and clear keep the cursor and anchor where they are", () => {
  sel.only(2);
  expect(bumps(() => sel.all())).toBe(true);
  expect(picked()).toBe("abcde");
  expect(bumps(() => sel.clear())).toBe(true);
  expect(picked()).toBe("");
  expect([sel.anchor, sel.cursor]).toEqual([2, 2]);
});

test("toggle flips one item and leaves the cursor alone", () => {
  sel.only(0);
  expect(bumps(() => sel.toggle(2))).toBe(true);
  expect(picked()).toBe("ac");
  sel.toggle(0);
  expect(picked()).toBe("c");
  expect(sel.cursor).toBe(0);
});

test("reset starts a new folder or tab: nothing chosen, no cursor, maybe one item", () => {
  sel.range(0, 2, false);
  expect(bumps(() => sel.reset())).toBe(true);
  expect(picked()).toBe("");
  expect([sel.anchor, sel.cursor]).toEqual([-1, -1]);
  sel.reset("/d/e");
  expect(picked()).toBe("e");
  expect(sel.cursor).toBe(-1);
});

test("noteIntent counts a change the selection can't see, like typing a search", () => {
  expect(bumps(() => sel.noteIntent())).toBe(true);
});

// ---- The app's own changes ----------------------------------------------------

test("the app's own changes never count as the user's choice", () => {
  sel.only(1);
  const mark = sel.mark();
  const changes: (() => void)[] = [
    () => sel.focus(2),
    () => sel.selectFirst(),
    () => sel.dropCursor(),
    () => sel.retain(["/d/a"], shown),
    () => sel.replace(["/d/b"]),
    () => sel.place(1, 1),
    () => sel.follow(mark, true),
  ];
  for (const change of changes) expect(bumps(change)).toBe(false);
});

test("focus moves the cursor and anchor without changing what is selected", () => {
  sel.only(0);
  sel.focus(3);
  expect(picked()).toBe("a");
  expect([sel.anchor, sel.cursor]).toEqual([3, 3]);
});

test("selectFirst picks the first item, and changes nothing when there are none", () => {
  sel.range(2, 3, false);
  sel.selectFirst();
  expect(picked()).toBe("a");
  expect([sel.anchor, sel.cursor]).toEqual([0, 0]);
  shown = [];
  sel.selectFirst();
  expect(sel.size).toBe(1);
});

test("dropCursor forgets the cursor and anchor but keeps the selection", () => {
  sel.only(2);
  sel.dropCursor();
  expect(picked()).toBe("c");
  expect([sel.anchor, sel.cursor]).toEqual([-1, -1]);
});

test("retain keeps the chosen paths that are still listed", () => {
  sel.all();
  sel.retain(["/d/a", "/d/gone", "/d/c"], items("a", "b", "c"));
  expect([...sel.paths()].sort()).toEqual(["/d/a", "/d/c"]);
});

test("replace sets the selection outright, as the rubber band does", () => {
  sel.only(0);
  sel.replace(["/d/b", "/d/d"]);
  expect(picked()).toBe("bd");
});

test("place puts back a tab's cursor and anchor, kept inside the list", () => {
  sel.place(1, 9);
  expect([sel.anchor, sel.cursor]).toEqual([1, 4]);
  shown = [];
  sel.place(1, 2);
  expect([sel.anchor, sel.cursor]).toEqual([-1, -1]);
});

// ---- Keeping the cursor on its item -------------------------------------------

test("follow keeps the cursor and anchor on the same items when new ones land above", () => {
  sel.only(1);
  sel.range(1, 3, false);
  const mark = sel.mark();
  shown = items("new", "a", "b", "c", "d", "e");
  sel.follow(mark, true);
  expect([sel.anchor, sel.cursor]).toEqual([2, 4]);
});

test("follow near: a vanished item leaves the cursor at its old place, kept in range", () => {
  sel.only(4);
  const mark = sel.mark();
  shown = items("a", "b");
  sel.follow(mark, true);
  expect([sel.anchor, sel.cursor]).toEqual([1, 1]);
});

test("follow exact: a vanished item leaves no cursor", () => {
  sel.only(4);
  const mark = sel.mark();
  shown = items("a", "b");
  sel.follow(mark, false);
  expect([sel.anchor, sel.cursor]).toEqual([-1, -1]);
});

test("selected lists the selected items in the order shown, from the current list", () => {
  sel.replace(["/d/d", "/d/a"]);
  expect(picked()).toBe("ad");
  shown = items("d", "c", "b", "a");
  expect(picked()).toBe("da");
  expect(sel.has("/d/a")).toBe(true);
  expect(sel.indexOf("/d/c")).toBe(1);
  expect(sel.indexOf("/d/zz")).toBe(-1);
});
