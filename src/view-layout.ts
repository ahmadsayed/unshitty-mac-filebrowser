// ---------------------------------------------------------------------------
// Layout maths of the virtualized views. Items sit in fixed-size slots
// computed here, never measured, so a folder of 10,000 items lays out and
// scrolls like a small one. A view is one or more sections; each starts on a
// new row, below its header if it has one (search: "In This Folder", "In
// Subfolders"). Outside search there is one section without a header.
//
// Every y here is in the items layer: 0 is its top (`bodyTop` below the top
// of the scrolled content).
// ---------------------------------------------------------------------------

export interface Section {
  /** Index in `shown` of the section's first item. */
  start: number;
  header: boolean;
}

export interface ViewLayout {
  cols: number;
  itemW: number;
  itemH: number;
  gap: number;
  padX: number;
  padY: number;
  /** Where the items layer starts within the scrolled content. */
  bodyTop: number;
  count: number;
  /** In order, starts ascending, the first at 0. A section may be empty. */
  sections: Section[];
  headerH: number;
}

/** Where a section sits: its items `[start, end)` and the top of its rows. */
interface Block {
  start: number;
  end: number;
  /** Top of the header, or of the first row when there is none. */
  top: number;
  rowsTop: number;
  rows: number;
}

function blocks(l: ViewLayout): Block[] {
  const pitch = l.itemH + l.gap;
  const out: Block[] = [];
  let top = l.padY;
  l.sections.forEach((s, k) => {
    const end = k + 1 < l.sections.length ? l.sections[k + 1].start : l.count;
    const rows = Math.ceil((end - s.start) / l.cols);
    const rowsTop = top + (s.header ? l.headerH : 0);
    out.push({ start: s.start, end, top, rowsTop, rows });
    top = rowsTop + rows * pitch;
  });
  return out;
}

/** The block holding item `i` (the last one for an index past the end). */
function blockOf(all: Block[], i: number): Block {
  return all.find((b) => i < b.end) ?? all[all.length - 1];
}

export function itemPos(l: ViewLayout, i: number): { x: number; y: number } {
  const b = blockOf(blocks(l), i);
  const offset = i - b.start;
  const row = Math.floor(offset / l.cols);
  const col = offset % l.cols;
  return { x: l.padX + col * (l.itemW + l.gap), y: b.rowsTop + row * (l.itemH + l.gap) };
}

export function headerTop(l: ViewLayout, section: number): number {
  return blocks(l)[section].top;
}

/** Top of what to show for item `i`: its section's header when the item is
 * in the section's first row, so scrolling up to it reveals the header. */
export function revealTop(l: ViewLayout, i: number): number {
  const b = blockOf(blocks(l), i);
  return i - b.start < l.cols ? b.top : itemPos(l, i).y;
}

export function contentHeight(l: ViewLayout): number {
  const all = blocks(l);
  const last = all[all.length - 1];
  if (!last) return l.padY * 2;
  const end = last.rowsTop + last.rows * (l.itemH + l.gap);
  // The gap after the last row isn't part of the content.
  return (last.rows > 0 ? end - l.gap : end) + l.padY;
}

/** The rows of block `b` that [top, bottom] touches, or null. */
function rowSpan(l: ViewLayout, b: Block, top: number, bottom: number): [number, number] | null {
  const pitch = l.itemH + l.gap;
  const first = Math.max(0, Math.floor((top - b.rowsTop) / pitch));
  const last = Math.min(b.rows - 1, Math.floor((bottom - b.rowsTop) / pitch));
  return first <= last ? [first, last] : null;
}

/** Items whose row [top, bottom] touches, as an inclusive range; `end` is
 * below `start` when there are none. */
export function indicesInRows(l: ViewLayout, top: number, bottom: number): { start: number; end: number } {
  let start = l.count;
  let end = -1;
  for (const b of blocks(l)) {
    const span = rowSpan(l, b, top, bottom);
    if (!span) continue;
    start = Math.min(start, b.start + span[0] * l.cols);
    end = Math.max(end, Math.min(b.end - 1, b.start + span[1] * l.cols + l.cols - 1));
  }
  return start <= end ? { start, end } : { start: 0, end: -1 };
}

/** Items a rubber band covers. A band only over a gap or a header has none. */
export function indicesInRect(l: ViewLayout, left: number, top: number, right: number, bottom: number): number[] {
  const pitchX = l.itemW + l.gap;
  const pitchY = l.itemH + l.gap;
  const firstCol = Math.max(0, Math.floor((left - l.padX) / pitchX));
  const lastCol = Math.min(l.cols - 1, Math.floor((right - l.padX) / pitchX));
  const out: number[] = [];
  for (const b of blocks(l)) {
    const span = rowSpan(l, b, top, bottom);
    if (!span) continue;
    for (let row = span[0]; row <= span[1]; row++) {
      // Skip a row when the band only covers the gap below it.
      if (b.rowsTop + row * pitchY + l.itemH < top) continue;
      for (let col = firstCol; col <= lastCol; col++) {
        if (left - l.padX > col * pitchX + l.itemW) continue;
        const i = b.start + row * l.cols + col;
        if (i < b.end) out.push(i);
      }
    }
  }
  return out;
}

/** The item a row down (`dir` 1) or up (-1) from `i`, crossing into the next
 * or previous section in the same column, clamped to that row's length. Past
 * the first or last row it goes to the first or last item, as before. */
export function stepRow(l: ViewLayout, i: number, dir: 1 | -1): number {
  const all = blocks(l).filter((b) => b.end > b.start);
  const k = all.findIndex((b) => i < b.end);
  const b = all[k];
  if (!b) return i; // no items
  const offset = i - b.start;
  const row = Math.floor(offset / l.cols);
  const col = offset % l.cols;
  if (row + dir >= 0 && row + dir < b.rows) {
    return Math.min(b.start + (row + dir) * l.cols + col, b.end - 1);
  }
  const next = all[k + dir];
  if (!next) return dir > 0 ? l.count - 1 : 0;
  const rowStart = dir > 0 ? next.start : next.start + (next.rows - 1) * l.cols;
  return Math.min(rowStart + col, next.end - 1);
}
