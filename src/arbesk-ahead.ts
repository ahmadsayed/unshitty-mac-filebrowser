// A copy out of Arbesk builds ahead (performance plan, phase 5), no DOM.
// While the copy places one item, the Arbesk sources of the next
// `BUILDS_AHEAD` items are built into the app cache (`arbesk_prefetch`:
// the very export the copy's own build runs, so the copy then finds the
// file there). Only the download moves ahead: placing stays the copy's,
// one item at a time and in order. At an item's turn the copy waits for
// its build ahead if that still runs (`turn`): its failure then is the
// copy's (the copy's own build would have waited for it and failed the
// same way), told in today's words; a build ahead that had already failed
// before the turn is not told, the copy builds again. A built item never
// placed (a Skip, a Cancel) stays in the app cache, never in the target
// folder, and the build of an item the copy passed is stopped.

/** How many items after the one being placed are built ahead. */
export const BUILDS_AHEAD = 4;

/** What the builds ahead ask of the backend. */
export interface AheadCalls {
  /** Build `path` into the app cache under cancel ticket `ticket`. */
  build(path: string, ticket: number): Promise<unknown>;
  /** Stop the builds with these tickets. */
  cancel(tickets: number[]): void;
  /** A ticket no other fetch of this window has used. */
  ticket(): number;
}

export interface BuildsAhead {
  /** The copy is about to place item `i`: stop the builds of the items
   *  before it still running (passed: a Skip, a failure), then start the
   *  builds of the items after it, up to `BUILDS_AHEAD` of them, not
   *  started yet. So at most `BUILDS_AHEAD` builds ahead run, plus item
   *  `i`'s own while the copy waits for it. */
  placing(i: number): void;
  /** Item `i`'s turn: once its build ahead, if it was still running, has
   *  ended. Its error when it failed then (a cancel too), else null: no
   *  build ahead, one that ended before the turn, or one that built. */
  turn(i: number): Promise<{ error: unknown } | null>;
  /** Stop every build ahead still running and start none again (a
   *  Cancel, or the copy ended). */
  stop(): void;
}

/** One build ahead: its item, its ticket, and how it ends. */
type Ahead = { index: number; ticket: number; ended: Promise<{ error: unknown } | null> };

/** The builds ahead of a copy of `sources` (in placing order), for the
 *  sources `wanted` says are built before they are placed. */
export function buildsAhead(sources: readonly string[], wanted: (path: string) => boolean, calls: AheadCalls): BuildsAhead {
  /** The builds still running, by item. */
  const running = new Map<number, Ahead>();
  const cancel = (builds: Ahead[]) => {
    for (const build of builds) running.delete(build.index);
    if (builds.length > 0) calls.cancel(builds.map((build) => build.ticket));
  };
  let next = 0;
  let stopped = false;
  return {
    placing(i) {
      if (stopped) return;
      cancel([...running.values()].filter((build) => build.index < i));
      next = Math.max(next, i + 1);
      for (; next <= Math.min(i + BUILDS_AHEAD, sources.length - 1); next++) {
        if (!wanted(sources[next])) continue;
        const index = next;
        const ticket = calls.ticket();
        const build: Ahead = {
          index,
          ticket,
          ended: calls.build(sources[index], ticket).then(
            () => null,
            (error: unknown) => ({ error }),
          ).finally(() => {
            if (running.get(index) === build) running.delete(index);
          }),
        };
        running.set(index, build);
      }
    },
    async turn(i) {
      const build = running.get(i);
      return build ? build.ended : null;
    },
    stop() {
      stopped = true;
      cancel([...running.values()]);
    },
  };
}
