// The view's selection: which items are selected, the cursor (the item the
// keyboard is on) and the anchor (where a Shift range starts). Indices are
// positions in the list on screen, which the app replaces as it sorts,
// filters and reloads, so the selection reads it through `items()` rather
// than keeping a copy. No DOM here: the app redraws after a change.
//
// Every change the user makes bumps `intent`; the app's own changes (reloads,
// restoring a tab, following the cursor) don't. A copy or move selects what
// it made only when `intent` hasn't moved since it was pasted, so a newer
// choice by the user wins (#50).

/** Anything with a path: the selection only reads `path`. */
export interface Selectable {
  path: string;
}

/** The items under the cursor and anchor, to find again after the list changes. */
export interface Mark {
  cursorPath: string | undefined;
  anchorPath: string | undefined;
  cursor: number;
  anchor: number;
}

export class Selection<T extends Selectable> {
  private readonly chosen = new Set<string>();
  private anchorAt = -1;
  private cursorAt = -1;
  private intentCount = 0;

  constructor(private readonly items: () => readonly T[]) {}

  get anchor(): number {
    return this.anchorAt;
  }

  get cursor(): number {
    return this.cursorAt;
  }

  /** How many choices the user has made; compare two readings to see if they chose again. */
  get intent(): number {
    return this.intentCount;
  }

  get size(): number {
    return this.chosen.size;
  }

  has(path: string): boolean {
    return this.chosen.has(path);
  }

  /** The selected paths, including any not in the list on screen. */
  paths(): string[] {
    return [...this.chosen];
  }

  /** The selected items, in the order shown. */
  selected(): T[] {
    return this.items().filter((item) => this.chosen.has(item.path));
  }

  /** Where `path` is in the list on screen, or -1. */
  indexOf(path: string): number {
    return this.items().findIndex((item) => item.path === path);
  }

  // ---- The user's changes ---------------------------------------------------

  /** Count a user change the selection itself can't see, such as typing a search. */
  noteIntent(): void {
    this.intentCount++;
  }

  /** Select just the item at `index`, with the cursor and anchor on it. */
  only(index: number): void {
    this.noteIntent();
    this.chosen.clear();
    const item = this.items()[index];
    if (item) this.chosen.add(item.path);
    this.anchorAt = this.cursorAt = index;
  }

  /** Select from `from` to `to` (either way round), adding to the selection when `additive`; the cursor goes to `to`. */
  range(from: number, to: number, additive: boolean): void {
    this.noteIntent();
    if (!additive) this.chosen.clear();
    const list = this.items();
    const [a, b] = from < to ? [from, to] : [to, from];
    for (let i = a; i <= b; i++) if (list[i]) this.chosen.add(list[i].path);
    this.cursorAt = to;
  }

  all(): void {
    this.noteIntent();
    for (const item of this.items()) this.chosen.add(item.path);
  }

  clear(): void {
    this.noteIntent();
    this.chosen.clear();
  }

  /** Select or unselect the item at `index`, leaving the cursor where it is. */
  toggle(index: number): void {
    this.noteIntent();
    const path = this.items()[index].path;
    if (this.chosen.has(path)) this.chosen.delete(path);
    else this.chosen.add(path);
  }

  /** A new folder or tab: nothing chosen and no cursor, or just `select` (an item to show the user). */
  reset(select?: string): void {
    this.noteIntent();
    this.chosen.clear();
    this.anchorAt = this.cursorAt = -1;
    if (select) this.chosen.add(select);
  }

  // ---- The app's own changes ------------------------------------------------

  /** Put the cursor and anchor on `index` without changing what is selected. */
  focus(index: number): void {
    this.anchorAt = this.cursorAt = index;
  }

  /** Select the first item, as search does with its first match; nothing changes when the list is empty. */
  selectFirst(): void {
    const first = this.items()[0];
    if (!first) return;
    this.chosen.clear();
    this.chosen.add(first.path);
    this.anchorAt = this.cursorAt = 0;
  }

  /** Forget the cursor and anchor, keeping what is selected. */
  dropCursor(): void {
    this.anchorAt = this.cursorAt = -1;
  }

  /** Select the paths of `keep` that are still in `available`, and nothing else. */
  retain(keep: Iterable<string>, available: readonly Selectable[]): void {
    const listed = new Set(available.map((item) => item.path));
    const wanted = [...keep];
    this.chosen.clear();
    for (const path of wanted) if (listed.has(path)) this.chosen.add(path);
  }

  /** Select exactly `paths`, as the rubber band does while it moves. */
  replace(paths: Iterable<string>): void {
    const next = [...paths];
    this.chosen.clear();
    for (const path of next) this.chosen.add(path);
  }

  /** Put back a saved anchor and cursor, kept inside the list on screen. */
  place(anchor: number, cursor: number): void {
    const last = this.items().length - 1;
    this.anchorAt = last < 0 ? -1 : Math.min(anchor, last);
    this.cursorAt = last < 0 ? -1 : Math.min(cursor, last);
  }

  // ---- Keeping the cursor on its item -----------------------------------------

  mark(): Mark {
    const list = this.items();
    return {
      cursorPath: list[this.cursorAt]?.path,
      anchorPath: list[this.anchorAt]?.path,
      cursor: this.cursorAt,
      anchor: this.anchorAt,
    };
  }

  /**
   * Move the cursor and anchor back onto the items `mark` saw. An item that
   * has gone leaves its old place (kept in range) when `near`, no cursor
   * otherwise.
   */
  follow(mark: Mark, near: boolean): void {
    this.cursorAt = this.findAgain(mark.cursorPath, near ? mark.cursor : -1);
    this.anchorAt = this.findAgain(mark.anchorPath, near ? mark.anchor : -1);
  }

  private findAgain(path: string | undefined, fallback: number): number {
    const i = path === undefined ? -1 : this.indexOf(path);
    return i >= 0 ? i : Math.min(fallback, this.items().length - 1);
  }
}
