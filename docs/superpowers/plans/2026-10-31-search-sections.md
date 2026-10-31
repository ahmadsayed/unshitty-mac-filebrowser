# Search Sections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A recursive search shows "In This Folder" at once, then "In Subfolders" (nearest first) as the backend finds them.

**Architecture:** The backend walk becomes breadth-first. The view's layout arithmetic moves from `main.ts` into a pure module, `src/view-layout.ts`, that knows about sections and their headers; `main.ts` builds `shown` as two sections and draws their headers.

**Tech Stack:** Rust (`fs_ops.rs`), TypeScript (no framework), Vitest + jsdom, the mock backend in `dev/mock-tauri/`.

**Spec:** `docs/superpowers/specs/2026-10-31-search-sections-design.md`

## Global Constraints

- Labels exactly **In This Folder** and **In Subfolders**, the count after the label; "Searching…" while the search runs (D3, D10).
- Sections only in recursive searches (`canSearchRecursively()` and a non-empty trimmed query); everywhere else one section without a header, identical to today (D1).
- Virtualized views: never render all of `shown`, never measure items with `getBoundingClientRect`, use `scrollToIndex` (AGENTS.md).
- Look-related values are theme tokens; per-theme sizes the views compute go in `LAYOUT_SIZES` (AGENTS.md).
- The search command's arguments and results don't change (D7).

## Review Focus

- Searching at `/` (location without a trailing-slash parent): "In This Folder" must hold `/x`, depth computed from `/`.
- A search that is still running when the user sorts or switches grid/list: headers and counts must survive the re-render.
- A rubber band started on a header and dragged over both sections selects items in both, none for the header.
- Up from the first row of "In Subfolders" when "In This Folder" ends in a short row lands on that row's last item.
- Results arriving while the cursor is in "In This Folder" never move the cursor or the selection.

---

### Task 1: Breadth-first search walk (backend and mock)

**Files:**
- Modify: `src-tauri/src/fs_ops.rs` (`search_dir`, tests module)
- Modify: `dev/mock-tauri/core.ts` (`search`)
- Test: `src-tauri/src/fs_ops.rs` tests, `tests/mock-core.test.ts`

**Interfaces:** Produces nothing new; `search_dir` / `search_dir` mock keep their signatures, only the order changes.

- [ ] **Step 1: Failing Rust tests** `search_reads_each_level_before_the_next` (folders `a/b/c` each holding `note-<depth>.txt`, plus `z/note-1b.txt`; the flattened batches list every depth-1 match before any depth-2 one, depth-2 before depth-3) and `search_limit_keeps_shallow_matches` (`deep/deeper/deepest/note.txt` created first, `note.txt` at the root and `x/note.txt`; with `limit = 2` the result is exactly `note.txt` and `x/note.txt`).
- [ ] **Step 2:** `cd src-tauri && cargo test --lib search_` fails on the order.
- [ ] **Step 3:** In `search_dir` replace the `Vec` stack with `std::collections::VecDeque` (`push_back` / `pop_front`).
- [ ] **Step 4:** `cargo test --lib search_` passes.
- [ ] **Step 5: Mock.** Test in `tests/mock-core.test.ts`: `search_dir` for "notes"-like names in a mock folder with nested matches streams depth-1 results before deeper ones. Make the mock's `search` use a queue (`shift` instead of `pop`). `npx vitest run tests/mock-core.test.ts` passes.
- [ ] **Step 6: Commit** `perf: search nearest folders first`.

### Task 2: `src/view-layout.ts`, the layout maths

**Files:**
- Create: `src/view-layout.ts`
- Test: `tests/view-layout.test.ts`

**Interfaces (produces):**

```ts
export interface Section { start: number; header: boolean }
export interface ViewLayout {
  cols: number; itemW: number; itemH: number; gap: number;
  padX: number; padY: number;
  /** Where the items layer starts within the scrolled content. */
  bodyTop: number;
  count: number;
  /** In order, starts ascending, first start 0. A section may be empty. */
  sections: Section[];
  headerH: number;
}
export function itemPos(l: ViewLayout, i: number): { x: number; y: number };
export function headerTop(l: ViewLayout, section: number): number;
/** Top of what to reveal for item i: its section's header when i is in the section's first row. */
export function revealTop(l: ViewLayout, i: number): number;
export function contentHeight(l: ViewLayout): number;
/** Items whose row intersects [top, bottom] (items-layer y), as an inclusive range; end < start when none. */
export function indicesInRows(l: ViewLayout, top: number, bottom: number): { start: number; end: number };
export function indicesInRect(l: ViewLayout, left: number, top: number, right: number, bottom: number): number[];
/** The item one row down (dir 1) or up (-1), crossing sections, same column clamped to the row; i itself at the ends. */
export function stepRow(l: ViewLayout, i: number, dir: 1 | -1): number;
```

Geometry: each section is a block starting at `blockTop` (first block at `padY`): a header of `headerH` when `header`, then `ceil(len / cols)` rows of pitch `itemH + gap`; the next block starts where this one ends. `contentHeight = lastBlockEnd - gap + padY` when there are rows, so one headerless section gives today's `padY * 2 + rows * pitch - gap`.

- [ ] **Step 1: Failing tests** in `tests/view-layout.test.ts`:
  - one headerless section, `cols 4, itemW 100, itemH 120, gap 6, padX 12, padY 12, count 10`: `itemPos(l, 5)` is `{ x: 12 + 106, y: 12 + 126 }`; `contentHeight` is `12 * 2 + 3 * 126 - 6`; `indicesInRows(l, 0, 130)` is `{ start: 0, end: 7 }`; `stepRow(l, 1, 1)` is 5, `stepRow(l, 9, 1)` is 9, `stepRow(l, 2, -1)` is 2.
  - two sections `[{start 0, header}, {start 3, header}]`, `cols 4, count 8, headerH 30`: `itemPos(l, 0).y` is `12 + 30`; `itemPos(l, 3)` is `{ x: 12, y: 12 + 30 + 126 + 30 }`; `headerTop(l, 1)` is `12 + 30 + 126`; `revealTop(l, 4)` equals `headerTop(l, 1)`; `stepRow(l, 2, 1)` is 5; `stepRow(l, 6, -1)` is 2 (short row clamp); `stepRow(l, 3, -1)` is 2.
  - `indicesInRect` over both sections returns indices from both; a rect covering only header 1's band returns `[]`.
  - an empty second section with a header adds `headerH` to `contentHeight`.
- [ ] **Step 2:** `npx vitest run tests/view-layout.test.ts` fails (module missing).
- [ ] **Step 3:** Implement the module as above.
- [ ] **Step 4:** The tests pass; `npx tsc --noEmit` clean.
- [ ] **Step 5: Commit** `refactor: view layout maths in one module`.

### Task 3: `main.ts` uses `view-layout.ts` (no sections yet)

**Files:** Modify `src/main.ts` (`Layout`, `computeLayout`, `itemPos`, `relayout`, `renderWindow`, `scrollToIndex`, rubber band in the mousedown handler, `onViewKeyDown` / `VIEW_KEYS` Up and Down).

**Interfaces:** Consumes Task 2. `layout` becomes a `ViewLayout`; `computeLayout` fills `count: shown.length`, `sections: [{ start: 0, header: false }]`, `headerH: 0` for now.

- [ ] **Step 1:** Replace the six inline formulas with calls: `itemPos` → `itemPos(layout, i)`; `relayout` height → `contentHeight(layout) + (list ? 6 : 0)`; `renderWindow` → `indicesInRows` with the overscan rows converted to pixels (`overscan * (itemH + gap)`), and the visible range for thumbnails from `indicesInRows` without overscan; `scrollToIndex` uses `revealTop` for the upper bound; the rubber band → `indicesInRect` in items-layer coordinates (`y - bodyTop`); ArrowDown/ArrowUp → `moveCursor(stepRow(layout, cur, ±1), shift)` (cursor -1 still goes to 0).
- [ ] **Step 2:** `npm test` passes unchanged (the existing keyboard, rubber band and huge-folder tests are the guard); `npx tsc --noEmit` clean.
- [ ] **Step 3: Commit** `refactor: views take their positions from view-layout`.

### Task 4: Two sections in search

**Files:**
- Modify: `src/main.ts` (`computeShown`, `computeLayout`, `renderWindow`, `itemHtml`, `addSearchResults`)
- Modify: `src/theme.ts` (`LayoutSizes.sectionHeader`: gnome 36, macos 30)
- Modify: `src/styles.css` (`.section-header`)
- Test: `tests/search-sections.test.ts` (new)

**Interfaces:** Consumes Tasks 2–3. New module state in `main.ts`: `let sectionStart = -1` (index of the first "In Subfolders" item, -1 outside sectioned search) and `function searchSections(): { label: string; start: number; count: number; busy: boolean }[]`, which the layout and the header drawing both read.

- [ ] **Step 1: Failing tests** (Linux user agent not needed; mock home `/Users/me`; build fixtures with the mock's `make_dir`/files under a fresh folder, e.g. `Documents/notes.txt`, `Documents/Work/notes-old.txt`, `Documents/Work/2023/notes-2023.txt`, `Documents/a-notes.md`):
  - `In This Folder` header first with count 2, its items `a-notes.md`, `notes.txt`; then `In Subfolders` with `notes-old.txt` before `notes-2023.txt`.
  - D5 rows: only subfolder matches → one header "In Subfolders" and its first item selected; only local matches → after the search ends, one header; none → "No Results Found".
  - Down from the last "In This Folder" item selects the first "In Subfolders" item.
  - Sorting by name reversed (sort menu or list header) reverses within each section; "In This Folder" stays first.
  - In Recent and Trash a search shows no `.section-header`.
  - The selected local item stays selected after the subfolder results arrive.
- [ ] **Step 2:** `npx vitest run tests/search-sections.test.ts` fails.
- [ ] **Step 3: `computeShown`.** When sectioned (`searchQuery` trimmed non-empty and `canSearchRecursively()`): here = `entries` filtered by `fold`, sorted with `compareEntries`; deeper = `searchResults ?? []` without items whose `parentOf(path) === location`, sorted by depth below `location` (count of `/` in the path after `location`'s prefix, `/` handled) then `compareEntries`; `shown = [...here, ...deeper]`, `sectionStart = here.length`. Add `searching` to the memo key.
- [ ] **Step 4: `searchSections` and layout.** Sections: "In This Folder" when `sectionStart > 0`; "In Subfolders" when deeper is non-empty, or while `searching` and here is non-empty. `computeLayout` maps them to `{ start, header: true }` and `headerH = LAYOUT_SIZES[currentTheme].sectionHeader`.
- [ ] **Step 5: Drawing.** `renderWindow` also writes one `<div class="section-header" style="top:Ypx">` per section (label, `<span class="count">N</span>`, and `<span class="searching">Searching…</span>` when `busy`) before the items; at most two, so always drawn. Grid items in "In This Folder" get an empty where line (heights stay uniform); list rows there an empty Location cell.
- [ ] **Step 6: Selection.** `addSearchResults` restores the cursor and anchor by path for the first batch too, and calls `selectFirstMatch` only when `shown` was empty before the batch.
- [ ] **Step 7: Style.** `.section-header`: absolute, `left`/`right` by the layout's `padX`, height `var(--section-header-h)` set from `LAYOUT_SIZES`, bold, smaller font, `opacity: var(--dim)`, a top border `var(--border)` on the second header, the spinner from `.searchbar.busy` reused for `.searching::before`. No new colours.
- [ ] **Step 8:** `npx vitest run tests/search-sections.test.ts` passes; `npm test` and `npx tsc --noEmit` pass.
- [ ] **Step 9: Commit** `feat: search shows this folder first, then subfolders`.

### Task 5: Docs, checks, screenshots, PR

- [ ] **Step 1:** README: the search row and limits mention "this folder first, then subfolders"; spec status → implemented.
- [ ] **Step 2:** `npm run build`, `npm test`, `cd src-tauri && cargo test`, commit (CRAP gate runs).
- [ ] **Step 3:** Screenshots with `npm run dev:mock`: before (main checkout build) and after, `--scene grid` and `list`, GNOME and macOS, light and dark, a search in a mock folder with nested matches; check `/big` search stays responsive.
- [ ] **Step 4:** Push, `scripts/pr-screenshots.sh`, PR from the template, self-review comment.
