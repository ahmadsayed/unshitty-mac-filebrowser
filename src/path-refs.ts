// What the app remembers by path, kept pointing at items that were moved or
// renamed, or that went to the Trash. Code that moves, renames or trashes
// items reports it here once per batch (`moved`, `gone`) instead of updating
// each holder by hand; this module knows every holder and its rule:
//
//   holder       moved or renamed                   sent to the Trash
//   stars        follow (starred.json)               kept
//   bookmarks    follow, in their order (pins.json)  kept
//   clipboard    follow (the same copy or cut)       dropped from it
//
// A holder is replaced at most once per report, and only when it changed,
// so it saves once: two saves of one batch could land out of order. A new
// path is always one the backend reported for a moved item, or a path
// inside a moved folder with the folder's part swapped: nothing is guessed.
// No DOM here: main.ts keeps the state, and saves and redraws in `set`.

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
 *  object: only its `paths` are replaced on it. */
export interface Clip {
  paths: string[];
  dirs: Set<string>;
  mode: "copy" | "cut";
}

/** Everything that remembers paths. */
export interface PathHolders {
  /** Starred items; `set` saves them to starred.json. */
  stars: Slot<Set<string>>;
  /** The sidebar's bookmarks, in order; `set` saves them to pins.json and redraws the sidebar. */
  bookmarks: Slot<string[]>;
  /** The clipboard, null when empty. */
  clipboard: Slot<Clip | null>;
}

export class PathRefs {
  constructor(private readonly holders: PathHolders) {}

  /** Items the backend moved or renamed, one batch at a time: only pairs
   *  that really moved, so a star never follows a move that failed. */
  moved(moves: readonly Move[]): void {
    if (moves.length === 0) return;
    const follow = (path: string) => followPath(path, moves);
    this.followStars(follow);
    this.followBookmarks(follow);
    this.followClipboard(follow);
  }

  /** Items that went to the Trash: the clipboard drops them, and is emptied
   *  when nothing is left. Stars and bookmarks stay on their paths. */
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
}
