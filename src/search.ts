// The search: whether it is open and its text, the walk through subfolders
// and its answers, how names are compared, and how the matches are arranged
// in the view. No DOM: main.ts draws what it is told has changed.
import type { Entry, SearchSummary } from "./api";

/**
 * Text as search compares it: lower case, without accents. Combining marks
 * are stripped after canonical decomposition, exactly as the backend's
 * `fs_ops::fold` does, so the quick filter here and the recursive search
 * agree; `tests/fold-cases.json` holds the cases both are tested against.
 */
export function fold(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

type Compare = (a: Entry, b: Entry) => number;

/** What the view shows, in order. In a recursive search the items before
 *  `sectionStart` are "In This Folder" and the rest "In Subfolders"; -1 when
 *  the view has no sections. */
export interface Arranged {
  items: Entry[];
  sectionStart: number;
}

/**
 * The view's items for the search `query` at the folder `at` (no search when
 * `query` is blank). A place that `recursive`ly searches shows this folder's
 * matches from its listing at once, sorted, then the walk's `results` from
 * subfolders, nearest folders first; the walk's matches in `at` itself are
 * already in the first part. Elsewhere the listing is filtered in place.
 */
export function arrangeSearch(entries: readonly Entry[], results: readonly Entry[] | null, query: string,
                              at: string, compare: Compare, recursive: boolean): Arranged {
  const q = fold(query.trim());
  if (!q) return { items: [...entries].sort(compare), sectionStart: -1 };
  const here = entries.filter((e) => fold(e.name).includes(q)).sort(compare);
  if (!recursive) return { items: here, sectionStart: -1 };
  const prefix = at.endsWith("/") ? at : `${at}/`;
  const depth = (e: Entry) => e.path.slice(prefix.length).split("/").length;
  const deeper = (results ?? [])
    .filter((e) => depth(e) > 1)
    .sort((a, b) => depth(a) - depth(b) || compare(a, b));
  return { items: [...here, ...deeper], sectionStart: here.length };
}

/** A headed part of the view. */
export interface SearchSection {
  label: string;
  start: number;
  count: number;
  /** Still searching: the header says so. */
  busy: boolean;
}

/** The headed sections of `count` items arranged with `sectionStart`; none
 *  outside a recursive search. "In Subfolders" shows while the search is
 *  `busy`, and afterwards only with matches. */
export function searchSections(sectionStart: number, count: number, busy: boolean): SearchSection[] {
  if (sectionStart < 0) return [];
  const deeper = count - sectionStart;
  const out: SearchSection[] = [];
  if (sectionStart > 0) out.push({ label: "In This Folder", start: 0, count: sectionStart, busy: false });
  if (deeper > 0 || (busy && sectionStart > 0)) {
    out.push({ label: "In Subfolders", start: sectionStart, count: deeper, busy });
  }
  return out;
}

// ---------------------------------------------------------------------------
// The session: open, text, walk, answers
// ---------------------------------------------------------------------------

/** Cap on recursive results, so a broad query can't build an endless view. */
export const SEARCH_LIMIT = 1000;
/** How long typing must pause before the walk starts. */
export const SEARCH_DELAY_MS = 160;

/** Walk `at`'s subfolders for `query`, sending matches in batches. */
export type SearchWalk = (at: string, query: string, limit: number, onFound: (batch: Entry[]) => void) => Promise<SearchSummary>;

/**
 * What changed, for the view to draw:
 * - `restarted`: the text changed or the search ran again; old results and
 *   the stop are gone, and `busy` says whether a walk is coming;
 * - `grew`: a batch of results arrived;
 * - `ended`: the walk finished (`results` is no longer null, `stop` says
 *   whether it stopped early);
 * - `failed`: the walk failed (with the error); `results` is empty.
 */
export type SearchChange = "restarted" | "grew" | "ended" | "failed";

/**
 * The search in one window: whether it is open, its text, and the walk
 * through subfolders that a place which searches recursively runs.
 *
 * Every restart forgets the old results and stop, and only the latest
 * restart's answers count: a batch or end that arrives after the text
 * changed, the search closed or the view moved to another place is dropped.
 * The search's place is the view's: navigating closes the search, but a key
 * typed while a navigation still waits opens it on the folder being left,
 * so the view calls `moveTo` once it has moved. A walk waits for a pause in
 * typing, and the backend stops a running walk when the next one starts;
 * the session cancels it itself when it closes or when the text no longer
 * walks.
 */
export class SearchSession {
  private openText: string | null = null;
  private at = "";
  private recursive = false;
  /** Bumped by every restart and close; answers carry the one they began with. */
  private seq = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private found: Entry[] | null = null;
  private walking = false;
  private stopped: SearchSummary | null = null;

  constructor(
    private readonly walk: SearchWalk,
    private readonly cancel: () => Promise<void>,
    private readonly onChange: (change: SearchChange, error?: unknown) => void,
    private readonly delayMs = SEARCH_DELAY_MS,
  ) {}

  get isOpen(): boolean {
    return this.openText !== null;
  }

  /** The text as typed; empty when closed. */
  get text(): string {
    return this.openText ?? "";
  }

  /** Whether a walk is coming or running. */
  get busy(): boolean {
    return this.walking;
  }

  /** How the walk ended, once it has. */
  get stop(): SearchSummary | null {
    return this.stopped;
  }

  /** The walk's results so far; null until it answers. A new array per
   *  batch, so a memo keyed on identity sees the change. */
  get results(): readonly Entry[] | null {
    return this.found;
  }

  /** What a tab keeps to bring the search back: its text, null when closed. */
  get saved(): string | null {
    return this.openText;
  }

  /** Open the search at the folder `at`, with no text yet; `recursive` is
   *  whether this place walks subfolders. Nothing changes until `setText`. */
  open(at: string, recursive: boolean): void {
    this.close();
    this.openText = "";
    this.at = at;
    this.recursive = recursive;
  }

  /** The view moved to `at` with the search open: search there instead.
   *  Answers for the old place are dropped, and a text runs again here (a
   *  place that doesn't walk cancels the old place's walk). */
  moveTo(at: string, recursive: boolean): void {
    if (this.openText === null) return;
    this.at = at;
    this.recursive = recursive;
    if (this.openText) this.restart();
  }

  /** The user typed: restart with `text`. */
  setText(text: string): void {
    if (this.openText === null) return;
    this.openText = text;
    this.restart();
  }

  /** Run the same search again, as a reload does. */
  rerun(): void {
    if (this.openText !== null) this.restart();
  }

  close(): void {
    if (this.openText === null) return;
    this.openText = null;
    this.forget();
    if (this.walking) this.cancelWalk();
    this.walking = false;
  }

  private restart(): void {
    this.forget();
    const query = this.text.trim();
    const walks = !!query && this.recursive;
    if (this.walking && !walks) this.cancelWalk();
    this.walking = walks;
    if (walks) {
      const seq = this.seq;
      this.timer = setTimeout(() => this.start(seq, query), this.delayMs);
    }
    this.onChange("restarted");
  }

  /** Drop the old results and stop, and make every answer on its way stale. */
  private forget(): void {
    this.seq += 1;
    clearTimeout(this.timer);
    this.found = null;
    this.stopped = null;
  }

  private cancelWalk(): void {
    void this.cancel().catch(() => {});
  }

  private start(seq: number, query: string): void {
    const current = () => seq === this.seq;
    this.walk(this.at, query, SEARCH_LIMIT, (batch) => {
      if (!current()) return;
      this.found = (this.found ?? []).concat(batch);
      this.onChange("grew");
    })
      .then((summary) => {
        if (!current()) return;
        this.walking = false;
        this.found ??= [];
        this.stopped = summary;
        this.onChange("ended");
      })
      .catch((err) => {
        if (!current()) return;
        this.walking = false;
        this.found = [];
        this.onChange("failed", err);
      });
  }
}
