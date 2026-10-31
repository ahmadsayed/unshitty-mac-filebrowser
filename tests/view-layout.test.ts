// The view's layout maths: one headerless section must give exactly the old
// formulas; search sections add a header above each section, which starts on
// a new row.
import { expect, test } from "vitest";
import { contentHeight, headerTop, indicesInRect, indicesInRows, itemPos, revealTop, stepRow, type ViewLayout } from "../src/view-layout";

const grid = { cols: 4, itemW: 100, itemH: 120, gap: 6, padX: 12, padY: 12, bodyTop: 0 };
const pitch = 126;

/** Ten items, no sections: a folder outside search. */
const plain: ViewLayout = { ...grid, count: 10, sections: [{ start: 0, header: false }], headerH: 0 };

/** "In This Folder" with 3 items, "In Subfolders" with 5. */
const two: ViewLayout = { ...grid, count: 8, sections: [{ start: 0, header: true }, { start: 3, header: true }], headerH: 30 };

test("without sections, positions and height are the old grid formulas", () => {
  expect(itemPos(plain, 0)).toEqual({ x: 12, y: 12 });
  expect(itemPos(plain, 5)).toEqual({ x: 12 + 106, y: 12 + pitch });
  expect(contentHeight(plain)).toBe(12 * 2 + 3 * pitch - 6);
  expect(revealTop(plain, 5)).toBe(12 + pitch);
});

test("without sections, the drawn range is the rows the viewport touches", () => {
  expect(indicesInRows(plain, 0, 130)).toEqual({ start: 0, end: 3 });
  expect(indicesInRows(plain, 0, 300)).toEqual({ start: 0, end: 9 });
  expect(indicesInRows(plain, 140, 200)).toEqual({ start: 4, end: 7 });
  // Below the last row: nothing.
  const none = indicesInRows(plain, 2000, 2400);
  expect(none.end).toBeLessThan(none.start);
});

test("without sections, Up and Down move a row and clamp at the ends, as before", () => {
  expect(stepRow(plain, 1, 1)).toBe(5);
  expect(stepRow(plain, 5, -1)).toBe(1);
  expect(stepRow(plain, 7, 1)).toBe(9); // the last row is short
  expect(stepRow(plain, 9, 1)).toBe(9);
  expect(stepRow(plain, 2, -1)).toBe(0);
});

test("each section starts on a new row below its header", () => {
  expect(headerTop(two, 0)).toBe(12);
  expect(itemPos(two, 0)).toEqual({ x: 12, y: 12 + 30 });
  expect(headerTop(two, 1)).toBe(12 + 30 + pitch);
  expect(itemPos(two, 3)).toEqual({ x: 12, y: 12 + 30 + pitch + 30 });
  expect(itemPos(two, 7)).toEqual({ x: 12, y: 12 + 30 + pitch + 30 + pitch });
  expect(contentHeight(two)).toBe(12 + 30 + pitch + 30 + 2 * pitch - 6 + 12);
});

test("revealing the first row of a section shows its header", () => {
  expect(revealTop(two, 4)).toBe(headerTop(two, 1));
  expect(revealTop(two, 0)).toBe(headerTop(two, 0));
  expect(revealTop(two, 7)).toBe(itemPos(two, 7).y);
});

test("an empty section still has its header's height", () => {
  const waiting: ViewLayout = { ...two, count: 3 };
  expect(contentHeight(waiting)).toBe(headerTop(waiting, 1) + 30 + 12);
});

test("Up and Down cross sections, keeping the column where the row allows", () => {
  expect(stepRow(two, 2, 1)).toBe(5);
  expect(stepRow(two, 6, -1)).toBe(2); // "In This Folder" ends in a short row
  expect(stepRow(two, 3, -1)).toBe(0);
  expect(stepRow(two, 7, -1)).toBe(3);
  expect(stepRow(two, 4, 1)).toBe(7);
});

test("the drawn range and the rubber band span both sections, never a header", () => {
  expect(indicesInRows(two, 0, 250)).toEqual({ start: 0, end: 6 });
  expect(indicesInRect(two, 0, 50, 500, 210)).toEqual([0, 1, 2, 3, 4, 5, 6]);
  // Only the second header's band.
  const header = headerTop(two, 1);
  expect(indicesInRect(two, 0, header + 2, 500, header + 28)).toEqual([]);
  // A narrow band in the second column.
  expect(indicesInRect(two, 130, 50, 140, 400)).toEqual([1, 4]);
});

test("an empty view has no items to step to and only its padding", () => {
  const empty: ViewLayout = { ...grid, count: 0, sections: [{ start: 0, header: false }], headerH: 0 };
  expect(stepRow(empty, 0, 1)).toBe(0);
  expect(contentHeight(empty)).toBe(24);
  expect(indicesInRect(empty, 0, 0, 500, 500)).toEqual([]);
});
