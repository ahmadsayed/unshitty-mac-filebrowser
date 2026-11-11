// Performance plan, phase 5: which items a copy builds ahead of the one it
// places (`buildsAhead`, src/arbesk-ahead.ts): up to BUILDS_AHEAD after it,
// each once, only those it is told are built first, none once stopped; a
// stop cancels the builds still running, never one that ended; an item
// passed has its build stopped; an item's turn waits for its build still
// running and takes its failure.
import { expect, test } from "vitest";
import { BUILDS_AHEAD, buildsAhead, type AheadCalls } from "../src/arbesk-ahead";

/** Calls that record each build and cancel; `finish(path)` ends a build. */
function recorder(): AheadCalls & { started: string[]; cancelled: number[][]; finish(path: string, fail?: boolean): Promise<void> } {
  let seq = 100;
  const ends = new Map<string, { resolve: () => void; reject: (e: unknown) => void }>();
  const calls = {
    started: [] as string[],
    cancelled: [] as number[][],
    build: (path: string) => {
      calls.started.push(path);
      return new Promise<void>((resolve, reject) => ends.set(path, { resolve, reject }));
    },
    cancel: (tickets: number[]) => void calls.cancelled.push(tickets),
    ticket: () => ++seq,
    async finish(path: string, fail = false) {
      const end = ends.get(path)!;
      if (fail) end.reject("Arbesk's gateway could not be reached.");
      else end.resolve();
      await new Promise((r) => setTimeout(r, 0));
    },
  };
  return calls;
}

const items = (n: number) => Array.from({ length: n }, (_, i) => `arbesk://a/1~C/a${i}~A${i}.gltf`);

test("up to four items after the one placed are built, each once, in order", () => {
  expect(BUILDS_AHEAD).toBe(4);
  const calls = recorder();
  const sources = items(8);
  const ahead = buildsAhead(sources, () => true, calls);
  ahead.placing(0);
  expect(calls.started).toEqual(sources.slice(1, 5));
  ahead.placing(1);
  expect(calls.started).toEqual(sources.slice(1, 6));
  // A step back (or the same item again) starts nothing new.
  ahead.placing(1);
  ahead.placing(0);
  expect(calls.started).toEqual(sources.slice(1, 6));
  // A jump (the items between passed by) builds only what follows the
  // item placed: that one the copy builds itself.
  ahead.placing(6);
  expect(calls.started).toEqual([...sources.slice(1, 6), sources[7]]);
});

test("only the sources it is told are built first; the window still counts every item", () => {
  const calls = recorder();
  const sources = [...items(3), "/Users/me/notes.txt", ...items(6).slice(3)];
  const ahead = buildsAhead(sources, (path) => path.startsWith("arbesk://"), calls);
  ahead.placing(0);
  expect(calls.started).toEqual([sources[1], sources[2], sources[4]]);
});

test("a stop cancels the builds still running, and starts none again", async () => {
  const calls = recorder();
  const sources = items(6);
  const ahead = buildsAhead(sources, () => true, calls);
  ahead.placing(0);
  await calls.finish(sources[1]);
  await calls.finish(sources[2], true);
  ahead.stop();
  expect(calls.cancelled).toEqual([[103, 104]]);
  ahead.placing(1);
  expect(calls.started).toEqual(sources.slice(1, 5));
  // Stopping again sends nothing.
  ahead.stop();
  expect(calls.cancelled).toHaveLength(1);
});

test("one item, or none built first: no build and no cancel", () => {
  const calls = recorder();
  const one = buildsAhead(items(1), () => true, calls);
  one.placing(0);
  one.stop();
  const local = buildsAhead(["/a", "/b", "/c"], () => false, calls);
  local.placing(0);
  local.stop();
  expect(calls.started).toEqual([]);
  expect(calls.cancelled).toEqual([]);
});

test("items passed stop their builds: a long run of Skips never builds more than four ahead", () => {
  const calls = recorder();
  const sources = items(70);
  const ahead = buildsAhead(sources, () => true, calls);
  // The recorder's tickets go 101, 102, … in the order the builds start.
  const started = (path: string) => 101 + calls.started.indexOf(path);
  let peak = 0;
  for (let i = 0; i < sources.length; i++) {
    // Item i is skipped: placing(i), then straight on, nothing finishes.
    ahead.placing(i);
    const cancelled = new Set(calls.cancelled.flat());
    const running = calls.started.filter((path) => !cancelled.has(started(path)));
    peak = Math.max(peak, running.length);
    // Only item i (its own, the copy would wait for) and the four after it.
    expect(running.every((path) => sources.indexOf(path) >= i && sources.indexOf(path) <= i + BUILDS_AHEAD)).toBe(true);
  }
  ahead.stop();
  expect(peak).toBe(BUILDS_AHEAD + 1);
  // Every build ahead was cancelled, each once.
  const cancelled = calls.cancelled.flat().sort((a, b) => a - b);
  expect(cancelled).toEqual(calls.started.map(started).sort((a, b) => a - b));
  expect(calls.started).toEqual(sources.slice(1));
});

test("an item's turn waits for its build ahead still running, and takes its failure", async () => {
  const calls = recorder();
  const sources = items(6);
  const ahead = buildsAhead(sources, () => true, calls);
  ahead.placing(0);
  // No build ahead for the item the copy builds itself.
  await expect(ahead.turn(0)).resolves.toBeNull();
  // Item 1's still runs: its turn ends with it, and its failure.
  ahead.placing(1);
  const turn = ahead.turn(1);
  await calls.finish(sources[1], true);
  await expect(turn).resolves.toEqual({ error: "Arbesk's gateway could not be reached." });
  // Item 2's failed before its turn: the copy builds again, nothing told.
  await calls.finish(sources[2], true);
  ahead.placing(2);
  await expect(ahead.turn(2)).resolves.toBeNull();
  // Item 3's built while still running at its turn.
  ahead.placing(3);
  const built = ahead.turn(3);
  await calls.finish(sources[3]);
  await expect(built).resolves.toBeNull();
  // Once stopped, no turn waits.
  ahead.stop();
  await expect(ahead.turn(4)).resolves.toBeNull();
});
