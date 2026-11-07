// The search: how names are compared, and how the matches are arranged in
// the view. No DOM.
import type { Entry } from "./api";

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
