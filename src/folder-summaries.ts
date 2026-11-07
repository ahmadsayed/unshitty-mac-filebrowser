// What the view knows about the folders it shows: each one's item count
// ("12 items") and whether it keeps versions (the badge on its icon). They
// are fetched only for the folders the view asks about, in batches, never
// for remote folders (Drive, shares and SSH servers have no local count),
// and an answer that arrives after the view moved on is dropped. No DOM
// here: the app patches the view from `onChange`.

import type { FolderSummary } from "./api";
import { isRemotePath } from "./paths";

/** Folders per backend call: big enough for few calls, small enough that the first counts show soon. */
export const COUNT_BATCH = 400;

/** The folders whose count, and whose version badge, an answer changed. */
export interface SummaryChange {
  counts: string[];
  badges: string[];
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
  private generation = 0;
  private changes = 0;

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
    const pending = paths.filter((p) => !isRemotePath(p) && !this.counts.has(p) && !this.inFlight.has(p));
    if (pending.length === 0) return false;
    pending.forEach((p) => this.inFlight.add(p));
    const ask = this.ask();
    for (let i = 0; i < pending.length; i += COUNT_BATCH) {
      if (!(await this.answer(pending.slice(i, i + COUNT_BATCH), showHidden, ask))) return false;
    }
    return true;
  }

  /** Store one folder's summary, fetched by the caller. */
  remember(path: string, summary: FolderSummary | null): boolean {
    this.counts.set(path, summary?.count ?? null);
    this.changes++;
    const versioned = !!summary?.versioned;
    if (versioned === this.badges.has(path)) return false;
    if (versioned) this.badges.add(path);
    else this.badges.delete(path);
    return true;
  }

  /** Forget everything: a new location. Answers still on their way are dropped. */
  reset(): void {
    this.counts.clear();
    this.inFlight.clear();
    this.badges.clear();
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
    this.onChange({ counts: batch, badges });
    return true;
  }
}
