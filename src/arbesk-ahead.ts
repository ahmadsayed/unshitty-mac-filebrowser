// A copy out of Arbesk builds ahead (performance plan, phase 5), no DOM.
// While the copy places one item, the Arbesk sources of the next
// `BUILDS_AHEAD` items are built into the app cache (`arbesk_prefetch`:
// the very export the copy's own build runs, so the copy then finds the
// file there). Only the download moves ahead: placing stays the copy's,
// one item at a time and in order. A build ahead that fails is not told:
// the copy builds again at its turn and tells its own failure, in today's
// words. A built item never placed (a Skip, a Cancel) stays in the app
// cache, never in the target folder.

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
  /** The copy is about to place item `i`: start the builds of the items
   *  after it, up to `BUILDS_AHEAD` of them, not started yet. */
  placing(i: number): void;
  /** Stop every build ahead still running and start none again (a
   *  Cancel, or the copy ended). */
  stop(): void;
}

/** The builds ahead of a copy of `sources` (in placing order), for the
 *  sources `wanted` says are built before they are placed. */
export function buildsAhead(sources: readonly string[], wanted: (path: string) => boolean, calls: AheadCalls): BuildsAhead {
  const running = new Set<number>();
  let next = 0;
  let stopped = false;
  return {
    placing(i) {
      next = Math.max(next, i + 1);
      for (; !stopped && next <= Math.min(i + BUILDS_AHEAD, sources.length - 1); next++) {
        if (!wanted(sources[next])) continue;
        const ticket = calls.ticket();
        running.add(ticket);
        void calls.build(sources[next], ticket).then(
          () => running.delete(ticket),
          () => running.delete(ticket),
        );
      }
    },
    stop() {
      stopped = true;
      if (running.size > 0) calls.cancel([...running]);
      running.clear();
    },
  };
}
