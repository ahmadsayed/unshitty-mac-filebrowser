# Search: this folder first, then subfolders

**Date:** 2026-10-31

**Status:** Design approved in conversation; not implemented yet.

**Amends:** [File browser](2026-10-23-file-browser-design.md), search.

## Goal

Searching a folder answers "is it here?" first. Today the matches directly
in the current folder are mixed into one list with matches from every depth,
sorted together by name, so `notes.txt` sits between
`Work/2023/notes-old.txt` and `Archive/notes.zip`. The walk is also
depth-first, so when it stops at its limits a deep folder may have used them
up before a nearby one was looked at.

After this change a search shows two sections:

```
 In This Folder  3
 [notes.txt] [Meeting notes.md] [notes 2024.pdf]
 ─────────────────────────────────────────────
 In Subfolders  5                 ⟳ Searching…
 [Notes]  [notes-old.txt]  [release-notes.md] …
  Work     Work/2023        Projects/alpha
```

Out of scope: searching outside the current folder (home, other drives, the
whole disk, Spotlight or localsearch indexes), and matching file contents.

## Decisions

**D1. Two sections, only for recursive searches.** A search in a local
folder shows **In This Folder** (matches whose parent is the current folder)
and **In Subfolders** (every deeper match). Places that filter in place
(Recent, Starred, Trash, Google Drive, SFTP, received shares) keep today's
single list with no headers. Outside search there are no sections.

**D2. "In This Folder" shows at once.** Its matches come from the folder's
listing already in memory (today's quick local matches), so nothing waits on
the backend. Backend results whose parent is the current folder are dropped
as duplicates. The section is sorted by the view's sort order.

**D3. "In Subfolders" fills in as the search runs.** Its header shows the
count and, while the search runs, a spinner and "Searching…". Each result
shows the folder it is in, as today (`whereText`).

**D4. Nearest first.** "In Subfolders" is ordered by depth below the current
folder (one level down first), then by the view's sort order within the same
depth. Because the backend now walks level by level (D7), most new results
belong at the end, so the list grows at the bottom instead of reshuffling.

**D5. Empty sections.**

| This folder | Subfolders | Shown |
|---|---|---|
| matches | searching | both headers; "In Subfolders" has no items yet |
| matches | done, none | "In This Folder" only; the second header goes |
| none | matches | "In Subfolders" only |
| none | searching, none yet | "Searching…" status page, as today |
| none | done, none | "No Results Found", as today |

A header is shown only when its section is shown, so a single-section result
has one header (it still says where the matches are).

**D6. Selection.** The first match is selected while search runs, as in
GNOME Files: the first of "In This Folder", or of "In Subfolders" when the
first section is empty. Results arriving later never move "In This Folder",
so the selection and cursor stay put; the cursor and anchor follow their
items, as today.

**D7. The backend walks level by level.** `fs_ops::search_dir` uses a queue
instead of a stack: every folder at one depth is read before any folder one
level deeper. The 1,000-result and 100,000-item limits then cut the farthest
folders, and the "Showing the first 1,000 results" / "Search stopped after
looking at …" banner stays as it is. The command's arguments, results and
streaming are unchanged. The mock backend's search walks the same way.

**D8. Sorting.** Clicking a list column header (or the sort menu) sorts
within each section; "In This Folder" always stays above "In Subfolders",
and "In Subfolders" stays grouped by depth (D4).

**D9. Keyboard and mouse.** Left and Right move by one item and cross
sections. Down from the last row of "In This Folder" goes to the first row
of "In Subfolders" in the same column (or its last item when that row is
shorter); Up does the reverse. Home, End, Page Up/Down, Shift-extend and the
rubber band work across both sections. A header is not an item: it can't be
selected, clicked or dropped on, and a press on it starts a rubber band, as
a press between items does.

**D10. Wording.** **In This Folder** and **In Subfolders**, with the count
after the label. GNOME Files 50 has no sectioned search, so there is no
GNOME wording to follow.

## How it works

**Layout maths (`src/view-layout.ts`, new).** The views are virtualized:
items are absolutely positioned from layout maths, and only the visible
range is in the DOM (AGENTS.md). Today `main.ts` repeats `row = i / cols`,
`y = padY + row × pitch` in six places: `itemPos`, the content height in
`relayout`, the drawn range in `renderWindow`, `scrollToIndex`, the rubber
band, and the keyboard's row step and page size. These move into pure
functions over a layout description:

- the grid metrics of today (`cols`, `itemW`, `itemH`, `gap`, `padX`,
  `padY`, `bodyTop`), plus
- the sections: for each, the index of its first item and whether it has a
  header, and the theme's header height.

Each section starts on a new row; a header takes its height above the
section's first row. The functions answer:

- `itemPos(i)` and `headerPos(section)`,
- `contentHeight()`,
- `indicesInRows(top, bottom)` for the drawn range, and
  `indicesInRect(left, top, right, bottom)` for the rubber band,
- `stepRow(i, ±1)` for Up and Down across sections (D9), and the page size.

With one section and no header every answer equals today's formula, so the
views outside search are unchanged. The functions take no DOM and are tested
on their own.

**The list (`computeShown` in `main.ts`).** In a recursive search `shown` is
"In This Folder" (from the in-memory listing, sorted with `compareEntries`)
followed by "In Subfolders" (backend results minus the current folder's own,
sorted by depth, then `compareEntries`), and the index where the second
section starts is kept with it. Everything that indexes `shown` (selection,
cursor, anchor, drag, open) is unchanged.

**Drawing (`renderWindow`).** Section headers are absolutely positioned
elements in the items layer, drawn only when they intersect the visible
range, like items. Their height per theme goes in `LAYOUT_SIZES`
(`src/theme.ts`); their look uses existing tokens (`--window-fg`, `--dim`,
`--border`), so both themes and dark mode need no new colours. In list view
the headers are rows across the list, below the column header.

**Backend (`src-tauri/src/fs_ops.rs`).** `search_dir` swaps its `Vec` stack
for a `VecDeque` queue (D7). Nothing else changes.

## Testing

- **`tests/view-layout.test.ts` (new):** one section without a header gives
  today's positions, height, drawn range and row steps; with two sections
  the second starts on a new row below its header; `indicesInRect` takes
  items on both sides of a header and none for a rectangle over a header
  only; `stepRow` crosses sections and clamps to a short last row.
- **`tests/search-sections.test.ts` (new, app in jsdom):** both headers and
  "In This Folder" first; subfolder results nearest first; each row of the
  D5 table; Down from the first section into the second; a column sort
  sorting within each section; no sections in Recent, Trash or Drive.
- **Backend (`fs_ops.rs` tests):** the walk reads every folder at one depth
  before the next; with a tight limit a shallow match is kept and a deep one
  dropped.
- **Mock (`tests/mock-core.test.ts`):** the mock's search follows the same
  order.
- **Checks:** `tsc`, `npm run build`, `npm test`, `cargo test` and the CRAP
  gate (the rubber band handler and `computeShown` may need splitting).
- **Screenshots** from the mock preview, before (`main`) and after: grid and
  list, GNOME and macOS themes, light and dark. A search in `/big` (10,000
  items) to check typing and scrolling stay fast.
- **Real app:** a search in a real home folder (`npm run tauri dev`). No
  packaging or other programs are involved.

## Docs

README: the search line in the feature list.
