// What the view knows about the folders it shows: each one's item count
// ("12 items") and whether it keeps versions (the badge on its icon). They
// are fetched only for the folders the view asks about, in batches, never
// for remote folders (Drive, shares and SSH servers have no local count),
// and an answer that arrives after the view moved on is dropped. Counts
// depend on Show Hidden, so asking with the other setting forgets them. A
// reload may have changed any of them (a paste into a subfolder): they stay
// shown, but each is asked for again the next time the view asks. No DOM
// here: the app patches the view from `onChange`.

import type { FolderSummary } from "./api";
import { isRemotePath } from "./paths";

/** Folders per backend call: big enough for few calls, small enough that the first counts show soon. */
export const COUNT_BATCH = 400;

/** The folders whose count, and whose version badge, an answer changed;
 *  `recounted` are those among them asked for with one(). */
export interface SummaryChange {
  counts: string[];
  badges: string[];
  recounted: string[];
}

type Fetch = (paths: string[], showHidden: boolean) => Promise<(FolderSummary | null)[]>;

/** Which request an answer belongs to: dropped once the location or the generation moved on. */
interface Ask {
  location: string;
  generation: number;
}

export class FolderSummaries {
  private readonly counts = new Map<string, number | null>();
  private readonly badges = new Set<string>();
  private readonly inFlight = new Set<string>();
  /** Folders asked for with one() whose answer hasn't come yet. */
  private readonly recount = new Set<string>();
  /** Known folders a reload may have changed: shown until asked for and answered again. */
  private readonly stale = new Set<string>();
  private generation = 0;
  private changes = 0;
  /** The Show Hidden setting the counts were asked with; null before the first ask. */
  private showHidden: boolean | null = null;

  /**
   * `fetch` asks the backend, `current` is the location on screen, and
   * `onChange` hears about each batch of answers that was kept.
   */
  constructor(
    private readonly fetch: Fetch,
    private readonly current: () => string,
    private readonly onChange: (change: SummaryChange) => void,
  ) {}

  /** The folder's item count: undefined while unknown, null when it couldn't be counted. */
  count(path: string): number | null | undefined {
    return this.counts.get(path);
  }

  /** Whether the folder's summary said it keeps versions. */
  versioned(path: string): boolean {
    return this.badges.has(path);
  }

  /** Changes whenever a count does; sorting by size keys on it. */
  get version(): number {
    return this.changes;
  }

  /**
   * Count `paths` that aren't known or on their way yet, in batches of
   * COUNT_BATCH. True when every batch was answered and kept, so a view
   * sorted by size knows to sort again.
   */
  async request(paths: string[], showHidden: boolean): Promise<boolean> {
    this.useSetting(showHidden);
    const pending = paths.filter((p) => !isRemotePath(p) && !this.fresh(p) && !this.inFlight.has(p));
    if (pending.length === 0) return false;
    pending.forEach((p) => this.inFlight.add(p));
    const ask = this.ask();
    for (let i = 0; i < pending.length; i += COUNT_BATCH) {
      if (!(await this.answer(pending.slice(i, i + COUNT_BATCH), showHidden, ask))) return false;
    }
    return true;
  }

  /**
   * Count one folder (the selected one, for the floating bar) by the same
   * rules as request(). It is marked as asked for at once, so asking again
   * before the answer doesn't fetch it twice. Its answer, or that of the
   * batch already asking for it, lists it as `recounted`.
   */
  one(path: string, showHidden: boolean): void {
    this.useSetting(showHidden);
    if (isRemotePath(path) || this.fresh(path)) return;
    if (!this.counts.has(path)) {
      this.counts.set(path, null);
      this.changes++;
    }
    this.recount.add(path);
    if (this.inFlight.has(path)) return;
    this.inFlight.add(path);
    void this.answer([path], showHidden, this.ask());
  }

  /** Forget everything: a new location. Answers still on their way are dropped. */
  reset(): void {
    this.counts.clear();
    this.stale.clear();
    this.inFlight.clear();
    this.recount.clear();
    this.badges.clear();
    this.generation++;
    this.changes++;
  }

  /**
   * The location was listed again, and any folder in it may have changed:
   * every count and badge stays shown until a fresh answer replaces it, but
   * is asked for again the next time the view (or one()) asks. Answers on
   * their way were asked for before and are dropped.
   */
  reloaded(): void {
    this.counts.forEach((_, path) => this.stale.add(path));
    this.inFlight.clear();
    this.generation++;
  }

  /** Known, and not since a reload. */
  private fresh(path: string): boolean {
    return this.counts.has(path) && !this.stale.has(path);
  }

  /** A new Show Hidden setting forgets the counts and drops the answers on their way.
   *  The version badges stay: they don't depend on it. */
  private useSetting(showHidden: boolean): void {
    if (showHidden === this.showHidden) return;
    const changed = this.showHidden !== null;
    this.showHidden = showHidden;
    if (!changed) return;
    this.counts.clear();
    this.stale.clear();
    this.inFlight.clear();
    this.recount.clear();
    this.generation++;
    this.changes++;
  }

  private ask(): Ask {
    return { location: this.current(), generation: this.generation };
  }

  private stillCurrent(ask: Ask): boolean {
    return ask.generation === this.generation && ask.location === this.current();
  }

  /** Fetch one batch and keep the answers if they are still wanted. A failed call counts as "couldn't count". */
  private async answer(batch: string[], showHidden: boolean, ask: Ask): Promise<boolean> {
    const result = await this.fetch(batch, showHidden).catch((): (FolderSummary | null)[] => []);
    if (!this.stillCurrent(ask)) return false;
    batch.forEach((p) => this.inFlight.delete(p));
    const badges = batch.filter((p, i) => this.remember(p, result[i] ?? null));
    const recounted = batch.filter((p) => this.recount.delete(p));
    this.onChange({ counts: batch, badges, recounted });
    return true;
  }

  /** Store a folder's summary; true when its version badge changed. */
  private remember(path: string, summary: FolderSummary | null): boolean {
    this.counts.set(path, summary?.count ?? null);
    this.stale.delete(path);
    this.changes++;
    const versioned = !!summary?.versioned;
    if (versioned === this.badges.has(path)) return false;
    if (versioned) this.badges.add(path);
    else this.badges.delete(path);
    return true;
  }
}
