// Performance plan, phase 5: which items a copy builds ahead of the one it
// places (`buildsAhead`, src/arbesk-ahead.ts): up to BUILDS_AHEAD after it,
// each once, only those it is told are built first, none once stopped; a
// stop cancels the builds still running, never one that ended.
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
