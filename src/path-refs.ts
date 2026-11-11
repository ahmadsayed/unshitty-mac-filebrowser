// What the app remembers by path, kept pointing at items that were moved or
// renamed, or that went to the Trash. Code that moves, renames or trashes
// items reports it here as soon as the backend has done it (`moved`, `gone`)
// instead of updating each holder by hand; this module knows every holder
// and its rule:
//
//   holder       moved or renamed                     sent to the Trash
//   stars        follow (starred.json)                kept
//   bookmarks    follow, in order (pins.json)         kept
//   clipboard    follow (the same copy or cut)        dropped from it
//   tabs         follow: a background tab's folder,   kept
//                every tab's Back and Forward
//
// The folder the active tab shows isn't followed: the view does what it
// did before, going to the parent when a reload finds the folder gone.
//
// A holder is replaced at most once per report, and only when it changed,
// so a report saves it once (Batch Rename reports all its items at once). A
// paste reports each item as it lands, not at the end: until then a star
// would show on nothing, be lost if the window closed, and land on a dead
// path if the user renamed the landed item before the paste ended. A new
// path is always one the backend reported for a moved item, or a path
// inside a moved folder with the folder's part swapped: nothing is guessed.
// No DOM here: main.ts keeps the state, and saves and redraws in `set`.

import type { RepresentationsSummary } from "./api";
import { isWithin } from "./paths";

/** An item the backend moved or renamed from `from` to `to`. */
export interface Move {
  from: string;
  to: string;
}

/** Where `path` is after `moves`, taken in order: the moved item itself and
 *  anything inside it take the new path; anything else stays as it is. */
export function followPath(path: string, moves: readonly Move[]): string {
  let at = path;
  for (const { from, to } of moves) {
    if (isWithin(at, from)) at = to + at.slice(from.length);
  }
  return at;
}

/** `paths` after `follow`, or null when none of them moved. */
function followAll(paths: Iterable<string>, follow: (path: string) => string): string[] | null {
  let changed = false;
  const next: string[] = [];
  for (const path of paths) {
    const at = follow(path);
    changed ||= at !== path;
    next.push(at);
  }
  return changed ? next : null;
}

/** Something main.ts keeps: read it, and replace it (saving and redrawing
 *  what that needs). `set` runs at most once per report. */
export interface Slot<T> {
  get(): T;
  set(next: T): void;
}

/** Files copied or cut, waiting to be pasted. The same copy or cut keeps its
 *  object: only its `paths` and `dirs` are replaced on it. */
export interface Clip {
  paths: string[];
  dirs: Set<string>;
  mode: "copy" | "cut";
  /** What the listing said of each copied Arbesk asset's other files (its
   *  summary, undefined for none), so a paste elsewhere plans the copy
   *  out from it (performance plan, phase 4). Arbesk assets never move. */
  summaries?: ReadonlyMap<string, RepresentationsSummary | undefined>;
}

/** Where Back and Forward go. */
export interface History {
  back: string[];
  forward: string[];
}

/** Where a tab is, and where its Back and Forward go. */
export interface TabPaths extends History {
  location: string;
}

/** The open tabs: each one as saved when it was left, and the view's live
 *  history (`back`, `forward`), the one on screen's (`active`), whose saved
 *  copy is stale meanwhile. While a tab is switched to, it is `active` and
 *  `arriving` is the history it brings: the view takes that one when the
 *  tab's folder shows, and goes on with its own if the switch fails. */
export interface Tabs<Tab extends TabPaths = TabPaths> extends History {
  list: Tab[];
  active: number;
  arriving?: History | null;
}

/** `history` after `follow`, or null when nothing in it moved. */
function followHistory(history: History, follow: (path: string) => string): History | null {
  const back = followAll(history.back, follow);
  const forward = followAll(history.forward, follow);
  if (!back && !forward) return null;
  return { back: back ?? history.back, forward: forward ?? history.forward };
}

/** A background tab after `follow`, or null when nothing in it moved. */
function followTab<Tab extends TabPaths>(tab: Tab, follow: (path: string) => string): Tab | null {
  const location = follow(tab.location);
  const history = followHistory(tab, follow);
  if (location === tab.location && !history) return null;
  return { ...tab, location, ...history };
}

/** Everything that remembers paths. */
export interface PathHolders<Tab extends TabPaths = TabPaths> {
  /** Starred items; `set` saves them to starred.json. */
  stars: Slot<Set<string>>;
  /** The sidebar's bookmarks, in order; `set` saves them to pins.json and redraws the sidebar. */
  bookmarks: Slot<string[]>;
  /** The clipboard, null when empty. */
  clipboard: Slot<Clip | null>;
  /** The open tabs; `set` redraws the tab bar. */
  tabs: Slot<Tabs<Tab>>;
}

export class PathRefs<Tab extends TabPaths = TabPaths> {
  constructor(private readonly holders: PathHolders<Tab>) {}

  /** Items the backend moved or renamed, reported as soon as they moved:
   *  only pairs that really moved, so a star never follows a move that failed. */
  moved(moves: readonly Move[]): void {
    if (moves.length === 0) return;
    const follow = (path: string) => followPath(path, moves);
    this.followStars(follow);
    this.followBookmarks(follow);
    this.followClipboard(follow);
    this.followTabs(follow);
  }

  /** Items that went to the Trash: the clipboard drops them, and is emptied
   *  when nothing is left. Stars, bookmarks and tabs stay on their paths. */
  gone(paths: readonly string[]): void {
    const clip = this.holders.clipboard.get();
    if (!clip) return;
    const gone = new Set(paths);
    clip.paths = clip.paths.filter((path) => !gone.has(path));
    this.holders.clipboard.set(clip.paths.length > 0 ? clip : null);
  }

  private followStars(follow: (path: string) => string): void {
    const stars = followAll(this.holders.stars.get(), follow);
    if (stars) this.holders.stars.set(new Set(stars));
  }

  private followBookmarks(follow: (path: string) => string): void {
    const bookmarks = followAll(this.holders.bookmarks.get(), follow);
    // Two bookmarks can end at one path (Replace): the first one stays.
    if (bookmarks) this.holders.bookmarks.set([...new Set(bookmarks)]);
  }

  /** A copy or cut follows its items: a paste takes them from where they are now. */
  private followClipboard(follow: (path: string) => string): void {
    const clip = this.holders.clipboard.get();
    const paths = clip && followAll(clip.paths, follow);
    if (!clip || !paths) return;
    clip.paths = paths;
    clip.dirs = new Set([...clip.dirs].map(follow));
    this.holders.clipboard.set(clip);
  }

  private followTabs(follow: (path: string) => string): void {
    const tabs = this.holders.tabs.get();
    let changed = false;
    const list = tabs.list.map((tab, i) => {
      const moved = i === tabs.active ? null : followTab(tab, follow);
      changed ||= moved !== null;
      return moved ?? tab;
    });
    const live = followHistory(tabs, follow);
    const arriving = tabs.arriving && followHistory(tabs.arriving, follow);
    if (!changed && !live && !arriving) return;
    this.holders.tabs.set({
      list,
      active: tabs.active,
      back: live?.back ?? tabs.back,
      forward: live?.forward ?? tabs.forward,
      arriving: arriving ?? tabs.arriving,
    });
  }
}
