// FolderSummaries on its own, with a fetch the test answers by hand.
import { expect, test } from "vitest";
import type { FolderSummary } from "../src/api";
import { COUNT_BATCH, FolderSummaries, type SummaryChange } from "../src/folder-summaries";

type Call = { paths: string[]; showHidden: boolean; answer(summaries?: (FolderSummary | null)[]): Promise<void>; fail(): Promise<void> };

/** A FolderSummaries whose backend calls wait until the test answers them. */
function setup(location = "/home") {
  const calls: Call[] = [];
  const changes: SummaryChange[] = [];
  const here = { location };
  const fetch = (paths: string[], showHidden: boolean) =>
    new Promise<(FolderSummary | null)[]>((resolve, reject) => {
      const settle = async () => {
        for (let i = 0; i < 3; i++) await Promise.resolve();
      };
      calls.push({
        paths,
        showHidden,
        answer: (summaries) => {
          resolve(summaries ?? paths.map((_, i) => ({ count: i + 1, versioned: false })));
          return settle();
        },
        fail: () => {
          reject("Permission denied");
          return settle();
        },
      });
    });
  const summaries = new FolderSummaries(fetch, () => here.location, (c) => changes.push(c));
  return { summaries, calls, changes, here };
}

test("counts what it's asked for and reports each kept batch", async () => {
  const { summaries, calls, changes } = setup();
  const done = summaries.request(["/home/a", "/home/b"], false);
  expect(summaries.count("/home/a")).toBeUndefined();
  expect(calls.map((c) => c.paths)).toEqual([["/home/a", "/home/b"]]);
  const before = summaries.version;
  await calls[0].answer([{ count: 3, versioned: true }, null]);
  expect(await done).toBe(true);
  expect(summaries.count("/home/a")).toBe(3);
  expect(summaries.count("/home/b")).toBeNull();
  expect(summaries.versioned("/home/a")).toBe(true);
  expect(summaries.versioned("/home/b")).toBe(false);
  expect(summaries.version).not.toBe(before);
  expect(changes).toEqual([{ counts: ["/home/a", "/home/b"], badges: ["/home/a"], recounted: [] }]);
});

test("skips remote, known and in-flight folders", async () => {
  const { summaries, calls } = setup();
  void summaries.request(["/home/a"], false);
  await calls[0].answer();
  void summaries.request(["/home/b"], false);
  const again = summaries.request(["gdrive://root/x", "share://abc/x", "sftp://me@host/x", "/home/a", "/home/b"], false);
  expect(await again).toBe(false);
  expect(calls.map((c) => c.paths)).toEqual([["/home/a"], ["/home/b"]]);
});

test("asks in batches and reports each one as it lands", async () => {
  const { summaries, calls, changes } = setup();
  const paths = Array.from({ length: COUNT_BATCH + 5 }, (_, i) => `/home/f${i}`);
  const done = summaries.request(paths, true);
  expect(calls.map((c) => c.paths.length)).toEqual([COUNT_BATCH]);
  expect(calls[0].showHidden).toBe(true);
  await calls[0].answer();
  expect(changes).toHaveLength(1);
  expect(calls.map((c) => c.paths.length)).toEqual([COUNT_BATCH, 5]);
  await calls[1].answer();
  expect(await done).toBe(true);
  expect(changes.map((c) => c.counts.length)).toEqual([COUNT_BATCH, 5]);
});

test("a failed call leaves the folders uncounted rather than asking forever", async () => {
  const { summaries, calls } = setup();
  const done = summaries.request(["/home/a"], false);
  await calls[0].fail();
  expect(await done).toBe(true);
  expect(summaries.count("/home/a")).toBeNull();
  expect(await summaries.request(["/home/a"], false)).toBe(false);
});

test("drops answers that arrive after the location changed", async () => {
  const { summaries, calls, changes, here } = setup();
  const done = summaries.request(["/home/a"], false);
  here.location = "/elsewhere";
  await calls[0].answer();
  expect(await done).toBe(false);
  expect(summaries.count("/home/a")).toBeUndefined();
  expect(changes).toEqual([]);
});

test("reset forgets everything and drops answers still on their way", async () => {
  const { summaries, calls, changes } = setup();
  void summaries.request(["/home/a"], false);
  await calls[0].answer([{ count: 2, versioned: true }]);
  const late = summaries.request(["/home/b"], false);
  const before = summaries.version;
  summaries.reset();
  expect(summaries.version).not.toBe(before);
  expect(summaries.count("/home/a")).toBeUndefined();
  expect(summaries.versioned("/home/a")).toBe(false);
  // The same folder can be asked for again at once.
  void summaries.request(["/home/b"], false);
  expect(calls.map((c) => c.paths)).toEqual([["/home/a"], ["/home/b"], ["/home/b"]]);
  await calls[1].answer([{ count: 99, versioned: false }]);
  expect(await late).toBe(false);
  expect(summaries.count("/home/b")).toBeUndefined();
  await calls[2].answer([{ count: 4, versioned: false }]);
  expect(summaries.count("/home/b")).toBe(4);
  expect(changes.map((c) => c.counts)).toEqual([["/home/a"], ["/home/b"]]);
});

test("one() counts a single folder by the same rules", async () => {
  const { summaries, calls, changes, here } = setup();
  summaries.one("sftp://me@host/x", false);
  summaries.one("share://abc/x", false);
  expect(calls).toEqual([]);
  expect(summaries.count("sftp://me@host/x")).toBeUndefined();

  // Asked for at once, so asking again doesn't fetch twice.
  summaries.one("/home/a", true);
  expect(summaries.count("/home/a")).toBeNull();
  summaries.one("/home/a", true);
  expect(calls.map((c) => [c.paths, c.showHidden])).toEqual([[["/home/a"], true]]);
  await calls[0].answer([{ count: 7, versioned: true }]);
  expect(summaries.count("/home/a")).toBe(7);
  expect(changes).toEqual([{ counts: ["/home/a"], badges: ["/home/a"], recounted: ["/home/a"] }]);

  // A folder a request is already counting isn't fetched again.
  void summaries.request(["/home/b"], false);
  summaries.one("/home/b", false);
  expect(calls).toHaveLength(2);
  await calls[1].answer([{ count: 1, versioned: false }]);
  expect(summaries.count("/home/b")).toBe(1);
  // That request's answer says it was asked for on its own too.
  expect(changes.at(-1)!.recounted).toEqual(["/home/b"]);

  // A late answer is dropped.
  summaries.one("/home/c", false);
  here.location = "/elsewhere";
  await calls[2].answer();
  expect(summaries.count("/home/c")).toBeNull();
  expect(changes).toHaveLength(2);
});

test("asking with a new Show Hidden setting forgets the counts but keeps the badges", async () => {
  const { summaries, calls, changes } = setup();
  void summaries.request(["/home/a"], false);
  await calls[0].answer([{ count: 2, versioned: true }]);
  const old = summaries.request(["/home/b"], false);

  const before = summaries.version;
  const fresh = summaries.request(["/home/a", "/home/b"], true);
  expect(summaries.version).not.toBe(before);
  expect(summaries.count("/home/a")).toBeUndefined();
  expect(summaries.versioned("/home/a")).toBe(true);
  expect(calls.map((c) => [c.paths, c.showHidden])).toEqual([
    [["/home/a"], false],
    [["/home/b"], false],
    [["/home/a", "/home/b"], true],
  ]);
  // The answer counted without hidden items lands first and is dropped.
  await calls[1].answer([{ count: 1, versioned: false }]);
  expect(await old).toBe(false);
  expect(summaries.count("/home/b")).toBeUndefined();
  await calls[2].answer([{ count: 5, versioned: true }, { count: 3, versioned: false }]);
  expect(await fresh).toBe(true);
  expect(summaries.count("/home/a")).toBe(5);
  expect(summaries.count("/home/b")).toBe(3);
  // The badge didn't change, so no icon needs redrawing.
  expect(changes.at(-1)).toEqual({ counts: ["/home/a", "/home/b"], badges: [], recounted: [] });

  // one() follows the setting too.
  summaries.one("/home/a", false);
  expect(summaries.count("/home/a")).toBeNull();
  expect(calls.at(-1)).toMatchObject({ paths: ["/home/a"], showHidden: false });
  await calls.at(-1)!.answer([{ count: 2, versioned: false }]);
  expect(summaries.count("/home/b")).toBeUndefined();
  expect(summaries.versioned("/home/a")).toBe(false);
  expect(changes.at(-1)).toEqual({ counts: ["/home/a"], badges: ["/home/a"], recounted: ["/home/a"] });
});

test("a folder asked for with one() is reported as recounted once, and not after a reset", async () => {
  const { summaries, calls, changes } = setup();
  summaries.one("/home/a", false);
  summaries.reset();
  void summaries.request(["/home/a", "/home/b"], false);
  await calls[1].answer();
  expect(changes.at(-1)!.recounted).toEqual([]);

  summaries.reset();
  void summaries.request(["/home/a"], false);
  summaries.one("/home/a", false);
  await calls[2].answer();
  expect(changes.at(-1)!.recounted).toEqual(["/home/a"]);
  summaries.reset();
  void summaries.request(["/home/a"], false);
  await calls[3].answer();
  expect(changes.at(-1)!.recounted).toEqual([]);
});
